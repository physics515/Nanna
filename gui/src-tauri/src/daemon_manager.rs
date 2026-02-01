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
            startup_timeout: Duration::from_secs(10),
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
        let sidecar = shell.sidecar("nanna-daemon")
            .map_err(|e| format!("Failed to create sidecar command: {}", e))?;
        
        let (mut rx, child) = sidecar
            .args(["run", "--port", &self.config.port.to_string(), "--host", &self.config.host])
            .spawn()
            .map_err(|e| format!("Failed to spawn daemon: {}", e))?;
        
        // Store the child handle
        *self.child.write().await = Some(child);
        
        // Spawn a task to log daemon output
        tokio::spawn(async move {
            use tauri_plugin_shell::process::CommandEvent;
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stdout(line) => {
                        debug!("daemon: {}", String::from_utf8_lossy(&line));
                    }
                    CommandEvent::Stderr(line) => {
                        warn!("daemon: {}", String::from_utf8_lossy(&line));
                    }
                    CommandEvent::Terminated(payload) => {
                        info!("daemon terminated: {:?}", payload);
                        break;
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
            Err("Daemon startup timeout".to_string())
        }
    }
    
    /// Wait for daemon to be ready (accepting connections)
    async fn wait_for_ready(&self) -> bool {
        let url = self.ws_url();
        let deadline = tokio::time::Instant::now() + self.config.startup_timeout;
        
        while tokio::time::Instant::now() < deadline {
            // Try to connect
            match tokio_tungstenite::connect_async(&url).await {
                Ok((mut ws, _)) => {
                    // Connected! Close and return success
                    let _ = futures_util::SinkExt::close(&mut ws).await;
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
        if let Some(mut child) = self.child.write().await.take() {
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
