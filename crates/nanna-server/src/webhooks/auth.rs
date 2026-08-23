//! Origin checks shared by every webhook handler in this crate.
//!
//! **The governing rule is fail closed.** Each of these endpoints hands its
//! payload to the agent, which runs tools with this process's privileges, so an
//! endpoint that cannot prove who called it is a remote command channel for
//! anyone who learns the URL. Verification used to be conditional on a
//! credential being configured, which made the *unconfigured* case the least
//! protected one — exactly backwards.
//!
//! These are deliberate twins of the helpers in
//! `nanna-daemon/src/webhook.rs`. The two crates do not depend on each other
//! and the daemon is the surface P16 keeps; this copy exists because
//! `nanna serve` still binds the same routes. Change one, change the other —
//! the same note guards the Ed25519 and HMAC verifiers below them.

use axum::http::{HeaderMap, StatusCode};
use subtle::ConstantTimeEq;
use tracing::warn;

/// Widest age a provider-signed unix timestamp may have before it is treated as
/// a replay, in seconds.
///
/// Derived, not magic. Slack documents a 5-minute tolerance on
/// `X-Slack-Request-Timestamp` and rejects outside it, so accepting anything
/// wider would honour a window the provider itself would not. Discord signs a
/// timestamp too but publishes no tolerance, so it inherits this one — the
/// alternative is an unbounded replay window on a signature that never expires.
/// The bound must cover ordinary provider↔host clock skew: 5 minutes is ~2
/// orders of magnitude above NTP-synced skew and far below the time a captured
/// POST stays useful.
pub const MAX_TIMESTAMP_AGE_SECS: i64 = 300;

/// Longest header value that can still be a unix-seconds timestamp.
///
/// 20 digits covers every representable second for the rest of this universe.
/// Anything longer is not a timestamp, and bounding it keeps a caller from
/// handing the parser a megabyte of digits.
const MAX_TIMESTAMP_HEADER_LEN: usize = 20;

/// Treat a credential as configured only when it is present AND non-blank.
///
/// `Some("")` is the shape a half-finished config (or a keyring miss that wrote
/// through empty) leaves behind, and it is the dangerous shape: every caller
/// reads "a secret exists" from `Option::is_some`, so a blank one would arm the
/// endpoint with a credential nothing can fail to present.
pub fn configured(value: Option<&String>) -> Option<&str> {
    let present = value.map(String::as_str).filter(|v| !v.trim().is_empty());
    debug_assert!(
        present.is_none_or(|v| !v.trim().is_empty()),
        "configured() must never yield a blank credential"
    );
    present
}

/// Constant-time comparison of a configured shared secret against what a
/// request presented (absent → `""`).
///
/// A plain `==` compares byte-by-byte and short-circuits on the first mismatch,
/// leaking through response timing how much of a guess is correct. An **empty**
/// `expected` never matches: comparing an empty secret to an absent header is
/// `"" == ""`, which would authenticate every caller on earth.
pub fn secret_matches(expected: &str, provided: Option<&str>) -> bool {
    if expected.is_empty() {
        return false;
    }
    debug_assert!(
        !expected.is_empty(),
        "the empty-secret guard above must have returned"
    );
    let provided = provided.unwrap_or("");
    expected.as_bytes().ct_eq(provided.as_bytes()).into()
}

/// True when a provider-signed unix timestamp sits inside the replay window.
///
/// Returns `false` for every failure mode — unparseable, absurdly long. A
/// replay guard that cannot establish an age must refuse, never wave through on
/// an unknown one.
pub fn timestamp_is_fresh(timestamp: &str) -> bool {
    debug_assert!(
        MAX_TIMESTAMP_AGE_SECS > 0,
        "a zero window would reject every request, including legitimate ones"
    );
    debug_assert!(
        MAX_TIMESTAMP_HEADER_LEN >= i64::MAX.to_string().len(),
        "the header bound must still admit every representable timestamp"
    );

    if timestamp.is_empty() || timestamp.len() > MAX_TIMESTAMP_HEADER_LEN {
        return false;
    }
    let Ok(ts) = timestamp.parse::<i64>() else {
        return false;
    };
    let now = chrono::Utc::now().timestamp();
    now.saturating_sub(ts).abs() <= MAX_TIMESTAMP_AGE_SECS
}

/// Refuse a webhook for a channel whose credential is not configured.
///
/// The status is 503 and not 401 deliberately: 401 says "your proof was
/// wrong", 503 says "this host never armed this channel", and an operator
/// reading a log has to be able to tell those apart. The message names the one
/// config key that turns the endpoint on.
pub fn refuse_unconfigured(channel: &str, config_key: &str) -> StatusCode {
    warn!(
        "{channel} webhook refused: no credential is configured, so the caller cannot be \
         verified. Nothing was processed. Set `{config_key}` and restart to enable this \
         endpoint."
    );
    StatusCode::SERVICE_UNAVAILABLE
}

/// Check a shared-secret bearer credential presented as either
/// `Authorization: Bearer <secret>` or `X-Webhook-Secret: <secret>`.
///
/// Both arms are evaluated unconditionally: `||` would skip the second compare
/// whenever the first matched, and the timing of that skip is itself a signal
/// about which header was correct.
pub fn bearer_secret_ok(headers: &HeaderMap, expected: &str) -> bool {
    let bearer = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let direct = headers
        .get("X-Webhook-Secret")
        .and_then(|v| v.to_str().ok());

    let bearer_ok = secret_matches(expected, bearer);
    let direct_ok = secret_matches(expected, direct);
    bearer_ok | direct_ok
}

