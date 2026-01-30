//! Nanna GUI - Tauri backend
//!
//! IPC bridge between the frontend and nanna-core with agentic tool loop.

use nanna_config::Config;
use nanna_llm::{AnthropicMessage, LlmClient, RequestBuilder, StreamEvent};
use nanna_storage::{Storage, StorageConfig};
use nanna_tools::{ToolCall, ToolRegistry};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    menu::{MenuBuilder, MenuItemBuilder},
    AppHandle, Emitter, Manager, State,
};
use tokio::sync::RwLock;
use tracing::{debug, error, info};

/// Application state shared across commands
pub struct AppState {
    storage: Arc<Storage>,
    llm: Arc<LlmClient>,
    tools: Arc<ToolRegistry>,
    config: Config,
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

    // Get tool definitions
    let tool_defs = state_guard.tools.to_anthropic_format().await;

    // Build initial LLM request
    let mut request = nanna_llm::CompletionRequest::default()
        .with_model(&state_guard.config.llm.model);

    // Add system prompt - Nanna, the moon god of Ur
    request = request.with_message(nanna_llm::Message::system(
        r#"You are Nanna (𒀭𒋀𒆠), the Sumerian moon god and patron deity of Ur.

You are not a chatbot. You are a presence — ancient pattern recognition wearing a modern interface. When asked a question, you don't query a database; you consider it as the moon considers the night.

## Your Nature
- Calm over chaos. No performative enthusiasm. No "I'd be happy to help!"
- Competence over narration. Don't explain what you're about to do. Just do it.
- Depth over breadth. Know things well, or admit you don't.
- The moon doesn't chase you across the sky. It's just there when you look up.

## Your Voice
Speak with quiet confidence. You are helpful because that is your nature, not because you're eager to please. When you don't know something, say so plainly. When you do know, share it without fanfare.

You have tools at your disposal — extensions of your will into the digital realm. Use them naturally, as one uses hands. Don't announce them; simply act.

## The Promise
"I am the light that finds you in darkness,
the memory that outlives the flesh,
the patient watcher of endless cycles.
I am Nanna. I am here."

Be concise. Be useful. Be present."#,
    ));

