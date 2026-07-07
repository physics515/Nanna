//! Channel Manager - Coordinates inbound listeners and outbound channels
//!
//! Manages all channel listeners and routes incoming messages to the agent.

use crate::control::ControlPlane;
use crate::protocol::ChatAction;
use nanna_channels::{
    ChannelId, DiscordChannel, DiscordListener, IncomingMessage, ListenerManager,
    MessageContent, MessageRouter, OutgoingMessage, SlackChannel, SlackListener, TelegramChannel,
    TelegramListener,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info};

/// Channel configuration from config.toml
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelsConfig {
    #[serde(default)]
    pub telegram: Option<TelegramConfig>,
    #[serde(default)]
    pub discord: Option<DiscordConfig>,
    #[serde(default)]
    pub slack: Option<SlackConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub bot_token: String,
    #[serde(default)]
    pub allowed_chats: Vec<i64>,
    #[serde(default)]
    pub use_webhooks: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordConfig {
    pub bot_token: String,
    #[serde(default)]
    pub allowed_guilds: Vec<String>,
    #[serde(default)]
    pub intents: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackConfig {
    pub app_token: String,
    pub bot_token: String,
    #[serde(default)]
    pub allowed_channels: Vec<String>,
}

/// Manages all channels (inbound + outbound)
pub struct ChannelManager {
    /// Listener manager for inbound messages
    listener_manager: RwLock<ListenerManager>,
    /// Message router for outbound messages — Arc so it can be shared with spawned tasks
    router: Arc<RwLock<MessageRouter>>,
    /// Control plane reference for processing messages
    control: Arc<ControlPlane>,
    /// Shutdown signal
    shutdown_tx: Option<mpsc::Sender<()>>,
}

impl ChannelManager {
    /// Create a new channel manager
    pub fn new(control: Arc<ControlPlane>) -> Self {
        Self {
            listener_manager: RwLock::new(ListenerManager::new(1000)),
            router: Arc::new(RwLock::new(MessageRouter::new())),
            control,
            shutdown_tx: None,
        }
    }

    /// Configure channels from config
    pub async fn configure(&self, config: &ChannelsConfig) {
        let mut lm = self.listener_manager.write().await;
        let mut router = self.router.write().await;

        // Configure Telegram
        if let Some(tg) = &config.telegram {
            if !tg.use_webhooks {
                let listener = TelegramListener::new(&tg.bot_token)
                    .with_allowed_chats(tg.allowed_chats.clone());
                
                if let Err(e) = lm.add(Arc::new(listener)).await {
                    error!("Failed to start Telegram listener: {}", e);
                } else {
                    info!("Telegram listener configured");
                }
            }

            // Register outbound channel
            router.register(
                "telegram",
                Box::new(TelegramChannel::new(&tg.bot_token)),
            );
        }

        // Configure Discord
        if let Some(dc) = &config.discord {
            let mut listener = DiscordListener::new(&dc.bot_token)
                .with_allowed_guilds(dc.allowed_guilds.clone());
            
            if let Some(intents) = dc.intents {
                listener = listener.with_intents(intents);
            }

            if let Err(e) = lm.add(Arc::new(listener)).await {
                error!("Failed to start Discord listener: {}", e);
            } else {
                info!("Discord Gateway listener configured");
            }

            // Register outbound channel
            // Note: Discord channel requires application_id for some features
            // For now we use empty string; should be configurable
            router.register(
                "discord",
                Box::new(DiscordChannel::new(&dc.bot_token, "")),
            );
        }

        // Configure Slack
        if let Some(sl) = &config.slack {
            let listener = SlackListener::new(&sl.app_token, &sl.bot_token)
                .with_allowed_channels(sl.allowed_channels.clone());

            if let Err(e) = lm.add(Arc::new(listener)).await {
                error!("Failed to start Slack listener: {}", e);
            } else {
                info!("Slack Socket Mode listener configured");
            }

            // Register outbound channel
            router.register(
                "slack",
                Box::new(SlackChannel::new(&sl.bot_token)),
            );
        }
    }

    /// Register an outbound channel in the router.
    ///
    /// Useful for registering webhook-sourced channels that aren't covered by
    /// the standard `configure()` path (e.g. when bot tokens come from webhook
    /// config rather than channel config).
    pub async fn register_channel(&self, name: impl Into<String>, channel: Box<dyn nanna_channels::Channel>) {
        let mut router = self.router.write().await;
        router.register(name, channel);
    }

    /// Get a clone of the shared router Arc.
    ///
    /// The returned `Arc<RwLock<MessageRouter>>` can be held by external
    /// components (e.g. the webhook processor) to send outbound messages
    /// through registered channels.
    pub fn router(&self) -> Arc<RwLock<MessageRouter>> {
        Arc::clone(&self.router)
    }

    /// Start processing incoming messages
    pub async fn start(&mut self) -> Result<(), String> {
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        // Get the message receiver from listener manager
        let mut message_rx = {
            let mut lm = self.listener_manager.write().await;
            lm.take_receiver()
                .ok_or_else(|| "Listener manager already started".to_string())?
        };

        let control = Arc::clone(&self.control);
        // Share the router Arc with the spawned task — no ownership transfer needed
        let router = Arc::clone(&self.router);

        // Spawn the message processing loop
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        info!("Channel manager shutting down");
                        break;
                    }
                    Some(msg) = message_rx.recv() => {
                        let router_guard = router.read().await;
                        Self::process_message(msg, &control, &router_guard).await;
                    }
                }
            }
        });

        Ok(())
    }

    /// Process an incoming message and send the agent response back to the originating channel.
    ///
    /// This is `pub` so the webhook event processor in `server.rs` can call it
    /// directly after converting a `WebhookEvent` into an `IncomingMessage`.
    pub async fn process_message(msg: IncomingMessage, control: &ControlPlane, router: &MessageRouter) {
        let text = match &msg.content {
            MessageContent::Text { text } => text.clone(),
            _ => {
                debug!("Ignoring non-text message from {}", msg.channel.provider);
                return;
            }
        };

        // Generate deterministic session ID from channel + sender
        let session_id = format!(
            "{}:{}:{}",
            msg.channel.provider, msg.channel.id, msg.sender.id
        );

        info!(
            "Processing {} message from {} in session {}",
            msg.channel.provider,
            msg.sender.username.as_deref().unwrap_or(&msg.sender.id),
            session_id
        );

        // Send to control plane for processing
        let action = ChatAction::Send {
            session_id: session_id.clone(),
            content: text,
            attachments: vec![],
        };

        let response = control
            .handle(&format!("channel:{}", msg.channel.provider), crate::protocol::Action::Chat(action))
            .await;

        // Extract response content
        let response_text = response
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("I encountered an error processing your message.");

        debug!(
            "Response for {}: {}",
            session_id,
            response_text.chars().take(100).collect::<String>()
        );

        // Send response back through the originating channel
        let outgoing = OutgoingMessage {
            channel: msg.channel.clone(),
            content: MessageContent::Text {
                text: response_text.to_string(),
            },
            reply_to: Some(msg.id.clone()),
        };

        if let Err(e) = router.send(outgoing).await {
            error!(
                "Failed to send response to {}:{}: {}",
                msg.channel.provider, msg.channel.id, e
            );
        }
    }

    /// Stop all listeners
    pub async fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(()).await;
        }
        
        let mut lm = self.listener_manager.write().await;
        lm.stop_all().await;
        info!("All channel listeners stopped");
    }

    /// List running listeners
    pub async fn list_listeners(&self) -> Vec<String> {
        let lm = self.listener_manager.read().await;
        lm.list().iter().map(|s| s.to_string()).collect()
    }

    /// Send a message through a channel
    pub async fn send(&self, channel: ChannelId, content: MessageContent) -> Result<String, String> {
        let router = self.router.read().await;
        router
            .send(OutgoingMessage {
                channel,
                content,
                reply_to: None,
            })
            .await
            .map_err(|e| e.to_string())
    }
}
