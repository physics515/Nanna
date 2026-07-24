//! IPC Server - WebSocket-based communication
//!
//! Handles connections from channel clients (GUI, CLI, API, etc.)

use crate::protocol::{Event, Request, Response};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio_tungstenite::{accept_async_with_config, tungstenite::{protocol::WebSocketConfig, Message}};
use tracing::{debug, error, info, warn};

/// Maximum WebSocket message size (128 MB).
/// Sessions with large tool outputs or long histories can exceed the
/// default 16 MB tungstenite limit, crashing the connection.
const WS_MAX_MESSAGE_SIZE: usize = 128 * 1024 * 1024;

/// Server-initiated keepalive ping cadence. A live client answers with a pong
/// (an incoming frame), which resets the read deadline below.
const WS_PING_INTERVAL_SECS: u64 = 15;

/// Drop a connection whose peer has been silent this long — three missed
/// ping/pong cycles. Bounds the read await for force-killed clients that never
/// send a Close frame (Windows TCP keepalive is off by default).
const WS_READ_DEADLINE_SECS: u64 = 45;

/// Unique identifier for a connected client
pub type ConnectionId = String;

/// Configuration for the IPC server
#[derive(Debug, Clone)]
pub struct IpcServerConfig {
    /// Host to bind to
    pub host: String,
    /// Port to listen on
    pub port: u16,
    /// Maximum number of concurrent connections
    pub max_connections: usize,
}

/// The WebSocket IPC port every client is expected to reach the daemon on.
///
/// One definition on purpose: this used to be a literal repeated across the daemon, the CLI and
/// the docs, and the copies drifted — `nanna daemon start` defaulted to `9999` while
/// `nanna daemon status`, the GUI sidecar and the README all used `5149`, so a CLI-started daemon
/// reported itself as not running.
pub const DEFAULT_IPC_PORT: u16 = 5149;

impl Default for IpcServerConfig {
    fn default() -> Self {
        Self {
            host: nanna_config::bind::LOOPBACK_HOST.to_string(),
            port: DEFAULT_IPC_PORT,
            max_connections: 100,
        }
    }
}

/// Connected client state
#[derive(Debug)]
struct ClientConnection {
    _id: ConnectionId,
    _addr: SocketAddr,
    tx: mpsc::Sender<Message>,
    _subscriptions: Vec<String>,
}

/// IPC Server for daemon communication
pub struct IpcServer {
    config: IpcServerConfig,
    clients: Arc<RwLock<HashMap<ConnectionId, ClientConnection>>>,
    request_tx: mpsc::Sender<(ConnectionId, Request)>,
    request_rx: Arc<RwLock<Option<mpsc::Receiver<(ConnectionId, Request)>>>>,
    event_tx: broadcast::Sender<Event>,
    shutdown_tx: broadcast::Sender<()>,
}

impl IpcServer {
    /// Create a new IPC server
    pub fn new(config: IpcServerConfig) -> Self {
        let (request_tx, request_rx) = mpsc::channel(1000);
        let (event_tx, _) = broadcast::channel(1000);
        let (shutdown_tx, _) = broadcast::channel(1);
        
        Self {
            config,
            clients: Arc::new(RwLock::new(HashMap::new())),
            request_tx,
            request_rx: Arc::new(RwLock::new(Some(request_rx))),
            event_tx,
            shutdown_tx,
        }
    }
    
