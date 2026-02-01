#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! Channel abstraction for Nanna
//!
//! Provides a unified interface for different messaging platforms.
//!
//! # Architecture
//!
//! Channels have two parts:
//! - **Outbound** (`Channel` trait): Send messages, reactions, edits, etc.
//! - **Inbound** (`Listener` trait): Receive messages via polling or webhooks
//!
//! The `MessageRouter` coordinates outbound sends.
//! The `ListenerManager` coordinates inbound listeners.

pub mod discord;
pub mod listeners;
pub mod queue;
pub mod signal;
pub mod slack;
pub mod status;
pub mod telegram;
pub mod whatsapp;

pub use discord::DiscordChannel;
pub use listeners::{
    DiscordListener, Listener, ListenerError, ListenerHandle, ListenerManager, SignalListener,
    SignalReceiveMode, SlackListener, TelegramListener,
};
pub use queue::{MessageQueue, MessagePriority, QueueConfig, QueueEvent, QueueStats, QueuedMessage, SendResult};
pub use signal::SignalChannel;
pub use slack::SlackChannel;
pub use status::{ChannelStatus, ConnectionState, HealthChecker, HealthCheckResult, HealthMetrics, StatusEvent, StatusManager, StatusSummary};
pub use telegram::TelegramChannel;
pub use whatsapp::WhatsAppChannel;

use async_trait::async_trait;
use bitflags::bitflags;
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
    Video { url: String, duration_secs: Option<f32>, caption: Option<String> },
    Document { url: String, filename: String },
    Location { latitude: f64, longitude: f64 },
    Poll { question: String, options: Vec<String>, multiple: bool },
    Sticker { id: String, emoji: Option<String> },
}

/// Attachment for messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub url: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub size_bytes: Option<u64>,
}

/// Thread information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadInfo {
    pub id: String,
    pub name: Option<String>,
    pub message_count: Option<u32>,
}

/// Reaction information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reaction {
    pub emoji: String,
    pub user_id: String,
    pub timestamp: Option<i64>,
}

impl MessageContent {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    #[must_use] 
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            _ => None,
        }
    }
}

bitflags! {
    /// Flags representing supported channel features.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct ChannelFeatures: u16 {
        const REACTIONS  = 0b0000_0000_0001;
        const REPLIES    = 0b0000_0000_0010;
        const EDITS      = 0b0000_0000_0100;
        const THREADS    = 0b0000_0000_1000;
        const IMAGES     = 0b0000_0001_0000;
        const AUDIO      = 0b0000_0010_0000;
        const DOCUMENTS  = 0b0000_0100_0000;
        const POLLS      = 0b0000_1000_0000;
        const PINS       = 0b0001_0000_0000;
        const TYPING     = 0b0010_0000_0000;
        const UPLOADS    = 0b0100_0000_0000;
        const STICKERS   = 0b1000_0000_0000;
    }
}

/// Channel capabilities.
#[derive(Debug, Clone, Default)]
pub struct ChannelCapabilities {
    /// Supported features (reactions, replies, etc.).
    pub features: ChannelFeatures,
    /// Maximum message length, if any.
    pub max_message_length: Option<usize>,
}

impl ChannelCapabilities {
    /// Check if reactions are supported.
    #[must_use]
    pub const fn supports_reactions(&self) -> bool {
        self.features.contains(ChannelFeatures::REACTIONS)
    }

    /// Check if replies are supported.
    #[must_use]
    pub const fn supports_replies(&self) -> bool {
        self.features.contains(ChannelFeatures::REPLIES)
    }

    /// Check if edits are supported.
    #[must_use]
    pub const fn supports_edits(&self) -> bool {
        self.features.contains(ChannelFeatures::EDITS)
    }

    /// Check if threads are supported.
    #[must_use]
    pub const fn supports_threads(&self) -> bool {
        self.features.contains(ChannelFeatures::THREADS)
    }

    /// Check if images are supported.
    #[must_use]
    pub const fn supports_images(&self) -> bool {
        self.features.contains(ChannelFeatures::IMAGES)
    }

