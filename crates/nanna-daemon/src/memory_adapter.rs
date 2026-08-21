//! Adapter to bridge nanna-tools memory traits with nanna-memory MemoryService

use async_trait::async_trait;
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
        "tool_result" => 1.5,
        "preference" | "identity" => 4.0,
        "fact" | "insight" => 3.5,
        _ => 3.0,
    }
}

#[cfg(test)]
mod tests {
    use super::{episodic_importance, is_low_signal_memory};

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
}
