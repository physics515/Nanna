//! Telegram webhook handler

use crate::state::AppState;
use axum::{extract::State, http::StatusCode, Json};
use nanna_agent::RunOptions;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// Telegram Update object (simplified)
#[derive(Debug, Deserialize)]
pub struct TelegramUpdate {
    pub update_id: i64,
    pub message: Option<TelegramMessage>,
    pub edited_message: Option<TelegramMessage>,
    pub callback_query: Option<CallbackQuery>,
}

#[derive(Debug, Deserialize)]
pub struct TelegramMessage {
    pub message_id: i64,
    pub from: Option<TelegramUser>,
    pub chat: TelegramChat,
    pub date: i64,
    pub text: Option<String>,
    pub reply_to_message: Option<Box<TelegramMessage>>,
}

#[derive(Debug, Deserialize)]
pub struct TelegramUser {
    pub id: i64,
    pub first_name: String,
    pub last_name: Option<String>,
    pub username: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TelegramChat {
    pub id: i64,
    #[serde(rename = "type")]
    pub chat_type: String,
    pub title: Option<String>,
    pub username: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub id: String,
    pub from: TelegramUser,
    pub message: Option<TelegramMessage>,
    pub data: Option<String>,
}

/// Telegram API response wrapper
#[derive(Serialize)]
pub struct TelegramResponse {
    method: &'static str,
    chat_id: i64,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to_message_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parse_mode: Option<&'static str>,
}

/// Handle incoming Telegram webhook
pub async fn handle(
    State(state): State<AppState>,
    Json(update): Json<TelegramUpdate>,
) -> Result<Json<TelegramResponse>, StatusCode> {
    debug!("Received Telegram update: {:?}", update.update_id);

    // Extract message
    let message = update
        .message
        .or(update.edited_message)
        .ok_or(StatusCode::OK)?; // No message, just acknowledge

    let text = message.text.as_deref().unwrap_or("");
    if text.is_empty() {
        return Err(StatusCode::OK); // No text, acknowledge
    }

    let chat_id = message.chat.id;
    let user_id = message.from.as_ref().map(|u| u.id).unwrap_or(0);
    let username = message.from.as_ref().and_then(|u| u.username.clone());
    let session_id = format!("telegram:{}:{}", chat_id, user_id);

    info!(
        "Telegram message from {} (@{}) in {}: {}",
        user_id,
        username.as_deref().unwrap_or("unknown"),
        chat_id,
        text.chars().take(50).collect::<String>()
    );

    // Get or create agent for this session
    let system_prompt = format!(
        "You are Nanna — moon god of the digital realm.\n\
         You're chatting on Telegram with user {} (@{}).\n\
         Be helpful, concise, and conversational.",
        user_id,
        username.as_deref().unwrap_or("unknown")
    );
    let agent = state.get_or_create_agent(&session_id, Some(&system_prompt)).await;

    // Process message with full agent capabilities
    let response = {
        let agent_guard = agent.read().await;
        agent_guard.run(text, RunOptions::default()).await
    };

    let response_text = match response {
        Ok(r) => r.text,
        Err(e) => {
            warn!("Error processing Telegram message: {}", e);
            "Sorry, I encountered an error processing your message.".to_string()
        }
    };

    // Reply using webhook response method
    Ok(Json(TelegramResponse {
        method: "sendMessage",
        chat_id,
        text: response_text,
        reply_to_message_id: Some(message.message_id),
        parse_mode: Some("Markdown"),
    }))
}
