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
    // Required inputs must be present (an empty field can never verify).
    if signing_secret.is_empty() || signature.is_empty() {
        return false;
    }

    // Replay guard: the timestamp must parse and be within MAX_TIMESTAMP_AGE_SECS.
    let Ok(ts) = timestamp.parse::<i64>() else {
        warn!("Invalid Slack timestamp: {}", timestamp);
        return false;
    };
    let now = chrono::Utc::now().timestamp();
    if (now - ts).abs() > MAX_TIMESTAMP_AGE_SECS {
        warn!("Slack request timestamp too old: {}s", (now - ts).abs());
        return false;
    }

    // Slack signatures are `v0=<hex>`; strip the version prefix and hex-decode to
    // the raw digest so it can be compared as bytes.
    let Some(hex_digest) = signature.strip_prefix("v0=") else {
        return false;
    };
    let Ok(expected) = hex::decode(hex_digest) else {
        return false;
    };

    // HMAC-SHA256 over `v0:{timestamp}:{body}`. The body is hashed as **raw
    // bytes**, not a UTF-8-lossy string: the old `from_utf8(body).unwrap_or("")`
    // silently hashed an *empty* body for any non-UTF-8 payload, so a mangled
    // request could sail past with a signature computed over nothing.
    let mut mac = match HmacSha256::new_from_slice(signing_secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => {
            warn!("Invalid Slack signing secret");
            return false;
        }
    };
    mac.update(b"v0:");
    mac.update(timestamp.as_bytes());
    mac.update(b":");
    mac.update(body);

    // Constant-time verification via the MAC primitive (replaces the hand-rolled
    // hex-string compare).
    mac.verify_slice(&expected).is_ok()
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
    // Fail closed: without the signing secret every POST here is anonymous.
    let Some(signing_secret) =
        crate::webhooks::auth::configured(state.slack_signing_secret.as_ref())
    else {
        return Err(crate::webhooks::auth::refuse_unconfigured(
            "Slack",
            "channels.slack.signing_secret",
        ));
    };
    {
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
                            // A reaction that is not a feedback signal must change nothing:
                            // FSRS weights are the agent's long-term memory, and an emoji
                            // with no assigned meaning is not evidence either way.
                            if inner.event_type == "reaction_added" {
                                match classify_reaction(reaction) {
                                    ReactionFeedback::NotFeedback => {
                                        debug!(
                                            "Slack reaction {} on {} carries no feedback",
                                            reaction, message_key
                                        );
                                    }
                                    signal => {
                                        let positive = signal == ReactionFeedback::Positive;
                                        info!(
                                            "Slack reaction {} on {} (positive: {})",
                                            reaction, message_key, positive
                                        );
                                        state.record_message_feedback(&message_key, positive).await;
                                    }
                                }
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
    // Verify signature. Fail closed, same as the events endpoint — a slash
    // command runs the agent just as an event does.
    let Some(signing_secret) =
        crate::webhooks::auth::configured(state.slack_signing_secret.as_ref())
    else {
        return Err(crate::webhooks::auth::refuse_unconfigured(
            "Slack",
            "channels.slack.signing_secret",
        ));
    };
    {
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

/// What a reaction says about the memories behind the message it landed on.
///
/// Three states, not a `bool`, because "someone reacted" and "someone gave
/// feedback" are different facts. A 🍕 on an answer says nothing about whether
/// the answer was right, and a `bool` has nowhere to put that — which is how
/// the previous classifier came to return `true` (praise) for every emoji it
/// did not recognise, quietly promoting up to 50 recent memories per pizza.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReactionFeedback {
    /// Read as approval: promote the memories behind the message.
    Positive,
    /// Read as a correction: demote them.
    Negative,
    /// Not a feedback signal. Nothing happens.
    NotFeedback,
}

/// Reaction names read as approval. Matched exactly, after the skin-tone
/// modifier is stripped — substring matching is what made `broken_heart`
/// register as praise, because it contains `heart`.
const POSITIVE_REACTIONS: &[&str] = &[
    "+1",
    "100",
    "1st_place_medal",
    "ballot_box_with_check",
    "clap",
    "dart",
    "fire",
    "heart",
    "heart_eyes",
    "hearts",
    "heavy_check_mark",
    "medal",
    "muscle",
    "ok_hand",
    "pray",
    "raised_hands",
    "rocket",
    "sparkles",
    "sparkling_heart",
    "star",
    "star-struck",
    "star2",
    "sunglasses",
    "tada",
    "thumbs_up",
    "thumbsup",
    "trophy",
    "white_check_mark",
    "zap",
];

/// Reaction names read as a correction.
const NEGATIVE_REACTIONS: &[&str] = &[
    "-1",
    "angry",
    "broken_heart",
    "bug",
    "confused",
    "cry",
    "disappointed",
    "exploding_head",
    "facepalm",
    "hankey",
    "heavy_multiplication_x",
    "man-facepalm",
    "negative_squared_cross_mark",
    "no_entry",
    "no_entry_sign",
    "no_good",
    "poop",
    "rage",
    "skull",
    "sob",
    "thumbs_down",
    "thumbsdown",
    "warning",
    "woman-facepalm",
    "worried",
    "x",
];

/// Classify a Slack reaction name into a feedback signal.
///
/// Unrecognised reactions are [`ReactionFeedback::NotFeedback`] on purpose:
/// FSRS weights are the agent's long-term memory, and an emoji nobody assigned
/// a meaning to is not evidence about a memory's usefulness in either
/// direction. Silence is the honest answer.
fn classify_reaction(reaction: &str) -> ReactionFeedback {
    // Slack appends a skin-tone modifier to reactions that support one:
    // `thumbsup::skin-tone-6`. The tone carries no feedback, so the name is
    // everything before the separator.
    let name = reaction.split("::").next().unwrap_or(reaction).trim();
    debug_assert!(
        !name.contains("::"),
        "the skin-tone modifier must already be stripped"
    );

    if name.is_empty() {
        return ReactionFeedback::NotFeedback;
    }
    if POSITIVE_REACTIONS.contains(&name) {
        return ReactionFeedback::Positive;
    }
    if NEGATIVE_REACTIONS.contains(&name) {
        return ReactionFeedback::Negative;
    }
    ReactionFeedback::NotFeedback
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a valid Slack `v0=` signature for the given secret/timestamp/body.
    fn sign(secret: &str, timestamp: &str, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("key");
        mac.update(b"v0:");
        mac.update(timestamp.as_bytes());
        mac.update(b":");
        mac.update(body);
        format!("v0={}", hex::encode(mac.finalize().into_bytes()))
    }

    fn now_ts() -> String {
        chrono::Utc::now().timestamp().to_string()
    }

    #[test]
    fn accepts_a_valid_signature() {
        let secret = "topsecret";
        let ts = now_ts();
        let body = br#"{"type":"event_callback"}"#;
        let sig = sign(secret, &ts, body);
        assert!(verify_slack_signature(secret, &sig, &ts, body));
    }

    #[test]
    fn rejects_a_tampered_body() {
        let secret = "topsecret";
        let ts = now_ts();
        let sig = sign(secret, &ts, b"original body");
        assert!(!verify_slack_signature(secret, &sig, &ts, b"tampered body"));
    }

    #[test]
    fn rejects_a_stale_timestamp() {
        let secret = "topsecret";
        let stale = (chrono::Utc::now().timestamp() - MAX_TIMESTAMP_AGE_SECS - 60).to_string();
        let body = b"hello";
        let sig = sign(secret, &stale, body);
        assert!(
            !verify_slack_signature(secret, &sig, &stale, body),
            "a signature older than the replay window must be rejected"
        );
    }

    #[test]
    fn rejects_a_wrong_secret() {
        let ts = now_ts();
        let body = b"hello";
        let sig = sign("the-real-secret", &ts, body);
        let ok = verify_slack_signature("a-different-secret", &sig, &ts, body);
        assert!(!ok);
    }

    #[test]
    fn rejects_a_body_that_is_not_valid_utf8() {
        // The whole point of hashing raw bytes: a non-UTF-8 body must still be
        // verified correctly rather than silently hashed as empty.
        let secret = "topsecret";
        let ts = now_ts();
        let body: &[u8] = &[0xff, 0xfe, 0x00, 0x01, 0x80];
        let sig = sign(secret, &ts, body);
        assert!(verify_slack_signature(secret, &sig, &ts, body));
        // And a different non-UTF-8 body must not match that signature.
        let other: &[u8] = &[0xff, 0xfe, 0x00, 0x02, 0x80];
        assert!(!verify_slack_signature(secret, &sig, &ts, other));
    }

    #[test]
    fn rejects_missing_version_prefix_and_empty_inputs() {
        let ts = now_ts();
        let no_prefix = verify_slack_signature("s", "deadbeef", &ts, b"x");
        assert!(!no_prefix, "no v0= prefix");
        let empty_secret = verify_slack_signature("", "v0=deadbeef", &ts, b"x");
        assert!(!empty_secret, "empty secret");
        let empty_sig = verify_slack_signature("s", "", &ts, b"x");
        assert!(!empty_sig, "empty signature");
        let bad_ts = verify_slack_signature("s", "v0=deadbeef", "notanumber", b"x");
        assert!(!bad_ts, "bad timestamp");
    }

    #[test]
    fn known_reactions_classify_by_exact_name() {
        assert_eq!(classify_reaction("thumbsup"), ReactionFeedback::Positive);
        assert_eq!(classify_reaction("+1"), ReactionFeedback::Positive);
        assert_eq!(
            classify_reaction("white_check_mark"),
            ReactionFeedback::Positive
        );
        assert_eq!(classify_reaction("thumbsdown"), ReactionFeedback::Negative);
        assert_eq!(classify_reaction("-1"), ReactionFeedback::Negative);
        assert_eq!(classify_reaction("x"), ReactionFeedback::Negative);
    }

    #[test]
    fn an_unrecognised_reaction_is_not_feedback() {
        // The regression this replaces: the old classifier fell through to
        // `true`, so a pizza on an answer promoted every memory behind it.
        for name in ["pizza", "eyes", "wave", "cat", "spider", ""] {
            assert_eq!(
                classify_reaction(name),
                ReactionFeedback::NotFeedback,
                "{name} is not a feedback signal"
            );
        }
    }

    #[test]
    fn substring_lookalikes_no_longer_flip_the_sign() {
        // `broken_heart` contains `heart` and `heavy_multiplication_x`
        // contains `x`; substring matching read the first as praise.
        assert_eq!(
            classify_reaction("broken_heart"),
            ReactionFeedback::Negative
        );
        assert_eq!(
            classify_reaction("heavy_multiplication_x"),
            ReactionFeedback::Negative
        );
        assert_eq!(classify_reaction("heart"), ReactionFeedback::Positive);
        // And a name that merely *contains* a known one is not that one.
        assert_eq!(
            classify_reaction("thumbsup_but_sarcastic"),
            ReactionFeedback::NotFeedback
        );
    }

    #[test]
    fn skin_tone_modifiers_are_stripped() {
        // Slack sends `name::skin-tone-N` for N in 2..=6.
        for tone in 2..=6 {
            assert_eq!(
                classify_reaction(&format!("thumbsup::skin-tone-{tone}")),
                ReactionFeedback::Positive
            );
            assert_eq!(
                classify_reaction(&format!("thumbsdown::skin-tone-{tone}")),
                ReactionFeedback::Negative
            );
        }
    }

    #[test]
    fn the_two_reaction_tables_are_disjoint_and_well_formed() {
        for name in POSITIVE_REACTIONS.iter().chain(NEGATIVE_REACTIONS) {
            assert!(!name.is_empty(), "an empty reaction name matches nothing");
            assert!(
                !name.contains("::"),
                "{name} carries a modifier the lookup already strips"
            );
            assert_eq!(
                *name,
                name.to_lowercase(),
                "{name} must be lowercase to match Slack's names"
            );
        }
        for name in POSITIVE_REACTIONS {
            assert!(
                !NEGATIVE_REACTIONS.contains(name),
                "{name} cannot be both approval and correction"
            );
        }
    }
}
