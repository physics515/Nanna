//! Discord Gateway WebSocket Listener
//!
//! Connects to Discord's Gateway API for real-time message events.
//! Handles heartbeats, resume, and reconnection automatically.

use super::circuit_breaker::{BreakerAction, CircuitBreaker};
use super::{Listener, ListenerError, ListenerHandle};
use crate::status::StatusManager;
use crate::{ChannelId, IncomingMessage, MessageContent, Sender};
use async_trait::async_trait;
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::{
    connect_async,
    tungstenite::Message as WsMessage,
    MaybeTlsStream, WebSocketStream,
};
use tokio::net::TcpStream;
use tracing::{debug, error, info, warn};

const GATEWAY_URL: &str = "wss://gateway.discord.gg/?v=10&encoding=json";

/// Discord Gateway listener
pub struct DiscordListener {
    bot_token: String,
    /// Intents bitmask
    intents: u64,
    /// Last sequence number for resume
    sequence: AtomicU64,
    /// Session ID for resume
    session_id: Arc<RwLock<Option<String>>>,
    /// Resume gateway URL
    resume_url: Arc<RwLock<Option<String>>>,
    /// Bot's own user ID (to ignore own messages)
    self_id: Arc<RwLock<Option<String>>>,
    /// Allowed guild IDs (empty = allow all)
    allowed_guilds: Vec<String>,
    /// Optional status manager for reporting connection state to the UI
    status_manager: Option<Arc<StatusManager>>,
}

impl DiscordListener {
    /// Create a new Discord Gateway listener
    ///
    /// Default intents: GUILDS, `GUILD_MESSAGES`, `MESSAGE_CONTENT`, `DIRECT_MESSAGES`
    pub fn new(bot_token: impl Into<String>) -> Self {
        Self {
            bot_token: bot_token.into(),
            // Intents: GUILDS (1) | GUILD_MESSAGES (512) | MESSAGE_CONTENT (32768) | DIRECT_MESSAGES (4096)
            intents: 1 | 512 | 32768 | 4096,
            sequence: AtomicU64::new(0),
            session_id: Arc::new(RwLock::new(None)),
            resume_url: Arc::new(RwLock::new(None)),
            self_id: Arc::new(RwLock::new(None)),
            allowed_guilds: Vec::new(),
            status_manager: None,
        }
    }

    /// Set custom intents
    #[must_use]
    pub const fn with_intents(mut self, intents: u64) -> Self {
        self.intents = intents;
        self
    }

    /// Only process messages from specific guilds
    #[must_use]
    pub fn with_allowed_guilds(mut self, guilds: Vec<String>) -> Self {
        self.allowed_guilds = guilds;
        self
    }

    /// Attach a status manager for reporting connection state to the UI
    #[must_use]
    pub fn with_status_manager(mut self, manager: Arc<StatusManager>) -> Self {
        self.status_manager = Some(manager);
        self
    }

    /// Build the identify payload
    fn identify_payload(&self) -> Value {
        json!({
            "op": 2,
            "d": {
                "token": self.bot_token,
                "intents": self.intents,
                "properties": {
                    "os": std::env::consts::OS,
                    "browser": "nanna",
                    "device": "nanna"
                }
            }
        })
    }

    /// Build the resume payload
    fn resume_payload(&self, session_id: &str, sequence: u64) -> Value {
        json!({
            "op": 6,
            "d": {
                "token": self.bot_token,
                "session_id": session_id,
                "seq": sequence
            }
        })
    }

    /// Convert Discord message to `IncomingMessage`
    fn convert_message(&self, data: &Value, self_id: Option<&str>) -> Option<IncomingMessage> {
        // Skip messages from self
        let author_id = data.get("author")?.get("id")?.as_str()?;
        if self_id == Some(author_id) {
            return None;
        }

        // Skip bot messages
        if data.get("author")?.get("bot")?.as_bool().unwrap_or(false) {
            return None;
        }

        let guild_id = data.get("guild_id").and_then(|v| v.as_str());
        
        // Check if guild is allowed
        if !self.allowed_guilds.is_empty()
            && let Some(gid) = guild_id
                && !self.allowed_guilds.iter().any(|g| g == gid) {
                    debug!("Ignoring message from non-allowed guild {}", gid);
                    return None;
                }

        let channel_id = data.get("channel_id")?.as_str()?;
        let message_id = data.get("id")?.as_str()?;
        let content = data.get("content")?.as_str()?.to_string();
        
        // Skip empty messages
        if content.is_empty() {
            return None;
        }

        let author = data.get("author")?;
        let username = author.get("username").and_then(|v| v.as_str());
        let global_name = author.get("global_name").and_then(|v| v.as_str());

        // Build timestamp from snowflake (Discord epoch: 2015-01-01)
        let snowflake: u64 = message_id.parse().ok()?;
        // `snowflake >> 22` is below 2^42, so the sum stays far below `i64::MAX`
        // and the sign reinterpretation never wraps.
        let timestamp = ((snowflake >> 22) + 1_420_070_400_000).cast_signed() / 1000;

        let referenced = data.get("referenced_message")
            .and_then(|r| r.get("id"))
            .and_then(|v| v.as_str())
            .map(|id| format!("{channel_id}:{id}"));

        Some(IncomingMessage {
            id: format!("{channel_id}:{message_id}"),
            channel: ChannelId::new("discord", channel_id.to_string()),
            sender: Sender {
                id: author_id.to_string(),
                name: global_name.or(username).map(String::from),
                username: username.map(String::from),
            },
            content: MessageContent::Text { text: content },
            timestamp,
            reply_to: referenced,
        })
    }

