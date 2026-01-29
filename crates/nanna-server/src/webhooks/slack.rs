//! Slack webhook handler
#![allow(dead_code)]

use crate::state::AppState;
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Form, Json,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// Slack event wrapper
#[derive(Debug, Deserialize)]
pub struct SlackEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub challenge: Option<String>,
    pub token: Option<String>,
    pub team_id: Option<String>,
    pub event: Option<SlackEventInner>,
}

#[derive(Debug, Deserialize)]
pub struct SlackEventInner {
    #[serde(rename = "type")]
    pub event_type: String,
    pub user: Option<String>,
    pub channel: Option<String>,
    pub text: Option<String>,
    #[allow(dead_code)] // Used for threading support (future)
    pub ts: Option<String>,
    #[allow(dead_code)] // Parent thread timestamp for replies (future)
    pub thread_ts: Option<String>,
    pub bot_id: Option<String>,
}

/// Slack slash command
#[derive(Debug, Deserialize)]
pub struct SlackSlashCommand {
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
    Json(event): Json<SlackEvent>,
) -> Result<impl IntoResponse, StatusCode> {
    debug!("Slack event type: {}", event.event_type);

    // Handle URL verification challenge
    if event.event_type == "url_verification" {
        if let Some(challenge) = event.challenge {
            return Ok(Json(SlackResponse {
                text: None,
                response_type: None,
                challenge: Some(challenge),
            }));
        }
    }

    // Handle event callbacks
    if event.event_type == "event_callback" {
        if let Some(inner) = event.event {
            // Ignore bot messages
            if inner.bot_id.is_some() {
                return Ok(Json(SlackResponse {
                    text: None,
                    response_type: None,
                    challenge: None,
                }));
            }

            // Handle app_mention and message events
            if inner.event_type == "app_mention" || inner.event_type == "message" {
                use nanna_agent::RunOptions;

                let user_id = inner.user.as_deref().unwrap_or("unknown");
                let channel_id = inner.channel.as_deref().unwrap_or("unknown");
                let text = inner.text.as_deref().unwrap_or("");

                if text.is_empty() {
                    return Ok(Json(SlackResponse {
                        text: None,
                        response_type: None,
                        challenge: None,
                    }));
                }

                let session_id = format!("slack:{}:{}", channel_id, user_id);
                info!("Slack message from {}: {}", user_id, text.chars().take(50).collect::<String>());

                // Get or create agent
                let system_prompt = format!(
                    "You are Nanna — moon god of the digital realm.\n\
                     You're chatting on Slack with user {}.\n\
                     Be helpful and use Slack markdown (mrkdwn).",
                    user_id
                );
                let agent = state.get_or_create_agent(&session_id, Some(&system_prompt)).await;

                let response = {
                    let agent_guard = agent.read().await;
                    agent_guard.run(text, RunOptions::default()).await
                };

                let response_text = match response {
                    Ok(r) => r.text,
                    Err(e) => {
                        tracing::warn!("Error processing Slack message: {}", e);
                        "Sorry, I encountered an error.".to_string()
                    }
                };

                return Ok(Json(SlackResponse {
                    text: Some(response_text),
                    response_type: Some("in_channel".to_string()),
                    challenge: None,
                }));
            }
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
pub async fn handle_slash_command(
    State(state): State<AppState>,
    Form(command): Form<SlackSlashCommand>,
) -> Result<Json<SlackResponse>, StatusCode> {
    use nanna_agent::RunOptions;

    info!("Slack slash command from {}: {} {}", command.user_name, command.command, command.text);

    let session_id = format!("slack:{}:{}", command.channel_id, command.user_id);

    // Get or create agent
    let system_prompt = format!(
        "You are Nanna — moon god of the digital realm.\n\
         You're chatting on Slack with {} in #{}.\n\
         Be helpful and use Slack markdown (mrkdwn).",
        command.user_name, command.channel_name
    );
    let agent = state.get_or_create_agent(&session_id, Some(&system_prompt)).await;

    let response = {
        let agent_guard = agent.read().await;
        agent_guard.run(&command.text, RunOptions::default()).await
    };

    let response_text = match response {
        Ok(r) => r.text,
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
