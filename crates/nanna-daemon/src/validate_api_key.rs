//! Live validation of a provider API key the user just typed.
//!
//! One minimal, cheap, read-only request per provider — a `GET` of the
//! model catalogue (or, for `OpenRouter`, the key's own info endpoint) — sent
//! with the key **as supplied by the caller**. This module never reads the
//! keyring and never persists anything: it answers the single question
//! "does this string open the door?", so onboarding and Settings can say so
//! inline instead of letting the first real chat turn discover a typo.
//!
//! The verdict is deliberately three-valued. "Invalid" and "unreachable"
//! must not collapse into one error string: the first means *retype the
//! key*, the second means *the key may be fine — the network is not*, and
//! telling a user on a captive-portal Wi-Fi to re-enter a correct key is the
//! exact wrong advice.
//!
//! The classification is pure ([`classify_status`], [`classify_transport`])
//! so every verdict rule is unit-tested without a socket; the network is
//! confined to [`validate_api_key`].

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The providers whose keys can be checked. Ollama is absent on purpose —
/// it is keyless, and its liveness is `system.probe_ollama`'s question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyProvider {
    Anthropic,
    OpenAi,
    OpenRouter,
    GitHubModels,
}

impl KeyProvider {
    /// Parse the provider name as the config and the GUI spell it
    /// (`anthropic` / `openai` / `openrouter` / `github`), case-insensitive.
    /// `None` for anything else — including `ollama`, which has no key.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "anthropic" | "claude" => Some(Self::Anthropic),
            "openai" => Some(Self::OpenAi),
            "openrouter" => Some(Self::OpenRouter),
            "github" | "github_models" | "github-models" | "githubmodels" => {
                Some(Self::GitHubModels)
            }
            _ => None,
        }
    }

    /// The name a reply carries back, matching the config spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::OpenRouter => "openrouter",
            Self::GitHubModels => "github",
        }
    }

    /// The cheapest authenticated, read-only endpoint the provider offers.
    /// Same hosts `nanna-llm` talks to, so a key that validates here is a
    /// key the router will accept.
    #[must_use]
    pub const fn probe_url(self) -> &'static str {
        match self {
            Self::Anthropic => "https://api.anthropic.com/v1/models?limit=1",
            Self::OpenAi => "https://api.openai.com/v1/models",
            Self::OpenRouter => "https://openrouter.ai/api/v1/auth/key",
            Self::GitHubModels => "https://models.inference.ai.azure.com/models",
        }
    }
}

/// The answer, on the wire as `{ "verdict": "valid" | "invalid" | "unreachable", "reason"? }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum KeyVerdict {
    /// The provider accepted the key.
    Valid,
    /// The provider answered and refused the key — retype it.
    Invalid { reason: String },
    /// No usable answer: DNS, connect, TLS, timeout, or a provider-side
    /// failure. Says nothing about the key.
    Unreachable { reason: String },
}

impl KeyVerdict {
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        matches!(self, Self::Valid)
    }
}

/// How long one validation may take, connect and answer together. The
/// catalogue endpoints answer in well under a second; 8 s covers a slow
/// first TLS handshake without letting a black-holed host pin the
/// onboarding wizard.
pub const VALIDATE_TIMEOUT: Duration = Duration::from_secs(8);

/// Bound on the error body kept for the reason. A refusal is one sentence;
/// anything longer is an HTML login page, and 512 bytes of it is enough to
/// recognise.
const REASON_BODY_MAX: usize = 512;

