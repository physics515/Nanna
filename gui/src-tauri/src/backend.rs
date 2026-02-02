//! Backend abstraction layer
//!
//! Provides a unified interface that can use either:
//! - Daemon mode: Starts and connects to nanna-daemon sidecar
//! - Embedded mode: Run agent directly in the GUI process (fallback)
//!
//! The GUI commands use this layer, which automatically selects the right backend.

use crate::daemon_client::{ConnectionMode, DaemonClient, DaemonClientConfig, DaemonEvent};
use crate::daemon_manager::{DaemonManager, DaemonManagerConfig, DaemonState};
use crate::embedded::EmbeddedBackend;
use nanna_storage::Session;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

/// Backend mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendMode {
    /// Connected to daemon sidecar
    Daemon,
    /// Running embedded agent (fallback)
    Embedded,
}

/// Backend status for frontend
#[derive(Debug, Clone, Serialize)]
pub struct BackendStatus {
    pub mode: BackendMode,
    pub connected: bool,
    pub daemon_url: Option<String>,
    pub daemon_state: String,
    pub version: String,
}

/// The unified backend interface
pub struct Backend {
    mode: Arc<RwLock<BackendMode>>,
    daemon_manager: Arc<DaemonManager>,
    daemon_client: Arc<DaemonClient>,
    embedded: Arc<RwLock<Option<EmbeddedBackend>>>,
    app: Arc<RwLock<Option<AppHandle>>>,
}

impl Backend {
    /// Create a new backend (starts in embedded mode)
    pub fn new() -> Self {
        let manager_config = DaemonManagerConfig::default();
        let client_config = DaemonClientConfig {
            url: format!("ws://{}:{}", manager_config.host, manager_config.port),
            ..Default::default()
        };
        
        Self {
            mode: Arc::new(RwLock::new(BackendMode::Embedded)),
            daemon_manager: Arc::new(DaemonManager::new(manager_config)),
            daemon_client: Arc::new(DaemonClient::new(client_config)),
            embedded: Arc::new(RwLock::new(None)),
            app: Arc::new(RwLock::new(None)),
        }
    }
    
    /// Set the embedded backend (must be called before init)
    pub async fn set_embedded(&self, embedded: EmbeddedBackend) {
        *self.embedded.write().await = Some(embedded);
    }
    
    /// Set the app handle (required for sidecar and event emission)
    pub async fn set_app(&self, app: AppHandle) {
        *self.app.write().await = Some(app);
    }
    
    /// Initialize the backend:
    /// 1. Start the daemon sidecar
    /// 2. Connect the client
    /// 3. Fall back to embedded if daemon fails
    pub async fn init(&self, app: &AppHandle) -> BackendMode {
        info!("Initializing backend...");
        
        // Store app handle
        *self.app.write().await = Some(app.clone());
        
        // Step 1: Start the daemon sidecar
        match self.daemon_manager.start(app).await {
            Ok(()) => {
                info!("Daemon sidecar started");
                
                // Step 2: Connect the client
                let mode = self.daemon_client.connect_or_embed().await;
                
                match mode {
                    ConnectionMode::Daemon => {
                        info!("Backend: daemon mode (sidecar)");
                        *self.mode.write().await = BackendMode::Daemon;
                        
                        // Start event forwarding
                        self.start_event_forwarding(app.clone());
                        
                        // Start health monitoring
                        self.daemon_manager.clone().start_health_monitor(app.clone());
                        
                        BackendMode::Daemon
                    }
                    ConnectionMode::Embedded => {
                        warn!("Daemon started but client connection failed, falling back to embedded");
                        *self.mode.write().await = BackendMode::Embedded;
                        BackendMode::Embedded
                    }
                }
            }
            Err(e) => {
                error!("Failed to start daemon sidecar: {}", e);
                info!("Backend: embedded mode (fallback)");
                *self.mode.write().await = BackendMode::Embedded;
                BackendMode::Embedded
            }
        }
    }
    
