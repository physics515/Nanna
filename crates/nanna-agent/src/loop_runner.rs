//! Agent loop runner

use crate::{AgentContext, AgentError, ContextSummarizationConfig, prompts};
use nanna_llm::{
    AnthropicMessage, AnthropicRequest, CacheControl, ContentBlock, ImageSource, LlmClient, StreamEvent,
    ToolDefinition as LlmToolDef,
};
use nanna_tools::{ToolCall, ToolRegistry, OutputTarget};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Core tools always sent to the LLM. Everything else is discoverable via `discover_tools`.
const CORE_TOOL_NAMES: &[&str] = &["remember", "recall", "reflect", "discover_tools"];

/// Thinking mode for extended reasoning
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThinkingMode {
    /// No explicit thinking - fast responses
    #[default]
    Instant,
    /// Low thinking budget (1024 tokens)
    Low,
    /// Medium thinking budget (4096 tokens)
    Medium,
    /// High thinking budget (8192 tokens)
    High,
    /// Maximum thinking budget (16384 tokens)
    Maximum,
}

impl ThinkingMode {
    /// Get the thinking budget in tokens for this mode
    #[must_use]
    pub const fn budget_tokens(&self) -> Option<u32> {
        match self {
            Self::Instant => None,
            Self::Low => Some(1024),
            Self::Medium => Some(4096),
            Self::High => Some(8192),
            Self::Maximum => Some(16384),
        }
    }

    /// Check if thinking is enabled
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        !matches!(self, Self::Instant)
    }
}

/// Agent configuration
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Model to use (primary / most capable model)
    pub model: String,
    /// Maximum tokens in response
    pub max_tokens: u32,
    /// Temperature for sampling
    pub temperature: f32,
    /// Maximum iterations (tool call rounds). None = unlimited.
    pub max_iterations: Option<usize>,
    /// Thinking mode for extended reasoning
    pub thinking_mode: ThinkingMode,
    /// Model priority list for summarization (first working model is used)
    /// Format: "provider/model" e.g. ["ollama/llama3.2", "openai/gpt-4o-mini", "anthropic/claude-haiku"]
    pub summarization_priority: Vec<String>,
    /// Ollama URL for summarization (if using ollama)
    pub summarization_ollama_url: Option<String>,
    /// OpenRouter API key (for summarization/extraction via OpenRouter models)
    pub openrouter_api_key: Option<String>,
    /// OpenAI API key (for summarization/extraction via OpenAI models)
    pub openai_api_key: Option<String>,
    /// Threshold (in chars) above which tool results are replaced with a
    /// memory-reference stub in context. 0 = auto (scales with model context window).
    /// Default: 0 (auto).
    pub context_result_threshold: usize,
    /// Progressive context distillation interval (in iterations).
    /// Every N iterations, the agent produces a rolling structured summary of the conversation.
    /// 0 = disabled. Default: 0 (uses existing threshold-based summarization only).
    pub distillation_interval: usize,
    /// Model routing: prioritized list of models for cost optimization.
    /// Each entry is "provider/model:tier" where tier is simple|medium|complex.
    /// When enabled, the agent classifies each iteration's complexity and routes
    /// to the cheapest model capable of handling it.
    /// Empty = disabled (always use primary model).
    /// Example: ["claude-haiku-3-5-20241022:simple", "claude-sonnet-4-20250514:complex"]
    pub model_routing: Vec<ModelTier>,
    /// Whether to always use the primary model for the first iteration
    /// (user-facing response quality). Default: true.
    pub routing_first_turn_primary: bool,
    /// Model to use for sub-agent tasks (optional).
    /// When set, sub-agents spawned via the `task` tool use this model instead of the primary.
    /// Use a cheaper model here to reduce costs for delegated sub-tasks.
    /// Format: "provider/model" e.g. "ollama/qwen3:4b" or "claude-3-5-haiku-20241022"
    pub sub_agent_model: Option<String>,
}

/// A model with its maximum complexity tier for routing purposes.
#[derive(Debug, Clone)]
pub struct ModelTier {
    /// Full model spec (may include provider prefix e.g. "ollama/deepseek-r1:14b")
    pub model: String,
    /// Maximum complexity this model should handle
    pub tier: TaskComplexity,
}

impl ModelTier {
    /// Parse from "model:tier" format. If no tier specified, defaults to Complex.
    pub fn parse(spec: &str) -> Self {
        if let Some((model, tier_str)) = spec.rsplit_once(':') {
            // Check if this looks like a tier annotation vs a tag (e.g. "deepseek-r1:14b")
            let tier = match tier_str.to_lowercase().as_str() {
                "simple" => Some(TaskComplexity::Simple),
                "medium" => Some(TaskComplexity::Medium),
                "complex" => Some(TaskComplexity::Complex),
                _ => None, // Not a tier, treat whole thing as model name
            };
            if let Some(tier) = tier {
                return Self { model: model.to_string(), tier };
            }
        }
        Self { model: spec.to_string(), tier: TaskComplexity::Complex }
    }
}

/// Task complexity level for model routing decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskComplexity {
    /// Direct tool calls, simple Q&A, acknowledgments
    Simple = 0,
    /// Multi-step reasoning, code generation, summarization
    Medium = 1,
    /// Novel problem solving, long-form analysis, ambiguous requests
    Complex = 2,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-20250514".to_string(),
            max_tokens: 8192,
            temperature: 0.7,
            max_iterations: None,
            thinking_mode: ThinkingMode::Instant,
            summarization_priority: vec![],
            summarization_ollama_url: Some("http://localhost:11434".to_string()),
            openrouter_api_key: None,
            openai_api_key: None,
            context_result_threshold: 0, // 0 = auto (scales with model context window)
            distillation_interval: 5,
            model_routing: vec![],
            routing_first_turn_primary: true,
            sub_agent_model: None,
        }
    }
}

/// Callback for streaming text chunks
pub type StreamCallback = Box<dyn Fn(&str) + Send + Sync>;

/// Callback for storing extracted memories
pub type MemoryCallback = Box<dyn Fn(ExtractedMemory) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;

/// Callback for streaming thinking chunks
pub type ThinkingCallback = Box<dyn Fn(&str) + Send + Sync>;

/// Callback for tool start events (called with tool call id, name, and input)
pub type ToolStartCallback = Box<dyn Fn(&str, &str, &Value) + Send + Sync>;
/// Callback for tool completion: (call_id, name, output, success, duration_ms)
pub type ToolEndCallback = Box<dyn Fn(&str, &str, &str, bool, u64) + Send + Sync>;

/// Options for running the agent
#[derive(Default)]
pub struct RunOptions {
    /// Override max iterations for this run
    pub max_iterations: Option<usize>,
    /// Additional context to prepend
    pub context_prefix: Option<String>,
    /// Callback for streaming text (called with each text chunk)
    pub on_text: Option<StreamCallback>,
    /// Callback for streaming thinking/reasoning (called with each thinking chunk)
    pub on_thinking: Option<ThinkingCallback>,
    /// Auto-extract memories after each run
    pub auto_extract_memories: bool,
    /// Callback for storing extracted memories (required if auto_extract_memories is true)
    pub on_memory: Option<MemoryCallback>,
    /// Enable uncertainty/confidence tracking
    pub track_uncertainty: bool,
    /// Enable emotional context analysis
    pub track_emotions: bool,
    /// Override thinking mode for this run
    pub thinking_mode: Option<ThinkingMode>,
    /// Token budget for this run (total input + output tokens allowed)
    pub token_budget: Option<u64>,
    /// If true, inject budget awareness note into system prompt
    pub budget_awareness: bool,
    /// Callback for tool start events (called before each tool execution)
    pub on_tool_start: Option<ToolStartCallback>,
    /// Callback for tool end events (called after each tool execution)
    pub on_tool_end: Option<ToolEndCallback>,
    /// Shared flag for cooperative cancellation (set to true to stop the agent loop)
    pub cancellation_flag: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Image attachments for the current message: Vec<(base64_data, media_type)>
    pub attachments: Vec<(String, String)>,
}

/// Response from an agent run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    /// Final text response
    pub text: String,
    /// Tool calls made during this run
    pub tool_calls: Vec<ToolCallRecord>,
    /// Number of iterations used
    pub iterations: usize,
    /// Whether the agent hit max iterations
    pub truncated: bool,
    /// Token usage
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Confidence level (0.0-1.0) if uncertainty tracking is enabled
    pub confidence: Option<f32>,
    /// Emotional context detected in the conversation
    pub emotional_context: Option<EmotionalContext>,
    /// Reasoning/thinking content (if thinking mode was enabled)
    pub reasoning: Option<ReasoningContent>,
    /// Cumulative input tokens across all iterations
    pub cumulative_input_tokens: u64,
    /// Cumulative output tokens across all iterations
    pub cumulative_output_tokens: u64,
    /// Per-iteration model stats (for UI display)
    pub model_stats: Vec<crate::model_stats::RequestModelStats>,
}

/// Reasoning content from thinking mode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningContent {
    /// The full reasoning/thinking text
    pub content: String,
    /// Reasoning tokens used
    pub tokens: u32,
    /// Interleaved reasoning blocks (between tool calls)
    pub blocks: Vec<ReasoningBlock>,
}

/// A single reasoning block (interleaved between actions)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningBlock {
    /// The reasoning text for this block
    pub content: String,
    /// Which iteration this occurred in
    pub iteration: usize,
    /// Whether this was before a tool call
    pub before_tool: Option<String>,
}

/// Emotional context detected in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmotionalContext {
    /// Primary emotion (e.g., "neutral", "frustrated", "excited", "confused")
    pub primary_emotion: String,
    /// Intensity (0.0-1.0)
    pub intensity: f32,
    /// Suggested response tone adjustment
    pub suggested_tone: Option<String>,
}

/// Record of a tool call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub id: String,
    pub name: String,
    pub input: Value,
    pub output: String,
    pub success: bool,
    pub duration_ms: u64,
}

/// Internal result from LLM call
struct LlmResult {
    text: String,
    tool_uses: Vec<(String, String, Value)>,
    content_blocks: Vec<ContentBlock>,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_creation_tokens: u32,
    /// Error tool results from malformed JSON parsing failures.
    /// These need to be sent back to the model so it knows the call failed.
    error_tool_results: Vec<ContentBlock>,
}