    /// Bind a TCP listener, retrying on transient port conflicts.
    ///
    /// On Unix, sets SO_REUSEADDR so TIME_WAIT sockets don't block restart.
    /// On Windows, SO_REUSEADDR has dangerous semantics (allows hijacking),
    /// so we retry with a short delay instead.
    async fn bind_with_reuse(addr: &str) -> Result<TcpListener, std::io::Error> {
        let socket_addr: std::net::SocketAddr = addr.parse()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

        // On Unix, use SO_REUSEADDR for fast restart
        #[cfg(unix)]
        {
            let socket = socket2::Socket::new(
                socket2::Domain::for_address(socket_addr),
                socket2::Type::STREAM,
                Some(socket2::Protocol::TCP),
            )?;
            socket.set_reuse_address(true)?;
            socket.set_nonblocking(true)?;
            socket.bind(&socket_addr.into())?;
            socket.listen(128)?;
            return TcpListener::from_std(socket.into());
        }

        // On Windows, retry with delay if port is temporarily unavailable
        #[cfg(windows)]
        {
            for attempt in 0..5 {
                match TcpListener::bind(&socket_addr).await {
                    Ok(listener) => return Ok(listener),
                    Err(e) if attempt < 4 => {
                        warn!("IPC bind attempt {} failed ({}), retrying in 1s...", attempt + 1, e);
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                    Err(e) => return Err(e),
                }
            }
            unreachable!()
        }

        // Fallback for other platforms
        #[cfg(not(any(unix, windows)))]
        TcpListener::bind(&socket_addr).await
    }

    /// Get the address the server will bind to
    pub fn address(&self) -> String {
        format!("{}:{}", self.config.host, self.config.port)
    }
    
    /// Get a sender for broadcasting events to clients
    pub fn event_sender(&self) -> broadcast::Sender<Event> {
        self.event_tx.clone()
    }
    
    /// Get the request receiver (for the daemon to process requests)
    /// Can only be called once - returns None if already taken
    pub async fn take_request_receiver(&self) -> Option<mpsc::Receiver<(ConnectionId, Request)>> {
        let mut rx_lock = self.request_rx.write().await;
        rx_lock.take()
    }
    
    /// Send a response to a specific client
    pub async fn send_response(&self, client_id: &str, response: Response) -> Result<(), String> {
        let clients = self.clients.read().await;
        if let Some(client) = clients.get(client_id) {
            let msg = serde_json::to_string(&response).map_err(|e| e.to_string())?;
            client.tx.send(Message::Text(msg.into())).await.map_err(|e| e.to_string())?;
            Ok(())
        } else {
            Err(format!("Client not found: {}", client_id))
        }
    }
    
    /// Broadcast an event to all subscribed clients
    pub async fn broadcast_event(&self, event: Event) {
        let _ = self.event_tx.send(event);
    }
    
    /// Get connected client count
    pub async fn client_count(&self) -> usize {
        self.clients.read().await.len()
    }
    
    /// Get list of connected client IDs
    pub async fn client_ids(&self) -> Vec<ConnectionId> {
        self.clients.read().await.keys().cloned().collect()
    }
    
    /// Shutdown the server
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
    
    /// Run the IPC server
    pub async fn run(self: &Arc<Self>) -> Result<(), std::io::Error> {
        let addr = self.address();
        let listener = Self::bind_with_reuse(&addr).await?;
        info!("IPC server listening on ws://{}", addr);

        let mut shutdown_rx = self.shutdown_tx.subscribe();

        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, addr)) => {
                            // Check connection limit
                            if self.clients.read().await.len() >= self.config.max_connections {
                                warn!("Connection limit reached, rejecting {}", addr);
                                continue;
                            }

                            let client_id = uuid::Uuid::new_v4().to_string();
                            info!("New connection from {}: {}", addr, client_id);

                            // Spawn — NEVER await inline. The old inline await
                            // served one connection at a time, so a single
                            // silently-dead peer (force-killed client, no
                            // Close frame, no read deadline) parked the whole
                            // accept loop forever: new TCP connects completed
                            // in the kernel backlog but were never accepted
                            // (observed live; only a restart cleared it).
                            let this = Arc::clone(self);
                            tokio::spawn(async move {
                                this.handle_connection(client_id, stream, addr).await;
                            });
                        }
                        Err(e) => {
                            error!("Accept error: {}", e);
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("IPC server shutting down");
                    break;
                }
            }
        }

        Ok(())
    }
    
    async fn handle_connection(&self, client_id: ConnectionId, stream: TcpStream, addr: SocketAddr) {
        let mut ws_config = WebSocketConfig::default();
        ws_config.max_message_size = Some(WS_MAX_MESSAGE_SIZE);
        ws_config.max_frame_size = Some(WS_MAX_MESSAGE_SIZE);
        let ws_stream = match accept_async_with_config(stream, Some(ws_config)).await {
            Ok(ws) => ws,
            Err(e) => {
                error!("WebSocket handshake failed for {}: {}", addr, e);
                return;
            }
        };
        
        let (mut ws_tx, mut ws_rx) = ws_stream.split();
        let (msg_tx, mut msg_rx) = mpsc::channel::<Message>(100);
        
        // Store client connection
        {
            let mut clients = self.clients.write().await;
            clients.insert(client_id.clone(), ClientConnection {
                _id: client_id.clone(),
                _addr: addr,
                tx: msg_tx.clone(),
                _subscriptions: vec![],
            });
        }
        
        // Broadcast connect event
        let _ = self.event_tx.send(Event::Connected { client_id: client_id.clone() });
        
        let clients = self.clients.clone();
        let request_tx = self.request_tx.clone();
        let event_rx = self.event_tx.subscribe();
        let client_id_clone = client_id.clone();
        
        // Spawn task to handle outgoing messages
        let outgoing_task = tokio::spawn(async move {
            let mut event_rx = event_rx;
            
            loop {
                tokio::select! {
                    // Forward messages from the channel to WebSocket
                    Some(msg) = msg_rx.recv() => {
                        if ws_tx.send(msg).await.is_err() {
                            break;
                        }
                    }
                    // Forward broadcast events to this client
                    Ok(event) = event_rx.recv() => {
                        if let Ok(json) = serde_json::to_string(&event) {
                            if ws_tx.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    else => break,
                }
            }
        });
        
        // Handle incoming messages. Every await here is bounded: the server
        // pings every WS_PING_INTERVAL_SECS (the pong resets the read
        // deadline), and a peer silent past WS_READ_DEADLINE_SECS is dropped.
        // Without this, a force-killed client (no Close frame, Windows TCP
        // keepalive off) left `ws_rx.next()` pending forever.
        let client_id_for_incoming = client_id.clone();
        let mut ping_interval =
            tokio::time::interval(std::time::Duration::from_secs(WS_PING_INTERVAL_SECS));
        ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        'incoming: loop {
            tokio::select! {
                maybe_msg = tokio::time::timeout(
                    std::time::Duration::from_secs(WS_READ_DEADLINE_SECS),
                    ws_rx.next(),
                ) => {
                    let msg = match maybe_msg {
                        Err(_elapsed) => {
                            warn!(
                                "Read deadline ({WS_READ_DEADLINE_SECS}s) exceeded for {} — dropping dead connection",
                                client_id_for_incoming
                            );
                            break 'incoming;
                        }
                        Ok(None) => break 'incoming,
                        Ok(Some(m)) => m,
                    };
                    match msg {
                        Ok(Message::Text(text)) => {
                            match serde_json::from_str::<Request>(&text) {
                                Ok(request) => {
                                    debug!("Request from {}: {:?}", client_id_for_incoming, request.action);
                                    if request_tx.send((client_id_for_incoming.clone(), request)).await.is_err() {
                                        break 'incoming;
                                    }
                                }
                                Err(e) => {
                                    warn!("Invalid request from {}: {}", client_id_for_incoming, e);
                                    // Send error response
                                    let error_response = Response::error(
                                        "unknown".to_string(),
                                        "parse_error",
                                        format!("Invalid request: {}", e),
                                    );
                                    if let Ok(json) = serde_json::to_string(&error_response) {
                                        let _ = msg_tx.send(Message::Text(json.into())).await;
                                    }
                                }
                            }
                        }
                        Ok(Message::Ping(data)) => {
                            let _ = msg_tx.send(Message::Pong(data)).await;
                        }
                        Ok(Message::Close(_)) => {
                            debug!("Client {} sent close", client_id_for_incoming);
                            break 'incoming;
                        }
                        Err(e) => {
                            debug!("WebSocket error for {}: {}", client_id_for_incoming, e);
                            break 'incoming;
                        }
                        _ => {}
                    }
                }
                _ = ping_interval.tick() => {
                    // Server-initiated keepalive: forces the OS to notice a
                    // dead peer even with TCP keepalive off.
                    if msg_tx.send(Message::Ping(Vec::new().into())).await.is_err() {
                        break 'incoming;
                    }
                }
            }
        }
        
        // Cleanup
        outgoing_task.abort();
        
        {
            let mut clients_guard = clients.write().await;
            clients_guard.remove(&client_id_clone);
        }
        
        // Broadcast disconnect event
        let _ = self.event_tx.send(Event::Disconnected { client_id: client_id_clone.clone() });
        
        info!("Client {} disconnected", client_id_clone);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_config_default() {
        let config = IpcServerConfig::default();
        assert_eq!(config.port, 5149);
        assert_eq!(config.host, "127.0.0.1");
    }
}
