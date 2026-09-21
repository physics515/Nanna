//! A one-shot, bounded probe of an Ollama server: is it answering, and which
//! models does it have?
//!
//! One `GET /api/tags` with a hard timeout, reading the answer in bounded
//! chunks so an unexpected server cannot make it buffer without limit.
//!
//! The bearer token is the caller's choice. The daemon's probe sends the one
//! the configured server was given (a remote, Ollama-compatible server may sit
//! behind one — mummu's shim behind a proxy, say); `nanna doctor --online`
//! sends none, by its own rule never to carry a credential, and a server that
//! wants one is then reported as exactly that.

use std::time::Duration;

/// One model an Ollama server lists, as `/api/tags` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OllamaModel {
    /// The tag Ollama lists it under (`qwen3:8b`, `nomic-embed-text:latest`).
    pub name: String,
    /// On-disk size in bytes; `0` when the server did not say (an older
    /// server, or a hand-written body). Never guessed.
    pub size_bytes: u64,
}

/// Outcome of probing one Ollama server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OllamaProbe {
    /// It answered `GET /api/tags`; these are the models it has.
    Reachable { models: Vec<OllamaModel> },
    /// Nothing usable answered; `reason` says what happened, in words.
    Unreachable { reason: String },
}

impl OllamaProbe {
    /// The names of a reachable server's models, in the order listed; empty
    /// when unreachable. The shape `doctor` and the daemon compare against.
    #[must_use]
    pub fn model_names(&self) -> Vec<&str> {
        match self {
            Self::Reachable { models } => models.iter().map(|m| m.name.as_str()).collect(),
            Self::Unreachable { .. } => Vec::new(),
        }
    }
}

/// Largest `/api/tags` answer the probe will read. A store of a few hundred
/// models answers in tens of KiB; 4 MiB is two orders of magnitude of room and
/// still a bound.
const PROBE_BODY_BYTES_MAX: usize = 4 * 1024 * 1024;

/// Most model names a probe reports.
const PROBE_MODELS_MAX: usize = 4096;

/// Probe `base_url` with one `GET /api/tags`.
///
/// `base_url` is a local server (`http://localhost:11434`) or a remote one,
/// path included (`https://host/ollama`). `bearer` is sent as
/// `Authorization: Bearer …` when given and not blank. The connect and the
/// whole request are each bounded by `timeout`.
pub async fn probe_ollama(base_url: &str, bearer: Option<&str>, timeout: Duration) -> OllamaProbe {
    debug_assert!(!timeout.is_zero(), "a probe needs time to answer");
    let url = format!("{}/api/tags", base_url.trim().trim_end_matches('/'));
    let bearer = bearer.map(str::trim).filter(|token| !token.is_empty());
    let client = match reqwest::Client::builder()
        .connect_timeout(timeout)
        .timeout(timeout)
        .build()
    {
        Ok(client) => client,
        Err(e) => {
            return OllamaProbe::Unreachable {
                reason: format!("could not build an HTTP client: {e}"),
            };
        }
    };
    let mut request = client.get(&url);
    if let Some(token) = bearer {
        request = request.bearer_auth(token);
    }
    let mut response = match request.send().await {
        Ok(response) => response,
        Err(e) => {
            return OllamaProbe::Unreachable {
                reason: describe(&e),
            };
        }
    };
    if !response.status().is_success() {
        return OllamaProbe::Unreachable {
            reason: refusal_reason(&url, response.status(), bearer.is_some()),
        };
    }
    let mut body: Vec<u8> = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) if body.len() + chunk.len() <= PROBE_BODY_BYTES_MAX => {
                body.extend_from_slice(&chunk);
            }
            Ok(Some(_)) => {
                return OllamaProbe::Unreachable {
                    reason: format!("{url} answered with more than {PROBE_BODY_BYTES_MAX} bytes"),
                };
            }
            Ok(None) => break,
            Err(e) => {
                return OllamaProbe::Unreachable {
                    reason: describe(&e),
                };
            }
        }
    }
    debug_assert!(body.len() <= PROBE_BODY_BYTES_MAX, "the read is bounded");
    match serde_json::from_slice::<serde_json::Value>(&body) {
        Ok(tags) => OllamaProbe::Reachable {
            models: listed_models(&tags),
        },
        Err(e) => OllamaProbe::Unreachable {
            reason: format!("{url} did not answer like Ollama: {e}"),
        },
    }
}

