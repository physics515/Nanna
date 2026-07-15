//! Agent context management

use crate::chunker::Chunk;
use nanna_llm::{AnthropicMessage, AnthropicRequest, ContentBlock, LlmClient, ModelInfo, RequestBuilder};
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
    /// Whether to include MEMORY.md in workspace context (false for group chats)
    #[serde(default = "default_include_memory")]
    pub include_workspace_memory: bool,
    /// Consolidated summary of all previously summarized messages.
    /// This is prepended to messages when building API requests.
    #[serde(default)]
    pub consolidated_summary: Option<String>,
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

        let raw = if let Some(ref summary) = self.consolidated_summary {
            let mut messages = Vec::with_capacity(deduped_messages.len() + 2);

            // Inject summary as first user message with clear framing
            let summary_message = format!(
                "<previous_context>\nThe following is a summary of earlier conversation that has been compressed to save context space:\n\n{}\n</previous_context>",
                summary
            );
            messages.push(AnthropicMessage::user_text(summary_message));

            // Add a placeholder assistant acknowledgment to maintain user/assistant alternation
            messages.push(AnthropicMessage::assistant_text(
                "I understand the previous context. How can I help you continue?"
            ));

            // Then add deduplicated current messages
            messages.extend(deduped_messages);
            messages
        } else {
            // No summary, just return (possibly deduplicated) messages
            deduped_messages
        };

        // Sanitize: remove empty text blocks and ensure every message has content
        Self::sanitize_messages(raw)
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

        summary_tokens + self.estimate_tokens()
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
    pub fn configure_for_model(&mut self, model_info: &ModelInfo) -> bool {
        let model_changed = self.current_model.as_ref() != Some(&model_info.id);

        if model_changed {
            info!(
                model = %model_info.id,
                context_window = model_info.context_window,
                compression_threshold = model_info.compression_threshold(),
                hard_limit = model_info.hard_input_limit(),
                "Configuring context for model"
            );
        }

        self.compression_threshold = model_info.compression_threshold();
        self.hard_limit = model_info.hard_input_limit();
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
        self.include_workspace_memory = workspace.config.include_memory;
        self
    }

    /// Set workspace from files directly
    #[must_use]
    pub fn with_workspace_files(mut self, root: PathBuf, files: &WorkspaceFiles, include_memory: bool) -> Self {
        self.workspace_root = Some(root);
        self.workspace_context = Some(files.to_system_context(include_memory));
        self.include_workspace_memory = include_memory;
        self
    }

    /// Set whether to include MEMORY.md in workspace context
    #[must_use]
    pub fn with_workspace_memory(mut self, include: bool) -> Self {
        self.include_workspace_memory = include;
        self
    }

    /// Get the effective system prompt (base + workspace context)
    #[must_use]
    pub fn effective_system_prompt(&self) -> String {
        match &self.workspace_context {
            Some(ws_ctx) if !ws_ctx.is_empty() => {
                if self.system_prompt.is_empty() {
                    ws_ctx.clone()
                } else {
                    format!("{}\n\n{}", self.system_prompt, ws_ctx)
                }
            }
            _ => self.system_prompt.clone(),
        }
    }

    /// Reload workspace context from disk
    ///
    /// # Errors
    /// Returns error if workspace cannot be loaded
    pub async fn reload_workspace(&mut self) -> Result<(), nanna_workspace::WorkspaceError> {
        if let Some(ref root) = self.workspace_root {
            let files = WorkspaceFiles::load(root).await;
            self.workspace_context = Some(files.to_system_context(self.include_workspace_memory));
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
        let system_tokens = estimate_token_count(self.system_prompt.len());
        let summary_tokens: usize = self.summaries.iter().map(|s| estimate_token_count(s.summary.len())).sum();
        let message_tokens: usize = self
            .messages
            .iter()
            .map(|m| {
                m.content
                    .iter()
                    .map(|c| match c {
                        ContentBlock::Text { text } => estimate_token_count(text.len()),
                        ContentBlock::ToolUse { input, .. } => {
                            estimate_token_count(input.to_string().len()) + 50
                        }
                        ContentBlock::ToolResult { content, .. } => estimate_token_count(content.len()) + 20,
                        ContentBlock::Image { .. } => 1000, // Images are ~1k tokens
                        ContentBlock::Thinking { thinking, .. } => estimate_token_count(thinking.len()),
                    })
                    .sum::<usize>()
            })
            .sum();

        system_tokens + summary_tokens + message_tokens
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
    ) -> Result<usize, String> {
        if config.model_priority.is_empty() {
            // No summarization configured, fall back to truncation
            if self.exceeds_hard_limit() {
                warn!("No summarization models configured, truncating context");
                self.truncate_to_limit();
            }
            return Ok(0);
        }

        let mut iterations = 0;
        // Candidate summarizers resolve and enforce their own provider-reported limits.
        // This only bounds extraction to the context already held by this agent.
        let max_chars_per_chunk = self.hard_limit.saturating_mul(4);

        while self.exceeds_hard_limit() && iterations < config.max_iterations {
            iterations += 1;

            let current_tokens = self.estimate_tokens();
            info!(
                iteration = iterations,
                current_tokens = current_tokens,
                hard_limit = self.hard_limit,
                "Context exceeds limit, summarizing"
            );

            // Find content to summarize (oldest messages first, keeping most recent)
            let content_to_summarize = self.extract_content_for_summarization(max_chars_per_chunk);

            if content_to_summarize.is_empty() {
                warn!("No content available to summarize, truncating remaining");
                self.truncate_to_limit();
                break;
            }

            // Try to summarize with fallback
            match Self::summarize_content_with_fallback(&content_to_summarize, config).await
            {
                Ok(summary) => {
                    info!(
                        original_len = content_to_summarize.len(),
                        summary_len = summary.len(),
                        compression = format!(
                            "{:.1}x",
                            content_to_summarize.len() as f64 / summary.len().max(1) as f64
                        ),
                        "Content summarized successfully"
                    );

                    // Replace the summarized content with the summary
                    self.replace_with_summary(&content_to_summarize, &summary);
                }
                Err(e) => {
                    warn!(error = %e, "All summarization models failed, truncating");
                    self.truncate_to_limit();
                    break;
                }
            }
        }

        if iterations >= config.max_iterations && self.exceeds_hard_limit() {
            warn!(
                iterations = iterations,
                "Max summarization iterations reached, force truncating"
            );
            self.truncate_to_limit();
        }

        Ok(iterations)
    }

    /// Extract content from oldest messages for summarization
    fn extract_content_for_summarization(&self, max_chars: usize) -> String {
        let mut content = String::new();
        let mut chars_collected = 0;

        // Keep at least the last 2 messages (user query + assistant response in progress)
        let messages_to_consider = if self.messages.len() > 2 {
            &self.messages[..self.messages.len() - 2]
        } else {
            return String::new(); // Not enough messages to summarize
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
                    return content;
                }

                content.push_str(&format!("[{}]: {}\n", msg.role, block_text));
                chars_collected += block_text.len();
            }
        }

        content
    }

    /// Try to summarize content using models in priority order
    async fn summarize_content_with_fallback(
        content: &str,
        config: &ContextSummarizationConfig,
    ) -> Result<String, String> {
        for model_spec in &config.model_priority {
            debug!(model = %model_spec, "Attempting summarization");

            match Self::try_summarize_with_model(model_spec, content, config).await {
                Ok(summary) => {
                    info!(model = %model_spec, "Summarization succeeded");
                    return Ok(summary);
                }
                Err(e) => {
                    warn!(model = %model_spec, error = %e, "Summarization failed, trying next");
                }
            }
        }

        Err("All summarization models failed".to_string())
    }

    /// Try to summarize with a specific model via direct LLM call
    async fn try_summarize_with_model(
        model_spec: &str,
        content: &str,
        config: &ContextSummarizationConfig,
    ) -> Result<String, String> {
        let (client, model_name) = Self::create_client_for_model(model_spec, config)?;

        // Fetch this fallback model's own limits; summarizers may have radically
        // different windows even when they are in the same priority list.
        let cache = nanna_llm::ModelInfoCache::default_location();
        let model_info = client.get_model_info(&model_name, cache.as_ref()).await;
        // Reserve output capacity and prompt framing before filling the input.
        let max_chars = model_info.hard_input_limit().saturating_sub(512).saturating_mul(4);
        let truncated = if content.len() > max_chars {
            &content[..content.floor_char_boundary(max_chars)]
        } else {
            content
        };

        let prompt = format!(
            "Summarize the following conversation history concisely. Preserve key facts, decisions, \
             file paths, code snippets, and important context needed to continue the conversation.\n\n\
             ---\n{}\n---\n\nProvide a concise summary:",
            truncated
        );

        let request = AnthropicRequest {
            model: model_name,
            messages: vec![AnthropicMessage::user_text(prompt)],
            max_tokens: u32::try_from(model_info.max_output_tokens.min(2_048)).unwrap_or(u32::MAX),
            temperature: Some(0.3),
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
    fn replace_with_summary(&mut self, _original_content: &str, summary: &str) {
        // Remove old messages that were summarized (keep last 2)
        let keep_count = 2.min(self.messages.len());
        let remove_count = self.messages.len().saturating_sub(keep_count);

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
}
