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

    let mut smallest_tokens = usize::MAX;
    for model in models {
        let limit = router.get_model_info(model).await.hard_input_limit();
        smallest_tokens = smallest_tokens.min(limit);
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
            let mut last_error = String::from("no summarization models configured");
            for model in &models {
                let request = nanna_llm::CompletionRequest::default()
                    .with_model(model)
                    .with_message(nanna_llm::Message::user(&prompt));
                match router.complete(model, request).await {
                    Ok(summary) => return Ok(summary),
                    Err(e) => {
                        tracing::warn!("Dream summarization model {model} failed: {e}");
                        last_error = format!("{model}: {e}");
                    }
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
}
