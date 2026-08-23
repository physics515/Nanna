//! Generic webhook handler for custom integrations

use crate::state::AppState;
use crate::webhooks::auth;
use axum::{Json, body::Bytes, extract::State, http::HeaderMap, http::StatusCode};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

/// Generic webhook request
#[derive(Debug, Deserialize)]
pub struct GenericWebhook {
    /// Channel/source identifier
    pub channel: String,
    /// User identifier
    pub user_id: String,
    /// Optional user display name
#[serde(rename = "user_name")]
    pub _user_name: Option<String>,
    /// Message content
    pub message: String,
    /// Optional session ID (will be generated if not provided)
    pub session_id: Option<String>,
    /// Optional metadata
#[serde(rename = "metadata")]
    pub _metadata: Option<serde_json::Value>,
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
///
/// This endpoint takes an arbitrary `message` and runs it, so it is the most
/// directly abusable of the five. `server.webhook_secret` was already parsed
/// into `AppState` and then read by nobody; it is the credential now.
pub async fn handle(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<GenericWebhookResponse>, StatusCode> {
    let Some(secret) = auth::configured(state.webhook_secret.as_ref()) else {
        return Err(auth::refuse_unconfigured(
            "Generic",
            "server.webhook_secret",
        ));
    };
    if !auth::bearer_secret_ok(&headers, secret) {
        warn!("Generic webhook: invalid shared secret");
        return Err(StatusCode::UNAUTHORIZED);
    }

    // Parse only now. `Bytes` rather than `Json<T>` so the credential check
    // above runs first — see `auth::parse_authenticated_body`. This route is
    // the most abusable of the five (it runs whatever `message` it is handed),
    // so it is the one that least deserves an anonymous deserializer.
    let webhook: GenericWebhook = auth::parse_authenticated_body("Generic", &body)?;

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
