//! Signal channel implementation via signald
//!
//! Communicates with signald daemon over Unix socket (or TCP on Windows).
//! See: https://signald.org/

use crate::{
    Channel, ChannelCapabilities, ChannelError, ChannelFeatures, IncomingMessage, MessageContent,
    OutgoingMessage, Sender,
};
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{debug, info, warn};

/// Default signald socket path
#[cfg(unix)]
pub const DEFAULT_SOCKET_PATH: &str = "/var/run/signald/signald.sock";

#[cfg(windows)]
pub const DEFAULT_SOCKET_PATH: &str = "127.0.0.1:12345"; // TCP fallback on Windows

/// Signal channel via signald
pub struct SignalChannel {
    /// Account phone number (e.g., "+1234567890")
    account: String,
    /// Socket path or TCP address
    socket_path: String,
    /// Writer for sending commands
    writer: Arc<Mutex<Option<SignalWriter>>>,
    /// Pending requests waiting for responses
    pending: Arc<RwLock<HashMap<String, tokio::sync::oneshot::Sender<serde_json::Value>>>>,
    /// Request ID counter
    request_id: AtomicU64,
    /// Incoming message channel
    incoming_tx: Option<mpsc::Sender<IncomingMessage>>,
}

type SignalWriter = tokio::io::WriteHalf<tokio::net::TcpStream>;

impl SignalChannel {
    /// Create a new Signal channel.
    pub fn new(account: impl Into<String>) -> Self {
        Self {
            account: account.into(),
            socket_path: DEFAULT_SOCKET_PATH.to_string(),
            writer: Arc::new(Mutex::new(None)),
            pending: Arc::new(RwLock::new(HashMap::new())),
            request_id: AtomicU64::new(1),
            incoming_tx: None,
        }
    }

    /// Set custom socket path.
    #[must_use]
    pub fn with_socket(mut self, path: impl Into<String>) -> Self {
        self.socket_path = path.into();
        self
    }

    /// Set incoming message channel.
    pub fn with_incoming_channel(mut self, tx: mpsc::Sender<IncomingMessage>) -> Self {
        self.incoming_tx = Some(tx);
        self
    }

    /// Connect to signald.
    pub async fn connect(&self) -> Result<(), ChannelError> {
        info!(socket = %self.socket_path, "Connecting to signald");

        // Connect via TCP (cross-platform)
        let stream = tokio::net::TcpStream::connect(&self.socket_path)
            .await
            .map_err(|e| ChannelError::Connection(format!("Failed to connect to signald: {e}")))?;

        let (reader, writer) = tokio::io::split(stream);
        
        // Store writer
        {
            let mut w = self.writer.lock().await;
            *w = Some(writer);
        }

        // Subscribe to messages for our account
        self.subscribe().await?;

        // Spawn reader task
        let pending = self.pending.clone();
        let incoming_tx = self.incoming_tx.clone();
        let account = self.account.clone();
        
        tokio::spawn(async move {
            Self::reader_task(reader, pending, incoming_tx, account).await;
        });

        info!("Connected to signald");
        Ok(())
    }

    /// Subscribe to incoming messages.
    async fn subscribe(&self) -> Result<(), ChannelError> {
        let request = SignaldRequest {
            r#type: "subscribe".to_string(),
            id: Some(self.next_id()),
            version: "v1".to_string(),
            account: Some(self.account.clone()),
            ..Default::default()
        };

        self.send_request(&request).await?;
        Ok(())
    }

    /// Generate next request ID.
    fn next_id(&self) -> String {
        self.request_id.fetch_add(1, Ordering::SeqCst).to_string()
    }

