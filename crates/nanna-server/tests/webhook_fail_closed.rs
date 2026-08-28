//! `nanna serve`'s inbound webhook surface must fail CLOSED.
//!
//! The `nanna-daemon` copy of this surface has had an end-to-end harness since
//! 2026-08-22; this crate's copy — the one `nanna serve` binds — had only unit
//! tests over the shared `auth` primitives plus compile-checked wiring. That is
//! exactly the gap the daemon bug lived in: **every verifier was correct and
//! every handler skipped it**, so a full unit suite passed while five endpoints
//! accepted anonymous payloads. A test that never drives the handler cannot see
//! a handler that never calls the verifier.
//!
//! So these drive the real `create_router()` through `tower::ServiceExt::oneshot`
//! — real axum routing, real extractors, real handler bodies — over a real
//! `AppState`. Three properties, mirroring the daemon suite:
//!
//! 1. **Unconfigured ⇒ refused.** A channel with no credential answers `503`
//!    and processes nothing. 503 and not 401, so an operator reading a log can
//!    tell "this host never armed this channel" from "your proof was wrong".
//! 2. **Configured ⇒ proof required.** With a credential set, a request
//!    carrying none — or the wrong one — answers `401`, and one carrying the
//!    right proof is admitted.
//! 3. **Blank ⇒ unconfigured.** `Some("")` is the shape a half-finished config
//!    leaves behind and the one shape that could authenticate everybody.
//!
//! Plus a fourth the daemon suite cannot express, because only this crate signs
//! its own fixtures: a **genuinely Ed25519-signed Discord capture with a
//! day-old timestamp is refused**, proving the replay window is wired into this
//! copy and not merely defined in it.
//!
//! ## Why the admitted requests are safe to make
//!
//! Every payload chosen for an "admitted" leg reaches a handler branch that
//! returns before any agent work: Discord `PING`→`PONG`, Slack
//! `url_verification`→challenge echo, a Telegram update with no message, a
//! Signal envelope with no `dataMessage`. So passing authentication proves
//! authentication passed — it never reaches the LLM, and the suite needs no
//! network and no API key. The generic hook has no such branch (it runs
//! whatever `message` it is handed, which is what makes it the most abusable
//! route), so it is asserted on its refusal legs only.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ed25519_dalek::{Signer, SigningKey};
use hmac::{Hmac, KeyInit, Mac};
use nanna_core::{LlmClient, Nanna, NannaConfig};
use nanna_server::{AppState, AppStateBuilder, create_router};
use nanna_storage::Storage;
use nanna_tools::ToolRegistry;
use sha2::Sha256;
use std::sync::Arc;
use tower::ServiceExt;

/// A syntactically valid payload for each route that parses its body.
///
/// The `Json<T>` extractor runs before the handler body, so a malformed payload
/// would answer 422 and the authentication branch under test would never run.
/// These are the smallest bodies that deserialize *and* reach a no-agent
/// branch.
const TELEGRAM_NO_MESSAGE: &str = r#"{"update_id":1}"#;
const SIGNAL_NO_DATA_MESSAGE: &str = r#"{"account":"+15555550100","envelope":{"timestamp":0}}"#;
const GENERIC_BODY: &str = r#"{"channel":"test","user_id":"u1","message":"hello"}"#;
const DISCORD_PING: &str = r#"{"id":"1","type":1,"token":"t"}"#;
const SLACK_URL_VERIFICATION: &str = r#"{"type":"url_verification","challenge":"c0ffee"}"#;

/// Every webhook route, paired with a body that deserializes on it.
fn routes() -> [(&'static str, &'static str); 5] {
    [
        ("/webhooks/telegram", TELEGRAM_NO_MESSAGE),
        ("/webhooks/discord", DISCORD_PING),
        ("/webhooks/slack", SLACK_URL_VERIFICATION),
        ("/webhooks/signal", SIGNAL_NO_DATA_MESSAGE),
        ("/webhooks/generic", GENERIC_BODY),
    ]
}