/// Classify an HTTP answer. Pure — the whole verdict table lives here.
///
/// * `2xx` → valid.
/// * `401` / `403` → invalid: the provider read the credential and said no.
/// * `429` → valid: a rate limit or quota notice is only ever raised for an
///   **authenticated** key — an unknown key gets 401 first. Reporting it as
///   invalid would send the user to retype a key that works.
/// * `5xx` → unreachable: the provider is failing, the key is undecided.
/// * any other status → unreachable with the status named: an unexpected
///   answer is not evidence about the key either way, and guessing "invalid"
///   from, say, a 404 caused by an endpoint move would be a lie.
#[must_use]
pub fn classify_status(status: u16, body: &str) -> KeyVerdict {
    let detail = body_detail(body);
    match status {
        200..=299 => KeyVerdict::Valid,
        401 | 403 => KeyVerdict::Invalid {
            reason: with_detail(
                format!("the provider rejected this key (HTTP {status})"),
                &detail,
            ),
        },
        429 => KeyVerdict::Valid,
        500..=599 => KeyVerdict::Unreachable {
            reason: with_detail(
                format!("the provider is having trouble (HTTP {status}); the key was not checked"),
                &detail,
            ),
        },
        other => KeyVerdict::Unreachable {
            reason: with_detail(
                format!("unexpected answer from the provider (HTTP {other}); the key was not checked"),
                &detail,
            ),
        },
    }
}

/// Classify a transport failure — nothing came back at all. Always
/// unreachable: no HTTP status means the provider never judged the key.
#[must_use]
pub fn classify_transport(error: &reqwest::Error) -> KeyVerdict {
    let what = if error.is_timeout() {
        "timed out waiting for the provider"
    } else if error.is_connect() {
        "could not connect to the provider"
    } else if error.is_request() {
        "could not send the request to the provider"
    } else {
        "no usable answer from the provider"
    };
    // `reqwest::Error`'s Display carries the URL, which is fine (no secret is
    // in it — the key travels in a header), and the underlying cause, which
    // is what an operator needs ("dns error", "certificate verify failed").
    KeyVerdict::Unreachable {
        reason: format!("{what} — {}; the key was not checked", scrub_error(error)),
    }
}

/// The reason text an error body contributes, if any: the provider's own
/// `error.message` when the body is the JSON shape all four use, else a
/// bounded, single-line excerpt, else nothing.
fn body_detail(body: &str) -> Option<String> {
    let body = body.trim();
    if body.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
        let message = value
            .pointer("/error/message")
            .or_else(|| value.pointer("/message"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|message| !message.is_empty());
        if let Some(message) = message {
            return Some(truncate_chars(message, REASON_BODY_MAX));
        }
    }
    let one_line = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.starts_with('<') {
        // An HTML page is never a provider's answer about a key; it is a
        // proxy or a captive portal speaking instead of the provider.
        return Some("the answer was an HTML page, not the provider's API".to_string());
    }
    Some(truncate_chars(&one_line, REASON_BODY_MAX))
}

fn with_detail(base: String, detail: &Option<String>) -> String {
    match detail {
        Some(detail) => format!("{base}: {detail}"),
        None => base,
    }
}

/// Cut on a char boundary, never a byte offset — a provider message can
/// carry any script, and byte-slicing is how this codebase has panicked
/// before.
fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(max_chars).collect();
    cut.push('…');
    cut
}

/// The transport error's text, on one line.
fn scrub_error(error: &reqwest::Error) -> String {
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    truncate_chars(
        &text.split_whitespace().collect::<Vec<_>>().join(" "),
        REASON_BODY_MAX,
    )
}

/// Send the one probe request for `provider` carrying `key`, and classify
/// the answer. The key is used exactly once, in the request header, and is
/// never logged.
///
/// A blank key is refused before any request: the provider would only say
/// 401 and the caller would have paid a round-trip to learn that an empty
/// string is not a credential.
pub async fn validate_api_key(provider: KeyProvider, key: &str, timeout: Duration) -> KeyVerdict {
    let key = key.trim();
    if key.is_empty() {
        return KeyVerdict::Invalid {
            reason: "no key was entered".to_string(),
        };
    }
    let client = match reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout)
        .user_agent(concat!("nanna/", env!("CARGO_PKG_VERSION")))
        .build()
    {
        Ok(client) => client,
        Err(error) => return classify_transport(&error),
    };

    let mut request = client.get(provider.probe_url());
    request = match provider {
        KeyProvider::Anthropic => request
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01"),
        KeyProvider::OpenAi | KeyProvider::OpenRouter | KeyProvider::GitHubModels => {
            request.bearer_auth(key)
        }
    };

    let response = match request.send().await {
        Ok(response) => response,
        Err(error) => return classify_transport(&error),
    };
    let status = response.status().as_u16();
    let body = if (200..=299).contains(&status) {
        // A success body is a model catalogue we do not need; do not read it.
        String::new()
    } else {
        read_bounded_body(response).await
    };
    classify_status(status, &body)
}

