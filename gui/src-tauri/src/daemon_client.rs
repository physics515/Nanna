//! Daemon Client Integration
//!
//! Connects the Tauri GUI to the nanna-daemon via WebSocket.
//! Falls back to embedded mode if daemon is not available.

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
// use tauri::{AppHandle, Emitter};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc, oneshot, RwLock};
use tokio_tungstenite::{connect_async_with_config, tungstenite::{protocol::WebSocketConfig, Message}, MaybeTlsStream, WebSocketStream};
use tracing::{debug, error, info, warn};

/// Maximum WebSocket message size (128 MB) — must match the daemon's limit.
const WS_MAX_MESSAGE_SIZE: usize = 128 * 1024 * 1024;

fn ws_config() -> WebSocketConfig {
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(WS_MAX_MESSAGE_SIZE);
    config.max_frame_size = Some(WS_MAX_MESSAGE_SIZE);
    config
}

/// Health-check polling constants for long-running requests
const HEALTH_POLL_INTERVAL: Duration = Duration::from_secs(30);
const HEALTH_PING_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_MISSED_PINGS: u32 = 3;
/// Grace period after agent finishes before giving up on the IPC response.
/// Needs to be long enough for post-processing (tool stats, DB writes,
/// memory extraction/saving, session persistence) to complete.
const IDLE_GRACE_PERIOD: Duration = Duration::from_secs(60);

/// Connection mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionMode {
    /// Connected to daemon via WebSocket
    Daemon,
    /// Running embedded (no daemon)
    Embedded,
}

/// Connection state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
}

/// Daemon client configuration
#[derive(Debug, Clone)]
pub struct DaemonClientConfig {
    pub url: String,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub auto_reconnect: bool,
}

impl Default for DaemonClientConfig {
    fn default() -> Self {
        Self {
            url: "ws://127.0.0.1:5149".to_string(),
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(300), // 5 minutes for large content summarization (many chunks)
            auto_reconnect: true,
        }
    }
}

/// IPC Request (mirrors nanna-daemon protocol)
#[derive(Debug, Clone, Serialize)]
pub struct Request {
    pub id: String,
    pub action: Value,
}

/// IPC Response
#[derive(Debug, Clone, Deserialize)]
pub struct Response {
    pub id: String,
    pub result: ResponseResult,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ResponseResult {
    Success { data: Value },
    Error { code: String, message: String },
}

impl Response {
    pub fn is_error(&self) -> bool {
        matches!(self.result, ResponseResult::Error { .. })
    }
    
    pub fn data(&self) -> Option<&Value> {
        match &self.result {
            ResponseResult::Success { data } => Some(data),
            ResponseResult::Error { .. } => None,
        }
    }
    
