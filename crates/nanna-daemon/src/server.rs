//! Daemon Server - Main daemon orchestrator
//!
//! Combines IPC server, control plane, sessions, and all subsystems.

use crate::control::ControlPlane;
use crate::ipc::{IpcServer, IpcServerConfig};
use crate::protocol::{Request, Response};
use crate::session::SessionManager;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{error, info};

/// Configuration for the daemon server
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// IPC server configuration
    pub ipc: IpcServerConfig,
    /// Data directory
    pub data_dir: std::path::PathBuf,
    /// Log level
    pub log_level: String,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        let data_dir = directories::ProjectDirs::from("com", "nanna", "nanna-daemon")
            .map(|d| d.data_dir().to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("./data"));
        
        Self {
            ipc: IpcServerConfig::default(),
            data_dir,
            log_level: "info".to_string(),
        }
    }
}

/// The main daemon server
pub struct DaemonServer {
    config: DaemonConfig,
    sessions: Arc<SessionManager>,
    control: Arc<ControlPlane>,
    ipc: IpcServer,
    shutdown_tx: broadcast::Sender<()>,
}

impl DaemonServer {
    /// Create a new daemon server
    pub fn new(config: DaemonConfig) -> Self {
        let sessions = Arc::new(SessionManager::new());
        let control = Arc::new(ControlPlane::new(sessions.clone()));
        let ipc = IpcServer::new(config.ipc.clone());
        let (shutdown_tx, _) = broadcast::channel(1);
        
        Self {
            config,
            sessions,
            control,
            ipc,
            shutdown_tx,
        }
    }
    
    /// Get the shutdown sender (for signaling shutdown)
    pub fn shutdown_handle(&self) -> broadcast::Sender<()> {
        self.shutdown_tx.clone()
    }
    
    /// Get the IPC server address
    pub fn ipc_address(&self) -> String {
        self.ipc.address()
    }
    
    /// Run the daemon server
    pub async fn run(&mut self) -> Result<(), crate::DaemonError> {
        info!("Starting Nanna daemon...");
        info!("Data directory: {:?}", self.config.data_dir);
        
        // Ensure data directory exists
        std::fs::create_dir_all(&self.config.data_dir)?;
        
        // Create default session
        let default_session = self.sessions.create(Some("Main".to_string())).await;
        info!("Created default session: {}", default_session.id);
        
        // Take the request receiver from IPC server
        let mut request_rx = self.ipc.take_request_receiver();
        
        // Clone what we need for the request handler
        let control = self.control.clone();
        let ipc_for_response = Arc::new(tokio::sync::RwLock::new(None::<Arc<IpcServer>>));
        
        // We can't easily share IpcServer, so we'll use channels for responses
        // For now, we'll handle responses inline
        
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        
        // Spawn IPC server
        let ipc_handle = {
            let ipc_server = IpcServer::new(self.config.ipc.clone());
            tokio::spawn(async move {
                if let Err(e) = ipc_server.run().await {
                    error!("IPC server error: {}", e);
                }
            })
        };
        
        info!("Daemon ready. IPC server listening on ws://{}", self.ipc.address());
        
        // Main event loop
        loop {
            tokio::select! {
                // Handle incoming requests
                Some((client_id, request)) = request_rx.recv() => {
                    let result = control.handle(&client_id, request.action).await;
                    let response = Response::success(request.id, result);
                    
                    // TODO: Send response back to client
                    // For now we just log it
                    info!("Response for {}: {:?}", client_id, response.result);
                }
                
                // Handle shutdown signal
                _ = shutdown_rx.recv() => {
                    info!("Shutdown signal received");
                    break;
                }
            }
        }
        
        // Cleanup
        self.ipc.shutdown();
        ipc_handle.abort();
        
        info!("Daemon stopped");
        Ok(())
    }
}

/// Builder for DaemonServer
pub struct DaemonBuilder {
    config: DaemonConfig,
}

impl DaemonBuilder {
    pub fn new() -> Self {
        Self {
            config: DaemonConfig::default(),
        }
    }
    
    pub fn with_port(mut self, port: u16) -> Self {
        self.config.ipc.port = port;
        self
    }
    
    pub fn with_host(mut self, host: impl Into<String>) -> Self {
        self.config.ipc.host = host.into();
        self
    }
    
    pub fn with_data_dir(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.config.data_dir = path.into();
        self
    }
    
    pub fn with_log_level(mut self, level: impl Into<String>) -> Self {
        self.config.log_level = level.into();
        self
    }
    
    pub fn build(self) -> DaemonServer {
        DaemonServer::new(self.config)
    }
}

impl Default for DaemonBuilder {
    fn default() -> Self {
        Self::new()
    }
}
