#![warn(clippy::all, clippy::restriction)]
#![deny(clippy::pedantic, clippy::nursery)]

//! Channel abstraction for Nanna
//!
//! Provides a unified interface for different messaging platforms.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ChannelError {
    #[error("Connection failed: {0}")]
    Connection(String),
    #[error("Send failed: {0}")]
    Send(String),
    #[error("Receive failed: {0}")]
    Receive(String),
    #[error("Authentication failed: {0}")]
    Auth(String),
    #[error("Rate limited")]
    RateLimited,
    #[error("Channel not found: {0}")]
    NotFound(String),
}

/// Unique identifier for a channel
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChannelId {
    pub provider: String,
    pub id: String,
}

impl ChannelId {
    pub fn new(provider: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            id: id.into(),
        }
    }
}

/// Incoming message from any channel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingMessage {
    pub id: String,
    pub channel: ChannelId,
    pub sender: Sender,
    pub content: MessageContent,
    pub timestamp: i64,
    pub reply_to: Option<String>,
}

/// Outgoing message to any channel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutgoingMessage {
    pub channel: ChannelId,
    pub content: MessageContent,
    pub reply_to: Option<String>,
}

/// Message sender info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sender {
    pub id: String,
    pub name: Option<String>,
    pub username: Option<String>,
}

/// Message content types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageContent {
    Text { text: String },
    Image { url: String, caption: Option<String> },
    Audio { url: String, duration_secs: Option<f32> },
    Document { url: String, filename: String },
    Location { latitude: f64, longitude: f64 },
}

impl MessageContent {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            _ => None,
        }
    }
}

/// Channel capabilities
#[derive(Debug, Clone, Default)]
pub struct ChannelCapabilities {
    pub supports_reactions: bool,
    pub supports_replies: bool,
    pub supports_edits: bool,
    pub supports_threads: bool,
    pub supports_images: bool,
    pub supports_audio: bool,
    pub supports_documents: bool,
    pub max_message_length: Option<usize>,
}

/// Trait for channel implementations
#[async_trait]
pub trait Channel: Send + Sync {
    /// Get the provider name
    fn provider(&self) -> &str;

    /// Get channel capabilities
    fn capabilities(&self) -> ChannelCapabilities;

    /// Send a message
    async fn send(&self, message: OutgoingMessage) -> Result<String, ChannelError>;

    /// React to a message
    async fn react(&self, _message_id: &str, _emoji: &str) -> Result<(), ChannelError> {
        Err(ChannelError::Send("Reactions not supported".into()))
    }

    /// Edit a message
    async fn edit(&self, _message_id: &str, _content: MessageContent) -> Result<(), ChannelError> {
        Err(ChannelError::Send("Edits not supported".into()))
    }

    /// Delete a message
    async fn delete(&self, _message_id: &str) -> Result<(), ChannelError> {
        Err(ChannelError::Send("Deletes not supported".into()))
    }
}

/// Message router for multiple channels
pub struct MessageRouter {
    channels: std::collections::HashMap<String, Box<dyn Channel>>,
    incoming_tx: flume::Sender<IncomingMessage>,
    incoming_rx: flume::Receiver<IncomingMessage>,
}

impl MessageRouter {
    pub fn new() -> Self {
        let (incoming_tx, incoming_rx) = flume::unbounded();
        Self {
            channels: std::collections::HashMap::new(),
            incoming_tx,
            incoming_rx,
        }
    }

    /// Register a channel
    pub fn register(&mut self, name: impl Into<String>, channel: Box<dyn Channel>) {
        self.channels.insert(name.into(), channel);
    }

    /// Get a channel by name
    pub fn get(&self, name: &str) -> Option<&dyn Channel> {
        self.channels.get(name).map(|c| c.as_ref())
    }

    /// Get the incoming message sender (for channel implementations)
    pub fn incoming_sender(&self) -> flume::Sender<IncomingMessage> {
        self.incoming_tx.clone()
    }

    /// Receive incoming messages
    pub async fn recv(&self) -> Option<IncomingMessage> {
        self.incoming_rx.recv_async().await.ok()
    }

    /// Send a message through the appropriate channel
    pub async fn send(&self, message: OutgoingMessage) -> Result<String, ChannelError> {
        let channel = self
            .channels
            .get(&message.channel.provider)
            .ok_or_else(|| ChannelError::NotFound(message.channel.provider.clone()))?;

        channel.send(message).await
    }
}

impl Default for MessageRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_id() {
        let id = ChannelId::new("telegram", "123456");
        assert_eq!(id.provider, "telegram");
        assert_eq!(id.id, "123456");
    }

    #[test]
    fn test_message_content() {
        let content = MessageContent::text("Hello, world!");
        assert_eq!(content.as_text(), Some("Hello, world!"));
    }
}