/// Detect degenerate narration loops in streaming text.
///
/// Returns `true` if the text shows signs of the model narrating tool usage
/// without actually emitting tool calls (a known failure mode).
///
/// Two detection strategies:
/// 1. **Repeated intent phrases**: The model keeps saying "let me X" without doing X.
/// 2. **Phantom completion**: The model claims to have read/written/verified files
///    without any tool calls — it hallucinated the entire workflow.
fn detect_narration_loop(text: &str) -> bool {
    let lower = text.to_lowercase();

    // ── Strategy 1: Repeated intent-to-act phrases ──
    // Phrases that indicate the model is *talking about* using tools
    const NARRATION_PHRASES: &[&str] = &[
        "let me read",
        "let me look",
        "let me find",
        "let me list",
        "let me examine",
        "let me check",
        "let me start",
        "let me try",
        "let me write",
        "let me rewrite",
        "let me update",
        "let me verify",
        "let me create",
        "let me modify",
        "let me open",
        "let me review",
        "let me fix",
        "let me see",
        "now let me",
        "now i'll",
        "i'll start by",
        "i'll read",
        "i'll look",
        "i'll examine",
        "i'll list",
        "i'll write",
        "i'll rewrite",
        "i'll update",
        "i'll check",
        "i'll verify",
        "i'll create",
        "i'll modify",
        "i need to",
        "i should read",
        "i should look",
        "i should check",
        "i should write",
        "reading the file",
        "listing the directory",
        "executing the command",
        "running the command",
        "writing the file",
        "rewriting the file",
        "use my tools",
        "invoke my tools",
        "actually execute",
        "actually use",
    ];

    // Count total hits across all phrases (not just unique phrases with 2+ hits)
    let total_hits: usize = NARRATION_PHRASES
        .iter()
        .map(|p| lower.matches(p).count())
        .sum();

    // If 4+ total narration phrase hits, it's narrating instead of acting
    if total_hits >= 4 {
        return true;
    }

    // Also keep the old check: 2+ different phrases each appearing 2+ times
    let distinct_repeated = NARRATION_PHRASES
        .iter()
        .filter(|p| lower.matches(*p).count() >= 2)
        .count();
    if distinct_repeated >= 2 {
        return true;
    }

    // ── Strategy 2: Phantom completion ──
    // The model claims it performed file operations and reports success,
    // but no tool calls were actually made. This catches the pattern where
    // a weak model says "I've rewritten the file... verified... it's clean"
    // without ever calling write_file.
    const COMPLETION_CLAIMS: &[&str] = &[
        "the file is clean",
        "the file is now",
        "the rewrite is complete",
        "the redesign is complete",
        "successfully wrote",
        "successfully updated",
        "successfully created",
        "successfully modified",
        "file has been updated",
        "file has been rewritten",
        "file has been modified",
        "changes are complete",
        "changes are done",
        "the update is complete",
        "here's what i changed",
        "i've rewritten",
        "i've updated",
        "i've modified",
        "i've created the",
        "i've written the",
        "verified the",
        "file is correct",
        "it now uses only",
    ];

    const ACTION_CLAIMS: &[&str] = &[
        "let me read",
        "let me check",
        "let me verify",
        "let me write",
        "let me rewrite",
        "now let me",
    ];

    let completion_hits = COMPLETION_CLAIMS.iter().filter(|p| lower.contains(*p)).count();
    let action_hits = ACTION_CLAIMS.iter().filter(|p| lower.contains(*p)).count();

    // If the model both narrates actions AND claims completion, it hallucinated the workflow
    if completion_hits >= 1 && action_hits >= 2 {
        return true;
    }

    false
}

/// Detect repetitive text by checking if the same line appears multiple times.
/// Returns `true` if significant repetition is found.
fn detect_repetition(text: &str) -> bool {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| l.len() > 20)
        .collect();

    if lines.len() < 6 {
        return false;
    }

    let mut seen: HashMap<&str, usize> = HashMap::new();
    for line in &lines {
        *seen.entry(line).or_insert(0) += 1;
    }

    // If any substantial line appears 3+ times, it's repetitive
    seen.values().any(|&count| count >= 3)
}

/// Detect thinking spiral: the model's reasoning is going in circles without
/// making progress. Catches analysis paralysis where the model repeatedly
/// considers the same options, re-asks the same questions, or flip-flops
/// between approaches.
fn detect_thinking_spiral(thinking: &str) -> bool {
    // Must have enough thinking to check (at least ~2000 chars)
    if thinking.len() < 2000 {
        return false;
    }

    let lower = thinking.to_lowercase();

    // Indicator 1: Repeated deliberation phrases that signal circular reasoning
    const SPIRAL_PHRASES: &[&str] = &[
        "wait, but",
        "but wait",
        "hmm, the problem",
        "let me think",
        "let me reconsider",
        "alternatively,",
        "on the other hand",
        "so the correct",
        "so the answer",
        "so maybe",
        "but the user",
        "but since",
        "but the tools",
        "but the instructions",
        "but according to",
        "wait, the user",
        "wait, the problem",
        "so i should",
        "so the tool call",
        "so the response should",
        "the tools provided",
        "the available functions",
        "the functions are",
        "i don't have access",
        "i can't access",
        "i don't have the ability",
        "i don't have that",
    ];

    let spiral_matches = SPIRAL_PHRASES
        .iter()
        .filter(|p| lower.matches(*p).count() >= 2)
        .count();

    // If 4+ different spiral phrases appear at least twice, it's spinning
    if spiral_matches >= 4 {
        return true;
    }

    // Indicator 2: Sentence-level repetition in thinking (same sentence reappears 3+ times)
    let sentences: Vec<&str> = lower
        .split(|c: char| c == '.' || c == '?' || c == '!')
        .map(str::trim)
        .filter(|s| s.len() > 30)
        .collect();

    if sentences.len() >= 6 {
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for s in &sentences {
            *seen.entry(s).or_insert(0) += 1;
        }
        if seen.values().any(|&count| count >= 3) {
            return true;
        }
    }

    // Indicator 3: The thinking text is very long but contains no tool-call intent markers
    // (model is just philosophizing about what to do, not converging on action)
    if thinking.len() > 4000 {
        let has_tool_call_intent = lower.contains("tool_call")
            || lower.contains("function name")
            || lower.contains("\"name\":")
            || lower.contains("arguments");
        let reconsider_count = lower.matches("wait,").count()
            + lower.matches("but ").count()
            + lower.matches("alternatively").count()
            + lower.matches("however").count();
        // Lots of hedging + no convergence on a tool call = spiral
        if !has_tool_call_intent && reconsider_count >= 8 {
            return true;
        }
    }

    false
}

/// The main agent
pub struct Agent {
    config: AgentConfig,
    llm: Arc<LlmClient>,
    tools: Arc<ToolRegistry>,
    context: RwLock<AgentContext>,
    /// Optional model statistics tracker (shared across sessions)
    stats: Option<crate::model_stats::ModelStatsTracker>,
    /// Optional tool statistics tracker (shared across sessions)
    tool_stats: Option<crate::tool_stats::ToolStatsTracker>,
}

impl Agent {
    pub fn new(config: AgentConfig, llm: Arc<LlmClient>, tools: Arc<ToolRegistry>) -> Self {
        let context = AgentContext::new(Uuid::new_v4().to_string())
            .with_system_prompt(prompts::DEFAULT_SYSTEM_PROMPT);

        Self {
            config,
            llm,
            tools,
            context: RwLock::new(context),
            stats: None,
            tool_stats: None,
        }
    }

    /// Set a shared model stats tracker.
    pub fn with_stats(mut self, stats: crate::model_stats::ModelStatsTracker) -> Self {
        self.stats = Some(stats);
        self
    }

    /// Set a shared tool stats tracker.
    pub fn with_tool_stats(mut self, tool_stats: crate::tool_stats::ToolStatsTracker) -> Self {
        self.tool_stats = Some(tool_stats);
        self
    }

    /// Set the agent context.
    #[must_use]
    pub fn with_context(mut self, context: AgentContext) -> Self {
        self.context = RwLock::new(context);
        self
    }

