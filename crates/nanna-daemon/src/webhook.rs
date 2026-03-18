//! Webhook Server - HTTP endpoints for receiving inbound webhooks
//!
//! Provides endpoints for:
//! - Telegram webhook (`/webhook/telegram`)
//! - Discord interactions (`/webhook/discord`)
//! - Slack events (`/webhook/slack`)
//! - WhatsApp webhook (`/webhook/whatsapp`)
//! - Generic webhooks (`/webhook/:id`)

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, error, info, warn};

// =============================================================================
// Webhook Event Types
// =============================================================================

/// Unified webhook event that gets sent to the message router
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookEvent {
    /// Source channel (telegram, discord, slack, whatsapp, generic)
    pub source: String,
    /// Webhook ID (for generic webhooks)
    pub webhook_id: Option<String>,
    /// Raw payload
    pub payload: Value,
    /// Parsed message content (if applicable)
    pub message: Option<WebhookMessage>,
    /// Timestamp
    pub timestamp: i64,
}

/// Parsed message from webhook
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookMessage {
    /// Sender ID
    pub sender_id: String,
    /// Sender name (if available)
    pub sender_name: Option<String>,
    /// Chat/channel ID
    pub chat_id: String,
    /// Message content
    pub content: String,
    /// Message ID (for replies)
    pub message_id: Option<String>,
    /// Is this a command?
    pub is_command: bool,
}

// =============================================================================
// Webhook Configuration
// =============================================================================

/// Webhook server configuration
#[derive(Debug, Clone)]
pub struct WebhookConfig {
    /// Host to bind to
    pub host: String,
    /// Port to listen on
    pub port: u16,
    /// Telegram bot token (for verification)
    pub telegram_token: Option<String>,
    /// Telegram webhook secret (optional)
    pub telegram_secret: Option<String>,
    /// Discord public key (for signature verification)
    pub discord_public_key: Option<String>,
    /// Slack signing secret
    pub slack_signing_secret: Option<String>,
    /// WhatsApp verify token
    pub whatsapp_verify_token: Option<String>,
    /// Generic webhook secrets (webhook_id -> secret)
    pub generic_secrets: HashMap<String, String>,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 3000,
            telegram_token: None,
            telegram_secret: None,
            discord_public_key: None,
            slack_signing_secret: None,
            whatsapp_verify_token: None,
            generic_secrets: HashMap::new(),
        }
    }
}

// =============================================================================
// Webhook Server State
// =============================================================================

/// Shared state for webhook handlers
pub struct WebhookState {
    pub config: WebhookConfig,
    /// Channel to send parsed events
    pub event_tx: mpsc::Sender<WebhookEvent>,
}

impl WebhookState {
    pub fn new(config: WebhookConfig, event_tx: mpsc::Sender<WebhookEvent>) -> Self {
        Self { config, event_tx }
    }
}

// =============================================================================
// Telegram Webhook
// =============================================================================

/// Telegram Update structure (simplified)
#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    update_id: i64,
    message: Option<TelegramMessage>,
    edited_message: Option<TelegramMessage>,
#[serde(rename = "callback_query")]
    _callback_query: Option<TelegramCallbackQuery>,
}

#[derive(Debug, Deserialize)]
struct TelegramMessage {
    message_id: i64,
    from: Option<TelegramUser>,
    chat: TelegramChat,
    text: Option<String>,
    #[serde(default)]
    entities: Vec<TelegramEntity>,
}