/// Build a real `AppState` with no credentials configured.
///
/// The LLM key is deliberately a placeholder: no admitted request in this suite
/// reaches the model, so a real one would only be an opportunity to spend
/// money. GPU probing, dreaming and the scheduler are off — none is on the path
/// under test, and each would start work a status-code assertion has to outlive.
async fn unconfigured_state() -> AppState {
    let storage = Storage::in_memory()
        .await
        .expect("an in-memory store opens");
    let bot_config = NannaConfig {
        enable_gpu: false,
        ..Default::default()
    };
    let bot = Nanna::new(bot_config, LlmClient::anthropic("test-key-never-used"))
        .await
        .expect("constructing Nanna touches no network");

    AppStateBuilder::new()
        .bot(bot)
        .storage_arc(Arc::new(storage))
        .llm(LlmClient::anthropic("test-key-never-used"))
        .tools_arc(Arc::new(ToolRegistry::new()))
        .dreaming(false)
        .scheduler(false)
        .build()
}

/// An `AppState` with every shared-secret credential armed.
async fn armed_state() -> AppState {
    let mut state = unconfigured_state().await;
    state.telegram_webhook_secret = Some("tg-secret".to_string());
    state.signal_webhook_secret = Some("sig-secret".to_string());
    state.webhook_secret = Some("gen-secret".to_string());
    state
}

/// POST `body` to `path` with `headers` and report the status code.
async fn post(state: AppState, path: &str, body: &str, headers: &[(&str, &str)]) -> StatusCode {
    let mut req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    let req = req
        .body(Body::from(body.to_string()))
        .expect("a valid request");

    create_router(state)
        .oneshot(req)
        .await
        .expect("the router answers every request")
        .status()
}

#[tokio::test]
async fn every_unconfigured_route_refuses_with_503() {
    for (path, body) in routes() {
        let status = post(unconfigured_state().await, path, body, &[]).await;
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{path} must refuse while unconfigured, got {status}"
        );
    }
}

#[tokio::test]
async fn a_configured_shared_secret_channel_still_demands_proof() {
    // Telegram, Signal and the generic hook all authenticate with a shared
    // secret; before 2026-08-22 the first two read no header at all.
    let cases: [(&str, &str, &str, &str); 3] = [
        (
            "/webhooks/telegram",
            TELEGRAM_NO_MESSAGE,
            "X-Telegram-Bot-Api-Secret-Token",
            "tg-secret",
        ),
        (
            "/webhooks/signal",
            SIGNAL_NO_DATA_MESSAGE,
            "X-Webhook-Secret",
            "sig-secret",
        ),
        (
            "/webhooks/generic",
            GENERIC_BODY,
            "X-Webhook-Secret",
            "gen-secret",
        ),
    ];

    for (path, body, header, secret) in cases {
        let missing = post(armed_state().await, path, body, &[]).await;
        assert_eq!(
            missing,
            StatusCode::UNAUTHORIZED,
            "{path} with no proof must be 401, got {missing}"
        );

        let wrong = post(armed_state().await, path, body, &[(header, "wrong")]).await;
        assert_eq!(
            wrong,
            StatusCode::UNAUTHORIZED,
            "{path} with a wrong proof must be 401, got {wrong}"
        );

        // A prefix of the real secret must not pass: `secret_matches` is
        // constant-time precisely so a byte-by-byte compare cannot be walked.
        let prefix = &secret[..secret.len() - 1];
        let partial = post(armed_state().await, path, body, &[(header, prefix)]).await;
        assert_eq!(
            partial,
            StatusCode::UNAUTHORIZED,
            "{path} with a prefix of the secret must be 401, got {partial}"
        );

        let right = post(armed_state().await, path, body, &[(header, secret)]).await;
        assert_ne!(
            right,
            StatusCode::UNAUTHORIZED,
            "{path} rejects its own secret"
        );
        assert_ne!(
            right,
            StatusCode::SERVICE_UNAVAILABLE,
            "{path} is configured, so it is not 'unconfigured'"
        );
    }
}

#[tokio::test]
async fn the_generic_hook_requires_the_bearer_scheme() {
    // A bare token without the `Bearer ` scheme is not the secret — the
    // `Authorization` arm strips a required prefix rather than comparing whole
    // header values.
    let bare = post(
        armed_state().await,
        "/webhooks/generic",
        GENERIC_BODY,
        &[("Authorization", "gen-secret")],
    )
    .await;
    assert_eq!(bare, StatusCode::UNAUTHORIZED, "the scheme is required");

    // Two wrong guesses, one in each accepted header, must not combine into a
    // pass: both arms are evaluated, and neither matching is still a refusal.
    let both_wrong = post(
        armed_state().await,
        "/webhooks/generic",
        GENERIC_BODY,
        &[
            ("Authorization", "Bearer nope"),
            ("X-Webhook-Secret", "also-nope"),
        ],
    )
    .await;
    assert_eq!(
        both_wrong,
        StatusCode::UNAUTHORIZED,
        "two wrong guesses are still wrong"
    );
}