    /// Run the agent with a user message.
    ///
    /// # Errors
    ///
    /// Returns `AgentError::Llm` if the LLM request fails.
    /// Returns `AgentError::Tool` if a tool execution fails critically.
    pub async fn run(
        &self,
        message: &str,
        options: RunOptions,
    ) -> Result<AgentResponse, AgentError> {
        // Resolve effective max_iterations: run option > config > unlimited
        let max_iterations = options.max_iterations.or(self.config.max_iterations);
        let mut state = RunState::new();

        // Add user message with optional budget awareness
        self.add_user_message_with_budget(message, &options).await;

        // Agent loop
        loop {
            // Check cancellation flag
            if let Some(ref flag) = options.cancellation_flag {
                if flag.load(std::sync::atomic::Ordering::Relaxed) {
                    info!("Agent cancelled by user");
                    state.final_text.push_str("\n\n[Cancelled by user]");
                    return Ok(state.into_response(true));
                }
            }

            state.iterations += 1;
            if let Some(max) = max_iterations {
                if state.iterations > max {
                    // Extract memories before bailing — don't lose a long run's knowledge
                    if options.auto_extract_memories {
                        if let Some(ref on_memory) = options.on_memory {
                            if let Ok(memories) = self.extract_memories().await {
                                for memory in memories {
                                    on_memory(memory).await;
                                }
                            }
                        }
                    }
                    warn!(
                        iterations = state.iterations,
                        max = max,
                        final_text_len = state.final_text.len(),
                        "Agent hit max iterations limit, returning truncated response"
                    );
                    return Ok(state.into_response(true));
                }
            }

            // Progressive wrap-up nudges: gentle at first, increasingly urgent.
            // Starts at iteration 25, then every 10 iterations with escalating tone.
            // This helps the model stay focused without imposing a hard cap.
            {
                let i = state.iterations;
                let nudge_msg = if i == 25 && state.wrapup_nudge_count == 0 {
                    Some("[SYSTEM: You've made 25 tool calls. Consider whether you have enough \
                          information to answer. If so, synthesize your response now rather than \
                          gathering more data.]")
                } else if i == 35 && state.wrapup_nudge_count <= 1 {
                    Some("[SYSTEM: You've made 35 tool calls. You likely have enough information. \
                          Stop exploring and respond to the user with what you know. \
                          It's better to give a good answer now than a perfect answer never.]")
                } else if i >= 45 && (i - 45) % 5 == 0 {
                    Some("[SYSTEM: URGENT — You have made a very large number of tool calls. \
                          STOP calling tools and respond to the user IMMEDIATELY with whatever \
                          you have. Do NOT make any more tool calls.]")
                } else {
                    None
                };

                if let Some(msg) = nudge_msg {
                    state.wrapup_nudge_count += 1;
                    info!(
                        iteration = i,
                        nudge_count = state.wrapup_nudge_count,
                        "⏰ Injecting wrap-up nudge (level {})",
                        state.wrapup_nudge_count
                    );
                    let nudge = AnthropicMessage::user_text(msg);
                    let mut ctx = self.context.write().await;
                    ctx.messages.push(nudge);
                }
            }

            debug!(
                iterations = state.iterations,
                max = ?max_iterations,
                "Agent iteration"
            );

            // Tiered context compression before API call
            {
                let mut ctx = self.context.write().await;
                let estimated = ctx.estimate_tokens();
                let compression_threshold = ctx.compression_threshold;
                let hard_limit = ctx.hard_limit;
                let proactive_threshold = compression_threshold * 40 / 100; // ~64K for 160K threshold

                // Tier 1 (proactive): Every 5 iterations, if >40% of compression_threshold
                if state.iterations > 1
                    && state.iterations % 5 == 0
                    && estimated > proactive_threshold
                    && estimated <= compression_threshold
                {
                    info!(
                        estimated_tokens = estimated,
                        proactive_threshold = proactive_threshold,
                        tier = "proactive",
                        "Tier 1: proactive compression triggered"
                    );
                    let dropped = ctx.drop_oldest(6);
                    if dropped > 0 {
                        info!(dropped_messages = dropped, "Tier 1 compression complete");
                    }
                }

                // Tier 2 (standard): When exceeding compression_threshold, full summarization if available
                if ctx.needs_compression() && !ctx.exceeds_hard_limit() {
                    if !self.config.summarization_priority.is_empty() {
                        let summarization_config = ContextSummarizationConfig {
                            model_priority: self.config.summarization_priority.clone(),
                            ollama_url: self.config.summarization_ollama_url.clone(),
                            summarizer_context_window: 8000,
                            max_iterations: 20,
                        };
                        info!(
                            estimated_tokens = estimated,
                            compression_threshold = compression_threshold,
                            tier = "standard",
                            "Tier 2: standard summarization triggered"
                        );
                        match ctx.enforce_limits_with_summarization(&summarization_config).await {
                            Ok(iterations) if iterations > 0 => {
                                info!(iterations = iterations, new_tokens = ctx.estimate_tokens(), "Tier 2 summarization complete");
                            }
                            Ok(_) => {}
                            Err(e) => {
                                warn!(error = %e, "Tier 2 summarization failed, dropping oldest");
                                ctx.drop_oldest(6);
                            }
                        }
                    } else {
                        info!(
                            estimated_tokens = estimated,
                            compression_threshold = compression_threshold,
                            tier = "standard",
                            "Tier 2: no summarization models, dropping oldest"
                        );
                        ctx.drop_oldest(6);
                    }
                }

                // Tier 3 (hard cap): When exceeding hard_limit, aggressive truncation
                if ctx.exceeds_hard_limit() {
                    let estimated = ctx.estimate_tokens();

                    if !self.config.summarization_priority.is_empty() {
                        let summarization_config = ContextSummarizationConfig {
                            model_priority: self.config.summarization_priority.clone(),
                            ollama_url: self.config.summarization_ollama_url.clone(),
                            summarizer_context_window: 8000,
                            max_iterations: 20,
                        };
                        warn!(
                            estimated_tokens = estimated,
                            hard_limit = hard_limit,
                            tier = "hard_cap",
                            "Tier 3: hard limit exceeded, aggressive summarization"
                        );
                        match ctx.enforce_limits_with_summarization(&summarization_config).await {
                            Ok(iterations) if iterations > 0 => {
                                info!(iterations = iterations, new_tokens = ctx.estimate_tokens(), "Tier 3 summarization complete");
                            }
                            Ok(_) => {}
                            Err(e) => {
                                warn!(error = %e, "Tier 3 summarization failed, truncating");
                                ctx.truncate_to_limit();
                            }
                        }
                    } else {
                        warn!(
                            estimated_tokens = estimated,
                            hard_limit = hard_limit,
                            tier = "hard_cap",
                            "Tier 3: hard limit exceeded, truncating"
                        );
                        ctx.truncate_to_limit();
                    }
                }
            }

            // Model routing: classify complexity and pick cheapest capable model
            let routed_model = self.route_model(&state).await;

            // Build and execute LLM request
            let mut request = self.build_request_with_thinking(options.thinking_mode, &state.active_tools).await;
            if let Some(ref routed) = routed_model {
                // Strip provider prefix for the API request model field
                // but keep the full spec for client routing
                if let Some((_provider, model_name)) = routed.split_once('/') {
                    request.model = model_name.to_string();
                } else {
                    request.model = routed.clone();
                }
                // Update cache_control based on new model
                request.cache_control = if request.model.starts_with("claude") {
                    Some(CacheControl::ephemeral())
                } else {
                    None
                };
            }
            let complexity = if routed_model.is_some() {
                Some(self.classify_complexity(&state).await)
            } else {
                None
            };

            // Call LLM with escalation: if a routed (cheap) model fails, retry with primary
            let llm_start = std::time::Instant::now();
            let mut result = self.call_llm(&request, &options, &mut state).await;
            let mut llm_latency = llm_start.elapsed();
            let mut escalated = false;

            // Escalation: if routed model failed or returned malformed tool calls, retry with primary
            if routed_model.is_some() {
                let should_escalate = match &result {
                    Err(_) => true,
                    Ok(r) => {
                        // Check for malformed tool calls (empty name or unparseable JSON)
                        r.tool_uses.iter().any(|(_, name, _)| name.is_empty())
                    }
                };
                if should_escalate {
                    warn!(
                        failed_model = %request.model,
                        "⬆️ Escalating: routed model failed, retrying with primary model"
                    );
                    // Record failure on the cheap model
                    if let Some(ref tracker) = self.stats {
                        tracker.record(crate::model_stats::RequestObservation {
                            model: request.model.clone(),
                            success: false,
                            latency: llm_latency,
                            input_tokens: 0,
                            output_tokens: 0,
                            cache_read_tokens: 0,
                            cache_creation_tokens: 0,
                            tier: complexity,
                            escalated: false,
                        }).await;
                    }
                    // Rebuild request with primary model
                    request.model = self.config.model.clone();
                    request.cache_control = if request.model.starts_with("claude") {
                        Some(CacheControl::ephemeral())
                    } else {
                        None
                    };
                    let escalation_start = std::time::Instant::now();
                    result = self.call_llm(&request, &options, &mut state).await;
                    llm_latency = escalation_start.elapsed();
                    escalated = true;
                }
            }

            let result = result?;

            // Record model statistics
            let actual_model = request.model.clone();
            let was_routed = routed_model.is_some() && !escalated;
            let tier_label = if escalated {
                "escalated".to_string()
            } else {
                complexity.map_or("primary".to_string(), |c| format!("{c:?}").to_lowercase())
            };

            state.model_stats.push(crate::model_stats::RequestModelStats {
                model: actual_model.clone(),
                was_routed,
                tier: tier_label,
                latency_ms: llm_latency.as_millis() as u64,
                throughput_tps: if llm_latency.as_millis() > 0 {
                    f64::from(result.output_tokens) / llm_latency.as_secs_f64()
                } else {
                    0.0
                },
                cache_read_tokens: result.cache_read_tokens,
                cache_creation_tokens: result.cache_creation_tokens,
                input_tokens: result.input_tokens,
                output_tokens: result.output_tokens,
            });

            if let Some(ref tracker) = self.stats {
                tracker.record(crate::model_stats::RequestObservation {
                    model: actual_model,
                    success: true,
                    latency: llm_latency,
                    input_tokens: result.input_tokens,
                    output_tokens: result.output_tokens,
                    cache_read_tokens: result.cache_read_tokens,
                    cache_creation_tokens: result.cache_creation_tokens,
                    tier: complexity,
                    escalated,
                }).await;
            }

            state.input_tokens += result.input_tokens;
            state.output_tokens += result.output_tokens;

            // Post-hoc thinking spiral detection (catches sync/non-streaming path
            // where we can't abort mid-stream)
            if !state.thinking_spiral_nudged
                && result.tool_uses.is_empty()
                && result.text.is_empty()
                && detect_thinking_spiral(&state.reasoning_content)
            {
                warn!(
                    reasoning_len = state.reasoning_content.len(),
                    reasoning_tokens = state.reasoning_tokens,
                    "🌀 Post-hoc thinking spiral detected (sync path) — injecting action nudge"
                );
                state.thinking_spiral_nudged = true;
                state.reasoning_content.clear();
                state.current_reasoning.clear();

                let nudge = AnthropicMessage::user_text(
                    "You were stuck in a reasoning loop. STOP deliberating and ACT. \
                     You have tools available — use `discover_tools` to see all of them. \
                     It unlocks file operations (read_file, write_file, list_dir, explore), \
                     shell commands (exec), web access (web_search, web_fetch), and more. \
                     Call discover_tools NOW, then use the appropriate tool to answer the question."
                );
                {
                    let mut ctx = self.context.write().await;
                    ctx.messages.push(nudge);
                }
                continue;
            }

            // Token budget enforcement
            if let Some(budget) = options.token_budget {
                let cumulative = u64::from(state.input_tokens) + u64::from(state.output_tokens);
                let budget_pct = (cumulative * 100) / budget.max(1);
                if cumulative >= budget {
                    warn!(
                        cumulative_tokens = cumulative,
                        budget = budget,
                        "Token budget exhausted, stopping agent"
                    );
                    state.final_text = format!(
                        "{}\n\n[Token budget exhausted: used {} of {} tokens]",
                        state.final_text, cumulative, budget
                    );
                    return Ok(state.into_response(true));
                } else if budget_pct >= 80 {
                    warn!(
                        cumulative_tokens = cumulative,
                        budget = budget,
                        pct = budget_pct,
                        "Token budget at {}%, approaching limit",
                        budget_pct
                    );
                }
            }

            // Store assistant response
            self.store_assistant_response(&result.content_blocks).await;

            // Tool result eviction: once the LLM has responded referencing tool results,
            // replace old large tool results with compact stubs since the information
            // has been synthesized into the assistant's response.
            self.evict_referenced_tool_results(&result.content_blocks).await;

            // Update final text
            if !result.text.is_empty() {
                state.final_text = result.text;
            }

            // If no tool calls, check for narration loop before exiting
            if result.tool_uses.is_empty() {
                // Detect narration loop: model talked about using tools but never called them
                if detect_narration_loop(&state.final_text) && !state.narration_nudged {
                    warn!(
                        text_len = state.final_text.len(),
                        iteration = state.iterations,
                        "🔄 Narration loop detected — injecting nudge and retrying"
                    );
                    state.narration_nudged = true;

                    // Store the broken response so the model sees what it did
                    self.store_assistant_response(&result.content_blocks).await;

                    // Inject a user-role nudge to break the pattern
                    let nudge = AnthropicMessage::user_text(
                        "[SYSTEM] You narrated tool calls instead of actually executing them. \
                         NOTHING you described actually happened — no files were read or written. \
                         You MUST use tool calls (read_file, write_file, exec, etc.) to perform actions. \
                         Describing an action in text does NOT execute it. \
                         Start over: call the tools directly with NO narration."
                    );
                    {
                        let mut ctx = self.context.write().await;
                        ctx.messages.push(nudge);
                    }

                    // Continue the loop — the next iteration will re-call the LLM
                    continue;
                }

                // Detect thinking spiral: model spent a lot of reasoning tokens
                // going in circles without producing useful output
                if !state.thinking_spiral_nudged
                    && state.final_text.contains("[THINKING SPIRAL DETECTED]")
                {
                    warn!(
                        reasoning_tokens = state.reasoning_tokens,
                        iteration = state.iterations,
                        "🌀 Thinking spiral recovery — injecting action nudge and retrying"
                    );
                    state.thinking_spiral_nudged = true;

                    // Clear the spiral marker text
                    state.final_text.clear();
                    // Clear accumulated reasoning so the model starts fresh
                    state.reasoning_content.clear();
                    state.current_reasoning.clear();

                    // Inject a firm nudge that grounds the model on its available tools
                    let nudge = AnthropicMessage::user_text(
                        "You were stuck in a reasoning loop. STOP deliberating and ACT. \
                         You have tools available — use `discover_tools` to see all of them. \
                         It unlocks file operations (read_file, write_file, list_dir, explore), \
                         shell commands (exec), web access (web_search, web_fetch), and more. \
                         Call discover_tools NOW, then use the appropriate tool to answer the question."
                    );
                    {
                        let mut ctx = self.context.write().await;
                        ctx.messages.push(nudge);
                    }
                    continue;
                }

                // Normal exit: no tool calls and not a narration loop
                // Analyze uncertainty if enabled
                if options.track_uncertainty {
                    state.confidence = self.analyze_confidence(&state.final_text).await;
                }

                // Analyze emotional context if enabled
                if options.track_emotions {
                    state.emotional_context = self.analyze_emotions().await;
                }

                // Auto-extract memories if enabled
                if options.auto_extract_memories {
                    if let Some(ref on_memory) = options.on_memory {
                        if let Ok(memories) = self.extract_memories().await {
                            for memory in memories {
                                on_memory(memory).await;
                            }
                        }
                    }
                }
                return Ok(state.into_response(false));
            }

            // Finalize any reasoning that occurred before tool calls (interleaved reasoning)
            if !result.tool_uses.is_empty() {
                let first_tool = result.tool_uses.first().map(|(_, name, _)| name.clone());
                state.finalize_reasoning_block(first_tool);
            }

            // Execute tools and continue loop
            let mut tool_results = self.execute_tools(&result.tool_uses, &mut state, &options).await;
            // Merge in any error results from malformed tool call JSON.
            // These tell the model its call was unparseable so it can retry.
            if !result.error_tool_results.is_empty() {
                tool_results.extend(result.error_tool_results);
            }
            self.store_tool_results(tool_results).await;

            // Progressive context distillation: rolling summary every N iterations
            if self.config.distillation_interval > 0
                && state.iterations > 0
                && state.iterations % self.config.distillation_interval == 0
            {
                self.run_progressive_distillation().await;
            }

            // Semantic deduplication: evict superseded tool results
            self.deduplicate_tool_results().await;

            // Periodic memory extraction every 10 iterations
            if options.auto_extract_memories
                && state.iterations > 0
                && state.iterations % 10 == 0
            {
                if let Some(ref on_memory) = options.on_memory {
                    info!(iteration = state.iterations, "Periodic memory extraction");
                    if let Ok(memories) = self.extract_memories().await {
                        for memory in memories {
                            on_memory(memory).await;
                        }
                    }
                }
            }
        }
    }