/// Read at most `REASON_BODY_MAX` bytes of the body — the reason wants one
/// sentence, and an error page has no business filling memory.
async fn read_bounded_body(mut response: reqwest::Response) -> String {
    let mut collected: Vec<u8> = Vec::with_capacity(REASON_BODY_MAX);
    while let Ok(Some(chunk)) = response.chunk().await {
        let room = REASON_BODY_MAX.saturating_sub(collected.len());
        if room == 0 {
            break;
        }
        collected.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if collected.len() >= REASON_BODY_MAX {
            break;
        }
    }
    String::from_utf8_lossy(&collected).into_owned()
}

/// The `system.validate_api_key` handler body: parse the provider name a
/// client sent, run the one probe under the default timeout, and shape the
/// wire answer. A provider name this cannot check (`ollama` — keyless — or a
/// typo) is answered as an `unreachable` verdict naming the accepted names,
/// not as a network attempt: there is no endpoint to ask, and the caller
/// needs to know the request itself was wrong.
///
/// Answer shape: `{ "provider", "verdict": "valid"|"invalid"|"unreachable",
/// "reason"?, "valid": bool }`. `valid` is redundant with `verdict` on
/// purpose — a client that only needs the boolean should not have to know
/// the enum.
pub async fn handle_validate_api_key(provider_name: &str, key: &str) -> serde_json::Value {
    let Some(provider) = KeyProvider::parse(provider_name) else {
        return verdict_report(
            provider_name.trim(),
            &KeyVerdict::Unreachable {
                reason: format!(
                    "no key check exists for provider {:?}; accepted: anthropic, openai, \
                     openrouter, github (Ollama needs no key)",
                    provider_name.trim()
                ),
            },
        );
    };
    let verdict = validate_api_key(provider, key, VALIDATE_TIMEOUT).await;
    verdict_report(provider.as_str(), &verdict)
}

