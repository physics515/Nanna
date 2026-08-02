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

/// Kill a process and its descendants by PID, best-effort. A current daemon's
/// own kill-on-close Job Object (adopted at startup) already reaps its
/// children when it dies; this is belt-and-braces for daemons where adoption
/// failed or that predate it.
fn kill_process_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        // Unix children run in their own process groups with kill_on_drop;
        // the caller's CommandChild::kill() covers the direct child.
        let _ = pid;
    }
}

/// Manages the daemon sidecar process
pub struct DaemonManager {
    config: DaemonManagerConfig,
    state: Arc<RwLock<DaemonState>>,
    restart_count: Arc<RwLock<u32>>,
    child: Arc<RwLock<Option<CommandChild>>>,
    /// Whether the spawned sidecar process has terminated. Distinguishes a
    /// live sidecar we own (stop = graceful shutdown, then tree-kill) from a
    /// dead one whose PID may have been recycled — e.g. the AlreadyRunning
    /// exit when we merely attached to a standalone daemon, which is not
    /// ours to stop.
    sidecar_exited: Arc<std::sync::atomic::AtomicBool>,
}

impl DaemonManager {
    /// Create a new daemon manager
    pub fn new(config: DaemonManagerConfig) -> Self {
        Self {
            config,
            state: Arc::new(RwLock::new(DaemonState::Stopped)),
            restart_count: Arc::new(RwLock::new(0)),
            child: Arc::new(RwLock::new(None)),
            sidecar_exited: Arc::new(std::sync::atomic::AtomicBool::new(false)),
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
        
        // A dev build must never share the installed app's store: the daemon
        // takes an exclusive lock on nanna.db and runs migrations at startup,
        // so two builds pointed at one file is unsupported and unsafe.
        // NANNA_DEV_DATA_DIR isolates a dev run; unset in production, where
        // the daemon resolves its own default data dir as before.
        let port = self.config.port.to_string();
        let mut args: Vec<String> = vec![
            "--port".into(),
            port,
            "--host".into(),
            self.config.host.clone(),
        ];
        if let Ok(dev_data_dir) = std::env::var("NANNA_DEV_DATA_DIR") {
            if !dev_data_dir.trim().is_empty() {
                info!("NANNA_DEV_DATA_DIR set — isolating daemon store at {dev_data_dir}");
                args.push("--data-dir".into());
                args.push(dev_data_dir);
            }
        }
        args.push("run".into());
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
        self.sidecar_exited
            .store(false, std::sync::atomic::Ordering::SeqCst);

        // Spawn a task to log daemon output (use info level so it's visible in production)
        // It also flips the shared state to Crashed on termination so wait_for_ready
        // can abort immediately instead of polling out the full startup timeout.
        let state_for_events = self.state.clone();
        let sidecar_exited = self.sidecar_exited.clone();
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
                        // Record the death so any in-flight ready-wait bails
                        // out now, and stop() can tell a graceful exit from a
                        // hang (and never tree-kills a recycled PID).
                        sidecar_exited.store(true, std::sync::atomic::Ordering::SeqCst);
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
                // Tree-kill only a sidecar that is still alive: a dead one's
                // PID may already belong to an unrelated process.
                if !self.sidecar_exited.load(std::sync::atomic::Ordering::SeqCst) {
                    kill_process_tree(child.pid());
                }
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

        if let Some(child) = self.child.write().await.take() {
            if self.sidecar_exited.load(std::sync::atomic::Ordering::SeqCst) {
                // The sidecar died earlier — usually its AlreadyRunning exit
                // after attaching to a standalone daemon. That daemon isn't
                // ours to stop, and the sidecar's PID may have been recycled,
                // so neither a shutdown request nor a tree-kill is safe here.
                info!("Sidecar already exited — leaving any attached daemon running");
            } else {
                let pid = child.pid();
                // Prefer a graceful IPC shutdown: the daemon flushes state and
                // its kill-on-close Job Object reaps in-flight exec children on
                // exit. A pre-Job-Object daemon acks the request without
                // stopping — the bounded wait catches that and falls through.
                let exited = self.request_graceful_shutdown().await
                    && self.wait_for_terminated(Duration::from_secs(5)).await;
                if exited {
                    info!("Daemon exited gracefully");
                } else {
                    // Hard stop: kill the TREE, not just the daemon — a bare
                    // kill() orphans in-flight exec children on daemons whose
                    // Job Object never adopted (or that predate it).
                    warn!("Graceful daemon shutdown failed — tree-killing PID {pid}");
                    kill_process_tree(pid);
                    if let Err(e) = child.kill() {
                        warn!("Failed to kill daemon process: {}", e);
                    }
                }
            }
        }

        *self.state.write().await = DaemonState::Stopped;
        info!("Daemon stopped");
        Ok(())
    }

    /// Ask the daemon to stop via its control plane. Returns whether the
    /// request was delivered — not whether the daemon actually exited.
    async fn request_graceful_shutdown(&self) -> bool {
        use futures_util::SinkExt;
        use tokio_tungstenite::tungstenite::Message;

        let url = self.ws_url();
        let Ok(Ok((mut ws, _))) =
            tokio::time::timeout(Duration::from_secs(2), tokio_tungstenite::connect_async(&url))
                .await
        else {
            return false;
        };
        let request = serde_json::json!({
            "id": "daemon-manager-shutdown",
            "action": { "type": "system", "action": "shutdown" }
        });
        // send() flushes before returning; the daemon acks, then stops.
        let sent = ws.send(Message::Text(request.to_string().into())).await.is_ok();
        let _ = ws.close(None).await;
        sent
    }

    /// Wait (bounded) for the sidecar's Terminated event, recorded by the
    /// event task in `sidecar_exited`.
    async fn wait_for_terminated(&self, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        while tokio::time::Instant::now() < deadline {
            if self.sidecar_exited.load(std::sync::atomic::Ordering::SeqCst) {
                return true;
            }
            sleep(Duration::from_millis(100)).await;
        }
        false
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
