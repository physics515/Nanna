//! Multi-provider embedding router with automatic fallback and re-embedding
//!
//! Routes embedding requests to the primary provider, falling back to alternates
//! when the primary is unavailable. Tracks provider switches and signals when
//! re-embedding may be needed due to dimension changes.

use nanna_llm::EmbeddingClient;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Identifies an embedding provider
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EmbeddingProviderInfo {
    /// Provider name (e.g., "ollama", "openai")
    pub name: String,
    /// Model name (e.g., "nomic-embed-text", "text-embedding-3-small")
    pub model: String,
}

impl std::fmt::Display for EmbeddingProviderInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.name, self.model)
    }
}

/// Backoff schedule used when EVERY provider is congested at once.
///
/// Congestion on a free tier is a throughput limit, not a failure — the correct
/// response is to come back later, and roughly nine minutes of patience is a
/// fair price for not losing the memory entirely. It only applies when there is
/// nothing else to try: a provider that is merely busy is waited for, a provider
/// that is broken is skipped, and any healthy provider wins immediately.
const BACKOFF_SECS: [u64; 7] = [2, 5, 15, 30, 60, 120, 240];

/// How long a deterministically-failing provider stays benched before it gets
/// one probe call.
///
/// A non-rate-limit failure (401 bad key, 402 no credit, 404 wrong model, 422
/// unprocessable, network down) does not clear on its own — something outside
/// has to change, and there is no signal for when it does. Retrying per call,
/// as the 2026-08-02 incident did with a 422ing primary, buys nothing but an
/// error per write. The only honest recovery is a bounded re-probe horizon,
/// and the router already has one: the last step of [`BACKOFF_SECS`] is the
/// longest it is ever willing to assume provider state stays put. Reuse that
/// rather than invent a second number.
const DEMOTION_SECS: u64 = BACKOFF_SECS[BACKOFF_SECS.len() - 1];

/// An embedding provider entry in the router
struct EmbeddingProviderEntry {
    info: EmbeddingProviderInfo,
    client: Arc<EmbeddingClient>,
}

/// Multi-provider embedding router with automatic fallback.
///
/// Tries the primary provider first, then falls back through alternates
/// in order. Tracks which provider is currently active so consumers can
/// detect provider switches and trigger re-embedding if needed.
pub struct EmbeddingRouter {
    /// Ordered list: index 0 is primary, rest are fallbacks
    providers: Vec<EmbeddingProviderEntry>,
    /// Index of the currently active provider
    active_index: RwLock<usize>,
    /// Generation counter — incremented on every provider switch.
    /// Consumers compare their last-seen generation to detect changes.
    generation: AtomicU64,
    /// Per-provider bench deadline; `Some(t)` means "do not call before `t`".
    /// Same length as `providers`. See [`DEMOTION_SECS`].
    demoted_until: RwLock<Vec<Option<Instant>>>,
}

impl EmbeddingRouter {
    /// Create a new router with a primary provider.
    pub fn new(info: EmbeddingProviderInfo, client: Arc<EmbeddingClient>) -> Self {
        Self {
            providers: vec![EmbeddingProviderEntry { info, client }],
            active_index: RwLock::new(0),
            generation: AtomicU64::new(0),
            demoted_until: RwLock::new(vec![None]),
        }
    }

    /// Add a fallback provider. Order matters — tried in insertion order.
    #[must_use]
    pub fn with_fallback(mut self, info: EmbeddingProviderInfo, client: Arc<EmbeddingClient>) -> Self {
        self.providers.push(EmbeddingProviderEntry { info, client });
        self.demoted_until.get_mut().push(None);
        self
    }

    /// Get the current generation counter.
    /// Consumers should store this and compare on subsequent calls.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// Get info about the currently active provider.
    pub async fn active_provider(&self) -> EmbeddingProviderInfo {
        let idx = *self.active_index.read().await;
        self.providers[idx].info.clone()
    }

