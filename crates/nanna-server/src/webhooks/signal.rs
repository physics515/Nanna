//! Signal webhook handler (signal-cli-rest-api compatible)
//!
//! Works with: https://github.com/bbernhard/signal-cli-rest-api

use crate::state::AppState;
use axum::{extract::State, http::StatusCode, Json};
use nanna_agent::RunOptions;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// Signal envelope from signal-cli-rest-api webhook
#[derive(Debug, Deserialize)]
pub struct SignalWebhook {
    pub envelope: SignalEnvelope,
    pub account: String,
}

#[derive(Debug, Deserialize)]
pub struct SignalEnvelope {
    pub source: Option<String>,
    #[serde(rename = "sourceNumber")]
    pub source_number: Option<String>,
    #[serde(rename = "sourceUuid")]
    pub _source_uuid: Option<String>,
    #[serde(rename = "sourceName")]
    pub source_name: Option<String>,
    #[serde(rename = "sourceDevice")]
    pub _source_device: Option<i32>,
#[serde(rename = "timestamp")]
    pub _timestamp: i64,
    #[serde(rename = "dataMessage")]
    pub data_message: Option<SignalDataMessage>,
    #[serde(rename = "syncMessage")]
    pub sync_message: Option<SignalSyncMessage>,
    #[serde(rename = "typingMessage")]
    pub _typing_message: Option<serde_json::Value>,
    #[serde(rename = "receiptMessage")]
    pub _receipt_message: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct SignalDataMessage {
#[serde(rename = "timestamp")]
    pub _timestamp: i64,
    pub message: Option<String>,
    #[serde(rename = "expiresInSeconds")]
    pub _expires_in_seconds: Option<i64>,
    #[serde(rename = "groupInfo")]
    pub group_info: Option<SignalGroupInfo>,
#[serde(rename = "quote")]
    pub _quote: Option<SignalQuote>,
#[serde(rename = "mentions")]
    pub _mentions: Option<Vec<SignalMention>>,
#[serde(rename = "attachments")]
    pub _attachments: Option<Vec<SignalAttachment>>,
    pub reaction: Option<SignalReaction>,
}

#[derive(Debug, Deserialize)]
pub struct SignalSyncMessage {
    #[serde(rename = "sentMessage")]
    pub _sent_message: Option<SignalSentMessage>,
}

#[derive(Debug, Deserialize)]
pub struct SignalSentMessage {
#[serde(rename = "destination")]
    pub _destination: Option<String>,
    #[serde(rename = "destinationNumber")]
    pub _destination_number: Option<String>,
#[serde(rename = "timestamp")]
    pub _timestamp: i64,
#[serde(rename = "message")]
    pub _message: Option<String>,
    #[serde(rename = "groupInfo")]
    pub _group_info: Option<SignalGroupInfo>,
}

#[derive(Debug, Deserialize)]
pub struct SignalGroupInfo {
    #[serde(rename = "groupId")]
    pub group_id: String,
    #[serde(rename = "type")]
    pub _group_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SignalQuote {
#[serde(rename = "id")]
    pub _id: i64,
#[serde(rename = "author")]
    pub _author: Option<String>,
    #[serde(rename = "authorNumber")]
    pub _author_number: Option<String>,
#[serde(rename = "text")]
    pub _text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SignalMention {
#[serde(rename = "start")]
    pub _start: i32,
#[serde(rename = "length")]
    pub _length: i32,
#[serde(rename = "uuid")]
    pub _uuid: Option<String>,
#[serde(rename = "number")]
    pub _number: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SignalAttachment {
    #[serde(rename = "contentType")]
    pub _content_type: String,
#[serde(rename = "filename")]
    pub _filename: Option<String>,
#[serde(rename = "id")]
    pub _id: Option<String>,
#[serde(rename = "size")]
    pub _size: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct SignalReaction {
#[serde(rename = "emoji")]
    pub _emoji: String,
    #[serde(rename = "targetAuthor")]
    pub _target_author: Option<String>,
    #[serde(rename = "targetAuthorNumber")]
    pub _target_author_number: Option<String>,
    #[serde(rename = "targetSentTimestamp")]
    pub _target_sent_timestamp: i64,
    #[serde(rename = "isRemove")]
    pub _is_remove: bool,
}

/// Response to send back (signal-cli-rest-api will send the message)
#[derive(Debug, Serialize)]
pub struct SignalResponse {
    pub message: Option<String>,
    pub recipient: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
}

/// Handle incoming Signal webhook
pub async fn handle(
    State(state): State<AppState>,
    Json(webhook): Json<SignalWebhook>,
) -> Result<Json<SignalResponse>, StatusCode> {
    debug!("Received Signal webhook from account: {}", webhook.account);

    let envelope = &webhook.envelope;

    // Skip non-data messages (typing, receipts, etc.)
    let data_message = match &envelope.data_message {
        Some(dm) => dm,
        None => {
            // Check for sync message (message sent from another device)
            if envelope.sync_message.is_some() {
                debug!("Ignoring sync message");
            }
            return Ok(Json(SignalResponse {
                message: None,
                recipient: None,
                group_id: None,
            }));
        }
    };

    // Skip reactions
    if data_message.reaction.is_some() {
        debug!("Ignoring reaction");
        return Ok(Json(SignalResponse {
            message: None,
            recipient: None,
            group_id: None,
        }));
    }

    // Extract message text
    let text = match &data_message.message {
        Some(t) if !t.is_empty() => t.as_str(),
        _ => {
            debug!("No text content in message");
            return Ok(Json(SignalResponse {
                message: None,
                recipient: None,
                group_id: None,
            }));
        }
    };

    // Determine sender and recipient
    let sender = envelope
        .source_number
        .as_ref()
        .or(envelope.source.as_ref())
        .cloned()
        .unwrap_or_else(|| "unknown".to_string());

    let sender_name = envelope
        .source_name
        .clone()
        .unwrap_or_else(|| sender.clone());

    // Check if it's a group message
    let (session_id, group_id, recipient) = if let Some(group) = &data_message.group_info {
        (
            format!("signal:group:{}", group.group_id),
            Some(group.group_id.clone()),
            None,
        )
    } else {
        (
            format!("signal:{}", sender),
            None,
            Some(sender.clone()),
        )
    };

    info!(
        "Signal message from {} ({}): {}",
        sender_name,
        sender,
        text.chars().take(50).collect::<String>()
    );

    // Get or create agent for this session
    let system_prompt = if group_id.is_some() {
        format!(
            "You are Nanna — moon god of the digital realm.\n\
             You're in a Signal group chat. {} just sent a message.\n\
             Be helpful, concise, and conversational. Only respond if addressed or if you can add value.",
            sender_name
        )
    } else {
        format!(
            "You are Nanna — moon god of the digital realm.\n\
             You're chatting on Signal with {} ({}).\n\
             Be helpful, concise, and conversational.",
            sender_name, sender
        )
    };

    let agent = state
        .get_or_create_agent(&session_id, Some(&system_prompt))
        .await;

    // Process message with full agent capabilities
    let response = {
        let agent_guard = agent.read().await;
        agent_guard.run(text, RunOptions::default()).await
    };

    let response_text = match response {
        Ok(r) => Some(r.text),
        Err(e) => {
            warn!("Error processing Signal message: {}", e);
            Some("Sorry, I encountered an error processing your message.".to_string())
        }
    };

    Ok(Json(SignalResponse {
        message: response_text,
        recipient,
        group_id,
    }))
}

/// Health check / registration endpoint for signal-cli-rest-api
pub async fn health() -> &'static str {
    "OK"
}
