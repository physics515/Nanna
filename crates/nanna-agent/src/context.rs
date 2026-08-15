//! Agent context management

use crate::chunker::Chunk;
use nanna_llm::{estimate_tokens, estimate_tokens_for_family, AnthropicMessage, AnthropicRequest, ContentBlock, LlmClient, ModelInfo, RequestBuilder, TokenContentFamily};
use nanna_workspace::{Workspace, WorkspaceFiles};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Minimum content size (chars) to consider for deduplication
const DEDUP_MIN_SIZE: usize = 4_000; // Lowered since CDC handles small chunks well

/// Threshold for considering content "mostly duplicate" (0.0-1.0)
const DEDUP_THRESHOLD: f32 = 0.7;

/// Calculate deduplication coverage: fraction of chunks whose hashes are already known.
fn dedup_coverage(content: &str, known_hashes: &HashSet<u64>) -> f32 {
    let chunks = Chunk::split_on_boundaries(content);
    if chunks.is_empty() {
        return 0.0;
    }
    let known_count = chunks.iter().filter(|c| known_hashes.contains(&c.hash)).count();
    known_count as f32 / chunks.len() as f32
}

/// Get chunk hashes for content (for dedup tracking after summarization).
fn chunk_and_hash(content: &str) -> Vec<u64> {
    Chunk::split_on_boundaries(content)
        .into_iter()
        .map(|c| c.hash)
        .collect()
}

/// Estimate one message's token cost — the per-block heuristic shared by
/// [`AgentContext::estimate_tokens`] (whole transcript) and
/// [`AgentContext::step_frame_tokens`] (the irreducible first message).
fn estimate_message_tokens(msg: &AnthropicMessage) -> usize {
    msg.content
        .iter()
        .map(|c| match c {
            ContentBlock::Text { text } => estimate_tokens(text),
            ContentBlock::ToolUse { input, .. } => {
                estimate_tokens_for_family(&input.to_string(), TokenContentFamily::Code) + 50
            }
            ContentBlock::ToolResult { content, .. } => estimate_tokens(content) + 20,
            ContentBlock::Image { .. } => 1000, // Images are ~1k tokens
            ContentBlock::Thinking { thinking, .. } => estimate_tokens(thinking),
        })
        .sum()
}

/// Whether a model-produced summary is plausible enough to stand in for the
/// original content. Small models sometimes return degenerate output — empty
/// text, "...", or a bare title — which, if accepted, silently REPLACES real
/// data (observed live 2026-07-10: an 80 KB tool result "summarized" to
/// 17 chars). Reject anything too short in absolute terms or relative to the
/// source; the caller then tries the next model or falls back to truncation,
/// which at least preserves a real prefix of the data.
#[must_use]
pub fn plausible_summary(summary: &str, source_len: usize) -> bool {
    let len = summary.trim().len();
    if source_len < 1_000 {
        // Tiny sources can have legitimately tiny summaries.
        return len > 0;
    }
    // At least 64 chars, and at least 0.1% of the source.
    len >= 64 && len.saturating_mul(1_000) >= source_len
}

/// Decide whether Tier-1 proactive compression should fire, from MEASURED
/// headroom rather than a fixed fraction of the threshold.
///
/// Fires when the run's own observed growth says the NEXT interval could
/// cross `compression_threshold`: once `estimated_tokens +
/// max_observed_growth` exceeds it, waiting one more interval risks entering
/// the standard tier mid-step. Above the threshold the standard tier owns the
/// problem, so this returns false there (the same band the ladder always
/// gave Tier 1).
///
/// `max_observed_growth == 0` means no growth has been measured yet — there
/// is no evidence to act on, and proactive compression stays quiet. The rule
/// this replaces (fire past 40% of the threshold, tuned for 200k windows)
/// fired 80× at 4,423 tokens on a 16,384-token window with ~3.7k tokens of
/// real headroom still free, each firing shrinking live working context.
#[must_use]
pub fn proactive_compression_due(
    estimated_tokens: usize,
    max_observed_growth: usize,
    compression_threshold: usize,
) -> bool {
    max_observed_growth > 0
        && estimated_tokens <= compression_threshold
        && estimated_tokens + max_observed_growth > compression_threshold
}

/// Live measurement of how fast a context grows between compression-ladder
/// checks. [`proactive_compression_due`] derives the Tier-1 trigger from the
/// largest growth ever observed, so the trigger scales with the actual
/// workload and window instead of a constant tuned for one window size.
///
/// The baseline is recorded AFTER each ladder pass (post-compression) and
/// the delta measured at the next ladder ENTRY, so one observation spans
/// exactly one loop interval — the model response, its tool results, and any
/// injected notices: the growth the next interval could plausibly repeat.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextGrowthTracker {
    /// Token estimate recorded at the end of the previous ladder pass.
    pub last_observed_tokens: Option<usize>,
    /// Largest single-interval growth observed so far this run.
    pub max_observed_growth: usize,
}

impl ContextGrowthTracker {
    /// Record the estimate at ladder entry. Returns the growth since the
    /// previous baseline (0 before the first baseline exists — no evidence,
    /// no trigger). Shrinkage never records: compression between baseline
    /// and observation only makes the delta smaller, so the max is an
    /// under-estimate of true growth, never an over-estimate.
    pub fn observe(&mut self, estimated_tokens: usize) -> usize {
        let growth = self
            .last_observed_tokens
            .map_or(0, |prev| estimated_tokens.saturating_sub(prev));
        if growth > self.max_observed_growth {
            self.max_observed_growth = growth;
        }
        growth
    }

    /// Re-baseline after the ladder ran (post-compression), so the next
    /// observation measures only NEW material, not compression's effect.
    pub const fn rebaseline(&mut self, estimated_tokens: usize) {
        self.last_observed_tokens = Some(estimated_tokens);
    }
}

/// One fact proven by execution: a command that ran to a definite exit
/// status at a known time. Held in [`AgentContext::verified_outcomes`] — the
/// never-compressed slot — because these are exactly the facts whose loss
/// turns a model against its own passing work (observed live 2026-08-10: a
/// summarization pass collapsed the record of ten just-verified commands and
/// the model's next move was a from-scratch rewrite over them).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedOutcome {
    /// What ran — the command line, verbatim.
    pub subject: String,
    /// The verdict the environment returned (e.g. "exit 0").
    pub outcome: String,
    /// Unix seconds of the LATEST execution asserting this outcome, or `0`
    /// when the source that supplied the fact recorded no time (a verdict read
    /// back from the store whose completion timestamp is missing). Rendered as
    /// "time not recorded" rather than as the epoch — a slot whose whole
    /// contract is exactness must not invent a moment.
    pub verified_at: i64,
    /// How many executions have asserted this exact (subject, outcome).
    pub times: u32,
}

/// Chars of a subject shown per verified-outcome line. Identification, not
/// reproduction: the full command already lives in the transcript and tool
/// records; a slot line only needs enough to name the fact unambiguously,
/// and the elision marker carries the hidden length so nothing is silently
/// truncated. 120 holds a full typical test invocation (a script path, a
/// cargo test filter) with room to spare, in line with the preview widths
/// the transcript already uses elsewhere (80–200 chars).
const VERIFIED_SUBJECT_PREVIEW_CHARS: usize = 120;

/// First line of `subject`, capped for display, with an elision marker
/// naming how many chars are not shown ("one line per outcome" — the slot's
/// unit is a line, so newlines never render).
fn verified_subject_preview(subject: &str) -> String {
    let first_line = subject.lines().next().unwrap_or("");
    let end = first_line.floor_char_boundary(VERIFIED_SUBJECT_PREVIEW_CHARS.min(first_line.len()));
    let shown = &first_line[..end];
    let hidden = subject.len() - shown.len();
    if hidden == 0 {
        shown.to_string()
    } else {
        format!("{shown} …[+{hidden} chars]")
    }
}

/// Compose the in-context announcement for a summarization failure that
/// forced messages to be dropped un-summarized.
///
/// Rule: every truncation artifact must say WHAT was lost, WHY, and that the
/// operations themselves SUCCEEDED — an unannounced gap reads as corruption
/// and seeds restart-from-scratch spirals.
#[must_use]
pub fn summarization_failure_notice(dropped_messages: usize, reason: &str) -> String {
    format!(
        "[CONTEXT NOTICE — history shortened WITHOUT summarization]\n\
         WHAT: {dropped_messages} older conversation message(s) were dropped \
         from your in-memory context with no summary standing in for them.\n\
         WHY: {reason}.\n\
         Disk is unaffected: every file write and command in the dropped \
         messages already ran and SUCCEEDED unless it said otherwise at the \
         time. Files on disk are the ground truth — re-read them if unsure; \
         do NOT restart or rewrite work just because history looks short."
    )
}

/// How far [`AgentContext::enforce_limits_with_summarization`] should summarize.
///
/// The loop used to test `exceeds_hard_limit()` unconditionally, which made it
/// a no-op for its own tier-2 caller: that tier is entered on
/// `needs_compression() && !exceeds_hard_limit()`, so the predicate was false
/// on entry, zero iterations ran, and `Ok(0)` returned after the log line had
/// already announced that summarization was happening. The band between the
/// compression threshold and the hard limit was therefore never summarized —
/// harmless when a model reported a 32k window and the band was a couple of
/// thousand tokens, and decidedly not once models report their real windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummarizationTarget {
    /// Summarize until back under the proactive compression threshold (tier 2).
    CompressionThreshold,
    /// Summarize until back under the hard input limit (tier 3).
    HardLimit,
}

/// Configuration for context summarization
#[derive(Debug, Clone, Default)]
pub struct ContextSummarizationConfig {
    /// Model priority list for summarization (e.g., ["ollama/llama3.2", "anthropic/claude-3-haiku"])
    pub model_priority: Vec<String>,
    /// Ollama URL if using ollama models
    pub ollama_url: Option<String>,
    /// Maximum iterations to prevent infinite loops
    pub max_iterations: usize,
    /// OpenRouter API key (for "openrouter/" prefixed models)
    pub openrouter_api_key: Option<String>,
    /// OpenAI API key (for "openai/" prefixed models)
    pub openai_api_key: Option<String>,
}

impl ContextSummarizationConfig {
    pub fn new(model_priority: Vec<String>) -> Self {
        Self {
            model_priority,
            ollama_url: Some("http://localhost:11434".to_string()),
            max_iterations: 20,
            openrouter_api_key: None,
            openai_api_key: None,
        }
    }

    pub fn with_ollama_url(mut self, url: impl Into<String>) -> Self {
        self.ollama_url = Some(url.into());
        self
    }
}

/// Compressed context summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSummary {
    /// The compressed summary text
    pub summary: String,
    /// Number of messages that were compressed
    pub messages_compressed: usize,
    /// Approximate tokens saved
    pub tokens_saved: usize,
    /// When the summary was created
    pub created_at: i64,
}

/// Context isolation mode for sub-agents
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContextIsolation {
    /// Full context inherited from parent
    #[default]
    Full,
    /// Only system prompt inherited
    SystemOnly,
    /// Summary of parent context provided
    Summary,
    /// Completely isolated (fresh context)
    Isolated,
}

