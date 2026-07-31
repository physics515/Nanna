//! The summarizer the daemon's dream cycles call.
//!
//! Both consolidation paths — the scheduled cycle in [`crate::server`] and the
//! IPC `MemoryAction::Consolidate` in [`crate::control`] — need the same three
//! things: the user's configured summarization models, a **failover walk** over
//! that list, and a cluster byte budget that is safe for whichever model ends
//! up answering. They used to disagree: the scheduled path took only
//! `summarization_priority.first()` and made a single attempt, so one
//! unavailable or rate-limited model killed the whole nightly dream cycle,
//! while the IPC path already walked the list. This module is the one
//! implementation both now share.

use crate::llm_router::LlmRouter;
use nanna_llm::RequestBuilder;
use std::sync::Arc;
use std::sync::OnceLock;

/// Minimum spacing between consecutive OpenRouter requests from dream cycles.
///
/// OpenRouter's free tier allows 20 requests per minute; 60s / 20 = 3s. The
/// bound is the published limit, not a tuning knob. A dream cycle issues one
/// request per cluster back to back, and before this gate existed the first
/// working cycle spent its entire minute budget in about one second and lost
/// 20 of 22 clusters to 429s. Reactive backoff cleans up after that mistake;
/// this stops it being made.
const OPENROUTER_MIN_SPACING: tokio::time::Duration = tokio::time::Duration::from_secs(3);

/// When the previous OpenRouter-bound request was issued, across every dream
/// cluster and cycle in the process. `tokio::time::Instant` so tests can drive
/// it with paused time.
static LAST_OPENROUTER_REQUEST: OnceLock<tokio::sync::Mutex<Option<tokio::time::Instant>>> =
    OnceLock::new();

/// Hold OpenRouter-bound requests to [`OPENROUTER_MIN_SPACING`].
///
/// Non-OpenRouter models pass through untouched — local Ollama has no request
/// quota, and stalling it 3s per cluster would slow every dream for no reason.
/// The lock is held across the sleep deliberately: concurrent callers must
/// serialize, or they would all measure from the same "last" stamp and fire
/// together anyway.
async fn respect_provider_spacing(model: &str) {
    if !model.starts_with("openrouter/") {
        return;
    }
    let gate = LAST_OPENROUTER_REQUEST.get_or_init(|| tokio::sync::Mutex::new(None));
    let mut last = gate.lock().await;
    if let Some(prev) = *last {
        let elapsed = prev.elapsed();
        if elapsed < OPENROUTER_MIN_SPACING {
            tokio::time::sleep(OPENROUTER_MIN_SPACING - elapsed).await;
        }
    }
    *last = Some(tokio::time::Instant::now());
}

/// Smallest summarizer context window we will ever size a cluster against.
///
/// Mirrors `nanna_memory`'s own fallback for an unknown model. A `ModelInfo`
/// this small means we failed to resolve the model at all; clamping here keeps
/// the byte-budget math meaningful instead of collapsing to zero.
const MIN_SUMMARIZER_CONTEXT_TOKENS: usize = 8_192;

/// The ordered list of models a dream cycle may summarize with.
///
/// Returns `priority` when the user configured one, else `fallback` — the two
/// callers differ in what "fallback" means (the scheduled cycle falls back to
/// the agent's single main model, the IPC one to the whole `model_priority`
/// list), so it is taken as a slice rather than baked in.
///
/// Pure. The result is empty only when **both** inputs are empty, which is the
/// genuinely unconfigured case the callers report as such.
#[must_use]
pub fn summarization_models(priority: &[String], fallback: &[String]) -> Vec<String> {
    let models: Vec<String> = if priority.is_empty() {
        fallback.to_vec()
    } else {
        priority.to_vec()
    };

    debug_assert!(
        !(models.is_empty() && !(priority.is_empty() && fallback.is_empty())),
        "the list may only be empty when both inputs are empty"
    );
    debug_assert!(
        priority.is_empty() || models.len() == priority.len(),
        "a configured priority list must be preserved verbatim"
    );
    models
}