    /// Check if audio is supported.
    #[must_use]
    pub const fn supports_audio(&self) -> bool {
        self.features.contains(ChannelFeatures::AUDIO)
    }

    /// Check if documents are supported.
    #[must_use]
    pub const fn supports_documents(&self) -> bool {
        self.features.contains(ChannelFeatures::DOCUMENTS)
    }

    /// Check if polls are supported.
    #[must_use]
    pub const fn supports_polls(&self) -> bool {
        self.features.contains(ChannelFeatures::POLLS)
    }

    /// Check if pins are supported.
    #[must_use]
    pub const fn supports_pins(&self) -> bool {
        self.features.contains(ChannelFeatures::PINS)
    }

    /// Check if typing indicators are supported.
    #[must_use]
    pub const fn supports_typing(&self) -> bool {
        self.features.contains(ChannelFeatures::TYPING)
    }

    /// Check if file uploads are supported.
    #[must_use]
    pub const fn supports_uploads(&self) -> bool {
        self.features.contains(ChannelFeatures::UPLOADS)
    }

    /// Check if stickers are supported.
    #[must_use]
    pub const fn supports_stickers(&self) -> bool {
        self.features.contains(ChannelFeatures::STICKERS)
    }
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

    /// Remove a reaction from a message
    async fn unreact(&self, _message_id: &str, _emoji: &str) -> Result<(), ChannelError> {
        Err(ChannelError::Send("Reactions not supported".into()))
    }

    /// Get reactions on a message
    async fn get_reactions(&self, _message_id: &str) -> Result<Vec<Reaction>, ChannelError> {
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

    /// Pin a message
    async fn pin(&self, _message_id: &str) -> Result<(), ChannelError> {
        Err(ChannelError::Send("Pins not supported".into()))
    }

    /// Unpin a message
    async fn unpin(&self, _message_id: &str) -> Result<(), ChannelError> {
        Err(ChannelError::Send("Pins not supported".into()))
    }

    /// Create a thread from a message
    async fn create_thread(&self, _message_id: &str, _name: &str) -> Result<ThreadInfo, ChannelError> {
        Err(ChannelError::Send("Threads not supported".into()))
    }

    /// Reply in a thread
    async fn reply_thread(&self, _thread_id: &str, _content: MessageContent) -> Result<String, ChannelError> {
        Err(ChannelError::Send("Threads not supported".into()))
    }

    /// Send typing indicator
    async fn send_typing(&self, _channel_id: &ChannelId) -> Result<(), ChannelError> {
        Ok(()) // Silently succeed if not supported
    }

    /// Upload a file and return its URL
    async fn upload_file(&self, _filename: &str, _data: &[u8], _content_type: &str) -> Result<String, ChannelError> {
        Err(ChannelError::Send("File uploads not supported".into()))
    }

    /// Send a poll
    async fn send_poll(
        &self,
        _channel: &ChannelId,
        _question: &str,
        _options: &[String],
        _multiple: bool,
    ) -> Result<String, ChannelError> {
        Err(ChannelError::Send("Polls not supported".into()))
    }
}

/// Message router for multiple channels
pub struct MessageRouter {
    channels: std::collections::HashMap<String, Box<dyn Channel>>,
    incoming_tx: flume::Sender<IncomingMessage>,
    incoming_rx: flume::Receiver<IncomingMessage>,
}

impl MessageRouter {
    #[must_use] 
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
    #[must_use] 
    pub fn get(&self, name: &str) -> Option<&dyn Channel> {
        self.channels.get(name).map(std::convert::AsRef::as_ref)
    }

    /// Get the incoming message sender (for channel implementations)
    #[must_use] 
    pub fn incoming_sender(&self) -> flume::Sender<IncomingMessage> {
        self.incoming_tx.clone()
    }

    /// Receive incoming messages
    pub async fn recv(&self) -> Option<IncomingMessage> {
        self.incoming_rx.recv_async().await.ok()
    }

    /// Send a message through the appropriate channel.
    ///
    /// # Errors
    ///
    /// Returns `ChannelError::NotFound` if the channel provider is not registered.
    /// May also return errors from the underlying channel implementation.
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
