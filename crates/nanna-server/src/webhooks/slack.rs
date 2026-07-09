//! Slack webhook handler

use crate::state::AppState;
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

const PROVIDER: &str = "slack";

/// Maximum age of a request timestamp before we reject it (5 minutes)
const MAX_TIMESTAMP_AGE_SECS: i64 = 300;

type HmacSha256 = Hmac<Sha256>;

/// Verify Slack request signature using HMAC-SHA256.
///
/// Slack signs requests with: v0=HMAC-SHA256(signing_secret, "v0:{timestamp}:{body}")
fn verify_slack_signature(
    signing_secret: &str,
    signature: &str,
    timestamp: &str,
    body: &[u8],
) -> bool {
    // Check timestamp freshness to prevent replay attacks
    if let Ok(ts) = timestamp.parse::<i64>() {
        let now = chrono::Utc::now().timestamp();
        if (now - ts).abs() > MAX_TIMESTAMP_AGE_SECS {
            warn!("Slack request timestamp too old: {}s", (now - ts).abs());
            return false;
        }
    } else {
        warn!("Invalid Slack timestamp: {}", timestamp);
        return false;
    }

    // Build the base string: v0:{timestamp}:{body}
    let body_str = std::str::from_utf8(body).unwrap_or("");
    let base_string = format!("v0:{timestamp}:{body_str}");

    // Compute HMAC-SHA256
    let mut mac = match HmacSha256::new_from_slice(signing_secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => {
            warn!("Invalid Slack signing secret");
            return false;
        }
    };
    mac.update(base_string.as_bytes());
    let result = mac.finalize();
    let computed = format!("v0={}", hex::encode(result.into_bytes()));

    // Constant-time comparison
    if computed.len() != signature.len() {
        return false;
    }
    computed
        .as_bytes()
        .iter()
        .zip(signature.as_bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// Slack event wrapper
#[derive(Debug, Deserialize)]
pub struct SlackEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub challenge: Option<String>,
    #[serde(rename = "token")]
    pub _token: Option<String>,
    #[serde(rename = "team_id")]
    pub _team_id: Option<String>,
    pub event: Option<SlackEventInner>,
}

#[derive(Debug, Deserialize)]
pub struct SlackEventInner {
    #[serde(rename = "type")]
    pub event_type: String,
    pub user: Option<String>,
    pub channel: Option<String>,
    pub text: Option<String>,
    /// Message timestamp (used as message ID)
    pub ts: Option<String>,
    /// Parent thread timestamp for replies
    #[serde(rename = "thread_ts")]
    pub _thread_ts: Option<String>,
    pub bot_id: Option<String>,
    /// For reaction events
    pub reaction: Option<String>,
    /// Item that was reacted to (for reaction_added/removed)
    pub item: Option<SlackReactionItem>,
}

/// Slack item that received a reaction
#[derive(Debug, Deserialize)]
pub struct SlackReactionItem {
    #[serde(rename = "type")]
    pub item_type: String,
    pub channel: Option<String>,
    pub ts: Option<String>,
}

/// Slack slash command
#[derive(Debug, Deserialize)]
pub struct _SlackSlashCommand {
    pub token: String,
    pub team_id: String,
    pub team_domain: String,
    pub channel_id: String,
    pub channel_name: String,
    pub user_id: String,
    pub user_name: String,
    pub command: String,
    pub text: String,
    pub response_url: String,
    pub trigger_id: String,
}

/// Slack response
#[derive(Debug, Serialize)]
pub struct SlackResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub challenge: Option<String>,
}

