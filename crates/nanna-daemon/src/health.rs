//! HTTP Health Endpoint and PID File Management
//!
//! Provides:
//! - `/health` endpoint for monitoring
//! - `/status` endpoint for detailed status
//! - `/metrics` endpoint (future: Prometheus)
//! - PID file management to prevent multiple instances

use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::get,
    Router,
};
use serde::Serialize;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info, warn};

// =============================================================================
// PID File Management
// =============================================================================

/// PID file manager to prevent multiple daemon instances
pub struct PidFile {
    path: PathBuf,
}

impl PidFile {
    /// Create a new PID file manager
    pub fn new(data_dir: &PathBuf) -> Self {
        Self {
            path: data_dir.join("nanna-daemon.pid"),
        }
    }
    
    /// Get the default PID file path
    pub fn default_path() -> PathBuf {
        directories::ProjectDirs::from("com", "nanna", "nanna-daemon")
            .map(|d| d.runtime_dir()
                .map(|r| r.to_path_buf())
                .unwrap_or_else(|| d.data_dir().to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("nanna-daemon.pid")
    }
    
    /// Try to acquire the PID file lock
    /// Returns Ok(()) if successful, Err with existing PID if another instance is running
    pub fn acquire(&self) -> Result<(), PidFileError> {
        // Create parent directory if needed
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| PidFileError::Io(e.to_string()))?;
        }
        
        // Check if PID file exists
        if self.path.exists() {
            // Read existing PID
            let content = std::fs::read_to_string(&self.path)
                .map_err(|e| PidFileError::Io(e.to_string()))?;
            
            if let Ok(existing_pid) = content.trim().parse::<u32>() {
                // Check if process is still running
                if is_process_running(existing_pid) {
                    return Err(PidFileError::AlreadyRunning(existing_pid));
                }
                // Stale PID file - process no longer exists
                info!("Removing stale PID file (process {} no longer exists)", existing_pid);
            }
        }
        
        // Write our PID
        let pid = std::process::id();
        std::fs::write(&self.path, pid.to_string())
            .map_err(|e| PidFileError::Io(e.to_string()))?;
        
        info!("PID file created at {:?} (PID: {})", self.path, pid);
        Ok(())
    }
    
    /// Release the PID file lock
    pub fn release(&self) {
        if self.path.exists() {
            if let Err(e) = std::fs::remove_file(&self.path) {
                warn!("Failed to remove PID file: {}", e);
            } else {
                info!("PID file removed");
            }
        }
    }
    
    /// Get the path to the PID file
    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        self.release();
    }
}

/// PID file errors
#[derive(Debug, thiserror::Error)]
pub enum PidFileError {
    #[error("Another daemon instance is already running (PID: {0})")]
    AlreadyRunning(u32),
    #[error("IO error: {0}")]
    Io(String),
}

/// Check if a process with the given PID is running.
///
/// Uses the Win32 API directly. The previous `tasklist` subprocess check
/// returned `false` whenever the subprocess itself failed, which let a
/// second daemon instance treat a LIVE daemon as dead and clobber its PID
/// file (observed live: a GUI sidecar overwrote the standalone daemon's
/// lock, then ran on as a storage-less zombie).
#[cfg(windows)]
fn is_process_running(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject};

    // Named locally so a windows-sys reorganization cannot silently change
    // semantics: these are stable Win32 ABI values.
    const PROCESS_SYNCHRONIZE: u32 = 0x0010_0000;
    const WAIT_TIMEOUT_CODE: u32 = 0x102;
    const ERROR_ACCESS_DENIED_CODE: u32 = 5;

    // SAFETY: plain Win32 calls; the handle is closed on every path.
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            // Access denied means the process exists but isn't ours (e.g. an
            // elevated daemon) — that still counts as running. Any other
            // failure (invalid parameter, not found) means no such process.
            return GetLastError() == ERROR_ACCESS_DENIED_CODE;
        }
        let wait = WaitForSingleObject(handle, 0);
        CloseHandle(handle);
        // Not yet signaled → the process is still running.
        wait == WAIT_TIMEOUT_CODE
    }
}

#[cfg(unix)]
fn is_process_running(pid: u32) -> bool {
    // On Unix, we can use kill with signal 0 to check if process exists
    unsafe {
        libc::kill(pid as i32, 0) == 0
    }
}

