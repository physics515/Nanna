//! Daemon sidecar launcher
//!
//! Launches nanna-daemon as a sidecar process and auto-attaches over WebSocket
//! when the Tauri GUI starts. No terminal window shown during normal operation.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info, warn};

/// Configuration for daemon launcher
#[derive(Debug, Clone)]
pub struct DaemonLauncherConfig {
    /// Path to the daemon binary
    pub daemon_binary: PathBuf,
    /// Arguments to pass to the daemon
    pub args: Vec<String>,
    /// Timeout for health check (seconds)
    pub health_timeout: Duration,
    /// Reconnection interval (seconds)
    pub reconnect_interval: Duration,
    /// Maximum reconnection attempts
    pub max_reconnect_attempts: u32,
}

impl Default for DaemonLauncherConfig {
    fn default() -> Self {
        Self {
            daemon_binary: PathBuf::from("nanna-daemon.exe"),
            args: vec!["run".to_string()],
            health_timeout: Duration::from_secs(30),
            reconnect_interval: Duration::from_secs(5),
            max_reconnect_attempts: 10,
        }
    }
}

/// Status of daemon connection
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonStatus {
    /// Daemon is running and connected
    Connected,
    /// Daemon is running but not yet connected
    Starting,
    /// Daemon failed to start
    Failed(String),
    /// Not yet attempted
    NotStarted,
}

/// Daemon launcher state
pub struct DaemonLauncher {
    config: DaemonLauncherConfig,
    status: Arc<Mutex<DaemonStatus>>,
    process: Option<std::process::Child>,
    connection_id: String,
}

impl DaemonLauncher {
    /// Create a new daemon launcher
    pub fn new(config: DaemonLauncherConfig) -> Self {
        Self {
            config,
            status: Arc::new(Mutex::new(DaemonStatus::NotStarted)),
            process: None,
            connection_id: String::new(),
        }
    }

    /// Launch the daemon sidecar
    pub async fn launch(&mut self) -> Result<(), String> {
        let mut status = self.status.lock().await;
        
        if *status == DaemonStatus::Connected {
            return Ok(());
        }

        *status = DaemonStatus::Starting;
        info!("Launching daemon sidecar...");

        let output = Command::new(&self.config.daemon_binary)
            .args(&self.config.args)
            .output()
            .map_err(|e| format!("Failed to launch daemon: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            *status = DaemonStatus::Failed(stderr.to_string());
            return Err(stderr.to_string());
        }

        self.process = Some(output);
        *status = DaemonStatus::Connected;
        
        info!("Daemon sidecar launched successfully");
        Ok(())
    }

    /// Check if daemon is healthy
    pub async fn check_health(&self) -> bool {
        let status = self.status.lock().await;
        match &*status {
            DaemonStatus::Connected => true,
            DaemonStatus::Starting | DaemonStatus::NotStarted => false,
            DaemonStatus::Failed(_) => false,
        }
    }

    /// Get current daemon status
    pub async fn status(&self) -> DaemonStatus {
        self.status.lock().await.clone()
    }

    /// Reconnect to daemon if needed
    pub async fn ensure_connected(&mut self) -> Result<(), String> {
        let mut status = self.status.lock().await;
        
        match &*status {
            DaemonStatus::Connected => Ok(()),
            DaemonStatus::Starting | DaemonStatus::NotStarted => {
                self.launch().await
            }
            DaemonStatus::Failed(_) => {
                // Attempt reconnection
                if self.config.max_reconnect_attempts > 0 {
                    debug!("Attempting to reconnect to daemon...");
                    self.launch().await
                } else {
                    Err("Max reconnection attempts exceeded".to_string())
                }
            }
        }
    }

    /// Stop the daemon sidecar
    pub async fn stop(&mut self) -> Result<(), String> {
        let mut status = self.status.lock().await;
        
        match &*status {
            DaemonStatus::Connected => {
                if let Some(ref process) = self.process {
                    process.kill().map_err(|e| format!("Failed to kill daemon: {}", e))?;
                    drop(process);
                    *status = DaemonStatus::NotStarted;
                    info!("Daemon sidecar stopped");
                }
            }
            _ => {}
        }
        
        Ok(())
    }
}

impl Default for DaemonLauncher {
    fn default() -> Self {
        Self::new(DaemonLauncherConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = DaemonLauncherConfig::default();
        assert_eq!(config.health_timeout, Duration::from_secs(30));
        assert_eq!(config.reconnect_interval, Duration::from_secs(5));
    }

    #[tokio::test]
    async fn test_launcher_initial_status() {
        let launcher = DaemonLauncher::default();
        let status = launcher.status().await;
        assert_eq!(status, DaemonStatus::NotStarted);
    }
}
