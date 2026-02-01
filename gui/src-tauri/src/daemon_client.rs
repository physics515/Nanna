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
use tauri::{AppHandle, Emitter};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc, oneshot, RwLock};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use tracing::{debug, error, info, warn};

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
            connect_timeout: Duration::from_secs(3),
            request_timeout: Duration::from_secs(30),
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
    ToolStart { session_id: String, call_id: String, name: String },
    ToolEnd { session_id: String, call_id: String, output: String, success: bool },
    Error { code: String, message: String },
    Connected { client_id: String },
    Disconnected { client_id: String },
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
        }
    }
    
    /// Try to connect to the daemon
    pub async fn connect(&self) -> Result<(), String> {
        *self.state.write().await = ConnectionState::Connecting;
        
        info!("Attempting to connect to daemon at {}", self.config.url);
        
        let connect_future = connect_async(&self.config.url);
        
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
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        
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
            *mode.write().await = ConnectionMode::Embedded;
            info!("Disconnected from daemon, falling back to embedded mode");
            
            // Fail all pending requests
            let mut pending = pending.write().await;
            for (_, req) in pending.drain() {
                let _ = req.tx.send(Err("Disconnected".to_string()));
            }
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
    
    /// Send a request to the daemon
    pub async fn request(&self, action: Value) -> Result<Value, String> {
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
        match tokio::time::timeout(self.config.request_timeout, rx).await {
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
    
    /// Disconnect from daemon
    pub fn disconnect(&self) {
        let _ = self.shutdown_tx.send(());
    }
    
    // =========================================================================
    // Convenience methods
    // =========================================================================
    
    /// Send a chat message
    pub async fn chat_send(&self, session_id: &str, content: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "chat",
            "action": "send",
            "session_id": session_id,
            "content": content,
            "attachments": []
        })).await
    }
    
    /// List sessions
    pub async fn sessions_list(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "list"
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
    
    /// Get session history
    pub async fn session_history(&self, session_id: &str, limit: Option<usize>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "history",
            "id": session_id,
            "limit": limit
        })).await
    }
    
    /// Get system status
    pub async fn system_status(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "system",
            "action": "status"
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
