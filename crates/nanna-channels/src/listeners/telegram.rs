//! Telegram Long-Polling Listener
//!
//! Connects to Telegram's getUpdates API and pushes incoming messages
//! to the message router.

use super::{Listener, ListenerError, ListenerHandle};
use crate::{ChannelId, IncomingMessage, MessageContent, Sender};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

const TELEGRAM_API_BASE: &str = "https://api.telegram.org";
const LONG_POLL_TIMEOUT: u64 = 30;
const MAX_RETRY_DELAY: u64 = 60;

/// Telegram long-polling listener
pub struct TelegramListener {
    client: Client,
    bot_token: String,
    /// Last processed update ID
    offset: AtomicI64,
    /// Allowed chat IDs (empty = allow all)
    allowed_chats: Vec<i64>,
}

impl TelegramListener {
    /// Create a new Telegram listener
    pub fn new(bot_token: impl Into<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(LONG_POLL_TIMEOUT + 10))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            bot_token: bot_token.into(),
            offset: AtomicI64::new(0),
            allowed_chats: Vec::new(),
        }
    }

    /// Only process messages from specific chats
    #[must_use]
    pub fn with_allowed_chats(mut self, chats: Vec<i64>) -> Self {
        self.allowed_chats = chats;
        self
    }

    /// Set the starting offset (for resuming after restart)
    #[must_use]
    pub fn with_offset(self, offset: i64) -> Self {
        self.offset.store(offset, Ordering::SeqCst);
        self
    }

    /// Build API URL
    fn api_url(&self, method: &str) -> String {
        format!("{}/bot{}/{}", TELEGRAM_API_BASE, self.bot_token, method)
    }

    /// Fetch updates from Telegram
    async fn get_updates(&self) -> Result<Vec<TelegramUpdate>, ListenerError> {
        let offset = self.offset.load(Ordering::SeqCst);

        let response = self
            .client
            .get(self.api_url("getUpdates"))
            .query(&[
                ("offset", offset.to_string()),
                ("timeout", LONG_POLL_TIMEOUT.to_string()),
                ("allowed_updates", r#"["message","edited_message"]"#.to_string()),
            ])
            .send()
            .await
            .map_err(|e| ListenerError::Connection(e.to_string()))?;

        let status = response.status();
        if status == 401 {
            return Err(ListenerError::Auth("Invalid bot token".to_string()));
        }

        let body: TelegramApiResponse<Vec<TelegramUpdate>> = response
            .json()
            .await
            .map_err(|e| ListenerError::Api(format!("Failed to parse response: {e}")))?;

        if !body.ok {
            return Err(ListenerError::Api(
                body.description.unwrap_or_else(|| format!("HTTP {status}")),
            ));
        }

        Ok(body.result.unwrap_or_default())
    }

    /// Convert a Telegram update to an IncomingMessage
    fn convert_update(&self, update: &TelegramUpdate) -> Option<IncomingMessage> {
        let message = update.message.as_ref().or(update.edited_message.as_ref())?;

        // Check if chat is allowed
        if !self.allowed_chats.is_empty() && !self.allowed_chats.contains(&message.chat.id) {
            debug!(
                "Ignoring message from non-allowed chat {}",
                message.chat.id
            );
            return None;
        }

        let sender = message.from.as_ref()?;
        let content = self.extract_content(message)?;

        Some(IncomingMessage {
            id: format!("{}:{}", message.chat.id, message.message_id),
            channel: ChannelId::new("telegram", message.chat.id.to_string()),
            sender: Sender {
                id: sender.id.to_string(),
                name: Some(format!(
                    "{}{}",
                    sender.first_name,
                    sender
                        .last_name
                        .as_ref()
                        .map(|l| format!(" {l}"))
                        .unwrap_or_default()
                )),
                username: sender.username.clone(),
            },
            content,
            timestamp: message.date,
            reply_to: message
                .reply_to_message
                .as_ref()
                .map(|r| format!("{}:{}", message.chat.id, r.message_id)),
        })
    }

    /// Extract message content
    fn extract_content(&self, message: &TelegramMessage) -> Option<MessageContent> {
        // Text message
        if let Some(text) = &message.text {
            return Some(MessageContent::Text { text: text.clone() });
        }

        // Photo
        if let Some(photos) = &message.photo {
            if let Some(largest) = photos.last() {
                return Some(MessageContent::Image {
                    url: largest.file_id.clone(), // Will need to be resolved via getFile
                    caption: message.caption.clone(),
                });
            }
        }

        // Document
        if let Some(doc) = &message.document {
            return Some(MessageContent::Document {
                url: doc.file_id.clone(),
                filename: doc.file_name.clone().unwrap_or_else(|| "document".to_string()),
            });
        }

        // Audio
        if let Some(audio) = &message.audio {
            return Some(MessageContent::Audio {
                url: audio.file_id.clone(),
                duration_secs: Some(audio.duration as f32),
            });
        }

        // Voice
        if let Some(voice) = &message.voice {
            return Some(MessageContent::Audio {
                url: voice.file_id.clone(),
                duration_secs: Some(voice.duration as f32),
            });
        }

        // Video
        if let Some(video) = &message.video {
            return Some(MessageContent::Video {
                url: video.file_id.clone(),
                duration_secs: Some(video.duration as f32),
                caption: message.caption.clone(),
            });
        }

        // Location
        if let Some(loc) = &message.location {
            return Some(MessageContent::Location {
                latitude: loc.latitude,
                longitude: loc.longitude,
            });
        }

        // Sticker
        if let Some(sticker) = &message.sticker {
            return Some(MessageContent::Sticker {
                id: sticker.file_id.clone(),
                emoji: sticker.emoji.clone(),
            });
        }

        // Caption-only (for media without specific handling)
        if let Some(caption) = &message.caption {
            return Some(MessageContent::Text {
                text: caption.clone(),
            });
        }

        None
    }

    /// Run the polling loop
    async fn poll_loop(
        self: Arc<Self>,
        sender: mpsc::Sender<IncomingMessage>,
        mut shutdown_rx: mpsc::Receiver<()>,
    ) {
        info!("Telegram listener started");

        let mut retry_delay = Duration::from_secs(1);

        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    info!("Telegram listener received shutdown signal");
                    break;
                }
                result = self.get_updates() => {
                    match result {
                        Ok(updates) => {
                            retry_delay = Duration::from_secs(1); // Reset on success

                            for update in updates {
                                // Update offset to acknowledge this update
                                self.offset.store(update.update_id + 1, Ordering::SeqCst);

                                if let Some(message) = self.convert_update(&update) {
                                    debug!("Received message: {:?}", message.id);

                                    if sender.send(message).await.is_err() {
                                        error!("Failed to send message to router (receiver dropped)");
                                        return;
                                    }
                                }
                            }
                        }
                        Err(ListenerError::Auth(e)) => {
                            error!("Telegram auth failed: {e}");
                            // Don't retry auth errors
                            break;
                        }
                        Err(e) => {
                            warn!("Telegram poll error: {e}, retrying in {:?}", retry_delay);
                            tokio::time::sleep(retry_delay).await;
                            retry_delay = (retry_delay * 2).min(Duration::from_secs(MAX_RETRY_DELAY));
                        }
                    }
                }
            }
        }

        info!("Telegram listener stopped");
    }
}