    /// Whether `idx` is currently benched (deterministic failure, cooldown not
    /// yet passed).
    async fn is_demoted(&self, idx: usize) -> bool {
        self.demoted_until.read().await[idx].is_some_and(|until| Instant::now() < until)
    }

    /// Bench `idx` for `cooldown` — it failed deterministically, so calling it
    /// again sooner is a guaranteed error per call.
    async fn demote(&self, idx: usize, cooldown: Duration) {
        self.demoted_until.write().await[idx] = Some(Instant::now() + cooldown);
    }

    /// Record that the caller has just observed a healthy embedding from the
    /// provider at `idx`; if it differs from the live active provider, switch
    /// to it and bump the generation.
    ///
    /// The check-and-swap runs under the live lock, not against the index the
    /// sweep started from — so a burst of racing calls that all found the same
    /// fallback produces ONE switch and ONE generation bump, not one per call.
    /// (Each spurious bump used to trigger a full store rebind downstream.)
    ///
    /// Returns the provider's info when this call actually flipped the active
    /// provider, `None` when it was already active.
    async fn commit_success(&self, idx: usize) -> Option<EmbeddingProviderInfo> {
        self.demoted_until.write().await[idx] = None;
        let mut active = self.active_index.write().await;
        if *active == idx {
            return None;
        }
        let old_info = self.providers[*active].info.clone();
        *active = idx;
        drop(active);
        self.generation.fetch_add(1, Ordering::Relaxed);
        let entry = &self.providers[idx];
        info!(
            "Embedding provider switched: {} → {} (generation {})",
            old_info,
            entry.info,
            self.generation.load(Ordering::Relaxed)
        );
        Some(entry.info.clone())
    }

