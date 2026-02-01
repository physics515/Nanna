//! Daemon Server - Main daemon orchestrator
//!
//! Combines IPC server, control plane, sessions, persistence, and all subsystems.

use crate::control::ControlPlane;
use crate::ipc::{IpcServer, IpcServerConfig};
use crate::persistence::PersistenceManager;
use crate::protocol::Response;
use crate::session::SessionManager;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

/// Configuration for the daemon server
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// IPC server configuration
    pub ipc: IpcServerConfig,
    /// Data directory
    pub data_dir: std::path::PathBuf,
    /// Log level
    pub log_level: String,
    /// Auto-save interval in seconds
    pub auto_save_interval_secs: u64,
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
            auto_save_interval_secs: 60, // Save every minute
        }
    }
}

/// The main daemon server
pub struct DaemonServer {
    config: DaemonConfig,
    sessions: Arc<SessionManager>,
    control: Arc<ControlPlane>,
    ipc: IpcServer,
    persistence: Arc<PersistenceManager>,
    shutdown_tx: broadcast::Sender<()>,
}

impl DaemonServer {
    /// Create a new daemon server
    pub fn new(config: DaemonConfig) -> Self {
        let sessions = Arc::new(SessionManager::new());
        let control = Arc::new(ControlPlane::new(sessions.clone()));
        let ipc = IpcServer::new(config.ipc.clone());
        let persistence = Arc::new(PersistenceManager::new(&config.data_dir));
        let (shutdown_tx, _) = broadcast::channel(1);
        
        Self {
            config,
            sessions,
            control,
            ipc,
            persistence,
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
        
        // Load persisted sessions
        match self.persistence.load_sessions_with_fallback().await {
            Ok((sessions, default_id)) => {
                info!("Loaded {} persisted sessions", sessions.len());
                for session in sessions {
                    self.sessions.restore(session).await;
                }
                if let Some(id) = default_id {
                    self.sessions.set_default(&id).await;
                }
            }
            Err(e) => {
                warn!("Failed to load persisted sessions: {}", e);
            }
        }
        
        // Create default session if none exist
        if self.sessions.count().await == 0 {
            let default_session = self.sessions.create(Some("Main".to_string())).await;
            info!("Created default session: {}", default_session.id);
        }
        
        // Take the request receiver from IPC server
        let mut request_rx = self.ipc.take_request_receiver();
        
        // Clone what we need for the request handler
        let control = self.control.clone();
        let ipc_clone = Arc::new(tokio::sync::RwLock::new(IpcServer::new(self.config.ipc.clone())));
        
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        
        // Spawn IPC server
        let ipc_config = self.config.ipc.clone();
        let ipc_handle = tokio::spawn(async move {
            let ipc_server = IpcServer::new(ipc_config);
            if let Err(e) = ipc_server.run().await {
                error!("IPC server error: {}", e);
            }
        });
        
        // Spawn auto-save task
        let persistence = self.persistence.clone();
        let sessions_for_save = self.sessions.clone();
        let save_interval = Duration::from_secs(self.config.auto_save_interval_secs);
        let save_shutdown = self.shutdown_tx.subscribe();
        
        let save_handle = tokio::spawn(async move {
            let sessions_map = sessions_for_save.sessions_map();
            let default_session = sessions_for_save.default_session_id();
            persistence.auto_save_loop(sessions_map, default_session, save_interval, save_shutdown).await;
        });
        
        info!("Daemon ready. IPC server listening on ws://{}", self.ipc.address());
        
        // Main event loop
        loop {
            tokio::select! {
                // Handle incoming requests
                Some((client_id, request)) = request_rx.recv() => {
                    let result = control.handle(&client_id, request.action).await;
                    let response = Response::success(request.id, result);
                    
                    // TODO: Send response back to client via IPC
                    // For now we just log it
                    info!("Response for {}: success={}", client_id, !response.is_error());
                }
                
                // Handle shutdown signal
                _ = shutdown_rx.recv() => {
                    info!("Shutdown signal received");
                    break;
                }
            }
        }
        
        // Cleanup
        info!("Shutting down daemon...");
        self.ipc.shutdown();
        
        // Wait for auto-save to complete final save
        let _ = tokio::time::timeout(Duration::from_secs(5), save_handle).await;
        
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
    
    pub fn with_auto_save_interval(mut self, secs: u64) -> Self {
        self.config.auto_save_interval_secs = secs;
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
