//! What a config change may change on the embedding clients a running daemon
//! already holds, and what has to wait for a restart.
//!
//! The embedding router is built once, at boot, from [`EmbeddingConfig`], and
//! the memory store is bound to the `provider:model` it produced and to that
//! model's width. A config change — Settings saving through `config.reload`, a
//! `config.set`, a hand edit the config watcher picks up — splits in two:
//!
//! - **The Ollama token follows at once.** It is a credential, not part of the
//!   binding: the same server's same model answers with the same vectors
//!   whichever token opens the door. Every Ollama embedding client reads it
//!   from one [`SharedBearer`] per request, and the router lifts a bench it
//!   set under the old token.
//! - **The server address and the model list wait for a restart.** The store's
//!   binding is keyed by `provider:model` alone, not by server, so another
//!   server's model of the same name would write into the same bucket at
//!   whatever width that server gives it, and nothing would re-check: the
//!   width is learned at boot (`probe_or_seed_dimension`, then
//!   `probe_and_align_dimension`) and re-bound on a router failover, never on
//!   a config change. Following either live is the 2026-08-02 split-brain (model
//!   rebound, width latch stale, every write failing); a restart is the path
//!   that re-probes. What waits is logged once per change, because nothing
//!   else says so — chat follows a new address at once, so the embedder
//!   staying behind is otherwise invisible.

use std::sync::{Mutex, PoisonError};

use nanna_llm::SharedBearer;
use tracing::{info, warn};

use crate::server::EmbeddingConfig;

/// The embedding settings the running router was built from, and the one of
/// them a config change may update in place: the Ollama token.
pub struct LiveEmbeddingSettings {
    /// What the router was built from — the server and the model list every
    /// later config is compared against.
    boot: EmbeddingConfig,
    /// The Ollama token, shared by every Ollama embedding client the router
    /// holds and read by them per request.
    ollama_bearer: SharedBearer,
    /// What the last applied config left waiting for a restart. A change in it
    /// is logged once; a save that changes nothing about it logs nothing.
    waiting: Mutex<Vec<String>>,
}

impl LiveEmbeddingSettings {
    /// The settings of a router built from `boot`.
    #[must_use]
    pub fn new(boot: &EmbeddingConfig) -> Self {
        Self {
            boot: boot.clone(),
            ollama_bearer: SharedBearer::new(boot.ollama_api_key.as_deref().unwrap_or_default()),
            waiting: Mutex::new(Vec::new()),
        }
    }

    /// The token handle every Ollama embedding client is built with.
    #[must_use]
    pub fn ollama_bearer(&self) -> SharedBearer {
        self.ollama_bearer.clone()
    }

    /// Bring the running embedding clients in line with `config` as far as is
    /// safe without a restart, and log what waits for one.
    pub fn apply(&self, config: &nanna_config::Config) {
        let now = EmbeddingConfig::from_nanna(config);

        // The token goes only to the server it is configured for. While a new
        // address waits for a restart, a token saved beside it is for a
        // server these clients are not talking to, so it waits too.
        let same_server =
            nanna_config::same_ollama_server(&now.ollama_host, &self.boot.ollama_host);
        if same_server
            && self
                .ollama_bearer
                .replace(now.ollama_api_key.as_deref().unwrap_or_default())
        {
            info!(
                "Ollama embedding token updated from the config; the next embed request sends it"
            );
        }

        let waiting = waiting_for_restart(&self.boot, &now);
        let announced = std::mem::replace(
            &mut *self.waiting.lock().unwrap_or_else(PoisonError::into_inner),
            waiting.clone(),
        );
        if announced == waiting {
            return;
        }
        if waiting.is_empty() {
            info!(
                "Embedding settings match the running embedder again; nothing waits for a restart"
            );
        } else {
            warn!(
                "Memory embeddings keep {} until the daemon restarts. The memory store is \
                 bound to the embedding model it started with, and another server or model \
                 can answer at a different width, which only a restart re-checks.",
                waiting.join(", and ")
            );
        }
    }
}