/// The cluster byte budget must hold for **whichever** model actually answers.
///
/// A dream cycle builds one prompt and then walks the failover list with it, so
/// sizing the budget to the *first* model would overflow a smaller fallback.
/// Taking the **minimum** `hard_input_limit` across every candidate makes the
/// prompt safe for all of them. The cost is only that a large model consolidates
/// a little less per pass — never lost content, because a cluster that would
/// breach the bound simply re-clusters on a later seed.
pub async fn summarizer_context_window_tokens(router: &LlmRouter, models: &[String]) -> usize {
    debug_assert!(
        !models.is_empty(),
        "context window must be resolved against at least one model"
    );

    // Seed with the floor rather than `usize::MAX`: an empty list must fall back
    // to the smallest safe window, never leave the budget effectively unbounded.
    // (Both callers guarantee a non-empty list today — this keeps the bound a
    // bound if a third one ever forgets.)
    let mut smallest_tokens = MIN_SUMMARIZER_CONTEXT_TOKENS;
    for (index, model) in models.iter().enumerate() {
        let limit = router.get_model_info(model).await.hard_input_limit();
        smallest_tokens = if index == 0 {
            limit
        } else {
            smallest_tokens.min(limit)
        };
    }

    let resolved = smallest_tokens.max(MIN_SUMMARIZER_CONTEXT_TOKENS);
    debug_assert!(
        resolved >= MIN_SUMMARIZER_CONTEXT_TOKENS,
        "resolved window must respect the floor"
    );
    resolved
}