    /// Run the gateway connection loop
    async fn gateway_loop(
        self: Arc<Self>,
        sender: mpsc::Sender<IncomingMessage>,
        mut shutdown_rx: mpsc::Receiver<()>,
    ) {
        info!("Discord Gateway listener starting");

        let mut cb = CircuitBreaker::new("discord");
        if let Some(sm) = &self.status_manager {
            cb = cb.with_status_manager(Arc::clone(sm));
        }

        let mut should_resume = false;

        loop {
            // Check for shutdown
            if shutdown_rx.try_recv().is_ok() {
                info!("Discord Gateway listener received shutdown signal");
                break;
            }

            // Get connection URL (resume or fresh)
            let url = if should_resume {
                self.resume_url.read().await.clone().unwrap_or_else(|| GATEWAY_URL.to_string())
            } else {
                GATEWAY_URL.to_string()
            };

            cb.report_connecting().await;
            info!("Connecting to Discord Gateway: {}", url);

            let (ws_stream, _): (WebSocketStream<MaybeTlsStream<TcpStream>>, _) = match connect_async(&url).await {
                Ok(conn) => conn,
                Err(e) => {
                    let detail = format!("WebSocket connect failed: {e}");
                    if cb.record_conn_failure(&detail).await == BreakerAction::Stop {
                        break;
                    }
                    cb.backoff().await;
                    continue;
                }
            };

            let (mut write, mut read) = ws_stream.split();

            info!("Discord Gateway connected");

            'connection: loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        info!("Discord Gateway listener shutting down");
                        // Send close frame
                        let _ = write.close().await;
                        break 'connection;
                    }
                    msg = read.next() => {
                        let Some(msg) = msg else {
                            warn!("Discord Gateway connection closed");
                            should_resume = self.session_id.read().await.is_some();
                            break 'connection;
                        };

                        let msg = match msg {
                            Ok(WsMessage::Text(text)) => text,
                            Ok(WsMessage::Close(_)) => {
                                info!("Discord Gateway sent close frame");
                                should_resume = true;
                                break 'connection;
                            }
                            Ok(_) => continue,
                            Err(e) => {
                                warn!("Discord Gateway error: {}", e);
                                should_resume = true;
                                break 'connection;
                            }
                        };

                        let payload: GatewayPayload = match serde_json::from_str(&msg) {
                            Ok(p) => p,
                            Err(e) => {
                                warn!("Failed to parse Gateway payload: {}", e);
                                continue;
                            }
                        };

                        // Update sequence
                        if let Some(seq) = payload.s {
                            self.sequence.store(seq, Ordering::SeqCst);
                        }

                        let mut conn = GatewayConnection {
                            write: &mut write,
                            sender: &sender,
                            cb: &mut cb,
                            should_resume: &mut should_resume,
                        };
                        match self.handle_payload(&payload, &mut conn).await {
                            PayloadFlow::Continue => {}
                            PayloadFlow::Reconnect => break 'connection,
                            PayloadFlow::Stop => return,
                        }
                    }
                }
            }

            // Backoff before reconnecting (no-op if counters are zero)
            cb.backoff().await;
        }

        info!("Discord Gateway listener stopped");
    }

    /// Act on one decoded gateway payload (its sequence number is already stored).
    async fn handle_payload(
        &self,
        payload: &GatewayPayload,
        conn: &mut GatewayConnection<'_>,
    ) -> PayloadFlow {
        match payload.op {
            // Dispatch (event)
            0 => self.handle_dispatch(payload, conn).await,
            // Heartbeat (server requesting)
            1 => {
                let seq = self.sequence.load(Ordering::SeqCst);
                let hb = json!({ "op": 1, "d": seq });
                if conn.write.send(WsMessage::Text(hb.to_string().into())).await.is_err() {
                    warn!("Failed to send heartbeat");
                    return PayloadFlow::Reconnect;
                }
                PayloadFlow::Continue
            }
            // Reconnect
            7 => {
                info!("Discord Gateway requested reconnect");
                *conn.should_resume = true;
                PayloadFlow::Reconnect
            }
            // Invalid session
            9 => {
                let resumable = payload.d.as_ref()
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                warn!("Discord Gateway invalid session (resumable: {})", resumable);
                *conn.should_resume = resumable;
                if !resumable {
                    *self.session_id.write().await = None;
                    if conn.cb.record_auth_failure("invalid session (non-resumable)").await == BreakerAction::Stop {
                        return PayloadFlow::Stop;
                    }
                }
                PayloadFlow::Reconnect
            }
            // Hello
            10 => self.handle_hello(payload, conn).await,
            // Heartbeat ACK
            11 => {
                debug!("Heartbeat ACK");
                PayloadFlow::Continue
            }
            _ => {
                debug!("Unknown opcode: {}", payload.op);
                PayloadFlow::Continue
            }
        }
    }

    /// Handle an op 0 dispatch event (`READY`, `RESUMED`, `MESSAGE_CREATE`, ...).
    async fn handle_dispatch(
        &self,
        payload: &GatewayPayload,
        conn: &mut GatewayConnection<'_>,
    ) -> PayloadFlow {
        let event_name = payload.t.as_deref().unwrap_or("");

        match event_name {
            "READY" => {
                if let Some(d) = &payload.d {
                    // Store session info for resume
                    if let Some(sid) = d.get("session_id").and_then(|v| v.as_str()) {
                        *self.session_id.write().await = Some(sid.to_string());
                    }
                    if let Some(url) = d.get("resume_gateway_url").and_then(|v| v.as_str()) {
                        *self.resume_url.write().await = Some(url.to_string());
                    }
                    if let Some(user) = d.get("user")
                        && let Some(id) = user.get("id").and_then(|v| v.as_str()) {
                            *self.self_id.write().await = Some(id.to_string());
                        }
                    conn.cb.record_success().await;
                    info!("Discord Gateway READY");
                }
            }
            "RESUMED" => {
                conn.cb.record_success().await;
                info!("Discord Gateway RESUMED");
            }
            "MESSAGE_CREATE" => {
                if let Some(d) = &payload.d {
                    let self_id = self.self_id.read().await.clone();
                    if let Some(message) = self.convert_message(d, self_id.as_deref()) {
                        debug!("Discord message: {:?}", message.id);
                        if conn.sender.send(message).await.is_err() {
                            error!("Failed to send message to router");
                            return PayloadFlow::Reconnect;
                        }
                    }
                }
            }
            _ => {
                debug!("Discord event: {}", event_name);
            }
        }
        PayloadFlow::Continue
    }

    /// Handle op 10 Hello: log the heartbeat interval, then identify or resume.
    async fn handle_hello(
        &self,
        payload: &GatewayPayload,
        conn: &mut GatewayConnection<'_>,
    ) -> PayloadFlow {
        if let Some(d) = &payload.d
            && let Some(interval) = d.get("heartbeat_interval").and_then(serde_json::Value::as_u64) {
                debug!("Heartbeat interval: {}ms", interval);
            }

        // Send identify or resume
        let session = self.session_id.read().await.clone();
        let identify = if *conn.should_resume && let Some(session) = session.as_deref() {
            let seq = self.sequence.load(Ordering::SeqCst);
            self.resume_payload(session, seq)
        } else {
            self.identify_payload()
        };

        if conn.write.send(WsMessage::Text(identify.to_string().into())).await.is_err() {
            warn!("Failed to send identify/resume");
            return PayloadFlow::Reconnect;
        }

        *conn.should_resume = false;

        // Start heartbeat task
        // Note: heartbeat sending is handled inline when we receive op:1
        // In a full implementation, we'd spawn a task that sends heartbeats on interval
        // and coordinate with the main loop. For now, we rely on Discord's heartbeat requests.
        PayloadFlow::Continue
    }
}