/// What differs between the embedder the router was built from and `now`, and
/// so waits for a restart: one phrase per setting, empty when nothing does.
///
/// Spelling is not a change: the same server with a trailing slash, or
/// `nomic-embed-text` for `ollama/nomic-embed-text`, compares equal. Nor is a
/// new Ollama address, for a router that holds no Ollama model to send there.
fn waiting_for_restart(boot: &EmbeddingConfig, now: &EmbeddingConfig) -> Vec<String> {
    let resolved = |config: &EmbeddingConfig| -> Vec<Option<(String, String)>> {
        config
            .specs()
            .iter()
            .map(|spec| crate::server::split_embedding_spec(spec))
            .collect()
    };
    let (boot_models, now_models) = (resolved(boot), resolved(now));
    let embeds_with_ollama = boot_models
        .iter()
        .flatten()
        .any(|(provider, _)| provider == "ollama");

    let mut waiting = Vec::new();
    if embeds_with_ollama
        && !nanna_config::same_ollama_server(&now.ollama_host, &boot.ollama_host)
    {
        waiting.push(format!(
            "the Ollama server {} (with its token) instead of {}",
            boot.ollama_host, now.ollama_host
        ));
    }
    if boot_models != now_models {
        waiting.push(format!(
            "the embedding models [{}] instead of [{}]",
            boot.specs().join(", "),
            now.specs().join(", ")
        ));
    }
    waiting
}

#[cfg(test)]
pub(crate) mod test_ollama {
    //! A stand-in for an Ollama-compatible server behind an authenticating
    //! proxy, shared by the tests that follow a token from the config to the
    //! wire.

    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// One request as the server saw it.
    #[derive(Debug, Clone)]
    pub struct SeenRequest {
        pub path: String,
        /// The `Authorization` header, `None` when the request carried none.
        pub authorization: Option<String>,
        /// The JSON body's `model`, `None` when it named none.
        pub model: Option<String>,
    }

    /// The chat router's view of one Ollama server: its address and the
    /// token bound to it, and no other provider.
    pub fn ollama_credentials(host: &str, token: &str) -> crate::llm_router::ProviderCredentials {
        crate::llm_router::ProviderCredentials {
            anthropic: None,
            anthropic_absent_reason: None,
            openai_api_key: None,
            openrouter_api_key: None,
            github_token: None,
            ollama_host: host.to_string(),
            ollama_api_key: Some(token.to_string()),
        }
    }

    /// What `/api/chat` answers with the token: one memory, in the JSON shape
    /// memory extraction asks for, so a summarizer's call can be followed
    /// end to end through a real agent.
    pub const CHAT_MEMORY: &str = "The user keeps the build green";

    /// Serve every request on an ephemeral port, recording each one: `401`
    /// unless it carries `Authorization: Bearer <token>`.
    ///
    /// With the token, one three-wide embedding in whichever shape the path
    /// asks for (Ollama's `/api/embed` and legacy `/api/embeddings`, or
    /// `/v1/embeddings`), and on `/api/chat` an assistant reply carrying
    /// [`CHAT_MEMORY`]. `Connection: close` makes each request its own
    /// connection.
    pub async fn spawn_token_gated(token: &'static str) -> (String, Arc<Mutex<Vec<SeenRequest>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind an ephemeral port");
        let addr = listener.local_addr().expect("read back the bound addr");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        let memories = serde_json::json!([{ "content": CHAT_MEMORY, "category": "fact" }]);
        let chat = serde_json::json!({
            "message": { "role": "assistant", "content": memories.to_string() },
            "done": true,
            "done_reason": "stop",
        })
        .to_string();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let request = read_request(&mut stream).await;
                let authorized =
                    request.authorization.as_deref() == Some(format!("Bearer {token}").as_str());
                let (status, body) = match request.path.as_str() {
                    _ if !authorized => ("401 Unauthorized", r#"{"error":"unauthorized"}"#),
                    "/api/embed" => ("200 OK", r#"{"embeddings":[[0.1,0.2,0.3]]}"#),
                    "/api/embeddings" => ("200 OK", r#"{"embedding":[0.1,0.2,0.3]}"#),
                    "/v1/embeddings" => ("200 OK", r#"{"data":[{"embedding":[0.1,0.2,0.3]}]}"#),
                    "/api/chat" => ("200 OK", chat.as_str()),
                    _ => ("404 Not Found", r#"{"error":"not found"}"#),
                };
                record.lock().expect("record lock").push(request);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len(),
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.flush().await;
            }
        });
        (format!("http://{addr}"), seen)
    }

    /// Serve `/api/chat` on an ephemeral port with no authentication, each
    /// model answering the reply `answers` pairs with it — an empty reply
    /// included, the shape of a runner that stops at once — and recording
    /// every request. A model not listed, and every other path, gets `404`.
    pub async fn spawn_answering(
        answers: &'static [(&'static str, &'static str)],
    ) -> (String, Arc<Mutex<Vec<SeenRequest>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind an ephemeral port");
        let addr = listener.local_addr().expect("read back the bound addr");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let request = read_request(&mut stream).await;
                let answer = answers
                    .iter()
                    .find(|(model, _)| request.model.as_deref() == Some(*model))
                    .map(|(_, reply)| *reply)
                    .filter(|_| request.path == "/api/chat");
                let (status, body) = answer.map_or_else(
                    || ("404 Not Found", r#"{"error":"not found"}"#.to_string()),
                    |reply| {
                        let body = serde_json::json!({
                            "message": { "role": "assistant", "content": reply },
                            "done": true,
                            "done_reason": "stop",
                        });
                        ("200 OK", body.to_string())
                    },
                );
                record.lock().expect("record lock").push(request);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len(),
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.flush().await;
            }
        });
        (format!("http://{addr}"), seen)
    }

