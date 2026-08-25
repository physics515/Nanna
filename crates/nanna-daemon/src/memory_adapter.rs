//! Adapter to bridge nanna-tools memory traits with nanna-memory MemoryService

use async_trait::async_trait;
use nanna_agent::{ExtractedMemory, TOOL_RESULT_CATEGORY};
use nanna_memory::MemoryService;
use nanna_tools::{MemoryResult, MemoryStorage};
use std::collections::HashMap;
use std::sync::Arc;

/// Adapter that implements MemoryStorage using the full MemoryService
pub struct MemoryServiceAdapter {
    service: Arc<MemoryService>,
    workspace_id: Option<String>,
}

impl MemoryServiceAdapter {
    pub fn new(service: Arc<MemoryService>) -> Self {
        Self { service, workspace_id: None }
    }

    pub fn with_workspace(service: Arc<MemoryService>, workspace_id: Option<String>) -> Self {
        Self { service, workspace_id }
    }
}

#[async_trait]
impl MemoryStorage for MemoryServiceAdapter {
    async fn store(&self, content: &str, tags: &[String]) -> Result<String, String> {
        let mut metadata = HashMap::new();
        if !tags.is_empty() {
            metadata.insert("tags".to_string(), tags.join(","));
        }
        
        // Use scoped remember so memories are tied to the active workspace
        self.service
            .remember_scoped(content, metadata, 3.0, self.workspace_id.clone())
            .await
            .map(|(id, _)| id)
            .map_err(|e| e.to_string())
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryResult>, String> {
        self.service
            .recall_scoped(query, self.workspace_id.as_deref())
            .await
            .map(|results| {
                results
                    .into_iter()
                    .take(limit)
                    .map(|r| MemoryResult {
                        id: r.id,
                        content: r.content,
                        score: Some(r.score),
                    })
                    .collect()
            })
            .map_err(|e| e.to_string())
    }

    async fn delete(&self, id: &str) -> Result<bool, String> {
        self.service
            .forget(id)
            .await
            .map(|_| true)
            .map_err(|e| e.to_string())
    }

    async fn list(&self, limit: usize) -> Result<Vec<MemoryResult>, String> {
        let all = self.service.list_all().await;
        Ok(all
            .into_iter()
            .take(limit)
            .map(|m| MemoryResult {
                id: m.id,
                content: m.content,
                score: Some(m.retrievability),
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Episodic write policy — ONE copy, used by every memory sink in the daemon.
//
// There were two, and they disagreed. `tasks.rs` (the harness/task runner) had
// the reasoned filter below; `agent_service.rs` (ordinary interactive chat)
// still carried the older one this doc comment describes as the bug — the
// substring failure tests plus a `content.len() < 20` floor, a dead `[Tool:`
// prefix, and a "dominated by non-ASCII" test that classified `tree` output and
// any non-Latin script as binary. So the exact loss documented below — an agent
// unable to recall its own failures, or its own source when the source contains
// an error string — was still live in the path the user actually chats through.
// One policy, one home, and every sink calls it.
// ---------------------------------------------------------------------------

/// Content not worth a memory: machine noise rather than an observation.
///
/// Deliberately narrow, and narrower than it used to be. This filter also
/// matched six "failure shapes" — `"Error:"`, `"Command failed"` and friends —
/// with `content.contains(s)` across the WHOLE body, not just the prefix.
/// Upstream, `loop_runner` rewrites every unsuccessful tool result to
/// `format!("Error: {…}")`, so the combination discarded **100% of failed tool
/// calls**: 704 of them in one 2-hour run, with not a single ingest line in the
/// whole day's log containing `FAILED`.
///
/// That is backwards. What went wrong is exactly what an agent must remember —
/// an agent that cannot recall its own failures repeats them, which is what a
/// long-horizon run looks like when it stalls. The substring form also ate
/// SUCCESSFUL calls whose output merely mentioned an error: `cat ./minidb`
/// stored nothing, twice, because the script contains its own error strings.
/// The agent could not remember reading its own source.
///
/// Failure is now carried structurally instead — the episodic writer stamps
/// `[tool → target — FAILED]` into the content and an `outcome` tag beside it —
/// so it can be filtered at RECALL time by anyone who wants only successes,
/// without being unwritable in the first place.
pub(crate) fn is_low_signal_memory(content: &str) -> bool {
    let trimmed = content.trim_start();
    if trimmed.is_empty() {
        return true;
    }
    // Binary/garbled output. Judged by CONTROL characters and decode failures,
    // not by "not ASCII" — the old test counted every non-ASCII char as noise,
    // so 40 box-drawing characters in `tree` output, or any text in a
    // non-Latin script, was classified as binary and deleted. It also flagged
    // this very writer's own `[exec → cmd — ok]` header punctuation.
    //
    // Real binary shows up as C0 control bytes and U+FFFD replacement
    // characters after a lossy decode; legitimate text does not.
    let noise = trimmed
        .chars()
        .take(200)
        .filter(|c| (c.is_control() && !c.is_whitespace()) || *c == '\u{FFFD}')
        .count();
    if noise > 40 {
        return true;
    }
    // Heartbeat chatter is the machinery talking to itself — not an observation.
    trimmed.starts_with("HEARTBEAT_OK")
}

/// Recall-ranking weight for an auto-extracted memory, by category.
///
/// The second thing both sinks had a private copy of. A tool result is raw
/// episodic material — worth keeping, but it must not outrank a preference the
/// user stated, so the spread between them is the whole point of the table and
/// is not a place for two copies to drift.
#[must_use]
pub(crate) fn episodic_importance(category: &str) -> f32 {
    match category {
        TOOL_RESULT_CATEGORY => 1.5,
        "preference" | "identity" => 4.0,
        "fact" | "insight" => 3.5,
        _ => 3.0,
    }
}

/// Store one auto-extracted memory — the whole sink, filter and route included.
///
/// The third policy both daemon sinks kept a private copy of. The filter and
/// the importance table were unified here after they drifted and cost 704
/// writes; the *route* — which write path a category takes — was about to
/// become the same story, so it lives here from the start. `agent_service.rs`
/// (interactive chat) and `tasks.rs` (the harness) now differ only in how they
/// log, which is the one thing they genuinely should.
///
/// The one branch that matters: **a tool result's vector does not go on the
/// turn's critical path** (P24.3 part 3). Every other category is a handful of
/// sentences extracted once per turn — one embedding round-trip, and folding it
/// against its neighbours is worth that round-trip. A tool result arrives per
/// call, already chunked, and each chunk cost an embed plus a vector search
/// plus an insert, awaited inline against the same local server that serves
/// generation; one measured run made zero model decisions for 189 of its 246
/// minutes. Its row still lands synchronously — so the `recall(...)` handle the
/// model is handed in the same turn resolves — and only the vector waits, for a
/// drain that does not wait for the turn to end.
///
/// Returns `Ok(None)` when the content was filtered as machine noise, so the
/// caller can say so; `Ok(Some(id))` for a write that landed.
///
/// # Errors
///
/// Returns `MemoryError` if the store rejects the write.
pub(crate) async fn store_extracted_memory(
    service: &MemoryService,
    memory: ExtractedMemory,
    workspace_id: Option<String>,
) -> Result<Option<String>, nanna_memory::MemoryError> {
    if is_low_signal_memory(&memory.content) {
        return Ok(None);
    }
    let mut metadata = memory.tags.unwrap_or_default();
    metadata.insert("category".to_string(), memory.category.clone());
    // Persist provenance so the store records STATED vs OBSERVED instead of
    // everything defaulting to "stated".
    metadata.insert(
        "fact_type".to_string(),
        memory.provenance.as_str().to_string(),
    );
    let importance = episodic_importance(&memory.category);

    let (id, _action) = if memory.category == TOOL_RESULT_CATEGORY {
        service
            .remember_deferred_vector(&memory.content, metadata, importance, workspace_id)
            .await?
    } else if let Some(workspace_id) = workspace_id {
        service
            .remember_scoped(&memory.content, metadata, importance, Some(workspace_id))
            .await?
    } else {
        service
            .remember_with_importance(&memory.content, metadata, importance)
            .await?
    };
    Ok(Some(id))
}

#[cfg(test)]
mod tests {
    use super::{
        ExtractedMemory, MemoryService, TOOL_RESULT_CATEGORY, episodic_importance,
        is_low_signal_memory, store_extracted_memory,
    };
    use nanna_agent::MemoryProvenance;
    use nanna_memory::{EmbedFn, MemoryServiceConfig};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The regression this filter exists to prevent: `loop_runner` rewrites an
    /// unsuccessful tool result to `format!("Error: {…}")`, and the old chat
    /// filter matched that substring anywhere in the body — so 100% of failed
    /// tool calls were unwritable. An agent that cannot recall its own failures
    /// repeats them.
    #[test]
    fn a_failed_tool_result_is_worth_remembering() {
        assert!(!is_low_signal_memory(
            "[exec → cargo test — FAILED] Error: Execution failed: 3 tests failed"
        ));
        assert!(!is_low_signal_memory(
            "Error: Command failed with exit status 1"
        ));
    }

    /// The second half of the same bug: a SUCCESSFUL call whose output merely
    /// mentions an error. `cat ./minidb` stored nothing, twice, because the
    /// script contains its own error strings.
    #[test]
    fn a_successful_result_that_quotes_an_error_is_worth_remembering() {
        assert!(!is_low_signal_memory(
            r#"[exec → cat ./minidb — ok] if [ $? -ne 0 ]; then echo "Error: no such table"; fi"#
        ));
    }

    /// The dropped "dominated by non-ASCII" test judged text by not being
    /// ASCII, so box-drawing output and every non-Latin script read as binary.
    #[test]
    fn box_drawing_and_non_latin_text_are_not_binary() {
        let tree = "[exec → tree — ok] ".to_string() + &"├── src\n│   └── main.rs\n".repeat(20);
        assert!(!is_low_signal_memory(&tree));
        assert!(!is_low_signal_memory(
            "[read_file → notes.txt — ok] 本番データベースを直接呼び出さないこと"
        ));
    }

    /// Positive space: real machine noise still goes.
    #[test]
    fn machine_noise_is_dropped() {
        assert!(is_low_signal_memory(""));
        assert!(is_low_signal_memory("   \n  "));
        assert!(is_low_signal_memory("HEARTBEAT_OK 12"));
        // C0 control bytes and replacement chars are what a lossy binary decode
        // actually looks like; 60 of either clears the 40-char noise bound.
        assert!(is_low_signal_memory(&"\u{0}".repeat(60)));
        assert!(is_low_signal_memory(&"\u{FFFD}".repeat(60)));
    }

    /// A short result is a result. The old chat filter dropped anything under
    /// 20 bytes, which is most of what a passing check prints.
    #[test]
    fn a_short_result_is_still_a_result() {
        assert!(!is_low_signal_memory("[exec → ls — ok] ok"));
    }

    /// The spread that makes recall ranking mean something.
    #[test]
    fn a_tool_result_never_outranks_a_stated_preference() {
        assert!(episodic_importance("tool_result") < episodic_importance("preference"));
        assert!(episodic_importance("tool_result") < episodic_importance("fact"));
        assert!((episodic_importance("identity") - episodic_importance("preference")).abs() < f32::EPSILON);
        assert!((episodic_importance("anything else") - 3.0).abs() < f32::EPSILON);
    }

    /// A constant embedder that counts consultations, so a test can assert
    /// which route a category took by whether the embedder was spent.
    fn counting_embed_fn(calls: Arc<AtomicUsize>) -> EmbedFn {
        Arc::new(move |_t: &str| {
            calls.fetch_add(1, Ordering::Relaxed);
            Box::pin(async { Ok(vec![1.0_f32, 0.0, 0.0]) })
        })
    }

    fn sink_fixture(calls: &Arc<AtomicUsize>) -> MemoryService {
        MemoryService::new(MemoryServiceConfig {
            dimension: 3,
            ..Default::default()
        })
        .with_embed_fn(counting_embed_fn(calls.clone()))
    }

    fn extracted(category: &str, content: &str) -> ExtractedMemory {
        ExtractedMemory {
            content: content.to_string(),
            category: category.to_string(),
            provenance: MemoryProvenance::Observed,
            tags: None,
        }
    }

    /// P24.3 part 3, at the seam both sinks share: a tool result's vector must
    /// not be bought on the turn's critical path, and everything else's still
    /// must be. Asserted by whether the embedder was consulted, because that
    /// consultation *is* the round-trip the defect was about.
    #[tokio::test]
    async fn only_a_tool_result_defers_its_vector() {
        let calls = Arc::new(AtomicUsize::new(0));
        let service = sink_fixture(&calls);

        let stored = store_extracted_memory(
            &service,
            extracted(TOOL_RESULT_CATEGORY, "[exec → cargo test — ok] 234 passed"),
            None,
        )
        .await
        .expect("a tool-result write must land");
        assert!(stored.is_some(), "the row must exist, only its vector waits");
        assert_eq!(calls.load(Ordering::Relaxed), 0, "no inline embed for a tool result");
        assert_eq!(service.take_queued_vector_count(), 1, "and its vector must be queued");

        let stored = store_extracted_memory(
            &service,
            extracted("preference", "the user prefers tabs over spaces"),
            None,
        )
        .await
        .expect("an ordinary write must land");
        assert!(stored.is_some());
        assert_eq!(calls.load(Ordering::Relaxed), 1, "an ordinary fact still embeds inline");
        assert_eq!(service.take_queued_vector_count(), 0, "and queues nothing");
    }

    /// The filter runs on BOTH routes. It used to live in each sink, and that
    /// is how the two drifted; a route that skipped it would reintroduce the
    /// same split by a different door.
    #[tokio::test]
    async fn machine_noise_is_dropped_on_the_deferred_route_too() {
        let calls = Arc::new(AtomicUsize::new(0));
        let service = sink_fixture(&calls);

        let stored =
            store_extracted_memory(&service, extracted(TOOL_RESULT_CATEGORY, "HEARTBEAT_OK 12"), None)
                .await
                .expect("filtering is not an error");
        assert!(stored.is_none(), "heartbeat chatter is not an observation");
        assert!(service.list_all().await.is_empty(), "and it must not be stored");
        assert_eq!(service.take_queued_vector_count(), 0, "nor queued");
    }

    /// A failed tool call is exactly what an agent must remember — and it takes
    /// the deferred route like any other tool result, so the 704-write loss
    /// cannot come back through the new branch.
    #[tokio::test]
    async fn a_failed_tool_result_still_reaches_the_store() {
        let calls = Arc::new(AtomicUsize::new(0));
        let service = sink_fixture(&calls);

        let stored = store_extracted_memory(
            &service,
            extracted(
                TOOL_RESULT_CATEGORY,
                "[exec → cargo test — FAILED] Error: Execution failed: 3 tests failed",
            ),
            None,
        )
        .await
        .expect("write");
        assert!(stored.is_some());
        let all = service.list_all().await;
        assert_eq!(all.len(), 1);
        assert!(all[0].content.contains("3 tests failed"));
    }
}