/// Write half of the gateway WebSocket.
type GatewaySink = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, WsMessage>;

/// The per-connection state a gateway payload handler may touch.
struct GatewayConnection<'a> {
    write: &'a mut GatewaySink,
    sender: &'a mpsc::Sender<IncomingMessage>,
    cb: &'a mut CircuitBreaker,
    should_resume: &'a mut bool,
}

/// What the connection loop does after handling one gateway payload.
enum PayloadFlow {
    /// Keep reading this connection.
    Continue,
    /// Leave this connection; the outer loop backs off and reconnects.
    Reconnect,
    /// Stop the listener immediately (the circuit breaker gave up).
    Stop,
}

#[async_trait]
impl Listener for DiscordListener {
    fn provider(&self) -> &'static str {
        "discord"
    }

    async fn start(
        self: Arc<Self>,
        sender: mpsc::Sender<IncomingMessage>,
    ) -> Result<ListenerHandle, ListenerError> {
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

        let listener = Arc::clone(&self);
        let handle = tokio::spawn(async move {
            listener.gateway_loop(sender, shutdown_rx).await;
        });

        Ok(ListenerHandle::new(shutdown_tx, handle))
    }
}

/// Discord Gateway payload
#[derive(Debug, Deserialize)]
struct GatewayPayload {
    op: u8,
    d: Option<Value>,
    s: Option<u64>,
    t: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identify_payload() {
        let listener = DiscordListener::new("test_token");
        let payload = listener.identify_payload();
        assert_eq!(payload["op"], 2);
        assert_eq!(payload["d"]["token"], "test_token");
    }
}
