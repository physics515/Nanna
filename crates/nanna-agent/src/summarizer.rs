
use crate::chunker::analyze_content;
use nanna_llm::LlmClient;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;

/// A resolver's signature: a summarization-model spec as Settings lists it
/// (`ollama/qwen3:4b`, `anthropic/claude-haiku-4-5`, `openrouter/…`) to the
/// client that serves it and the bare model id that client knows it by — or a
/// sentence saying why it cannot be reached.
type ResolveFn = dyn Fn(&str) -> Result<(LlmClient, String), String> + Send + Sync;

/// The one way a summarization-model spec becomes a client.
///
/// Injected, because the grammar and the credentials belong to whoever owns
/// the providers — the daemon's chat router — and this crate cannot depend on
/// it. Every summarizing consumer in the agent resolves through this one seam
/// (the context ladder and its preamble re-condense, tool-result compression
/// and summarization, distillation, memory extraction), so a spec means the
/// same thing to all of them and to chat: an `ollama/` entry reaches the server
/// and token chat uses, an `anthropic/` entry the Anthropic credential.
///
/// Called on every use and never cached here. The daemon's resolver snapshots
/// the router's provider map per call, so a config reload — a new Ollama
/// server, token or key — reaches the very next summarization, even inside a
/// turn that is already running.
#[derive(Clone)]
pub struct SummarizerClients(Arc<ResolveFn>);

impl SummarizerClients {
    /// Wrap a resolver.
    #[must_use]
    pub fn new(
        resolve: impl Fn(&str) -> Result<(LlmClient, String), String> + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(resolve))
    }

    /// The client and bare model id for `spec`.
    ///
    /// # Errors
    ///
    /// Returns the resolver's own sentence when `spec` names a provider this
    /// process cannot reach (no credential, an unknown prefix, a blank entry).
    pub fn resolve(&self, spec: &str) -> Result<(LlmClient, String), String> {
        (self.0)(spec)
    }
}

impl std::fmt::Debug for SummarizerClients {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A closure has nothing printable; saying one is attached is the
        // useful half of the answer.
        f.write_str("SummarizerClients(<resolver>)")
    }
}

/// Resolve `spec` through `clients`, or say that nothing can.
///
/// With no resolver attached — a construction that never wired one, such as
/// `nanna serve`'s per-session agent — no spec resolves. The consumers read
/// that exactly as they read an unreachable provider: move to the next model,
/// and when none is left, cut to fit (or, for memory extraction, fall back to
/// the chat model).
pub fn resolve_summarizer(
    clients: Option<&SummarizerClients>,
    spec: &str,
) -> Result<(LlmClient, String), String> {
    clients.map_or_else(
        || {
            Err(format!(
                "no summarization provider is attached to this agent, so `{spec}` cannot be \
                 reached"
            ))
        },
        |clients| clients.resolve(spec),
    )
}

/// Configuration for the summarizer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SummarizerConfig {
    /// Minimum content length (bytes) before summarization is triggered.
    pub threshold: usize,
    /// Maximum summary length (tokens).
    pub max_summary_tokens: usize,
}

impl Default for SummarizerConfig {
    fn default() -> Self {
        Self {
            threshold: 50_000,
            max_summary_tokens: 1000,
        }
    }
}

/// Summarizes long content using local or remote LLMs.
///
/// Breaks content into chunks, deduplicates against known content,
/// and generates summaries to reduce context window usage.
pub struct Summarizer {
    config: SummarizerConfig,
    known_hashes: HashSet<u64>,
}

impl Summarizer {
    /// Create a new summarizer with the given config.
    #[must_use]
    pub fn new(config: SummarizerConfig) -> Self {
        Self {
            config,
            known_hashes: HashSet::new(),
        }
    }

    /// Check if content should be summarized based on length threshold.
    #[must_use]
    pub const fn should_summarize(&self, content: &str) -> bool {
        content.len() > self.config.threshold
    }

    /// Analyze content for redundancy.
    ///
    /// Returns analysis of novel vs. redundant chunks.
    /// Used to skip processing previously-seen content.
    #[must_use]
    pub fn analyze_redundancy(&self, content: &str) -> crate::chunker::DeduplicationAnalysis {
        analyze_content(content, &self.known_hashes)
    }

    /// Register content hashes as known (already processed).
    ///
    /// Prevents re-summarization of identical or similar content.
    pub fn register_hashes(&mut self, hashes: HashSet<u64>) {
        self.known_hashes.extend(hashes);
    }

    /// Get the current configuration.
    #[must_use]
    pub const fn config(&self) -> &SummarizerConfig {
        &self.config
    }

    /// Update configuration.
    pub const fn set_config(&mut self, config: SummarizerConfig) {
        self.config = config;
    }