/// The wire shape of one verdict. Pure, so the envelope is pinned by test.
#[must_use]
pub fn verdict_report(provider: &str, verdict: &KeyVerdict) -> serde_json::Value {
    let mut report = serde_json::to_value(verdict).unwrap_or_else(|_| serde_json::json!({}));
    if let serde_json::Value::Object(map) = &mut report {
        map.insert("provider".to_string(), serde_json::Value::from(provider));
        map.insert("valid".to_string(), serde_json::Value::from(verdict.is_valid()));
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_2xx_answer_is_valid() {
        assert_eq!(classify_status(200, r#"{"data":[]}"#), KeyVerdict::Valid);
        assert_eq!(classify_status(204, ""), KeyVerdict::Valid);
    }

    #[test]
    fn a_401_or_403_is_invalid_and_carries_the_providers_own_words() {
        let anthropic = r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#;
        match classify_status(401, anthropic) {
            KeyVerdict::Invalid { reason } => {
                assert!(reason.contains("401"), "{reason}");
                assert!(reason.contains("invalid x-api-key"), "{reason}");
            }
            other => panic!("expected invalid, got {other:?}"),
        }
        let openai = r#"{"error":{"message":"Incorrect API key provided: sk-abc.","type":"invalid_request_error"}}"#;
        match classify_status(403, openai) {
            KeyVerdict::Invalid { reason } => {
                assert!(reason.contains("Incorrect API key"), "{reason}");
            }
            other => panic!("expected invalid, got {other:?}"),
        }
    }

    /// A rate limit is raised only for a key the provider recognised; an
    /// unknown key gets 401 before quota is ever consulted.
    #[test]
    fn a_429_means_the_key_was_accepted() {
        assert_eq!(
            classify_status(429, r#"{"error":{"message":"Rate limit reached"}}"#),
            KeyVerdict::Valid
        );
    }

    #[test]
    fn a_5xx_is_unreachable_not_invalid() {
        match classify_status(503, "<html>upstream down</html>") {
            KeyVerdict::Unreachable { reason } => {
                assert!(reason.contains("503"), "{reason}");
                assert!(reason.contains("not checked"), "{reason}");
                assert!(reason.contains("HTML page"), "{reason}");
            }
            other => panic!("expected unreachable, got {other:?}"),
        }
    }

    /// An unexpected 4xx (an endpoint moved, a proxy in the way) is not
    /// evidence about the key, so it must not be reported as invalid.
    #[test]
    fn an_unexpected_status_is_unreachable_and_named() {
        match classify_status(404, "") {
            KeyVerdict::Unreachable { reason } => assert!(reason.contains("404"), "{reason}"),
            other => panic!("expected unreachable, got {other:?}"),
        }
    }

    #[test]
    fn a_network_error_is_unreachable_with_its_cause() {
        // A request that cannot be built is the one transport failure
        // constructible without a socket.
        let error = reqwest::Client::new()
            .get("http://[::1]:99999/")
            .build()
            .expect_err("an out-of-range port cannot build");
        match classify_transport(&error) {
            KeyVerdict::Unreachable { reason } => {
                assert!(reason.contains("not checked"), "{reason}");
            }
            other => panic!("expected unreachable, got {other:?}"),
        }
    }

    /// A blank key is refused before any request; the runtime is only here
    /// because the function is async, no socket is opened.
    #[tokio::test]
    async fn a_blank_key_is_invalid_without_a_request() {
        for key in ["", "   ", "\t\n"] {
            match validate_api_key(KeyProvider::OpenAi, key, Duration::from_millis(1)).await {
                KeyVerdict::Invalid { reason } => assert!(reason.contains("no key"), "{reason}"),
                other => panic!("expected invalid for {key:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn provider_names_parse_as_the_config_spells_them() {
        assert_eq!(KeyProvider::parse("anthropic"), Some(KeyProvider::Anthropic));
        assert_eq!(KeyProvider::parse(" OpenAI "), Some(KeyProvider::OpenAi));
        assert_eq!(KeyProvider::parse("openrouter"), Some(KeyProvider::OpenRouter));
        assert_eq!(KeyProvider::parse("github"), Some(KeyProvider::GitHubModels));
        assert_eq!(KeyProvider::parse("ollama"), None, "Ollama has no key to check");
        assert_eq!(KeyProvider::parse(""), None);
    }

    #[test]
    fn the_verdict_serializes_as_the_documented_envelope() {
        let invalid = KeyVerdict::Invalid {
            reason: "nope".to_string(),
        };
        let json = serde_json::to_value(&invalid).unwrap();
        assert_eq!(json["verdict"], "invalid");
        assert_eq!(json["reason"], "nope");
        assert_eq!(serde_json::to_value(KeyVerdict::Valid).unwrap()["verdict"], "valid");
    }

    #[test]
    fn body_detail_never_slices_inside_a_char() {
        let long = "é".repeat(REASON_BODY_MAX + 40);
        let detail = body_detail(&long).expect("non-empty body");
        assert!(detail.ends_with('…'));
        assert_eq!(detail.chars().count(), REASON_BODY_MAX + 1);
    }

    /// The handler answers an unknown or keyless provider without opening a
    /// socket, and says which names it does accept.
    #[tokio::test]
    async fn the_handler_refuses_a_provider_it_cannot_check() {
        for name in ["ollama", "gemini", ""] {
            let report = handle_validate_api_key(name, "sk-anything").await;
            assert_eq!(report["verdict"], "unreachable", "{name:?}: {report}");
            assert_eq!(report["valid"], false);
            let reason = report["reason"].as_str().unwrap_or_default();
            assert!(reason.contains("anthropic"), "{reason}");
        }
    }

    #[test]
    fn the_report_carries_provider_verdict_and_the_boolean() {
        let report = verdict_report("openai", &KeyVerdict::Valid);
        assert_eq!(report["provider"], "openai");
        assert_eq!(report["verdict"], "valid");
        assert_eq!(report["valid"], true);
        assert!(report.get("reason").is_none(), "a valid verdict has no reason");

        let refused = verdict_report(
            "anthropic",
            &KeyVerdict::Invalid {
                reason: "no".to_string(),
            },
        );
        assert_eq!(refused["valid"], false);
        assert_eq!(refused["reason"], "no");
    }
}