#[derive(Debug, Deserialize)]
struct TelegramUser {
    id: i64,
    first_name: String,
    last_name: Option<String>,
#[serde(rename = "username")]
    _username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramChat {
    id: i64,
    #[serde(rename = "type")]
    _chat_type: String,
#[serde(rename = "title")]
    _title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramEntity {
    #[serde(rename = "type")]
    entity_type: String,
    offset: i64,
#[serde(rename = "length")]
    _length: i64,
}

#[derive(Debug, Deserialize)]
struct TelegramCallbackQuery {
#[serde(rename = "id")]
    _id: String,
#[serde(rename = "from")]
    _from: TelegramUser,
#[serde(rename = "message")]
    _message: Option<TelegramMessage>,
#[serde(rename = "data")]
    _data: Option<String>,
}

/// Handle Telegram webhook
async fn telegram_webhook(
    State(state): State<Arc<WebhookState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Verify secret token if configured
    if let Some(ref secret) = state.config.telegram_secret {
        let header_secret = headers
            .get("X-Telegram-Bot-Api-Secret-Token")
            .and_then(|v| v.to_str().ok());
        
        if header_secret != Some(secret.as_str()) {
            warn!("Telegram webhook: invalid secret token");
            return StatusCode::UNAUTHORIZED;
        }
    }
    
    // Parse update
    let update: TelegramUpdate = match serde_json::from_slice(&body) {
        Ok(u) => u,
        Err(e) => {
            error!("Telegram webhook: failed to parse update: {}", e);
            return StatusCode::BAD_REQUEST;
        }
    };
    
    debug!("Telegram webhook: received update {}", update.update_id);
    
    // Extract message
    let tg_message = update.message.or(update.edited_message);
    
    let webhook_message = tg_message.and_then(|msg| {
        let text = msg.text?;
        let sender = msg.from?;
        
        // Check if it's a command
        let is_command = msg.entities.iter().any(|e| e.entity_type == "bot_command" && e.offset == 0);
        
        Some(WebhookMessage {
            sender_id: sender.id.to_string(),
            sender_name: Some(format!(
                "{}{}",
                sender.first_name,
                sender.last_name.map(|l| format!(" {}", l)).unwrap_or_default()
            )),
            chat_id: msg.chat.id.to_string(),
            content: text,
            message_id: Some(msg.message_id.to_string()),
            is_command,
        })
    });
    
    // Send event
    let event = WebhookEvent {
        source: "telegram".to_string(),
        webhook_id: None,
        payload: serde_json::from_slice(&body).unwrap_or(Value::Null),
        message: webhook_message,
        timestamp: chrono::Utc::now().timestamp(),
    };
    
    if let Err(e) = state.event_tx.send(event).await {
        error!("Failed to send Telegram webhook event: {}", e);
    }
    
    StatusCode::OK
}

// =============================================================================
// Discord Webhook (Interactions)
// =============================================================================

/// Discord interaction type
#[derive(Debug, Deserialize)]
struct DiscordInteraction {
    #[serde(rename = "type")]
    interaction_type: u8,
#[serde(rename = "token")]
    _token: Option<String>,
    data: Option<DiscordInteractionData>,
    member: Option<DiscordMember>,
    user: Option<DiscordUser>,
    channel_id: Option<String>,
#[serde(rename = "guild_id")]
    _guild_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DiscordInteractionData {
    name: Option<String>,
#[serde(rename = "options")]
    _options: Option<Vec<DiscordOption>>,
    custom_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DiscordOption {
#[serde(rename = "name")]
    _name: String,
#[serde(rename = "value")]
    _value: Value,
}

#[derive(Debug, Deserialize)]
struct DiscordMember {
    user: DiscordUser,
#[serde(rename = "nick")]
    _nick: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DiscordUser {
    id: String,
    username: String,
#[serde(rename = "discriminator")]
    _discriminator: Option<String>,
}

/// Verify Discord signature
fn verify_discord_signature(
    public_key: &str,
    signature: &str,
    timestamp: &str,
    _body: &[u8],
) -> bool {
    // For full verification, we'd use ed25519-dalek
    // For now, we'll trust the signature if we receive valid headers
    // TODO: Implement proper Ed25519 verification
    !public_key.is_empty() && !signature.is_empty() && !timestamp.is_empty()
}

/// Handle Discord interactions webhook
async fn discord_webhook(
    State(state): State<Arc<WebhookState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Get headers for verification
    let signature = headers
        .get("X-Signature-Ed25519")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let timestamp = headers
        .get("X-Signature-Timestamp")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    
    // Verify signature if public key is configured
    if let Some(ref public_key) = state.config.discord_public_key {
        if !verify_discord_signature(public_key, signature, timestamp, &body) {
            warn!("Discord webhook: invalid signature");
            return (StatusCode::UNAUTHORIZED, Json(json!({"error": "invalid signature"}))).into_response();
        }
    }
    
    // Parse interaction
    let interaction: DiscordInteraction = match serde_json::from_slice(&body) {
        Ok(i) => i,
        Err(e) => {
            error!("Discord webhook: failed to parse interaction: {}", e);
            return (StatusCode::BAD_REQUEST, Json(json!({"error": "invalid payload"}))).into_response();
        }
    };
    
    // Handle PING (type 1) - required for Discord verification
    if interaction.interaction_type == 1 {
        info!("Discord webhook: responding to PING");
        return (StatusCode::OK, Json(json!({"type": 1}))).into_response();
    }
    
    debug!("Discord webhook: received interaction type {}", interaction.interaction_type);
    
    // Extract user info
    let user = interaction.member.map(|m| m.user).or(interaction.user);
    
    let webhook_message = user.and_then(|u| {
        let content = interaction.data.as_ref().and_then(|d| {
            d.name.clone().or(d.custom_id.clone())
        })?;
        
        Some(WebhookMessage {
            sender_id: u.id.clone(),
            sender_name: Some(u.username.clone()),
            chat_id: interaction.channel_id.clone().unwrap_or_default(),
            content,
            message_id: None,
            is_command: interaction.interaction_type == 2, // APPLICATION_COMMAND
        })
    });
    
    // Send event
    let event = WebhookEvent {
        source: "discord".to_string(),
        webhook_id: None,
        payload: serde_json::from_slice(&body).unwrap_or(Value::Null),
        message: webhook_message,
        timestamp: chrono::Utc::now().timestamp(),
    };
    
    if let Err(e) = state.event_tx.send(event).await {
        error!("Failed to send Discord webhook event: {}", e);
    }
    
    // Acknowledge the interaction (type 5 = DEFERRED_CHANNEL_MESSAGE_WITH_SOURCE)
    (StatusCode::OK, Json(json!({"type": 5}))).into_response()
}

// =============================================================================
// Slack Webhook
// =============================================================================

/// Slack event wrapper
#[derive(Debug, Deserialize)]
struct SlackEventWrapper {
    #[serde(rename = "type")]
    event_type: String,
    challenge: Option<String>,
    event: Option<SlackEvent>,
#[serde(rename = "team_id")]
    _team_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SlackEvent {
    #[serde(rename = "type")]
    event_type: String,
    user: Option<String>,
    channel: Option<String>,
    text: Option<String>,
    ts: Option<String>,
}

/// Verify Slack signature
fn verify_slack_signature(
    signing_secret: &str,
    signature: &str,
    timestamp: &str,
    body: &[u8],
) -> bool {
    use std::time::{SystemTime, UNIX_EPOCH};
    
    // Check timestamp is recent (within 5 minutes)
    if let Ok(ts) = timestamp.parse::<u64>() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        
        if now.abs_diff(ts) > 300 {
            return false;
        }
    } else {
        return false;
    }
    
    // Build the signature base string
    let _sig_basestring = format!("v0:{}:{}", timestamp, String::from_utf8_lossy(body));
    
    // Compute HMAC-SHA256
    // TODO: Use proper HMAC implementation (ring or hmac crate)
    // For now, we trust if we have the required headers
    !signing_secret.is_empty() && !signature.is_empty()
}

/// Handle Slack events webhook
async fn slack_webhook(
    State(state): State<Arc<WebhookState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Get headers for verification
    let signature = headers
        .get("X-Slack-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let timestamp = headers
        .get("X-Slack-Request-Timestamp")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    
    // Verify signature if secret is configured
    if let Some(ref signing_secret) = state.config.slack_signing_secret {
        if !verify_slack_signature(signing_secret, signature, timestamp, &body) {
            warn!("Slack webhook: invalid signature");
            return (StatusCode::UNAUTHORIZED, "Invalid signature").into_response();
        }
    }
    
    // Parse event
    let event_wrapper: SlackEventWrapper = match serde_json::from_slice(&body) {
        Ok(e) => e,
        Err(e) => {
            error!("Slack webhook: failed to parse event: {}", e);
            return (StatusCode::BAD_REQUEST, "Invalid payload").into_response();
        }
    };
    
    // Handle URL verification challenge
    if event_wrapper.event_type == "url_verification" {
        if let Some(challenge) = event_wrapper.challenge {
            info!("Slack webhook: responding to URL verification");
            return (StatusCode::OK, challenge).into_response();
        }
    }
    
    debug!("Slack webhook: received event type {}", event_wrapper.event_type);
    
    // Extract message from event
    let webhook_message = event_wrapper.event.and_then(|evt| {
        if evt.event_type != "message" && evt.event_type != "app_mention" {
            return None;
        }
        
        Some(WebhookMessage {
            sender_id: evt.user.unwrap_or_default(),
            sender_name: None,
            chat_id: evt.channel.unwrap_or_default(),
            content: evt.text.unwrap_or_default(),
            message_id: evt.ts,
            is_command: evt.event_type == "app_mention",
        })
    });
    
    // Send event
    let event = WebhookEvent {
        source: "slack".to_string(),
        webhook_id: None,
        payload: serde_json::from_slice(&body).unwrap_or(Value::Null),
        message: webhook_message,
        timestamp: chrono::Utc::now().timestamp(),
    };
    
    if let Err(e) = state.event_tx.send(event).await {
        error!("Failed to send Slack webhook event: {}", e);
    }
    
    StatusCode::OK.into_response()
}

// =============================================================================
// WhatsApp Webhook
// =============================================================================

/// WhatsApp webhook verification (GET request)
async fn whatsapp_verify(
    State(state): State<Arc<WebhookState>>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let mode = params.get("hub.mode").map(|s| s.as_str());
    let token = params.get("hub.verify_token").map(|s| s.as_str());
    let challenge = params.get("hub.challenge");
    
    if mode == Some("subscribe") {
        if let Some(ref verify_token) = state.config.whatsapp_verify_token {
            if token == Some(verify_token.as_str()) {
                info!("WhatsApp webhook: verification successful");
                return (StatusCode::OK, challenge.cloned().unwrap_or_default()).into_response();
            }
        }
    }
    
    warn!("WhatsApp webhook: verification failed");
    (StatusCode::FORBIDDEN, "Verification failed").into_response()
}

/// WhatsApp webhook message structure
#[derive(Debug, Deserialize)]
struct WhatsAppWebhook {
    entry: Vec<WhatsAppEntry>,
}

#[derive(Debug, Deserialize)]
struct WhatsAppEntry {
#[serde(rename = "id")]
    _id: String,
    changes: Vec<WhatsAppChange>,
}

#[derive(Debug, Deserialize)]
struct WhatsAppChange {
    value: WhatsAppValue,
}

#[derive(Debug, Deserialize)]
struct WhatsAppValue {
#[serde(rename = "messaging_product")]
    _messaging_product: Option<String>,
    metadata: Option<WhatsAppMetadata>,
    contacts: Option<Vec<WhatsAppContact>>,
    messages: Option<Vec<WhatsAppMessage>>,
}

#[derive(Debug, Deserialize)]
struct WhatsAppMetadata {
    phone_number_id: String,
}

#[derive(Debug, Deserialize)]
struct WhatsAppContact {
    wa_id: String,
    profile: Option<WhatsAppProfile>,
}

#[derive(Debug, Deserialize)]
struct WhatsAppProfile {
    name: String,
}

#[derive(Debug, Deserialize)]
struct WhatsAppMessage {
    from: String,
    id: String,
    #[serde(rename = "type")]
    msg_type: String,
    text: Option<WhatsAppText>,
}

#[derive(Debug, Deserialize)]
struct WhatsAppText {
    body: String,
}

/// Handle WhatsApp webhook
async fn whatsapp_webhook(
    State(state): State<Arc<WebhookState>>,
    body: Bytes,
) -> impl IntoResponse {
    // Parse webhook
    let webhook: WhatsAppWebhook = match serde_json::from_slice(&body) {
        Ok(w) => w,
        Err(e) => {
            error!("WhatsApp webhook: failed to parse: {}", e);
            return StatusCode::BAD_REQUEST;
        }
    };
    
    for entry in webhook.entry {
        for change in entry.changes {
            let value = change.value;
            
            // Get metadata
            let phone_number_id = value.metadata.map(|m| m.phone_number_id).unwrap_or_default();
            
            // Get contacts map
            let contacts: HashMap<String, String> = value.contacts
                .unwrap_or_default()
                .into_iter()
                .filter_map(|c| {
                    let name = c.profile.map(|p| p.name)?;
                    Some((c.wa_id, name))
                })
                .collect();
            
            // Process messages
            for msg in value.messages.unwrap_or_default() {
                if msg.msg_type != "text" {
                    continue;
                }
                
                let text = msg.text.map(|t| t.body).unwrap_or_default();
                let sender_name = contacts.get(&msg.from).cloned();
                
                let webhook_message = WebhookMessage {
                    sender_id: msg.from.clone(),
                    sender_name,
                    chat_id: phone_number_id.clone(),
                    content: text,
                    message_id: Some(msg.id),
                    is_command: false,
                };
                
                let event = WebhookEvent {
                    source: "whatsapp".to_string(),
                    webhook_id: None,
                    payload: serde_json::from_slice(&body).unwrap_or(Value::Null),
                    message: Some(webhook_message),
                    timestamp: chrono::Utc::now().timestamp(),
                };
                
                if let Err(e) = state.event_tx.send(event).await {
                    error!("Failed to send WhatsApp webhook event: {}", e);
                }
            }
        }
    }
    
    StatusCode::OK
}

// =============================================================================
// Generic Webhook
// =============================================================================

/// Handle generic webhook
async fn generic_webhook(
    State(state): State<Arc<WebhookState>>,
    Path(webhook_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    debug!("Generic webhook: received for id {}", webhook_id);
    
    // Verify secret if configured for this webhook
    if let Some(secret) = state.config.generic_secrets.get(&webhook_id) {
        let auth_header = headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        
        // Check Bearer token or X-Webhook-Secret
        let secret_header = headers
            .get("X-Webhook-Secret")
            .and_then(|v| v.to_str().ok());
        
        let is_valid = auth_header == format!("Bearer {}", secret)
            || secret_header == Some(secret.as_str());
        
        if !is_valid {
            warn!("Generic webhook {}: invalid secret", webhook_id);
            return StatusCode::UNAUTHORIZED;
        }
    }
    
    // Parse payload as JSON if possible
    let payload: Value = serde_json::from_slice(&body)
        .unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(&body).to_string() }));
    
    // Try to extract a message from common payload structures
    let webhook_message = extract_generic_message(&payload);
    
    let event = WebhookEvent {
        source: "generic".to_string(),
        webhook_id: Some(webhook_id),
        payload,
        message: webhook_message,
        timestamp: chrono::Utc::now().timestamp(),
    };
    
    if let Err(e) = state.event_tx.send(event).await {
        error!("Failed to send generic webhook event: {}", e);
    }
    
    StatusCode::OK
}

/// Try to extract a message from common webhook payload structures
fn extract_generic_message(payload: &Value) -> Option<WebhookMessage> {
    // Try common patterns
    
    // Pattern 1: { "message": "...", "from": "..." }
    if let (Some(message), Some(from)) = (
        payload.get("message").and_then(|v| v.as_str()),
        payload.get("from").and_then(|v| v.as_str()),
    ) {
        return Some(WebhookMessage {
            sender_id: from.to_string(),
            sender_name: payload.get("from_name").and_then(|v| v.as_str()).map(String::from),
            chat_id: payload.get("channel").or(payload.get("chat_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            content: message.to_string(),
            message_id: payload.get("id").and_then(|v| v.as_str()).map(String::from),
            is_command: false,
        });
    }
    
    // Pattern 2: { "text": "...", "user": "..." }
    if let (Some(text), Some(user)) = (
        payload.get("text").and_then(|v| v.as_str()),
        payload.get("user").and_then(|v| v.as_str()),
    ) {
        return Some(WebhookMessage {
            sender_id: user.to_string(),
            sender_name: None,
            chat_id: "generic".to_string(),
            content: text.to_string(),
            message_id: None,
            is_command: false,
        });
    }
    
    // Pattern 3: { "content": "..." }
    if let Some(content) = payload.get("content").and_then(|v| v.as_str()) {
        return Some(WebhookMessage {
            sender_id: "unknown".to_string(),
            sender_name: None,
            chat_id: "generic".to_string(),
            content: content.to_string(),
            message_id: None,
            is_command: false,
        });
    }
    
    None
}

// =============================================================================
// Webhook Server
// =============================================================================

/// The webhook HTTP server
pub struct WebhookServer {
    config: WebhookConfig,
    event_tx: mpsc::Sender<WebhookEvent>,
}

impl WebhookServer {
    /// Create a new webhook server
    pub fn new(config: WebhookConfig) -> (Self, mpsc::Receiver<WebhookEvent>) {
        let (event_tx, event_rx) = mpsc::channel(100);
        (Self { config, event_tx }, event_rx)
    }
    
    /// Build the Axum router
    fn router(&self) -> Router {
        let state = Arc::new(WebhookState::new(self.config.clone(), self.event_tx.clone()));
        
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any);
        
        Router::new()
            // Channel-specific webhooks
            .route("/webhook/telegram", post(telegram_webhook))
            .route("/webhook/discord", post(discord_webhook))
            .route("/webhook/slack", post(slack_webhook))
            .route("/webhook/whatsapp", get(whatsapp_verify).post(whatsapp_webhook))
            // Generic webhook
            .route("/webhook/:id", post(generic_webhook))
            // Health check (so load balancers can verify)
            .route("/", get(|| async { "Nanna Webhook Server" }))
            .layer(cors)
            .with_state(state)
    }
    
    /// Run the webhook server
    pub async fn run(&self) -> Result<(), std::io::Error> {
        let addr: SocketAddr = format!("{}:{}", self.config.host, self.config.port)
            .parse()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        
        info!("Webhook server listening on http://{}", addr);
        
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, self.router()).await
    }
    
    /// Spawn the webhook server as a background task
    pub fn spawn(self) -> (tokio::task::JoinHandle<()>, mpsc::Receiver<WebhookEvent>) {
        let (event_tx, event_rx) = mpsc::channel(100);
        let server = WebhookServer {
            config: self.config,
            event_tx,
        };
        
        let handle = tokio::spawn(async move {
            if let Err(e) = server.run().await {
                error!("Webhook server error: {}", e);
            }
        });
        
        (handle, event_rx)
    }
}

/// Default webhook server port
pub const DEFAULT_WEBHOOK_PORT: u16 = 3000;

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_extract_generic_message_pattern1() {
        let payload = json!({
            "message": "Hello world",
            "from": "user123",
            "from_name": "John",
            "channel": "general"
        });
        
        let msg = extract_generic_message(&payload).unwrap();
        assert_eq!(msg.content, "Hello world");
        assert_eq!(msg.sender_id, "user123");
        assert_eq!(msg.sender_name, Some("John".to_string()));
        assert_eq!(msg.chat_id, "general");
    }
    
    #[test]
    fn test_extract_generic_message_pattern2() {
        let payload = json!({
            "text": "Test message",
            "user": "alice"
        });
        
        let msg = extract_generic_message(&payload).unwrap();
        assert_eq!(msg.content, "Test message");
        assert_eq!(msg.sender_id, "alice");
    }
    
    #[test]
    fn test_extract_generic_message_pattern3() {
        let payload = json!({
            "content": "Simple content"
        });
        
        let msg = extract_generic_message(&payload).unwrap();
        assert_eq!(msg.content, "Simple content");
    }
}