    /// Read one whole request — head, then as much body as it declares.
    async fn read_request(stream: &mut tokio::net::TcpStream) -> SeenRequest {
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        let head_end = loop {
            if let Some(at) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                break at + 4;
            }
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => break bytes.len(),
                Ok(n) => bytes.extend_from_slice(&chunk[..n]),
            }
        };
        let head = String::from_utf8_lossy(&bytes[..head_end]).to_string();
        let header = |wanted: &str| {
            head.lines().skip(1).find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.trim()
                    .eq_ignore_ascii_case(wanted)
                    .then(|| value.trim().to_string())
            })
        };
        let length: usize = header("content-length")
            .and_then(|len| len.parse().ok())
            .unwrap_or(0);
        while bytes.len() < head_end + length {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => bytes.extend_from_slice(&chunk[..n]),
            }
        }
        let body = bytes.get(head_end..).unwrap_or_default();
        let model = serde_json::from_slice::<serde_json::Value>(body)
            .ok()
            .and_then(|json| json.get("model")?.as_str().map(str::to_string));
        SeenRequest {
            path: head
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_string(),
            authorization: header("authorization"),
            model,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boot(host: &str, token: Option<&str>) -> EmbeddingConfig {
        EmbeddingConfig {
            provider: "ollama".to_string(),
            model: "nomic-embed-text".to_string(),
            ollama_host: host.to_string(),
            ollama_api_key: token.map(str::to_string),
            priority: Vec::new(),
        }
    }

    fn config(host: &str, token: Option<&str>) -> nanna_config::Config {
        let mut config = nanna_config::Config::default();
        config.memory.ollama_host = host.to_string();
        config.memory.embedding_provider = "ollama".to_string();
        config.memory.embedding_model = "nomic-embed-text".to_string();
        config.memory.embedding_priority = Vec::new();
        config.llm.ollama_api_key = token.map(str::to_string);
        config
    }

    /// The token goes only to the server it was configured for. Embeddings
    /// keep the boot address until a restart, so a token saved alongside a
    /// NEW address is for a server they are not talking to — it waits with
    /// the address instead of going to the old one.
    #[test]
    fn a_token_follows_only_while_the_server_is_the_same() {
        let live = LiveEmbeddingSettings::new(&boot("https://host/ollama", Some("t0")));
        let bearer = live.ollama_bearer();

        live.apply(&config("https://elsewhere/ollama", Some("t1")));
        assert_eq!(bearer.token(), "t0", "held back from the old server");

        // The same server, spelled as a user pastes it.
        live.apply(&config(" https://HOST/ollama/ ", Some("t1")));
        assert_eq!(bearer.token(), "t1", "the saved token is the one sent");

        live.apply(&config("https://host/ollama", None));
        assert_eq!(bearer.token(), "", "a removed token is removed");
    }

    /// What waits for a restart is named, and only what really differs: the
    /// same server or model list spelled another way is not a change.
    #[test]
    fn only_a_real_change_waits_for_a_restart() {
        let running = boot("http://localhost:11434", None);

        let mut same = boot("http://127.0.0.1:11434/", Some("a token is not an address"));
        same.priority = vec!["ollama/nomic-embed-text".to_string()];
        assert_eq!(waiting_for_restart(&running, &same), Vec::<String>::new());

        let mut moved = boot("https://host/ollama", None);
        moved.model = "mxbai-embed-large".to_string();
        let waiting = waiting_for_restart(&running, &moved);
        assert_eq!(waiting.len(), 2, "{waiting:?}");
        assert!(
            waiting[0].contains("http://localhost:11434")
                && waiting[0].contains("https://host/ollama"),
            "names both servers: {waiting:?}"
        );
        assert!(
            waiting[1].contains("nomic-embed-text") && waiting[1].contains("mxbai-embed-large"),
            "names both model lists: {waiting:?}"
        );
    }
}
