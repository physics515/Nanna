//! The inbound webhook surface must fail CLOSED.
//!
//! These drive the **real** `WebhookServer` over a real TCP socket with a real
//! HTTP client — not the verifier functions in isolation — because the bug
//! being guarded against lived entirely in the handler wiring: each verifier
//! was correct and each handler skipped it when nothing was configured, so
//! every unit test of the verifiers passed while the endpoints were open.
//!
//! Two properties are asserted end to end:
//!
//! 1. **Unconfigured ⇒ refused.** A channel with no credential answers
//!    `503 Service Unavailable` and processes nothing. 503 (not 401) so an
//!    operator can tell "never armed" from "wrong proof".
//! 2. **Configured ⇒ proof required.** With a credential set, a request that
//!    presents none — or the wrong one — answers `401 Unauthorized`.

use nanna_daemon::{WebhookConfig, WebhookServer};
use std::collections::HashMap;

/// Reserve a free loopback port by binding and immediately releasing it.
///
/// The server takes its port from config and never reports the bound address,
/// so the port has to be chosen before it starts. The race between release and
/// re-bind is unavoidable and vanishingly small on loopback; a collision shows
/// up as a connection error, not as a false pass.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let port = listener
        .local_addr()
        .expect("an ephemeral bind has a local address")
        .port();
    assert_ne!(port, 0, "the OS must have assigned a real port");
    port
}

/// Start a webhook server on loopback and wait until it accepts connections.
async fn start(config: WebhookConfig) -> (String, tokio::task::JoinHandle<()>) {
    let port = free_port();
    let mut config = config;
    config.host = "127.0.0.1".to_string();
    config.port = port;

    let (server, _rx) = WebhookServer::new(config);
    let handle = server.spawn().0;

    // Poll rather than sleep a fixed amount: the bind is the readiness signal.
    let base = format!("http://127.0.0.1:{port}");
    for _ in 0..200 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return (base, handle);
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("webhook server did not start listening on {base}");
}

async fn post(client: &reqwest::Client, url: &str, headers: &[(&str, &str)]) -> u16 {
    let mut req = client.post(url).body(r#"{"hello":"world"}"#);
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    req.send().await.expect("request reaches the server").status().as_u16()
}

#[tokio::test]
async fn every_unconfigured_channel_refuses_with_503() {
    // The default config configures nothing — which used to mean every endpoint
    // accepted every payload from anyone who could reach the port.
    let (base, handle) = start(WebhookConfig::default()).await;
    let client = reqwest::Client::new();

    for path in [
        "/webhook/telegram",
        "/webhook/discord",
        "/webhook/slack",
        "/webhook/whatsapp",
        "/webhook/anything-at-all",
    ] {
        let status = post(&client, &format!("{base}{path}"), &[]).await;
        assert_eq!(
            status, 503,
            "{path} must refuse while unconfigured, got {status}"
        );
    }

    handle.abort();
}

#[tokio::test]
async fn a_configured_channel_still_demands_proof() {
    let mut config = WebhookConfig::default();
    config.telegram_secret = Some("telegram-secret".to_string());
    config
        .generic_secrets
        .insert("hooks".to_string(), "generic-secret".to_string());
    let (base, handle) = start(config).await;
    let client = reqwest::Client::new();

    // Telegram: no header, wrong header, right header.
    let missing = post(&client, &format!("{base}/webhook/telegram"), &[]).await;
    assert_eq!(missing, 401, "a Telegram POST with no secret token is refused");
    let wrong = post(
        &client,
        &format!("{base}/webhook/telegram"),
        &[("X-Telegram-Bot-Api-Secret-Token", "wrong")],
    )
    .await;
    assert_eq!(wrong, 401, "a wrong secret token is refused");
    let right = post(
        &client,
        &format!("{base}/webhook/telegram"),
        &[("X-Telegram-Bot-Api-Secret-Token", "telegram-secret")],
    )
    .await;
    assert_ne!(right, 401, "the configured secret token is accepted");
    assert_ne!(right, 503, "a configured channel is not 'unconfigured'");

    // Generic: the registered id demands its secret; an unregistered id is
    // refused as unconfigured rather than accepted.
    let no_token = post(&client, &format!("{base}/webhook/hooks"), &[]).await;
    assert_eq!(no_token, 401, "a registered generic hook demands its secret");
    let bearer = post(
        &client,
        &format!("{base}/webhook/hooks"),
        &[("Authorization", "Bearer generic-secret")],
    )
    .await;
    assert_eq!(bearer, 200, "the bearer form of the secret is accepted");
    let header = post(
        &client,
        &format!("{base}/webhook/hooks"),
        &[("X-Webhook-Secret", "generic-secret")],
    )
    .await;
    assert_eq!(header, 200, "the header form of the secret is accepted");
    let unknown = post(&client, &format!("{base}/webhook/not-registered"), &[]).await;
    assert_eq!(unknown, 503, "an unregistered id is refused, not served");

    handle.abort();
}

#[tokio::test]
async fn a_blank_configured_secret_is_treated_as_unconfigured() {
    // `Some("")` is what a half-finished config leaves behind, and it is the
    // one shape that could authenticate everybody: comparing an empty secret
    // against an absent header is `"" == ""`.
    let mut config = WebhookConfig::default();
    config.telegram_secret = Some(String::new());
    config.slack_signing_secret = Some("   ".to_string());
    let mut secrets = HashMap::new();
    secrets.insert("blank".to_string(), String::new());
    config.generic_secrets = secrets;

    let (base, handle) = start(config).await;
    let client = reqwest::Client::new();

    for path in ["/webhook/telegram", "/webhook/slack", "/webhook/blank"] {
        let status = post(&client, &format!("{base}{path}"), &[]).await;
        assert_eq!(status, 503, "{path} with a blank secret must refuse");
    }

    handle.abort();
}
