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
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

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

impl Default for IpcServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 5149,
            max_connections: 100,
        }
    }
}

/// Connected client state
#[derive(Debug)]
struct ClientConnection {
    id: ConnectionId,
    addr: SocketAddr,
    tx: mpsc::Sender<Message>,
    subscriptions: Vec<String>,
}

/// IPC Server for daemon communication
pub struct IpcServer {
    config: IpcServerConfig,
    clients: Arc<RwLock<HashMap<ConnectionId, ClientConnection>>>,
    request_tx: mpsc::Sender<(ConnectionId, Request)>,
    request_rx: mpsc::Receiver<(ConnectionId, Request)>,
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
            request_rx,
            event_tx,
            shutdown_tx,
        }
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
    pub fn take_request_receiver(&mut self) -> mpsc::Receiver<(ConnectionId, Request)> {
        let (tx, rx) = mpsc::channel(1000);
        std::mem::replace(&mut self.request_tx, tx);
        std::mem::replace(&mut self.request_rx, rx)
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
    pub async fn run(&self) -> Result<(), std::io::Error> {
        let addr = self.address();
        let listener = TcpListener::bind(&addr).await?;
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
                            
                            self.handle_connection(client_id, stream, addr).await;
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
        let ws_stream = match accept_async(stream).await {
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
                id: client_id.clone(),
                addr,
                tx: msg_tx.clone(),
                subscriptions: vec![],
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
        
        // Handle incoming messages
        let client_id_for_incoming = client_id.clone();
        while let Some(msg) = ws_rx.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    match serde_json::from_str::<Request>(&text) {
                        Ok(request) => {
                            debug!("Request from {}: {:?}", client_id_for_incoming, request.action);
                            if request_tx.send((client_id_for_incoming.clone(), request)).await.is_err() {
                                break;
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
                    break;
                }
                Err(e) => {
                    debug!("WebSocket error for {}: {}", client_id_for_incoming, e);
                    break;
                }
                _ => {}
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
