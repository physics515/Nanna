//! Daemon Manager - Manages the nanna-daemon sidecar lifecycle
//!
//! Responsibilities:
//! - Start daemon on app boot
//! - Monitor daemon health
//! - Restart on crash
//! - Stop on app exit

use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_shell::{ShellExt, process::CommandChild};
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

/// Daemon process state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Crashed,
}

/// Daemon manager configuration
#[derive(Debug, Clone)]
pub struct DaemonManagerConfig {
    /// Port for the daemon to listen on
    pub port: u16,
    /// Host to bind to
    pub host: String,
    /// Maximum restart attempts before giving up
    pub max_restarts: u32,
    /// Delay between restart attempts
    pub restart_delay: Duration,
    /// Health check interval
    pub health_check_interval: Duration,
    /// Startup timeout
    pub startup_timeout: Duration,
}

impl Default for DaemonManagerConfig {
    fn default() -> Self {
        Self {
            port: 5149,
            host: "127.0.0.1".to_string(),
            max_restarts: 3,
            restart_delay: Duration::from_secs(2),
            health_check_interval: Duration::from_secs(30),
            // Generous: first boot can be slow (DB open + tool discovery). The
            // slow embedding-model probe was moved off the daemon's startup
            // critical path, so readiness is normally a few seconds — this is
            // purely a worst-case ceiling.
            startup_timeout: Duration::from_secs(90),
        }
    }
}

/// Manages the daemon sidecar process
pub struct DaemonManager {
    config: DaemonManagerConfig,
    state: Arc<RwLock<DaemonState>>,
    restart_count: Arc<RwLock<u32>>,
    child: Arc<RwLock<Option<CommandChild>>>,
}

impl DaemonManager {
    /// Create a new daemon manager
    pub fn new(config: DaemonManagerConfig) -> Self {
        Self {
            config,
            state: Arc::new(RwLock::new(DaemonState::Stopped)),
            restart_count: Arc::new(RwLock::new(0)),
            child: Arc::new(RwLock::new(None)),
        }
    }
    
    /// Get the daemon WebSocket URL
    pub fn ws_url(&self) -> String {
        format!("ws://{}:{}", self.config.host, self.config.port)
    }
    
    /// Get current daemon state
    pub async fn state(&self) -> DaemonState {
        *self.state.read().await
    }
    
