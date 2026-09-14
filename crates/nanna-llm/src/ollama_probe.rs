//! A one-shot, bounded probe of an Ollama server: is it answering, and which
//! models does it have?
//!
//! For `nanna doctor --online`, which must never hang and must never carry a
//! credential: the probe is one unauthenticated `GET /api/tags` with a hard
//! timeout, and it reads the answer in bounded chunks so an unexpected server
//! cannot make it buffer without limit.

use std::time::Duration;

/// Outcome of probing one Ollama server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OllamaProbe {
    /// It answered `GET /api/tags`; these are the model names it has.
    Reachable { models: Vec<String> },
    /// Nothing usable answered; `reason` says what happened, in words.
    Unreachable { reason: String },
}

/// Largest `/api/tags` answer the probe will read. A store of a few hundred
/// models answers in tens of KiB; 4 MiB is two orders of magnitude of room and
/// still a bound.
const PROBE_BODY_BYTES_MAX: usize = 4 * 1024 * 1024;

/// Most model names a probe reports.
const PROBE_MODELS_MAX: usize = 4096;

/// Probe `base_url` (e.g. `http://localhost:11434`) with one `GET /api/tags`,
/// the connect and the whole request each bounded by `timeout`.
pub async fn probe_ollama(base_url: &str, timeout: Duration) -> OllamaProbe {
    debug_assert!(!timeout.is_zero(), "a probe needs time to answer");
    let url = format!("{}/api/tags", base_url.trim().trim_end_matches('/'));
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
    let mut response = match client.get(&url).send().await {
        Ok(response) => response,
        Err(e) => {
            return OllamaProbe::Unreachable {
                reason: describe(&e),
            };
        }
    };
    if !response.status().is_success() {
        return OllamaProbe::Unreachable {
            reason: format!("{url} answered HTTP {}", response.status()),
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
            models: model_names(&tags),
        },
        Err(e) => OllamaProbe::Unreachable {
            reason: format!("{url} did not answer like Ollama: {e}"),
        },
    }
}

/// The `name` of every entry under `models` in an `/api/tags` body, bounded.
fn model_names(tags: &serde_json::Value) -> Vec<String> {
    let names: Vec<String> = tags
        .get("models")
        .and_then(serde_json::Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|m| m.get("name").and_then(serde_json::Value::as_str))
                .take(PROBE_MODELS_MAX)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    debug_assert!(names.len() <= PROBE_MODELS_MAX, "the list is bounded");
    names
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
    fn model_names_come_from_the_models_array() {
        let tags = serde_json::json!({ "models": [
            { "name": "qwen3:8b", "size": 1 },
            { "name": "nomic-embed-text:latest" },
            { "size": 3 }
        ]});
        assert_eq!(
            model_names(&tags),
            vec!["qwen3:8b", "nomic-embed-text:latest"]
        );
    }

    #[test]
    fn a_body_without_models_yields_none() {
        let none: Vec<String> = Vec::new();
        assert_eq!(model_names(&serde_json::json!({})), none);
        assert_eq!(model_names(&serde_json::json!({ "models": "nope" })), none);
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

    #[tokio::test]
    async fn a_reachable_server_reports_its_models() {
        let url = serve_once(
            "200 OK",
            r#"{"models":[{"name":"qwen3:8b"},{"name":"bge-m3:latest"}]}"#,
        )
        .await;
        let probe = probe_ollama(&format!("{url}/"), Duration::from_secs(5)).await;
        assert_eq!(
            probe,
            OllamaProbe::Reachable {
                models: vec!["qwen3:8b".to_string(), "bge-m3:latest".to_string()]
            }
        );
    }

    #[tokio::test]
    async fn an_error_status_or_a_foreign_body_is_not_ollama() {
        let url = serve_once("404 Not Found", "{}").await;
        let probe = probe_ollama(&url, Duration::from_secs(5)).await;
        assert!(
            matches!(&probe, OllamaProbe::Unreachable { reason } if reason.contains("404")),
            "{probe:?}"
        );
        let url = serve_once("200 OK", "<html>a router login page</html>").await;
        let probe = probe_ollama(&url, Duration::from_secs(5)).await;
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
        let probe = probe_ollama(&format!("http://127.0.0.1:{port}"), Duration::from_secs(2)).await;
        assert!(
            matches!(probe, OllamaProbe::Unreachable { .. }),
            "{probe:?}"
        );
    }
}