    async fn add_user_message_with_budget(&self, message: &str, options: &RunOptions) {
        // Build budget prefix if budget awareness is enabled
        let budget_note = if options.budget_awareness {
            options.token_budget.map(|budget| {
                format!(
                    "[Budget: {} tokens. Be efficient with tool calls. \
                     Delegate independent sub-tasks with the `task` tool to save context.]",
                    budget
                )
            })
        } else {
            None
        };

        let mut ctx = self.context.write().await;
        let mut msg = String::new();
        if let Some(ref budget) = budget_note {
            msg.push_str(budget);
            msg.push_str("\n\n");
        }
        if let Some(ref prefix) = options.context_prefix {
            msg.push_str(prefix);
            msg.push_str("\n\n");
        }
        msg.push_str(message);
        if options.attachments.is_empty() {
            ctx.messages.push(AnthropicMessage::user_text(msg));
        } else {
            // Build content blocks: text first, then images
            let mut blocks = vec![ContentBlock::Text { text: msg }];
            for (data, media_type) in &options.attachments {
                blocks.push(ContentBlock::Image {
                    source: ImageSource::Base64 {
                        media_type: media_type.clone(),
                        data: data.clone(),
                    },
                });
            }
            ctx.messages.push(AnthropicMessage::user(blocks));
        }
    }

