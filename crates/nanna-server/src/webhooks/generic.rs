//! Generic webhook handler for custom integrations

use crate::state::AppState;
use crate::webhooks::auth;
use axum::{Json, body::Bytes, extract::State, http::HeaderMap, http::StatusCode};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

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

/// The conversation key for a generic-webhook request.
///
/// Stable, not fresh. The previous fallback appended a `Uuid::new_v4()`, so a
/// caller that does not thread `session_id` back itself got a **brand-new
/// conversation on every request** — the agent had no memory across turns on
/// this route alone. Its four siblings all derive a stable key from the
/// identity fields they are given (`telegram:{chat_id}:{user_id}`,
/// `discord:{channel_id}:{user_id}`, …), and `channel` and `user_id` are both
/// required on this payload, so there is nothing to fall back to and no
/// randomness to add.
///
/// An explicitly supplied `session_id` still wins, so a caller that manages its
/// own sessions is unaffected. A blank one does not count as supplied — it is
/// the shape a half-filled template leaves behind, and honouring it would put
/// every such caller in one session named `""`.
pub fn session_key(session_id: Option<&str>, channel: &str, user_id: &str) -> String {
    if let Some(explicit) = session_id.map(str::trim).filter(|s| !s.is_empty()) {
        return explicit.to_string();
    }
    let key = format!("generic:{channel}:{user_id}");
    debug_assert!(
        key.starts_with("generic:"),
        "a derived key must be namespaced, or it can collide with another provider's"
    );
    key
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

    let session_id = session_key(
        webhook.session_id.as_deref(),
        &webhook.channel,
        &webhook.user_id,
    );

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

#[cfg(test)]
mod tests {
    use super::session_key;

    #[test]
    fn the_derived_key_is_stable_across_requests() {
        // The bug this pins: the fallback used to append a fresh
        // `Uuid::new_v4()`, so two identical requests landed in two different
        // conversations and the agent never remembered a thing on this route.
        let first = session_key(None, "zapier", "alice");
        let second = session_key(None, "zapier", "alice");
        assert_eq!(first, second, "the same caller is the same conversation");
        assert_eq!(first, "generic:zapier:alice");
    }

    #[test]
    fn distinct_callers_get_distinct_keys() {
        let alice = session_key(None, "zapier", "alice");
        let bob = session_key(None, "zapier", "bob");
        assert_ne!(alice, bob, "two users are two conversations");

        let other_channel = session_key(None, "n8n", "alice");
        assert_ne!(alice, other_channel, "two channels are two conversations");
    }

    #[test]
    fn an_explicit_session_id_wins() {
        let explicit = session_key(Some("my-own-thread"), "zapier", "alice");
        assert_eq!(
            explicit, "my-own-thread",
            "a caller that manages its own sessions is unaffected"
        );
        // Surrounding whitespace is not part of the identity.
        assert_eq!(
            session_key(Some("  my-own-thread  "), "zapier", "alice"),
            "my-own-thread"
        );
    }

    #[test]
    fn a_blank_session_id_falls_back_rather_than_naming_a_shared_room() {
        // `Some("")` is what a half-filled template leaves behind. Honouring it
        // would put every such caller into one conversation named "".
        for blank in ["", "   ", "	 "] {
            assert_eq!(
                session_key(Some(blank), "zapier", "alice"),
                "generic:zapier:alice",
                "blank {blank:?} is not a session id"
            );
        }
    }

    #[test]
    fn a_derived_key_cannot_collide_with_another_providers() {
        // The other handlers build `telegram:{chat}:{user}` /
        // `discord:{chan}:{user}` etc. into the same session namespace, so an
        // un-namespaced key here could land a webhook caller inside somebody's
        // Telegram conversation.
        let key = session_key(None, "telegram", "12345");
        assert!(key.starts_with("generic:"), "got {key}");
        assert_ne!(
            key,
            "telegram:12345",
            "must not alias a real Telegram session"
        );
    }
}
