//! Agent loop runner

use crate::{AgentContext, AgentError, prompts};
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
use tracing::{debug, info};
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
    /// Maximum iterations (tool call rounds)
    pub max_iterations: usize,
    /// Thinking mode for extended reasoning
    pub thinking_mode: ThinkingMode,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-20250514".to_string(),
            max_tokens: 8192,
            temperature: 0.7,
            max_iterations: 10,
            thinking_mode: ThinkingMode::Instant,
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
    /// Enable context compression for long conversations
    pub enable_context_compression: bool,
    /// Maximum context tokens before compression kicks in
    pub context_compression_threshold: Option<usize>,
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
        let max_iterations = options.max_iterations.unwrap_or(self.config.max_iterations);
        let mut state = RunState::new();

        // Add user message
        self.add_user_message(message, &options).await;

        // Agent loop
        loop {
            state.iterations += 1;
            if state.iterations > max_iterations {
                return Ok(state.into_response(true));
            }

            debug!("Agent iteration {}/{}", state.iterations, max_iterations);

            // Build and execute LLM request
            let request = self.build_request_with_thinking(options.thinking_mode).await;
            let result = self.call_llm(&request, &options, &mut state).await?;

            state.input_tokens += result.input_tokens;
            state.output_tokens += result.output_tokens;

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
            let tool_results = self.execute_tools(&result.tool_uses, &mut state).await;
            self.store_tool_results(tool_results).await;
        }
    }

    async fn add_user_message(&self, message: &str, options: &RunOptions) {
        let mut ctx = self.context.write().await;
        let msg = options
            .context_prefix
            .as_ref()
            .map_or_else(|| message.to_string(), |prefix| format!("{prefix}\n\n{message}"));
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
        ctx.messages
            .push(AnthropicMessage::assistant(content_blocks.to_vec()));
    }

    async fn execute_tools(
        &self,
        tool_uses: &[(String, String, Value)],
        state: &mut RunState,
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

            state.tool_records.push(ToolCallRecord {
                id: id.clone(),
                name: name.clone(),
                input: input.clone(),
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

            tool_results.push(ContentBlock::ToolResult {
                tool_use_id: id.clone(),
                content: result_content,
                is_error: if response.result.success {
                    None
                } else {
                    Some(true)
                },
            });
        }

        tool_results
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
            messages: ctx.messages.clone(),
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

        let request = AnthropicRequest {
            model: self.config.model.clone(),
            messages: vec![AnthropicMessage::user_text(extraction_prompt)],
            max_tokens: 1024,
            temperature: Some(0.3), // Lower temperature for more consistent extraction
            system: Some("You are a memory extraction system. Output only valid JSON.".to_string()),
            tools: None,
            stream: None,
            thinking: None,
        };

        let response = self.llm.complete_anthropic(&request).await?;

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
        }
    }
}
