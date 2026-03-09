//! Generic webhook handler for custom integrations
#![allow(dead_code)]

use crate::state::AppState;
use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

/// Generic webhook request
#[derive(Debug, Deserialize)]
pub struct GenericWebhook {
    /// Channel/source identifier
    pub channel: String,
    /// User identifier
    pub user_id: String,
    /// Optional user display name
    pub user_name: Option<String>,
    /// Message content
    pub message: String,
    /// Optional session ID (will be generated if not provided)
    pub session_id: Option<String>,
    /// Optional metadata
    pub metadata: Option<serde_json::Value>,
}

/// Generic webhook response
#[derive(Debug, Serialize)]
pub struct GenericWebhookResponse {
    pub success: bool,
    pub session_id: String,
    pub response: Option<String>,
    pub error: Option<String>,
}

/// Handle generic webhook
pub async fn handle(
    State(state): State<AppState>,
    Json(webhook): Json<GenericWebhook>,
) -> Result<Json<GenericWebhookResponse>, StatusCode> {
    let session_id = webhook
        .session_id
        .unwrap_or_else(|| format!("{}:{}:{}", webhook.channel, webhook.user_id, Uuid::new_v4()));

    info!(
        "Generic webhook from {}:{} - {}",
        webhook.channel,
        webhook.user_id,
        webhook.message.chars().take(50).collect::<String>()
    );

    match state.bot.process_message(&session_id, &webhook.message).await {
        Ok(response) => Ok(Json(GenericWebhookResponse {
            success: true,
            session_id,
            response: Some(response),
            error: None,
        })),
        Err(e) => Ok(Json(GenericWebhookResponse {
            success: false,
            session_id,
            response: None,
            error: Some(e.to_string()),
        })),
    }
}