#[async_trait]
impl Listener for TelegramListener {
    fn provider(&self) -> &str {
        "telegram"
    }

    async fn start(
        self: Arc<Self>,
        sender: mpsc::Sender<IncomingMessage>,
    ) -> Result<ListenerHandle, ListenerError> {
        // Delete any existing webhook to enable polling
        let _ = self
            .client
            .post(self.api_url("deleteWebhook"))
            .send()
            .await;

        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

        let listener = Arc::clone(&self);
        let handle = tokio::spawn(async move {
            listener.poll_loop(sender, shutdown_rx).await;
        });

        Ok(ListenerHandle::new(shutdown_tx, handle))
    }
}

// ============================================================================
// Telegram API Types
// ============================================================================

#[derive(Debug, Deserialize)]
struct TelegramApiResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    update_id: i64,
    message: Option<TelegramMessage>,
    edited_message: Option<TelegramMessage>,
}

#[derive(Debug, Deserialize)]
struct TelegramMessage {
    message_id: i64,
    chat: TelegramChat,
    from: Option<TelegramUser>,
    date: i64,
    text: Option<String>,
    caption: Option<String>,
    photo: Option<Vec<TelegramPhotoSize>>,
    document: Option<TelegramDocument>,
    audio: Option<TelegramAudio>,
    voice: Option<TelegramVoice>,
    video: Option<TelegramVideo>,
    location: Option<TelegramLocation>,
    sticker: Option<TelegramSticker>,
    reply_to_message: Option<Box<TelegramMessage>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Telegram API response - chat metadata unused (only id is needed)
struct TelegramChat {
    id: i64,
    #[serde(rename = "type")]
    chat_type: String,
    title: Option<String>,
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramUser {
    id: i64,
    first_name: String,
    last_name: Option<String>,
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Telegram API response - dimensions unused (only file_id needed)
struct TelegramPhotoSize {
    file_id: String,
    width: i32,
    height: i32,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Telegram API response - metadata unused (TODO: implement file handling)
struct TelegramDocument {
    file_id: String,
    file_name: Option<String>,
    mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Telegram API response - title unused (TODO: display audio metadata)
struct TelegramAudio {
    file_id: String,
    duration: i32,
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramVoice {
    file_id: String,
    duration: i32,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Telegram API response - dimensions unused (only file_id needed)
struct TelegramVideo {
    file_id: String,
    duration: i32,
    width: i32,
    height: i32,
}

#[derive(Debug, Deserialize)]
struct TelegramLocation {
    latitude: f64,
    longitude: f64,
}

#[derive(Debug, Deserialize)]
struct TelegramSticker {
    file_id: String,
    emoji: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_url() {
        let listener = TelegramListener::new("123:ABC");
        assert_eq!(
            listener.api_url("getUpdates"),
            "https://api.telegram.org/bot123:ABC/getUpdates"
        );
    }

    #[test]
    fn test_with_allowed_chats() {
        let listener = TelegramListener::new("token").with_allowed_chats(vec![123, 456]);
        assert_eq!(listener.allowed_chats, vec![123, 456]);
    }
}
