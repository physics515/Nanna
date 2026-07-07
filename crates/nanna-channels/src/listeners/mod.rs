//! Channel Listeners - Inbound message receivers
//!
//! Each listener connects to a messaging platform and pushes incoming
//! messages to the MessageRouter for processing.
//!
//! # Available Listeners
//!
//! - `TelegramListener` - Long-polling via getUpdates API
//! - `DiscordListener` - WebSocket via Discord Gateway
//! - `SlackListener` - WebSocket via Slack Socket Mode
//! - `SignalListener` - signal-cli-rest-api (SSE or polling)
//! - `WhatsAppWebListener` - WhatsApp Web bridge (WebSocket, SSE, or polling)

pub mod circuit_breaker;
pub mod discord;
pub mod signal;
pub mod slack;
pub mod telegram;
pub mod whatsapp;

pub use circuit_breaker::{CircuitBreaker, BreakerAction};
pub use discord::DiscordListener;
pub use signal::{SignalListener, ReceiveMode as SignalReceiveMode};
pub use slack::SlackListener;
pub use telegram::TelegramListener;
pub use whatsapp::{WhatsAppWebListener, ReceiveMode as WhatsAppReceiveMode};

use crate::IncomingMessage;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Trait for channel listeners
#[async_trait]
pub trait Listener: Send + Sync {
    /// Get the provider name
    fn provider(&self) -> &str;

    /// Start listening for messages
    /// Returns a handle that can be used to stop the listener
    async fn start(
        self: Arc<Self>,
        sender: mpsc::Sender<IncomingMessage>,
    ) -> Result<ListenerHandle, ListenerError>;
}

/// Handle to control a running listener
pub struct ListenerHandle {
    /// Shutdown signal sender
    shutdown_tx: mpsc::Sender<()>,
    /// Join handle for the listener task
    join_handle: tokio::task::JoinHandle<()>,
}

impl ListenerHandle {
    /// Create a new listener handle
    pub fn new(shutdown_tx: mpsc::Sender<()>, join_handle: tokio::task::JoinHandle<()>) -> Self {
        Self {
            shutdown_tx,
            join_handle,
        }
    }

    /// Stop the listener gracefully
    pub async fn stop(self) {
        // Send shutdown signal
        let _ = self.shutdown_tx.send(()).await;
        // Wait for task to finish
        let _ = self.join_handle.await;
    }

    /// Check if the listener is still running
    pub fn is_running(&self) -> bool {
        !self.join_handle.is_finished()
    }
}

/// Listener errors
#[derive(Debug, thiserror::Error)]
pub enum ListenerError {
    #[error("Connection failed: {0}")]
    Connection(String),

    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error("Already running")]
    AlreadyRunning,

    #[error("Shutdown: {0}")]
    Shutdown(String),

    #[error("API error: {0}")]
    Api(String),
}

/// Listener manager - coordinates multiple listeners
pub struct ListenerManager {
    listeners: Vec<(String, ListenerHandle)>,
    message_tx: mpsc::Sender<IncomingMessage>,
    message_rx: Option<mpsc::Receiver<IncomingMessage>>,
}

impl ListenerManager {
    /// Create a new listener manager
    pub fn new(buffer_size: usize) -> Self {
        let (tx, rx) = mpsc::channel(buffer_size);
        Self {
            listeners: Vec::new(),
            message_tx: tx,
            message_rx: Some(rx),
        }
    }

    /// Get the message sender (for adding listeners)
    pub fn sender(&self) -> mpsc::Sender<IncomingMessage> {
        self.message_tx.clone()
    }

    /// Take the message receiver (can only be called once)
    pub fn take_receiver(&mut self) -> Option<mpsc::Receiver<IncomingMessage>> {
        self.message_rx.take()
    }

    /// Add and start a listener
    pub async fn add<L: Listener + 'static>(
        &mut self,
        listener: Arc<L>,
    ) -> Result<(), ListenerError> {
        let provider = listener.provider().to_string();
        let handle = listener.start(self.message_tx.clone()).await?;
        self.listeners.push((provider, handle));
        Ok(())
    }

    /// Stop a specific listener by provider name
    pub async fn stop(&mut self, provider: &str) -> bool {
        if let Some(idx) = self.listeners.iter().position(|(p, _)| p == provider) {
            let (_, handle) = self.listeners.remove(idx);
            handle.stop().await;
            true
        } else {
            false
        }
    }

    /// Stop all listeners
    pub async fn stop_all(&mut self) {
        for (_, handle) in self.listeners.drain(..) {
            handle.stop().await;
        }
    }

    /// List running listeners
    pub fn list(&self) -> Vec<&str> {
        self.listeners
            .iter()
            .filter(|(_, h)| h.is_running())
            .map(|(p, _)| p.as_str())
            .collect()
    }
}