    /// Embed a single text with automatic fallback.
    ///
    /// Tries the current active provider first. On failure, iterates through
    /// all providers starting from the next one. On successful fallback,
    /// updates the active provider and increments the generation counter.
    ///
    /// Returns `(embedding, switched_to)` where `switched_to` names the
    /// provider that produced the vector IF this call flipped the active
    /// provider to it. Vector and provider identity travel together so the
    /// caller can rebind its store to a consistent `(model, width)` pair —
    /// reading the active provider back after the fact can race a second
    /// switch and tear the pair apart.
    pub async fn embed_one(
        &self,
        text: &str,
    ) -> Result<(Vec<f32>, Option<EmbeddingProviderInfo>), String> {
        let current_idx = *self.active_index.read().await;
        let total = self.providers.len();

        // Sweep EVERY provider before waiting on any of them.
        //
        // The earlier shape spent the whole nine-minute patience budget on the
        // active provider before it would so much as look at the next one. That
        // is wrong twice over. A provider that is out of credit is not going to
        // recover in nine minutes, so a paid model hitting its cap would strand
        // the run while a perfectly healthy free-tier model sat unused. And the
        // reason waiting was preferred to switching — that a switch forced a
        // full re-embed at the new width — stopped being true the moment
        // embeddings became retained per-model buckets: a switch is now a
        // rebind, and switching back is a lookup.
        //
        // So: try everyone, and only wait if EVERYONE is merely congested.
        // Congestion is the one condition that clears on its own.
        for (round, wait) in std::iter::once(0)
            .chain(BACKOFF_SECS.iter().copied())
            .enumerate()
        {
            if wait > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            }

            let mut soonest_retry: Option<u64> = None;
            let mut congested = 0usize;
            let mut benched = 0usize;
            let mut last_error = String::new();

            for offset in 0..total {
                let idx = (current_idx + offset) % total;
                let entry = &self.providers[idx];

                // A benched provider's last error was deterministic; calling it
                // again before the cooldown is a guaranteed failure. Skip it —
                // this is what turns "422 per call" into "422 once, then quiet
                // until the probe horizon".
                if self.is_demoted(idx).await {
                    benched += 1;
                    continue;
                }

                match entry.client.embed_one(text).await {
                    Ok(embedding) if embedding.is_empty() => {
                        // An empty vector can never be a usable embedding, and
                        // downstream it would rebind the store to width 0.
                        // Treat it as the deterministic fault it is.
                        warn!("Embedding provider {} returned an empty vector", entry.info);
                        self.demote(idx, Duration::from_secs(DEMOTION_SECS)).await;
                        last_error = format!("{} returned an empty vector", entry.info);
                    }
                    Ok(embedding) => {
                        let switched_to = self.commit_success(idx).await;
                        return Ok((embedding, switched_to));
                    }
                    Err(e) if e.is_rate_limit() => {
                        congested += 1;
                        // Wait only as long as the SOONEST provider needs, not
                        // as long as the slowest — the schedule is a fallback
                        // for providers that decline to say.
                        if let nanna_llm::LlmError::RateLimit { retry_after, .. } = &e
                            && let Some(secs) = retry_after
                        {
                            soonest_retry =
                                Some(soonest_retry.map_or(*secs, |cur: u64| cur.min(*secs)));
                        }
                        debug!("Embedding provider {} is congested", entry.info);
                        last_error = e.to_string();
                    }
                    Err(e) => {
                        // Genuinely unavailable — no key, no credit, wrong
                        // model, unprocessable payload. Waiting will not fix
                        // it, so it does not count toward "everything is busy"
                        // — and it is benched so the next call does not pay to
                        // rediscover the same failure.
                        warn!(
                            "Embedding provider {} unavailable (benched {}s): {}",
                            entry.info, DEMOTION_SECS, e
                        );
                        self.demote(idx, Duration::from_secs(DEMOTION_SECS)).await;
                        last_error = e.to_string();
                    }
                }
            }

            if congested == 0 {
                if benched == total {
                    return Err(format!(
                        "All {total} embedding providers are benched after deterministic \
                         failures — next probe in at most {DEMOTION_SECS}s"
                    ));
                }
                return Err(format!(
                    "All {total} embedding providers failed (none merely congested) — {last_error}"
                ));
            }

            let next = soonest_retry.unwrap_or(wait);
            warn!(
                "All {} embedding provider(s) congested ({} of {}) — waiting {}s before sweep {}",
                congested,
                congested,
                total,
                next,
                round + 2
            );
        }