    pub fn into_data(self) -> Result<Value, String> {
        match self.result {
            ResponseResult::Success { data } => Ok(data),
            ResponseResult::Error { message, .. } => Err(message),
        }
    }
}

/// IPC Event (from daemon)
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum DaemonEvent {
    MessageStart { session_id: String, message_id: String },
    MessageDelta { session_id: String, message_id: String, delta: String },
    MessageEnd { session_id: String, message_id: String, content: String },
    ThinkingDelta { session_id: String, delta: String },
    ModelSwitch { model: String, reason: Option<String> },
    ToolStart { session_id: String, call_id: String, name: String, #[serde(default)] input: Option<serde_json::Value>, #[serde(default)] model: Option<String>, #[serde(default)] tokens: Option<u64>, #[serde(default)] total_tokens: Option<u64> },
    ToolEnd { session_id: String, call_id: String, output: String, success: bool, #[serde(default)] duration_ms: Option<u64>, #[serde(default)] data: Option<serde_json::Value> },
    Error { code: String, message: String, #[serde(default)] session_id: Option<String> },
    ContextUsage { session_id: String, used: u64, window: u64 },
    Connected { client_id: String },
    Disconnected { client_id: String },
    TaskRunStarted { scope: String, #[serde(default)] scope_id: Option<String>, goal: String },
    TaskRunProgress { scope: String, #[serde(default)] scope_id: Option<String>, #[serde(default)] task_id: Option<i64>, kind: String, detail: serde_json::Value },
    TaskRunCompleted { scope: String, #[serde(default)] scope_id: Option<String>, report: serde_json::Value },
}

/// Pending request waiting for response
struct PendingRequest {
    tx: oneshot::Sender<Result<Value, String>>,
}

/// Daemon client for GUI
pub struct DaemonClient {
    config: DaemonClientConfig,
    state: Arc<RwLock<ConnectionState>>,
    mode: Arc<RwLock<ConnectionMode>>,
    msg_tx: Arc<RwLock<Option<mpsc::Sender<Message>>>>,
    pending: Arc<RwLock<HashMap<String, PendingRequest>>>,
    event_tx: broadcast::Sender<DaemonEvent>,
    shutdown_tx: broadcast::Sender<()>,
    /// Tracks whether a reconnection loop is running
    reconnecting: Arc<RwLock<bool>>,
}

impl DaemonClient {
    /// Create a new daemon client
    pub fn new(config: DaemonClientConfig) -> Self {
        let (event_tx, _) = broadcast::channel::<DaemonEvent>(100);
        let (shutdown_tx, _) = broadcast::channel::<()>(1);

        Self {
            config,
            state: Arc::new(RwLock::new(ConnectionState::Disconnected)),
            mode: Arc::new(RwLock::new(ConnectionMode::Embedded)),
            msg_tx: Arc::new(RwLock::new(None)),
            pending: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
            shutdown_tx,
            reconnecting: Arc::new(RwLock::new(false)),
        }
    }
    
    /// Try to connect to the daemon
    pub async fn connect(&self) -> Result<(), String> {
        *self.state.write().await = ConnectionState::Connecting;
        
        info!("Attempting to connect to daemon at {}", self.config.url);
        
        let connect_future = connect_async_with_config(&self.config.url, Some(ws_config()), false);
        
        match tokio::time::timeout(self.config.connect_timeout, connect_future).await {
            Ok(Ok((ws, _))) => {
                *self.state.write().await = ConnectionState::Connected;
                *self.mode.write().await = ConnectionMode::Daemon;
                
                info!("Connected to daemon");
                
                // Start message handler
                self.spawn_handler(ws).await;
                
                Ok(())
            }
            Ok(Err(e)) => {
                *self.state.write().await = ConnectionState::Disconnected;
                warn!("Failed to connect to daemon: {}", e);
                Err(format!("Connection failed: {}", e))
            }
            Err(_) => {
                *self.state.write().await = ConnectionState::Disconnected;
                warn!("Connection to daemon timed out");
                Err("Connection timeout".to_string())
            }
        }
    }
    
    /// Try to connect, fall back to embedded mode if daemon not available
    pub async fn connect_or_embed(&self) -> ConnectionMode {
        match self.connect().await {
            Ok(()) => {
                info!("Running in daemon mode");
                ConnectionMode::Daemon
            }
            Err(e) => {
                info!("Daemon not available ({}), running in embedded mode", e);
                *self.mode.write().await = ConnectionMode::Embedded;
                ConnectionMode::Embedded
            }
        }
    }
    
    async fn spawn_handler(&self, ws: WebSocketStream<MaybeTlsStream<TcpStream>>) {
        let (mut ws_tx, mut ws_rx) = ws.split();
        let (msg_tx, mut msg_rx) = mpsc::channel::<Message>(100);

        *self.msg_tx.write().await = Some(msg_tx);

        let pending = self.pending.clone();
        let event_tx = self.event_tx.clone();
        let state = self.state.clone();
        let mode = self.mode.clone();
        let reconnecting = self.reconnecting.clone();
        let config = self.config.clone();
        let msg_tx_holder = self.msg_tx.clone();
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        let shutdown_tx = self.shutdown_tx.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Send outgoing messages
                    Some(msg) = msg_rx.recv() => {
                        if ws_tx.send(msg).await.is_err() {
                            break;
                        }
                    }

                    // Receive incoming messages
                    Some(msg) = ws_rx.next() => {
                        match msg {
                            Ok(Message::Text(text)) => {
                                Self::handle_message_static(&text, &pending, &event_tx).await;
                            }
                            Ok(Message::Ping(data)) => {
                                let _ = ws_tx.send(Message::Pong(data)).await;
                            }
                            Ok(Message::Close(_)) => {
                                debug!("Server sent close");
                                break;
                            }
                            Err(e) => {
                                error!("WebSocket error: {}", e);
                                break;
                            }
                            _ => {}
                        }
                    }

                    // Shutdown signal
                    _ = shutdown_rx.recv() => {
                        debug!("Shutdown signal received");
                        let _ = ws_tx.close().await;
                        break;
                    }
                }
            }

            *state.write().await = ConnectionState::Disconnected;
            info!("Disconnected from daemon");

            // Fail all pending requests
            {
                let mut pending = pending.write().await;
                for (_, req) in pending.drain() {
                    let _ = req.tx.send(Err("Disconnected".to_string()));
                }
            }

            // Start reconnection loop if auto_reconnect is enabled
            if config.auto_reconnect {
                // Check if reconnection loop is already running
                let already_reconnecting = {
                    let mut flag = reconnecting.write().await;
                    if *flag {
                        true
                    } else {
                        *flag = true;
                        false
                    }
                };

                if !already_reconnecting {
                    Self::start_reconnect_loop(
                        config,
                        state,
                        mode,
                        msg_tx_holder,
                        pending,
                        event_tx,
                        shutdown_tx,
                        reconnecting,
                    );
                }
            } else {
                *mode.write().await = ConnectionMode::Embedded;
                info!("Auto-reconnect disabled, falling back to embedded mode");
            }
        });
    }

    /// Start a background task to periodically attempt reconnection
    fn start_reconnect_loop(
        config: DaemonClientConfig,
        state: Arc<RwLock<ConnectionState>>,
        mode: Arc<RwLock<ConnectionMode>>,
        msg_tx: Arc<RwLock<Option<mpsc::Sender<Message>>>>,
        pending: Arc<RwLock<HashMap<String, PendingRequest>>>,
        event_tx: broadcast::Sender<DaemonEvent>,
        shutdown_tx: broadcast::Sender<()>,
        reconnecting: Arc<RwLock<bool>>,
    ) {
        tokio::spawn(async move {
            let reconnect_interval = Duration::from_secs(10);
            let max_attempts = 30; // Give up after ~5 minutes
            let mut attempt = 0;
            let mut shutdown_rx = shutdown_tx.subscribe();

            info!("Starting reconnection loop (interval: {:?})", reconnect_interval);

            loop {
                attempt += 1;

                // Wait before attempting reconnection
                tokio::select! {
                    _ = tokio::time::sleep(reconnect_interval) => {}
                    _ = shutdown_rx.recv() => {
                        info!("Reconnection loop stopped by shutdown signal");
                        break;
                    }
                }

                // Check if we should give up
                if attempt > max_attempts {
                    warn!("Max reconnection attempts ({}) reached, giving up", max_attempts);
                    *mode.write().await = ConnectionMode::Embedded;
                    break;
                }

                *state.write().await = ConnectionState::Reconnecting;
                info!("Reconnection attempt {}/{}", attempt, max_attempts);

                // Try to connect
                let connect_future = connect_async_with_config(&config.url, Some(ws_config()), false);
                match tokio::time::timeout(config.connect_timeout, connect_future).await {
                    Ok(Ok((ws, _))) => {
                        info!("Reconnected to daemon successfully");
                        *state.write().await = ConnectionState::Connected;
                        *mode.write().await = ConnectionMode::Daemon;

                        // Spawn new handler for this connection
                        let (mut ws_tx, mut ws_rx) = ws.split();
                        let (new_msg_tx, mut msg_rx) = mpsc::channel::<Message>(100);
                        *msg_tx.write().await = Some(new_msg_tx);

                        let pending_clone = pending.clone();
                        let event_tx_clone = event_tx.clone();
                        let state_clone = state.clone();
                        let mode_clone = mode.clone();
                        let reconnecting_clone = reconnecting.clone();
                        let config_clone = config.clone();
                        let msg_tx_clone = msg_tx.clone();
                        let shutdown_tx_clone = shutdown_tx.clone();
                        let mut handler_shutdown_rx = shutdown_tx.subscribe();

                        tokio::spawn(async move {
                            loop {
                                tokio::select! {
                                    Some(msg) = msg_rx.recv() => {
                                        if ws_tx.send(msg).await.is_err() {
                                            break;
                                        }
                                    }
                                    Some(msg) = ws_rx.next() => {
                                        match msg {
                                            Ok(Message::Text(text)) => {
                                                Self::handle_message_static(&text, &pending_clone, &event_tx_clone).await;
                                            }
                                            Ok(Message::Ping(data)) => {
                                                let _ = ws_tx.send(Message::Pong(data)).await;
                                            }
                                            Ok(Message::Close(_)) => {
                                                debug!("Server sent close");
                                                break;
                                            }
                                            Err(e) => {
                                                error!("WebSocket error: {}", e);
                                                break;
                                            }
                                            _ => {}
                                        }
                                    }
                                    _ = handler_shutdown_rx.recv() => {
                                        debug!("Shutdown signal received");
                                        let _ = ws_tx.close().await;
                                        break;
                                    }
                                }
                            }

                            *state_clone.write().await = ConnectionState::Disconnected;
                            info!("Disconnected from daemon again");

                            // Fail pending requests
                            {
                                let mut pending = pending_clone.write().await;
                                for (_, req) in pending.drain() {
                                    let _ = req.tx.send(Err("Disconnected".to_string()));
                                }
                            }

                            // Start another reconnection loop
                            if config_clone.auto_reconnect {
                                let already = {
                                    let mut flag = reconnecting_clone.write().await;
                                    if *flag {
                                        true
                                    } else {
                                        *flag = true;
                                        false
                                    }
                                };

                                if !already {
                                    Self::start_reconnect_loop(
                                        config_clone,
                                        state_clone,
                                        mode_clone,
                                        msg_tx_clone,
                                        pending_clone,
                                        event_tx_clone,
                                        shutdown_tx_clone,
                                        reconnecting_clone,
                                    );
                                }
                            } else {
                                *mode_clone.write().await = ConnectionMode::Embedded;
                            }
                        });

                        // Exit the reconnection loop - handler will manage future disconnects
                        break;
                    }
                    Ok(Err(e)) => {
                        debug!("Reconnection attempt {} failed: {}", attempt, e);
                    }
                    Err(_) => {
                        debug!("Reconnection attempt {} timed out", attempt);
                    }
                }
            }

            // Mark reconnection loop as done
            *reconnecting.write().await = false;
        });
    }
    
    async fn handle_message_static(
        text: &str,
        pending: &Arc<RwLock<HashMap<String, PendingRequest>>>,
        event_tx: &broadcast::Sender<DaemonEvent>,
    ) {
        // Try to parse as Response first
        if let Ok(response) = serde_json::from_str::<Response>(text) {
            let mut pending = pending.write().await;
            if let Some(req) = pending.remove(&response.id) {
                let result = response.into_data();
                let _ = req.tx.send(result);
            } else {
                warn!("Received response for unknown request: {}", response.id);
            }
            return;
        }
        
        // Try to parse as Event
        if let Ok(event) = serde_json::from_str::<DaemonEvent>(text) {
            let _ = event_tx.send(event);
            return;
        }
        
        warn!("Unknown message format: {}", text.chars().take(100).collect::<String>());
    }
    
    /// Get current connection mode
    pub async fn mode(&self) -> ConnectionMode {
        *self.mode.read().await
    }
    
    /// Get current connection state
    pub async fn state(&self) -> ConnectionState {
        *self.state.read().await
    }
    
    /// Check if connected to daemon
    pub async fn is_connected(&self) -> bool {
        *self.state.read().await == ConnectionState::Connected
    }
    
    /// Check if in embedded mode
    pub async fn is_embedded(&self) -> bool {
        *self.mode.read().await == ConnectionMode::Embedded
    }
    
    /// Subscribe to daemon events
    pub fn subscribe_events(&self) -> broadcast::Receiver<DaemonEvent> {
        self.event_tx.subscribe()
    }

    /// Sender side of the event bus. Used in embedded mode to inject events
    /// from the in-process `AgentService` so the same subscribers (the single
    /// Tauri event-forwarding task) see them exactly like daemon events.
    pub fn event_sender(&self) -> broadcast::Sender<DaemonEvent> {
        self.event_tx.clone()
    }
    
    /// Send a request to the daemon with the default timeout
    pub async fn request(&self, action: Value) -> Result<Value, String> {
        self.request_with_timeout(action, self.config.request_timeout).await
    }

    /// Send a request to the daemon with a specific timeout
    async fn request_with_timeout(&self, action: Value, timeout: Duration) -> Result<Value, String> {
        if *self.mode.read().await != ConnectionMode::Daemon {
            return Err("Not connected to daemon".to_string());
        }

        let msg_tx = {
            let guard = self.msg_tx.read().await;
            guard.clone().ok_or_else(|| "No message sender".to_string())?
        };

        let id = uuid::Uuid::new_v4().to_string();
        let request = Request { id: id.clone(), action };

        let json = serde_json::to_string(&request)
            .map_err(|e| format!("Serialization error: {}", e))?;

        // Create response channel
        let (tx, rx) = oneshot::channel();

        // Register pending request
        {
            let mut pending = self.pending.write().await;
            pending.insert(id.clone(), PendingRequest { tx });
        }

        // Send request
        msg_tx.send(Message::Text(json.into())).await
            .map_err(|e| format!("Send error: {}", e))?;

        // Wait for response with timeout
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                let mut pending = self.pending.write().await;
                pending.remove(&id);
                Err("Response channel closed".to_string())
            }
            Err(_) => {
                let mut pending = self.pending.write().await;
                pending.remove(&id);
                Err("Request timeout".to_string())
            }
        }
    }

    /// Send a request with health-check polling for long-running operations.
    ///
    /// Instead of a hard timeout, periodically pings `get_run_state` to check
    /// if the daemon is still actively processing. Only times out if the daemon
    /// becomes unresponsive (multiple missed pings) or reports it stopped running.
    async fn request_with_health_polling(
        &self,
        action: Value,
        session_id: &str,
    ) -> Result<Value, String> {
        if *self.mode.read().await != ConnectionMode::Daemon {
            return Err("Not connected to daemon".to_string());
        }

        let msg_tx = {
            let guard = self.msg_tx.read().await;
            guard.clone().ok_or_else(|| "No message sender".to_string())?
        };

        let id = uuid::Uuid::new_v4().to_string();
        let request = Request {
            id: id.clone(),
            action,
        };

        let json = serde_json::to_string(&request)
            .map_err(|e| format!("Serialization error: {}", e))?;

        // Create response channel
        let (tx, rx) = oneshot::channel();

        // Register pending request
        {
            let mut pending = self.pending.write().await;
            pending.insert(id.clone(), PendingRequest { tx });
        }

        // Send request
        msg_tx
            .send(Message::Text(json.into()))
            .await
            .map_err(|e| format!("Send error: {}", e))?;

        // Health-check polling loop
        let mut rx = rx;
        let mut missed_pings: u32 = 0;

        loop {
            tokio::select! {
                // Branch A: response arrives
                result = &mut rx => {
                    return match result {
                        Ok(result) => result,
                        Err(_) => {
                            let mut pending = self.pending.write().await;
                            pending.remove(&id);
                            Err("Response channel closed".to_string())
                        }
                    };
                }

                // Branch B: health-check interval fires
                _ = tokio::time::sleep(HEALTH_POLL_INTERVAL) => {
                    let ping_payload = serde_json::json!({
                        "type": "session",
                        "action": "get_run_state",
                        "id": session_id
                    });

                    match self.request_with_timeout(ping_payload, HEALTH_PING_TIMEOUT).await {
                        Ok(state) => {
                            missed_pings = 0;
                            let is_running = state
                                .get("is_running")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);

                            if !is_running {
                                // Daemon says it's not running — give a grace period
                                // for the response to arrive through the normal channel.
                                // Post-processing (tool stats, DB writes, memory saves)
                                // can take a while after the agent loop finishes.
                                debug!(
                                    session_id = session_id,
                                    "Run state reports not running, waiting grace period ({:?})",
                                    IDLE_GRACE_PERIOD,
                                );
                                match tokio::time::timeout(IDLE_GRACE_PERIOD, &mut rx).await {
                                    Ok(Ok(result)) => return result,
                                    Ok(Err(_)) => {
                                        let mut pending = self.pending.write().await;
                                        pending.remove(&id);
                                        return Err("Response channel closed".to_string());
                                    }
                                    Err(_) => {
                                        // Grace period expired without receiving the IPC response.
                                        // The agent DID finish (is_running=false), so the response
                                        // was likely delivered via streaming events already.
                                        // Return a synthetic success rather than an error.
                                        warn!(
                                            session_id = session_id,
                                            "Grace period expired — IPC response not received, \
                                             but agent completed. Returning synthetic response."
                                        );
                                        let mut pending = self.pending.write().await;
                                        pending.remove(&id);
                                        // Return empty success — the GUI already has the streamed content
                                        return Ok(serde_json::json!({
                                            "status": "success",
                                            "content": "",
                                            "tool_calls": [],
                                            "usage": { "input_tokens": 0, "output_tokens": 0 },
                                            "_synthetic": true
                                        }));
                                    }
                                }
                            }

                            debug!(
                                session_id = session_id,
                                "Health ping OK — agent still running"
                            );
                        }
                        Err(e) => {
                            missed_pings += 1;
                            warn!(
                                session_id = session_id,
                                missed = missed_pings,
                                max = MAX_MISSED_PINGS,
                                error = %e,
                                "Health ping failed"
                            );
                            if missed_pings >= MAX_MISSED_PINGS {
                                let mut pending = self.pending.write().await;
                                pending.remove(&id);
                                return Err(format!(
                                    "Daemon unresponsive ({} missed health pings)",
                                    missed_pings
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    
    /// Disconnect from daemon
    pub fn disconnect(&self) {
        let _ = self.shutdown_tx.send(());
    }
    
    // =========================================================================
    // Convenience methods
    // =========================================================================
    
    /// Send a chat message (uses health-check polling instead of hard timeout)
    pub async fn chat_send(&self, session_id: &str, content: &str, attachments: Vec<serde_json::Value>) -> Result<Value, String> {
        self.request_with_health_polling(
            serde_json::json!({
                "type": "chat",
                "action": "send",
                "session_id": session_id,
                "content": content,
                "attachments": attachments
            }),
            session_id,
        ).await
    }
    
    /// Cancel an active chat
    pub async fn chat_cancel(&self, session_id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "chat",
            "action": "cancel",
            "session_id": session_id
        })).await
    }

    /// Get system logs
    pub async fn system_logs(&self, limit: Option<usize>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "system",
            "action": "logs",
            "lines": limit,
            "level": null
        })).await
    }

    /// List sessions
    pub async fn sessions_list(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "list"
        })).await
    }