    /// Send a request to signald.
    async fn send_request(&self, request: &SignaldRequest) -> Result<(), ChannelError> {
        let json = serde_json::to_string(request)
            .map_err(|e| ChannelError::Send(format!("JSON error: {e}")))?;

        debug!(request = %json, "Sending to signald");

        let mut writer = self.writer.lock().await;
        let writer = writer
            .as_mut()
            .ok_or_else(|| ChannelError::Connection("Not connected".to_string()))?;

        writer
            .write_all(json.as_bytes())
            .await
            .map_err(|e| ChannelError::Send(e.to_string()))?;
        writer
            .write_all(b"\n")
            .await
            .map_err(|e| ChannelError::Send(e.to_string()))?;
        writer
            .flush()
            .await
            .map_err(|e| ChannelError::Send(e.to_string()))?;

        Ok(())
    }

    /// Send a request and wait for response.
    async fn request<T: DeserializeOwned>(&self, request: SignaldRequest) -> Result<T, ChannelError> {
        let id = request.id.clone().unwrap_or_default();
        
        // Set up response channel
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut pending = self.pending.write().await;
            pending.insert(id.clone(), tx);
        }

        // Send request
        self.send_request(&request).await?;

        // Wait for response with timeout
        let response = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .map_err(|_| ChannelError::Send("Timeout waiting for signald response".to_string()))?
            .map_err(|_| ChannelError::Send("Response channel closed".to_string()))?;

        // Check for error
        if let Some(error) = response.get("error") {
            return Err(ChannelError::Send(format!("signald error: {error}")));
        }

