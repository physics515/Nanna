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
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-20250514".to_string(),
            max_tokens: 8192,
            temperature: 0.7,
            max_iterations: 10,
        }
    }
}

/// Callback for streaming text chunks
pub type StreamCallback = Box<dyn Fn(&str) + Send + Sync>;

/// Options for running the agent
#[derive(Default)]
pub struct RunOptions {
    /// Override max iterations for this run
    pub max_iterations: Option<usize>,
    /// Additional context to prepend
    pub context_prefix: Option<String>,
    /// Callback for streaming text (called with each text chunk)
    pub on_text: Option<StreamCallback>,
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
            let request = self.build_request().await;
            let result = self.call_llm(&request, &options).await?;

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
                return Ok(state.into_response(false));
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
    ) -> Result<LlmResult, AgentError> {
        if let Some(ref on_text) = options.on_text {
            self.call_llm_streaming(request, on_text).await
        } else {
            self.call_llm_sync(request).await
        }
    }

    async fn call_llm_streaming(
        &self,
        request: &AnthropicRequest,
        on_text: &StreamCallback,
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
                StreamEvent::ContentBlockStart { content_type, .. } => {
                    current_block_type = content_type;
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

    async fn call_llm_sync(&self, request: &AnthropicRequest) -> Result<LlmResult, AgentError> {
        let response = self.llm.complete_anthropic(request).await?;
        let mut tool_uses = Vec::new();
        let mut response_text = String::new();

        for block in &response.content {
            match block {
                ContentBlock::Text { text } => response_text.push_str(text),
                ContentBlock::ToolUse { id, name, input } => {
                    tool_uses.push((id.clone(), name.clone(), input.clone()));
                }
                ContentBlock::ToolResult { .. } => {}
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

    async fn build_request(&self) -> AnthropicRequest {
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

        AnthropicRequest {
            model: self.config.model.clone(),
            messages: ctx.messages.clone(),
            max_tokens: self.config.max_tokens,
            temperature: Some(self.config.temperature),
            system: if ctx.system_prompt.is_empty() {
                None
            } else {
                Some(ctx.system_prompt.clone())
            },
            tools: if tools.is_empty() { None } else { Some(tools) },
            stream: None,
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
}

/// Internal state for a run
struct RunState {
    iterations: usize,
    tool_records: Vec<ToolCallRecord>,
    input_tokens: u32,
    output_tokens: u32,
    final_text: String,
}

impl RunState {
    const fn new() -> Self {
        Self {
            iterations: 0,
            tool_records: Vec::new(),
            input_tokens: 0,
            output_tokens: 0,
            final_text: String::new(),
        }
    }

    fn into_response(self, truncated: bool) -> AgentResponse {
        AgentResponse {
            text: self.final_text,
            tool_calls: self.tool_records,
            iterations: self.iterations,
            truncated,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
        }
    }
}