    /// List sessions filtered by workspace
    pub async fn sessions_list_by_workspace(&self, workspace_id: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "list_by_workspace",
            "workspace_id": workspace_id
        })).await
    }
    
    /// Create a session
    pub async fn session_create(&self, name: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "create",
            "name": name
        })).await
    }

    /// Create a session in a specific workspace
    pub async fn session_create_in_workspace(&self, name: Option<&str>, workspace_id: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "create_in_workspace",
            "name": name,
            "workspace_id": workspace_id
        })).await
    }
    
    /// Set or clear the workspace for a session
    pub async fn session_set_workspace(&self, session_id: &str, workspace_id: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "set_workspace",
            "id": session_id,
            "workspace_id": workspace_id
        })).await
    }

    /// Get session history
    pub async fn session_history(&self, session_id: &str, limit: Option<usize>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "history",
            "id": session_id,
            "limit": limit,
            "before": null
        })).await
    }
    
    /// Get session run state (in-flight streaming text, active tools).
    /// `light: true` omits the run journal — use it for periodic polls that
    /// only need counters, not the full multi-hour record.
    pub async fn session_get_run_state(&self, session_id: &str, light: bool) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "get_run_state",
            "id": session_id,
            "light": light
        })).await
    }

    /// Get system status
    pub async fn system_status(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "system",
            "action": "status"
        })).await
    }
    
    // =========================================================================
    // Session management (additional methods)
    // =========================================================================
    
    /// Delete a session
    pub async fn session_delete(&self, session_id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "delete",
            "id": session_id
        })).await
    }

    /// Delete all sessions
    pub async fn sessions_delete_all(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "delete_all"
        })).await
    }

    /// Rename a session
    pub async fn session_rename(&self, session_id: &str, name: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "rename",
            "id": session_id,
            "name": name
        })).await
    }
    
    /// Clear session history
    pub async fn session_clear(&self, session_id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "clear",
            "id": session_id
        })).await
    }
    
    // =========================================================================
    // Memory operations
    // =========================================================================

    /// List all memories
    pub async fn memory_list(&self, scope: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "memory",
            "action": "list",
            "scope": scope
        })).await
    }

    /// Search memories
    pub async fn memory_search(&self, query: &str, limit: Option<usize>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "memory",
            "action": "search",
            "query": query,
            "limit": limit
        })).await
    }
    
    /// Get a specific memory
    pub async fn memory_get(&self, id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "memory",
            "action": "get",
            "id": id
        })).await
    }
    
    /// Create a memory
    pub async fn memory_create(&self, content: &str, tags: Option<Vec<String>>, importance: Option<u8>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "memory",
            "action": "create",
            "content": content,
            "tags": tags,
            "importance": importance
        })).await
    }
    
    /// Update a memory
    pub async fn memory_update(&self, id: &str, content: Option<&str>, tags: Option<Vec<String>>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "memory",
            "action": "update",
            "id": id,
            "content": content,
            "tags": tags
        })).await
    }
    
    /// Delete a memory
    pub async fn memory_delete(&self, id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "memory",
            "action": "delete",
            "id": id
        })).await
    }

    /// Clear memories ("global", a workspace id, or None = all)
    pub async fn memory_clear(&self, scope: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "memory",
            "action": "clear",
            "scope": scope
        })).await
    }

    /// Get memory stats
    pub async fn memory_stats(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "memory",
            "action": "stats"
        })).await
    }
    
    /// Trigger memory consolidation
    pub async fn memory_consolidate(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "memory",
            "action": "consolidate"
        })).await
    }
    
    // =========================================================================
    // Scheduler operations
    // =========================================================================
    
    /// List scheduled jobs
    pub async fn scheduler_list(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "scheduler",
            "action": "list"
        })).await
    }
    
    /// Get job details
    pub async fn scheduler_get(&self, id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "scheduler",
            "action": "get",
            "id": id
        })).await
    }
    
    /// Add a cron job
    pub async fn scheduler_add(&self, schedule: &str, task: &str, name: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "scheduler",
            "action": "add",
            "schedule": schedule,
            "task": task,
            "name": name
        })).await
    }
    
    /// Update a job
    pub async fn scheduler_update(&self, id: &str, schedule: Option<&str>, task: Option<&str>, enabled: Option<bool>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "scheduler",
            "action": "update",
            "id": id,
            "schedule": schedule,
            "task": task,
            "enabled": enabled
        })).await
    }
    
    /// Remove a job
    pub async fn scheduler_remove(&self, id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "scheduler",
            "action": "remove",
            "id": id
        })).await
    }
    
    /// Run a job immediately
    pub async fn scheduler_run_now(&self, id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "scheduler",
            "action": "run_now",
            "id": id
        })).await
    }
    
    /// Get job history
    pub async fn scheduler_history(&self, id: &str, limit: Option<usize>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "scheduler",
            "action": "history",
            "id": id,
            "limit": limit
        })).await
    }
    
    // =========================================================================
    // Tool operations
    // =========================================================================
    
    /// List all tools
    pub async fn tool_list(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "tool",
            "action": "list"
        })).await
    }
    
    /// Execute a tool
    pub async fn tool_execute(&self, name: &str, input: Value) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "tool",
            "action": "execute",
            "name": name,
            "input": input
        })).await
    }
    
    /// Create a user tool
    pub async fn tool_create(&self, name: &str, description: &str, code: &str, needs_shell: Option<bool>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "tool",
            "action": "create",
            "name": name,
            "description": description,
            "code": code,
            "needs_shell": needs_shell
        })).await
    }
    
    /// Update a user tool
    pub async fn tool_update(&self, name: &str, description: Option<&str>, code: Option<&str>, needs_shell: Option<bool>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "tool",
            "action": "update",
            "name": name,
            "description": description,
            "code": code,
            "needs_shell": needs_shell
        })).await
    }
    
    /// Delete a user tool
    pub async fn tool_delete(&self, name: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "tool",
            "action": "delete",
            "name": name
        })).await
    }
    
    /// Test a user tool (without saving)
    pub async fn tool_test(&self, code: &str, input: Value) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "tool",
            "action": "test",
            "code": code,
            "input": input
        })).await
    }
    
    /// List user-created tools
    pub async fn tool_list_user(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "tool",
            "action": "list_user"
        })).await
    }

    /// Get tool source code
    pub async fn tool_get_source(&self, name: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "tool",
            "action": "get_source",
            "name": name
        })).await
    }
    
    // =========================================================================
    // Config operations
    // =========================================================================
    
    /// Get config (full or by path)
    pub async fn config_get(&self, path: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "config",
            "action": "get",
            "path": path
        })).await
    }
    
    /// Set config value
    pub async fn config_set(&self, path: &str, value: Value) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "config",
            "action": "set",
            "path": path,
            "value": value
        })).await
    }
    
    /// Reset config
    pub async fn config_reset(&self, path: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "config",
            "action": "reset",
            "path": path
        })).await
    }
    
    /// Reload config from disk
    pub async fn config_reload(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "config",
            "action": "reload"
        })).await
    }
    
    /// Export config
    pub async fn config_export(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "config",
            "action": "export"
        })).await
    }
    
    /// Import config
    pub async fn config_import(&self, config: Value) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "config",
            "action": "import",
            "config": config
        })).await
    }
    
    // =========================================================================
    // Workspace operations
    // =========================================================================
    
    /// List workspaces
    pub async fn workspace_list(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "workspace",
            "action": "list"
        })).await
    }
    
    /// Get workspace details
    pub async fn workspace_get(&self, id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "workspace",
            "action": "get",
            "id": id
        })).await
    }
    
    /// Open/register a workspace
    pub async fn workspace_open(&self, path: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "workspace",
            "action": "open",
            "path": path
        })).await
    }
    
    /// Close/unregister a workspace
    pub async fn workspace_close(&self, id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "workspace",
            "action": "close",
            "id": id
        })).await
    }
    
    /// Set active workspace
    pub async fn workspace_set_active(&self, id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "workspace",
            "action": "set_active",
            "id": id
        })).await
    }
    
    /// Clear active workspace (global mode)
    pub async fn workspace_clear_active(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "workspace",
            "action": "clear_active"
        })).await
    }
    
    /// Reload workspace context
    pub async fn workspace_reload(&self, id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "workspace",
            "action": "reload",
            "id": id
        })).await
    }
    
    /// Get workspace context (SOUL.md, USER.md, etc.)
    pub async fn workspace_get_context(&self, id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "workspace",
            "action": "get_context",
            "id": id
        })).await
    }
    
    /// Update workspace context file
    pub async fn workspace_update_context(&self, id: &str, file: &str, content: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "workspace",
            "action": "update_context",
            "id": id,
            "file": file,
            "content": content
        })).await
    }
    
    // =========================================================================
    // Channel operations
    // =========================================================================
    
    /// List channels
    pub async fn channel_list(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "channel",
            "action": "list"
        })).await
    }
    
    /// Get channel status
    pub async fn channel_status(&self, id: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "channel",
            "action": "status",
            "id": id
        })).await
    }

    // =========================================================================
    // Task operations
    // =========================================================================

    /// Merge a pass-through payload's fields into a base task request.
    fn task_request_with_payload(action: &str, payload: &Value) -> Value {
        let mut req = serde_json::json!({
            "type": "task",
            "action": action
        });
        if let (Some(obj), Some(payload_obj)) = (req.as_object_mut(), payload.as_object()) {
            for (k, v) in payload_obj {
                obj.insert(k.clone(), v.clone());
            }
        }
        req
    }

    /// List tasks in a scope
    pub async fn task_list(&self, scope: &str, session_id: Option<&str>, include_closed: Option<bool>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "task",
            "action": "list",
            "scope": scope,
            "session_id": session_id,
            "include_closed": include_closed
        })).await
    }

    /// Get one task with notes + activity
    pub async fn task_get(&self, id: i64) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "task",
            "action": "get",
            "id": id
        })).await
    }

    /// Create a task (payload fields flattened into the request)
    pub async fn task_create(&self, payload: Value) -> Result<Value, String> {
        self.request(Self::task_request_with_payload("create", &payload)).await
    }

    /// Partially update a task
    pub async fn task_update(&self, id: i64, patch: Value) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "task",
            "action": "update",
            "id": id,
            "patch": patch
        })).await
    }

    /// Complete a task (with acceptance verification)
    pub async fn task_done(&self, id: i64, workdir: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "task",
            "action": "done",
            "id": id,
            "workdir": workdir
        })).await
    }

    /// Delete a task subtree
    pub async fn task_delete(&self, id: i64) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "task",
            "action": "delete",
            "id": id
        })).await
    }

    /// Append a working note to a task
    pub async fn task_note(&self, id: i64, content: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "task",
            "action": "note",
            "id": id,
            "content": content
        })).await
    }

    /// Filter-language query over a scope's tasks
    pub async fn task_query(&self, filter: &str, scope: &str, session_id: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "task",
            "action": "query",
            "filter": filter,
            "scope": scope,
            "session_id": session_id
        })).await
    }

    /// Start a long-horizon run (payload fields flattened into the request)
    pub async fn task_start_run(&self, payload: Value) -> Result<Value, String> {
        self.request(Self::task_request_with_payload("start_run", &payload)).await
    }

    /// Status of the scope's run (live or last report)
    pub async fn task_run_status(&self, scope: &str, session_id: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "task",
            "action": "run_status",
            "scope": scope,
            "session_id": session_id
        })).await
    }

    /// Cancel the scope's active run
    pub async fn task_cancel_run(&self, scope: &str, session_id: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "task",
            "action": "cancel_run",
            "scope": scope,
            "session_id": session_id
        })).await
    }
}

/// Connection status for frontend
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionStatus {
    pub mode: ConnectionMode,
    pub state: ConnectionState,
    pub daemon_url: String,
}

impl DaemonClient {
    /// Get connection status for frontend
    pub async fn status(&self) -> ConnectionStatus {
        ConnectionStatus {
            mode: *self.mode.read().await,
            state: *self.state.read().await,
            daemon_url: self.config.url.clone(),
        }
    }
}
