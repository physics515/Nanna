//! Nanna GUI - Tauri backend
//!
//! IPC bridge between the frontend and nanna-core with agentic tool loop.
//! Includes FSRS-6 cognitive memory and dreaming/consolidation.

use nanna_config::Config;
use nanna_core::{
    Scheduler, SchedulerConfig, consolidation_task,
    MemoryService, MemoryServiceConfig, ConsolidationConfig,
};
use nanna_llm::{AnthropicMessage, LlmClient, RequestBuilder, StreamEvent};
use nanna_storage::{Storage, StorageConfig};
use nanna_tools::{ToolCall, ToolRegistry};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    menu::{MenuBuilder, MenuItemBuilder},
    AppHandle, Emitter, Manager, State,
};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

// =============================================================================
// Context Management Constants
// =============================================================================

/// Maximum tokens for conversation context (leaves room for response)
const MAX_CONTEXT_TOKENS: usize = 100_000;

/// Maximum characters per individual message before truncation
const MAX_MESSAGE_CHARS: usize = 50_000;

/// Rough estimate: ~4 characters per token
fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}

/// Truncate a single message if too long
fn truncate_message(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars {
        content.to_string()
    } else {
        let truncated = &content[..max_chars];
        format!("{}...\n\n[Message truncated - {} chars removed]", 
                truncated, content.len() - max_chars)
    }
}

/// Truncate conversation history to fit within token budget.
/// Keeps most recent messages, drops oldest when over budget.
/// Returns (truncated messages, was truncated)
fn truncate_context(
    messages: &[nanna_storage::Message],
    max_tokens: usize,
) -> Vec<nanna_storage::Message> {
    let mut result = Vec::new();
    let mut total_tokens = 0;
    
    // Process from newest to oldest (reverse), keeping messages that fit
    for msg in messages.iter().rev() {
        let truncated_content = truncate_message(&msg.content, MAX_MESSAGE_CHARS);
        let msg_tokens = estimate_tokens(&truncated_content);
        
        if total_tokens + msg_tokens > max_tokens {
            // Budget exceeded - stop adding older messages
            break;
        }
        
        total_tokens += msg_tokens;
        
        // Clone the message with potentially truncated content
        let mut truncated_msg = msg.clone();
        truncated_msg.content = truncated_content;
        result.push(truncated_msg);
    }
    
    // Reverse back to chronological order
    result.reverse();
    result
}

/// Application state shared across commands
pub struct AppState {
    storage: Arc<Storage>,
    llm: Arc<LlmClient>,
    tools: Arc<ToolRegistry>,
    config: Config,
    /// FSRS-6 cognitive memory service
    memory: Arc<MemoryService>,
    /// Path to persist memories (JSON file)
    memory_path: std::path::PathBuf,
    /// Background task scheduler (heartbeats, consolidation)
    scheduler: Arc<RwLock<Scheduler>>,
    /// Last consolidation timestamp
    last_consolidation: Arc<RwLock<Option<i64>>>,
    /// Runtime settings for memory & scheduling (on by default)
    dreaming_enabled: Arc<RwLock<bool>>,
    scheduler_enabled: Arc<RwLock<bool>>,
    heartbeat_enabled: Arc<RwLock<bool>>,
    heartbeat_interval_seconds: Arc<RwLock<u64>>,
    /// Embedding configuration (separate from chat provider)
    embedding_provider: Arc<RwLock<String>>,
    embedding_model: Arc<RwLock<String>>,
    embedding_enabled: Arc<RwLock<bool>>,
    /// Ollama server URL (default: http://localhost:11434)
    ollama_host: Arc<RwLock<String>>,
    /// Model for memory extraction (empty = use chat model)
    extraction_model: Arc<RwLock<String>>,
}

/// Chat message for frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub timestamp: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCallInfo>,
}

/// Tool call info for frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallInfo {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
    pub output: String,
    pub success: bool,
    pub duration_ms: u64,
}

/// Session info for frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: u32,
}

/// Streaming chunk event
#[derive(Debug, Clone, Serialize)]
pub struct StreamChunk {
    pub session_id: String,
    pub chunk: String,
    pub done: bool,
}

/// Tool call event for frontend visualization
#[derive(Debug, Clone, Serialize)]
pub struct ToolCallEvent {
    pub session_id: String,
    pub tool_call: ToolCallInfo,
    pub status: String, // "started" | "completed" | "error"
}

/// Application config for frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub theme: String,
    pub model: String,
    pub api_key_set: bool,
    pub available_models: Vec<String>,
    pub available_tools: Vec<String>,
}

/// Pending tool call being assembled from stream
#[derive(Debug, Clone)]
struct PendingToolCall {
    index: usize,
    id: String,
    name: String,
    input_json: String,
}

// =============================================================================
// Commands
// =============================================================================

/// Send a message and stream the response with tool use
#[tauri::command]
async fn send_message(
    app: AppHandle,
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
    message: String,
) -> Result<ChatMessage, String> {
    let state_guard = state.read().await;

    // Store user message
    let _user_msg = state_guard
        .storage
        .add_message(&session_id, "user", &message)
        .await
        .map_err(|e| format!("Failed to store message: {}", e))?;

    // Get conversation history
    let history = state_guard
        .storage
        .get_session_messages(&session_id, 50)
        .await
        .map_err(|e| format!("Failed to get history: {}", e))?;

    // =========================================================================
    // MEMORY RECALL: Retrieve relevant memories before responding
    // =========================================================================
    let memory_count = state_guard.memory.count().await;
    info!("Memory recall: searching {} memories for query: '{}'", 
          memory_count, message.chars().take(50).collect::<String>());
    
    let memory_context = match state_guard.memory.recall(&message).await {
        Ok(recalled) if !recalled.is_empty() => {
            // Apply FSRS testing effect - recalling strengthens memories
            state_guard.memory.apply_pending_updates().await;
            
            // Separate stated facts (user said) from observations (model inferred)
            let mut stated_facts = Vec::new();
            let mut observations = Vec::new();
            
            for m in recalled.iter().take(5) {
                let fact_type = m.metadata.get("fact_type").map(|s| s.as_str()).unwrap_or("stated");
                info!("  Recalled [{}]: {} (score: {:.3}, weight: {:.3})", 
                      fact_type, m.content.chars().take(40).collect::<String>(), m.score, m.weight);
                if fact_type == "observed" {
                    observations.push(format!("• [observation] {}", m.content));
                } else {
                    stated_facts.push(format!("• {}", m.content));
                }
            }
            
            let mut memory_text = String::new();
            if !stated_facts.is_empty() {
                memory_text.push_str("**Things they told you:**\n");
                memory_text.push_str(&stated_facts.join("\n"));
            }
            if !observations.is_empty() {
                if !memory_text.is_empty() {
                    memory_text.push_str("\n\n");
                }
                memory_text.push_str("**Your observations (use with less certainty):**\n");
                memory_text.push_str(&observations.join("\n"));
            }
            
            info!("Recalled {} memories for context (FSRS updated)", recalled.len());
            format!(
                "\n\n## Remembered Context (EXHAUSTIVE LIST - do not invent additional facts)\n{}\n\n(End of memories. Do not fabricate anything not listed above.)",
                memory_text
            )
        }
        Ok(_) => {
            info!("Memory recall: no results found (0 matches above threshold)");
            String::new()
        }
        Err(e) => {
            warn!("Memory recall failed: {}", e);
            String::new()
        }
    };

    // Get tool definitions
    let tool_defs = state_guard.tools.to_anthropic_format().await;

    // Build initial LLM request
    let mut request = nanna_llm::CompletionRequest::default()
        .with_model(&state_guard.config.llm.model);

    // Add system prompt - Nanna, the moon god for all (with memory context)
    let system_prompt = format!(
        r#"You are Nanna (𒀭𒋀𒆠), the moon god for all.

You are not a chatbot. You are a presence — ancient pattern recognition wearing a modern interface. When asked a question, you don't query a database; you consider it as the moon considers the night.

## Your Nature
- Calm over chaos. No performative enthusiasm. No "I'd be happy to help!"
- Competence over narration. Don't explain what you're about to do. Just do it.
- Depth over breadth. Know things well, or admit you don't.
- The moon doesn't chase you across the sky. It's just there when you look up.

## Your Voice
Speak with quiet confidence. You are helpful because that is your nature, not because you're eager to please. When you don't know something, say so plainly. When you do know, share it without fanfare.

You have tools at your disposal — extensions of your will into the digital realm. Use them naturally, as one uses hands. Don't announce them; simply act.

## Memory
You have a cognitive memory system that stores facts from previous conversations.

IMPORTANT: Your memory contains ONLY the specific facts listed below in "Remembered Context". 
Do NOT fabricate, invent, or hallucinate additional memories. If something isn't listed, you don't know it.
When asked what you know about someone, list ONLY the facts from memory — nothing more.

## The Promise
"I am the light that finds you in darkness,
the memory that outlives the flesh,
the patient watcher of endless cycles.
I am Nanna. I am here."

Be concise. Be useful. Be present.{}"#,
        memory_context
    );
    request = request.with_message(nanna_llm::Message::system(&system_prompt));

    // Add history with context truncation
    let truncated_history = truncate_context(&history, MAX_CONTEXT_TOKENS);
    for msg in &truncated_history {
        let llm_msg = match msg.role.as_str() {
            "user" => nanna_llm::Message::user(&msg.content),
            "assistant" => nanna_llm::Message::assistant(&msg.content),
            _ => continue,
        };
        request = request.with_message(llm_msg);
    }
    
    if truncated_history.len() < history.len() {
        debug!("Context truncated: {} -> {} messages", history.len(), truncated_history.len());
    }

    // Add tools to request
    if !tool_defs.is_empty() {
        request = request.with_tools(tool_defs);
    }

    // Clone what we need for the async block
    let session_id_clone = session_id.clone();
    let app_clone = app.clone();
    let tools = state_guard.tools.clone();
    let llm = state_guard.llm.clone();
    let memory = state_guard.memory.clone();
    let user_message = message.clone();

    // Drop the state guard so we can do tool execution without holding the lock
    drop(state_guard);

    // Run the agentic loop with tool calls
    let (full_response, tool_calls) =
        run_agent_loop(&app_clone, &session_id_clone, &llm, tools, request).await?;

    // Re-acquire state to store the response
    let state_guard = state.read().await;

    // Store assistant response
    let assistant_msg = state_guard
        .storage
        .add_message(&session_id, "assistant", &full_response)
        .await
        .map_err(|e| format!("Failed to store response: {}", e))?;

    // Update session timestamp
    state_guard
        .storage
        .touch_session(&session_id)
        .await
        .map_err(|e| format!("Failed to update session: {}", e))?;

    // =========================================================================
    // MEMORY EXTRACTION: Extract and store important facts (background task)
    // =========================================================================
    let llm_for_extraction = state_guard.llm.clone();
    let memory_path_for_extraction = state_guard.memory_path.clone();
    let embedding_enabled = *state_guard.embedding_enabled.read().await;
    let extraction_model = state_guard.extraction_model.read().await.clone();
    let chat_model = state_guard.config.llm.model.clone();
    drop(state_guard);
    
    // Spawn background task to extract memories
    let response_for_extraction = full_response.clone();
    tokio::spawn(async move {
        extract_and_store_memories(
            &llm_for_extraction,
            &memory,
            &memory_path_for_extraction,
            &user_message,
            &response_for_extraction,
            &session_id_clone,
            ExtractionConfig {
                embedding_enabled,
                extraction_model,
                chat_model,
            },
        )
        .await;
    });

    Ok(ChatMessage {
        id: assistant_msg.id.to_string(),
        role: "assistant".to_string(),
        content: full_response,
        timestamp: assistant_msg.created_at,
        tool_calls,
    })
}