/// Every named entry under `models` in an `/api/tags` body, bounded. An
/// entry without a `name` is skipped (there is nothing to pull or pick);
/// one without a `size` is kept at `size_bytes: 0`.
fn listed_models(tags: &serde_json::Value) -> Vec<OllamaModel> {
    let models: Vec<OllamaModel> = tags
        .get("models")
        .and_then(serde_json::Value::as_array)
        .map_or_default(|models| {
            models
                .iter()
                .filter_map(|m| {
                    let name = m.get("name").and_then(serde_json::Value::as_str)?;
                    let size_bytes = m.get("size").and_then(serde_json::Value::as_u64).unwrap_or(0);
                    Some(OllamaModel {
                        name: name.to_string(),
                        size_bytes,
                    })
                })
                .take(PROBE_MODELS_MAX)
                .collect()
        });
    debug_assert!(models.len() <= PROBE_MODELS_MAX, "the list is bounded");
    models
}

/// A non-success answer in words a person can act on.
///
/// The two worth naming are the ones a remote server produces: a token it
/// wants (or refuses), and no Ollama API at that address at all — an
/// Ollama-compatible server often lives under a path prefix, so the base URL
/// has to include it (the probe appends `/api/tags` itself).
fn refusal_reason(url: &str, status: reqwest::StatusCode, sent_token: bool) -> String {
    match status.as_u16() {
        401 | 403 if sent_token => {
            format!("{url} refused the bearer token (HTTP {status}) — check the token")
        }
        401 | 403 => format!("{url} answered HTTP {status} — this server needs a bearer token"),
        404 => format!(
            "{url} answered HTTP 404 — nothing Ollama-compatible at that address. If the server \
             serves Ollama under a path, include it in the URL (e.g. https://host/ollama)"
        ),
        _ => format!("{url} answered HTTP {status}"),
    }
}