    /// Estimate the token count for content.
    ///
    /// Rough heuristic: ~4 characters per token.
    #[must_use]
    pub const fn estimate_tokens(&self, content: &str) -> usize {
        content.len().div_ceil(4)
    }
}

#[cfg(test)]
pub mod test_server {
    //! Stand-in summarization servers for the tests that follow a Settings
    //! list from spec to wire: one that answers, and an address that refuses.

    use super::SummarizerClients;
    use nanna_llm::LlmClient;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A port on this machine nothing listens on: every request is refused
    /// at once, the way a stopped Ollama server refuses.
    pub const REFUSING: &str = "http://127.0.0.1:9";

    /// Serve Ollama's `/api/chat` on an ephemeral port, answering every chat
    /// with `answer` as the assistant's text and recording the model each one
    /// named. Every other path is a 404, so model-info lookups fall back to
    /// the unknown-model floor without writing anything to the info cache.
    pub async fn spawn_answering(answer: &str) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind an ephemeral port");
        let addr = listener.local_addr().expect("read back the bound addr");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&seen);
        let chat_body = serde_json::json!({
            "message": { "role": "assistant", "content": answer },
            "done": true,
            "done_reason": "stop",
        })
        .to_string();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let (path, model) = read_request(&mut stream).await;
                let (status, body) = if path == "/api/chat" {
                    record
                        .lock()
                        .expect("record lock")
                        .push(model.unwrap_or_default());
                    ("200 OK", chat_body.as_str())
                } else {
                    ("404 Not Found", r#"{"error":"not found"}"#)
                };
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

    /// Accept every connection and never answer: a server that has taken the
    /// request and is still thinking about it. Connections are held open for
    /// as long as the test runs.
    pub async fn spawn_silent() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind an ephemeral port");
        let addr = listener.local_addr().expect("read back the bound addr");
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                // Each connection lives in its own task that never returns,
                // so it stays open — silent — until the runtime goes away.
                tokio::spawn(async move {
                    let _held = stream;
                    std::future::pending::<()>().await;
                });
            }
        });
        format!("http://{addr}")
    }

    /// A resolver that sends each listed `ollama/<model>` spec to the server
    /// named beside it and refuses every other spec, as a router with no
    /// provider for it would.
    pub fn stub_clients(routes: &[(&str, &str)]) -> SummarizerClients {
        let routes: Vec<(String, String)> = routes
            .iter()
            .map(|(spec, url)| ((*spec).to_string(), (*url).to_string()))
            .collect();
        SummarizerClients::new(move |spec| {
            routes
                .iter()
                .find(|(listed, _)| listed == spec)
                .map(|(_, url)| {
                    let bare = spec.strip_prefix("ollama/").unwrap_or(spec);
                    (LlmClient::ollama(url), bare.to_string())
                })
                .ok_or_else(|| format!("no provider serves `{spec}`"))
        })
    }

    /// Read one whole request: the path, and the JSON body's `model`.
    async fn read_request(stream: &mut tokio::net::TcpStream) -> (String, Option<String>) {
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
        let length: usize = head
            .lines()
            .skip(1)
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.trim()
                    .eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().ok())?
            })
            .unwrap_or(0);
        while bytes.len() < head_end + length {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => bytes.extend_from_slice(&chunk[..n]),
            }
        }
        let model = serde_json::from_slice::<serde_json::Value>(bytes.get(head_end..).unwrap_or_default())
            .ok()
            .and_then(|json| json.get("model")?.as_str().map(str::to_string));
        let path = head.split_whitespace().nth(1).unwrap_or_default().to_string();
        (path, model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_summarizer_creation() {
        let config = SummarizerConfig::default();
        let summarizer = Summarizer::new(config);
        assert_eq!(summarizer.config.threshold, 50_000);
    }

    #[test]
    fn test_should_summarize() {
        let config = SummarizerConfig {
            threshold: 1000,
            ..Default::default()
        };
        let summarizer = Summarizer::new(config);

        let short = "short text";
        let long = "a".repeat(2000);

        assert!(!summarizer.should_summarize(short));
        assert!(summarizer.should_summarize(&long));
    }

    #[test]
    fn test_register_hashes() {
        let config = SummarizerConfig::default();
        let mut summarizer = Summarizer::new(config);

        let mut hashes = HashSet::new();
        hashes.insert(12345);
        hashes.insert(67890);

        summarizer.register_hashes(hashes);
        assert_eq!(summarizer.known_hashes.len(), 2);
    }

    #[test]
    fn test_estimate_tokens() {
        let config = SummarizerConfig::default();
        let summarizer = Summarizer::new(config);

        let content = "This is a test."; // ~15 chars
        let tokens = summarizer.estimate_tokens(content);
        assert!(tokens > 0);
    }
}
