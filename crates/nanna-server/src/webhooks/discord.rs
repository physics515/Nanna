//! Discord webhook handler
#![allow(dead_code)]

use crate::state::AppState;
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

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

/// Handle Discord interaction webhook
pub async fn handle(
    State(state): State<AppState>,
    _headers: HeaderMap,
    body: Bytes,
) -> Result<Json<InteractionResponse>, StatusCode> {
    // TODO: Verify Discord signature using Ed25519
    // let signature = headers.get("X-Signature-Ed25519");
    // let timestamp = headers.get("X-Signature-Timestamp");

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
        use nanna_agent::RunOptions;

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

        // Get or create agent for this session
        let system_prompt = format!(
            "You are Nanna — moon god of the digital realm.\n\
             You're chatting on Discord with user {username} (ID: {user_id}).\n\
             Be helpful, concise, and conversational. Use Discord markdown."
        );
        let agent = state.get_or_create_agent(&session_id, Some(&system_prompt)).await;

        let response = {
            let agent_guard = agent.read().await;
            agent_guard.run(input, RunOptions::default()).await
        };

        let response_text = match response {
            Ok(r) => r.text,
            Err(e) => {
                tracing::warn!("Error processing Discord command: {}", e);
                "Sorry, I encountered an error.".to_string()
            }
        };

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