/// A transport failure in words a person can act on.
fn describe(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "timed out".to_string()
    } else if error.is_connect() {
        "connection refused or host unreachable".to_string()
    } else {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_come_from_the_models_array_with_their_sizes() {
        let tags = serde_json::json!({ "models": [
            { "name": "qwen3:8b", "size": 5_200_000_000_u64 },
            { "name": "nomic-embed-text:latest" },
            { "size": 3 }
        ]});
        assert_eq!(
            listed_models(&tags),
            vec![
                OllamaModel { name: "qwen3:8b".to_string(), size_bytes: 5_200_000_000 },
                // No size reported: kept, at zero — never invented.
                OllamaModel { name: "nomic-embed-text:latest".to_string(), size_bytes: 0 },
            ]
        );
        let probe = OllamaProbe::Reachable { models: listed_models(&tags) };
        assert_eq!(probe.model_names(), vec!["qwen3:8b", "nomic-embed-text:latest"]);
    }

    #[test]
    fn a_body_without_models_yields_none() {
        let none: Vec<OllamaModel> = Vec::new();
        assert_eq!(listed_models(&serde_json::json!({})), none);
        assert_eq!(listed_models(&serde_json::json!({ "models": "nope" })), none);
        let down = OllamaProbe::Unreachable { reason: "x".to_string() };
        assert_eq!(down.model_names(), [] as [&str; 0]);
    }

    /// A one-connection HTTP server on a free loopback port that answers its
    /// first request with `status` and `body`; returns its base URL.
    async fn serve_once(status: &'static str, body: &'static str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = [0_u8; 2048];
            let _ = socket.read(&mut request).await;
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
        });
        format!("http://{addr}")
    }

    /// A one-connection server that answers `200` with one model only when the
    /// request carries `Authorization: Bearer <token>`, `401` otherwise — the
    /// shape of an Ollama-compatible server behind an authenticating proxy.
    ///
    /// Matched the way such a server matches: the header name and the
    /// `Bearer` scheme case-insensitively (RFC 9110), the token byte for byte.
    async fn serve_once_requiring(token: &'static str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = [0_u8; 4096];
            let n = socket.read(&mut request).await.unwrap_or(0);
            let head = String::from_utf8_lossy(&request[..n]);
            let authorized = head.lines().any(|line| {
                line.split_once(':').is_some_and(|(name, value)| {
                    name.trim().eq_ignore_ascii_case("authorization")
                        && value
                            .trim()
                            .split_once(' ')
                            .is_some_and(|(scheme, credential)| {
                                scheme.eq_ignore_ascii_case("bearer") && credential.trim() == token
                            })
                })
            });
            let (status, body) = if authorized {
                ("200 OK", r#"{"models":[{"name":"qwen2.5-1.5b-instruct","size":7}]}"#)
            } else {
                ("401 Unauthorized", r#"{"error":"unauthorized"}"#)
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn a_bearer_token_opens_a_protected_server() {
        let url = serve_once_requiring("s3cret").await;
        let probe = probe_ollama(&url, Some("  s3cret "), Duration::from_secs(5)).await;
        assert_eq!(probe.model_names(), vec!["qwen2.5-1.5b-instruct"], "{probe:?}");
    }

    #[tokio::test]
    async fn a_protected_server_says_it_wants_a_token_or_refused_one() {
        let url = serve_once_requiring("s3cret").await;
        let probe = probe_ollama(&url, None, Duration::from_secs(5)).await;
        assert!(
            matches!(&probe, OllamaProbe::Unreachable { reason } if reason.contains("needs a bearer token")),
            "{probe:?}"
        );
        // A blank token is no token: it must not be sent as `Bearer `.
        let url = serve_once_requiring("s3cret").await;
        let probe = probe_ollama(&url, Some("   "), Duration::from_secs(5)).await;
        assert!(
            matches!(&probe, OllamaProbe::Unreachable { reason } if reason.contains("needs a bearer token")),
            "{probe:?}"
        );
        let url = serve_once_requiring("s3cret").await;
        let probe = probe_ollama(&url, Some("wrong"), Duration::from_secs(5)).await;
        assert!(
            matches!(&probe, OllamaProbe::Unreachable { reason } if reason.contains("refused the bearer token")),
            "{probe:?}"
        );
    }

    /// A bearer token is an opaque secret, so a token that differs only in
    /// case is a different token. The test server must refuse it, or every
    /// token test here would pass for a probe that mangled the token's case.
    #[tokio::test]
    async fn a_token_that_differs_only_in_case_is_refused() {
        let url = serve_once_requiring("s3cret").await;
        let probe = probe_ollama(&url, Some("S3CRET"), Duration::from_secs(5)).await;
        assert!(
            matches!(&probe, OllamaProbe::Unreachable { reason } if reason.contains("refused the bearer token")),
            "{probe:?}"
        );
    }

    #[tokio::test]
    async fn a_404_says_to_include_the_servers_path() {
        let url = serve_once("404 Not Found", r#"{"error":"not found"}"#).await;
        let probe = probe_ollama(&url, None, Duration::from_secs(5)).await;
        assert!(
            matches!(&probe, OllamaProbe::Unreachable { reason } if reason.contains("include it in the URL")),
            "{probe:?}"
        );
    }

    #[tokio::test]
    async fn a_reachable_server_reports_its_models() {
        let url = serve_once(
            "200 OK",
            r#"{"models":[{"name":"qwen3:8b","size":42},{"name":"bge-m3:latest"}]}"#,
        )
        .await;
        let probe = probe_ollama(&format!("{url}/"), None, Duration::from_secs(5)).await;
        assert_eq!(
            probe,
            OllamaProbe::Reachable {
                models: vec![
                    OllamaModel { name: "qwen3:8b".to_string(), size_bytes: 42 },
                    OllamaModel { name: "bge-m3:latest".to_string(), size_bytes: 0 },
                ]
            }
        );
    }

    #[tokio::test]
    async fn an_error_status_or_a_foreign_body_is_not_ollama() {
        let url = serve_once("404 Not Found", "{}").await;
        let probe = probe_ollama(&url, None, Duration::from_secs(5)).await;
        assert!(
            matches!(&probe, OllamaProbe::Unreachable { reason } if reason.contains("404")),
            "{probe:?}"
        );
        let url = serve_once("200 OK", "<html>a router login page</html>").await;
        let probe = probe_ollama(&url, None, Duration::from_secs(5)).await;
        assert!(
            matches!(&probe, OllamaProbe::Unreachable { reason } if reason.contains("like Ollama")),
            "{probe:?}"
        );
    }

    /// Nothing listens on the port: the probe says so, and does not panic or hang.
    #[tokio::test]
    async fn an_unreachable_server_is_reported_not_raised() {
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
            listener.local_addr().expect("addr").port()
        };
        let probe = probe_ollama(&format!("http://127.0.0.1:{port}"), None, Duration::from_secs(2)).await;
        assert!(
            matches!(probe, OllamaProbe::Unreachable { .. }),
            "{probe:?}"
        );
    }
}