#[tokio::test]
async fn a_blank_configured_secret_is_treated_as_unconfigured() {
    // `Some("")` reads as "a secret exists" through `Option::is_some`, and
    // comparing an empty secret against an absent header is `"" == ""` — a
    // credential nothing can fail to present.
    for blank in ["", "   \t "] {
        let mut state = unconfigured_state().await;
        state.telegram_webhook_secret = Some(blank.to_string());
        state.signal_webhook_secret = Some(blank.to_string());
        state.webhook_secret = Some(blank.to_string());
        state.slack_signing_secret = Some(blank.to_string());
        state.discord_public_key = Some(blank.to_string());

        for (path, body) in routes() {
            let status = post(state.clone(), path, body, &[]).await;
            assert_eq!(
                status,
                StatusCode::SERVICE_UNAVAILABLE,
                "{path} with a blank secret ({blank:?}) must refuse, got {status}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Signed channels: Slack HMAC-SHA256 and Discord Ed25519.
// ---------------------------------------------------------------------------

/// Sign `body` the way Slack does: `v0=` + HMAC-SHA256 over `v0:ts:body`.
fn slack_signature(secret: &str, timestamp: &str, body: &str) -> String {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts a key of any length");
    mac.update(format!("v0:{timestamp}:{body}").as_bytes());
    format!("v0={}", hex::encode(mac.finalize().into_bytes()))
}

#[tokio::test]
async fn slack_admits_only_a_correctly_signed_request() {
    const SECRET: &str = "slack-signing-secret";
    async fn armed(secret: &str) -> AppState {
        let mut state = unconfigured_state().await;
        state.slack_signing_secret = Some(secret.to_string());
        state
    }
    let now = chrono::Utc::now().timestamp().to_string();

    let unsigned = post(
        armed(SECRET).await,
        "/webhooks/slack",
        SLACK_URL_VERIFICATION,
        &[],
    )
    .await;
    assert_eq!(
        unsigned,
        StatusCode::UNAUTHORIZED,
        "an unsigned POST is refused"
    );

    let forged = post(
        armed(SECRET).await,
        "/webhooks/slack",
        SLACK_URL_VERIFICATION,
        &[
            ("X-Slack-Signature", "v0=deadbeef"),
            ("X-Slack-Request-Timestamp", &now),
        ],
    )
    .await;
    assert_eq!(
        forged,
        StatusCode::UNAUTHORIZED,
        "a forged signature is refused"
    );

    let signature = slack_signature(SECRET, &now, SLACK_URL_VERIFICATION);
    let signed = post(
        armed(SECRET).await,
        "/webhooks/slack",
        SLACK_URL_VERIFICATION,
        &[
            ("X-Slack-Signature", &signature),
            ("X-Slack-Request-Timestamp", &now),
        ],
    )
    .await;
    assert_eq!(
        signed,
        StatusCode::OK,
        "a correctly signed challenge is answered"
    );

    // A signature that is valid for a *different* body must not carry this one:
    // the HMAC covers the body, and a test that only flipped the key would not
    // notice a verifier that ignored it.
    let other = slack_signature(
        SECRET,
        &now,
        r#"{"type":"url_verification","challenge":"x"}"#,
    );
    let swapped = post(
        armed(SECRET).await,
        "/webhooks/slack",
        SLACK_URL_VERIFICATION,
        &[
            ("X-Slack-Signature", &other),
            ("X-Slack-Request-Timestamp", &now),
        ],
    )
    .await;
    assert_eq!(
        swapped,
        StatusCode::UNAUTHORIZED,
        "a signature for another body is refused"
    );
}

#[tokio::test]
async fn discord_admits_only_a_fresh_correctly_signed_request() {
    // A deterministic signing key, and the hex public key an operator pastes
    // into config.
    let signing = SigningKey::from_bytes(&[7u8; 32]);
    let public_key = hex::encode(signing.verifying_key().to_bytes());

    async fn armed(public_key: &str) -> AppState {
        let mut state = unconfigured_state().await;
        state.discord_public_key = Some(public_key.to_string());
        state
    }
    let sign = |timestamp: &str| {
        hex::encode(
            signing
                .sign(format!("{timestamp}{DISCORD_PING}").as_bytes())
                .to_bytes(),
        )
    };
    let now = chrono::Utc::now().timestamp().to_string();

    let unsigned = post(
        armed(&public_key).await,
        "/webhooks/discord",
        DISCORD_PING,
        &[],
    )
    .await;
    assert_eq!(
        unsigned,
        StatusCode::UNAUTHORIZED,
        "an unsigned interaction is refused"
    );

    let signature = sign(&now);
    let fresh = post(
        armed(&public_key).await,
        "/webhooks/discord",
        DISCORD_PING,
        &[
            ("X-Signature-Ed25519", &signature),
            ("X-Signature-Timestamp", &now),
        ],
    )
    .await;
    assert_eq!(
        fresh,
        StatusCode::OK,
        "a fresh signed PING is answered with PONG"
    );

    // The replay window. Discord signs a timestamp but publishes no tolerance,
    // so this capture is *cryptographically valid forever* — only the window
    // refuses it. That makes this the one assertion that would still fail if
    // the freshness check were deleted and the Ed25519 check kept.
    let day_old = (chrono::Utc::now().timestamp() - 86_400).to_string();
    let replay = post(
        armed(&public_key).await,
        "/webhooks/discord",
        DISCORD_PING,
        &[
            ("X-Signature-Ed25519", &sign(&day_old)),
            ("X-Signature-Timestamp", &day_old),
        ],
    )
    .await;
    assert_eq!(
        replay,
        StatusCode::UNAUTHORIZED,
        "a day-old capture verifies under Ed25519 and must be refused by the replay window"
    );

    // A signature made over a different timestamp than the one presented must
    // not verify: the timestamp is part of the signed message, not a hint.
    let mismatched = post(
        armed(&public_key).await,
        "/webhooks/discord",
        DISCORD_PING,
        &[
            ("X-Signature-Ed25519", &sign("1234567890")),
            ("X-Signature-Timestamp", &now),
        ],
    )
    .await;
    assert_eq!(
        mismatched,
        StatusCode::UNAUTHORIZED,
        "the timestamp is signed, not asserted"
    );
}

#[tokio::test]
async fn a_malformed_body_never_outranks_the_refusal() {
    // Authentication must run before the body is parsed. When it did not, the
    // three `Json<T>` routes answered the parser's **400** to an anonymous
    // caller on an *unconfigured* channel — burying the 503 whose whole job is
    // to tell an operator "this host never armed this channel", and letting an
    // unauthenticated caller drive `serde_json` besides. Slack and Discord
    // always took `Bytes` and always answered 503 here, which is what made the
    // split visible.
    const MALFORMED: &str = "{not json";

    for (path, _) in routes() {
        let status = post(unconfigured_state().await, path, MALFORMED, &[]).await;
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{path} is unconfigured, so it must refuse before parsing, got {status}"
        );
    }

    // Armed but unproven: still 401, not the parser's 400.
    for (path, header) in [
        ("/webhooks/telegram", "X-Telegram-Bot-Api-Secret-Token"),
        ("/webhooks/signal", "X-Webhook-Secret"),
        ("/webhooks/generic", "X-Webhook-Secret"),
    ] {
        let status = post(armed_state().await, path, MALFORMED, &[(header, "wrong")]).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{path} refuses a wrong proof before parsing, got {status}"
        );
    }

    // Armed *and* proven: now the body is the caller's problem, and 400 is the
    // honest answer — the caller is known, so this is a bad request and not an
    // anonymous one.
    for (path, header, secret) in [
        (
            "/webhooks/telegram",
            "X-Telegram-Bot-Api-Secret-Token",
            "tg-secret",
        ),
        ("/webhooks/signal", "X-Webhook-Secret", "sig-secret"),
        ("/webhooks/generic", "X-Webhook-Secret", "gen-secret"),
    ] {
        let status = post(armed_state().await, path, MALFORMED, &[(header, secret)]).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{path} parses only after the caller is known, got {status}"
        );
    }
}