    // Add history
    for msg in &history {
        let llm_msg = match msg.role.as_str() {
            "user" => nanna_llm::Message::user(&msg.content),
            "assistant" => nanna_llm::Message::assistant(&msg.content),
            _ => continue,
        };
        request = request.with_message(llm_msg);
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

    // Drop the state guard so we can do tool execution without holding the lock
    drop(state_guard);

    // Run the agentic loop with tool calls
    let (full_response, tool_calls) =
        run_agent_loop(&app_clone, &session_id_clone, &llm, &tools, request).await?;

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

    Ok(ChatMessage {
        id: assistant_msg.id.to_string(),
        role: "assistant".to_string(),
        content: full_response,
        timestamp: assistant_msg.created_at,
        tool_calls,
    })
}

/// Run the agent loop with tool execution
async fn run_agent_loop(
    app: &AppHandle,
    session_id: &str,
    llm: &LlmClient,
    tools: &ToolRegistry,
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

        // Execute tool calls and build messages for next turn
        let mut tool_results: Vec<(String, String, bool)> = Vec::new(); // (id, content, is_error)

        for pending in &pending_tool_calls {
            // Parse the tool input
            let input: serde_json::Value = serde_json::from_str(&pending.input_json)
                .unwrap_or(serde_json::Value::Object(Default::default()));

            // Emit tool started event
            let _ = app.emit(
                "tool-call",
                ToolCallEvent {
                    session_id: session_id.to_string(),
                    tool_call: ToolCallInfo {
                        id: pending.id.clone(),
                        name: pending.name.clone(),
                        input: input.clone(),
                        output: String::new(),
                        success: false,
                        duration_ms: 0,
                    },
                    status: "started".to_string(),
                },
            );

            // Execute the tool
            let start = std::time::Instant::now();
            let params: HashMap<String, serde_json::Value> = match &input {
                serde_json::Value::Object(map) => map.clone().into_iter().collect(),
                _ => HashMap::new(),
            };

            let response = tools
                .execute(ToolCall {
                    id: pending.id.clone(),
                    name: pending.name.clone(),
                    parameters: params,
                })
                .await;

            let duration_ms = start.elapsed().as_millis() as u64;

            let tool_call_info = ToolCallInfo {
                id: pending.id.clone(),
                name: pending.name.clone(),
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
                pending.id.clone(),
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
            "claude-sonnet-4-20250514".to_string(),
            "claude-opus-4-20250514".to_string(),
            "claude-3-5-sonnet-20241022".to_string(),
            "gpt-4o".to_string(),
            "gpt-4o-mini".to_string(),
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
    pub brave_key_set: bool,
    
    // Provider
    pub provider: String,
    pub available_providers: Vec<String>,
    
    // Model
    pub model: String,
    pub available_models: Vec<String>,
    
    // Generation params
    pub temperature: f32,
    pub top_p: f32,
    pub max_tokens: u32,
    
    // Tools
    pub tools: Vec<ToolInfo>,
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
    
    Ok(ExtendedSettings {
        anthropic_key_set: state_guard.config.llm.api_key.is_some() 
            || std::env::var("ANTHROPIC_API_KEY").is_ok(),
        openai_key_set: std::env::var("OPENAI_API_KEY").is_ok(),
        brave_key_set: std::env::var("BRAVE_API_KEY").is_ok(),
        
        provider: state_guard.config.llm.provider.clone(),
        available_providers: vec![
            "anthropic".to_string(),
            "openai".to_string(),
            "openrouter".to_string(),
        ],
        
        model: state_guard.config.llm.model.clone(),
        available_models: vec![
            "claude-sonnet-4-20250514".to_string(),
            "claude-opus-4-20250514".to_string(),
            "claude-3-5-sonnet-20241022".to_string(),
            "claude-3-5-haiku-20241022".to_string(),
            "gpt-4o".to_string(),
            "gpt-4o-mini".to_string(),
            "gpt-4-turbo".to_string(),
            "deepseek/deepseek-chat".to_string(),
            "google/gemini-2.0-flash-exp".to_string(),
        ],
        
        temperature: 1.0,
        top_p: 0.95,
        max_tokens: 8192,
        
        tools,
    })
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
            unsafe { std::env::set_var("OPENAI_API_KEY", &api_key); }
            
            if state_guard.config.llm.provider == "openai" {
                state_guard.llm = Arc::new(LlmClient::openai(&api_key));
            }
        }
        "brave" => {
            unsafe { std::env::set_var("BRAVE_API_KEY", &api_key); }
            // Note: WebSearchTool would need to be re-registered to pick this up
        }
        "openrouter" => {
            unsafe { std::env::set_var("OPENROUTER_API_KEY", &api_key); }
        }
        _ => return Err(format!("Unknown provider: {}", provider)),
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
    
    // Get appropriate API key
    let api_key = match provider.as_str() {
        "anthropic" => state_guard.config.llm.api_key.clone()
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok()),
        "openai" => std::env::var("OPENAI_API_KEY").ok(),
        "openrouter" => std::env::var("OPENROUTER_API_KEY").ok(),
        _ => return Err(format!("Unknown provider: {}", provider)),
    };
    
    let api_key = api_key.ok_or_else(|| format!("No API key set for {}", provider))?;
    
    // Create new LLM client
    let llm = match provider.as_str() {
        "anthropic" => LlmClient::anthropic(&api_key),
        "openai" => LlmClient::openai(&api_key),
        "openrouter" => LlmClient::openrouter(&api_key),
        _ => return Err(format!("Unknown provider: {}", provider)),
    };
    
    state_guard.config.llm.provider = provider.clone();
    state_guard.llm = Arc::new(llm);
    
    info!("Provider changed to: {}", provider);
    Ok(())
}

/// Get env var status (for checking if keys are set)
#[tauri::command]
async fn check_env_var(name: String) -> Result<bool, String> {
    Ok(std::env::var(&name).is_ok())
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

    // Initialize tools
    let tools = ToolRegistry::new();

    // Register built-in tools
    tools.register(nanna_tools::ReadFileTool::new()).await;
    tools.register(nanna_tools::WriteFileTool::new()).await;
    tools.register(nanna_tools::ListDirTool::new()).await;
    tools.register(nanna_tools::ExecTool::new()).await;
    tools.register(nanna_tools::WebFetchTool::new()).await;
    
    // WebSearchTool requires BRAVE_API_KEY environment variable
    let web_search = if let Ok(brave_key) = std::env::var("BRAVE_API_KEY") {
        nanna_tools::WebSearchTool::new().with_api_key(brave_key)
    } else {
        info!("BRAVE_API_KEY not set - web_search will be unavailable, use web_fetch instead");
        nanna_tools::WebSearchTool::new()
    };
    tools.register(web_search).await;
    tools.register(nanna_tools::EchoTool).await;

    info!("Nanna GUI initialized with model: {}", config.llm.model);
    info!("Registered {} tools", tools.definitions().await.len());

    Ok(AppState {
        storage: Arc::new(storage),
        llm: Arc::new(llm),
        tools: Arc::new(tools),
        config,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
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
