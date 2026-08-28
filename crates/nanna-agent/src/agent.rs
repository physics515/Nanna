use crate::types::*;
use serde::{Deserialize, Serialize};

/// Internal result from LLM call
#[derive(Default)]
pub struct LlmResult {
    pub text: String,
    pub tool_uses: Vec<(String, String, Value)>,
    pub content_blocks: Vec<ContentBlock>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_creation_tokens: u32,
    /// Error tool results from malformed JSON parsing failures.
    /// These need to be sent back to the model so it knows the call failed.
    pub error_tool_results: Vec<ContentBlock>,
}

/// Detect a literal tool-call loop: the newest tool call repeated an earlier
/// call with the same tool, the same arguments, and the same result (P14).
///
/// Identical result twice means the environment did not change — repeating
/// the call cannot make progress. Text-level detectors miss this because the
/// surrounding narration usually varies.
///
/// **Per-key, not per-adjacency.** This used to compare only the two most
/// recent records, which any interleaving defeated: an A-B-A-B alternation
/// never puts two identical records side by side, so the nudge — the FIRST
/// rung of the escalation ladder every breaker threshold is derived from —
/// never fired at all. Observed live 2026-08-02 (ollama/gemma4:12b, session
/// 05775d1d): a 20-minute wedged turn alternating `explore` with `write_file`
/// rewriting one file, with no loop nudge in the entire turn. The streak that
/// matters is per call shape, so the lookback is per call shape too: find the
/// most recent EARLIER record with the same (name, input) key and compare its
/// output — exactly the rule the sibling breaker implements.
pub fn detect_literal_tool_loop(
    records: &[ToolCallRecord],
) -> Option<(&ToolCallRecord, &ToolCallRecord)> {
    let mut loop_detected = None;

    for (idx, record) in records.iter().enumerate() {
        if idx == 0 {
            continue; // Need at least two records to detect a loop
        }

        // Find the most recent earlier record with the same (name, input) key
        for earlier in records.iter().rev().take(100).skip(1) {
            if earlier.name == record.name && serde_json::to_string(&earlier.input) == serde_json::to_string(&record.input) {
                // Compare outputs — identical output means the environment didn't change
                if earlier.output == record.output {
                    loop_detected = Some((earlier, record));
                    break;
                }
            }
        }

        if let Some(detected) = loop_detected {
            return Some(detected);
        }
    }

    None
}