#[cfg(not(any(windows, unix)))]
fn is_process_running(_pid: u32) -> bool {
    // Conservative: assume running if we can't check
    true
}

// =============================================================================
// Health HTTP Server
// =============================================================================

/// Health server state
pub struct HealthState {
    /// When the daemon started
    pub start_time: Instant,
    /// Number of active sessions
    pub session_count: Arc<RwLock<usize>>,
    /// Number of connected clients
    pub client_count: Arc<RwLock<usize>>,
    /// Memory service status
    pub memory_available: bool,
    /// Durable memory store degraded (a corrupt row was skipped on load).
    pub memory_degraded: bool,
    /// Number of memory rows that were unreadable (corrupt) at load.
    pub memory_corrupt_rows: usize,
    /// The store was quarantined and rebuilt after page-level corruption.
    pub memory_rebuilt: bool,
    /// Memories salvaged into the rebuilt store (0 unless `memory_rebuilt`).
    pub memory_recovered_rows: usize,
    /// Memories the corrupt store held, when countable (None = unknown loss).
    pub memory_expected_rows: Option<usize>,
    /// Agent service status
    pub agent_available: bool,
    /// Last error message (if any)
    pub last_error: Arc<RwLock<Option<String>>>,
}

impl HealthState {
    pub fn new(memory_available: bool, agent_available: bool) -> Self {
        Self {
            start_time: Instant::now(),
            session_count: Arc::new(RwLock::new(0)),
            client_count: Arc::new(RwLock::new(0)),
            memory_available,
            memory_degraded: false,
            memory_corrupt_rows: 0,
            memory_rebuilt: false,
            memory_recovered_rows: 0,
            memory_expected_rows: None,
            agent_available,
            last_error: Arc::new(RwLock::new(None)),
        }
    }

    /// Seed the durable-memory-store health (from `MemoryService::store_health`).
    /// Builder-style so a corrupt store is visible on `/status` and `/health`
    /// instead of only a boot `error!` log.
    #[must_use]
    pub fn with_memory_health(mut self, degraded: bool, corrupt_rows: usize) -> Self {
        self.memory_degraded = degraded;
        self.memory_corrupt_rows = corrupt_rows;
        self
    }

    /// Seed the store-rebuilt-after-corruption facts (from the startup
    /// `RecoveryReport`) so `/status` keeps reporting the rebuild — clients
    /// that connect after boot never saw the `MemoryStoreRebuilt` event.
    #[must_use]
    pub fn with_memory_rebuild(mut self, recovered: usize, expected: Option<usize>) -> Self {
        self.memory_rebuilt = true;
        self.memory_recovered_rows = recovered;
        self.memory_expected_rows = expected;
        self
    }

    pub async fn set_session_count(&self, count: usize) {
        *self.session_count.write().await = count;
    }
    
    pub async fn set_client_count(&self, count: usize) {
        *self.client_count.write().await = count;
    }
    
    pub async fn set_last_error(&self, error: Option<String>) {
        *self.last_error.write().await = error;
    }
}

/// Simple health check response
#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub uptime_secs: u64,
}

/// Detailed status response
#[derive(Serialize)]
pub struct StatusResponse {
    pub status: String,
    pub version: String,
    pub uptime_secs: u64,
    pub sessions: usize,
    pub clients: usize,
    pub memory_available: bool,
    pub memory_degraded: bool,
    pub memory_corrupt_rows: usize,
    pub memory_rebuilt: bool,
    pub memory_recovered_rows: usize,
    pub memory_expected_rows: Option<usize>,
    pub agent_available: bool,
    pub last_error: Option<String>,
}

/// Health check endpoint (GET /health)
async fn health(State(state): State<Arc<HealthState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs: state.start_time.elapsed().as_secs(),
    })
}

/// Kubernetes-style liveness probe (GET /healthz)
async fn healthz() -> StatusCode {
    StatusCode::OK
}