/// Context for an agent session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContext {
    /// Session identifier
    pub session_id: String,
    /// System prompt
    pub system_prompt: String,
    /// Conversation history (Anthropic format)
    pub messages: Vec<AnthropicMessage>,
    /// Session metadata
    pub metadata: HashMap<String, String>,
    /// Maximum number of messages to keep
    pub max_messages: usize,
    /// Compressed summaries of older context
    #[serde(default)]
    pub summaries: Vec<ContextSummary>,
    /// Maximum tokens before compression triggers
    #[serde(default = "default_compression_threshold")]
    pub compression_threshold: usize,
    /// Hard limit on input tokens (model's context window minus output tokens)
    #[serde(default = "default_hard_limit")]
    pub hard_limit: usize,
    /// Current model ID (for tracking model changes)
    #[serde(default)]
    pub current_model: Option<String>,
    /// Parent context ID (if this is a sub-agent)
    #[serde(default)]
    pub parent_context_id: Option<String>,
    /// How much context was inherited from parent
    #[serde(default)]
    pub isolation_mode: Option<String>,
    /// Context budget in tokens for sub-agents (limits how much context can be used)
    #[serde(default)]
    pub context_budget: Option<usize>,
    /// Workspace root path (if workspace is active)
    #[serde(default)]
    pub workspace_root: Option<PathBuf>,
    /// Workspace context (injected into system prompt)
    #[serde(default)]
    pub workspace_context: Option<String>,
    /// Deprecated no-op (memory is DB-backed; kept for serde compat).
    #[serde(default = "default_include_memory", alias = "include_workspace_memory")]
    pub include_workspace_memory: bool,
    /// Consolidated summary of all previously summarized messages.
    /// This is prepended to messages when building API requests.
    #[serde(default)]
    pub consolidated_summary: Option<String>,
    /// Rolling distilled-facts note (progressive distillation's output), in
    /// its OWN slot so the distiller can replace it wholesale without
    /// touching `consolidated_summary`. Distillation used to overwrite the
    /// consolidated summary with ≤512 tokens of facts about the last ten
    /// messages, destroying the only record of everything summarized before
    /// it (observed live 2026-08-10: 2571→934 chars immediately before
    /// verified-passing work was rewritten from scratch).
    #[serde(default)]
    pub distilled_facts: Option<String>,
    /// Facts proven by execution (command, exit status, when) — the
    /// never-compressed slot. Rendered into every request after the summary
    /// (see [`Self::messages_for_request`]) and never handed to any
    /// summarizer, so no summarization pass can drop one. Appended by
    /// [`Self::record_verified_outcome`]; no code path removes entries.
    /// Bound: one entry per distinct (subject, outcome) pair this run
    /// actually executed — the slot grows strictly slower than the work
    /// feeding it, since every entry costs at least one real command
    /// execution.
    #[serde(default)]
    pub verified_outcomes: Vec<VerifiedOutcome>,
    /// Live growth measurement feeding [`proactive_compression_due`].
    #[serde(default)]
    pub growth: ContextGrowthTracker,
    /// In-context loss announcements, composed where the loss happens (deep
    /// in summarization fallbacks) and drained by the agent loop AFTER its
    /// compression ladder so compression cannot drop its own announcement.
    #[serde(default)]
    pending_loss_notices: Vec<String>,
    /// Hashes of content that has been summarized (for deduplication).
    /// If new messages contain content matching these hashes, we skip it
    /// since it's already represented in the consolidated_summary.
    #[serde(default)]
    summarized_content_hashes: HashSet<u64>,
}

fn default_include_memory() -> bool {
    true
}

fn default_compression_threshold() -> usize {
    nanna_llm::unknown_model_info("", "").compression_threshold()
}

fn default_hard_limit() -> usize {
    nanna_llm::unknown_model_info("", "").hard_input_limit()
}