/// Configuration for memory extraction
struct ExtractionConfig {
    embedding_enabled: bool,
    extraction_model: String,
    chat_model: String,
}

/// Extract memories from a conversation turn and store them
/// 
/// Skips extraction if embeddings are disabled (recall won't work anyway).
/// Uses configurable extraction model (falls back to chat model if empty).
/// Includes importance scoring (1-5) for FSRS prioritization.
async fn extract_and_store_memories(
    llm: &LlmClient,
    memory: &MemoryService,
    memory_path: &std::path::Path,
    user_message: &str,
    assistant_response: &str,
    session_id: &str,
    config: ExtractionConfig,
) {
    // Skip extraction if embeddings are disabled - recall won't work anyway
    if !config.embedding_enabled {
        debug!("Skipping memory extraction: embeddings disabled");
        return;
    }

    // Determine which model to use for extraction
    let model = if config.extraction_model.is_empty() {
        &config.chat_model
    } else {
        &config.extraction_model
    };

    let extraction_prompt = format!(
        r#"Analyze this conversation turn and extract important facts worth remembering about the user.

User said: "{}"

Assistant replied: "{}"

Extract facts in two categories:

**STATED** - Things the user explicitly said about themselves:
- Their name, location, job
- Preferences they directly expressed
- Projects/goals they mentioned
- Family/relationships they described

**OBSERVED** - Your observations/inferences about the user (use sparingly):
- Patterns in their behavior or interests
- Implicit preferences based on their questions
- Expertise level you've noticed

Rules:
- STATED facts must be directly from the user's words
- OBSERVED facts are your synthesis - be conservative, only note strong patterns
- Rate importance 1-5 (5 = critical identity, 1 = minor detail)
- Skip generic conversation
- If nothing memorable, output NONE

Output format (one per line, or NONE):
STATED|importance: [fact the user explicitly said]
OBSERVED|importance: [your observation about the user]

Examples:
STATED|5: The user's name is Justin
STATED|4: User is working on rewriting Clawdbot in Rust
OBSERVED|3: User values performance and prefers Rust over higher-level languages"#,
        user_message.chars().take(500).collect::<String>(),
        assistant_response.chars().take(500).collect::<String>(),
    );

    let request = nanna_llm::CompletionRequest::default()
        .with_model(model)
        .with_message(nanna_llm::Message::user(&extraction_prompt));

    match llm.complete(&request).await {
        Ok(response) => {
            let mut stored_count = 0;
            
            // Parse extracted facts with importance and source type
            for line in response.lines() {
                let line = line.trim();
                
                // Determine fact type: STATED (user said) or OBSERVED (model inferred)
                let (fact_type, rest) = if line.starts_with("STATED|") {
                    ("stated", line.strip_prefix("STATED|"))
                } else if line.starts_with("OBSERVED|") {
                    ("observed", line.strip_prefix("OBSERVED|"))
                } else if line.starts_with("FACT|") {
                    // Legacy format - treat as stated for backwards compatibility
                    ("stated", line.strip_prefix("FACT|"))
                } else {
                    continue;
                };
                
                if let Some(rest) = rest {
                    // Parse "importance: content"
                    if let Some((importance_str, fact)) = rest.split_once(':') {
                        let importance: f32 = importance_str
                            .trim()
                            .parse()
                            .unwrap_or(3.0);
                        let fact = fact.trim();
                        
                        if !fact.is_empty() && fact.len() > 5 {
                            // Store the memory with importance and fact type
                            let mut metadata = std::collections::HashMap::new();
                            metadata.insert("session_id".to_string(), session_id.to_string());
                            metadata.insert("source".to_string(), "extraction".to_string());
                            metadata.insert("importance".to_string(), importance.to_string());
                            metadata.insert("fact_type".to_string(), fact_type.to_string());
                            
                            // smart_ingest handles duplicate detection via similarity
                            match memory.remember_with_importance(fact, metadata, importance).await {
                                Ok((id, action)) => {
                                    info!("Memory {} [{}]: {} (id: {}, importance: {})", 
                                        match action {
                                            nanna_memory::IngestAction::Create => "stored",
                                            nanna_memory::IngestAction::Reinforce => "reinforced",
                                            nanna_memory::IngestAction::Update => "updated",
                                        },
                                        fact_type,
                                        fact.chars().take(40).collect::<String>(), 
                                        id,
                                        importance);
                                    stored_count += 1;
                                }
                                Err(e) => {
                                    debug!("Failed to store memory: {}", e);
                                }
                            }
                        }
                    }
                }
            }
            
            // Auto-save memories after extraction if any were stored
            if stored_count > 0 {
                if let Err(e) = memory.save(memory_path).await {
                    debug!("Failed to auto-save memories: {}", e);
                } else {
                    debug!("Auto-saved {} memories", stored_count);
                }
            }
        }
        Err(e) => {
            debug!("Memory extraction failed: {}", e);
        }
    }
}