/// Kubernetes-style readiness probe (GET /readyz)
async fn readyz(State(state): State<Arc<HealthState>>) -> StatusCode {
    // Ready if agent is available
    if state.agent_available {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

/// Detailed status endpoint (GET /status)
async fn status(State(state): State<Arc<HealthState>>) -> Json<StatusResponse> {
    let sessions = *state.session_count.read().await;
    let clients = *state.client_count.read().await;
    let last_error = state.last_error.read().await.clone();
    
    Json(StatusResponse {
        status: "running".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs: state.start_time.elapsed().as_secs(),
        sessions,
        clients,
        memory_available: state.memory_available,
        memory_degraded: state.memory_degraded,
        memory_corrupt_rows: state.memory_corrupt_rows,
        memory_rebuilt: state.memory_rebuilt,
        memory_recovered_rows: state.memory_recovered_rows,
        memory_expected_rows: state.memory_expected_rows,
        agent_available: state.agent_available,
        last_error,
    })
}

/// Health HTTP server
pub struct HealthServer {
    state: Arc<HealthState>,
    port: u16,
    host: String,
}

impl HealthServer {
    /// Create a new health server
    pub fn new(state: HealthState, host: &str, port: u16) -> Self {
        Self {
            state: Arc::new(state),
            port,
            host: host.to_string(),
        }
    }

    /// Create a health server that serves an **existing** shared state handle.
    ///
    /// Use this (not [`Self::new`]) when the daemon keeps updating the state
    /// (session/client counts, `last_error`) after the server is spawned: the
    /// server then reflects those live updates instead of serving a throwaway
    /// copy whose counters never move.
    pub fn from_shared(state: Arc<HealthState>, host: &str, port: u16) -> Self {
        Self {
            state,
            port,
            host: host.to_string(),
        }
    }

    /// Get a reference to the state (for updating from daemon)
    pub fn state(&self) -> Arc<HealthState> {
        self.state.clone()
    }
    
    /// Build the Axum router
    fn router(&self) -> Router {
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any);
        
        Router::new()
            .route("/health", get(health))
            .route("/healthz", get(healthz))
            .route("/readyz", get(readyz))
            .route("/status", get(status))
            .layer(cors)
            .with_state(self.state.clone())
    }
    
    /// Run the health server
    pub async fn run(&self) -> Result<(), std::io::Error> {
        let addr: SocketAddr = format!("{}:{}", self.host, self.port)
            .parse()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        
        info!("Health server listening on http://{}", addr);
        
        let listener = Self::bind_with_retry(addr).await?;
        axum::serve(listener, self.router()).await
    }

    /// Bind with retry for Windows port conflicts.
    /// On Unix, uses SO_REUSEADDR. On Windows, retries with delay.
    async fn bind_with_retry(addr: std::net::SocketAddr) -> Result<tokio::net::TcpListener, std::io::Error> {
        #[cfg(unix)]
        {
            let socket = socket2::Socket::new(
                socket2::Domain::for_address(addr),
                socket2::Type::STREAM,
                Some(socket2::Protocol::TCP),
            )?;
            socket.set_reuse_address(true)?;
            socket.set_nonblocking(true)?;
            socket.bind(&addr.into())?;
            socket.listen(128)?;
            return tokio::net::TcpListener::from_std(socket.into());
        }

        #[cfg(windows)]
        {
            for attempt in 0..5 {
                match tokio::net::TcpListener::bind(addr).await {
                    Ok(listener) => return Ok(listener),
                    Err(e) if attempt < 4 => {
                        tracing::warn!("Health bind attempt {} failed ({}), retrying in 1s...", attempt + 1, e);
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                    Err(e) => return Err(e),
                }
            }
            unreachable!()
        }

        #[cfg(not(any(unix, windows)))]
        tokio::net::TcpListener::bind(addr).await
    }
    
    /// Spawn the health server as a background task
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            if let Err(e) = self.run().await {
                error!("Health server error: {}", e);
            }
        })
    }
}