/// Deserialize a webhook body **after** its caller has been authenticated.
///
/// The `Json<T>` extractor cannot be used on these routes. Axum runs extractors
/// before the handler body, so `Json<T>` would parse an unauthenticated
/// caller's bytes before any credential is looked at — and, worse, an
/// **unconfigured** channel handed a malformed body would answer the parser's
/// `400` instead of the `503` whose entire job is to tell an operator "this
/// host never armed this channel". Taking `Bytes` and calling this after the
/// auth branch restores the intended order, and matches what the Slack and
/// Discord handlers here (and every handler in `nanna-daemon`) already do.
///
/// A parse failure is `400`: by the time this runs the caller has proved who it
/// is, so a bad body is a bad request and not an anonymous one.
pub fn parse_authenticated_body<T: serde::de::DeserializeOwned>(
    channel: &str,
    body: &[u8],
) -> Result<T, StatusCode> {
    debug_assert!(
        !channel.is_empty(),
        "the log line must name the channel it refused"
    );
    serde_json::from_slice(body).map_err(|e| {
        warn!("{channel} webhook: authenticated caller sent an unparseable payload: {e}");
        StatusCode::BAD_REQUEST
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_credential_is_not_configured() {
        assert_eq!(configured(None), None);
        let empty = String::new();
        assert_eq!(configured(Some(&empty)), None, "empty is not configured");
        let blank = "  \t ".to_string();
        assert_eq!(configured(Some(&blank)), None, "blank is not configured");
        let real = "s3cr3t".to_string();
        assert_eq!(configured(Some(&real)), Some("s3cr3t"));
    }

    #[test]
    fn an_empty_secret_authenticates_nobody() {
        assert!(!secret_matches("", None), "empty vs absent");
        assert!(!secret_matches("", Some("")), "empty vs empty");
        assert!(!secret_matches("", Some("guess")), "empty vs anything");
    }

    #[test]
    fn secret_matches_only_on_exact_equality() {
        assert!(secret_matches("s3cr3t", Some("s3cr3t")));
        assert!(!secret_matches("s3cr3t", Some("s3cr3T")), "one wrong byte");
        assert!(!secret_matches("s3cr3t", Some("s3cr3")), "a prefix");
        assert!(!secret_matches("s3cr3t", Some("s3cr3t2")), "a superstring");
        assert!(!secret_matches("s3cr3t", None), "an absent token");
    }

    #[test]
    fn the_replay_window_accepts_now_and_refuses_captures() {
        let now = chrono::Utc::now().timestamp();
        assert!(timestamp_is_fresh(&now.to_string()), "now is fresh");
        let edge = (now - MAX_TIMESTAMP_AGE_SECS + 5).to_string();
        assert!(timestamp_is_fresh(&edge), "just inside the window");

        let stale = (now - MAX_TIMESTAMP_AGE_SECS - 60).to_string();
        assert!(!timestamp_is_fresh(&stale), "a replay is refused");
        let future = (now + MAX_TIMESTAMP_AGE_SECS + 60).to_string();
        assert!(
            !timestamp_is_fresh(&future),
            "a far-future stamp is refused"
        );

        assert!(!timestamp_is_fresh(""), "absent");
        assert!(!timestamp_is_fresh("not-a-number"), "unparseable");
        let overlong = "9".repeat(MAX_TIMESTAMP_HEADER_LEN + 1);
        assert!(!timestamp_is_fresh(&overlong), "bounded header length");
    }

    #[test]
    fn refusal_status_distinguishes_unarmed_from_wrong_proof() {
        let status = refuse_unconfigured("Telegram", "channels.telegram.webhook_secret");
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_ne!(status, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn an_authenticated_body_parses_or_is_a_bad_request() {
        #[derive(serde::Deserialize, Debug, PartialEq)]
        struct Payload {
            id: u8,
        }

        let ok: Payload =
            parse_authenticated_body("Test", br#"{"id":7}"#).expect("a valid body parses");
        assert_eq!(ok, Payload { id: 7 });

        // 400 and not 401/503: authentication already succeeded, so the caller
        // is known and the *body* is what is wrong.
        let err = parse_authenticated_body::<Payload>("Test", b"{not json")
            .expect_err("a malformed body is refused");
        assert_eq!(err, StatusCode::BAD_REQUEST);
        assert_ne!(err, StatusCode::UNAUTHORIZED);
        assert_ne!(err, StatusCode::SERVICE_UNAVAILABLE);

        let wrong_shape = parse_authenticated_body::<Payload>("Test", br#"{"id":"seven"}"#)
            .expect_err("a well-formed body of the wrong shape is refused");
        assert_eq!(wrong_shape, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn bearer_secret_accepts_both_header_forms_and_nothing_else() {
        let mut headers = HeaderMap::new();
        assert!(!bearer_secret_ok(&headers, "s3cr3t"), "no header at all");

        headers.insert("Authorization", "Bearer s3cr3t".parse().expect("header"));
        assert!(bearer_secret_ok(&headers, "s3cr3t"), "bearer form");

        let mut headers = HeaderMap::new();
        headers.insert("X-Webhook-Secret", "s3cr3t".parse().expect("header"));
        assert!(bearer_secret_ok(&headers, "s3cr3t"), "direct header form");

        let mut headers = HeaderMap::new();
        headers.insert("Authorization", "Bearer wrong".parse().expect("header"));
        headers.insert("X-Webhook-Secret", "also-wrong".parse().expect("header"));
        assert!(!bearer_secret_ok(&headers, "s3cr3t"), "two wrong guesses");

        // A bare token without the `Bearer ` scheme is not the secret.
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", "s3cr3t".parse().expect("header"));
        assert!(!bearer_secret_ok(&headers, "s3cr3t"), "scheme is required");
    }
}