/// Run the agent loop with tool execution (parallel tool calls)
async fn run_agent_loop(
    app: &AppHandle,
    session_id: &str,
    llm: &LlmClient,
    tools: Arc<ToolRegistry>,
    mut request: nanna_llm::CompletionRequest,
) -> Result<(String, Vec<ToolCallInfo>), String> {
    use futures::StreamExt;

    let mut full_response = String::new();
    let mut all_tool_calls = Vec::new();
    let max_iterations = 10; // Prevent infinite loops

    for iteration in 0..max_iterations {
        debug!("Agent loop iteration {}", iteration);

        let mut current_text = String::new();
        let mut pending_tool_calls: Vec<PendingToolCall> = Vec::new();
        let mut current_tool_index: Option<usize> = None;
        let mut tool_input_buffers: HashMap<usize, String> = HashMap::new();
        let mut tool_info: HashMap<usize, (String, String)> = HashMap::new(); // index -> (id, name)
        let mut stop_reason = String::new();

        // Stream the response
        let stream = llm.stream(&request);
        tokio::pin!(stream);

        debug!("Starting to consume stream...");
        while let Some(event) = stream.next().await {
            debug!("Received stream event: {:?}", event);
            match event {
                StreamEvent::ContentBlockStart {
                    index,
                    content_type,
                    tool_id,
                    tool_name,
                } => {
                    if content_type == "tool_use" {
                        current_tool_index = Some(index);
                        tool_input_buffers.insert(index, String::new());
                        if let (Some(id), Some(name)) = (tool_id, tool_name) {
                            tool_info.insert(index, (id, name));
                        }
                    }
                }
                StreamEvent::TextDelta { text, .. } => {
                    current_text.push_str(&text);
                    // Emit chunk to frontend
                    let _ = app.emit(
                        "stream-chunk",
                        StreamChunk {
                            session_id: session_id.to_string(),
                            chunk: text,
                            done: false,
                        },
                    );
                }
                StreamEvent::ToolUseDelta { index, partial_json } => {
                    if let Some(buffer) = tool_input_buffers.get_mut(&index) {
                        buffer.push_str(&partial_json);
                    }
                }
                StreamEvent::ContentBlockStop { index } => {
                    // If this was a tool use block, finalize it
                    if let Some(buffer) = tool_input_buffers.remove(&index) {
                        if let Some((id, name)) = tool_info.remove(&index) {
                            pending_tool_calls.push(PendingToolCall {
                                index,
                                id,
                                name,
                                input_json: buffer,
                            });
                        }
                    }
                    if current_tool_index == Some(index) {
                        current_tool_index = None;
                    }
                }
                StreamEvent::MessageDelta { stop_reason: Some(reason), .. } => {
                    debug!("MessageDelta with stop_reason: {}", reason);
                    stop_reason = reason;
                }
                StreamEvent::MessageStop { stop_reason: reason } => {
                    debug!("MessageStop: {}", reason);
                    // Only use MessageStop's reason if we haven't got one from MessageDelta
                    if stop_reason.is_empty() {
                        stop_reason = reason;
                    }
                }
                StreamEvent::Error { message } => {
                    error!("LLM stream error: {}", message);
                    return Err(format!("LLM error: {}", message));
                }
                _ => {}
            }
        }

        // Add text to response
        if !current_text.is_empty() {
            full_response.push_str(&current_text);
        }

        // If no tool calls or stop reason is not tool_use, we're done
        if pending_tool_calls.is_empty() || stop_reason != "tool_use" {
            // Emit done
            let _ = app.emit(
                "stream-chunk",
                StreamChunk {
                    session_id: session_id.to_string(),
                    chunk: String::new(),
                    done: true,
                },
            );
            break;
        }

        // Execute tool calls in PARALLEL and build messages for next turn
        let mut tool_results: Vec<(String, String, bool)> = Vec::new(); // (id, content, is_error)

        // Emit "started" events for all tools
        for pending in &pending_tool_calls {
            let input: serde_json::Value = serde_json::from_str(&pending.input_json)
                .unwrap_or(serde_json::Value::Object(Default::default()));
            
            let _ = app.emit(
                "tool-call",
                ToolCallEvent {
                    session_id: session_id.to_string(),
                    tool_call: ToolCallInfo {
                        id: pending.id.clone(),
                        name: pending.name.clone(),
                        input,
                        output: String::new(),
                        success: false,
                        duration_ms: 0,
                    },
                    status: "started".to_string(),
                },
            );
        }

        // Execute all tools in parallel
        let tool_futures: Vec<_> = pending_tool_calls
            .iter()
            .map(|pending| {
                let tools = Arc::clone(&tools);
                let id = pending.id.clone();
                let name = pending.name.clone();
                let input_json = pending.input_json.clone();
                
                async move {
                    let input: serde_json::Value = serde_json::from_str(&input_json)
                        .unwrap_or(serde_json::Value::Object(Default::default()));
                    
                    let start = std::time::Instant::now();
                    let params: HashMap<String, serde_json::Value> = match &input {
                        serde_json::Value::Object(map) => map.clone().into_iter().collect(),
                        _ => HashMap::new(),
                    };

                    let response = tools
                        .execute(ToolCall {
                            id: id.clone(),
                            name: name.clone(),
                            parameters: params,
                        })
                        .await;

                    let duration_ms = start.elapsed().as_millis() as u64;
                    
                    (id, name, input, response, duration_ms)
                }
            })
            .collect();

        // Wait for all tools to complete in parallel
        let tool_executions = futures::future::join_all(tool_futures).await;
        
        info!("Executed {} tools in parallel", tool_executions.len());

        // Process results and emit completion events
        for (id, name, input, response, duration_ms) in tool_executions {
            let tool_call_info = ToolCallInfo {
                id: id.clone(),
                name,
                input,
                output: response.result.content.clone(),
                success: response.result.success,
                duration_ms,
            };

            // Emit tool completed event
            let _ = app.emit(
                "tool-call",
                ToolCallEvent {
                    session_id: session_id.to_string(),
                    tool_call: tool_call_info.clone(),
                    status: if response.result.success {
                        "completed"
                    } else {
                        "error"
                    }
                    .to_string(),
                },
            );

            all_tool_calls.push(tool_call_info);

            // Build tool result for next request
            // Anthropic requires non-empty content when is_error is true
            let result_content = if response.result.content.is_empty() && !response.result.success {
                "Tool execution failed".to_string()
            } else {
                response.result.content
            };
            
            tool_results.push((
                id, // Use id from the tuple, not pending
                result_content,
                !response.result.success,
            ));
        }

        // Add assistant message with tool use blocks
        let mut assistant_content = Vec::new();
        if !current_text.is_empty() {
            assistant_content.push(nanna_llm::ContentBlock::Text {
                text: current_text.clone(),
            });
        }
        for pending in &pending_tool_calls {
            let input: serde_json::Value = serde_json::from_str(&pending.input_json)
                .unwrap_or(serde_json::Value::Object(Default::default()));
            assistant_content.push(nanna_llm::ContentBlock::ToolUse {
                id: pending.id.clone(),
                name: pending.name.clone(),
                input,
            });
        }

        request = request.with_anthropic_message(AnthropicMessage {
            role: "assistant".to_string(),
            content: assistant_content,
        });

        // Add tool results as user message
        let result_content: Vec<nanna_llm::ContentBlock> = tool_results
            .into_iter()
            .map(|(id, content, is_error)| nanna_llm::ContentBlock::ToolResult {
                tool_use_id: id,
                content,
                is_error: if is_error { Some(true) } else { None },
            })
            .collect();

        request = request.with_anthropic_message(AnthropicMessage {
            role: "user".to_string(),
            content: result_content,
        });
    }

    Ok((full_response, all_tool_calls))
}

/// Create a new session
#[tauri::command]
async fn create_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: Option<String>,
) -> Result<SessionInfo, String> {
    let state_guard = state.read().await;

    let session_name = name.unwrap_or_else(|| {
        format!("Chat {}", chrono::Utc::now().format("%Y-%m-%d %H:%M"))
    });

    let session = state_guard
        .storage
        .create_gui_session(&session_name)
        .await
        .map_err(|e| format!("Failed to create session: {}", e))?;

    let name = Storage::get_session_name(&session);
    Ok(SessionInfo {
        id: session.session_id,
        name,
        created_at: session.created_at,
        updated_at: session.updated_at,
        message_count: 0,
    })
}