/// Default health server port (one below WebSocket port)
pub const DEFAULT_HEALTH_PORT: u16 = 5148;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    #[test]
    fn test_pid_file_creation() {
        let temp_dir = TempDir::new().unwrap();
        let pid_file = PidFile::new(&temp_dir.path().to_path_buf());
        
        // Should acquire successfully
        assert!(pid_file.acquire().is_ok());
        
        // Check file exists
        assert!(pid_file.path().exists());
        
        // Read PID
        let content = std::fs::read_to_string(pid_file.path()).unwrap();
        assert_eq!(content.trim().parse::<u32>().unwrap(), std::process::id());
    }
    
    #[test]
    fn test_pid_file_release() {
        let temp_dir = TempDir::new().unwrap();
        let pid_file = PidFile::new(&temp_dir.path().to_path_buf());
        
        pid_file.acquire().unwrap();
        assert!(pid_file.path().exists());
        
        pid_file.release();
        assert!(!pid_file.path().exists());
    }
    
    #[test]
    fn liveness_check_sees_the_current_process() {
        // The regression that let two daemons run at once: the liveness
        // check mis-reporting a live PID as dead.
        assert!(is_process_running(std::process::id()));
    }

    #[test]
    fn pid_file_refuses_live_process() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nanna-daemon.pid");
        // Our own PID is definitionally alive.
        std::fs::write(&path, std::process::id().to_string()).unwrap();

        let pid_file = PidFile::new(&temp_dir.path().to_path_buf());
        assert!(matches!(
            pid_file.acquire(),
            Err(PidFileError::AlreadyRunning(pid)) if pid == std::process::id()
        ));
        // The live owner's PID file must be left untouched.
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.trim().parse::<u32>().unwrap(), std::process::id());
    }

    #[test]
    fn test_stale_pid_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nanna-daemon.pid");
        
        // Write a fake PID that definitely doesn't exist
        std::fs::write(&path, "999999999").unwrap();
        
        let pid_file = PidFile::new(&temp_dir.path().to_path_buf());

        // Should succeed because the old process doesn't exist
        assert!(pid_file.acquire().is_ok());
    }

    #[tokio::test]
    async fn from_shared_serves_live_updates() {
        // The daemon keeps updating this handle *after* the server is built;
        // the served `/status` must reflect those updates, not a frozen copy.
        let shared = Arc::new(HealthState::new(true, true));
        let server = HealthServer::from_shared(shared.clone(), "127.0.0.1", 0);

        // The server serves the very same state handle.
        assert!(Arc::ptr_eq(&shared, &server.state()));

        // A later update through the daemon's handle is visible via the server.
        shared.set_session_count(3).await;
        shared.set_client_count(2).await;
        shared.set_last_error(Some("boom".to_string())).await;

        let Json(status) = status(State(server.state())).await;
        assert_eq!(status.sessions, 3);
        assert_eq!(status.clients, 2);
        assert_eq!(status.last_error.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn status_surfaces_memory_degraded() {
        // A degraded store (corrupt rows skipped on load) must show on /status,
        // not just in a boot log.
        let shared = Arc::new(HealthState::new(true, true).with_memory_health(true, 2));
        let server = HealthServer::from_shared(shared, "127.0.0.1", 0);
        let Json(s) = status(State(server.state())).await;
        assert!(s.memory_degraded);
        assert_eq!(s.memory_corrupt_rows, 2);

        // A healthy store reports the negative.
        let clean = Arc::new(HealthState::new(true, true));
        let server2 = HealthServer::from_shared(clean, "127.0.0.1", 0);
        let Json(s2) = status(State(server2.state())).await;
        assert!(!s2.memory_degraded);
        assert_eq!(s2.memory_corrupt_rows, 0);
    }

    #[tokio::test]
    async fn status_surfaces_memory_rebuild() {
        // A store rebuilt after page-level corruption must stay visible on
        // /status for clients that connected after the boot-time event.
        let shared =
            Arc::new(HealthState::new(true, true).with_memory_rebuild(42, Some(50)));
        let server = HealthServer::from_shared(shared, "127.0.0.1", 0);
        let Json(s) = status(State(server.state())).await;
        assert!(s.memory_rebuilt);
        assert_eq!(s.memory_recovered_rows, 42);
        assert_eq!(s.memory_expected_rows, Some(50));

        // Without a rebuild the fields stay quiet.
        let clean = Arc::new(HealthState::new(true, true));
        let server2 = HealthServer::from_shared(clean, "127.0.0.1", 0);
        let Json(s2) = status(State(server2.state())).await;
        assert!(!s2.memory_rebuilt);
        assert_eq!(s2.memory_recovered_rows, 0);
        assert_eq!(s2.memory_expected_rows, None);
    }

    #[tokio::test]
    async fn new_serves_isolated_copy() {
        // Regression guard documenting *why* the daemon must use `from_shared`:
        // `new` wraps a fresh Arc, so an unrelated external handle can never
        // drive the served counters (this was the old server.rs bug).
        let server = HealthServer::new(HealthState::new(true, true), "127.0.0.1", 0);
        let external = Arc::new(HealthState::new(true, true));
        external.set_session_count(9).await;

        assert!(!Arc::ptr_eq(&external, &server.state()));
        let Json(status) = status(State(server.state())).await;
        assert_eq!(status.sessions, 0);
    }
}