/// Build the `summarize_fn` a dream cycle calls, walking `models` in order and
/// returning the first success.
///
/// Failure of one model is an *expected* operational condition (down,
/// rate-limited, out of credit), so it is logged and the walk continues; only
/// exhausting every candidate is an error, and that error names the last real
/// failure rather than a generic message.
///
/// The returned closure is `Send + Sync` and its future is `Send`, matching the
/// bounds `nanna_memory::DreamingService` requires.
#[must_use]
pub fn summarize_with_failover(
    router: Arc<LlmRouter>,
    models: Vec<String>,
) -> impl Fn(
    String,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
+ Send
+ Sync {
    debug_assert!(
        !models.is_empty(),
        "a failover summarizer needs at least one model"
    );

    move |prompt: String| {
        let router = Arc::clone(&router);
        let models = models.clone();
        Box::pin(async move {
            // Wait out congestion rather than failing the cluster.
            //
            // A dream cycle asks for one summary per cluster, back to back, and
            // a free tier allows 20 requests a minute — so the first cycle after
            // this path started working burned its whole budget in one second
            // and lost 20 of 22 clusters to 429s. Those clusters are not
            // retried later; the consolidation simply does not happen, so the
            // store keeps growing uncompressed all night.
            //
            // Congestion is a throughput limit, not a failure. Waiting spreads
            // the cycle over a few minutes, which is the correct price. Only a
            // model that is genuinely broken gets failed over.
            const BACKOFF_SECS: [u64; 6] = [5, 15, 30, 60, 120, 240];

            let mut last_error = String::from("no summarization models configured");
            for (round, wait) in std::iter::once(0)
                .chain(BACKOFF_SECS.iter().copied())
                .enumerate()
            {
                if wait > 0 {
                    tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                }

                let mut soonest_retry: Option<u64> = None;
                let mut congested = 0usize;

                for model in &models {
                    respect_provider_spacing(model).await;
                    let request = nanna_llm::CompletionRequest::default()
                        .with_model(model)
                        .with_message(nanna_llm::Message::user(&prompt));
                    match router.complete(model, request).await {
                        Ok(summary) => {
                            if round > 0 {
                                tracing::info!(
                                    "Dream summarization cleared congestion after {round} wait(s)"
                                );
                            }
                            return Ok(summary);
                        }
                        Err(e) if e.is_rate_limit() => {
                            congested += 1;
                            if let nanna_llm::LlmError::RateLimit { retry_after, .. } = &e
                                && let Some(secs) = retry_after
                            {
                                soonest_retry =
                                    Some(soonest_retry.map_or(*secs, |cur: u64| cur.min(*secs)));
                            }
                            last_error = format!("{model}: {e}");
                        }
                        Err(e) => {
                            tracing::warn!("Dream summarization model {model} failed: {e}");
                            last_error = format!("{model}: {e}");
                        }
                    }
                }

                if congested == 0 {
                    break;
                }

                // The provider says when it will be ready; prefer that over the
                // schedule, which only exists for providers that decline to say.
                let next = soonest_retry.unwrap_or_else(|| {
                    BACKOFF_SECS.get(round).copied().unwrap_or(240)
                });
                tracing::info!(
                    "Dream summarization congested on all {} model(s) — waiting {}s",
                    models.len(),
                    next
                );
                if let Some(secs) = soonest_retry {
                    tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                }
            }

            Err(format!(
                "all {} summarization model(s) failed; last error — {last_error}",
                models.len()
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn configured_priority_is_used_verbatim() {
        // The whole list, in order — this is the fix: the scheduled cycle used
        // to take only the head and make a single attempt.
        let priority = v(&["small-local", "big-cloud"]);
        let models = summarization_models(&priority, &v(&["main-model"]));
        assert_eq!(models, priority, "order and contents must be preserved");
    }

    #[test]
    fn empty_priority_falls_back() {
        // Single-model fallback (the scheduled cycle's shape)…
        assert_eq!(summarization_models(&[], &v(&["main"])), v(&["main"]));
        // …and a whole fallback list (the IPC path's shape).
        assert_eq!(summarization_models(&[], &v(&["a", "b"])), v(&["a", "b"]));
    }

    #[test]
    fn priority_wins_over_fallback() {
        // Negative space: the fallback must not leak in when a priority exists.
        let models = summarization_models(&v(&["chosen"]), &v(&["ignored"]));
        assert_eq!(models, v(&["chosen"]));
        assert!(!models.contains(&"ignored".to_string()));
    }

    #[test]
    fn empty_only_when_nothing_is_configured() {
        // The single case callers must report as unconfigured.
        assert!(summarization_models(&[], &[]).is_empty());
        // …and never otherwise.
        assert!(!summarization_models(&v(&["a"]), &[]).is_empty());
        assert!(!summarization_models(&[], &v(&["b"])).is_empty());
    }

    /// Back-to-back OpenRouter requests must be held to the derived 3s spacing
    /// (20/min free tier) — the first working dream cycle fired one request per
    /// cluster in the same second and lost 20 of 22 clusters to 429s.
    #[tokio::test(start_paused = true)]
    async fn openrouter_requests_are_spaced() {
        let t0 = tokio::time::Instant::now();
        respect_provider_spacing("openrouter/openrouter/free").await;
        respect_provider_spacing("openrouter/nvidia/nemotron:free").await;
        respect_provider_spacing("openrouter/openrouter/free").await;
        let elapsed = t0.elapsed();
        assert!(
            elapsed >= OPENROUTER_MIN_SPACING * 2,
            "three consecutive calls must span at least two spacing intervals, got {elapsed:?}"
        );
    }

    /// Local models have no quota; stalling them would slow every dream for
    /// nothing.
    #[tokio::test(start_paused = true)]
    async fn local_models_are_never_stalled() {
        let t0 = tokio::time::Instant::now();
        for _ in 0..5 {
            respect_provider_spacing("ollama/qwen3.5:9b").await;
        }
        assert_eq!(
            t0.elapsed(),
            tokio::time::Duration::ZERO,
            "non-OpenRouter models must pass through untouched"
        );
    }
}