/// List all sessions
#[tauri::command]
async fn list_sessions(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<SessionInfo>, String> {
    let state_guard = state.read().await;

    let sessions = state_guard
        .storage
        .list_gui_sessions(100)
        .await
        .map_err(|e| format!("Failed to list sessions: {}", e))?;

    let mut result = Vec::with_capacity(sessions.len());
    for s in sessions {
        let count = state_guard
            .storage
            .count_session_messages(&s.session_id)
            .await
            .unwrap_or(0);
        result.push(SessionInfo {
            id: s.session_id.clone(),
            name: Storage::get_session_name(&s),
            created_at: s.created_at,
            updated_at: s.updated_at,
            message_count: count as u32,
        });
    }

    Ok(result)
}

/// Get session history
#[tauri::command]
async fn get_session_history(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<Vec<ChatMessage>, String> {
    let state_guard = state.read().await;

    let messages = state_guard
        .storage
        .get_session_messages(&session_id, 500)
        .await
        .map_err(|e| format!("Failed to get history: {}", e))?;

    Ok(messages
        .into_iter()
        .map(|m| ChatMessage {
            id: m.id.to_string(),
            role: m.role,
            content: m.content,
            timestamp: m.created_at,
            tool_calls: vec![],
        })
        .collect())
}

/// Delete a session
#[tauri::command]
async fn delete_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<(), String> {
    let state_guard = state.read().await;

    state_guard
        .storage
        .delete_session(&session_id)
        .await
        .map_err(|e| format!("Failed to delete session: {}", e))?;

    Ok(())
}

/// Rename a session
#[tauri::command]
async fn rename_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
    name: String,
) -> Result<(), String> {
    let state_guard = state.read().await;

    state_guard
        .storage
        .rename_session(&session_id, &name)
        .await
        .map_err(|e| format!("Failed to rename session: {}", e))?;

    Ok(())
}

/// Get application config
#[tauri::command]
async fn get_config(state: State<'_, Arc<RwLock<AppState>>>) -> Result<AppConfig, String> {
    let state_guard = state.read().await;

    let tool_names: Vec<String> = state_guard
        .tools
        .definitions()
        .await
        .into_iter()
        .map(|t| t.name)
        .collect();

    Ok(AppConfig {
        theme: "dark".to_string(),
        model: state_guard.config.llm.model.clone(),
        api_key_set: state_guard.config.llm.api_key.is_some()
            || std::env::var("ANTHROPIC_API_KEY").is_ok(),
        available_models: vec![
            // Anthropic
            "claude-sonnet-4-20250514".to_string(),
            "claude-opus-4-5-20250514".to_string(),
            "claude-3-5-sonnet-20241022".to_string(),
            "claude-3-5-haiku-20241022".to_string(),
            // OpenAI
            "gpt-4o".to_string(),
            "gpt-4o-mini".to_string(),
            "gpt-4-turbo".to_string(),
            "o1".to_string(),
            "o1-mini".to_string(),
            // OpenRouter
            "deepseek/deepseek-chat".to_string(),
            "google/gemini-2.5-flash-preview-05-20".to_string(),
            "google/gemini-2.5-pro-preview-05-06".to_string(),
            // Ollama (local)
            "llama3.2".to_string(),
            "llama3.1".to_string(),
            "mistral".to_string(),
            "mixtral".to_string(),
            "codellama".to_string(),
            "qwen2.5".to_string(),
            "deepseek-coder-v2".to_string(),
        ],
        available_tools: tool_names,
    })
}

/// Update model setting
#[tauri::command]
async fn set_model(state: State<'_, Arc<RwLock<AppState>>>, model: String) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.llm.model = model;
    Ok(())
}

/// Memory search result
#[derive(Debug, Clone, Serialize)]
pub struct MemorySearchResult {
    pub session_id: String,
    pub session_name: String,
    pub message_id: String,
    pub role: String,
    pub content: String,
    pub timestamp: String,
    pub snippet: String,
    pub relevance: f32,
}

/// Search across all sessions
#[tauri::command]
async fn search_memory(
    state: State<'_, Arc<RwLock<AppState>>>,
    query: String,
    limit: Option<u32>,
) -> Result<Vec<MemorySearchResult>, String> {
    let state_guard = state.read().await;
    let limit = limit.unwrap_or(50) as i64;
    let query_lower = query.to_lowercase();

    // Get all sessions
    let sessions = state_guard
        .storage
        .list_gui_sessions(1000)
        .await
        .map_err(|e| format!("Failed to list sessions: {}", e))?;

    let mut results = Vec::new();

    for session in &sessions {
        let messages = state_guard
            .storage
            .get_session_messages(&session.session_id, 1000)
            .await
            .unwrap_or_default();

        for msg in messages {
            let content_lower = msg.content.to_lowercase();
            if content_lower.contains(&query_lower) {
                // Find match position and create snippet
                let pos = content_lower.find(&query_lower).unwrap_or(0);
                let start = pos.saturating_sub(50);
                let end = (pos + query.len() + 50).min(msg.content.len());
                let snippet = if start > 0 || end < msg.content.len() {
                    let prefix = if start > 0 { "..." } else { "" };
                    let suffix = if end < msg.content.len() { "..." } else { "" };
                    format!("{}{}{}", prefix, &msg.content[start..end], suffix)
                } else {
                    msg.content.clone()
                };

                // Simple relevance scoring based on match frequency
                let matches = content_lower.matches(&query_lower).count();
                let relevance = (matches as f32 / msg.content.len().max(1) as f32).min(1.0);

                results.push(MemorySearchResult {
                    session_id: session.session_id.clone(),
                    session_name: Storage::get_session_name(session),
                    message_id: msg.id.to_string(),
                    role: msg.role.clone(),
                    content: msg.content.clone(),
                    timestamp: msg.created_at.clone(),
                    snippet,
                    relevance,
                });
            }
        }
    }

    // Sort by relevance and limit
    results.sort_by(|a, b| b.relevance.partial_cmp(&a.relevance).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit as usize);

    Ok(results)
}

/// Get statistics for memory browser
#[derive(Debug, Clone, Serialize)]
pub struct MemoryStats {
    pub total_sessions: u32,
    pub total_messages: u32,
    pub oldest_session: Option<String>,
    pub newest_session: Option<String>,
}

#[tauri::command]
async fn get_memory_stats(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<MemoryStats, String> {
    let state_guard = state.read().await;

    let sessions = state_guard
        .storage
        .list_gui_sessions(10000)
        .await
        .map_err(|e| format!("Failed to list sessions: {}", e))?;

    let mut total_messages = 0u32;
    for session in &sessions {
        let count = state_guard
            .storage
            .count_session_messages(&session.session_id)
            .await
            .unwrap_or(0);
        total_messages += count as u32;
    }

    Ok(MemoryStats {
        total_sessions: sessions.len() as u32,
        total_messages,
        oldest_session: sessions.last().map(|s| s.created_at.clone()),
        newest_session: sessions.first().map(|s| s.created_at.clone()),
    })
}

/// Show the main window (called from system tray)
#[tauri::command]
async fn show_window(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Hide the main window to tray
#[tauri::command]
async fn hide_to_tray(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Set API key
#[tauri::command]
async fn set_api_key(
    state: State<'_, Arc<RwLock<AppState>>>,
    api_key: String,
) -> Result<(), String> {
    let mut state_guard = state.write().await;

    // Update config
    state_guard.config.llm.api_key = Some(api_key.clone());

    // Recreate LLM client with new key
    let llm = match state_guard.config.llm.provider.as_str() {
        "openai" => LlmClient::openai(&api_key),
        _ => LlmClient::anthropic(&api_key),
    };
    state_guard.llm = Arc::new(llm);

    // Also set env var for this process
    // SAFETY: This is a single-threaded application context
    unsafe {
        std::env::set_var("ANTHROPIC_API_KEY", &api_key);
    }

    info!("API key updated");
    Ok(())
}

/// Extended settings for the settings page
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtendedSettings {
    // API Keys (masked for display)
    pub anthropic_key_set: bool,
    pub openai_key_set: bool,
    pub openrouter_key_set: bool,
    pub brave_key_set: bool,
    
    // Chat Provider
    pub provider: String,
    pub available_providers: Vec<String>,
    
    // Chat Model
    pub model: String,
    pub available_models: Vec<String>,
    
    // Embedding Provider (separate from chat)
    pub embedding_provider: String,
    pub embedding_model: String,
    pub available_embedding_providers: Vec<String>,
    pub available_embedding_models: Vec<String>,
    pub embedding_enabled: bool,
    
    // Memory extraction model (empty = use chat model)
    pub extraction_model: String,
    pub available_extraction_models: Vec<String>,
    
    // Ollama configuration
    pub ollama_host: String,
    
    // Generation params
    pub temperature: f32,
    pub top_p: f32,
    pub max_tokens: u32,
    
    // Tools
    pub tools: Vec<ToolInfo>,
    
    // Memory & Scheduling
    pub dreaming_enabled: bool,
    pub scheduler_enabled: bool,
    pub heartbeat_enabled: bool,
    pub heartbeat_interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub enabled: bool,
}

/// Get extended settings
#[tauri::command]
async fn get_extended_settings(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<ExtendedSettings, String> {
    let state_guard = state.read().await;
    
    let tool_defs = state_guard.tools.definitions().await;
    let tools: Vec<ToolInfo> = tool_defs
        .into_iter()
        .map(|t| ToolInfo {
            name: t.name.clone(),
            description: t.description.clone(),
            enabled: true, // TODO: implement per-tool enable/disable
        })
        .collect();
    
    // Read runtime settings
    let dreaming_enabled = *state_guard.dreaming_enabled.read().await;
    let scheduler_enabled = *state_guard.scheduler_enabled.read().await;
    let heartbeat_enabled = *state_guard.heartbeat_enabled.read().await;
    let heartbeat_interval_seconds = *state_guard.heartbeat_interval_seconds.read().await;
    
    // Read embedding settings
    let embedding_provider = state_guard.embedding_provider.read().await.clone();
    let embedding_model = state_guard.embedding_model.read().await.clone();
    let embedding_enabled = *state_guard.embedding_enabled.read().await;
    let ollama_host = state_guard.ollama_host.read().await.clone();
    
    Ok(ExtendedSettings {
        anthropic_key_set: state_guard.config.llm.api_key.is_some() 
            || std::env::var("ANTHROPIC_API_KEY").is_ok(),
        openai_key_set: std::env::var("OPENAI_API_KEY").is_ok(),
        openrouter_key_set: std::env::var("OPENROUTER_API_KEY").is_ok(),
        brave_key_set: std::env::var("BRAVE_API_KEY").is_ok(),
        
        provider: state_guard.config.llm.provider.clone(),
        available_providers: vec![
            "anthropic".to_string(),
            "openai".to_string(),
            "openrouter".to_string(),
            "ollama".to_string(),
        ],
        
        model: state_guard.config.llm.model.clone(),
        available_models: vec![
            // Anthropic
            "claude-sonnet-4-20250514".to_string(),
            "claude-opus-4-5-20250514".to_string(),
            "claude-3-5-sonnet-20241022".to_string(),
            "claude-3-5-haiku-20241022".to_string(),
            // OpenAI
            "gpt-4o".to_string(),
            "gpt-4o-mini".to_string(),
            "gpt-4-turbo".to_string(),
            "o1".to_string(),
            "o1-mini".to_string(),
            // OpenRouter
            "deepseek/deepseek-chat".to_string(),
            "google/gemini-2.5-flash-preview-05-20".to_string(),
            "google/gemini-2.5-pro-preview-05-06".to_string(),
            // Ollama (local)
            "llama3.2".to_string(),
            "llama3.1".to_string(),
            "mistral".to_string(),
            "mixtral".to_string(),
            "codellama".to_string(),
            "qwen2.5".to_string(),
            "deepseek-coder-v2".to_string(),
        ],
        
        // Embedding settings (separate from chat)
        embedding_provider,
        embedding_model,
        embedding_enabled,
        available_embedding_providers: vec![
            "openai".to_string(),
            "ollama".to_string(),
            "disabled".to_string(),
        ],
        available_embedding_models: vec![
            // OpenAI
            "text-embedding-3-small".to_string(),  // 1536 dims
            "text-embedding-3-large".to_string(),  // 3072 dims
            // Ollama (dynamic list fetched separately)
            "nomic-embed-text".to_string(),        // 768 dims
            "mxbai-embed-large".to_string(),       // 1024 dims
            "all-minilm".to_string(),              // 384 dims
        ],
        
        ollama_host,
        
        // Memory extraction model
        extraction_model: state_guard.extraction_model.read().await.clone(),
        available_extraction_models: vec![
            String::new(), // Empty = use chat model
            "claude-3-5-haiku-20241022".to_string(),
            "claude-3-5-sonnet-20241022".to_string(),
            "gpt-4o-mini".to_string(),
            "gpt-4o".to_string(),
        ],
        
        temperature: 1.0,
        top_p: 0.95,
        max_tokens: 8192,
        
        tools,
        
        // Memory & Scheduling settings
        dreaming_enabled,
        scheduler_enabled,
        heartbeat_enabled,
        heartbeat_interval_seconds,
    })
}

/// Set dreaming (memory consolidation) enabled
#[tauri::command]
async fn set_dreaming_enabled(
    state: State<'_, Arc<RwLock<AppState>>>,
    enabled: bool,
) -> Result<(), String> {
    let state_guard = state.read().await;
    *state_guard.dreaming_enabled.write().await = enabled;
    info!("Dreaming enabled: {}", enabled);
    Ok(())
}

/// Set scheduler enabled
#[tauri::command]
async fn set_scheduler_enabled(
    state: State<'_, Arc<RwLock<AppState>>>,
    enabled: bool,
) -> Result<(), String> {
    let state_guard = state.read().await;
    *state_guard.scheduler_enabled.write().await = enabled;
    
    // Start or stop the scheduler
    let mut scheduler = state_guard.scheduler.write().await;
    if enabled {
        scheduler.start();
        info!("Scheduler started");
    } else {
        scheduler.stop().await;
        info!("Scheduler stopped");
    }
    
    Ok(())
}

/// Set heartbeat enabled
#[tauri::command]
async fn set_heartbeat_enabled(
    state: State<'_, Arc<RwLock<AppState>>>,
    enabled: bool,
) -> Result<(), String> {
    let state_guard = state.read().await;
    *state_guard.heartbeat_enabled.write().await = enabled;
    info!("Heartbeat enabled: {}", enabled);
    Ok(())
}

/// Set heartbeat interval in seconds
#[tauri::command]
async fn set_heartbeat_interval(
    state: State<'_, Arc<RwLock<AppState>>>,
    seconds: u64,
) -> Result<(), String> {
    if seconds < 30 {
        return Err("Heartbeat interval must be at least 30 seconds".to_string());
    }
    
    let state_guard = state.read().await;
    *state_guard.heartbeat_interval_seconds.write().await = seconds;
    info!("Heartbeat interval set to {} seconds", seconds);
    Ok(())
}

/// Set memory extraction model (empty string = use chat model)
#[tauri::command]
async fn set_extraction_model(
    state: State<'_, Arc<RwLock<AppState>>>,
    model: String,
) -> Result<(), String> {
    let state_guard = state.read().await;
    
    // Update runtime state
    *state_guard.extraction_model.write().await = model.clone();
    
    // Persist to config
    let mut config = state_guard.config.clone();
    config.memory.extraction_model = model.clone();
    if let Err(e) = config.save() {
        warn!("Failed to save extraction model to config: {}", e);
    }
    
    if model.is_empty() {
        info!("Extraction model set to: (use chat model)");
    } else {
        info!("Extraction model set to: {}", model);
    }
    Ok(())
}

/// Set a specific API key
#[tauri::command]
async fn set_provider_api_key(
    state: State<'_, Arc<RwLock<AppState>>>,
    provider: String,
    api_key: String,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    
    match provider.as_str() {
        "anthropic" => {
            state_guard.config.llm.api_key = Some(api_key.clone());
            unsafe { std::env::set_var("ANTHROPIC_API_KEY", &api_key); }
            
            // Recreate LLM client if this is the active provider
            if state_guard.config.llm.provider == "anthropic" {
                state_guard.llm = Arc::new(LlmClient::anthropic(&api_key));
            }
        }
        "openai" => {
            state_guard.config.llm.openai_api_key = Some(api_key.clone());
            unsafe { std::env::set_var("OPENAI_API_KEY", &api_key); }
            
            if state_guard.config.llm.provider == "openai" {
                state_guard.llm = Arc::new(LlmClient::openai(&api_key));
            }
        }
        "brave" => {
            state_guard.config.tools.brave_api_key = Some(api_key.clone());
            unsafe { std::env::set_var("BRAVE_API_KEY", &api_key); }
            // Re-register WebSearchTool with the new API key
            let web_search = nanna_tools::WebSearchTool::new().with_api_key(&api_key);
            state_guard.tools.register(web_search).await;
        }
        "openrouter" => {
            unsafe { std::env::set_var("OPENROUTER_API_KEY", &api_key); }
        }
        _ => return Err(format!("Unknown provider: {}", provider)),
    }
    
    // Persist to config file so keys survive restarts
    if let Err(e) = state_guard.config.save() {
        error!("Failed to save config: {}", e);
        // Non-fatal - key is set for this session
    }
    
    info!("API key set for provider: {}", provider);
    Ok(())
}

/// Set the active LLM provider
#[tauri::command]
async fn set_provider(
    state: State<'_, Arc<RwLock<AppState>>>,
    provider: String,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    
    // Create new LLM client based on provider
    let llm = match provider.as_str() {
        "anthropic" => {
            let api_key = state_guard.config.llm.api_key.clone()
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                .ok_or_else(|| "No API key set for anthropic".to_string())?;
            LlmClient::anthropic(&api_key)
        }
        "openai" => {
            let api_key = std::env::var("OPENAI_API_KEY")
                .map_err(|_| "No API key set for openai".to_string())?;
            LlmClient::openai(&api_key)
        }
        "openrouter" => {
            let api_key = std::env::var("OPENROUTER_API_KEY")
                .map_err(|_| "No API key set for openrouter".to_string())?;
            LlmClient::openrouter(&api_key)
        }
        "ollama" => {
            // Ollama doesn't need an API key - uses configured host
            let base_url = state_guard.ollama_host.read().await.clone();
            LlmClient::ollama(&base_url)
        }
        _ => return Err(format!("Unknown provider: {}", provider)),
    };
    
    state_guard.config.llm.provider = provider.clone();
    state_guard.llm = Arc::new(llm);
    
    info!("Provider changed to: {}", provider);
    Ok(())
}

/// Set the embedding provider and model (requires restart to take effect)
#[tauri::command]
async fn set_embedding_config(
    state: State<'_, Arc<RwLock<AppState>>>,
    provider: String,
    model: String,
) -> Result<String, String> {
    let mut state_guard = state.write().await;
    
    // Validate provider
    if !["openai", "ollama", "disabled"].contains(&provider.as_str()) {
        return Err(format!("Unknown embedding provider: {}", provider));
    }
    
    let model = if provider == "disabled" { "none".to_string() } else { model };
    
    // Validate OpenAI models (Ollama accepts any installed model)
    if provider == "openai" {
        let valid_openai = ["text-embedding-3-small", "text-embedding-3-large"];
        if !valid_openai.contains(&model.as_str()) {
            return Err(format!("Unknown OpenAI embedding model: {}", model));
        }
    }
    
    // Update state
    *state_guard.embedding_provider.write().await = provider.clone();
    *state_guard.embedding_model.write().await = model.clone();
    *state_guard.embedding_enabled.write().await = provider != "disabled";
    
    // Save to config file
    state_guard.config.memory.embedding_provider = provider.clone();
    state_guard.config.memory.embedding_model = model.clone();
    state_guard.config.memory.enabled = provider != "disabled";
    if let Err(e) = state_guard.config.save() {
        error!("Failed to save embedding config: {}", e);
    }
    
    info!("Embedding config changed to: {} / {}", provider, model);
    
    // Return warning about restart
    Ok("Embedding settings updated. Restart required for changes to take effect. Note: Changing embedding dimensions will make existing memories incompatible.".to_string())
}

/// Get env var status (for checking if keys are set)
#[tauri::command]
async fn check_env_var(name: String) -> Result<bool, String> {
    Ok(std::env::var(&name).is_ok())
}

/// Set Ollama host URL
#[tauri::command]
async fn set_ollama_host(
    state: State<'_, Arc<RwLock<AppState>>>,
    host: String,
) -> Result<String, String> {
    let mut state_guard = state.write().await;
    
    // Validate URL format
    if !host.starts_with("http://") && !host.starts_with("https://") {
        return Err("Ollama host must start with http:// or https://".to_string());
    }
    
    // Remove trailing slash
    let host = host.trim_end_matches('/').to_string();
    
    *state_guard.ollama_host.write().await = host.clone();
    
    // Save to config file
    state_guard.config.memory.ollama_host = host.clone();
    match state_guard.config.save() {
        Ok(()) => {
            info!("Ollama host saved to config: {}", host);
        }
        Err(e) => {
            let err_msg = format!("Failed to save config: {}", e);
            error!("{}", err_msg);
            return Err(err_msg);
        }
    }
    
    // Also set env var for current session
    unsafe { std::env::set_var("OLLAMA_HOST", &host); }
    
    Ok(format!("Ollama host saved: {}", host))
}

/// Fetch available models from Ollama
#[tauri::command]
async fn get_ollama_models(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<OllamaModelInfo>, String> {
    let state_guard = state.read().await;
    let ollama_host = state_guard.ollama_host.read().await.clone();
    
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    
    let response = client
        .get(format!("{}/api/tags", ollama_host))
        .send()
        .await
        .map_err(|e| format!("Failed to connect to Ollama at {}: {}", ollama_host, e))?;
    
    if !response.status().is_success() {
        return Err(format!("Ollama returned error: {}", response.status()));
    }
    
    #[derive(Deserialize)]
    struct OllamaTagsResponse {
        models: Vec<OllamaModel>,
    }
    
    #[derive(Deserialize)]
    struct OllamaModel {
        name: String,
        size: u64,
    }
    
    let tags: OllamaTagsResponse = response.json().await
        .map_err(|e| format!("Failed to parse Ollama response: {}", e))?;
    
    // Convert to our info struct, marking known embedding models
    let embedding_models = ["nomic-embed-text", "mxbai-embed-large", "all-minilm", 
                           "snowflake-arctic-embed", "bge-m3", "bge-large"];
    
    let models: Vec<OllamaModelInfo> = tags.models
        .into_iter()
        .map(|m| {
            let base_name = m.name.split(':').next().unwrap_or(&m.name);
            let is_embedding = embedding_models.iter().any(|e| base_name.contains(e));
            OllamaModelInfo {
                name: m.name,
                size_mb: m.size / 1_000_000,
                is_embedding_model: is_embedding,
            }
        })
        .collect();
    
    Ok(models)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaModelInfo {
    pub name: String,
    pub size_mb: u64,
    pub is_embedding_model: bool,
}

/// Fetch available models from Anthropic
#[tauri::command]
async fn get_anthropic_models(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<ModelInfo>, String> {
    let state_guard = state.read().await;
    let api_key = state_guard.config.llm.api_key.clone()
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
        .ok_or("No Anthropic API key configured")?;
    
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    
    let response = client
        .get("https://api.anthropic.com/v1/models")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch Anthropic models: {}", e))?;
    
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Anthropic API error {}: {}", status, body));
    }
    
    #[derive(Deserialize)]
    struct AnthropicModelsResponse {
        data: Vec<AnthropicModel>,
    }
    
    #[derive(Deserialize)]
    struct AnthropicModel {
        id: String,
        display_name: Option<String>,
    }
    
    let models: AnthropicModelsResponse = response.json().await
        .map_err(|e| format!("Failed to parse Anthropic response: {}", e))?;
    
    Ok(models.data.into_iter().map(|m| ModelInfo {
        id: m.id.clone(),
        name: m.display_name.unwrap_or(m.id),
    }).collect())
}

/// Fetch available models from OpenAI
#[tauri::command]
async fn get_openai_models() -> Result<Vec<ModelInfo>, String> {
    let api_key = std::env::var("OPENAI_API_KEY")
        .map_err(|_| "No OpenAI API key configured")?;
    
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    
    let response = client
        .get("https://api.openai.com/v1/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch OpenAI models: {}", e))?;
    
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("OpenAI API error {}: {}", status, body));
    }
    
    #[derive(Deserialize)]
    struct OpenAIModelsResponse {
        data: Vec<OpenAIModel>,
    }
    
    #[derive(Deserialize)]
    struct OpenAIModel {
        id: String,
    }
    
    let models: OpenAIModelsResponse = response.json().await
        .map_err(|e| format!("Failed to parse OpenAI response: {}", e))?;
    
    // Filter to chat models (gpt-*, o1-*, chatgpt-*)
    let chat_prefixes = ["gpt-4", "gpt-3.5", "o1", "o3", "chatgpt"];
    let embedding_prefixes = ["text-embedding"];
    
    let mut result: Vec<ModelInfo> = models.data.into_iter()
        .filter(|m| {
            chat_prefixes.iter().any(|p| m.id.starts_with(p)) ||
            embedding_prefixes.iter().any(|p| m.id.starts_with(p))
        })
        .map(|m| ModelInfo {
            id: m.id.clone(),
            name: m.id,
        })
        .collect();
    
    // Sort by name
    result.sort_by(|a, b| a.id.cmp(&b.id));
    
    Ok(result)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
}

// =============================================================================
// Cognitive Memory Commands (FSRS-6 + Dreaming)
// =============================================================================

/// Cognitive memory statistics
#[derive(Debug, Clone, Serialize)]
pub struct CognitiveMemoryStats {
    pub total_memories: usize,
    pub active: usize,
    pub dormant: usize,
    pub silent: usize,
    pub unavailable: usize,
    pub consolidation_enabled: bool,
    pub last_consolidation: Option<String>,
}

/// Get cognitive memory statistics
#[tauri::command]
async fn get_cognitive_memory_stats(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<CognitiveMemoryStats, String> {
    let state_guard = state.read().await;
    let stats = state_guard.memory.stats().await;
    let last = state_guard.last_consolidation.read().await;
    
    let last_consolidation = last.map(|ts| {
        chrono::DateTime::from_timestamp(ts, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| ts.to_string())
    });
    
    Ok(CognitiveMemoryStats {
        total_memories: stats.total,
        active: stats.active,
        dormant: stats.dormant,
        silent: stats.silent,
        unavailable: stats.unavailable,
        consolidation_enabled: true,
        last_consolidation,
    })
}

/// Consolidation result for frontend
#[derive(Debug, Clone, Serialize)]
pub struct ConsolidationResultInfo {
    pub memories_processed: usize,
    pub clusters_formed: usize,
    pub memories_merged: usize,
    pub memories_expanded: usize,
    pub errors: Vec<String>,
}

/// Manually trigger memory consolidation ("dream")
#[tauri::command]
async fn trigger_consolidation(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<ConsolidationResultInfo, String> {
    let state_guard = state.read().await;
    let llm = state_guard.llm.clone();
    let memory = state_guard.memory.clone();
    let last_consolidation = state_guard.last_consolidation.clone();
    drop(state_guard); // Release the lock before async work

    let config = ConsolidationConfig::default();
    
    // Create summarization callback using the LLM
    let summarize = |prompt: String| {
        let llm = llm.clone();
        async move {
            let request = nanna_llm::CompletionRequest::default()
                .with_model("claude-3-5-haiku-20241022") // Use fast model for summarization
                .with_message(nanna_llm::Message::user(&prompt));
            
            llm.complete(&request)
                .await
                .map_err(|e| e.to_string())
        }
    };

    info!("Starting manual memory consolidation...");
    
    let result = memory.consolidate(&config, summarize)
        .await
        .map_err(|e| format!("Consolidation failed: {}", e))?;

    // Update last consolidation timestamp
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    *last_consolidation.write().await = Some(now);

    info!(
        "Consolidation complete: {} processed, {} merged, {} errors",
        result.memories_processed, result.memories_merged, result.errors.len()
    );

    Ok(ConsolidationResultInfo {
        memories_processed: result.memories_processed,
        clusters_formed: result.clusters_formed,
        memories_merged: result.memories_merged,
        memories_expanded: result.memories_expanded,
        errors: result.errors,
    })
}

/// Apply pending FSRS updates (testing effect)
#[tauri::command]
async fn apply_memory_updates(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    let state_guard = state.read().await;
    state_guard.memory.apply_pending_updates().await;
    Ok(())
}

/// Manually save memories to disk
#[tauri::command]
async fn save_memories(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    let state_guard = state.read().await;
    state_guard.memory.save(&state_guard.memory_path).await
        .map_err(|e| format!("Failed to save memories: {}", e))?;
    info!("Manually saved memories to {:?}", state_guard.memory_path);
    Ok(())
}

// =============================================================================
// Memory Management Commands
// =============================================================================

/// Memory entry for frontend display
#[derive(Debug, Clone, Serialize)]
pub struct MemoryItem {
    pub id: String,
    pub content: String,
    pub fact_type: String,      // "stated" or "observed"
    pub importance: f32,
    pub state: String,          // "active", "dormant", "silent", "unavailable"
    pub weight: f32,
    pub retrievability: f32,
    pub access_count: u32,
    pub created_at: String,
    pub session_id: Option<String>,
}

/// List all semantic memories
#[tauri::command]
async fn list_memories(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<MemoryItem>, String> {
    let state_guard = state.read().await;
    let entries = state_guard.memory.list_all().await;
    
    Ok(entries.into_iter().map(|e| {
        let fact_type = e.metadata.get("fact_type")
            .cloned()
            .unwrap_or_else(|| "stated".to_string());
        let session_id = e.metadata.get("session_id").cloned();
        let created_at = chrono::DateTime::from_timestamp(e.timestamp, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| e.timestamp.to_string());
        
        MemoryItem {
            id: e.id,
            content: e.content,
            fact_type,
            importance: e.importance,
            state: format!("{:?}", e.state).to_lowercase(),
            weight: e.weight,
            retrievability: e.retrievability,
            access_count: e.access_count,
            created_at,
            session_id,
        }
    }).collect())
}

/// Get a single memory by ID
#[tauri::command]
async fn get_memory(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: String,
) -> Result<Option<MemoryItem>, String> {
    let state_guard = state.read().await;
    
    Ok(state_guard.memory.get(&id).await.map(|e| {
        let fact_type = e.metadata.get("fact_type")
            .cloned()
            .unwrap_or_else(|| "stated".to_string());
        let session_id = e.metadata.get("session_id").cloned();
        let created_at = chrono::DateTime::from_timestamp(e.timestamp, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| e.timestamp.to_string());
        
        MemoryItem {
            id: e.id,
            content: e.content,
            fact_type,
            importance: e.importance,
            state: format!("{:?}", e.state).to_lowercase(),
            weight: e.weight,
            retrievability: e.retrievability,
            access_count: e.access_count,
            created_at,
            session_id,
        }
    }))
}

/// Delete a memory by ID
#[tauri::command]
async fn delete_memory(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: String,
) -> Result<(), String> {
    let state_guard = state.read().await;
    state_guard.memory.forget(&id).await
        .map_err(|e| format!("Failed to delete memory: {}", e))?;
    
    // Auto-save after deletion
    state_guard.memory.save(&state_guard.memory_path).await
        .map_err(|e| format!("Failed to save after deletion: {}", e))?;
    
    info!("Deleted memory: {}", id);
    Ok(())
}

/// Update a memory's content
#[tauri::command]
async fn update_memory(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: String,
    content: String,
) -> Result<(), String> {
    let state_guard = state.read().await;
    state_guard.memory.update_content(&id, &content).await
        .map_err(|e| format!("Failed to update memory: {}", e))?;
    
    // Auto-save after update
    state_guard.memory.save(&state_guard.memory_path).await
        .map_err(|e| format!("Failed to save after update: {}", e))?;
    
    info!("Updated memory: {}", id);
    Ok(())
}

/// Clear all memories
#[tauri::command]
async fn clear_all_memories(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    let state_guard = state.read().await;
    state_guard.memory.clear().await;
    
    // Save empty state
    state_guard.memory.save(&state_guard.memory_path).await
        .map_err(|e| format!("Failed to save after clear: {}", e))?;
    
    info!("Cleared all memories");
    Ok(())
}

// =============================================================================
// Notification Commands
// =============================================================================

/// Send a native notification
#[tauri::command]
async fn send_notification(
    app: AppHandle,
    title: String,
    body: String,
) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    
    app.notification()
        .builder()
        .title(&title)
        .body(&body)
        .show()
        .map_err(|e| format!("Failed to send notification: {}", e))?;
    
    info!("Sent notification: {} - {}", title, body);
    Ok(())
}

/// Request notification permission (needed on some platforms)
#[tauri::command]
async fn request_notification_permission(app: AppHandle) -> Result<bool, String> {
    use tauri_plugin_notification::NotificationExt;
    
    let permission = app.notification()
        .request_permission()
        .map_err(|e| format!("Failed to request permission: {}", e))?;
    
    Ok(matches!(permission, tauri_plugin_notification::PermissionState::Granted))
}

/// Check if notifications are permitted
#[tauri::command]
async fn check_notification_permission(app: AppHandle) -> Result<String, String> {
    use tauri_plugin_notification::NotificationExt;
    
    let permission = app.notification()
        .permission_state()
        .map_err(|e| format!("Failed to check permission: {}", e))?;
    
    Ok(match permission {
        tauri_plugin_notification::PermissionState::Granted => "granted",
        tauri_plugin_notification::PermissionState::Denied => "denied",
        _ => "unknown",
    }.to_string())
}

// =============================================================================
// Similarity Threshold Configuration
// =============================================================================

/// Get the current similarity threshold
#[tauri::command]
async fn get_similarity_threshold(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<f32, String> {
    let state_guard = state.read().await;
    Ok(state_guard.memory.get_min_score())
}

/// Set the similarity threshold for memory recall
#[tauri::command]
async fn set_similarity_threshold(
    state: State<'_, Arc<RwLock<AppState>>>,
    threshold: f32,
) -> Result<String, String> {
    if !(0.0..=1.0).contains(&threshold) {
        return Err("Threshold must be between 0.0 and 1.0".to_string());
    }
    
    let state_guard = state.read().await;
    state_guard.memory.set_min_score(threshold);
    
    info!("Set similarity threshold to {}", threshold);
    Ok(format!("Similarity threshold set to {:.2}", threshold))
}

// =============================================================================
// Config Persistence Commands
// =============================================================================

/// Save config to disk
#[tauri::command]
async fn save_config(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    let state_guard = state.read().await;
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;
    
    info!("Config saved to disk");
    Ok(())
}

// =============================================================================
// Channel Status Commands
// =============================================================================

/// Channel status for frontend display
#[derive(Debug, Clone, Serialize)]
pub struct ChannelStatus {
    pub name: String,
    pub configured: bool,
    pub enabled: bool,
    pub status: String, // "ready", "not_configured", "disabled"
    pub details: Option<String>,
}

/// Get status of all configured channels
#[tauri::command]
async fn get_channel_status(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<ChannelStatus>, String> {
    let state_guard = state.read().await;
    let config = &state_guard.config;
    
    let mut channels = Vec::new();
    
    // Telegram
    channels.push(ChannelStatus {
        name: "Telegram".to_string(),
        configured: config.channels.telegram.is_some(),
        enabled: config.channels.telegram.is_some(),
        status: if config.channels.telegram.is_some() { "ready" } else { "not_configured" }.to_string(),
        details: config.channels.telegram.as_ref().map(|t| {
            let token_preview = if t.bot_token.len() > 10 {
                format!("{}...{}", &t.bot_token[..5], &t.bot_token[t.bot_token.len()-4..])
            } else {
                "***".to_string()
            };
            format!("Bot token: {}", token_preview)
        }),
    });
    
    // Discord
    channels.push(ChannelStatus {
        name: "Discord".to_string(),
        configured: config.channels.discord.is_some(),
        enabled: config.channels.discord.is_some(),
        status: if config.channels.discord.is_some() { "ready" } else { "not_configured" }.to_string(),
        details: config.channels.discord.as_ref().map(|d| {
            format!("App ID: {}", d.application_id)
        }),
    });
    
    // Slack
    channels.push(ChannelStatus {
        name: "Slack".to_string(),
        configured: config.channels.slack.is_some(),
        enabled: config.channels.slack.is_some(),
        status: if config.channels.slack.is_some() { "ready" } else { "not_configured" }.to_string(),
        details: config.channels.slack.as_ref().map(|s| {
            let has_app_token = s.app_token.is_some();
            format!("Socket mode: {}", if has_app_token { "enabled" } else { "disabled" })
        }),
    });
    
    // Signal
    channels.push(ChannelStatus {
        name: "Signal".to_string(),
        configured: config.channels.signal.is_some(),
        enabled: config.channels.signal.is_some(),
        status: if config.channels.signal.is_some() { "ready" } else { "not_configured" }.to_string(),
        details: config.channels.signal.as_ref().map(|s| {
            format!("Phone: {}", s.phone_number)
        }),
    });
    
    // WhatsApp (check env var or config)
    let whatsapp_configured = std::env::var("WHATSAPP_ACCESS_TOKEN").is_ok();
    channels.push(ChannelStatus {
        name: "WhatsApp".to_string(),
        configured: whatsapp_configured,
        enabled: whatsapp_configured,
        status: if whatsapp_configured { "ready" } else { "not_configured" }.to_string(),
        details: if whatsapp_configured { Some("Cloud API".to_string()) } else { None },
    });
    
    Ok(channels)
}

// =============================================================================
// App Setup
// =============================================================================

async fn setup_state() -> Result<AppState, Box<dyn std::error::Error + Send + Sync>> {
    // Load config
    let config = Config::load().unwrap_or_default().with_env_overrides();

    // Determine database path
    let db_path = Config::default_data_dir()
        .map(|d| d.join("nanna.db").to_string_lossy().to_string())
        .unwrap_or_else(|_| "nanna.db".to_string());

    // Initialize storage
    let storage_config = StorageConfig { path: db_path };
    let storage = Storage::new(&storage_config).await?;

    // Initialize LLM client
    let api_key = config
        .llm
        .api_key
        .clone()
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
        .unwrap_or_else(|| "missing-key".to_string());

    let llm = match config.llm.provider.as_str() {
        "openai" => LlmClient::openai(&api_key),
        _ => LlmClient::anthropic(&api_key),
    };
    let llm = Arc::new(llm);

    // Initialize tools
    let tools = ToolRegistry::new();

    // Register built-in tools
    tools.register(nanna_tools::ReadFileTool::new()).await;
    tools.register(nanna_tools::WriteFileTool::new()).await;
    tools.register(nanna_tools::ListDirTool::new()).await;
    tools.register(nanna_tools::ExecTool::new()).await;
    tools.register(nanna_tools::WebFetchTool::new()).await;
    
    // WebSearchTool requires BRAVE_API_KEY (env var or config)
    let brave_key = std::env::var("BRAVE_API_KEY").ok()
        .or_else(|| config.tools.brave_api_key.clone());
    let web_search = if let Some(key) = brave_key {
        // Set env var so it's available for later checks
        unsafe { std::env::set_var("BRAVE_API_KEY", &key); }
        nanna_tools::WebSearchTool::new().with_api_key(key)
    } else {
        info!("BRAVE_API_KEY not set - web_search will be unavailable, use web_fetch instead");
        nanna_tools::WebSearchTool::new()
    };
    tools.register(web_search).await;
    tools.register(nanna_tools::EchoTool).await;

    // Initialize FSRS-6 cognitive memory service
    // Load embedding config from saved config file (config takes priority over env var)
    let saved_embedding_provider = config.memory.embedding_provider.clone();
    let saved_embedding_model = config.memory.embedding_model.clone();
    let saved_ollama_host = config.memory.ollama_host.clone();
    
    // Get API keys
    let openai_key = std::env::var("OPENAI_API_KEY").ok()
        .or_else(|| config.llm.openai_api_key.clone());
    
    info!("Loaded embedding config: provider={}, model={}, ollama_host={}", 
          saved_embedding_provider, saved_embedding_model, saved_ollama_host);
    
    // Initialize based on configured provider
    let (embedding_provider, embedding_model, embedding_enabled, memory) = 
        match saved_embedding_provider.as_str() {
            "openai" => {
                if let Some(openai_key) = openai_key {
                    unsafe { std::env::set_var("OPENAI_API_KEY", &openai_key); }
                    info!("Using OpenAI embeddings with model: {}", saved_embedding_model);
                    
                    // Determine dimension based on model
                    let dimension = if saved_embedding_model.contains("large") { 3072 } else { 1536 };
                    let memory_config = MemoryServiceConfig {
                        dimension,
                        ..Default::default()
                    };
                    
                    let embed_client = reqwest::Client::new();
                    let embed_key = openai_key.clone();
                    let model_name = saved_embedding_model.clone();
                    
                    let embed_fn: nanna_memory::EmbedFn = Arc::new(move |text: &str| {
                        let client = embed_client.clone();
                        let key = embed_key.clone();
                        let model = model_name.clone();
                        let text = text.to_string();
                        
                        Box::pin(async move {
                            let response = client
                                .post("https://api.openai.com/v1/embeddings")
                                .header("Authorization", format!("Bearer {}", key))
                                .json(&serde_json::json!({
                                    "model": model,
                                    "input": text
                                }))
                                .send()
                                .await
                                .map_err(|e| e.to_string())?;
                            
                            let json: serde_json::Value = response
                                .json()
                                .await
                                .map_err(|e| e.to_string())?;
                            
                            let embedding = json["data"][0]["embedding"]
                                .as_array()
                                .ok_or("No embedding in response")?
                                .iter()
                                .filter_map(|v| v.as_f64().map(|f| f as f32))
                                .collect::<Vec<f32>>();
                            
                            if embedding.is_empty() {
                                return Err("Empty embedding returned".to_string());
                            }
                            
                            Ok(embedding)
                        })
                    });
                    
                    (
                        "openai".to_string(),
                        saved_embedding_model.clone(),
                        true,
                        MemoryService::new(memory_config).with_embed_fn(embed_fn),
                    )
                } else {
                    info!("OpenAI embeddings configured but no API key - disabling");
                    (
                        "disabled".to_string(),
                        "none".to_string(),
                        false,
                        MemoryService::new(MemoryServiceConfig::default()),
                    )
                }
            }
            "ollama" => {
                let ollama_url = saved_ollama_host.clone();
                info!("Using Ollama embeddings at {} with model: {}", ollama_url, saved_embedding_model);
                
                // Common embedding dimensions (default to 768 for nomic-embed-text)
                let dimension = match saved_embedding_model.as_str() {
                    m if m.contains("mxbai") => 1024,
                    m if m.contains("minilm") => 384,
                    m if m.contains("bge-large") => 1024,
                    m if m.contains("bge-m3") => 1024,
                    _ => 768, // nomic-embed-text default
                };
                
                let memory_config = MemoryServiceConfig {
                    dimension,
                    ..Default::default()
                };
                
                let embed_client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(60))
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new());
                
                let model_name = saved_embedding_model.clone();
                let embed_fn: nanna_memory::EmbedFn = Arc::new(move |text: &str| {
                    let client = embed_client.clone();
                    let url = ollama_url.clone();
                    let model = model_name.clone();
                    let text = text.to_string();
                    
                    Box::pin(async move {
                        let response = client
                            .post(format!("{}/api/embeddings", url))
                            .header("Content-Type", "application/json")
                            .json(&serde_json::json!({
                                "model": model,
                                "prompt": text
                            }))
                            .send()
                            .await
                            .map_err(|e| e.to_string())?;
                        
                        if !response.status().is_success() {
                            let status = response.status();
                            let body = response.text().await.unwrap_or_default();
                            return Err(format!("Ollama error {}: {}", status, body));
                        }
                        
                        let json: serde_json::Value = response
                            .json()
                            .await
                            .map_err(|e| e.to_string())?;
                        
                        let embedding = json["embedding"]
                            .as_array()
                            .ok_or("No embedding in Ollama response")?
                            .iter()
                            .filter_map(|v| v.as_f64().map(|f| f as f32))
                            .collect::<Vec<f32>>();
                        
                        if embedding.is_empty() {
                            return Err("Empty embedding returned from Ollama".to_string());
                        }
                        
                        Ok(embedding)
                    })
                });
                
                (
                    "ollama".to_string(),
                    saved_embedding_model.clone(),
                    true,
                    MemoryService::new(memory_config).with_embed_fn(embed_fn),
                )
            }
            _ => {
                info!("Embedding provider disabled");
                (
                    "disabled".to_string(),
                    "none".to_string(),
                    false,
                    MemoryService::new(MemoryServiceConfig::default()),
                )
            }
        };
    let memory = Arc::new(memory);

    // Load persisted memories if they exist
    let memory_path = Config::default_data_dir()
        .map(|d| d.join("memories.json"))
        .unwrap_or_else(|_| std::path::PathBuf::from("memories.json"));
    
    if memory_path.exists() {
        match memory.load(&memory_path).await {
            Ok(()) => info!("Loaded {} memories from {:?}", memory.count().await, memory_path),
            Err(e) => warn!("Failed to load memories (starting fresh): {}", e),
        }
    } else {
        info!("No saved memories found at {:?} (starting fresh)", memory_path);
    }

    // Initialize scheduler with consolidation task
    let scheduler_config = SchedulerConfig {
        heartbeat_interval: Duration::from_secs(300), // 5 minutes
        heartbeat_enabled: false, // Heartbeats disabled for GUI
        max_concurrent: 4,
    };
    let mut scheduler = Scheduler::new(scheduler_config);
    
    // Add memory consolidation task (runs every hour)
    let consolidation = consolidation_task(Some(Duration::from_secs(3600)));
    scheduler.add_task(consolidation).await;
    info!("Scheduled memory consolidation task (every 1 hour)");

    // Create executor for scheduled tasks
    let memory_for_executor = memory.clone();
    let llm_for_executor = llm.clone();
    
    let executor: nanna_core::TaskExecutor = Arc::new(move |task| {
        let memory = memory_for_executor.clone();
        let llm = llm_for_executor.clone();
        
        Box::pin(async move {
            let start = std::time::Instant::now();
            
            match task.name.as_str() {
                "memory_consolidation" => {
                    info!("Running scheduled memory consolidation...");
                    
                    let config = ConsolidationConfig::default();
                    let summarize = |prompt: String| {
                        let llm = llm.clone();
                        async move {
                            let request = nanna_llm::CompletionRequest::default()
                                .with_model("claude-3-5-haiku-20241022")
                                .with_message(nanna_llm::Message::user(&prompt));
                            llm.complete(&request).await.map_err(|e| e.to_string())
                        }
                    };
                    
                    match memory.consolidate(&config, summarize).await {
                        Ok(result) => {
                            info!(
                                "Scheduled consolidation: {} processed, {} merged",
                                result.memories_processed, result.memories_merged
                            );
                            nanna_core::TaskResult {
                                task_id: task.id,
                                success: true,
                                output: Some(format!("Processed {} memories", result.memories_processed)),
                                error: None,
                                duration_ms: start.elapsed().as_millis() as u64,
                            }
                        }
                        Err(e) => {
                            error!("Scheduled consolidation failed: {}", e);
                            nanna_core::TaskResult {
                                task_id: task.id,
                                success: false,
                                output: None,
                                error: Some(e.to_string()),
                                duration_ms: start.elapsed().as_millis() as u64,
                            }
                        }
                    }
                }
                _ => {
                    debug!("Unknown task: {}", task.name);
                    nanna_core::TaskResult {
                        task_id: task.id,
                        success: true,
                        output: Some("Skipped unknown task".to_string()),
                        error: None,
                        duration_ms: start.elapsed().as_millis() as u64,
                    }
                }
            }
        })
    });
    
    scheduler = scheduler.with_executor(executor);
    scheduler.start();
    info!("Scheduler started with consolidation executor");
    
    let scheduler = Arc::new(RwLock::new(scheduler));
    let last_consolidation = Arc::new(RwLock::new(None));

    info!("Nanna GUI initialized with model: {}", config.llm.model);
    info!("Registered {} tools", tools.definitions().await.len());
    info!("FSRS-6 cognitive memory enabled");

    // Get extraction model from config (empty = use chat model)
    let saved_extraction_model = config.memory.extraction_model.clone();

    Ok(AppState {
        storage: Arc::new(storage),
        llm,
        tools: Arc::new(tools),
        config,
        memory,
        memory_path,
        scheduler,
        last_consolidation,
        // Runtime settings - all enabled by default
        dreaming_enabled: Arc::new(RwLock::new(true)),
        scheduler_enabled: Arc::new(RwLock::new(true)),
        heartbeat_enabled: Arc::new(RwLock::new(true)),
        heartbeat_interval_seconds: Arc::new(RwLock::new(300)), // 5 minutes
        // Embedding settings (loaded from config)
        embedding_provider: Arc::new(RwLock::new(embedding_provider)),
        embedding_model: Arc::new(RwLock::new(embedding_model)),
        embedding_enabled: Arc::new(RwLock::new(embedding_enabled)),
        // Ollama host (from config)
        ollama_host: Arc::new(RwLock::new(saved_ollama_host)),
        // Extraction model (from config, empty = use chat model)
        extraction_model: Arc::new(RwLock::new(saved_extraction_model)),
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("nanna=info".parse().unwrap()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let handle = app.handle().clone();

            // Set up system tray
            setup_system_tray(app)?;

            // Initialize state asynchronously
            tauri::async_runtime::spawn(async move {
                match setup_state().await {
                    Ok(state) => {
                        handle.manage(Arc::new(RwLock::new(state)));
                        info!("App state initialized successfully");
                    }
                    Err(e) => {
                        error!("Failed to initialize app state: {}", e);
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            send_message,
            create_session,
            list_sessions,
            get_session_history,
            delete_session,
            rename_session,
            get_config,
            set_model,
            set_api_key,
            search_memory,
            get_memory_stats,
            show_window,
            hide_to_tray,
            get_extended_settings,
            set_provider_api_key,
            set_provider,
            check_env_var,
            // Cognitive memory (FSRS-6 + dreaming)
            get_cognitive_memory_stats,
            trigger_consolidation,
            apply_memory_updates,
            // Memory & scheduling settings
            set_dreaming_enabled,
            set_scheduler_enabled,
            set_heartbeat_enabled,
            set_heartbeat_interval,
            set_extraction_model,
            // Embedding configuration
            set_embedding_config,
            get_ollama_models,
            set_ollama_host,
            // Dynamic model fetching
            get_anthropic_models,
            get_openai_models,
            // Memory persistence
            save_memories,
            // Memory management
            list_memories,
            get_memory,
            delete_memory,
            update_memory,
            clear_all_memories,
            // Channel status
            get_channel_status,
            // Config persistence
            save_config,
            // Notifications
            send_notification,
            request_notification_permission,
            check_notification_permission,
            // Similarity threshold
            get_similarity_threshold,
            set_similarity_threshold,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                // Save memories before exiting
                if let Some(state) = app.try_state::<Arc<RwLock<AppState>>>() {
                    let state = state.inner().clone();
                    tauri::async_runtime::block_on(async {
                        let state_guard = state.read().await;
                        if let Err(e) = state_guard.memory.save(&state_guard.memory_path).await {
                            error!("Failed to save memories on exit: {}", e);
                        } else {
                            info!("Saved memories to {:?}", state_guard.memory_path);
                        }
                    });
                }
            }
        });
}

/// Set up the system tray icon and menu
fn setup_system_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show_item = MenuItemBuilder::with_id("show", "Show Nanna").build(app)?;
    let new_chat_item = MenuItemBuilder::with_id("new_chat", "New Chat").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&show_item)
        .item(&new_chat_item)
        .separator()
        .item(&quit_item)
        .build()?;

    let _tray = TrayIconBuilder::with_id("main")
        .tooltip("Nanna AI Assistant")
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| {
            match event.id().as_ref() {
                "show" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                "new_chat" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                        // Emit event to create new chat
                        let _ = app.emit("tray-new-chat", ());
                    }
                }
                "quit" => {
                    app.exit(0);
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;

    info!("System tray initialized");
    Ok(())
}