/// Handle Slack event API webhook
pub async fn handle(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, StatusCode> {
    // Verify Slack signature if signing secret is configured
    if let Some(ref signing_secret) = state.slack_signing_secret {
        let signature = headers
            .get("X-Slack-Signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                warn!("Missing X-Slack-Signature header");
                StatusCode::UNAUTHORIZED
            })?;

        let timestamp = headers
            .get("X-Slack-Request-Timestamp")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                warn!("Missing X-Slack-Request-Timestamp header");
                StatusCode::UNAUTHORIZED
            })?;

        if !verify_slack_signature(signing_secret, signature, timestamp, &body) {
            warn!("Slack signature verification failed");
            return Err(StatusCode::UNAUTHORIZED);
        }

        debug!("Slack signature verified successfully");
    } else {
        warn!("Slack signing secret not configured - skipping signature verification");
    }

    // Parse the body
    let event: SlackEvent = serde_json::from_slice(&body).map_err(|e| {
        warn!("Failed to parse Slack event: {}", e);
        StatusCode::BAD_REQUEST
    })?;

    debug!("Slack event type: {}", event.event_type);

    // Handle URL verification challenge
    if event.event_type == "url_verification"
        && let Some(challenge) = event.challenge {
            return Ok(Json(SlackResponse {
                text: None,
                response_type: None,
                challenge: Some(challenge),
            }));
        }

    // Handle event callbacks
    if event.event_type == "event_callback"
        && let Some(inner) = event.event {
            // Ignore bot messages
            if inner.bot_id.is_some() {
                return Ok(Json(SlackResponse {
                    text: None,
                    response_type: None,
                    challenge: None,
                }));
            }

            // Handle reaction events (for memory feedback)
            if inner.event_type == "reaction_added" || inner.event_type == "reaction_removed" {
                if let Some(item) = &inner.item {
                    if item.item_type == "message" {
                        if let (Some(channel), Some(ts), Some(reaction)) = 
                            (&item.channel, &item.ts, &inner.reaction) 
                        {
                            let message_key = format!("{}:{}:{}", PROVIDER, channel, ts);
                            let positive = is_positive_reaction(reaction);
                            
                            // Only process additions of feedback reactions
                            if inner.event_type == "reaction_added" {
                                info!("Slack reaction {} on {} (positive: {})", reaction, message_key, positive);
                                state.record_message_feedback(&message_key, positive).await;
                            }
                        }
                    }
                }
                return Ok(Json(SlackResponse {
                    text: None,
                    response_type: None,
                    challenge: None,
                }));
            }

            // Handle app_mention and message events
            if inner.event_type == "app_mention" || inner.event_type == "message" {
                let user_id = inner.user.as_deref().unwrap_or("unknown");
                let channel_id = inner.channel.as_deref().unwrap_or("unknown");
                let text = inner.text.as_deref().unwrap_or("");
                let message_ts = inner.ts.as_deref().unwrap_or("");

                if text.is_empty() {
                    return Ok(Json(SlackResponse {
                        text: None,
                        response_type: None,
                        challenge: None,
                    }));
                }

                let session_id = format!("slack:{channel_id}:{user_id}");
                info!("Slack message from {}: {}", user_id, text.chars().take(50).collect::<String>());

                // Build system prompt
                let system_prompt = format!(
                    "You are Nanna — moon god of the digital realm.\n\
                     You're chatting on Slack with user {user_id}.\n\
                     Be helpful and use Slack markdown (mrkdwn)."
                );

                // Process message (with memory extraction if enabled)
                let response_text = match state.process_message(&session_id, text, Some(&system_prompt)).await {
                    Ok(text) => text,
                    Err(e) => {
                        tracing::warn!("Error processing Slack message: {}", e);
                        "Sorry, I encountered an error.".to_string()
                    }
                };

                // Link message to session for reaction-based feedback
                if !message_ts.is_empty() {
                    let message_key = format!("{}:{}:{}", PROVIDER, channel_id, message_ts);
                    state.link_message_to_session(&message_key, &session_id).await;
                }

                return Ok(Json(SlackResponse {
                    text: Some(response_text),
                    response_type: Some("in_channel".to_string()),
                    challenge: None,
                }));
            }
        }

    // Default acknowledgment
    Ok(Json(SlackResponse {
        text: None,
        response_type: None,
        challenge: None,
    }))
}

/// Handle Slack slash commands
pub async fn _handle_slash_command(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<SlackResponse>, StatusCode> {
    // Verify signature
    if let Some(ref signing_secret) = state.slack_signing_secret {
        let signature = headers
            .get("X-Slack-Signature")
            .and_then(|v| v.to_str().ok())
            .ok_or(StatusCode::UNAUTHORIZED)?;
        let timestamp = headers
            .get("X-Slack-Request-Timestamp")
            .and_then(|v| v.to_str().ok())
            .ok_or(StatusCode::UNAUTHORIZED)?;

        if !verify_slack_signature(signing_secret, signature, timestamp, &body) {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }

    // Parse form data
    let command: _SlackSlashCommand = serde_urlencoded::from_bytes(&body)
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    info!("Slack slash command from {}: {} {}", command.user_name, command.command, command.text);

    let session_id = format!("slack:{}:{}", command.channel_id, command.user_id);

    // Build system prompt
    let system_prompt = format!(
        "You are Nanna — moon god of the digital realm.\n\
         You're chatting on Slack with {} in #{}.\n\
         Be helpful and use Slack markdown (mrkdwn).",
        command.user_name, command.channel_name
    );

    // Process message (with memory extraction if enabled)
    let response_text = match state.process_message(&session_id, &command.text, Some(&system_prompt)).await {
        Ok(text) => text,
        Err(e) => {
            tracing::warn!("Error processing Slack command: {}", e);
            "Sorry, I encountered an error.".to_string()
        }
    };

    Ok(Json(SlackResponse {
        text: Some(response_text),
        response_type: Some("in_channel".to_string()),
        challenge: None,
    }))
}

/// Check if a Slack reaction is positive feedback
fn is_positive_reaction(reaction: &str) -> bool {
    let positive_reactions = [
        "+1", "thumbsup", "thumbs_up", "white_check_mark", "heavy_check_mark",
        "star", "star2", "heart", "hearts", "fire", "tada", "clap", "raised_hands",
        "100", "ok_hand", "muscle", "trophy", "medal", "1st_place_medal",
        "sunglasses", "rocket", "sparkles", "boom", "zap",
    ];
    
    let negative_reactions = [
        "-1", "thumbsdown", "thumbs_down", "x", "no_entry", "no_entry_sign",
        "warning", "rage", "angry", "disappointed", "confused", "pensive",
        "worried", "cry", "sob",
    ];
    
    if positive_reactions.iter().any(|r| reaction.contains(r)) {
        return true;
    }
    if negative_reactions.iter().any(|r| reaction.contains(r)) {
        return false;
    }
    
    true
}
