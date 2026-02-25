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
use crate::AppState;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
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
    /// Flag to prevent concurrent init attempts
    initializing: Arc<RwLock<bool>>,
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
            initializing: Arc::new(RwLock::new(false)),
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
        // Check if already initialized
        let current_mode = *self.mode.read().await;
        if current_mode == BackendMode::Daemon {
            info!("Backend already initialized in daemon mode");
            return BackendMode::Daemon;
        }

        // Check if initialization is already in progress
        {
            let mut initializing = self.initializing.write().await;
            if *initializing {
                info!("Backend initialization already in progress, waiting...");
                drop(initializing);
                // Wait for initialization to complete
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    if !*self.initializing.read().await {
                        return *self.mode.read().await;
                    }
                }
            }
            *initializing = true;
        }

        info!("Initializing backend...");
        
        // Store app handle
        *self.app.write().await = Some(app.clone());
        
        // Step 1: Start the daemon sidecar
        let result = match self.daemon_manager.start(app).await {
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
        };

        // Mark initialization as complete
        *self.initializing.write().await = false;
        result
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
        let _client_status = self.daemon_client.status().await;
        
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
                    DaemonEvent::ThinkingDelta { session_id, delta, .. } => {
                        let _ = app.emit("thinking-chunk", serde_json::json!({
                            "session_id": session_id,
                            "delta": delta,
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
                    DaemonEvent::ModelSwitch { model, reason } => {
                        // Update AppState's active_model so get_model_status stays in sync
                        if let Some(state) = app.try_state::<Arc<RwLock<AppState>>>() {
                            let state_guard = state.read().await;
                            let mut active = state_guard.active_model.write().await;
                            *active = model.clone();
                        }
                        let _ = app.emit("model-status", serde_json::json!({
                            "active_model": model,
                            "fallback_reason": reason,
                            "rate_limited_models": [],
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
    
    /// Cancel an active chat
    pub async fn chat_cancel(&self, session_id: &str) -> Result<bool, String> {
        if self.is_daemon_mode().await {
            match self.daemon_client.chat_cancel(session_id).await {
                Ok(val) => Ok(val.get("status").and_then(|s| s.as_str()) == Some("cancelled")),
                Err(e) => Err(e),
            }
        } else {
            Ok(false)
        }
    }

    /// Get daemon logs
    pub async fn get_logs(&self, limit: Option<usize>) -> Result<Vec<Value>, String> {
        if self.is_daemon_mode().await {
            match self.daemon_client.system_logs(limit).await {
                Ok(val) => {
                    let logs = val.get("logs")
                        .and_then(|l| l.as_array())
                        .cloned()
                        .unwrap_or_default();
                    Ok(logs)
                }
                Err(e) => Err(e),
            }
        } else {
            Ok(vec![])
        }
    }

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
    
    // =========================================================================
    // Session management (additional methods)
    // =========================================================================
    
    /// Delete a session
    pub async fn session_delete(&self, session_id: &str) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.session_delete(session_id).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }

    /// Delete all sessions
    pub async fn sessions_delete_all(&self) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.sessions_delete_all().await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }

    /// Rename a session
    pub async fn session_rename(&self, session_id: &str, name: &str) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.session_rename(session_id, name).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Get session run state (in-flight streaming text, active tools)
    pub async fn session_get_run_state(&self, session_id: &str) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.session_get_run_state(session_id).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }

    /// Clear session history
    pub async fn session_clear(&self, session_id: &str) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.session_clear(session_id).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    // =========================================================================
    // Memory operations
    // =========================================================================
    
    /// List all memories
    pub async fn memory_list(&self, scope: Option<&str>) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.memory_list(scope).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }

    /// Search memories
    pub async fn memory_search(&self, query: &str, limit: Option<usize>) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.memory_search(query, limit).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }

    /// Get a specific memory
    pub async fn memory_get(&self, id: &str) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.memory_get(id).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Create a memory
    pub async fn memory_create(&self, content: &str, tags: Option<Vec<String>>, importance: Option<u8>) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.memory_create(content, tags, importance).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Update a memory
    pub async fn memory_update(&self, id: &str, content: Option<&str>, tags: Option<Vec<String>>) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.memory_update(id, content, tags).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Delete a memory
    pub async fn memory_delete(&self, id: &str) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.memory_delete(id).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }

    /// Clear all memories
    pub async fn memory_clear(&self) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.memory_clear().await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }

    /// Get memory stats
    pub async fn memory_stats(&self) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.memory_stats().await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Trigger memory consolidation
    pub async fn memory_consolidate(&self) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.memory_consolidate().await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    // =========================================================================
    // Scheduler operations
    // =========================================================================
    
    /// List scheduled jobs
    pub async fn scheduler_list(&self) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.scheduler_list().await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Get job details
    pub async fn scheduler_get(&self, id: &str) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.scheduler_get(id).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Add a cron job
    pub async fn scheduler_add(&self, schedule: &str, task: &str, name: Option<&str>) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.scheduler_add(schedule, task, name).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Update a job
    pub async fn scheduler_update(&self, id: &str, schedule: Option<&str>, task: Option<&str>, enabled: Option<bool>) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.scheduler_update(id, schedule, task, enabled).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Remove a job
    pub async fn scheduler_remove(&self, id: &str) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.scheduler_remove(id).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Run a job immediately
    pub async fn scheduler_run_now(&self, id: &str) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.scheduler_run_now(id).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Get job history
    pub async fn scheduler_history(&self, id: &str, limit: Option<usize>) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.scheduler_history(id, limit).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    // =========================================================================
    // Tool operations
    // =========================================================================
    
    /// List all tools
    pub async fn tool_list(&self) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.tool_list().await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Execute a tool
    pub async fn tool_execute(&self, name: &str, input: Value) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.tool_execute(name, input).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Create a user tool
    pub async fn tool_create(&self, name: &str, description: &str, code: &str, needs_shell: Option<bool>) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.tool_create(name, description, code, needs_shell).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Update a user tool
    pub async fn tool_update(&self, name: &str, description: Option<&str>, code: Option<&str>, needs_shell: Option<bool>) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.tool_update(name, description, code, needs_shell).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Delete a user tool
    pub async fn tool_delete(&self, name: &str) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.tool_delete(name).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Test a user tool
    pub async fn tool_test(&self, code: &str, input: Value) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.tool_test(code, input).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// List user tools
    pub async fn tool_list_user(&self) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.tool_list_user().await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }

    /// Get tool source code
    pub async fn tool_get_source(&self, name: &str) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.tool_get_source(name).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    // =========================================================================
    // Config operations
    // =========================================================================
    
    /// Get config (full or by path)
    pub async fn config_get(&self, path: Option<&str>) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.config_get(path).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Set config value
    pub async fn config_set(&self, path: &str, value: Value) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.config_set(path, value).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Reset config
    pub async fn config_reset(&self, path: Option<&str>) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.config_reset(path).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Reload config from disk
    pub async fn config_reload(&self) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.config_reload().await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Export config
    pub async fn config_export(&self) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.config_export().await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Import config
    pub async fn config_import(&self, config: Value) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.config_import(config).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    // =========================================================================
    // Workspace operations
    // =========================================================================
    
    /// List workspaces
    pub async fn workspace_list(&self) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.workspace_list().await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Get workspace details
    pub async fn workspace_get(&self, id: &str) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.workspace_get(id).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Open/register a workspace
    pub async fn workspace_open(&self, path: &str) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.workspace_open(path).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Close/unregister a workspace
    pub async fn workspace_close(&self, id: &str) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.workspace_close(id).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Set active workspace
    pub async fn workspace_set_active(&self, id: &str) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.workspace_set_active(id).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Clear active workspace (global mode)
    pub async fn workspace_clear_active(&self) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.workspace_clear_active().await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Reload workspace context
    pub async fn workspace_reload(&self, id: &str) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.workspace_reload(id).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Get workspace context
    pub async fn workspace_get_context(&self, id: &str) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.workspace_get_context(id).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Update workspace context file
    pub async fn workspace_update_context(&self, id: &str, file: &str, content: &str) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.workspace_update_context(id, file, content).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    // =========================================================================
    // Channel operations
    // =========================================================================
    
    /// List channels
    pub async fn channel_list(&self) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.channel_list().await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
    
    /// Get channel status
    pub async fn channel_status(&self, id: Option<&str>) -> Result<Value, String> {
        if self.is_daemon_mode().await {
            self.daemon_client.channel_status(id).await
        } else {
            Err("EMBEDDED_MODE".to_string())
        }
    }
}

impl Default for Backend {
    fn default() -> Self {
        Self::new()
    }
}