    /// Start the daemon sidecar
    pub async fn start(&self, app: &AppHandle) -> Result<(), String> {
        let current_state = *self.state.read().await;
        if current_state == DaemonState::Running || current_state == DaemonState::Starting {
            return Ok(());
        }
        
        *self.state.write().await = DaemonState::Starting;
        info!("Starting nanna-daemon sidecar...");
        
        // Spawn the sidecar
        let shell = app.shell();
        info!("Creating sidecar command for nanna-daemon...");
        let sidecar = shell.sidecar("nanna-daemon")
            .map_err(|e| {
                error!("Failed to create sidecar command: {}", e);
                format!("Failed to create sidecar command: {}", e)
            })?;
        
        let args = ["--port", &self.config.port.to_string(), "--host", &self.config.host, "run"];
        info!("Spawning daemon with args: {:?}", args);
        let (mut rx, child) = sidecar
            .args(args)
            .spawn()
            .map_err(|e| {
                error!("Failed to spawn daemon: {}", e);
                format!("Failed to spawn daemon: {}", e)
            })?;
        
        // Store the child handle
        *self.child.write().await = Some(child);
        
        // Spawn a task to log daemon output (use info level so it's visible in production)
        // It also flips the shared state to Crashed on termination so wait_for_ready
        // can abort immediately instead of polling out the full startup timeout.
        let state_for_events = self.state.clone();
        tokio::spawn(async move {
            use tauri_plugin_shell::process::CommandEvent;
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stdout(line) => {
                        let msg = String::from_utf8_lossy(&line);
                        info!("daemon stdout: {}", msg);
                    }
                    CommandEvent::Stderr(line) => {
                        let msg = String::from_utf8_lossy(&line);
                        error!("daemon stderr: {}", msg);
                    }
                    CommandEvent::Terminated(payload) => {
                        // Record the death so any in-flight ready-wait bails out now.
                        {
                            let mut state = state_for_events.write().await;
                            if *state == DaemonState::Starting || *state == DaemonState::Running {
                                *state = DaemonState::Crashed;
                            }
                        }
                        if let Some(code) = payload.code {
                            if code != 0 {
                                error!("daemon terminated with exit code: {}", code);
                            } else {
                                info!("daemon terminated normally (code 0)");
                            }
                        } else if let Some(signal) = payload.signal {
                            warn!("daemon terminated by signal: {}", signal);
                        } else {
                            warn!("daemon terminated (unknown reason)");
                        }
                        break;
                    }
                    CommandEvent::Error(err) => {
                        error!("daemon error event: {}", err);
                    }
                    _ => {}
                }
            }
        });
        
        // Wait for daemon to be ready
        let ready = self.wait_for_ready().await;

        if ready {
            *self.state.write().await = DaemonState::Running;
            *self.restart_count.write().await = 0;
            info!("Daemon started successfully on {}", self.ws_url());
            Ok(())
        } else {
            *self.state.write().await = DaemonState::Crashed;
            error!("Daemon failed to start within timeout");
            // Kill the spawned child: an orphaned daemon would keep running,
            // holding the nanna.db file lock (and port 5149) — which would make
            // the embedded fallback unable to open storage at all.
            if let Some(child) = self.child.write().await.take() {
                warn!("Killing unresponsive daemon child so it cannot orphan the DB lock");
                if let Err(e) = child.kill() {
                    warn!("Failed to kill unresponsive daemon: {}", e);
                }
            }
            Err("Daemon startup timeout".to_string())
        }
    }
    
    /// Wait for daemon to be ready (accepting connections)
    async fn wait_for_ready(&self) -> bool {
        let url = self.ws_url();
        let deadline = tokio::time::Instant::now() + self.config.startup_timeout;
        
        let mut child_exited = false;
        while tokio::time::Instant::now() < deadline {
            // A dead child can still mean a healthy daemon: the sidecar
            // exits AlreadyRunning when a standalone daemon holds the
            // PID-file lock (single-instance guard), and that daemon may
            // still be mid-init (IPC binds late). Keep polling the port
            // until the deadline; only fail when nobody ever answers.
            if !child_exited && *self.state.read().await == DaemonState::Crashed {
                child_exited = true;
                warn!(
                    "Sidecar exited during startup — polling {} for an existing daemon instance",
                    url
                );
            }
            // Try to connect
            match tokio_tungstenite::connect_async(&url).await {
                Ok((mut ws, _)) => {
                    // Connected! Close and return success
                    let _ = futures_util::SinkExt::close(&mut ws).await;
                    if child_exited {
                        info!("Attached to an existing daemon instance on {}", url);
                    }
                    return true;
                }
                Err(_) => {
                    // Not ready yet, wait and retry
                    sleep(Duration::from_millis(200)).await;
                }
            }
        }
        
        false
    }
    
    /// Stop the daemon
    pub async fn stop(&self) -> Result<(), String> {
        let current_state = *self.state.read().await;
        if current_state == DaemonState::Stopped || current_state == DaemonState::Stopping {
            return Ok(());
        }
        
        *self.state.write().await = DaemonState::Stopping;
        info!("Stopping nanna-daemon...");
        
        // Kill the child process
        if let Some(child) = self.child.write().await.take() {
            if let Err(e) = child.kill() {
                warn!("Failed to kill daemon process: {}", e);
            }
        }
        
        *self.state.write().await = DaemonState::Stopped;
        info!("Daemon stopped");
        Ok(())
    }
    
    /// Restart the daemon
    pub async fn restart(&self, app: &AppHandle) -> Result<(), String> {
        self.stop().await?;
        sleep(Duration::from_millis(500)).await;
        self.start(app).await
    }
    
    /// Start health monitoring (call once after start)
    pub fn start_health_monitor(self: Arc<Self>, app: AppHandle) {
        let manager = self.clone();
        let config = self.config.clone();
        
        tokio::spawn(async move {
            loop {
                sleep(config.health_check_interval).await;
                
                let state = *manager.state.read().await;
                if state != DaemonState::Running {
                    continue;
                }
                
                // Health check: try to connect
                let url = manager.ws_url();
                match tokio_tungstenite::connect_async(&url).await {
                    Ok((mut ws, _)) => {
                        let _ = futures_util::SinkExt::close(&mut ws).await;
                        debug!("Daemon health check: OK");
                    }
                    Err(e) => {
                        warn!("Daemon health check failed: {}", e);
                        *manager.state.write().await = DaemonState::Crashed;
                        
                        // Try to restart
                        let restart_count = *manager.restart_count.read().await;
                        if restart_count < config.max_restarts {
                            *manager.restart_count.write().await = restart_count + 1;
                            warn!("Attempting daemon restart ({}/{})", restart_count + 1, config.max_restarts);
                            
                            sleep(config.restart_delay).await;
                            if let Err(e) = manager.start(&app).await {
                                error!("Daemon restart failed: {}", e);
                            }
                        } else {
                            error!("Max daemon restarts exceeded, giving up");
                        }
                    }
                }
            }
        });
    }
}

impl Drop for DaemonManager {
    fn drop(&mut self) {
        // Note: async drop not possible, but child will be killed when dropped
        info!("DaemonManager dropped");
    }
}