        Err(format!(
            "All {total} embedding providers still congested after the full retry schedule"
        ))
    }

    /// Try to restore the primary provider.
    ///
    /// Call this periodically (e.g., on a timer or every N embed calls) to
    /// check if the primary has recovered. Only probes if currently on a
    /// fallback, and never inside the primary's bench cooldown.
    ///
    /// Returns `true` if primary was restored.
    pub async fn try_restore_primary(&self) -> bool {
        let current_idx = *self.active_index.read().await;
        if current_idx == 0 {
            return false; // Already on primary
        }
        if self.is_demoted(0).await {
            return false; // Benched — probing now is the per-call retry again
        }

        let primary = &self.providers[0];
        match primary.client.embed_one("probe").await {
            Ok(embedding) if !embedding.is_empty() => {
                self.commit_success(0).await.is_some()
            }
            _ => {
                debug!("Primary embedding provider {} still unavailable", primary.info);
                self.demote(0, Duration::from_secs(DEMOTION_SECS)).await;
                false
            }
        }
    }

    /// Number of configured providers (primary + fallbacks)
    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Serve every request on an ephemeral port with a fixed status + body,
    /// counting how many requests arrived. `Connection: close` forces one TCP
    /// connection per request, so the accept count IS the request count.
    async fn spawn_counting_server(status: u16, body: &'static str) -> (String, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind an ephemeral port");
        let addr = listener.local_addr().expect("read back the bound addr");
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                counter.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0_u8; 4096];
                let _ = stream.read(&mut buf).await;
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len(),
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.flush().await;
            }
        });
        (format!("http://{addr}"), hits)
    }

    /// OpenAI-shaped client so each `embed_one` is exactly ONE http request
    /// (the Ollama client falls back new-API→legacy and would count double).
    fn openai_provider(name: &str, base_url: &str) -> (EmbeddingProviderInfo, Arc<EmbeddingClient>) {
        (
            EmbeddingProviderInfo {
                name: name.into(),
                model: "test-embed".into(),
            },
            Arc::new(
                EmbeddingClient::openai("test-key")
                    .with_model("test-embed")
                    .with_base_url(base_url),
            ),
        )
    }

    const HEALTHY_BODY: &str = r#"{"data":[{"embedding":[0.1,0.2,0.3]}]}"#;

    /// The incident's per-call retry: a provider that deterministically 422s
    /// must be called ONCE, benched, and then skipped without a network call —
    /// not re-tried on every embed.
    #[tokio::test]
    async fn a_deterministic_422_provider_is_benched_not_retried_per_call() {
        let (url, hits) = spawn_counting_server(422, r#"{"error":{"message":"bad input"}}"#).await;
        let (info, client) = openai_provider("openrouter", &url);
        let router = EmbeddingRouter::new(info, client);

        for _ in 0..3 {
            let err = router
                .embed_one("text")
                .await
                .expect_err("a 422-only router cannot produce an embedding");
            assert!(
                err.contains("benched") || err.contains("failed"),
                "the error names the condition: {err}"
            );
        }
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "the dead provider is hit once, then benched — never once per call"
        );
    }

    /// A switch reports the provider that produced the vector, exactly once —
    /// the vector and its provenance must travel together so the caller can
    /// rebind to a consistent (model, width) pair.
    #[tokio::test]
    async fn a_switch_reports_the_producing_provider_exactly_once() {
        let (dead_url, dead_hits) =
            spawn_counting_server(422, r#"{"error":{"message":"bad input"}}"#).await;
        let (healthy_url, _healthy_hits) = spawn_counting_server(200, HEALTHY_BODY).await;
        let (dead_info, dead_client) = openai_provider("openrouter", &dead_url);
        let (ok_info, ok_client) = openai_provider("ollama", &healthy_url);
        let router = EmbeddingRouter::new(dead_info, dead_client).with_fallback(ok_info, ok_client);

        let (embedding, switched) = router.embed_one("text").await.expect("fallback answers");
        assert_eq!(embedding.len(), 3, "the fallback's vector comes through");
        let switched = switched.expect("the first call flips the active provider");
        assert_eq!(
            switched.name, "ollama",
            "the reported provider is the one that produced the vector"
        );
        assert_eq!(router.generation(), 1, "one flip, one generation bump");

        let (_, switched_again) = router.embed_one("text").await.expect("still healthy");
        assert!(
            switched_again.is_none(),
            "an already-active provider is not reported as a switch"
        );
        assert_eq!(router.generation(), 1, "no spurious generation bump");
        assert_eq!(
            dead_hits.load(Ordering::SeqCst),
            1,
            "the dead primary was benched by the first sweep"
        );
    }

    /// The bench expires: after the cooldown the provider is eligible again,
    /// so a fixed outage never becomes a permanent demotion.
    #[tokio::test]
    async fn a_bench_expires_after_its_cooldown() {
        let (url, _hits) = spawn_counting_server(200, HEALTHY_BODY).await;
        let (info, client) = openai_provider("ollama", &url);
        let router = EmbeddingRouter::new(info, client);

        router.demote(0, Duration::ZERO).await;
        assert!(
            !router.is_demoted(0).await,
            "a zero cooldown is already expired — the provider must be eligible"
        );
        let (embedding, switched) = router.embed_one("text").await.expect("eligible again");
        assert_eq!(embedding.len(), 3);
        assert!(switched.is_none(), "it was the active provider all along");
    }
}
