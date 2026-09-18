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

/// Read limits for the daemon connection: the one shared
/// [`nanna_config::bind::IPC_MAX_MESSAGE_BYTES`], not a copy that "must match".
fn ws_config() -> WebSocketConfig {
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(nanna_config::bind::IPC_MAX_MESSAGE_BYTES);
    config.max_frame_size = Some(nanna_config::bind::IPC_MAX_MESSAGE_BYTES);
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
            url: nanna_config::default_daemon_ws_url(),
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
    #[must_use]
    pub const fn is_error(&self) -> bool {
        matches!(self.result, ResponseResult::Error { .. })
    }
    
    #[must_use]
    pub const fn data(&self) -> Option<&Value> {
        match &self.result {
            ResponseResult::Success { data } => Some(data),
            ResponseResult::Error { .. } => None,
        }
    }
    
    /// The success payload.
    ///
    /// # Errors
    ///
    /// Returns the daemon's `message` when the response is an error response.
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
    /// A complete message appended to a session outside a streamed turn (a
    /// reminder coming due). Mirrors `nanna_daemon::protocol::Event::SessionMessageAdded`.
    SessionMessageAdded { session_id: String, message_id: String, role: String, content: String },
    /// Every message of a session was removed (`session.clear`, or `/new` from a
    /// chat app). Mirrors `nanna_daemon::protocol::Event::SessionCleared`.
    SessionCleared { id: String },
    ThinkingDelta { session_id: String, delta: String },
    ModelSwitch { model: String, reason: Option<String> },
    ToolStart { session_id: String, call_id: String, name: String, #[serde(default)] input: Option<serde_json::Value>, #[serde(default)] model: Option<String>, #[serde(default)] tokens: Option<u64>, #[serde(default)] total_tokens: Option<u64> },
    ToolEnd { session_id: String, call_id: String, output: String, success: bool, #[serde(default)] duration_ms: Option<u64>, #[serde(default)] data: Option<serde_json::Value> },
    Error { code: String, message: String, #[serde(default)] session_id: Option<String> },
    ContextUsage { session_id: String, used: u64, window: u64 },
    /// Low-frequency beat while a chat turn is in flight (~30s, the cadence
    /// the daemon derives from its silence budgets). It carries what the turn
    /// is waiting on and for how long, which is the only thing that tells a
    /// working turn apart from a wedged one from outside the daemon.
    ///
    /// The daemon has emitted these since P22 Tier 4; this enum had no variant
    /// for them, so every beat fell out of the event parse and was logged as
    /// "Unknown message format" instead of reaching the GUI.
    ///
    /// `quiet_s`, `step_index` and `last_tool` are absent on a beat that has
    /// nothing to report yet (no output observed, no plan item, no tool run),
    /// so they are optional here exactly as they are on the wire.
    LivenessBeat {
        session_id: String,
        elapsed_s: u64,
        phase: String,
        awaiting: String,
        #[serde(default)] quiet_s: Option<u64>,
        #[serde(default)] step_index: Option<usize>,
        #[serde(default)] last_tool: Option<String>,
        beat: u64,
    },
    Connected { client_id: String },
    Disconnected { client_id: String },
    TaskRunStarted { scope: String, #[serde(default)] scope_id: Option<String>, goal: String },
    TaskRunProgress { scope: String, #[serde(default)] scope_id: Option<String>, #[serde(default)] task_id: Option<i64>, kind: String, detail: serde_json::Value },
    TaskRunCompleted { scope: String, #[serde(default)] scope_id: Option<String>, report: serde_json::Value },
    /// A workspace was registered or removed on the daemon. Payload-free by
    /// design: the workspace SET is shared, which one is active is this
    /// client's own view.
    WorkspacesChanged,
    /// The daemon's config was mutated and committed. Payload-free by design:
    /// each view re-fetches the slice it renders.
    ConfigChanged,
    /// A well-formed daemon event this build has no variant for.
    ///
    /// The daemon and the GUI ship separately, so the daemon's event set is
    /// routinely ahead of this enum. Without a catch-all, one unknown `event`
    /// value made the WHOLE message fail to parse, and it was then reported as
    /// "Unknown message format" — indistinguishable from a corrupt frame. That
    /// is how the liveness beat stayed invisible for its entire life.
    ///
    /// This catches an unrecognised `event` VALUE only: a message with no
    /// `event` field at all still fails the parse and is still reported, so a
    /// genuinely malformed frame is not quietly swallowed.
    #[serde(other)]
    Unknown,
}

/// Pending request waiting for response
struct PendingRequest {
    tx: oneshot::Sender<Result<Value, String>>,
}

/// The handles a connection's background tasks (the message pump and the
/// reconnection loop) share with the [`DaemonClient`] that spawned them.
#[derive(Clone)]
struct ConnectionShared {
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
    #[must_use]
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
    ///
    /// # Errors
    ///
    /// Returns `"Connection failed: …"` when the WebSocket handshake with the
    /// configured URL fails (typically nothing is listening), and
    /// `"Connection timeout"` when it does not complete within
    /// `connect_timeout`. Either way the state is left `Disconnected`.
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
                Err(format!("Connection failed: {e}"))
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
    
    /// Clone out every handle a connection's background tasks share with the
    /// client, so a task can outlive the borrow of `self`.
    fn shared(&self) -> ConnectionShared {
        ConnectionShared {
            config: self.config.clone(),
            state: self.state.clone(),
            mode: self.mode.clone(),
            msg_tx: self.msg_tx.clone(),
            pending: self.pending.clone(),
            event_tx: self.event_tx.clone(),
            shutdown_tx: self.shutdown_tx.clone(),
            reconnecting: self.reconnecting.clone(),
        }
    }

    async fn spawn_handler(&self, ws: WebSocketStream<MaybeTlsStream<TcpStream>>) {
        Self::attach_connection(ws, self.shared(), false).await;
    }

    /// Install `ws` as the live connection and spawn the task that pumps it
    /// until it closes, then runs [`Self::handle_disconnect`].
    ///
    /// `reconnected` only selects the disconnect log line, so a connection the
    /// reconnect loop re-established stays distinguishable in the logs.
    async fn attach_connection(
        ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
        shared: ConnectionShared,
        reconnected: bool,
    ) {
        let (mut ws_tx, mut ws_rx) = ws.split();
        let (msg_tx, mut msg_rx) = mpsc::channel::<Message>(100);

        *shared.msg_tx.write().await = Some(msg_tx);

        let mut shutdown_rx = shared.shutdown_tx.subscribe();

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
                                Self::handle_message_static(&text, &shared.pending, &shared.event_tx).await;
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

            Self::handle_disconnect(shared, reconnected).await;
        });
    }

    /// Settle a closed connection: mark the client disconnected, fail every
    /// pending request, then either start the reconnection loop (unless one is
    /// already running) or fall back to embedded mode.
    async fn handle_disconnect(shared: ConnectionShared, reconnected: bool) {
        *shared.state.write().await = ConnectionState::Disconnected;
        if reconnected {
            info!("Disconnected from daemon again");
        } else {
            info!("Disconnected from daemon");
        }

        // Fail all pending requests
        {
            let mut pending = shared.pending.write().await;
            for (_, req) in pending.drain() {
                let _ = req.tx.send(Err("Disconnected".to_string()));
            }
        }

        // Start reconnection loop if auto_reconnect is enabled
        if shared.config.auto_reconnect {
            // Check if reconnection loop is already running
            let already_reconnecting = {
                let mut flag = shared.reconnecting.write().await;
                if *flag {
                    true
                } else {
                    *flag = true;
                    false
                }
            };

            if !already_reconnecting {
                Self::start_reconnect_loop(shared);
            }
        } else {
            // Only a first connection can get here: the reconnect loop runs
            // only when `auto_reconnect` is set, and the config never changes.
            *shared.mode.write().await = ConnectionMode::Embedded;
            info!("Auto-reconnect disabled, falling back to embedded mode");
        }
    }

    /// Start a background task to periodically attempt reconnection
    fn start_reconnect_loop(shared: ConnectionShared) {
        tokio::spawn(async move {
            let reconnect_interval = Duration::from_secs(10);
            let max_attempts = 30; // Give up after ~5 minutes
            let mut attempt = 0;
            let mut shutdown_rx = shared.shutdown_tx.subscribe();

            info!("Starting reconnection loop (interval: {:?})", reconnect_interval);

            loop {
                attempt += 1;

                // Wait before attempting reconnection
                tokio::select! {
                    () = tokio::time::sleep(reconnect_interval) => {}
                    _ = shutdown_rx.recv() => {
                        info!("Reconnection loop stopped by shutdown signal");
                        break;
                    }
                }

                // Check if we should give up
                if attempt > max_attempts {
                    warn!("Max reconnection attempts ({}) reached, giving up", max_attempts);
                    *shared.mode.write().await = ConnectionMode::Embedded;
                    break;
                }

                *shared.state.write().await = ConnectionState::Reconnecting;
                info!("Reconnection attempt {}/{}", attempt, max_attempts);

                // Try to connect
                let connect_future = connect_async_with_config(&shared.config.url, Some(ws_config()), false);
                match tokio::time::timeout(shared.config.connect_timeout, connect_future).await {
                    Ok(Ok((ws, _))) => {
                        info!("Reconnected to daemon successfully");
                        *shared.state.write().await = ConnectionState::Connected;
                        *shared.mode.write().await = ConnectionMode::Daemon;

                        // Spawn new handler for this connection
                        Self::attach_connection(ws, shared.clone(), true).await;

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
            *shared.reconnecting.write().await = false;
        });
    }
    
    async fn handle_message_static(
        text: &str,
        pending: &Arc<RwLock<HashMap<String, PendingRequest>>>,
        event_tx: &broadcast::Sender<DaemonEvent>,
    ) {
        // Try to parse as Response first
        if let Ok(response) = serde_json::from_str::<Response>(text) {
            let waiting = pending.write().await.remove(&response.id);
            if let Some(req) = waiting {
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
    #[must_use]
    pub fn subscribe_events(&self) -> broadcast::Receiver<DaemonEvent> {
        self.event_tx.subscribe()
    }

    /// Sender side of the event bus. Used in embedded mode to inject events
    /// from the in-process `AgentService` so the same subscribers (the single
    /// Tauri event-forwarding task) see them exactly like daemon events.
    #[must_use]
    pub fn event_sender(&self) -> broadcast::Sender<DaemonEvent> {
        self.event_tx.clone()
    }
    
    /// Send a request to the daemon with the default timeout
    ///
    /// # Errors
    ///
    /// - `"Not connected to daemon"` when the client is not in daemon mode
    ///   (it never connected, or the reconnection loop gave up);
    /// - `"No message sender"` when no connection has been installed yet;
    /// - `"Send error: …"` when the connection's message pump has already
    ///   ended;
    /// - `"Disconnected"` when the connection drops before the reply arrives
    ///   (every pending request is failed with it), or `"Response channel
    ///   closed"` if the reply slot is discarded without an answer;
    /// - `"Request timeout"` when no reply arrives within the configured
    ///   `request_timeout` (5 minutes by default);
    /// - the `message` of an error response.
    ///
    /// A refused action is *not* an error here: the daemon answers every
    /// request it parsed with a successful reply and reports the refusal
    /// inside it (`{"error": …, "message": …}`). Its only error response is
    /// for a request it could not parse, and that carries no request id, so
    /// such a request ends in `"Request timeout"`.
    pub async fn request(&self, action: Value) -> Result<Value, String> {
        self.request_with_timeout(action, self.config.request_timeout).await
    }

    /// Register a pending request for `action` and hand it to the connection's
    /// message pump. Returns the request id and the slot its reply lands in.
    ///
    /// The pending entry is registered before the send, so a reply can never
    /// arrive for an id that is not yet waiting for it.
    async fn send_request(
        &self,
        action: Value,
    ) -> Result<(String, oneshot::Receiver<Result<Value, String>>), String> {
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
            .map_err(|e| format!("Serialization error: {e}"))?;

        // Create response channel
        let (tx, rx) = oneshot::channel();

        // Register pending request
        {
            let mut pending = self.pending.write().await;
            pending.insert(id.clone(), PendingRequest { tx });
        }

        // Send request
        msg_tx.send(Message::Text(json.into())).await
            .map_err(|e| format!("Send error: {e}"))?;

        Ok((id, rx))
    }

    /// Send a request to the daemon with a specific timeout
    async fn request_with_timeout(&self, action: Value, timeout: Duration) -> Result<Value, String> {
        let (id, rx) = self.send_request(action).await?;

        // Wait for response with timeout
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                self.pending.write().await.remove(&id);
                Err("Response channel closed".to_string())
            }
            Err(_) => {
                self.pending.write().await.remove(&id);
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
        let (id, mut rx) = self.send_request(action).await?;

        // Health-check polling loop
        let mut missed_pings: u32 = 0;

        loop {
            tokio::select! {
                // Branch A: response arrives
                result = &mut rx => {
                    return if let Ok(result) = result { result } else {
                        self.pending.write().await.remove(&id);
                        Err("Response channel closed".to_string())
                    };
                }

                // Branch B: health-check interval fires
                () = tokio::time::sleep(HEALTH_POLL_INTERVAL) => {
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
                                .and_then(serde_json::Value::as_bool)
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
                                        self.pending.write().await.remove(&id);
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
                                        self.pending.write().await.remove(&id);
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
                                self.pending.write().await.remove(&id);
                                return Err(format!(
                                    "Daemon unresponsive ({missed_pings} missed health pings)"
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
    ///
    /// # Errors
    ///
    /// Fails as [`Self::request`] does, with one difference: there is no fixed
    /// request timeout. Instead it fails with `"Daemon unresponsive (N missed health
    /// pings)"` once three consecutive `session.get_run_state` pings (one every
    /// 30 s, each allowed 15 s) go unanswered. A run the daemon reports as
    /// finished whose reply then never arrives within the 60 s grace period is
    /// answered with a synthetic empty success, not an error.
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
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `chat.cancel` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn chat_cancel(&self, session_id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "chat",
            "action": "cancel",
            "session_id": session_id
        })).await
    }

    /// Get system logs
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `system.logs` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn system_logs(&self, limit: Option<usize>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "system",
            "action": "logs",
            "lines": limit,
            "level": null
        })).await
    }

    /// List sessions
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `session.list` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn sessions_list(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "list"
        })).await
    }

    /// List sessions filtered by workspace
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `session.list_by_workspace` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn sessions_list_by_workspace(&self, workspace_id: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "list_by_workspace",
            "workspace_id": workspace_id
        })).await
    }
    
    /// Create a session
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `session.create` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn session_create(&self, name: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "create",
            "name": name
        })).await
    }

    /// Create a session in a specific workspace
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `session.create_in_workspace` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn session_create_in_workspace(&self, name: Option<&str>, workspace_id: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "create_in_workspace",
            "name": name,
            "workspace_id": workspace_id
        })).await
    }
    
    /// Set or clear the workspace for a session
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `session.set_workspace` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn session_set_workspace(&self, session_id: &str, workspace_id: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "set_workspace",
            "id": session_id,
            "workspace_id": workspace_id
        })).await
    }

    /// Set or clear the chat-model pin for a session
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `session.set_model` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn session_set_model(&self, session_id: &str, model: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "set_model",
            "id": session_id,
            "model": model
        })).await
    }

    /// Render a session as a document (`markdown` or `json`) — `{format, filename, content}`.
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `session.export` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn session_export(&self, session_id: &str, format: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "export",
            "id": session_id,
            "format": format
        }))
        .await
    }

    /// A session's file checkpoints, newest first.
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `session.file_history` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn session_file_history(
        &self,
        session_id: &str,
        limit: Option<usize>,
    ) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "file_history",
            "id": session_id,
            "limit": limit
        }))
        .await
    }

    /// Restore one file checkpoint of a session.
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `session.restore_file` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn session_restore_file(
        &self,
        session_id: &str,
        checkpoint: u64,
    ) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "restore_file",
            "id": session_id,
            "checkpoint": checkpoint
        }))
        .await
    }

    /// Set or clear the user-selected extra tools for a session.
    ///
    /// Additive by contract (the daemon unions them into the default active
    /// set); an empty list clears the selection and restores byte-identical
    /// default tool behavior.
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `session.set_tools` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn session_set_tools(&self, session_id: &str, tools: Vec<String>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "set_tools",
            "id": session_id,
            "tools": tools
        })).await
    }

    /// Get session history
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `session.history` inside the `Ok` reply (an `error` field), not as an `Err`.
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
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `session.get_run_state` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn session_get_run_state(&self, session_id: &str, light: bool) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "get_run_state",
            "id": session_id,
            "light": light
        })).await
    }

    /// Get system status
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `system.status` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn system_status(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "system",
            "action": "status"
        })).await
    }

    /// Probe an Ollama server through the daemon's one hardened probe.
    ///
    /// `base_url: None` means the daemon's configured `[memory].ollama_host`;
    /// `models` empty means every Ollama model the config names. The answer
    /// keeps "server down" (`reachable: false` + `reason`) apart from "server
    /// up, model missing" (`reachable: true` + `missing`), and lists the
    /// installed models as `{ name, size_bytes }`.
    pub async fn system_probe_ollama(&self, base_url: Option<&str>, models: Vec<String>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "system",
            "action": "probe_ollama",
            "base_url": base_url,
            "models": models
        })).await
    }
    
    // =========================================================================
    // Session management (additional methods)
    // =========================================================================
    
    /// Delete a session
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `session.delete` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn session_delete(&self, session_id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "delete",
            "id": session_id
        })).await
    }

    /// Delete all sessions
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `session.delete_all` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn sessions_delete_all(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "delete_all"
        })).await
    }

    /// Rename a session
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `session.rename` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn session_rename(&self, session_id: &str, name: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "session",
            "action": "rename",
            "id": session_id,
            "name": name
        })).await
    }
    
    /// Clear session history
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `session.clear` inside the `Ok` reply (an `error` field), not as an `Err`.
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
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `memory.list` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn memory_list(&self, scope: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "memory",
            "action": "list",
            "scope": scope
        })).await
    }

    /// Search memories
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `memory.search` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn memory_search(&self, query: &str, limit: Option<usize>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "memory",
            "action": "search",
            "query": query,
            "limit": limit
        })).await
    }
    
    /// Get a specific memory
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `memory.get` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn memory_get(&self, id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "memory",
            "action": "get",
            "id": id
        })).await
    }
    
    /// Create a memory
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `memory.create` inside the `Ok` reply (an `error` field), not as an `Err`.
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
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `memory.update` inside the `Ok` reply (an `error` field), not as an `Err`.
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
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `memory.delete` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn memory_delete(&self, id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "memory",
            "action": "delete",
            "id": id
        })).await
    }

    /// Clear memories ("global", a workspace id, or None = all)
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `memory.clear` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn memory_clear(&self, scope: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "memory",
            "action": "clear",
            "scope": scope
        })).await
    }

    /// Get memory stats
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `memory.stats` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn memory_stats(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "memory",
            "action": "stats"
        })).await
    }
    
    /// Trigger memory consolidation
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `memory.consolidate` inside the `Ok` reply (an `error` field), not as an `Err`.
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
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `scheduler.list` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn scheduler_list(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "scheduler",
            "action": "list"
        })).await
    }
    
    /// Get job details
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `scheduler.get` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn scheduler_get(&self, id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "scheduler",
            "action": "get",
            "id": id
        })).await
    }
    
    /// Add a cron job
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `scheduler.add` inside the `Ok` reply (an `error` field), not as an `Err`.
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
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `scheduler.update` inside the `Ok` reply (an `error` field), not as an `Err`.
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
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `scheduler.remove` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn scheduler_remove(&self, id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "scheduler",
            "action": "remove",
            "id": id
        })).await
    }
    
    /// Run a job immediately
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `scheduler.run_now` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn scheduler_run_now(&self, id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "scheduler",
            "action": "run_now",
            "id": id
        })).await
    }
    
    /// Get job history
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `scheduler.history` inside the `Ok` reply (an `error` field), not as an `Err`.
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
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `tool.list` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn tool_list(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "tool",
            "action": "list"
        })).await
    }
    
    /// Enable or disable a tool.
    ///
    /// One method rather than two, because the caller always has a boolean in
    /// hand (a toggle's new state) and never a choice of verb. The daemon's
    /// `enable`/`disable` split is an implementation detail of the wire format,
    /// so it is resolved here instead of at every call site.
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `tool.enable` / `tool.disable` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn tool_set_enabled(&self, name: &str, enabled: bool) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "tool",
            "action": if enabled { "enable" } else { "disable" },
            "name": name
        })).await
    }

    /// Execute a tool
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `tool.execute` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn tool_execute(&self, name: &str, input: Value) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "tool",
            "action": "execute",
            "name": name,
            "input": input
        })).await
    }
    
    /// Create a user tool
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `tool.create` inside the `Ok` reply (an `error` field), not as an `Err`.
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
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `tool.update` inside the `Ok` reply (an `error` field), not as an `Err`.
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
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `tool.delete` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn tool_delete(&self, name: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "tool",
            "action": "delete",
            "name": name
        })).await
    }
    
    /// Test a user tool (without saving)
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `tool.test` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn tool_test(&self, code: &str, input: Value) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "tool",
            "action": "test",
            "code": code,
            "input": input
        })).await
    }
    
    /// List user-created tools
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `tool.list_user` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn tool_list_user(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "tool",
            "action": "list_user"
        })).await
    }

    /// Get tool source code
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `tool.get_source` inside the `Ok` reply (an `error` field), not as an `Err`.
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
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `config.get` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn config_get(&self, path: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "config",
            "action": "get",
            "path": path
        })).await
    }
    
    /// Set config value
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `config.set` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn config_set(&self, path: &str, value: Value) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "config",
            "action": "set",
            "path": path,
            "value": value
        })).await
    }
    
    /// Reset config
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `config.reset` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn config_reset(&self, path: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "config",
            "action": "reset",
            "path": path
        })).await
    }
    
    /// Reload config from disk
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `config.reload` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn config_reload(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "config",
            "action": "reload"
        })).await
    }
    
    /// Export config
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `config.export` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn config_export(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "config",
            "action": "export"
        })).await
    }
    
    /// Import config
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `config.import` inside the `Ok` reply (an `error` field), not as an `Err`.
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
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `workspace.list` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn workspace_list(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "workspace",
            "action": "list"
        })).await
    }
    
    /// Get workspace details
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `workspace.get` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn workspace_get(&self, id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "workspace",
            "action": "get",
            "id": id
        })).await
    }
    
    /// Open/register a workspace
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `workspace.open` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn workspace_open(&self, path: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "workspace",
            "action": "open",
            "path": path
        })).await
    }
    
    /// Close/unregister a workspace
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `workspace.close` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn workspace_close(&self, id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "workspace",
            "action": "close",
            "id": id
        })).await
    }
    
    /// Set active workspace
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `workspace.set_active` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn workspace_set_active(&self, id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "workspace",
            "action": "set_active",
            "id": id
        })).await
    }
    
    /// Clear active workspace (global mode)
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `workspace.clear_active` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn workspace_clear_active(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "workspace",
            "action": "clear_active"
        })).await
    }
    
    /// Reload workspace context
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `workspace.reload` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn workspace_reload(&self, id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "workspace",
            "action": "reload",
            "id": id
        })).await
    }
    
    /// Get workspace context (SOUL.md, USER.md, etc.)
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `workspace.get_context` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn workspace_get_context(&self, id: &str) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "workspace",
            "action": "get_context",
            "id": id
        })).await
    }
    
    /// Update workspace context file
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `workspace.update_context` inside the `Ok` reply (an `error` field), not as an `Err`.
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
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `channel.list` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn channel_list(&self) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "channel",
            "action": "list"
        })).await
    }
    
    /// Get channel status
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `channel.status` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn channel_status(&self, id: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "channel",
            "action": "status",
            "id": id
        })).await
    }

    /// List tasks in a scope. The chat's task checklist reads the store this
    /// way — the store is the chat engine (P19), so the checklist is a view
    /// of the live run, not a separate task UI.
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `task.list` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn task_list(&self, scope: &str, session_id: Option<&str>, include_closed: Option<bool>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "task",
            "action": "list",
            "scope": scope,
            "session_id": session_id,
            "include_closed": include_closed
        })).await
    }

    /// Create a new task.
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `task.create` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn task_create(
        &self,
        title: &str,
        scope: &str,
        session_id: Option<&str>,
        parent_id: Option<i64>,
        description: Option<&str>,
        priority: Option<i64>,
    ) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "task",
            "action": "create",
            "title": title,
            "scope": scope,
            "session_id": session_id,
            "parent_id": parent_id,
            "description": description,
            "priority": priority
        })).await
    }

    /// Update a task (partial patch).
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `task.update` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn task_update(&self, id: i64, patch: Value) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "task",
            "action": "update",
            "id": id,
            "patch": patch
        })).await
    }

    /// Mark a task as done (with optional acceptance check).
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `task.done` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn task_done(&self, id: i64, workdir: Option<&str>) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "task",
            "action": "done",
            "id": id,
            "workdir": workdir
        })).await
    }

    /// Delete a task and its subtree.
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `task.delete` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn task_delete(&self, id: i64) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "task",
            "action": "delete",
            "id": id
        })).await
    }

    /// Reorder a task by updating its priority.
    ///
    /// # Errors
    ///
    /// Fails only as [`Self::request`] does. The daemon reports a refused
    /// `task.update` inside the `Ok` reply (an `error` field), not as an `Err`.
    pub async fn task_reorder(&self, id: i64, new_priority: i64) -> Result<Value, String> {
        self.request(serde_json::json!({
            "type": "task",
            "action": "update",
            "id": id,
            "patch": { "priority": new_priority }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The daemon's own wire shape for a beat, taken from
    /// `nanna_daemon::protocol::Event::LivenessBeat`. Optional members are
    /// omitted by `skip_serializing_if`, so the beat that arrives before any
    /// tool has run looks like this.
    const BEAT_MINIMAL: &str = r#"{
        "event": "liveness_beat",
        "session_id": "s-1",
        "elapsed_s": 91,
        "phase": "streaming",
        "awaiting": "model output (ollama/qwen3.5:9b): last token 41s ago",
        "beat": 3
    }"#;

    /// Before this variant existed the whole message failed to parse and was
    /// logged as "Unknown message format", so the GUI never saw a single beat.
    #[test]
    fn liveness_beat_deserializes() {
        let event: DaemonEvent = serde_json::from_str(BEAT_MINIMAL).expect("beat must parse");
        match event {
            DaemonEvent::LivenessBeat {
                session_id, elapsed_s, phase, awaiting, quiet_s, step_index, last_tool, beat,
            } => {
                assert_eq!(session_id, "s-1");
                assert_eq!(elapsed_s, 91);
                assert_eq!(phase, "streaming");
                assert!(awaiting.contains("last token 41s ago"));
                // Absent on the wire means "not reported", not zero.
                assert_eq!(quiet_s, None);
                assert_eq!(step_index, None);
                assert_eq!(last_tool, None);
                assert_eq!(beat, 3);
            }
            other => panic!("expected LivenessBeat, got {other:?}"),
        }
    }

    /// A fuller beat carries the quiet time the badge renders.
    #[test]
    fn liveness_beat_keeps_the_optional_members_when_present() {
        let json = r#"{
            "event": "liveness_beat",
            "session_id": "s-2",
            "elapsed_s": 600,
            "phase": "tool",
            "awaiting": "tool exec",
            "quiet_s": 305,
            "step_index": 4,
            "last_tool": "exec",
            "beat": 20
        }"#;
        match serde_json::from_str::<DaemonEvent>(json).expect("beat must parse") {
            DaemonEvent::LivenessBeat { quiet_s, step_index, last_tool, .. } => {
                assert_eq!(quiet_s, Some(305));
                assert_eq!(step_index, Some(4));
                assert_eq!(last_tool.as_deref(), Some("exec"));
            }
            other => panic!("expected LivenessBeat, got {other:?}"),
        }
    }

    /// A chat app's `/new` empties the session; before this variant the event
    /// parsed as `Unknown` and an open chat kept showing the old conversation.
    #[test]
    fn session_cleared_deserializes() {
        let json = r#"{ "event": "session_cleared", "id": "telegram:1:2" }"#;
        match serde_json::from_str::<DaemonEvent>(json).expect("event must parse") {
            DaemonEvent::SessionCleared { id } => assert_eq!(id, "telegram:1:2"),
            other => panic!("expected SessionCleared, got {other:?}"),
        }
    }

    /// The daemon's wire shape for a delivered reminder. Before this variant it
    /// parsed as `Unknown` and the open chat showed nothing until reloaded.
    #[test]
    fn session_message_added_deserializes() {
        let json = r#"{
            "event": "session_message_added",
            "session_id": "s-1",
            "message_id": "m-1",
            "role": "assistant",
            "content": "⏰ Reminder: stretch"
        }"#;
        match serde_json::from_str::<DaemonEvent>(json).expect("event must parse") {
            DaemonEvent::SessionMessageAdded { session_id, message_id, role, content } => {
                assert_eq!(session_id, "s-1");
                assert_eq!(message_id, "m-1");
                assert_eq!(role, "assistant");
                assert_eq!(content, "⏰ Reminder: stretch");
            }
            other => panic!("expected SessionMessageAdded, got {other:?}"),
        }
    }

    /// The daemon ships events ahead of the GUI. An event this build has no
    /// variant for must stop the message from being reported as malformed —
    /// that conflation is what hid the beat.
    #[test]
    fn an_unknown_event_parses_instead_of_failing() {
        let json = r#"{"event": "some_event_from_a_newer_daemon", "whatever": 1}"#;
        assert!(matches!(
            serde_json::from_str::<DaemonEvent>(json),
            Ok(DaemonEvent::Unknown)
        ));
    }

    /// ...but a frame that is not an event at all still fails, so a genuinely
    /// malformed message keeps being reported rather than swallowed.
    #[test]
    fn a_message_without_an_event_tag_still_fails_to_parse() {
        assert!(serde_json::from_str::<DaemonEvent>(r#"{"nonsense": true}"#).is_err());
    }
}
