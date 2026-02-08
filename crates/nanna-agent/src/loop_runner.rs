//! Agent loop runner

use crate::summarizer::{Summarizer, SummarizerConfig};
use crate::{AgentContext, AgentError, ContextSummarizationConfig, prompts};
use nanna_llm::{
    AnthropicMessage, AnthropicRequest, ContentBlock, LlmClient, StreamEvent,
    ToolDefinition as LlmToolDef,
};
use nanna_tools::{ToolCall, ToolRegistry};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

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
    /// Model to use
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
    /// Threshold (in chars) above which tool results get summarized
    pub summarization_threshold: usize,
    /// Threshold (in chars) above which tool results are stored in memory
    /// and replaced with a summary stub in context. Default: 2000.
    pub context_result_threshold: usize,
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
            summarization_threshold: 50_000, // ~12.5k tokens
            context_result_threshold: 2000,
        }
    }
}

/// Callback for streaming text chunks
pub type StreamCallback = Box<dyn Fn(&str) + Send + Sync>;

/// Callback for storing extracted memories
pub type MemoryCallback = Box<dyn Fn(ExtractedMemory) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;

/// Callback for streaming thinking chunks
pub type ThinkingCallback = Box<dyn Fn(&str) + Send + Sync>;

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
}

/// The main agent
pub struct Agent {
    config: AgentConfig,
    llm: Arc<LlmClient>,
    tools: Arc<ToolRegistry>,
    context: RwLock<AgentContext>,
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
        }
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

            // Build and execute LLM request
            let request = self.build_request_with_thinking(options.thinking_mode).await;
            let result = self.call_llm(&request, &options, &mut state).await?;

            state.input_tokens += result.input_tokens;
            state.output_tokens += result.output_tokens;

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

            // Update final text
            if !result.text.is_empty() {
                state.final_text = result.text;
            }

            // If no tool calls, we're done
            if result.tool_uses.is_empty() {
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
            let tool_results = self.execute_tools(&result.tool_uses, &mut state, &options).await;
            self.store_tool_results(tool_results).await;

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
        ctx.messages.push(AnthropicMessage::user_text(msg));
    }

    async fn call_llm(
        &self,
        request: &AnthropicRequest,
        options: &RunOptions,
        state: &mut RunState,
    ) -> Result<LlmResult, AgentError> {
        if let Some(ref on_text) = options.on_text {
            self.call_llm_streaming(request, on_text, options.on_thinking.as_ref(), state).await
        } else {
            self.call_llm_sync(request, state).await
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

        let mut current_tool_id = String::new();
        let mut current_tool_name = String::new();
        let mut current_tool_json = String::new();
        let mut current_block_type = String::new();

        while let Some(event) = stream.next().await {
            match event? {
                StreamEvent::TextDelta { text, .. } => {
                    on_text(&text);
                    response_text.push_str(&text);
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
                    if current_block_type == "tool_use" && !current_tool_id.is_empty() {
                        if let Ok(input) = serde_json::from_str(&current_tool_json) {
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

        Ok(LlmResult {
            text: response_text,
            tool_uses,
            content_blocks,
            input_tokens: 100, // Placeholder for streaming
            output_tokens,
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
                                Value::String(format!("[{size} bytes written]")),
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

            info!("Executing tool: {name}");
            let response = self.tools.execute(tool_call).await;
            let duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

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
                            Value::String(format!("[{size} bytes written]")),
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

            // Store tool result chunks in memory for future targeted recall
            if let Some(ref on_memory) = options.on_memory {
                let source_id = &Uuid::new_v4().to_string()[..8];
                let chunks = if result_content.len() > 3200 {
                    semantic_chunk(&result_content, 3200, 0.15)
                } else {
                    vec![(0, result_content.clone())]
                };
                let total_chunks = chunks.len();

                for (idx, chunk_content) in &chunks {
                    let mut tags = HashMap::new();
                    tags.insert("tool".to_string(), name.clone());
                    tags.insert("source_id".to_string(), source_id.to_string());
                    tags.insert("chunk".to_string(), format!("{}/{}", idx + 1, total_chunks));

                    on_memory(ExtractedMemory {
                        content: format!("[Tool: {name}] {chunk_content}"),
                        category: "tool_result".to_string(),
                        tags: Some(tags),
                    }).await;
                }
            }

            // For context: summarize large results, noting they're in memory
            let final_content = if result_content.len() > self.config.summarization_threshold {
                // Very large results get full summarization
                self.summarize_tool_result(&name, &result_content).await
            } else if result_content.len() > self.config.context_result_threshold && options.on_memory.is_some() {
                // Medium results: summary stub + memory note
                let preview = &result_content[..self.config.context_result_threshold.min(result_content.len())];
                let chunk_count = (result_content.len() / 3200).max(1);
                format!(
                    "{preview}\n\n[Full result stored in memory ({chunk_count} chunks, {} chars). Use recall to query specific sections.]",
                    result_content.len()
                )
            } else {
                result_content
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

    /// Summarize a large tool result using the configured summarization models (with fallback)
    async fn summarize_tool_result(&self, tool_name: &str, content: &str) -> String {
        // Check if summarization is configured
        if self.config.summarization_priority.is_empty() {
            info!(
                tool = tool_name,
                content_len = content.len(),
                "No summarization models configured, truncating large result"
            );
            let max_chars = self.config.summarization_threshold;
            return format!(
                "{}\n\n[... truncated {} chars, configure summarization_priority for better results ...]",
                &content[..max_chars.min(content.len())],
                content.len().saturating_sub(max_chars)
            );
        }

        // Hierarchical summarization handles any size - the summarizer will:
        // - Chunk content into model-sized pieces (max 20 chunks per level)
        // - Summarize each chunk
        // - Recursively summarize if combined result is still large
        // This keeps API calls bounded (max ~20 calls per level, ~5 levels max = ~100 calls worst case)

        let context_hint = format!(
            "This is the output from a tool called '{}'. Preserve all important information including file paths, code snippets, error messages, and key data.",
            tool_name
        );

        // Try each model in priority order
        for model_spec in &self.config.summarization_priority {
            info!(
                tool = tool_name,
                content_len = content.len(),
                model = %model_spec,
                "Attempting summarization"
            );

            match self.try_summarize_with_model(model_spec, content, &context_hint).await {
                Ok(summary) => {
                    info!(
                        model = %model_spec,
                        original_len = content.len(),
                        summary_len = summary.len(),
                        compression = format!("{:.1}x", content.len() as f64 / summary.len() as f64),
                        "Tool result summarized"
                    );
                    return format!(
                        "[Summarized from {} chars using {}]\n\n{}",
                        content.len(),
                        model_spec,
                        summary
                    );
                }
                Err(e) => {
                    warn!(model = %model_spec, error = %e, "Summarization failed, trying next model");
                }
            }
        }

        // All models failed, truncate as last resort
        warn!("All summarization models failed, truncating");
        let max_chars = self.config.summarization_threshold;
        format!(
            "{}\n\n[... truncated {} chars, all summarization models failed ...]",
            &content[..max_chars.min(content.len())],
            content.len().saturating_sub(max_chars)
        )
    }

    /// Try to summarize content with a specific model
    /// Model format: "provider/model" e.g. "ollama/llama3.2", "openai/gpt-4o-mini", "anthropic/claude-3-haiku"
    async fn try_summarize_with_model(
        &self,
        model_spec: &str,
        content: &str,
        context_hint: &str,
    ) -> Result<String, String> {
        let (client, model_name) = self.create_client_for_model(model_spec)?;

        // Get model's context window from cache or use default
        let context_window = self.get_model_context_window(&client, &model_name).await;

        // Calculate chunk size based on model's context window
        // Reserve tokens for: summarization prompt (~5K), system message (~2K), output (~4K), safety margin (~14K)
        // Use very conservative 2 chars/token ratio (file listings can tokenize at ~2.6 chars/token)
        let usable_tokens = context_window.saturating_sub(25000);
        let chunk_size = (usable_tokens * 2).max(4000); // ~2 chars per token (conservative), min 4k chars

        info!(
            model = model_name,
            context_window = context_window,
            chunk_size = chunk_size,
            "Using model context window for summarization"
        );

        let config = SummarizerConfig {
            model: model_name,
            chunk_size,
            target_summary_size: chunk_size / 4, // Target 25% compression per chunk
            chunk_overlap: (chunk_size / 20).max(100), // 5% overlap, min 100 chars
            // Use half the context window to reduce VRAM/KV cache usage
            context_limit: Some((context_window / 2) as u32),
        };

        let summarizer = Summarizer::new(client, config);
        summarizer.summarize(content, context_hint).await
    }

    /// Get a model's context window from cache or API
    async fn get_model_context_window(&self, client: &LlmClient, model: &str) -> usize {
        // Try to get from cache, fetch from API if needed, fall back to defaults
        let cache = nanna_llm::ModelInfoCache::default_location();
        let info = client.get_model_info(model, cache.as_ref()).await;
        info.context_window
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
                    // Would need API key from config - for now error
                    return Err("OpenAI summarization requires API key configuration".to_string());
                }
                "anthropic" => {
                    // Use the same client (it already has auth)
                    (*self.llm).clone()
                }
                "openrouter" => {
                    return Err("OpenRouter summarization requires API key configuration".to_string());
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

    async fn store_tool_results(&self, tool_results: Vec<ContentBlock>) {
        let mut ctx = self.context.write().await;
        ctx.messages.push(AnthropicMessage::user(tool_results));
    }

    async fn build_request_with_thinking(&self, thinking_override: Option<ThinkingMode>) -> AnthropicRequest {
        let ctx = self.context.read().await;

        let tool_defs = self.tools.definitions().await;
        let tools: Vec<LlmToolDef> = tool_defs
            .iter()
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

        // Use the first summarization model if available (cheaper than main model)
        let (client, model_name) = if !self.config.summarization_priority.is_empty() {
            match self.create_client_for_model(&self.config.summarization_priority[0]) {
                Ok(pair) => pair,
                Err(_) => ((*self.llm).clone(), self.config.model.clone()),
            }
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
        };

        let response = client.complete_anthropic(&request).await?;

        // Parse the response
        let mut memories = Vec::new();
        for block in &response.content {
            if let ContentBlock::Text { text } = block {
                // Try to parse as JSON array
                if let Ok(parsed) = serde_json::from_str::<Vec<ExtractedMemoryRaw>>(text.trim()) {
                    for raw in parsed {
                        memories.push(ExtractedMemory {
                            content: raw.content,
                            category: raw.category,
                            tags: None,
                        });
                    }
                }
            }
        }

        debug!("Extracted {} memories from conversation", memories.len());
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
}

impl RunState {
    const fn new() -> Self {
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

    while pos < text.len() {
        let end = (pos + target_chars).min(text.len());
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
        pos += step.min(snap_end - pos).max(1);
    }
    chunks
}

/// Check if a tool name is a write-type tool whose content should be stripped from context.
fn is_write_tool(name: &str) -> bool {
    matches!(name, "write_file" | "write" | "Write")
}