        serde_json::from_value(response)
            .map_err(|e| ChannelError::Send(format!("Failed to parse response: {e}")))
    }

    /// Reader task - processes incoming messages from signald.
    async fn reader_task(
        reader: tokio::io::ReadHalf<tokio::net::TcpStream>,
        pending: Arc<RwLock<HashMap<String, tokio::sync::oneshot::Sender<serde_json::Value>>>>,
        incoming_tx: Option<mpsc::Sender<IncomingMessage>>,
        account: String,
    ) {
        let mut lines = BufReader::new(reader).lines();

        while let Ok(Some(line)) = lines.next_line().await {
            debug!(line = %line, "Received from signald");

            let msg: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(e) => {
                    warn!(error = %e, "Failed to parse signald message");
                    continue;
                }
            };

            // Check if this is a response to a pending request
            if let Some(id) = msg.get("id").and_then(|v| v.as_str()) {
                let mut pending = pending.write().await;
                if let Some(tx) = pending.remove(id) {
                    let _ = tx.send(msg);
                    continue;
                }
            }

            // Check if this is an incoming message
            let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
            
            if msg_type == "IncomingMessage" {
                if let Some(ref tx) = incoming_tx {
                    if let Some(incoming) = Self::parse_incoming_message(&msg, &account) {
                        if tx.send(incoming).await.is_err() {
                            warn!("Failed to send incoming message to channel");
                        }
                    }
                }
            }
        }

        info!("signald reader task ended");
    }

    /// Parse an incoming message from signald.
    fn parse_incoming_message(msg: &serde_json::Value, account: &str) -> Option<IncomingMessage> {
        let data = msg.get("data")?;
        let envelope = data.get("data_message")?;
        
        let body = envelope.get("body")?.as_str()?;
        let timestamp = envelope.get("timestamp")?.as_i64()?;
        let source = data.get("source")?.get("number")?.as_str()?;
        
        Some(IncomingMessage {
            id: timestamp.to_string(),
            channel: crate::ChannelId::new("signal", account),
            sender: Sender {
                id: source.to_string(),
                name: None,
                username: Some(source.to_string()),
            },
            content: MessageContent::Text { text: body.to_string() },
            timestamp,
            reply_to: None,
        })
    }

    // ========================================================================
    // Message Operations
    // ========================================================================

    /// Send a message to a recipient.
    pub async fn send_message(
        &self,
        recipient: &str,
        text: &str,
    ) -> Result<SignalSendResult, ChannelError> {
        debug!(recipient, "Sending Signal message");

        let request = SignaldRequest {
            r#type: "send".to_string(),
            id: Some(self.next_id()),
            version: "v1".to_string(),
            account: Some(self.account.clone()),
            recipient_address: Some(SignalAddress {
                number: Some(recipient.to_string()),
                uuid: None,
            }),
            message_body: Some(text.to_string()),
            ..Default::default()
        };

        self.request(request).await
    }

    /// Send a message to a group.
    pub async fn send_group_message(
        &self,
        group_id: &str,
        text: &str,
    ) -> Result<SignalSendResult, ChannelError> {
        debug!(group_id, "Sending Signal group message");

        let request = SignaldRequest {
            r#type: "send".to_string(),
            id: Some(self.next_id()),
            version: "v1".to_string(),
            account: Some(self.account.clone()),
            recipient_group_id: Some(group_id.to_string()),
            message_body: Some(text.to_string()),
            ..Default::default()
        };

        self.request(request).await
    }

    /// React to a message.
    pub async fn react(
        &self,
        recipient: &str,
        target_timestamp: i64,
        emoji: &str,
    ) -> Result<(), ChannelError> {
        debug!(recipient, target_timestamp, emoji, "Sending Signal reaction");

        let request = SignaldRequest {
            r#type: "react".to_string(),
            id: Some(self.next_id()),
            version: "v1".to_string(),
            account: Some(self.account.clone()),
            recipient_address: Some(SignalAddress {
                number: Some(recipient.to_string()),
                uuid: None,
            }),
            reaction: Some(SignalReaction {
                emoji: emoji.to_string(),
                target_author: SignalAddress {
                    number: Some(recipient.to_string()),
                    uuid: None,
                },
                target_sent_timestamp: target_timestamp,
                remove: false,
            }),
            ..Default::default()
        };

        self.send_request(&request).await
    }

    /// Remove a reaction from a message.
    pub async fn unreact(
        &self,
        recipient: &str,
        target_timestamp: i64,
        emoji: &str,
    ) -> Result<(), ChannelError> {
        let request = SignaldRequest {
            r#type: "react".to_string(),
            id: Some(self.next_id()),
            version: "v1".to_string(),
            account: Some(self.account.clone()),
            recipient_address: Some(SignalAddress {
                number: Some(recipient.to_string()),
                uuid: None,
            }),
            reaction: Some(SignalReaction {
                emoji: emoji.to_string(),
                target_author: SignalAddress {
                    number: Some(recipient.to_string()),
                    uuid: None,
                },
                target_sent_timestamp: target_timestamp,
                remove: true,
            }),
            ..Default::default()
        };

        self.send_request(&request).await
    }

    /// Get linked devices.
    pub async fn get_linked_devices(&self) -> Result<Vec<SignalDevice>, ChannelError> {
        let request = SignaldRequest {
            r#type: "get_linked_devices".to_string(),
            id: Some(self.next_id()),
            version: "v1".to_string(),
            account: Some(self.account.clone()),
            ..Default::default()
        };

        #[derive(Deserialize)]
        struct Response {
            devices: Vec<SignalDevice>,
        }

        let result: Response = self.request(request).await?;
        Ok(result.devices)
    }

    /// List groups.
    pub async fn list_groups(&self) -> Result<Vec<SignalGroup>, ChannelError> {
        let request = SignaldRequest {
            r#type: "list_groups".to_string(),
            id: Some(self.next_id()),
            version: "v1".to_string(),
            account: Some(self.account.clone()),
            ..Default::default()
        };

        #[derive(Deserialize)]
        struct Response {
            groups: Vec<SignalGroup>,
        }

        let result: Response = self.request(request).await?;
        Ok(result.groups)
    }
}

// ============================================================================
// Channel Trait Implementation
// ============================================================================