impl AgentContext {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            system_prompt: String::new(),
            messages: Vec::new(),
            metadata: HashMap::new(),
            max_messages: 100,
            summaries: Vec::new(),
            compression_threshold: default_compression_threshold(),
            hard_limit: default_hard_limit(),
            current_model: None,
            parent_context_id: None,
            isolation_mode: None,
            context_budget: None,
            workspace_root: None,
            workspace_context: None,
            include_workspace_memory: true,
            consolidated_summary: None,
            distilled_facts: None,
            verified_outcomes: Vec::new(),
            growth: ContextGrowthTracker::default(),
            pending_loss_notices: Vec::new(),
            summarized_content_hashes: HashSet::new(),
        }
    }

    /// Get messages for API request, prepending consolidated summary if present.
    ///
    /// This is the key method for incremental summarization:
    /// - If we have a consolidated summary, it's injected as a "context" user message
    /// - Large content blocks that match previously summarized content are deduplicated
    /// - This way, previously summarized content is included without re-processing
    /// - Final messages are sanitized to remove empty text blocks (Anthropic rejects them)
    pub fn messages_for_request(&self) -> Vec<AnthropicMessage> {
        // Deduplicate messages if we have summarized content hashes
        let deduped_messages = if self.summarized_content_hashes.is_empty() {
            self.messages.clone()
        } else {
            self.deduplicate_messages()
        };

        let raw = if let Some(preamble) = self.context_preamble() {
            let mut messages = Vec::with_capacity(deduped_messages.len() + 2);
            messages.push(AnthropicMessage::user_text(preamble));

            // Add a placeholder assistant acknowledgment to maintain user/assistant alternation
            messages.push(AnthropicMessage::assistant_text(
                "I understand the previous context. How can I help you continue?",
            ));

            // Then add deduplicated current messages
            messages.extend(deduped_messages);
            messages
        } else {
            // No preamble, just return (possibly deduplicated) messages
            deduped_messages
        };

        // Sanitize: remove empty text blocks and ensure every message has content
        Self::sanitize_messages(raw)
    }

    /// The injected first-message preamble: the consolidated summary and/or
    /// distilled facts inside `<previous_context>` framing, then the
    /// verified-outcomes slot OUTSIDE that framing — the summary framing
    /// declares itself lossy shorthand, and the slot is exact fact that must
    /// not inherit the disclaimer.
    ///
    /// The framing must state what a summary is NOT: the model has been
    /// observed reading compressed context as evidence that work was lost or
    /// files corrupted, then restarting from scratch.
    fn context_preamble(&self) -> Option<String> {
        let mut lossy = String::new();
        if let Some(ref summary) = self.consolidated_summary {
            lossy.push_str(summary);
        }
        if let Some(ref facts) = self.distilled_facts {
            if !lossy.is_empty() {
                lossy.push_str("\n\n");
            }
            lossy.push_str("[DISTILLED FACTS]\n");
            lossy.push_str(facts);
        }

        let mut sections: Vec<String> = Vec::new();
        if !lossy.is_empty() {
            sections.push(format!(
                "<previous_context>\nThe following is a COMPRESSED SUMMARY of earlier \
                 conversation. WHY: the conversation grew longer than your context window \
                 can hold, so older messages were condensed to make room to keep working. \
                 Everything it describes already happened and SUCCEEDED unless it explicitly \
                 says otherwise — no work was lost. It is lossy shorthand, not literal \
                 messages: files on disk and recent tool results are the ground truth over \
                 anything here.\n\n{lossy}\n</previous_context>"
            ));
        }
        if let Some(block) = self.verified_outcomes_block() {
            sections.push(block);
        }

        if sections.is_empty() {
            None
        } else {
            Some(sections.join("\n\n"))
        }
    }

    /// Render the verified-outcomes slot, one line per outcome.
    ///
    /// Losslessness: every recorded execution is represented — a new
    /// (subject, outcome) pair appends a line; an identical re-execution
    /// increments that line's count and refreshes its timestamp (a reword,
    /// never a drop). No code path removes a line, so the asserted facts
    /// only accumulate. Bound: lines ≤ distinct (subject, outcome) pairs ≤
    /// executions this run actually performed — each line costs at least one
    /// real command execution, so the slot grows strictly slower than the
    /// work feeding it.
    #[must_use]
    pub fn verified_outcomes_block(&self) -> Option<String> {
        if self.verified_outcomes.is_empty() {
            return None;
        }
        let mut block = String::from(
            "<verified_outcomes>\nFacts proven by EXECUTION during this session — each \
             line is a command that actually ran and the exit status the environment \
             returned. This list is exact (not a summary), is never compressed, and \
             outlives every summarization pass. Trust it over any summary above; do NOT \
             re-do or rewrite work these lines already prove.\n",
        );
        for outcome in &self.verified_outcomes {
            let when = if outcome.verified_at <= 0 {
                "time not recorded".to_string()
            } else {
                chrono::DateTime::from_timestamp(outcome.verified_at, 0)
                    .map_or_else(|| outcome.verified_at.to_string(), |t| t.to_rfc3339())
            };
            block.push_str(&format!(
                "- {} → {} (×{}, last verified {})\n",
                verified_subject_preview(&outcome.subject),
                outcome.outcome,
                outcome.times,
                when,
            ));
        }
        block.push_str("</verified_outcomes>");
        Some(block)
    }

    /// Record a fact proven by execution into the never-compressed slot.
    ///
    /// The same (subject, outcome) collapses into its existing line (count
    /// and latest timestamp refresh — a reword, never a drop); a different
    /// outcome for the same subject appends its OWN line, so a later
    /// regression never erases the record of an earlier pass.
    pub fn record_verified_outcome(
        &mut self,
        subject: impl Into<String>,
        outcome: impl Into<String>,
    ) {
        self.record_verified_outcome_at(subject, outcome, chrono_timestamp());
    }

    /// [`Self::record_verified_outcome`] for a fact whose verification time is
    /// KNOWN and is not now — a verdict read back from the store when a fresh
    /// per-step context is seeded (the do-not-regress digest).
    ///
    /// The collapse rule is identical, and the timestamp is monotone: a
    /// re-assertion never moves a line's "last verified" backwards, so seeding
    /// an older verdict beside a fresh execution of the same fact cannot make
    /// the newer evidence look stale. `verified_at <= 0` means "time not
    /// recorded" and renders as such — never as the epoch.
    pub fn record_verified_outcome_at(
        &mut self,
        subject: impl Into<String>,
        outcome: impl Into<String>,
        verified_at: i64,
    ) {
        let subject = subject.into();
        let outcome = outcome.into();
        if let Some(existing) = self
            .verified_outcomes
            .iter_mut()
            .find(|o| o.subject == subject && o.outcome == outcome)
        {
            existing.times = existing.times.saturating_add(1);
            existing.verified_at = existing.verified_at.max(verified_at);
        } else {
            self.verified_outcomes.push(VerifiedOutcome {
                subject,
                outcome,
                verified_at,
                times: 1,
            });
        }
    }

    /// Replace the rolling distilled-facts note. Replacement is the
    /// distiller's contract (it re-reads recent messages every round);
    /// keeping the note OUT of `consolidated_summary` is what makes that
    /// safe — summarization products and drop notes survive every
    /// distillation round instead of being overwritten by it.
    pub fn set_distilled_facts(&mut self, facts: impl Into<String>) {
        self.distilled_facts = Some(facts.into());
    }

    /// Queue an in-context loss announcement (see
    /// [`summarization_failure_notice`]). No-op when nothing was dropped —
    /// an announcement of zero loss would be noise. Drained by the agent
    /// loop AFTER its compression ladder so the announcement cannot itself
    /// be compressed away in the same pass.
    pub fn push_summarization_failure_notice(&mut self, dropped_messages: usize, reason: &str) {
        if dropped_messages == 0 {
            return;
        }
        self.pending_loss_notices
            .push(summarization_failure_notice(dropped_messages, reason));
    }

    /// Take (and clear) the queued loss announcements.
    pub fn take_pending_loss_notices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_loss_notices)
    }

    /// Remove empty text blocks from messages and ensure every message has at least one content block.
    /// Anthropic API rejects requests with empty text content blocks.
    fn sanitize_messages(messages: Vec<AnthropicMessage>) -> Vec<AnthropicMessage> {
        messages
            .into_iter()
            .map(|mut msg| {
                // Remove empty text blocks
                msg.content.retain(|block| {
                    !matches!(block, ContentBlock::Text { text } if text.is_empty())
                });
                // Ensure message has at least one content block
                if msg.content.is_empty() {
                    msg.content.push(ContentBlock::Text {
                        text: "[No content]".to_string(),
                    });
                }
                msg
            })
            .collect()
    }

    /// Deduplicate messages by replacing large content blocks that were already summarized.
    ///
    /// Uses content-defined chunking (CDC) to detect partial duplicates - even if
    /// content is split differently, overlapping chunks will be detected.
    fn deduplicate_messages(&self) -> Vec<AnthropicMessage> {
        let mut dedup_count = 0;
        let mut bytes_saved = 0;
        let mut deduped = Vec::with_capacity(self.messages.len());

        for msg in &self.messages {
            let mut new_content = Vec::with_capacity(msg.content.len());

            for block in &msg.content {
                match block {
                    ContentBlock::Text { text } if text.len() >= DEDUP_MIN_SIZE => {
                        // Check CDC coverage - what percentage of chunks are already known?
                        let coverage = dedup_coverage(text, &self.summarized_content_hashes);

                        if coverage >= DEDUP_THRESHOLD {
                            // Most of this content was already summarized
                            new_content.push(ContentBlock::Text {
                                text: format!(
                                    "[Content ({:.0}% duplicate) already included in previous context summary]",
                                    coverage * 100.0
                                ),
                            });
                            dedup_count += 1;
                            bytes_saved += text.len();
                            debug!(
                                coverage = format!("{:.1}%", coverage * 100.0),
                                original_len = text.len(),
                                "Deduplicated previously summarized content via CDC"
                            );
                        } else if coverage > 0.0 {
                            // Partial overlap - keep full content but log it
                            debug!(
                                coverage = format!("{:.1}%", coverage * 100.0),
                                original_len = text.len(),
                                "Partial duplicate detected, keeping full content"
                            );
                            new_content.push(block.clone());
                        } else {
                            new_content.push(block.clone());
                        }
                    }
                    ContentBlock::ToolResult { tool_use_id, content, is_error }
                        if content.len() >= DEDUP_MIN_SIZE =>
                    {
                        let coverage = dedup_coverage(content, &self.summarized_content_hashes);

                        if coverage >= DEDUP_THRESHOLD {
                            new_content.push(ContentBlock::ToolResult {
                                tool_use_id: tool_use_id.clone(),
                                content: format!(
                                    "[Output ({:.0}% duplicate) already included in previous context summary]",
                                    coverage * 100.0
                                ),
                                is_error: *is_error,
                            });
                            dedup_count += 1;
                            bytes_saved += content.len();
                            debug!(
                                coverage = format!("{:.1}%", coverage * 100.0),
                                original_len = content.len(),
                                "Deduplicated previously summarized tool result via CDC"
                            );
                        } else {
                            new_content.push(block.clone());
                        }
                    }
                    _ => {
                        new_content.push(block.clone());
                    }
                }
            }

            deduped.push(AnthropicMessage {
                role: msg.role.clone(),
                content: new_content,
            });
        }

        if dedup_count > 0 {
            info!(
                dedup_count = dedup_count,
                bytes_saved = bytes_saved,
                "Deduplicated content blocks using CDC"
            );
        }

        deduped
    }

    /// Estimate tokens for messages that will be sent to API (includes summary).
    ///
    /// Uses a conservative ratio of ~3.2 chars per token (instead of 4) because
    /// code-heavy content, JSON, and tool calls tokenize at a higher ratio.
    /// Over-estimating is safer than under-estimating (summarize early > 400 error).
    pub fn estimate_request_tokens(&self) -> usize {
        let summary_tokens = self
            .consolidated_summary
            .as_ref()
            .map(|s| estimate_token_count(s.len()) + 100) // framing overhead
            .unwrap_or(0);
        // The distilled-facts and verified-outcomes slots ride in the same
        // injected preamble — unbudgeted tokens would overflow the window.
        let distilled_tokens = self
            .distilled_facts
            .as_ref()
            .map_or(0, |s| estimate_token_count(s.len()) + 10);
        let verified_tokens = self
            .verified_outcomes_block()
            .map_or(0, |b| estimate_token_count(b.len()));

        summary_tokens + distilled_tokens + verified_tokens + self.estimate_tokens()
    }

    /// Set the system prompt.
    #[must_use]
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = prompt.into();
        self
    }

    /// Set the compression threshold
    #[must_use]
    pub fn with_compression_threshold(mut self, threshold: usize) -> Self {
        self.compression_threshold = threshold;
        self
    }

    /// Set the hard limit
    #[must_use]
    pub fn with_hard_limit(mut self, limit: usize) -> Self {
        self.hard_limit = limit;
        self
    }

    /// Configure context limits based on model capabilities.
    ///
    /// This should be called when:
    /// - Starting a new session
    /// - Switching to a different model
    /// - After fetching updated model info from the API
    ///
    /// Returns true if the model changed (and limits were updated).
    ///
    /// Static-reserve variant for callers with no request budget in hand;
    /// prefer [`Self::configure_for_model_with_output`] when the per-request
    /// `max_tokens` is known — the output reserve then shrinks to what the
    /// request can actually generate, freeing the rest of the window for
    /// input.
    pub fn configure_for_model(&mut self, model_info: &ModelInfo) -> bool {
        self.configure_for_model_with_output(
            model_info,
            model_info
                .max_output_tokens
                .min(model_info.context_window / 4),
        )
    }

    /// Configure context limits from model capabilities AND the caller's
    /// actual per-request output budget. The reserve is derived via
    /// [`ModelInfo::effective_output_budget`], so input + output can never
    /// over-commit the window, and a small-output agent (e.g. 2k
    /// `max_tokens` on a 32k model) keeps ~94% of the window for input.
    pub fn configure_for_model_with_output(
        &mut self,
        model_info: &ModelInfo,
        requested_max_output_tokens: usize,
    ) -> bool {
        let reserve = model_info.effective_output_budget(requested_max_output_tokens);
        let model_changed = self.current_model.as_ref() != Some(&model_info.id);

        if model_changed {
            info!(
                model = %model_info.id,
                context_window = model_info.context_window,
                output_reserve = reserve,
                compression_threshold = model_info.compression_threshold_for(reserve),
                hard_limit = model_info.hard_input_limit_for(reserve),
                "Configuring context for model"
            );
        }

        self.compression_threshold = model_info.compression_threshold_for(reserve);
        self.hard_limit = model_info.hard_input_limit_for(reserve);
        self.current_model = Some(model_info.id.clone());

        model_changed
    }

    /// Configure context limits for a model by name.
    ///
    /// Prefer [`Self::configure_for_model`] with a live [`ModelInfo`] from the
    /// provider API. This name-only path uses the on-disk model-info cache when
    /// a previous API fetch stored windows, otherwise the universal floor in
    /// [`nanna_llm::unknown_model_info`] — **no per-model name table**.
    pub fn configure_for_model_name(&mut self, model: &str) {
        // Cache-or-universal-floor only — no per-model name table.
        // Prefer configure_for_model(&ModelInfo) once the provider has been queried.
        let info = nanna_llm::model_info_from_cache_or_unknown(model, "");
        let _ = self.configure_for_model(&info);
        // Force current_model to the given name (cache may use a stripped id).
        self.current_model = Some(model.to_string());
        debug_assert!(
            self.compression_threshold <= self.hard_limit,
            "compression must trigger before the hard input cap"
        );
    }

    /// Set the context budget in tokens
    #[must_use]
    pub fn with_context_budget(mut self, budget: usize) -> Self {
        self.context_budget = Some(budget);
        self
    }

    /// Set workspace root and load workspace context
    #[must_use]
    pub fn with_workspace(mut self, workspace: &Workspace) -> Self {
        self.workspace_root = Some(workspace.root.clone());
        self.workspace_context = Some(workspace.system_context());
        self
    }

    /// Set workspace from files directly
    #[must_use]
    pub fn with_workspace_files(mut self, root: PathBuf, files: &WorkspaceFiles) -> Self {
        self.workspace_root = Some(root);
        self.workspace_context = Some(files.to_system_context());
        self
    }

    /// Deprecated no-op kept for call-site compatibility.
    #[must_use]
    pub fn with_workspace_memory(self, _include: bool) -> Self {
        self
    }

    /// Get the effective system prompt (base + workspace context).
    ///
    /// The workspace slice is BOUNDED by [`Self::workspace_context_cap_chars`]:
    /// observed live, a workspace injected 330k chars (~60k tokens) into a 32k
    /// window — truncation can't shrink the system prompt, so Ollama clipped
    /// the prompt head and the model lost its own tool definitions, reducing
    /// it to narrated/confabulated tool calls. An oversized workspace context
    /// is cut at the cap with a visible marker instead.
    /// The "you are here" line prepended to the workspace slice.
    ///
    /// Without it the model is never told where it is working: the system
    /// prompt says nothing about a working directory and the workspace slice
    /// carries only file *contents*, so the only way to learn the path is to
    /// run `pwd` and reverse-engineer it. Observed live 2026-07-26: a model
    /// did exactly that, then addressed files as `<workspace-leaf>/minidb`
    /// — relative to a directory that already WAS the workspace — and spent
    /// an hour editing a shadow copy one directory deeper while the tests
    /// measured the real file.
    ///
    /// So: state the directory, and state that bare relative paths land in
    /// it. The tools accept absolute paths too (and repair a redundantly
    /// prefixed relative path), but not having to guess is cheaper than
    /// recovering from a wrong guess.
    fn workdir_preamble(&self) -> Option<String> {
        let root = self.workspace_root.as_ref()?;
        // The shell contract has to be stated. `exec` runs POSIX sh (Git Bash
        // on Windows), but the working directory is shown in native form, and
        // a Windows-looking path invites cmd.exe syntax. Observed live
        // 2026-07-27: a run drifted from `ls`/`cat` into `dir`, `findstr`,
        // `type` and `for %%f in (...)`, and 39 exec calls failed — the model
        // had no way to know which shell it was talking to.
        let shell_note = if cfg!(windows) {
            "`exec` runs POSIX **sh** (via Git Bash), NOT cmd.exe or PowerShell — \
             use `ls`, `cat`, `grep`, `sh script.sh`; `dir`, `type`, `findstr` and \
             `%%VAR%%` will fail. In shell commands this directory is also reachable \
             as a POSIX path."
        } else {
            "`exec` runs POSIX sh."
        };
        Some(format!(
            "# Working directory\n\nYou are working in `{}`.\n\nRelative paths in \
             tool calls resolve against this directory, so use `./minidb` or \
             `tests/test_01.sh` — do NOT prefix them with the directory's own name. \
             Absolute paths are accepted as well.\n\n{shell_note}",
            root.display()
        ))
    }

    #[must_use]
    pub fn effective_system_prompt(&self) -> String {
        // The working-directory line is small, fixed-size, and must never be
        // the thing that gets truncated, so it is joined AFTER the bounded
        // workspace slice is assembled rather than counted against its cap.
        // Placed LAST: identity and project context lead, and the concrete
        // "where you are" sits closest to the conversation that acts on it.
        let with_workdir = |body: String| -> String {
            match self.workdir_preamble() {
                Some(pre) if body.is_empty() => pre,
                Some(pre) => format!("{body}\n\n{pre}"),
                None => body,
            }
        };
        let base = match &self.workspace_context {
            Some(ws_ctx) if !ws_ctx.is_empty() => {
                let cap = self.workspace_context_cap_chars();
                let bounded: std::borrow::Cow<'_, str> = if ws_ctx.len() > cap {
                    // Cut on a char boundary at/below the cap, keep the head
                    // (README/AGENTS lead; deep ROADMAP prose is the bulk).
                    let mut cut = cap;
                    while cut > 0 && !ws_ctx.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    std::borrow::Cow::Owned(format!(
                        "{}\n\n[workspace context truncated: {} of {} chars shown — \
                         the full files are on disk; read them with read_file if needed]",
                        &ws_ctx[..cut],
                        cut,
                        ws_ctx.len()
                    ))
                } else {
                    std::borrow::Cow::Borrowed(ws_ctx.as_str())
                };
                if self.system_prompt.is_empty() {
                    bounded.into_owned()
                } else {
                    format!("{}\n\n{}", self.system_prompt, bounded)
                }
            }
            _ => self.system_prompt.clone(),
        };
        with_workdir(base)
    }

    /// Maximum chars of workspace context to inject: a quarter of the model's
    /// hard input limit, converted at ~4 chars/token — the ×4 (chars/token)
    /// and ÷4 (25% share) cancel, so the cap in chars equals `hard_limit`.
    /// Derived from the live window (not a magic constant): system prompt,
    /// tools, and history must all fit under `hard_limit`, so the workspace
    /// slice may claim at most a quarter of it. Floor of 2_000 chars keeps
    /// tiny windows functional.
    #[must_use]
    pub fn workspace_context_cap_chars(&self) -> usize {
        self.hard_limit.max(2_000)
    }

    /// Reload workspace context from disk
    ///
    /// # Errors
    /// Returns error if workspace cannot be loaded
    pub async fn reload_workspace(&mut self) -> Result<(), nanna_workspace::WorkspaceError> {
        if let Some(ref root) = self.workspace_root {
            let files = WorkspaceFiles::load(root).await;
            self.workspace_context = Some(files.to_system_context());
        }
        Ok(())
    }

    /// Allocate a portion of context budget to a sub-agent.
    ///
    /// Divides the available budget among multiple sub-agents, with the option
    /// to give priority to earlier agents (lower index gets slightly more).
    ///
    /// # Arguments
    /// * `num_agents` - Total number of sub-agents to allocate for
    /// * `agent_index` - Index of this agent (0-based)
    ///
    /// # Returns
    /// The allocated budget in tokens for this sub-agent.
    /// Returns a default of 10,000 tokens if no budget is set.
    #[must_use]
    pub fn allocate_budget(&self, num_agents: usize, agent_index: usize) -> usize {
        let total_budget = self.context_budget.unwrap_or(100_000);

        if num_agents == 0 {
            return total_budget;
        }

        // Reserve 20% for coordination/aggregation overhead
        let distributable = (total_budget * 80) / 100;

        // Base allocation per agent
        let base_per_agent = distributable / num_agents;

        // Give slightly more to earlier agents (they often do foundational work)
        // This creates a gentle gradient: first agent gets ~10% more than last
        let priority_bonus = if num_agents > 1 {
            let remaining_priority = (distributable * 10) / 100; // 10% for priority distribution
            let position_factor = (num_agents - 1 - agent_index) as f64 / (num_agents - 1) as f64;
            ((remaining_priority as f64 * position_factor) / num_agents as f64) as usize
        } else {
            0
        };

        base_per_agent + priority_bonus
    }

    /// Create an isolated sub-context based on isolation mode
    #[must_use]
    pub fn create_isolated(&self, mode: ContextIsolation) -> Self {
        let mut ctx = Self::new(Uuid::new_v4().to_string());
        ctx.parent_context_id = Some(self.session_id.clone());
        ctx.isolation_mode = Some(format!("{mode:?}"));

        match mode {
            ContextIsolation::Full => {
                ctx.system_prompt = self.system_prompt.clone();
                ctx.messages = self.messages.clone();
                ctx.summaries = self.summaries.clone();
                ctx.verified_outcomes = self.verified_outcomes.clone();
            }
            ContextIsolation::SystemOnly => {
                ctx.system_prompt = self.system_prompt.clone();
            }
            ContextIsolation::Summary => {
                ctx.system_prompt = self.system_prompt.clone();
                // Add summaries as context in system prompt
                if !self.summaries.is_empty() {
                    let summary_text: String = self.summaries
                        .iter()
                        .map(|s| s.summary.as_str())
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    ctx.system_prompt = format!(
                        "{}\n\n## Previous Context Summary\n{}",
                        ctx.system_prompt, summary_text
                    );
                }
            }
            ContextIsolation::Isolated => {
                // Completely fresh - only set parent_context_id for reference
            }
        }

        // Inherit model limits from parent
        ctx.compression_threshold = self.compression_threshold;
        ctx.hard_limit = self.hard_limit;
        ctx.current_model = self.current_model.clone();

        ctx
    }

    /// Add a user text message
    pub fn add_user_message(&mut self, content: impl Into<String>) {
        self.messages.push(AnthropicMessage::user_text(content));
        self.trim_if_needed();
    }

    /// Add an assistant text message
    pub fn add_assistant_message(&mut self, content: impl Into<String>) {
        self.messages.push(AnthropicMessage::assistant_text(content));
        self.trim_if_needed();
    }

    /// Estimate token count using a conservative heuristic.
    ///
    /// Uses ~3.2 chars per token (via [`estimate_token_count`]) instead of the
    /// commonly cited 4, because code, JSON, and tool calls tokenize at a higher
    /// ratio. Over-estimating triggers earlier compression, which is much better
    /// than hitting a 400 context_length_exceeded error mid-run.
    #[must_use]
    pub fn estimate_tokens(&self) -> usize {
        // Family-aware heuristic from nanna-llm (ASCII English/code + CJK density).
        let system_tokens = estimate_tokens(&self.system_prompt);
        let summary_tokens: usize = self.summaries.iter().map(|s| estimate_tokens(&s.summary)).sum();
        let message_tokens: usize = self.messages.iter().map(estimate_message_tokens).sum();

        system_tokens + summary_tokens + message_tokens
    }

    /// Estimated tokens of the step frame: the FIRST user message (the
    /// original request / harness step prompt). [`Self::truncate_to_limit`]
    /// always preserves it, so it is an irreducible part of every request this
    /// context will ever produce — the context-floor derivation counts it.
    #[must_use]
    pub fn step_frame_tokens(&self) -> usize {
        self.messages.first().map_or(0, estimate_message_tokens)
    }

    /// Check if compression is needed based on token count
    #[must_use]
    pub fn needs_compression(&self) -> bool {
        self.estimate_tokens() > self.compression_threshold
    }

    /// Check if context exceeds hard limit (must truncate before API call)
    ///
    /// This checks the full request tokens (including consolidated summary)
    /// since that's what will actually be sent to the API.
    #[must_use]
    pub fn exceeds_hard_limit(&self) -> bool {
        self.estimate_request_tokens() > self.hard_limit
    }

    /// Truncate oldest messages to get under the hard limit.
    ///
    /// This is a last-resort measure when compression isn't enough or isn't possible.
    /// Always keeps the first user message (the original request) and the most recent
    /// messages to avoid losing the user's intent.
    /// Returns the number of messages removed.
    pub fn truncate_to_limit(&mut self) -> usize {
        let mut removed = 0;

        // Keep at least 2 messages (first user message + most recent)
        // Remove from index 1 (after the first message) to preserve the original request
        while self.exceeds_hard_limit() && self.messages.len() > 2 {
            self.messages.remove(1);
            removed += 1;
        }

        // If still over limit with only 2 messages, fall back to removing the first
        while self.exceeds_hard_limit() && self.messages.len() > 1 {
            self.messages.remove(0);
            removed += 1;
        }

        // If still over limit with just 1 message, truncate large content blocks
        if self.exceeds_hard_limit() && !self.messages.is_empty() {
            self.truncate_large_content_blocks();
        }

        if removed > 0 {
            info!(
                removed_messages = removed,
                remaining_messages = self.messages.len(),
                estimated_tokens = self.estimate_tokens(),
                hard_limit = self.hard_limit,
                "Truncated context to fit within hard limit"
            );
        }

        removed
    }

    /// Truncate individual content blocks that are too large
    fn truncate_large_content_blocks(&mut self) {
        // Target: leave room for output tokens
        let target_tokens = self.hard_limit.saturating_sub(10_000);
        let max_block_chars = (target_tokens * 4).max(100); // ~4 chars per token, floor at 100

        for msg in &mut self.messages {
            for block in &mut msg.content {
                match block {
                    ContentBlock::ToolResult { content, .. } => {
                        if content.len() > max_block_chars {
                            let end = content.floor_char_boundary(max_block_chars.min(content.len()));
                            let truncated = &content[..end];
                            *content = format!(
                                "{}\n\n[... truncated {} chars to fit context limit ...]",
                                truncated,
                                content.len() - truncated.len()
                            );
                            info!(
                                original_len = content.len(),
                                truncated_to = max_block_chars,
                                "Truncated large tool result"
                            );
                        }
                    }
                    ContentBlock::Text { text } => {
                        if text.len() > max_block_chars {
                            let end = text.floor_char_boundary(max_block_chars.min(text.len()));
                            let truncated = &text[..end];
                            *text = format!(
                                "{}\n\n[... truncated {} chars to fit context limit ...]",
                                truncated,
                                text.len() - truncated.len()
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Ensure context is within limits, compressing or truncating as needed.
    ///
    /// Call this before making an API request to avoid context length errors.
    ///
    /// # Returns
    /// - `Ok(true)` if compression was performed
    /// - `Ok(false)` if no changes were needed
    /// - `Err` if compression failed (truncation will still be attempted)
    ///
    /// # Errors
    /// Returns error if LLM compression call fails.
    pub async fn enforce_limits(
        &mut self,
        llm: &LlmClient,
        model: &str,
    ) -> Result<bool, nanna_llm::LlmError> {
        let mut compressed = false;

        // Try compression first if over threshold
        if self.needs_compression() && self.messages.len() > 10 {
            info!(
                estimated_tokens = self.estimate_tokens(),
                compression_threshold = self.compression_threshold,
                "Context exceeds compression threshold, compressing"
            );
            self.compress(llm, model, 10).await?;
            compressed = true;
        }

        // If still over hard limit, truncate
        if self.exceeds_hard_limit() {
            self.truncate_to_limit();
        }

        Ok(compressed)
    }

    /// Recursively summarize context until it fits within the chat model's limit.
    ///
    /// This is the main entry point for intelligent context management:
    /// 1. Estimates current context size
    /// 2. If over limit, takes chunks that fit the summarization model
    /// 3. Summarizes each chunk
    /// 4. Replaces original content with summaries
    /// 5. Repeats until context fits or max iterations reached
    ///
    /// # Arguments
    /// * `config` - Summarization configuration (models, limits, etc.)
    ///
    /// # Returns
    /// * `Ok(iterations)` - Number of summarization passes performed
    /// * `Err` - If all summarization models fail
    pub async fn enforce_limits_with_summarization(
        &mut self,
        config: &ContextSummarizationConfig,
        target: SummarizationTarget,
    ) -> Result<usize, String> {
        if config.model_priority.is_empty() {
            // No summarization configured, fall back to truncation
            if self.exceeds_hard_limit() {
                warn!("No summarization models configured, truncating context");
                let dropped = self.truncate_to_limit();
                self.push_summarization_failure_notice(
                    dropped,
                    "no summarization models are configured, so nothing could \
                     stand in for the dropped messages",
                );
            }
            return Ok(0);
        }

        let mut iterations = 0;
        // Candidate summarizers resolve and enforce their own provider-reported limits.
        // This only bounds extraction to the context already held by this agent.
        let max_chars_per_chunk = self.hard_limit.saturating_mul(4);

        // The caller's tier decides how far down to summarize; testing the
        // hard limit here regardless is what made the tier-2 call a no-op.
        let over_target = |ctx: &Self| match target {
            SummarizationTarget::CompressionThreshold => ctx.needs_compression(),
            SummarizationTarget::HardLimit => ctx.exceeds_hard_limit(),
        };
        while over_target(self) && iterations < config.max_iterations {
            iterations += 1;

            let current_tokens = self.estimate_tokens();
            info!(
                iteration = iterations,
                current_tokens = current_tokens,
                hard_limit = self.hard_limit,
                "Context exceeds limit, summarizing"
            );

            // Find content to summarize (oldest messages first, keeping most recent)
            let (content_to_summarize, covered_ends) =
                self.extract_content_for_summarization(max_chars_per_chunk);

            if content_to_summarize.is_empty() {
                warn!("No content available to summarize, truncating remaining");
                let dropped = self.truncate_to_limit();
                self.push_summarization_failure_notice(
                    dropped,
                    "the remaining history has no summarizable middle (only \
                     the pinned request and the live tail)",
                );
                break;
            }

            // Try to summarize with fallback
            match Self::summarize_content_with_fallback(
                &content_to_summarize,
                &covered_ends,
                config,
            )
            .await
            {
                Ok((summary, consumed)) => {
                    // Only messages the summarizer actually READ may be
                    // dropped. It truncates the blob to its own window, which
                    // on a small local summarizer is a fraction of a large
                    // chat window — replacing the whole extraction on the
                    // strength of that summary silently discarded the rest.
                    let covered_messages =
                        covered_ends.iter().take_while(|&&end| end <= consumed).count();

                    if covered_messages == 0 {
                        // Not even one whole message fit. Dropping anything
                        // here would delete more than was summarized, and
                        // looping would spin on the same content forever.
                        warn!(
                            consumed,
                            first_message_ends_at = covered_ends.first().copied().unwrap_or(0),
                            "summarizer window too small for a single message; truncating instead"
                        );
                        let dropped = self.truncate_to_limit();
                        self.push_summarization_failure_notice(
                            dropped,
                            "the summarization model's window is too small to \
                             read even one whole message",
                        );
                        break;
                    }

                    info!(
                        extracted_len = content_to_summarize.len(),
                        summarized_len = consumed,
                        covered_messages,
                        summary_len = summary.len(),
                        compression = format!(
                            "{:.1}x",
                            consumed as f64 / summary.len().max(1) as f64
                        ),
                        "Content summarized successfully"
                    );

                    // Replace the summarized content with the summary
                    self.replace_with_summary(covered_messages, &summary);
                }
                Err(e) => {
                    warn!(error = %e, "All summarization models failed, truncating");
                    let dropped = self.truncate_to_limit();
                    self.push_summarization_failure_notice(
                        dropped,
                        &format!("every summarization model failed ({e})"),
                    );
                    break;
                }
            }
        }

        if iterations >= config.max_iterations && self.exceeds_hard_limit() {
            warn!(
                iterations = iterations,
                "Max summarization iterations reached, force truncating"
            );
            let dropped = self.truncate_to_limit();
            self.push_summarization_failure_notice(
                dropped,
                &format!(
                    "summarization spent its whole {iterations}-pass budget \
                     and the context still exceeded the hard input limit"
                ),
            );
        }

        Ok(iterations)
    }

    /// Extract content from oldest messages for summarization
    /// Gather the oldest messages into one blob, plus the running length of
    /// that blob after each message it FULLY covered.
    ///
    /// Those offsets are what make the replacement safe. Extraction stops at
    /// `max_chars`, and the summarizer truncates again to its own window, so
    /// what actually got summarized is a PREFIX of this content — not all of
    /// it, and not a whole number of messages unless someone checks. Knowing
    /// where each message ended lets the caller drop exactly the covered ones.
    fn extract_content_for_summarization(&self, max_chars: usize) -> (String, Vec<usize>) {
        let mut content = String::new();
        let mut chars_collected = 0;
        let mut covered_ends: Vec<usize> = Vec::new();

        // Keep at least the last 2 messages (user query + assistant response in progress)
        let messages_to_consider = if self.messages.len() > 2 {
            &self.messages[..self.messages.len() - 2]
        } else {
            return (String::new(), covered_ends); // Not enough messages to summarize
        };

        for msg in messages_to_consider {
            for block in &msg.content {
                let block_text = match block {
                    ContentBlock::Text { text } => text.clone(),
                    ContentBlock::ToolUse { name, input, .. } => {
                        format!("[Tool call: {} with input: {}]", name, input)
                    }
                    ContentBlock::ToolResult { content, .. } => content.clone(),
                    ContentBlock::Thinking { thinking, .. } => {
                        let end = thinking.floor_char_boundary(thinking.len().min(200));
                        format!("[Thinking: {}]", &thinking[..end])
                    }
                    ContentBlock::Image { .. } => "[Image]".to_string(),
                };

                if chars_collected + block_text.len() > max_chars {
                    // Take partial if we haven't collected anything yet
                    if content.is_empty() {
                        let end = block_text.floor_char_boundary(max_chars.min(block_text.len()));
                        content.push_str(&block_text[..end]);
                    }
                    return (content, covered_ends);
                }

                content.push_str(&format!("[{}]: {}\n", msg.role, block_text));
                chars_collected += block_text.len();
            }
            // Recorded only once every block of the message fit: a partially
            // captured message must never be dropped on the strength of a
            // summary that saw only part of it.
            covered_ends.push(content.len());
        }

        (content, covered_ends)
    }

    /// Split `content` into pieces that each fit `budget_chars`, cutting only
    /// at the message boundaries in `covered_ends`.
    ///
    /// Returns `(chunk_text, end_offset_in_content)` per piece. Cutting only at
    /// boundaries is what lets the caller say which whole messages a set of
    /// summaries covers; a chunk ending mid-message could not be counted.
    ///
    /// A single message larger than the budget gets its own chunk. The
    /// summarizer truncates it internally and the caller decides whether that
    /// partial reading may retire it — splitting mid-message is lossy either
    /// way, and at least this keeps every other message whole.
    fn chunk_at_message_boundaries(
        content: &str,
        covered_ends: &[usize],
        budget_chars: usize,
    ) -> Vec<(String, usize)> {
        let budget = budget_chars.max(1);
        let mut chunks: Vec<(String, usize)> = Vec::new();
        let mut start = 0usize;
        let mut last_end = 0usize;

        for &end in covered_ends {
            if end <= start {
                continue;
            }
            if end - start > budget {
                // Flush the whole messages that fit before this one.
                if last_end > start {
                    chunks.push((content[start..last_end].to_string(), last_end));
                    start = last_end;
                }
                // Still over? Then this message alone exceeds the budget.
                if end - start > budget {
                    chunks.push((content[start..end].to_string(), end));
                    start = end;
                }
            }
            last_end = end;
        }
        if last_end > start {
            chunks.push((content[start..last_end].to_string(), last_end));
        }
        chunks
    }

    /// Join per-chunk summaries into one document.
    ///
    /// Numbered because the chunks are sequential slices of one conversation:
    /// without the order a reader cannot tell whether two statements are a
    /// contradiction or a change over time.
    fn splice_summaries(parts: &[String]) -> String {
        if parts.len() == 1 {
            return parts[0].clone();
        }
        parts
            .iter()
            .enumerate()
            .map(|(i, p)| format!("[Part {} of {}]\n{}", i + 1, parts.len(), p.trim()))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Summarize the whole blob by chunking it to the summarizer's own window,
    /// summarizing each chunk in order, and splicing the results.
    ///
    /// Returns `(spliced_summary, chars_of_content_actually_read)`.
    ///
    /// Handing the whole blob over and letting the model truncate silently
    /// threw away everything past its window — and a secondary model's context
    /// is routinely a fraction of the chat model's, so a 32k local summarizer
    /// asked to compress a 1M-token conversation read the first few percent
    /// and reported success. Chunking reads all of it.
    ///
    /// A model that fails partway is abandoned for the next candidate rather
    /// than half-trusted: its summaries describe only a prefix, and splicing
    /// two models' voices mid-document is a worse artifact than letting the
    /// next candidate do the whole job.
    async fn summarize_content_with_fallback(
        content: &str,
        covered_ends: &[usize],
        config: &ContextSummarizationConfig,
    ) -> Result<(String, usize), String> {
        for model_spec in &config.model_priority {
            debug!(model = %model_spec, "Attempting summarization");

            let (client, model_name, budget) =
                match Self::resolve_summarizer(model_spec, config).await {
                    Ok(resolved) => resolved,
                    Err(e) => {
                        warn!(model = %model_spec, error = %e, "Summarizer unavailable, trying next");
                        continue;
                    }
                };

            let chunks = Self::chunk_at_message_boundaries(content, covered_ends, budget);
            if chunks.is_empty() {
                warn!(model = %model_spec, "Nothing to summarize after chunking");
                continue;
            }

            let mut parts: Vec<String> = Vec::with_capacity(chunks.len());
            let mut consumed = 0usize;
            let mut failed = false;

            for (i, (chunk, end)) in chunks.iter().enumerate() {
                match Self::summarize_chunk(&client, &model_name, chunk).await {
                    Ok(summary) => {
                        parts.push(summary);
                        consumed = *end;
                    }
                    Err(e) => {
                        warn!(
                            model = %model_spec,
                            chunk = i + 1,
                            of = chunks.len(),
                            error = %e,
                            "Chunk summarization failed"
                        );
                        failed = true;
                        break;
                    }
                }
            }

            if failed {
                continue;
            }

            info!(
                model = %model_spec,
                chunks = chunks.len(),
                read_chars = consumed,
                "Summarization succeeded"
            );
            return Ok((Self::splice_summaries(&parts), consumed));
        }

        Err("All summarization models failed".to_string())
    }

    /// Try to summarize with a specific model via direct LLM call
    /// Resolve a summarizer candidate to `(client, model, input budget in chars)`.
    ///
    /// Separated from the call so the budget can be known BEFORE the content is
    /// split: chunking to this number is what lets every chunk be read whole
    /// instead of the tail being silently truncated away. Summarizers in one
    /// priority list can have radically different windows, so this is resolved
    /// per candidate, not once for the list.
    async fn resolve_summarizer(
        model_spec: &str,
        config: &ContextSummarizationConfig,
    ) -> Result<(nanna_llm::LlmClient, String, usize), String> {
        let (client, model_name) = Self::create_client_for_model(model_spec, config)?;
        let cache = nanna_llm::ModelInfoCache::default_location();
        let model_info = client.get_model_info(&model_name, cache.as_ref()).await;
        // Reserve output capacity and prompt framing before filling the input.
        let budget = model_info
            .hard_input_limit()
            .saturating_sub(512)
            .saturating_mul(4);
        Ok((client, model_name, budget.max(1)))
    }

    /// Summarize one chunk that is already sized to fit `model_name`.
    ///
    /// The truncation here is a backstop, not the sizing mechanism — chunks
    /// arrive pre-fitted. It only bites when a single message exceeds the whole
    /// window, which the caller accounts for separately.
    async fn summarize_chunk(
        client: &nanna_llm::LlmClient,
        model_name: &str,
        content: &str,
    ) -> Result<String, String> {
        let cache = nanna_llm::ModelInfoCache::default_location();
        let model_info = client.get_model_info(model_name, cache.as_ref()).await;
        let max_chars = model_info.hard_input_limit().saturating_sub(512).saturating_mul(4);
        let truncated = if content.len() > max_chars {
            &content[..content.floor_char_boundary(max_chars)]
        } else {
            content
        };
        let model_name = model_name.to_string();

        let prompt = format!(
            "Summarize the following conversation history concisely. Preserve key facts, decisions, \
             file paths, code snippets, and important context needed to continue the conversation.\n\n\
             ---\n{}\n---\n\nProvide a concise summary:",
            truncated
        );

        let request = AnthropicRequest {
            context_limit: None,
            messages: vec![AnthropicMessage::user_text(prompt)],
            max_tokens: u32::try_from(model_info.max_output_tokens.min(2_048)).unwrap_or(u32::MAX),
            temperature: nanna_llm::sampling_temperature_for_model(&model_name, 0.3),
            model: model_name,
            system: Some("You are a conversation summarizer. Output only the summary, no preamble.".to_string()),
            tools: None,
            stream: None,
            thinking: None,
            cache_control: None,
        };

        let response = client.complete_anthropic(&request).await.map_err(|e| e.to_string())?;

        // Extract text from response
        let mut summary = String::new();
        for block in &response.content {
            if let ContentBlock::Text { text } = block {
                summary.push_str(text);
            }
        }

        if plausible_summary(&summary, truncated.len()) {
            Ok(summary)
        } else {
            Err(format!(
                "Implausible summary returned ({} chars for {} chars of input)",
                summary.trim().len(),
                truncated.len()
            ))
        }
    }

    /// Create an LLM client for the specified model
    fn create_client_for_model(
        model_spec: &str,
        config: &ContextSummarizationConfig,
    ) -> Result<(LlmClient, String), String> {
        if let Some((provider, model)) = model_spec.split_once('/') {
            let client = match provider.to_lowercase().as_str() {
                "ollama" => {
                    let url = config.ollama_url.as_deref().unwrap_or("http://localhost:11434");
                    LlmClient::ollama(url)
                }
                "anthropic" => {
                    // This would need API key - for now return error
                    // In practice, the Agent passes its own client
                    return Err(
                        "Anthropic summarization requires passing main client".to_string()
                    );
                }
                "openai" => {
                    if let Some(ref api_key) = config.openai_api_key {
                        LlmClient::openai(api_key)
                    } else {
                        return Err("OpenAI summarization requires API key (set openai_api_key)".to_string());
                    }
                }
                "openrouter" => {
                    if let Some(ref api_key) = config.openrouter_api_key {
                        LlmClient::openrouter(api_key)
                    } else {
                        return Err("OpenRouter summarization requires API key (set openrouter_api_key)".to_string());
                    }
                }
                _ => {
                    return Err(format!("Unknown provider: {}", provider));
                }
            };
            Ok((client, model.to_string()))
        } else {
            // No provider prefix - assume ollama
            let url = config.ollama_url.as_deref().unwrap_or("http://localhost:11434");
            Ok((LlmClient::ollama(url), model_spec.to_string()))
        }
    }

    /// Replace summarized content with the summary.
    ///
    /// Updates the consolidated_summary field for incremental summarization.
    /// On subsequent requests, the consolidated summary is prepended to messages,
    /// avoiding the need to re-summarize everything.
    ///
    /// Uses content-defined chunking (CDC) for deduplication - this creates
    /// deterministic chunk boundaries based on content, allowing detection of
    /// duplicate content even when split differently.
    /// Replace the first `covered_messages` messages with `summary`.
    ///
    /// The count is the caller's, not this function's, and that is the whole
    /// point. This used to take the extracted content, ignore it, and remove
    /// everything but the last two messages — correct only if the summarizer
    /// had read everything extracted. It had not: extraction stops at the
    /// chat model's budget and the summarizer truncates again to its own, so
    /// on a large chat window with a small local summarizer the great majority
    /// of the removed history was never summarized at all. It vanished, and
    /// the compression ratio logged against the pre-truncation length made it
    /// look like a triumph.
    fn replace_with_summary(&mut self, covered_messages: usize, summary: &str) {
        // Never take the last 2 (live user query + in-flight response), and
        // never take more than the summarizer actually read.
        let keep_count = 2.min(self.messages.len());
        let removable = self.messages.len().saturating_sub(keep_count);
        let remove_count = covered_messages.min(removable);

        if remove_count > 0 {
            // Use CDC to hash content blocks from messages being removed
            let mut new_chunk_hashes = 0;
            for msg in &self.messages[..remove_count] {
                for block in &msg.content {
                    let text = match block {
                        ContentBlock::Text { text } => text,
                        ContentBlock::ToolResult { content, .. } => content,
                        _ => continue,
                    };
                    // Only chunk content large enough to produce meaningful chunks
                    if text.len() >= DEDUP_MIN_SIZE {
                        // Use CDC to get content-defined chunk hashes
                        let chunk_hashes = chunk_and_hash(text);
                        for hash in chunk_hashes {
                            if self.summarized_content_hashes.insert(hash) {
                                new_chunk_hashes += 1;
                            }
                        }
                        debug!(
                            content_len = text.len(),
                            new_chunks = new_chunk_hashes,
                            "Stored CDC chunk hashes for deduplication"
                        );
                    }
                }
            }

            // Update the consolidated summary (append new summary to existing if present)
            let new_consolidated = if let Some(ref existing) = self.consolidated_summary {
                // Combine existing summary with new summary
                format!(
                    "{}\n\n---\n\n[Additional context from {} more messages:]\n{}",
                    existing, remove_count, summary
                )
            } else {
                summary.to_string()
            };

            self.consolidated_summary = Some(new_consolidated.clone());

            // Also store in summaries for history tracking
            self.summaries.push(ContextSummary {
                summary: summary.to_string(),
                messages_compressed: remove_count,
                tokens_saved: self.estimate_tokens(), // Approximate
                created_at: chrono_timestamp(),
            });

            // Remove the old messages
            self.messages.drain(0..remove_count);

            info!(
                removed_messages = remove_count,
                remaining_messages = self.messages.len(),
                consolidated_summary_len = new_consolidated.len(),
                new_chunk_hashes = new_chunk_hashes,
                total_chunk_hashes = self.summarized_content_hashes.len(),
                "Updated consolidated summary with CDC deduplication"
            );
        }
    }

    /// Drop oldest messages (no LLM required) as a fallback compression strategy.
    ///
    /// Keeps the first user message (original request) and the most recent
    /// `keep_recent` messages, dropping everything in between.
    /// Adds a note to `consolidated_summary` about what was dropped.
    /// Returns the number of messages dropped.
    pub fn drop_oldest(&mut self, keep_recent: usize) -> usize {
        // +1 for the pinned first message
        if self.messages.len() <= keep_recent + 1 {
            return 0;
        }

        // Preserve first message (the original user request) by only dropping from index 1+
        let droppable = &self.messages[1..]; // everything after the first message

        let drop_count = if droppable.len() > keep_recent {
            droppable.len() - keep_recent
        } else {
            return 0;
        };

        // Build a brief summary of what's being dropped (from index 1..1+drop_count)
        let mut dropped_summary_parts = Vec::new();
        for msg in &self.messages[1..1 + drop_count] {
            let role = &msg.role;
            for block in &msg.content {
                match block {
                    ContentBlock::Text { text } => {
                        let preview = if text.len() > 100 {
                            format!("{}...", &text[..100])
                        } else {
                            text.clone()
                        };
                        dropped_summary_parts.push(format!("[{}]: {}", role, preview));
                    }
                    ContentBlock::ToolUse { name, .. } => {
                        dropped_summary_parts.push(format!("[{}]: [tool call: {}]", role, name));
                    }
                    ContentBlock::ToolResult { content, .. } => {
                        let preview = if content.len() > 80 {
                            format!("{}...", &content[..80])
                        } else {
                            content.clone()
                        };
                        dropped_summary_parts.push(format!("[tool result]: {}", preview));
                    }
                    _ => {}
                }
            }
        }

        let drop_note = format!(
            "[{} older messages dropped to free context space. Key fragments:\n{}]",
            drop_count,
            dropped_summary_parts.join("\n")
        );

        // Update consolidated summary
        let new_summary = if let Some(ref existing) = self.consolidated_summary {
            format!("{}\n\n---\n\n{}", existing, drop_note)
        } else {
            drop_note
        };
        self.consolidated_summary = Some(new_summary);

        // Actually remove (from index 1, preserving the pinned first message)
        self.messages.drain(1..1 + drop_count);

        info!(
            dropped = drop_count,
            remaining = self.messages.len(),
            estimated_tokens = self.estimate_tokens(),
            "Dropped oldest messages (no-LLM fallback)"
        );

        drop_count
    }

    /// LLMLingua-style selective compression of older large tool results.
    ///
    /// Walks messages older than `keep_recent`, scores each large
    /// [`ContentBlock::ToolResult`] with the summarization-model priority list
    /// via `compress_with`, and rewrites the block in place when compression
    /// actually shrinks content. Pinned first message is never touched.
    /// Returns the number of tool results rewritten.
    pub async fn compress_older_tool_results<F, Fut>(
        &mut self,
        keep_recent: usize,
        min_chars: usize,
        mut compress_with: F,
    ) -> usize
    where
        F: FnMut(String) -> Fut,
        Fut: std::future::Future<Output = Option<String>>,
    {
        if self.messages.len() <= keep_recent + 1 {
            return 0;
        }
        // Preserve index 0 (pinned original request) and the most recent `keep_recent`.
        let end = self.messages.len().saturating_sub(keep_recent);
        if end <= 1 {
            return 0;
        }

        // Collect candidates first so we can drop the exclusive borrow before any await.
        let mut candidates: Vec<(usize, usize, String)> = Vec::new();
        for (msg_idx, msg) in self.messages[1..end].iter().enumerate() {
            let absolute = msg_idx + 1;
            for (block_idx, block) in msg.content.iter().enumerate() {
                if let ContentBlock::ToolResult { content, is_error, .. } = block {
                    if is_error.unwrap_or(false) {
                        continue; // keep errors verbatim
                    }
                    if content.len() >= min_chars {
                        candidates.push((absolute, block_idx, content.clone()));
                    }
                }
            }
        }

        let mut compressed_count = 0usize;
        for (msg_idx, block_idx, content) in candidates {
            let Some(compressed) = compress_with(content.clone()).await else {
                continue;
            };
            if compressed.len() >= content.len() {
                continue;
            }
            if let Some(msg) = self.messages.get_mut(msg_idx) {
                if let Some(ContentBlock::ToolResult {
                    content: slot,
                    ..
                }) = msg.content.get_mut(block_idx)
                {
                    let original_len = slot.len();
                    // The summary MUST announce itself. Observed live: an
                    // unmarked compressed result reads as garbled/corrupt
                    // data — the model concluded its files were damaged and
                    // rewrote them from the artifacts. A summary that names
                    // itself is context savings; one that doesn't is a
                    // hallucination seed.
                    *slot = format!(
                        "[COMPRESSED SUMMARY of an older tool result ({original_len} chars \
                         original). WHY: your context window is limited, and this older \
                         result was shortened to make room for current work — the tool call \
                         itself SUCCEEDED in full. The wording below is lossy shorthand, NOT \
                         the literal output; trust files on disk over this.]\n{compressed}"
                    );
                    compressed_count += 1;
                    debug!(
                        msg_idx,
                        block_idx,
                        original_len,
                        compressed_len = slot.len(),
                        "🗜️ Compressed older tool result in context"
                    );
                }
            }
        }

        if compressed_count > 0 {
            info!(
                compressed = compressed_count,
                estimated_tokens = self.estimate_tokens(),
                "LLMLingua selective older-context compression complete"
            );
        }
        compressed_count
    }

    /// Compress old messages into a summary using LLM.
    ///
    /// Keeps the most recent `keep_recent` messages and compresses the rest.
    ///
    /// # Errors
    /// Returns error if LLM call fails
    pub async fn compress(
        &mut self,
        llm: &LlmClient,
        model: &str,
        keep_recent: usize,
    ) -> Result<ContextSummary, nanna_llm::LlmError> {
        if self.messages.len() <= keep_recent {
            // Nothing to compress
            return Ok(ContextSummary {
                summary: String::new(),
                messages_compressed: 0,
                tokens_saved: 0,
                created_at: chrono_timestamp(),
            });
        }

        // Split messages into old (to compress) and recent (to keep).
        // Always preserve the first message (original user request) — start compressing from index 1.
        let split_point = self.messages.len() - keep_recent;
        let compress_start = 1.min(split_point); // skip index 0 (pinned first message)
        let old_messages = &self.messages[compress_start..split_point];

        // Build a text representation of old messages
        let mut conversation_text = String::new();
        for msg in old_messages {
            let role = &msg.role;
            for block in &msg.content {
                match block {
                    ContentBlock::Text { text } => {
                        conversation_text.push_str(&format!("[{role}]: {text}\n"));
                    }
                    ContentBlock::ToolUse { name, .. } => {
                        conversation_text.push_str(&format!("[{role}]: [Called tool: {name}]\n"));
                    }
                    ContentBlock::ToolResult { content, .. } => {
                        // Truncate long tool results in summary
                        let truncated = if content.len() > 200 {
                            format!("{}...", &content[..200])
                        } else {
                            content.clone()
                        };
                        conversation_text.push_str(&format!("[tool result]: {truncated}\n"));
                    }
                    ContentBlock::Thinking { thinking, .. } => {
                        // Include reasoning in summary, truncated
                        let truncated = if thinking.len() > 200 {
                            format!("{}...", &thinking[..200])
                        } else {
                            thinking.clone()
                        };
                        conversation_text.push_str(&format!("[thinking]: {truncated}\n"));
                    }
                    ContentBlock::Image { .. } => {
                        conversation_text.push_str("[image]\n");
                    }
                }
            }
        }

        // Create summarization prompt
        let prompt = format!(
            r#"Summarize this conversation concisely, preserving key facts, decisions, and context that would be important for continuing the conversation. Focus on:
- Important user preferences or information shared
- Key decisions or conclusions reached
- Relevant context about ongoing tasks or projects
- Any commitments or follow-ups mentioned

Conversation to summarize:
{}

Provide a concise summary (2-4 paragraphs max):"#,
            conversation_text
        );

        // Call LLM for summarization
        let request = nanna_llm::CompletionRequest::default()
            .with_model(model)
            .with_message(nanna_llm::Message::user(&prompt))
            .with_max_tokens(1024)
            .with_temperature(0.3);

        let summary_text = llm.complete(&request).await?;

        // Calculate tokens saved
        let old_tokens: usize = old_messages.iter()
            .map(|m| m.content.iter()
                .map(|c| match c {
                    ContentBlock::Text { text } => estimate_token_count(text.len()),
                    ContentBlock::ToolUse { input, .. } => estimate_token_count(input.to_string().len()),
                    ContentBlock::ToolResult { content, .. } => estimate_token_count(content.len()),
                    ContentBlock::Thinking { thinking, .. } => estimate_token_count(thinking.len()),
                    ContentBlock::Image { .. } => 1000,
                })
                .sum::<usize>()
            )
            .sum();
        let new_tokens = estimate_token_count(summary_text.len());
        let tokens_saved = old_tokens.saturating_sub(new_tokens);

        let summary = ContextSummary {
            summary: summary_text,
            messages_compressed: old_messages.len(),
            tokens_saved,
            created_at: chrono_timestamp(),
        };

        // Store summary and remove old compressed messages (preserve first message)
        self.summaries.push(summary.clone());
        let mut kept = vec![self.messages[0].clone()]; // pin first message
        kept.extend(self.messages.split_off(split_point));
        self.messages = kept;

        Ok(summary)
    }

    /// Get combined context including summaries for building prompts
    #[must_use]
    pub fn get_full_context(&self) -> String {
        let mut context = String::new();

        // Add summaries first (older context)
        if !self.summaries.is_empty() {
            context.push_str("## Previous Conversation Summary\n");
            for summary in &self.summaries {
                context.push_str(&summary.summary);
                context.push_str("\n\n");
            }
            context.push_str("---\n\n## Current Conversation\n");
        }

        context
    }

    fn trim_if_needed(&mut self) {
        while self.messages.len() > self.max_messages {
            self.messages.remove(0);
        }
    }
}

impl Default for AgentContext {
    fn default() -> Self {
        Self::new(Uuid::new_v4().to_string())
    }
}

/// Conservative token count estimate from character length.
///
/// Uses ~3.2 chars per token (multiply by 10, divide by 32) which is more
/// accurate for mixed content (code + prose + JSON) than the commonly cited
/// 4 chars/token ratio. For pure English prose the real ratio is ~4, but
/// code identifiers, JSON keys, and special characters tokenize much worse.
/// Over-estimating by ~20% is a good trade: it triggers compression a bit
/// earlier but avoids the catastrophic 400 context_length_exceeded error.
fn estimate_token_count(char_len: usize) -> usize {
    // (char_len * 10) / 32 ≈ char_len / 3.2
    (char_len * 10 + 31) / 32 // +31 for ceiling division
}

fn chrono_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The data-loss bug: extraction can gather far more than the summarizer
    /// will read, so the replacement must be keyed to what was actually
    /// summarized. Previously it removed everything but the last two messages
    /// regardless, which on a large chat window with a small local summarizer
    /// discarded the great majority of the history unsummarized.
    #[test]
    fn only_messages_the_summarizer_read_are_replaced() {
        let mut ctx = AgentContext::new("s1");
        for i in 0..10 {
            ctx.messages.push(AnthropicMessage::user_text(format!("message {i}")));
        }

        // Everything except the reserved last two is offered up...
        let (content, covered_ends) = ctx.extract_content_for_summarization(usize::MAX);
        assert_eq!(covered_ends.len(), 8, "10 messages, last 2 always reserved");
        assert!(!content.is_empty());

        // ...but suppose the summarizer's window only reached message 3.
        let consumed = covered_ends[2];
        let covered = covered_ends.iter().take_while(|&&e| e <= consumed).count();
        assert_eq!(covered, 3);

        ctx.replace_with_summary(covered, "summary of the first three");
        assert_eq!(
            ctx.messages.len(),
            7,
            "only the 3 summarized messages are gone; the other 7 survive"
        );
        assert!(
            ctx.messages.iter().any(|m| m.content.iter().any(|b| matches!(
                b,
                ContentBlock::Text { text } if text.contains("message 3")
            ))),
            "an unsummarized message must not be deleted"
        );
    }

    /// A partially-captured message is not counted as covered, so it cannot be
    /// deleted on the strength of a summary that saw only part of it.
    #[test]
    fn a_partially_captured_message_is_never_counted_as_covered() {
        let mut ctx = AgentContext::new("s1");
        for i in 0..5 {
            ctx.messages.push(AnthropicMessage::user_text(format!(
                "{i}: {}",
                "padding ".repeat(50)
            )));
        }
        // Room for roughly one message, so the second is cut mid-way.
        let (_, covered_ends) = ctx.extract_content_for_summarization(600);
        assert!(
            covered_ends.len() <= 1,
            "a truncated message is not reported as covered (got {})",
            covered_ends.len()
        );
    }

    /// Chunks must cut only at message boundaries, cover the whole input, and
    /// never exceed the summarizer's budget — that is what lets every chunk be
    /// read whole instead of the tail being truncated away.
    #[test]
    fn chunking_covers_everything_and_cuts_only_at_message_boundaries() {
        // Four messages ending at 100/200/300/400.
        let content: String = "x".repeat(400);
        let ends = vec![100usize, 200, 300, 400];

        let chunks = AgentContext::chunk_at_message_boundaries(&content, &ends, 250);
        assert!(chunks.len() >= 2, "400 chars into a 250 budget needs splitting");
        for (text, _) in &chunks {
            assert!(text.len() <= 250, "no chunk may exceed the budget");
        }
        // Every cut lands on a message boundary, and the last covers the end.
        for (_, end) in &chunks {
            assert!(ends.contains(end), "cut at {end} is not a message boundary");
        }
        assert_eq!(chunks.last().unwrap().1, 400, "all content is covered");
        let rebuilt: String = chunks.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(rebuilt, content, "chunks concatenate back to the input");
    }

    #[test]
    fn a_message_larger_than_the_budget_gets_its_own_chunk() {
        let content: String = "y".repeat(500);
        // Second message alone is 380 chars, over a 200 budget.
        let ends = vec![100usize, 480, 500];
        let chunks = AgentContext::chunk_at_message_boundaries(&content, &ends, 200);
        assert_eq!(chunks.last().unwrap().1, 500, "still covers everything");
        let rebuilt: String = chunks.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(rebuilt, content);
        assert!(
            chunks.iter().any(|(t, _)| t.len() > 200),
            "the oversized message is isolated rather than split mid-message"
        );
    }

    #[test]
    fn spliced_summaries_keep_their_order() {
        assert_eq!(AgentContext::splice_summaries(&["only".to_string()]), "only");
        let joined =
            AgentContext::splice_summaries(&["first".to_string(), "second".to_string()]);
        assert!(joined.contains("[Part 1 of 2]"));
        assert!(joined.contains("[Part 2 of 2]"));
        assert!(
            joined.find("first").unwrap() < joined.find("second").unwrap(),
            "order carries the difference between contradiction and change over time"
        );
    }

    #[test]
    fn implausible_summaries_are_rejected() {
        // The live failure: 80 KB "summarized" to 17 chars.
        assert!(!plausible_summary("Roadmap for Nanna", 80_765));
        assert!(!plausible_summary("", 80_765));
        assert!(!plausible_summary("   \n  ", 80_765));
        assert!(!plausible_summary("...", 5_000));
        // 64+ chars but under 0.1% of a very large source is still degenerate.
        assert!(!plausible_summary(&"x".repeat(70), 200_000));
    }

    #[test]
    fn plausible_summaries_are_accepted() {
        // A real ~2 KB summary of an 80 KB document.
        assert!(plausible_summary(&"a solid summary sentence. ".repeat(80), 80_765));
        // A modest but substantial summary of a mid-size result.
        assert!(plausible_summary(&"key fact retained here. ".repeat(4), 13_000));
        // Tiny sources may summarize tiny — anything non-empty passes.
        assert!(plausible_summary("ok", 500));
        assert!(!plausible_summary("", 500));
    }

    #[test]
    fn small_model_compression_threshold_stays_below_hard_limit() {
        // Explicit ModelInfo (as API would return), not a name table.
        let info = ModelInfo {
            id: "tiny".into(),
            context_window: 8_000,
            max_output_tokens: 4_096,
            supports_tools: true,
            supports_vision: false,
            embedding_dimension: None,
            cached_at: 0,
            provider: "test".into(),
        };
        let mut ctx = AgentContext::new("s");
        ctx.configure_for_model(&info);
        assert!(
            ctx.compression_threshold < ctx.hard_limit,
            "threshold {} must be below hard limit {}",
            ctx.compression_threshold,
            ctx.hard_limit
        );
    }

    #[test]
    fn large_model_compression_threshold_unchanged() {
        let info = ModelInfo {
            id: "claude-big".into(),
            context_window: 200_000,
            max_output_tokens: 8_192,
            supports_tools: true,
            supports_vision: true,
            embedding_dimension: None,
            cached_at: 0,
            provider: "test".into(),
        };
        let mut ctx = AgentContext::new("s");
        ctx.configure_for_model(&info);
        assert_eq!(ctx.compression_threshold, 160_000);
        assert!(ctx.compression_threshold < ctx.hard_limit);
    }

    #[test]
    fn name_path_uses_universal_floor_without_cache() {
        let mut ctx = AgentContext::new("s");
        ctx.configure_for_model_name("totally-unknown-local-model");
        // Mirrors unknown_model_info floors (no per-model table).
        assert_eq!(ctx.hard_limit, nanna_llm::unknown_model_info("x", "").hard_input_limit());
        assert!(ctx.compression_threshold <= ctx.hard_limit);
    }

    #[test]
    fn the_model_is_told_its_working_directory() {
        // Live 2026-07-26: nothing in the prompt said where the agent was
        // working, so the model learned the path from a `pwd` it happened to
        // run, then addressed files as "<ws-leaf>/minidb" — one directory too
        // deep — and edited a shadow copy for an hour while the acceptance
        // tests read the real file.
        let mut ctx = AgentContext::new("s");
        ctx.system_prompt = "BASE".to_string();
        ctx.workspace_root = Some(std::path::PathBuf::from("/tmp/bench-ws"));

        let effective = ctx.effective_system_prompt();
        assert!(effective.starts_with("BASE"), "identity still leads");
        assert!(
            effective.contains("bench-ws"),
            "the working directory must appear in the prompt: {effective}"
        );
        assert!(
            effective.contains("do NOT prefix"),
            "the prompt must warn against re-prefixing the directory name"
        );

        // No workspace: nothing to claim, so nothing is added.
        let mut global = AgentContext::new("s");
        global.system_prompt = "BASE".to_string();
        assert_eq!(global.effective_system_prompt(), "BASE");
    }

    #[test]
    fn workspace_context_is_bounded_by_the_model_window() {
        // The live regression: a 330k-char workspace context on a 32k model
        // blew past the window; Ollama clipped the prompt head and the model
        // lost its own tool definitions. The injection must be capped at the
        // window-derived limit with a visible marker.
        let mut ctx = AgentContext::new("s");
        ctx.hard_limit = 23_808; // qwen-like 32k window after output reserve
        ctx.system_prompt = "BASE".to_string();
        ctx.workspace_context = Some("x".repeat(330_249));

        let effective = ctx.effective_system_prompt();
        assert!(
            effective.len() < 30_000,
            "oversized workspace context must be truncated (got {} chars)",
            effective.len()
        );
        assert!(effective.contains("[workspace context truncated"));
        assert!(effective.starts_with("BASE"));

        // A small workspace context passes through untouched.
        ctx.workspace_context = Some("## README\nsmall".to_string());
        let effective = ctx.effective_system_prompt();
        assert!(effective.contains("## README\nsmall"));
        assert!(!effective.contains("truncated"));
    }

    #[test]
    fn output_budget_drives_the_input_limit() {
        // Dynamic split: the reserve tracks the request's max_tokens, not
        // the provider's max_output claim. A 2k-output agent on a 32k model
        // (whose provider claims max_output >= context) keeps ~94% of the
        // window for input.
        let info = ModelInfo {
            id: "qwen-like".into(),
            context_window: 32_000,
            max_output_tokens: 32_000,
            supports_tools: true,
            supports_vision: false,
            embedding_dimension: None,
            cached_at: 0,
            provider: "ollama".into(),
        };
        let mut ctx = AgentContext::new("s");
        ctx.configure_for_model_with_output(&info, 2_048);
        assert_eq!(ctx.hard_limit, 32_000 - 2_048);
        assert!(ctx.compression_threshold <= ctx.hard_limit);

        // The default 8k budget matches the static path's reservation.
        ctx.configure_for_model_with_output(&info, 8_192);
        assert_eq!(ctx.hard_limit, 32_000 - 8_192);

        // A degenerate budget can never claim more than half the window.
        ctx.configure_for_model_with_output(&info, 30_000);
        assert_eq!(ctx.hard_limit, 16_000);
    }

    /// The 2026-08-02 failure, replayed at the context level: a mid-run
    /// num_ctx demotion (16384 → 4096) must rebind EVERY window-derived
    /// budget — compression threshold, hard limit, workspace cap — and the
    /// existing no-LLM ladder must then shrink a transcript budgeted for the
    /// old window down to the new one. The mock window source is the real
    /// nanna-llm latch, driven by demote_context on a test-unique model.
    #[test]
    fn a_mid_run_demotion_rebinds_budgets_and_the_ladder_fits_the_new_window() {
        let model = "test-ctx-demotion-model:9b";
        let claim = ModelInfo {
            id: model.into(),
            context_window: 16_384,
            max_output_tokens: 8_192,
            supports_tools: true,
            supports_vision: false,
            embedding_dimension: None,
            cached_at: 0,
            provider: "ollama".into(),
        };

        // Budgeted for the full window, with a transcript sized to fit it.
        let mut ctx = AgentContext::new("s");
        ctx.configure_for_model_with_output(&claim, 2_048);
        let (old_threshold, old_hard, old_cap) = (
            ctx.compression_threshold,
            ctx.hard_limit,
            ctx.workspace_context_cap_chars(),
        );
        ctx.messages.push(AnthropicMessage::user_text("the step frame"));
        for i in 0..40 {
            ctx.messages
                .push(AnthropicMessage::assistant_text(format!("turn {i}: {}", "x".repeat(800))));
        }
        assert!(
            !ctx.exceeds_hard_limit(),
            "the transcript must fit the ORIGINAL window for the replay to mean anything"
        );

        // The demotion, exactly as VRAM pressure produces it: demote_context
        // walks the latch down rung by rung (3/4 on the 512 quantum, clamped
        // at the caller's floor), and the effective window follows. (An
        // unlatched model starts from the ladder ceiling.)
        while LlmClient::demote_context(model, Some(4_096)).is_some() {}
        let live = nanna_llm::effective_context_window(model, claim.context_window);
        assert_eq!(live, 4_096, "the latch is the live window source");

        // The rebind the agent loop performs: every budget re-derives.
        let live_info = nanna_llm::clamp_model_info_to_effective_window(model, claim);
        ctx.configure_for_model_with_output(&live_info, 2_048);
        assert!(ctx.hard_limit < old_hard, "hard limit must shrink with the window");
        assert!(ctx.compression_threshold < old_threshold);
        assert!(ctx.compression_threshold <= ctx.hard_limit);
        assert!(
            ctx.workspace_context_cap_chars() < old_cap,
            "the workspace cap re-derives from the live hard limit"
        );
        assert!(ctx.hard_limit + 2_048 <= 4_096, "input + output never over-commit");

        // A transcript budgeted for 16k now overflows 4k — and the existing
        // ladder (drop_oldest → truncate_to_limit, the no-LLM tail every
        // tier falls back to) brings it under the NEW hard limit.
        assert!(ctx.exceeds_hard_limit(), "the old transcript must overflow the demoted window");
        ctx.drop_oldest(8);
        ctx.truncate_to_limit();
        assert!(
            !ctx.exceeds_hard_limit(),
            "after the ladder the request fits the demoted window (est {} <= hard {})",
            ctx.estimate_request_tokens(),
            ctx.hard_limit
        );
        // The step frame survives every truncation path.
        assert!(matches!(
            &ctx.messages.first().expect("frame kept").content[0],
            ContentBlock::Text { text } if text == "the step frame"
        ));
    }

    // -----------------------------------------------------------------
    // P22 Tier 2 — the proactive trigger derives from measured headroom
    // -----------------------------------------------------------------

    /// The regression that forced the derivation: on a 16384-token window
    /// the fixed 40%-of-threshold rule fired 80× at 4,423 tokens with ~3.7k
    /// tokens of real headroom still free. With measured growth the trigger
    /// stays quiet there and fires only when the next interval could
    /// actually cross the threshold.
    #[test]
    fn proactive_trigger_derives_from_measured_headroom() {
        // The ~60%-of-window threshold of a 16384 window; the old failure
        // point was 4423 estimated with typical growth ~800/interval.
        let threshold = 9_830;
        assert!(
            !proactive_compression_due(4_423, 800, threshold),
            "must not fire with thousands of tokens of measured headroom"
        );
        // Near the ceiling the same growth says the next interval could cross.
        assert!(proactive_compression_due(9_200, 800, threshold));
        // Boundary: estimated + growth must EXCEED the threshold, not reach it.
        assert!(!proactive_compression_due(9_030, 800, threshold));
        assert!(proactive_compression_due(9_031, 800, threshold));
    }

    #[test]
    fn proactive_trigger_needs_evidence_and_defers_above_threshold() {
        // No growth measured yet → no evidence → never fires proactively.
        assert!(!proactive_compression_due(9_800, 0, 9_830));
        // Above the threshold the standard tier owns the problem.
        assert!(!proactive_compression_due(9_831, 800, 9_830));
    }

    #[test]
    fn growth_tracker_records_max_interval_growth_only_forward() {
        let mut tracker = ContextGrowthTracker::default();
        // First observation has no baseline: no growth, no evidence.
        assert_eq!(tracker.observe(5_000), 0);
        assert_eq!(tracker.max_observed_growth, 0);
        tracker.rebaseline(5_000);
        assert_eq!(tracker.observe(5_900), 900);
        // Compression shrank the context below the old baseline; shrinkage
        // never records as (negative) growth.
        tracker.rebaseline(4_000);
        assert_eq!(tracker.observe(4_200), 200);
        assert_eq!(
            tracker.max_observed_growth, 900,
            "the max survives smaller intervals"
        );
    }

    // -----------------------------------------------------------------
    // P22 Tier 2 — the verified-outcomes slot is monotone
    // -----------------------------------------------------------------

    #[test]
    fn identical_reverification_collapses_and_a_new_verdict_appends() {
        let mut ctx = AgentContext::new("s1");
        ctx.record_verified_outcome("sh tests/test_1.sh", "exit 0");
        ctx.record_verified_outcome("sh tests/test_1.sh", "exit 0");
        assert_eq!(
            ctx.verified_outcomes.len(),
            1,
            "identical assertions collapse"
        );
        assert_eq!(ctx.verified_outcomes[0].times, 2);
        // A later regression appends its OWN line; the pass record survives.
        ctx.record_verified_outcome("sh tests/test_1.sh", "exit 1");
        assert_eq!(ctx.verified_outcomes.len(), 2);
        assert_eq!(ctx.verified_outcomes[0].outcome, "exit 0");
        assert_eq!(ctx.verified_outcomes[1].outcome, "exit 1");
    }

    /// Seeding a fresh per-step context from the store's stored verdicts (the
    /// do-not-regress digest) must carry the verification time the STORE
    /// recorded, never "now" — a fact proven three turns ago that claims to
    /// have been proven this second is the same class of lie the slot exists to
    /// prevent. And the timestamp is monotone, so seeding an older assertion
    /// beside a fresh execution never ages the fresh evidence.
    #[test]
    fn seeded_outcomes_keep_the_stores_time_and_never_move_it_backwards() {
        let mut ctx = AgentContext::new("s1");
        ctx.record_verified_outcome_at("sh tests/test_1.sh", "exit 0", 1_700_000_000);
        assert_eq!(ctx.verified_outcomes[0].verified_at, 1_700_000_000);
        // A newer execution of the same fact advances the line.
        ctx.record_verified_outcome_at("sh tests/test_1.sh", "exit 0", 1_700_000_500);
        assert_eq!(ctx.verified_outcomes.len(), 1);
        assert_eq!(ctx.verified_outcomes[0].times, 2);
        assert_eq!(ctx.verified_outcomes[0].verified_at, 1_700_000_500);
        // An OLDER assertion collapses into it without aging it.
        ctx.record_verified_outcome_at("sh tests/test_1.sh", "exit 0", 1_600_000_000);
        assert_eq!(ctx.verified_outcomes[0].verified_at, 1_700_000_500);

        // A verdict the store recorded no time for says so, rather than
        // claiming the epoch.
        ctx.record_verified_outcome_at("#4 add the parser", "verified", 0);
        let block = ctx
            .verified_outcomes_block()
            .expect("the slot renders when non-empty");
        assert!(block.contains("time not recorded"), "{block}");
        assert!(!block.contains("1970-01-01"), "{block}");
    }

    /// The monotone guarantee: no compression path may drop a verified
    /// outcome. Drives the slot through every destructive operation the
    /// ladder can perform — summarization replacement, wholesale drops,
    /// hard-limit truncation, and a distillation round — then checks the
    /// facts still render in the assembled request.
    #[test]
    fn verified_outcomes_survive_every_compression_path() {
        let mut ctx = AgentContext::new("s1");
        ctx.record_verified_outcome("./minidb mset a 1", "exit 0");
        ctx.record_verified_outcome("sh tests/test_7.sh", "exit 0");
        for i in 0..30 {
            ctx.messages.push(AnthropicMessage::user_text(format!(
                "filler {i}: {}",
                "x".repeat(400)
            )));
        }

        ctx.replace_with_summary(5, "a summary of early work");
        ctx.drop_oldest(3);
        ctx.hard_limit = 50; // force truncation to bite as hard as it can
        ctx.truncate_to_limit();
        ctx.set_distilled_facts("current_state: testing");

        assert_eq!(
            ctx.verified_outcomes.len(),
            2,
            "no compression path may drop a verified outcome"
        );
        let request = ctx.messages_for_request();
        let ContentBlock::Text { text } = &request[0].content[0] else {
            panic!("preamble must be a text block");
        };
        assert!(text.contains("<verified_outcomes>"));
        assert!(text.contains("./minidb mset a 1"));
        assert!(text.contains("sh tests/test_7.sh"));
    }

    /// The 2026-08-10 destroyer: distillation used to overwrite the whole
    /// consolidated summary (2571→934 chars observed) with ≤512 tokens of
    /// facts about the last ten messages. Distilled facts now live in their
    /// own rolling slot; summarization products survive every round.
    #[test]
    fn distillation_no_longer_overwrites_the_consolidated_summary() {
        let mut ctx = AgentContext::new("s1");
        for i in 0..10 {
            ctx.messages
                .push(AnthropicMessage::user_text(format!("message {i}")));
        }
        ctx.replace_with_summary(4, "tests 1-10 verified passing; minidb built");
        let before = ctx.consolidated_summary.clone().expect("summary exists");

        ctx.set_distilled_facts("current_state: reading files");
        ctx.set_distilled_facts("current_state: writing test 11");

        assert_eq!(
            ctx.consolidated_summary.as_deref(),
            Some(before.as_str()),
            "distillation must never touch summarization products"
        );
        let request = ctx.messages_for_request();
        let ContentBlock::Text { text } = &request[0].content[0] else {
            panic!("preamble must be a text block");
        };
        assert!(text.contains("tests 1-10 verified passing"));
        assert!(text.contains("current_state: writing test 11"));
        assert!(
            !text.contains("reading files"),
            "the distilled slot itself is a rolling replace"
        );
    }

    #[test]
    fn verified_subject_preview_identifies_without_reproducing() {
        let heredoc = format!("python - <<'EOF'\n{}\nEOF", "x".repeat(5_000));
        let preview = verified_subject_preview(&heredoc);
        assert!(preview.starts_with("python - <<'EOF'"));
        assert!(!preview.contains('\n'), "one line per outcome");
        assert!(
            preview.contains("chars]"),
            "elision must announce the hidden length: {preview}"
        );
        let short = verified_subject_preview("cargo test -p nanna-agent");
        assert_eq!(
            short, "cargo test -p nanna-agent",
            "short subjects render whole"
        );
    }

    #[test]
    fn slot_costs_are_counted_in_request_estimates() {
        let mut ctx = AgentContext::new("s1");
        let base = ctx.estimate_request_tokens();
        ctx.record_verified_outcome("cargo test", "exit 0");
        ctx.set_distilled_facts("k: v");
        assert!(
            ctx.estimate_request_tokens() > base,
            "unbudgeted preamble tokens would overflow the window"
        );
    }

    #[test]
    fn old_serialized_contexts_deserialize_without_the_new_fields() {
        let json = serde_json::json!({
            "session_id": "s1",
            "system_prompt": "",
            "messages": [],
            "metadata": {},
            "max_messages": 100,
        });
        let ctx: AgentContext = serde_json::from_value(json).expect("pre-P22 contexts must load");
        assert!(ctx.verified_outcomes.is_empty());
        assert!(ctx.distilled_facts.is_none());
        assert_eq!(ctx.growth.max_observed_growth, 0);
    }

    // -----------------------------------------------------------------
    // P22 Tier 2 — failed summarization announces itself
    // -----------------------------------------------------------------

    /// Model-free failure path: no summarization models configured and the
    /// context over the hard limit — the fallback truncation must announce
    /// WHAT was dropped, WHY, and that disk is unaffected, instead of
    /// silently shrinking history.
    #[tokio::test]
    async fn unsummarized_truncation_announces_itself() {
        let mut ctx = AgentContext::new("s1");
        for i in 0..40 {
            ctx.messages.push(AnthropicMessage::user_text(format!(
                "filler {i}: {}",
                "y".repeat(300)
            )));
        }
        ctx.hard_limit = 200;
        let config = ContextSummarizationConfig {
            model_priority: vec![],
            ..Default::default()
        };
        ctx.enforce_limits_with_summarization(&config, SummarizationTarget::HardLimit)
            .await
            .expect("the no-model path cannot fail");

        let notices = ctx.take_pending_loss_notices();
        assert_eq!(notices.len(), 1, "one loss event, one announcement");
        assert!(notices[0].contains("WHAT:"), "{}", notices[0]);
        assert!(notices[0].contains("WHY:"), "{}", notices[0]);
        assert!(notices[0].contains("Disk is unaffected"), "{}", notices[0]);
        assert!(
            ctx.take_pending_loss_notices().is_empty(),
            "taking drains the queue"
        );
    }

    #[test]
    fn no_loss_means_no_notice() {
        let mut ctx = AgentContext::new("s1");
        ctx.push_summarization_failure_notice(0, "anything");
        assert!(
            ctx.take_pending_loss_notices().is_empty(),
            "announcing zero loss would be noise"
        );
    }
}
