//! Backend abstraction layer
//!
//! Provides a unified interface that can use either:
//! - Daemon mode: Connect to nanna-daemon via WebSocket
//! - Embedded mode: Run agent directly in the GUI process
//!
//! The GUI commands use this layer, which automatically selects the right backend.

use crate::daemon_client::{ConnectionMode, DaemonClient, DaemonClientConfig, DaemonEvent};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Backend mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendMode {
    /// Connected to external daemon
    Daemon,
    /// Running embedded agent
    Embedded,
}

/// Backend status for frontend
#[derive(Debug, Clone, Serialize)]
pub struct BackendStatus {
    pub mode: BackendMode,
    pub connected: bool,
    pub daemon_url: Option<String>,
    pub version: String,
}

/// The unified backend interface
pub struct Backend {
    mode: Arc<RwLock<BackendMode>>,
    daemon_client: Arc<DaemonClient>,
    app: Option<AppHandle>,
}

impl Backend {
    /// Create a new backend (starts in embedded mode)
    pub fn new() -> Self {
        let config = DaemonClientConfig::default();
        Self {
            mode: Arc::new(RwLock::new(BackendMode::Embedded)),
            daemon_client: Arc::new(DaemonClient::new(config)),
            app: None,
        }
    }
    
    /// Set the app handle for event emission
    pub fn with_app(mut self, app: AppHandle) -> Self {
        self.app = Some(app);
        self
    }
    
    /// Initialize the backend - try daemon, fall back to embedded
    pub async fn init(&self) -> BackendMode {
        info!("Initializing backend...");
        
        // Try to connect to daemon
        let mode = self.daemon_client.connect_or_embed().await;
        
        let backend_mode = match mode {
            ConnectionMode::Daemon => {
                info!("Backend: daemon mode");
                BackendMode::Daemon
            }
            ConnectionMode::Embedded => {
                info!("Backend: embedded mode");
                BackendMode::Embedded
            }
        };
        
        *self.mode.write().await = backend_mode;
        
        // Start event forwarding if in daemon mode
        if backend_mode == BackendMode::Daemon {
            self.start_event_forwarding();
        }
        
        backend_mode
    }
    
    /// Get current mode
    pub async fn mode(&self) -> BackendMode {
        *self.mode.read().await
    }
    
    /// Get backend status
    pub async fn status(&self) -> BackendStatus {
        let mode = *self.mode.read().await;
        let client_status = self.daemon_client.status().await;
        
        BackendStatus {
            mode,
            connected: mode == BackendMode::Daemon,
            daemon_url: if mode == BackendMode::Daemon {
                Some(client_status.daemon_url)
            } else {
                None
            },
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
    fn start_event_forwarding(&self) {
        let Some(ref app) = self.app else {
            warn!("No app handle for event forwarding");
            return;
        };
        
        let mut events = self.daemon_client.subscribe_events();
        let app = app.clone();
        
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
                    DaemonEvent::MessageEnd { session_id, content, .. } => {
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
            // In embedded mode, return an indicator that caller should use embedded path
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