#[async_trait]
impl Channel for SignalChannel {
    fn provider(&self) -> &str {
        "signal"
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            features: ChannelFeatures::REACTIONS
                | ChannelFeatures::REPLIES
                | ChannelFeatures::IMAGES
                | ChannelFeatures::DOCUMENTS,
            max_message_length: Some(65536), // Signal supports long messages
        }
    }

    async fn send(&self, message: OutgoingMessage) -> Result<String, ChannelError> {
        let recipient = &message.channel.id;

        let text = match message.content {
            MessageContent::Text { text } => text,
            MessageContent::Image { url, caption } => {
                format!("{}\n{url}", caption.unwrap_or_default())
            }
            _ => {
                return Err(ChannelError::Send(
                    "Unsupported message type for Signal".to_string(),
                ))
            }
        };

        // Check if recipient looks like a group ID
        let result = if recipient.starts_with("group:") {
            self.send_group_message(&recipient[6..], &text).await?
        } else {
            self.send_message(recipient, &text).await?
        };

        Ok(result.timestamp.to_string())
    }

    async fn react(&self, message_id: &str, emoji: &str) -> Result<(), ChannelError> {
        // message_id format: "recipient:timestamp"
        let (recipient, ts) = parse_message_id(message_id)?;
        self.react(&recipient, ts, emoji).await
    }

    async fn unreact(&self, message_id: &str, emoji: &str) -> Result<(), ChannelError> {
        let (recipient, ts) = parse_message_id(message_id)?;
        self.unreact(&recipient, ts, emoji).await
    }
}

/// Parse message ID in format "recipient:timestamp"
fn parse_message_id(id: &str) -> Result<(String, i64), ChannelError> {
    let parts: Vec<&str> = id.rsplitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(ChannelError::Send(format!(
            "Invalid message ID format: {id} (expected recipient:timestamp)"
        )));
    }
    let ts: i64 = parts[0]
        .parse()
        .map_err(|_| ChannelError::Send(format!("Invalid timestamp: {}", parts[0])))?;
    Ok((parts[1].to_string(), ts))
}

// ============================================================================
// Request/Response Types
// ============================================================================

/// signald request structure.
#[derive(Debug, Clone, Default, Serialize)]
struct SignaldRequest {
    #[serde(rename = "type")]
    r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    account: Option<String>,
    #[serde(rename = "recipientAddress", skip_serializing_if = "Option::is_none")]
    recipient_address: Option<SignalAddress>,
    #[serde(rename = "recipientGroupId", skip_serializing_if = "Option::is_none")]
    recipient_group_id: Option<String>,
    #[serde(rename = "messageBody", skip_serializing_if = "Option::is_none")]
    message_body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reaction: Option<SignalReaction>,
}

/// Signal address (phone number or UUID).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SignalAddress {
    #[serde(skip_serializing_if = "Option::is_none")]
    number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uuid: Option<String>,
}

/// Signal reaction.
#[derive(Debug, Clone, Serialize)]
struct SignalReaction {
    emoji: String,
    #[serde(rename = "targetAuthor")]
    target_author: SignalAddress,
    #[serde(rename = "targetSentTimestamp")]
    target_sent_timestamp: i64,
    remove: bool,
}

/// Send result.
#[derive(Debug, Clone, Deserialize)]
pub struct SignalSendResult {
    pub timestamp: i64,
}

/// Signal device.
#[derive(Debug, Clone, Deserialize)]
pub struct SignalDevice {
    pub id: i32,
    pub name: Option<String>,
    pub created: Option<i64>,
    pub last_seen: Option<i64>,
}

/// Signal group.
#[derive(Debug, Clone, Deserialize)]
pub struct SignalGroup {
    pub id: String,
    pub name: Option<String>,
    #[serde(rename = "memberCount")]
    pub member_count: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_message_id() {
        let (recipient, ts) = parse_message_id("+1234567890:1234567890123").unwrap();
        assert_eq!(recipient, "+1234567890");
        assert_eq!(ts, 1234567890123);
    }

    #[test]
    fn test_parse_invalid_message_id() {
        assert!(parse_message_id("invalid").is_err());
        assert!(parse_message_id("+1234567890:notanumber").is_err());
    }
}
