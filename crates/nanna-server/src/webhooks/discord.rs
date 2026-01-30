//! Discord webhook handler
#![allow(dead_code)]

use crate::state::AppState;
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

const PROVIDER: &str = "discord";

/// Discord interaction types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct InteractionType(pub u8);

impl InteractionType {
    pub const PING: Self = Self(1);
    pub const APPLICATION_COMMAND: Self = Self(2);
    pub const MESSAGE_COMPONENT: Self = Self(3);
    pub const AUTOCOMPLETE: Self = Self(4);
    pub const MODAL_SUBMIT: Self = Self(5);
}

/// Discord interaction response types
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(transparent)]
pub struct InteractionResponseType(pub u8);

impl InteractionResponseType {
    pub const PONG: Self = Self(1);
    pub const CHANNEL_MESSAGE: Self = Self(4);
    pub const DEFERRED_CHANNEL_MESSAGE: Self = Self(5);
    pub const DEFERRED_UPDATE_MESSAGE: Self = Self(6);
    pub const UPDATE_MESSAGE: Self = Self(7);
}

/// Discord interaction payload.
#[derive(Debug, Deserialize)]
pub struct Interaction {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: InteractionType,
    pub token: String,
    pub guild_id: Option<String>,
    pub channel_id: Option<String>,
    pub member: Option<Member>,
    pub user: Option<User>,
    pub data: Option<InteractionData>,
    pub message: Option<Message>,
}

#[derive(Debug, Deserialize)]
pub struct InteractionData {
    pub id: Option<String>,
    pub name: Option<String>,
    pub options: Option<Vec<CommandOption>>,
    pub custom_id: Option<String>,
    pub values: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct CommandOption {
    pub name: String,
    pub value: Option<serde_json::Value>,
    pub options: Option<Vec<Self>>,
}

#[derive(Debug, Deserialize)]
pub struct Member {
    pub user: Option<User>,
    pub nick: Option<String>,
    pub roles: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub discriminator: Option<String>,
    pub global_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    pub id: String,
    pub content: String,
    pub author: User,
    pub channel_id: String,
}

/// Discord interaction response
#[derive(Debug, Serialize)]
pub struct InteractionResponse {
    #[serde(rename = "type")]
    pub response_type: InteractionResponseType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<InteractionResponseData>,
}

#[derive(Debug, Serialize)]
pub struct InteractionResponseData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flags: Option<u32>,
}

/// Verify Discord request signature using Ed25519.
///
/// Discord signs requests with: signature = Ed25519(timestamp + body)
fn verify_discord_signature(
    public_key: &str,
    signature_hex: &str,
    timestamp: &str,
    body: &[u8],
) -> bool {
    // Decode the public key from hex
    let Ok(key_bytes) = hex::decode(public_key) else {
        warn!("Failed to decode Discord public key from hex");
        return false;
    };

    let Ok(key_bytes): Result<[u8; 32], _> = key_bytes.try_into() else {
        warn!("Discord public key has invalid length");
        return false;
    };

    let Ok(verifying_key) = VerifyingKey::from_bytes(&key_bytes) else {
        warn!("Invalid Discord public key");
        return false;
    };

    // Decode the signature from hex
    let Ok(sig_bytes) = hex::decode(signature_hex) else {
        warn!("Failed to decode Discord signature from hex");
        return false;
    };

    let Ok(sig_bytes): Result<[u8; 64], _> = sig_bytes.try_into() else {
        warn!("Discord signature has invalid length");
        return false;
    };

    let signature = Signature::from_bytes(&sig_bytes);

    // Build the message: timestamp + body
    let mut message = timestamp.as_bytes().to_vec();
    message.extend_from_slice(body);

    // Verify
    verifying_key.verify(&message, &signature).is_ok()
}

/// Handle Discord interaction webhook
pub async fn handle(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<InteractionResponse>, StatusCode> {
    // Verify Discord signature if public key is configured
    if let Some(ref public_key) = state.discord_public_key {
        let signature = headers
            .get("X-Signature-Ed25519")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                warn!("Missing X-Signature-Ed25519 header");
                StatusCode::UNAUTHORIZED
            })?;

        let timestamp = headers
            .get("X-Signature-Timestamp")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                warn!("Missing X-Signature-Timestamp header");
                StatusCode::UNAUTHORIZED
            })?;

        if !verify_discord_signature(public_key, signature, timestamp, &body) {
            warn!("Discord signature verification failed");
            return Err(StatusCode::UNAUTHORIZED);
        }

        debug!("Discord signature verified successfully");
    } else {
        warn!("Discord public key not configured - skipping signature verification");
    }

    let interaction: Interaction =
        serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;

    debug!("Discord interaction type: {:?}", interaction.kind);

    // Handle ping (required for Discord verification)
    if interaction.kind == InteractionType::PING {
        return Ok(Json(InteractionResponse {
            response_type: InteractionResponseType::PONG,
            data: None,
        }));
    }

    // Handle slash commands
    if interaction.kind == InteractionType::APPLICATION_COMMAND {
        let user = interaction
            .member
            .as_ref()
            .and_then(|m| m.user.as_ref())
            .or(interaction.user.as_ref());

        let user_id = user.map_or("unknown", |u| u.id.as_str());
        let username = user.map_or("unknown", |u| u.username.as_str());
        let channel_id = interaction.channel_id.as_deref().unwrap_or("unknown");
        let session_id = format!("discord:{channel_id}:{user_id}");

        // Get command input
        let input = interaction
            .data
            .as_ref()
            .and_then(|d| d.options.as_ref())
            .and_then(|opts| opts.first())
            .and_then(|opt| opt.value.as_ref())
            .and_then(|v| v.as_str())
            .unwrap_or("hello");

        info!("Discord command from {} ({}): {}", username, user_id, input);

        // Build system prompt for this user
        let system_prompt = format!(
            "You are Nanna — moon god of the digital realm.\n\
             You're chatting on Discord with user {username} (ID: {user_id}).\n\
             Be helpful, concise, and conversational. Use Discord markdown."
        );

        // Process message (with memory extraction if enabled)
        let response_text = match state.process_message(&session_id, input, Some(&system_prompt)).await {
            Ok(text) => text,
            Err(e) => {
                tracing::warn!("Error processing Discord command: {}", e);
                "Sorry, I encountered an error.".to_string()
            }
        };

        // Link interaction to session for potential reaction feedback
        // Note: Discord reactions typically come via Gateway, not webhooks
        let message_key = format!("{}:{}:{}", PROVIDER, channel_id, interaction.id);
        state.link_message_to_session(&message_key, &session_id).await;

        return Ok(Json(InteractionResponse {
            response_type: InteractionResponseType::CHANNEL_MESSAGE,
            data: Some(InteractionResponseData {
                content: Some(response_text),
                flags: None,
            }),
        }));
    }

    // Default: acknowledge
    Ok(Json(InteractionResponse {
        response_type: InteractionResponseType::CHANNEL_MESSAGE,
        data: Some(InteractionResponseData {
            content: Some("I don't know how to handle that yet.".to_string()),
            flags: Some(64), // Ephemeral
        }),
    }))
}