    /// Shutdown the backend (stop daemon if running)
    pub async fn shutdown(&self) {
        info!("Shutting down backend...");
        
        // Disconnect client
        self.daemon_client.disconnect();
        
        // Stop daemon
        if let Err(e) = self.daemon_manager.stop().await {
            warn!("Error stopping daemon: {}", e);
        }
        
        *self.mode.write().await = BackendMode::Embedded;
    }
    
    /// Get current mode
    pub async fn mode(&self) -> BackendMode {
        *self.mode.read().await
    }
    
    /// Get backend status
    pub async fn status(&self) -> BackendStatus {
        let mode = *self.mode.read().await;
        let daemon_state = self.daemon_manager.state().await;
        let client_status = self.daemon_client.status().await;
        
        BackendStatus {
            mode,
            connected: mode == BackendMode::Daemon && daemon_state == DaemonState::Running,
            daemon_url: if mode == BackendMode::Daemon {
                Some(self.daemon_manager.ws_url())
            } else {
                None
            },
            daemon_state: format!("{:?}", daemon_state).to_lowercase(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
    
    /// Check if using daemon mode
    pub async fn is_daemon_mode(&self) -> bool {
        *self.mode.read().await == BackendMode::Daemon
    }
    
    /// Send a request to daemon (only works in daemon mode)
    pub async fn daemon_request(&self, action: Value) -> Result<Value, String> {
        if !self.is_daemon_mode().await {
            return Err("Not in daemon mode".to_string());
        }
        self.daemon_client.request(action).await
    }
    
    /// Start forwarding daemon events to Tauri
    fn start_event_forwarding(&self, app: AppHandle) {
        let mut events = self.daemon_client.subscribe_events();
        
        tokio::spawn(async move {
            while let Ok(event) = events.recv().await {
                match &event {
                    DaemonEvent::MessageDelta { session_id, delta, .. } => {
                        let _ = app.emit("stream-chunk", serde_json::json!({
                            "session_id": session_id,
                            "chunk": delta,
                            "done": false,
                        }));
                    }
                    DaemonEvent::MessageEnd { session_id, .. } => {
                        let _ = app.emit("stream-chunk", serde_json::json!({
                            "session_id": session_id,
                            "chunk": "",
                            "done": true,
                        }));
                    }
                    DaemonEvent::ToolStart { session_id, call_id, name, .. } => {
                        let _ = app.emit("tool-call", serde_json::json!({
                            "session_id": session_id,
                            "tool_call": {
                                "id": call_id,
                                "name": name,
                            },
                            "status": "started",
                        }));
                    }
                    DaemonEvent::ToolEnd { session_id, call_id, output, success, .. } => {
                        let _ = app.emit("tool-call", serde_json::json!({
                            "session_id": session_id,
                            "tool_call": {
                                "id": call_id,
                                "output": output,
                                "success": success,
                            },
                            "status": if *success { "completed" } else { "error" },
                        }));
                    }
                    DaemonEvent::Error { code, message, .. } => {
                        let _ = app.emit("error", serde_json::json!({
                            "code": code,
                            "message": message,
                        }));
                    }
                    _ => {}
                }
            }
        });
    }
    
    // =========================================================================
    // High-level API (works in both modes)
    // =========================================================================
    
    /// Send a chat message
    /// In daemon mode: forwards to daemon
    /// In embedded mode: caller should use embedded agent
    pub async fn chat_send(&self, session_id: &str, content: &str) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.chat_send(session_id, content).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// List sessions
    pub async fn sessions_list(&self) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.sessions_list().await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Create a session
    pub async fn session_create(&self, name: Option<&str>) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.session_create(name).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Get session history
    pub async fn session_history(&self, session_id: &str, limit: Option<usize>) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.session_history(session_id, limit).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Get system status
    pub async fn system_status(&self) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.system_status().await
        } else {
            Ok(serde_json::json!({
                "mode": "embedded",
                "version": env!("CARGO_PKG_VERSION"),
            }))
        }
    }
}

impl Default for Backend {
    fn default() -> Self {
        Self::new()
    }
}