    async fn call_llm(
        &self,
        request: &AnthropicRequest,
        options: &RunOptions,
        state: &mut RunState,
    ) -> Result<LlmResult, AgentError> {
        // Retry with exponential backoff on rate limit errors
        let max_retries = 3u32;
        let mut attempt = 0;

        loop {
            let result = if let Some(ref on_text) = options.on_text {
                self.call_llm_streaming(request, on_text, options.on_thinking.as_ref(), state).await
            } else {
                self.call_llm_sync(request, state).await
            };

            match result {
                Ok(r) => return Ok(r),
                Err(ref e) if attempt < max_retries => {
                    let err_str = e.to_string().to_lowercase();
                    let is_rate_limit = err_str.contains("rate limit")
                        || err_str.contains("rate_limit")
                        || err_str.contains("429")
                        || err_str.contains("too many requests")
                        || err_str.contains("overloaded");

                    if is_rate_limit {
                        attempt += 1;
                        // Exponential backoff: 5s, 10s, 20s
                        let wait_secs = 5u64 * (1u64 << (attempt - 1));
                        warn!(
                            model = %request.model,
                            attempt = attempt,
                            wait_secs = wait_secs,
                            "Rate limited, retrying in {}s (attempt {}/{})",
                            wait_secs, attempt, max_retries
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;
                        continue;
                    }
                    return result;
                }
                Err(_) => return result,
            }
        }
    }

    async fn call_llm_streaming(
        &self,
        request: &AnthropicRequest,
        on_text: &StreamCallback,
        on_thinking: Option<&ThinkingCallback>,
        state: &mut RunState,
    ) -> Result<LlmResult, AgentError> {
        use futures::StreamExt;
        use std::pin::pin;

        let mut stream = pin!(self.llm.stream_anthropic(request));
        let mut tool_uses = Vec::new();
        let mut response_text = String::new();
        let mut content_blocks = Vec::new();
        let mut output_tokens = 0u32;
        let mut pending_error_tool_results: Vec<ContentBlock> = Vec::new();

        let mut current_tool_id = String::new();
        let mut current_tool_name = String::new();
        let mut current_tool_json = String::new();
        let mut current_block_type = String::new();
        let mut narration_check_len = 0usize; // track text length at last narration check

        while let Some(event) = stream.next().await {
            match event? {
                StreamEvent::TextDelta { text, .. } => {
                    on_text(&text);
                    response_text.push_str(&text);

                    // Periodically check for narration loops (every ~2000 chars of text)
                    if response_text.len() - narration_check_len > 2000 {
                        narration_check_len = response_text.len();
                        if detect_narration_loop(&response_text) || detect_repetition(&response_text) {
                            warn!(
                                text_len = response_text.len(),
                                "🔄 Narration loop detected in streaming response — aborting stream"
                            );
                            // Append a notice and break out of the stream
                            let notice = "\n\n[I got stuck narrating instead of acting. Let me try again with a focused approach.]";
                            on_text(notice);
                            response_text.push_str(notice);
                            break;
                        }
                    }
                }
                StreamEvent::ThinkingDelta { thinking, .. } => {
                    // Capture thinking/reasoning content
                    if let Some(callback) = on_thinking {
                        callback(&thinking);
                    }
                    state.reasoning_content.push_str(&thinking);
                    state.current_reasoning.push_str(&thinking);
                    // Estimate tokens (~4 chars per token)
                    state.reasoning_tokens += (thinking.len() / 4) as u32;

                    // Detect thinking spirals: model going in circles during reasoning.
                    // Check periodically (every ~3000 chars of thinking) to avoid overhead.
                    if state.reasoning_content.len() > 3000
                        && state.reasoning_content.len() % 3000 < thinking.len()
                        && detect_thinking_spiral(&state.reasoning_content)
                    {
                        warn!(
                            thinking_len = state.reasoning_content.len(),
                            thinking_tokens = state.reasoning_tokens,
                            "🌀 Thinking spiral detected — aborting stream and forcing action"
                        );
                        // Break out of the stream; the main loop will see no tool calls
                        // and no text, so we inject a forced-action response
                        response_text = "[THINKING SPIRAL DETECTED] I was overthinking. Let me act instead of deliberate.".to_string();
                        on_text(&response_text);
                        break;
                    }
                }
                StreamEvent::ContentBlockStart { content_type, tool_id, tool_name, .. } => {
                    current_block_type = content_type;
                    if let Some(id) = tool_id {
                        current_tool_id = id;
                    }
                    if let Some(name) = tool_name {
                        current_tool_name = name;
                    }
                }
                StreamEvent::ContentBlockStop { .. } => {
                    // Add a newline at the end of each thinking block so they don't run together
                    if current_block_type == "thinking" {
                        if let Some(callback) = on_thinking {
                            callback("\n");
                        }
                        state.reasoning_content.push('\n');
                        state.current_reasoning.push('\n');
                    }
                    if current_block_type == "tool_use" && !current_tool_id.is_empty() {
                        // Default empty tool JSON to "{}" (tool with no params)
                        if current_tool_json.trim().is_empty() {
                            current_tool_json = "{}".to_string();
                        }
                        match serde_json::from_str::<Value>(&current_tool_json) {
                            Ok(input) => {
                                tool_uses.push((
                                    current_tool_id.clone(),
                                    current_tool_name.clone(),
                                    input,
                                ));
                                content_blocks.push(ContentBlock::ToolUse {
                                    id: std::mem::take(&mut current_tool_id),
                                    name: std::mem::take(&mut current_tool_name),
                                    input: serde_json::from_str(&current_tool_json).unwrap_or_default(),
                                });
                            }
                            Err(e) => {
                                // Try to salvage a valid JSON object from garbled output.
                                // Some models (especially free-tier) emit concatenated or
                                // malformed JSON like: {"a":"b" garbage"}{"a":"b"}
                                // We attempt to find the last valid JSON object in the string.
                                let mut salvaged = false;
                                if let Some(last_brace) = current_tool_json.rfind('{') {
                                    let candidate = &current_tool_json[last_brace..];
                                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(candidate) {
                                        warn!(
                                            tool_id = %current_tool_id,
                                            tool_name = %current_tool_name,
                                            original_json = %current_tool_json,
                                            salvaged_json = %candidate,
                                            "Salvaged valid JSON from garbled tool call stream"
                                        );
                                        tool_uses.push((
                                            current_tool_id.clone(),
                                            current_tool_name.clone(),
                                            parsed.clone(),
                                        ));
                                        content_blocks.push(ContentBlock::ToolUse {
                                            id: std::mem::take(&mut current_tool_id),
                                            name: std::mem::take(&mut current_tool_name),
                                            input: parsed,
                                        });
                                        salvaged = true;
                                    }
                                }
                                if !salvaged {
                                    warn!(
                                        tool_id = %current_tool_id,
                                        tool_name = %current_tool_name,
                                        json = %current_tool_json,
                                        error = %e,
                                        "Failed to parse tool_use JSON from stream — returning error to model"
                                    );
                                    // Push the ToolUse to content_blocks so the assistant
                                    // message is well-formed (Claude API requires a matching
                                    // ToolResult for every ToolUse).
                                    content_blocks.push(ContentBlock::ToolUse {
                                        id: current_tool_id.clone(),
                                        name: current_tool_name.clone(),
                                        input: serde_json::json!({}),
                                    });
                                    // Push a synthetic error tool result that will be sent
                                    // back to the model so it knows the call failed.
                                    pending_error_tool_results.push(ContentBlock::ToolResult {
                                        tool_use_id: std::mem::take(&mut current_tool_id),
                                        content: format!(
                                            "Error: Your tool call for '{}' had malformed JSON arguments and could not be parsed. \
                                             Parse error: {}. Please retry with valid JSON.",
                                            current_tool_name, e
                                        ),
                                        is_error: Some(true),
                                    });
                                    current_tool_name.clear();
                                }
                            }
                        }
                        current_tool_json.clear();
                    } else if current_block_type == "thinking" {
                        // Thinking block finished - reasoning already accumulated
                    } else if !response_text.is_empty() {
                        content_blocks.push(ContentBlock::Text {
                            text: response_text.clone(),
                        });
                    }
                    current_block_type.clear();
                }
                StreamEvent::ToolUseDelta { partial_json, .. } => {
                    current_tool_json.push_str(&partial_json);
                }
                StreamEvent::MessageDelta {
                    output_tokens: tokens,
                    ..
                } => {
                    output_tokens += tokens;
                }
                _ => {}
            }
        }

        // Note: streaming doesn't easily provide cache token breakdowns
        // (they come in message_start which we currently parse as MessageStart{id, model}).
        // TODO: Parse usage from message_start event for accurate cache tracking in streaming mode.
        Ok(LlmResult {
            text: response_text,
            tool_uses,
            content_blocks,
            input_tokens: 100, // Placeholder for streaming
            output_tokens,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            error_tool_results: pending_error_tool_results,
        })
    }

    async fn call_llm_sync(&self, request: &AnthropicRequest, state: &mut RunState) -> Result<LlmResult, AgentError> {
        let response = self.llm.complete_anthropic(request).await?;
        let mut tool_uses = Vec::new();
        let mut response_text = String::new();

        for block in &response.content {
            match block {
                ContentBlock::Text { text } => response_text.push_str(text),
                ContentBlock::ToolUse { id, name, input } => {
                    tool_uses.push((id.clone(), name.clone(), input.clone()));
                }
                ContentBlock::Thinking { thinking } => {
                    // Capture thinking/reasoning content
                    state.reasoning_content.push_str(thinking);
                    state.current_reasoning.push_str(thinking);
                    // Estimate tokens (~4 chars per token)
                    state.reasoning_tokens += (thinking.len() / 4) as u32;
                }
                ContentBlock::ToolResult { .. } | ContentBlock::Image { .. } => {}
            }
        }

        Ok(LlmResult {
            text: response_text,
            tool_uses,
            content_blocks: response.content,
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
            cache_read_tokens: response.usage.cache_read_input_tokens,
            cache_creation_tokens: response.usage.cache_creation_input_tokens,
            error_tool_results: Vec::new(),
        })
    }

    async fn store_assistant_response(&self, content_blocks: &[ContentBlock]) {
        let mut ctx = self.context.write().await;
        let stripped = Self::strip_write_content_from_blocks(content_blocks);
        ctx.messages
            .push(AnthropicMessage::assistant(stripped));
    }

    /// Strip large content from write_file/write tool_use blocks before storing in context.
    ///
    /// The LLM already generated the content, so keeping it in stored context is pure waste.
    /// Replaces the `content` field with a size placeholder.
    fn strip_write_content_from_blocks(blocks: &[ContentBlock]) -> Vec<ContentBlock> {
        blocks.iter().map(|block| {
            match block {
                ContentBlock::ToolUse { id, name, input } if is_write_tool(name) => {
                    let mut input = input.clone();
                    if let Some(obj) = input.as_object_mut() {
                        if let Some(content_val) = obj.get("content") {
                            let size = content_val.as_str().map_or_else(
                                || content_val.to_string().len(),
                                str::len,
                            );
                            obj.insert(
                                "content".to_string(),
                                Value::String(format!("[content omitted from context — {size} bytes were written successfully to disk]")),
                            );
                        }
                    }
                    ContentBlock::ToolUse {
                        id: id.clone(),
                        name: name.clone(),
                        input,
                    }
                }
                _ => block.clone(),
            }
        }).collect()
    }

    async fn execute_tools(
        &self,
        tool_uses: &[(String, String, Value)],
        state: &mut RunState,
        options: &RunOptions,
    ) -> Vec<ContentBlock> {
        let mut tool_results = Vec::new();

        for (id, name, input) in tool_uses {
            let start = std::time::Instant::now();

            let params: HashMap<String, Value> = input.as_object().map_or_else(HashMap::new, |obj| {
                obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
            });

            let tool_call = ToolCall {
                id: id.clone(),
                name: name.clone(),
                parameters: params,
            };

            // Finalize any pending reasoning block before tool execution (interleaved reasoning)
            state.finalize_reasoning_block(Some(name.clone()));

            // Notify via callback before execution
            if let Some(ref cb) = options.on_tool_start {
                cb(id, name, input);
            }

            info!(tool = %name, "Executing tool");
            let response = self.tools.execute(tool_call).await;
            let duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

            if duration_ms > 10_000 {
                warn!(
                    tool = %name,
                    duration_ms,
                    success = response.result.success,
                    output_len = response.result.content.len(),
                    "🐌 Very slow tool execution (>10s)"
                );
            } else if duration_ms > 5_000 {
                warn!(
                    tool = %name,
                    duration_ms,
                    "⚠️ Slow tool execution (>5s)"
                );
            } else {
                debug!(tool = %name, duration_ms, "Tool completed");
            }

            // Record tool stats
            if let Some(ref tracker) = self.tool_stats {
                let error_msg = if !response.result.success {
                    response.result.error.clone()
                } else {
                    None
                };
                tracker.record(crate::tool_stats::ToolObservation {
                    tool_name: name.clone(),
                    success: response.result.success,
                    duration_ms,
                    output_size: response.result.content.len(),
                    error: error_msg,
                    session_id: None, // Session ID not available at this level
                }).await;
            }

            // Notify via callback after execution
            if let Some(ref cb) = options.on_tool_end {
                let result_content = if response.result.success {
                    &response.result.content
                } else {
                    response.result.error.as_deref().unwrap_or("Unknown error")
                };
                cb(id, name, result_content, response.result.success, duration_ms);
            }

            // Check for activate_tools in structured data (from discover_tools)
            if let Some(ref data) = response.result.data {
                if let Some(activate) = data.get("activate_tools") {
                    if let Some(arr) = activate.as_array() {
                        for tool_name in arr {
                            if let Some(s) = tool_name.as_str() {
                                info!(tool = s, "Activating tool via discover_tools");
                                state.active_tools.insert(s.to_string());
                            }
                        }
                    }
                }
            }

            // Strip write content from stored tool call record (same as context blocks)
            let stored_input = if is_write_tool(name) {
                let mut input = input.clone();
                if let Some(obj) = input.as_object_mut() {
                    if let Some(content_val) = obj.get("content") {
                        let size = content_val.as_str().map_or_else(
                            || content_val.to_string().len(),
                            str::len,
                        );
                        obj.insert(
                            "content".to_string(),
                            Value::String(format!("[content omitted from context — {size} bytes were written successfully to disk]")),
                        );
                    }
                }
                input
            } else {
                input.clone()
            };

            state.tool_records.push(ToolCallRecord {
                id: id.clone(),
                name: name.clone(),
                input: stored_input,
                output: response.result.content.clone(),
                success: response.result.success,
                duration_ms,
            });

            let result_content = if response.result.success {
                response.result.content
            } else {
                format!(
                    "Error: {}",
                    response
                        .result
                        .error
                        .unwrap_or_else(|| "Unknown error".to_string())
                )
            };

            let output_target = response.output_target;
            // Dynamic threshold: 0 = auto-scale based on max_tokens (proxy for model size)
            // ~4 chars per token, use 2x max_tokens as threshold (generous for large models)
            // Floor: 2000 chars, Cap: 32000 chars
            let threshold = if self.config.context_result_threshold == 0 {
                (self.config.max_tokens as usize * 2).clamp(2000, 32000)
            } else {
                self.config.context_result_threshold
            };

            let final_content = match output_target {
                OutputTarget::Context => {
                    // Context-targeted tools: never store in memory, never stub.
                    // For large outputs: try LLMLingua compression → summarization → truncation.
                    if result_content.len() > threshold {
                        // Try LLMLingua-style compression first (faster, preserves more detail)
                        let compressed = if !self.config.summarization_priority.is_empty() {
                            if let Ok((client, model_name)) = self.create_client_for_model(&self.config.summarization_priority[0]) {
                                crate::compressor::compress_text(&client, &model_name, &result_content, 4).await
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                        if let Some(compressed) = compressed {
                            if compressed.len() < result_content.len() / 2 {
                                info!(
                                    tool = name,
                                    original_len = result_content.len(),
                                    compressed_len = compressed.len(),
                                    "🗜️ Compressed tool output ({} → {} chars)",
                                    result_content.len(), compressed.len()
                                );
                                compressed
                            } else if let Some(summarized) = self.summarize_tool_output(name, &result_content).await {
                                summarized
                            } else {
                                let end = truncate_boundary(&result_content, threshold);
                                format!(
                                    "{}...\n\n[truncated — showing {end} of {} chars.]",
                                    &result_content[..end], result_content.len()
                                )
                            }
                        } else if let Some(summarized) = self.summarize_tool_output(name, &result_content).await {
                            info!(
                                tool = name,
                                original_len = result_content.len(),
                                summarized_len = summarized.len(),
                                "📝 Summarized tool output ({} → {} chars)",
                                result_content.len(), summarized.len()
                            );
                            summarized
                        } else {
                            // Fallback: truncate at a clean boundary
                            let end = truncate_boundary(&result_content, threshold);
                            format!(
                                "{}...\n\n[truncated — showing {end} of {} chars. Use recall with a more specific query to find particular details.]",
                                &result_content[..end],
                                result_content.len()
                            )
                        }
                    } else {
                        result_content
                    }
                }
                OutputTarget::Memory => {
                    // Default: store in memory, stub large results in context.
                    let source_id = Uuid::new_v4().to_string()[..8].to_string();
                    if let Some(ref on_memory) = options.on_memory {
                        let chunks = if result_content.len() > 3200 {
                            semantic_chunk(&result_content, 3200, 0.15)
                        } else {
                            vec![(0, result_content.clone())]
                        };
                        let total_chunks = chunks.len();

                        for (idx, chunk_content) in &chunks {
                            let mut tags = HashMap::new();
                            tags.insert("tool".to_string(), name.clone());
                            tags.insert("source_id".to_string(), source_id.clone());
                            tags.insert("chunk".to_string(), format!("{}/{}", idx + 1, total_chunks));

                            on_memory(ExtractedMemory {
                                content: format!("[Tool: {name}] {chunk_content}"),
                                category: "tool_result".to_string(),
                                tags: Some(tags),
                            }).await;
                        }
                    }

                    if options.on_memory.is_some() && result_content.len() > threshold {
                        let chunk_count = (result_content.len() / 3200).max(1);
                        format!(
                            "[Result from '{}' stored in memory (source_id={}, {} chunks, {} chars). \
                             Use recall('query about {}') to retrieve specific sections.]",
                            name, source_id, chunk_count, result_content.len(), name
                        )
                    } else {
                        result_content
                    }
                }
            };

            // Ensure tool result content is never empty (Anthropic rejects empty text blocks)
            let final_content = if final_content.is_empty() {
                "[No output]".to_string()
            } else {
                final_content
            };

            tool_results.push(ContentBlock::ToolResult {
                tool_use_id: id.clone(),
                content: final_content,
                is_error: if response.result.success {
                    None
                } else {
                    Some(true)
                },
            });
        }

        tool_results
    }

    /// Create an LLM client for the specified model
    /// Model format: "provider/model" or just "model" (uses main client's provider)
    fn create_client_for_model(&self, model_spec: &str) -> Result<(LlmClient, String), String> {
        if let Some((provider, model)) = model_spec.split_once('/') {
            let client = match provider.to_lowercase().as_str() {
                "ollama" => {
                    let url = self.config.summarization_ollama_url
                        .as_deref()
                        .unwrap_or("http://localhost:11434");
                    LlmClient::ollama(url)
                }
                "openai" => {
                    let api_key = self.config.openai_api_key.as_ref()
                        .ok_or("OpenAI summarization requires API key configuration")?;
                    LlmClient::openai(api_key)
                }
                "anthropic" => {
                    // Use the same client (it already has auth)
                    (*self.llm).clone()
                }
                "openrouter" => {
                    let api_key = self.config.openrouter_api_key.as_ref()
                        .ok_or("OpenRouter summarization requires API key configuration")?;
                    LlmClient::openrouter(api_key)
                }
                _ => {
                    return Err(format!("Unknown provider: {}", provider));
                }
            };
            Ok((client, model.to_string()))
        } else {
            // No provider prefix - use the main LLM client with the specified model
            Ok(((*self.llm).clone(), model_spec.to_string()))
        }
    }

    /// Classify the current iteration's complexity and route to the cheapest capable model.
    /// Returns None if routing is disabled or the primary model should be used.
    async fn route_model(&self, state: &RunState) -> Option<String> {
        if self.config.model_routing.is_empty() {
            return None;
        }

        // First iteration: always use primary model if configured
        if state.iterations <= 1 && self.config.routing_first_turn_primary {
            debug!("Model routing: first turn, using primary model");
            return None;
        }

        let complexity = self.classify_complexity(state).await;

        // Find cheapest model whose tier >= required complexity
        // The routing list is in priority order (cheapest first)
        for tier_entry in &self.config.model_routing {
            if tier_entry.tier >= complexity {
                // Skip unhealthy models (consecutive failures >= threshold)
                if let Some(ref tracker) = self.stats {
                    if !tracker.is_healthy(&tier_entry.model).await {
                        debug!(
                            model = %tier_entry.model,
                            "⚠️ Skipping unhealthy model in routing"
                        );
                        continue;
                    }
                }

                if tier_entry.model != self.config.model {
                    info!(
                        routed_model = %tier_entry.model,
                        complexity = ?complexity,
                        tier = ?tier_entry.tier,
                        iteration = state.iterations,
                        "🔀 Model routing: using {} for {:?} task",
                        tier_entry.model, complexity
                    );
                    return Some(tier_entry.model.clone());
                }
                // Already the primary model
                return None;
            }
        }

        // No model in routing list can handle this complexity — use primary
        None
    }

    /// Classify the complexity of the current iteration based on context signals.
    async fn classify_complexity(&self, state: &RunState) -> TaskComplexity {
        let ctx = self.context.read().await;
        let messages = &ctx.messages;

        // If we have no messages yet, it's the initial turn — complex
        if messages.is_empty() {
            return TaskComplexity::Complex;
        }

        // Look at the last assistant message to understand what's happening
        let last_assistant = messages.iter().rev().find(|m| m.role == "assistant");

        // If the last assistant message was entirely tool calls with no text,
        // the next iteration is likely just continuing tool execution — simple
        if let Some(assistant_msg) = last_assistant {
            let has_text = assistant_msg.content.iter().any(|b| matches!(b, ContentBlock::Text { .. }));
            let has_tools = assistant_msg.content.iter().any(|b| matches!(b, ContentBlock::ToolUse { .. }));

            if has_tools && !has_text {
                // Pure tool-calling iteration: the model just needs to decide what tool to call next
                return TaskComplexity::Simple;
            }
        }

        // Look at the last user message (which may contain tool results)
        let last_user = messages.iter().rev().find(|m| m.role == "user");
        if let Some(user_msg) = last_user {
            let has_tool_results = user_msg.content.iter().any(|b| matches!(b, ContentBlock::ToolResult { .. }));

            if has_tool_results {
                // We're in a tool result → next LLM call cycle
                // Simple if we've been doing straightforward tool calls
                if state.iterations > 2 {
                    return TaskComplexity::Simple;
                }
                return TaskComplexity::Medium;
            }
        }

        // Early iterations with user text: likely complex (initial analysis)
        if state.iterations <= 2 {
            return TaskComplexity::Complex;
        }

        // Default to medium for mid-conversation turns
        TaskComplexity::Medium
    }

    /// Run progressive context distillation: produce a structured rolling summary
    /// of recent conversation and evict old tool results that have been referenced.
    async fn run_progressive_distillation(&self) {
        if self.config.summarization_priority.is_empty() {
            return;
        }

        let (client, model_name) = match self.create_client_for_model(&self.config.summarization_priority[0]) {
            Ok(pair) => pair,
            Err(_) => return,
        };

        let ctx = self.context.read().await;
        // Only distill if we have enough messages
        if ctx.messages.len() < 6 {
            return;
        }

        // Build a summary of the last N messages (not the whole context)
        let recent_messages: Vec<String> = ctx.messages.iter().rev().take(10).rev().map(|m| {
            let role = &m.role;
            let content_preview: String = m.content.iter().take(3).map(|b| match b {
                ContentBlock::Text { text } => {
                    if text.len() > 200 { format!("{}...", &text[..200]) } else { text.clone() }
                }
                ContentBlock::ToolUse { name, .. } => format!("[tool_use: {name}]"),
                ContentBlock::ToolResult { content, .. } => {
                    if content.len() > 100 { let end = content.floor_char_boundary(100); format!("[result: {}...]", &content[..end]) } else { format!("[result: {content}]") }
                }
                _ => "[...]".to_string(),
            }).collect::<Vec<_>>().join(" ");
            format!("{role}: {content_preview}")
        }).collect();

        let prompt = format!(
            "Distill the following conversation into structured key-value facts. \
             Format: one fact per line as `key: value`. \
             Include: user_goal, files_modified, decisions_made, current_state, blockers, next_steps.\n\n{}",
            recent_messages.join("\n")
        );
        drop(ctx);

        let request = AnthropicRequest {
            model: model_name,
            messages: vec![AnthropicMessage::user_text(prompt)],
            max_tokens: 512,
            temperature: Some(0.2),
            system: Some("You are a conversation distiller. Output ONLY structured key-value facts, one per line. No prose.".to_string()),
            tools: None,
            stream: None,
            thinking: None,
            cache_control: None,
        };

        match client.complete_anthropic(&request).await {
            Ok(response) => {
                let facts: String = response.content.iter().filter_map(|b| {
                    if let ContentBlock::Text { text } = b { Some(text.as_str()) } else { None }
                }).collect();
                if !facts.is_empty() {
                    let mut ctx = self.context.write().await;
                    // Update consolidated summary with latest facts
                    ctx.consolidated_summary = Some(format!(
                        "[DISTILLED FACTS]\n{facts}"
                    ));
                    info!(
                        facts_len = facts.len(),
                        "🧬 Progressive distillation complete"
                    );
                }
            }
            Err(e) => {
                debug!(error = %e, "Progressive distillation failed");
            }
        }
    }

    /// Semantic deduplication: detect and evict superseded tool results.
    /// When the same file is read twice or the same URL fetched again,
    /// keep only the latest result and replace the old one with a stub.
    async fn deduplicate_tool_results(&self) {
        let mut ctx = self.context.write().await;

        // Build a map of tool_use_id → (tool_name, input_key) for tool results
        // to detect duplicates (same tool + same primary argument)
        let mut seen: HashMap<String, usize> = HashMap::new(); // key → latest message index
        let mut to_stub: Vec<(usize, String)> = Vec::new(); // (message_idx, tool_use_id)

        for (msg_idx, msg) in ctx.messages.iter().enumerate() {
            if msg.role != "user" { continue; }
            for block in &msg.content {
                if let ContentBlock::ToolResult { tool_use_id, content, .. } = block {
                    // Skip already-stubbed results
                    if content.starts_with("[superseded")
                        || content.starts_with("[evicted")
                        || content.starts_with("[Result from") {
                        continue;
                    }

                    // Find the corresponding tool_use to get the dedup key
                    let dedup_key = self.find_tool_dedup_key(&ctx.messages, tool_use_id);
                    if let Some(key) = dedup_key {
                        if let Some(&prev_idx) = seen.get(&key) {
                            // This tool+input was called before — the previous result is superseded
                            to_stub.push((prev_idx, tool_use_id.clone()));
                        }
                        seen.insert(key, msg_idx);
                    }
                }
            }
        }

        // Replace superseded results with stubs
        let stub_count = to_stub.len();
        for (msg_idx, _tool_use_id) in to_stub {
            if let Some(msg) = ctx.messages.get_mut(msg_idx) {
                for block in &mut msg.content {
                    if let ContentBlock::ToolResult { content, .. } = block {
                        if !content.starts_with("[superseded") {
                            let old_len = content.len();
                            *content = format!("[superseded by later call — {old_len} chars removed]");
                        }
                    }
                }
            }
        }

        if stub_count > 0 {
            info!(
                deduped = stub_count,
                "🔁 Semantic dedup: evicted {} superseded tool results",
                stub_count
            );
        }
    }

    /// Evict tool results that have been referenced by the LLM's response.
    /// After the assistant synthesizes information from tool results into its response,
    /// the raw tool results are replaced with compact stubs to save context space.
    /// Only evicts results that are "old" (not from the most recent tool call round)
    /// and exceed a minimum size threshold.
    async fn evict_referenced_tool_results(&self, response_blocks: &[ContentBlock]) {
        const MIN_EVICT_SIZE: usize = 1000; // Only evict results larger than this

        // Extract text from the assistant's response
        let response_text: String = response_blocks.iter().filter_map(|b| {
            if let ContentBlock::Text { text } = b { Some(text.as_str()) } else { None }
        }).collect::<Vec<_>>().join("\n");

        if response_text.is_empty() {
            return;
        }

        let mut ctx = self.context.write().await;
        let msg_count = ctx.messages.len();
        if msg_count < 4 {
            return; // Not enough history to evict
        }

        // Only look at messages before the last 2 (skip the most recent tool call round)
        let eviction_range = msg_count.saturating_sub(2);
        let mut evicted = 0;
        let mut bytes_saved = 0usize;

        for msg in ctx.messages[..eviction_range].iter_mut() {
            if msg.role != "user" { continue; }
            for block in &mut msg.content {
                if let ContentBlock::ToolResult { tool_use_id, content, .. } = block {
                    // Skip already-stubbed results
                    if content.len() < MIN_EVICT_SIZE
                        || content.starts_with("[superseded")
                        || content.starts_with("[evicted")
                        || content.starts_with("[Result from")
                    {
                        continue;
                    }

                    // Check if the response references this tool result's content
                    // Use a simple heuristic: if the response contains a significant
                    // substring from the tool result, it's been "consumed"
                    let is_referenced = {
                        // Sample a few lines from the tool result
                        let lines: Vec<&str> = content.lines()
                            .filter(|l| l.len() > 20)
                            .take(5)
                            .collect();
                        // If response mentions any distinctive line, consider it referenced
                        lines.iter().any(|line| {
                            let sample = if line.len() > 60 { &line[..60] } else { line };
                            response_text.contains(sample)
                        })
                    };

                    if is_referenced {
                        let old_len = content.len();
                        *content = format!(
                            "[evicted: {tool_use_id} — {old_len} chars, discussed in response above]"
                        );
                        bytes_saved += old_len;
                        evicted += 1;
                    }
                }
            }
        }

        if evicted > 0 {
            info!(
                evicted_count = evicted,
                bytes_saved = bytes_saved,
                "🗑️ Evicted {} referenced tool results (~{} chars saved)",
                evicted, bytes_saved
            );
        }
    }

    /// Find a dedup key for a tool result by looking up its corresponding tool_use block.
    /// Returns "tool_name:primary_arg" for dedup-eligible tools.
    fn find_tool_dedup_key(&self, messages: &[AnthropicMessage], tool_use_id: &str) -> Option<String> {
        for msg in messages {
            if msg.role != "assistant" { continue; }
            for block in &msg.content {
                if let ContentBlock::ToolUse { id, name, input } = block {
                    if id == tool_use_id {
                        // Extract primary argument for dedup
                        let primary_arg = match name.as_str() {
                            "read_file" | "read" => input.get("file_path").or_else(|| input.get("path"))
                                .and_then(|v| v.as_str()).map(String::from),
                            "web_fetch" => input.get("url").and_then(|v| v.as_str()).map(String::from),
                            "list_dir" | "glob" => input.get("path").and_then(|v| v.as_str()).map(String::from),
                            "code_outline" => input.get("file_path").or_else(|| input.get("path"))
                                .and_then(|v| v.as_str()).map(String::from),
                            _ => None,
                        };
                        return primary_arg.map(|arg| format!("{name}:{arg}"));
                    }
                }
            }
        }
        None
    }

    /// Summarize a large tool output using the cheapest available summarization model.
    /// Returns None if no summarization model is configured.
    async fn summarize_tool_output(&self, tool_name: &str, content: &str) -> Option<String> {
        if self.config.summarization_priority.is_empty() {
            return None;
        }

        let (client, model_name) = match self.create_client_for_model(&self.config.summarization_priority[0]) {
            Ok(pair) => pair,
            Err(_) => return None,
        };

        // Tool-type-aware summarization prompts
        let instruction = match tool_name {
            "read_file" | "read" => "Summarize the key content of this file. Keep function signatures, struct definitions, important constants, and any content that seems directly relevant to the current task. Omit boilerplate, imports, and obvious code.",
            "web_fetch" | "web_search" => "Extract the key information from this web content. Keep facts, data, and answer-relevant paragraphs. Remove navigation, ads, and boilerplate.",
            "exec" | "bash" => "Summarize this command output. Keep the exit status, key results, errors, and important data. For long listings, keep only the most relevant entries.",
            "list_dir" | "glob" => "Compact this directory listing. Show the structure concisely, grouping similar files. Keep file names but remove metadata unless unusual.",
            "code_search" | "grep" => "Summarize these search results. Keep the matching lines with file paths. Remove redundant context lines.",
            _ => "Summarize this tool output concisely. Keep all important information, data, and results. Remove redundancy and verbose formatting.",
        };

        let prompt = format!(
            "{instruction}\n\nTool: {tool_name}\nOutput ({} chars):\n{content}",
            content.len()
        );

        let request = AnthropicRequest {
            model: model_name,
            messages: vec![AnthropicMessage::user_text(prompt)],
            max_tokens: 1024,
            temperature: Some(0.2),
            system: Some("You are a tool output summarizer. Output ONLY the summarized content, no preamble or explanation. Preserve key information density.".to_string()),
            tools: None,
            stream: None,
            thinking: None,
            cache_control: None,
        };

        match client.complete_anthropic(&request).await {
            Ok(response) => {
                let text: String = response.content.iter().filter_map(|b| {
                    if let ContentBlock::Text { text } = b { Some(text.as_str()) } else { None }
                }).collect();
                if text.is_empty() { None } else { Some(text) }
            }
            Err(e) => {
                debug!(error = %e, "Tool output summarization failed, falling back to truncation");
                None
            }
        }
    }

    async fn store_tool_results(&self, tool_results: Vec<ContentBlock>) {
        let mut ctx = self.context.write().await;
        ctx.messages.push(AnthropicMessage::user(tool_results));
    }

    async fn build_request_with_thinking(&self, thinking_override: Option<ThinkingMode>, active_tools: &HashSet<String>) -> AnthropicRequest {
        let ctx = self.context.read().await;

        // Build the set of tool names to send: core + any activated via discover_tools
        let mut names: HashSet<String> = CORE_TOOL_NAMES.iter().map(|s| (*s).to_string()).collect();
        names.extend(active_tools.iter().cloned());

        let tool_defs = self.tools.definitions_for_names(&names).await;

        // Dedup safety net: the registry may return both a canonical name and its
        // alias for the same tool. The Anthropic API rejects duplicate tool names.
        let mut seen_names = HashSet::new();
        let tools: Vec<LlmToolDef> = tool_defs
            .iter()
            .filter(|t| seen_names.insert(t.name.clone()))
            .map(|t| LlmToolDef {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.to_anthropic_format()["input_schema"].clone(),
            })
            .collect();

        // Determine thinking mode (override takes precedence)
        let thinking_mode = thinking_override.unwrap_or(self.config.thinking_mode);
        let thinking = thinking_mode.budget_tokens().map(nanna_llm::ThinkingConfig::new);

        // Get effective system prompt (includes workspace context if set)
        let system_prompt = ctx.effective_system_prompt();

        // Enable prompt caching for Anthropic models (system prompt + tools get cached,
        // 90% discount on cached input tokens). Safe to send for non-Anthropic providers
        // as the field is skipped when None.
        let cache_control = if self.config.model.starts_with("claude") {
            Some(CacheControl::ephemeral())
        } else {
            None
        };

        AnthropicRequest {
            model: self.config.model.clone(),
            // Use messages_for_request() to include consolidated summary if present
            messages: ctx.messages_for_request(),
            max_tokens: self.config.max_tokens,
            temperature: Some(self.config.temperature),
            system: if system_prompt.is_empty() {
                None
            } else {
                Some(system_prompt)
            },
            tools: if tools.is_empty() { None } else { Some(tools) },
            stream: None,
            thinking,
            cache_control,
        }
    }

    /// Get a copy of the current context
    pub async fn context(&self) -> AgentContext {
        self.context.read().await.clone()
    }

    /// Set a new context
    pub async fn set_context(&self, context: AgentContext) {
        *self.context.write().await = context;
    }

    /// Clear the conversation history
    pub async fn clear(&self) {
        let mut ctx = self.context.write().await;
        ctx.messages.clear();
    }

    /// Set the workspace for this agent
    pub async fn set_workspace(&self, workspace: &nanna_workspace::Workspace) {
        let mut ctx = self.context.write().await;
        ctx.workspace_root = Some(workspace.root.clone());
        ctx.workspace_context = Some(workspace.system_context());
        ctx.include_workspace_memory = workspace.config.include_memory;
    }

    /// Reload workspace context from disk
    pub async fn reload_workspace(&self) -> Result<(), nanna_workspace::WorkspaceError> {
        let mut ctx = self.context.write().await;
        ctx.reload_workspace().await
    }

    /// Get the current workspace root (if set)
    pub async fn workspace_root(&self) -> Option<std::path::PathBuf> {
        self.context.read().await.workspace_root.clone()
    }

    /// Analyze confidence level in a response.
    ///
    /// Uses heuristics and optional LLM analysis to estimate confidence.
    async fn analyze_confidence(&self, response: &str) -> Option<f32> {
        // Quick heuristic analysis (no LLM call needed for basic cases)
        let lower = response.to_lowercase();

        // High uncertainty indicators
        let uncertain_phrases = [
            "i'm not sure",
            "i think",
            "probably",
            "maybe",
            "might be",
            "could be",
            "i believe",
            "it seems",
            "possibly",
            "not certain",
            "uncertain",
            "i don't know",
            "hard to say",
        ];

        // High confidence indicators
        let confident_phrases = [
            "definitely",
            "certainly",
            "absolutely",
            "i know",
            "it is",
            "clearly",
            "obviously",
            "without a doubt",
        ];

        let uncertain_count = uncertain_phrases
            .iter()
            .filter(|p| lower.contains(*p))
            .count();
        let confident_count = confident_phrases
            .iter()
            .filter(|p| lower.contains(*p))
            .count();

        // Calculate base confidence
        let base_confidence = if uncertain_count > confident_count {
            0.5 - (uncertain_count as f32 * 0.1)
        } else if confident_count > uncertain_count {
            0.8 + (confident_count as f32 * 0.05)
        } else {
            0.7 // Neutral
        };

        Some(base_confidence.clamp(0.1, 0.99))
    }

    /// Analyze emotional context of the conversation.
    ///
    /// Uses heuristics to detect user emotional state.
    async fn analyze_emotions(&self) -> Option<EmotionalContext> {
        let ctx = self.context.read().await;

        // Get the last user message
        let last_user_msg = ctx
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .and_then(|m| {
                m.content.iter().find_map(|b| {
                    if let ContentBlock::Text { text } = b {
                        Some(text.clone())
                    } else {
                        None
                    }
                })
            });

        let user_text = last_user_msg?;
        let lower = user_text.to_lowercase();

        // Emotion detection heuristics
        let emotions = [
            ("frustrated", vec!["frustrated", "annoyed", "ugh", "why won't", "doesn't work", "broken", "useless", "terrible", "hate"]),
            ("confused", vec!["confused", "don't understand", "what do you mean", "huh", "?", "lost", "unclear"]),
            ("excited", vec!["excited", "amazing", "awesome", "love it", "fantastic", "great", "wonderful", "!", "can't wait"]),
            ("grateful", vec!["thank", "thanks", "appreciate", "grateful", "helped"]),
            ("anxious", vec!["worried", "anxious", "nervous", "scared", "urgent", "asap", "hurry"]),
            ("neutral", vec![]),
        ];

        let mut detected_emotion = "neutral";
        let mut max_matches = 0;

        for (emotion, keywords) in &emotions {
            let matches = keywords.iter().filter(|k| lower.contains(*k)).count();
            if matches > max_matches {
                max_matches = matches;
                detected_emotion = emotion;
            }
        }

        // Calculate intensity based on punctuation and caps
        let exclamations = user_text.matches('!').count();
        let _questions = user_text.matches('?').count(); // Reserved for future use
        let caps_ratio = user_text
            .chars()
            .filter(|c| c.is_uppercase())
            .count() as f32
            / user_text.len().max(1) as f32;

        let intensity = (0.3 + (exclamations as f32 * 0.1) + (caps_ratio * 0.3) + (max_matches as f32 * 0.1))
            .clamp(0.0, 1.0);

        // Suggest tone adjustment
        let suggested_tone = match detected_emotion {
            "frustrated" => Some("patient and helpful".to_string()),
            "confused" => Some("clear and explanatory".to_string()),
            "excited" => Some("enthusiastic and supportive".to_string()),
            "anxious" => Some("calm and reassuring".to_string()),
            "grateful" => Some("warm and appreciative".to_string()),
            _ => None,
        };

        Some(EmotionalContext {
            primary_emotion: detected_emotion.to_string(),
            intensity,
            suggested_tone,
        })
    }

    /// Extract memorable facts from the current conversation.
    ///
    /// Uses a quick LLM call to identify noteworthy information that should be
    /// remembered long-term. Returns a list of extracted memory strings.
    ///
    /// # Errors
    ///
    /// Returns `AgentError::Llm` if the extraction LLM call fails.
    pub async fn extract_memories(&self) -> Result<Vec<ExtractedMemory>, AgentError> {
        let ctx = self.context.read().await;

        // Skip if no conversation yet
        if ctx.messages.is_empty() {
            return Ok(Vec::new());
        }

        // Build a summary of the conversation for extraction
        let mut conversation_text = String::new();
        for msg in &ctx.messages {
            let role = &msg.role;
            for block in &msg.content {
                if let ContentBlock::Text { text } = block {
                    conversation_text.push_str(&format!("{role}: {text}\n"));
                }
            }
        }

        // Skip if conversation is too short
        if conversation_text.len() < 100 {
            return Ok(Vec::new());
        }

        drop(ctx);

        // Create extraction request
        let extraction_prompt = format!(
            r#"Analyze this conversation and extract noteworthy facts that should be remembered long-term.

Focus on:
- User preferences and personal information
- Important decisions or conclusions
- Facts about projects, people, or systems
- Anything the user explicitly asked to remember

Conversation:
{conversation_text}

Respond with a JSON array of objects, each with "content" (the fact to remember) and "category" (preference/fact/decision/reminder).
If nothing notable, respond with an empty array: []

Example: [{{"content": "User prefers dark mode", "category": "preference"}}]"#
        );

        // Use the first usable summarization model (cheaper than main model)
        let (client, model_name) = if !self.config.summarization_priority.is_empty() {
            let mut found = None;
            for model_spec in &self.config.summarization_priority {
                match self.create_client_for_model(model_spec) {
                    Ok(pair) => { found = Some(pair); break; }
                    Err(e) => {
                        debug!("Skipping summarization model {}: {}", model_spec, e);
                    }
                }
            }
            found.unwrap_or_else(|| ((*self.llm).clone(), self.config.model.clone()))
        } else {
            ((*self.llm).clone(), self.config.model.clone())
        };

        info!(model = %model_name, "Running memory extraction");

        let request = AnthropicRequest {
            model: model_name,
            messages: vec![AnthropicMessage::user_text(extraction_prompt)],
            max_tokens: 1024,
            temperature: Some(0.3), // Lower temperature for more consistent extraction
            system: Some("You are a memory extraction system. Output only valid JSON.".to_string()),
            tools: None,
            stream: None,
            thinking: None,
            cache_control: None, // Short one-shot requests don't benefit from caching
        };

        let response = client.complete_anthropic(&request).await?;

        // Parse the response
        let mut memories = Vec::new();
        for block in &response.content {
            if let ContentBlock::Text { text } = block {
                let trimmed = text.trim();

                // Strip markdown code fences if present (LLMs often wrap JSON in ```json ... ```)
                let json_str = if trimmed.starts_with("```") {
                    let without_opening = trimmed
                        .strip_prefix("```json")
                        .or_else(|| trimmed.strip_prefix("```"))
                        .unwrap_or(trimmed);
                    without_opening
                        .strip_suffix("```")
                        .unwrap_or(without_opening)
                        .trim()
                } else {
                    trimmed
                };

                match serde_json::from_str::<Vec<ExtractedMemoryRaw>>(json_str) {
                    Ok(parsed) => {
                        for raw in parsed {
                            memories.push(ExtractedMemory {
                                content: raw.content,
                                category: raw.category,
                                tags: None,
                            });
                        }
                    }
                    Err(e) => {
                        warn!("Memory extraction JSON parse failed: {} — raw response: {}", e, &json_str[..json_str.len().min(200)]);
                    }
                }
            }
        }

        info!("Extracted {} memories from conversation", memories.len());
        Ok(memories)
    }
}

/// A memory extracted from conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedMemory {
    pub content: String,
    pub category: String,
    /// Optional metadata tags (e.g. tool name, source_id, chunk index)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct ExtractedMemoryRaw {
    content: String,
    category: String,
}

/// Internal state for a run
struct RunState {
    iterations: usize,
    tool_records: Vec<ToolCallRecord>,
    input_tokens: u32,
    output_tokens: u32,
    final_text: String,
    confidence: Option<f32>,
    emotional_context: Option<EmotionalContext>,
    /// Accumulated reasoning content
    reasoning_content: String,
    /// Reasoning tokens used
    reasoning_tokens: u32,
    /// Interleaved reasoning blocks
    reasoning_blocks: Vec<ReasoningBlock>,
    /// Current reasoning block being built
    current_reasoning: String,
    /// Tools activated via `discover_tools` this run (on top of core tools)
    active_tools: HashSet<String>,
    /// Per-iteration model statistics for UI display
    model_stats: Vec<crate::model_stats::RequestModelStats>,
    /// Whether we've already injected a narration-loop nudge (only retry once)
    narration_nudged: bool,
    /// Whether we've already injected a thinking-spiral nudge (only retry once)
    thinking_spiral_nudged: bool,
    /// How many wrap-up nudges have been injected (escalates over time)
    wrapup_nudge_count: usize,
}

impl RunState {
    fn new() -> Self {
        Self {
            iterations: 0,
            tool_records: Vec::new(),
            input_tokens: 0,
            output_tokens: 0,
            final_text: String::new(),
            confidence: None,
            emotional_context: None,
            reasoning_content: String::new(),
            reasoning_tokens: 0,
            reasoning_blocks: Vec::new(),
            current_reasoning: String::new(),
            active_tools: HashSet::new(),
            model_stats: Vec::new(),
            narration_nudged: false,
            thinking_spiral_nudged: false,
            wrapup_nudge_count: 0,
        }
    }

    /// Finalize any pending reasoning block before a tool call
    fn finalize_reasoning_block(&mut self, before_tool: Option<String>) {
        if !self.current_reasoning.is_empty() {
            self.reasoning_blocks.push(ReasoningBlock {
                content: std::mem::take(&mut self.current_reasoning),
                iteration: self.iterations,
                before_tool,
            });
        }
    }

    fn into_response(self, truncated: bool) -> AgentResponse {
        let reasoning = if self.reasoning_content.is_empty() && self.reasoning_blocks.is_empty() {
            None
        } else {
            Some(ReasoningContent {
                content: self.reasoning_content,
                tokens: self.reasoning_tokens,
                blocks: self.reasoning_blocks,
            })
        };

        AgentResponse {
            text: self.final_text,
            tool_calls: self.tool_records,
            iterations: self.iterations,
            truncated,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            confidence: self.confidence,
            emotional_context: self.emotional_context,
            reasoning,
            cumulative_input_tokens: u64::from(self.input_tokens),
            cumulative_output_tokens: u64::from(self.output_tokens),
            model_stats: self.model_stats,
        }
    }
}

/// Chunk text into pieces of ~`target_chars` with `overlap_pct` overlap, snapping to line boundaries.
/// Returns (chunk_index, chunk_content) pairs.
fn semantic_chunk(text: &str, target_chars: usize, overlap_pct: f32) -> Vec<(usize, String)> {
    if text.len() <= target_chars {
        return vec![(0, text.to_string())];
    }
    let overlap = (target_chars as f32 * overlap_pct) as usize;
    let step = target_chars.saturating_sub(overlap).max(1);
    let mut chunks = Vec::new();
    let mut pos = 0;

    // Helper: snap a byte index forward to the nearest char boundary
    let snap_char = |idx: usize| -> usize {
        let mut i = idx.min(text.len());
        while i < text.len() && !text.is_char_boundary(i) {
            i += 1;
        }
        i
    };

    while pos < text.len() {
        let end = snap_char((pos + target_chars).min(text.len()));
        // Snap to nearest newline after target (prefer not splitting mid-line)
        let snap_end = if end < text.len() {
            text[pos..end]
                .rfind('\n')
                .map_or(end, |nl| pos + nl + 1)
        } else {
            end
        };
        // Ensure we make progress
        let snap_end = if snap_end <= pos { end } else { snap_end };
        chunks.push((chunks.len(), text[pos..snap_end].to_string()));
        let advance = step.min(snap_end - pos).max(1);
        pos = snap_char(pos + advance);
    }
    chunks
}

/// Find the largest byte index <= max_bytes that is a valid char boundary.
fn truncate_boundary(s: &str, max_bytes: usize) -> usize {
    if s.len() <= max_bytes {
        return s.len();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// Check if a tool name is a write-type tool whose content should be stripped from context.
fn is_write_tool(name: &str) -> bool {
    matches!(name, "write_file" | "write" | "Write" | "create_tool")
}
