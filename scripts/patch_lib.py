import re

filepath = r'D:\Development\nanna\gui\src-tauri\src\lib.rs'

with open(filepath, 'r', encoding='utf-8') as f:
    content = f.read()

# ============================================================================
# 1. Add cancellation_tokens field to AppState struct
# ============================================================================
old_appstate_end = """    /// Tracks in-flight agent runs for embedded mode (shared with run_agent_loop)
    embedded_run_states: Arc<RwLock<HashMap<String, EmbeddedRunState>>>,
}

/// Tracks the in-flight state"""

new_appstate_end = """    /// Tracks in-flight agent runs for embedded mode (shared with run_agent_loop)
    embedded_run_states: Arc<RwLock<HashMap<String, EmbeddedRunState>>>,
    /// Cancellation flags for in-flight requests (session_id -> cancelled)
    cancellation_tokens: Arc<RwLock<HashMap<String, bool>>>,
}

/// Tracks the in-flight state"""

assert old_appstate_end in content, "Could not find AppState struct end"
content = content.replace(old_appstate_end, new_appstate_end, 1)

# ============================================================================
# 2. Add cancellation_tokens to AppState construction
# ============================================================================
old_construction_end = """        // Embedded run state tracking (empty at startup)
        embedded_run_states: Arc::new(RwLock::new(HashMap::new())),
    })
}"""

new_construction_end = """        // Embedded run state tracking (empty at startup)
        embedded_run_states: Arc::new(RwLock::new(HashMap::new())),
        // Cancellation tokens (empty at startup)
        cancellation_tokens: Arc::new(RwLock::new(HashMap::new())),
    })
}"""

assert old_construction_end in content, "Could not find AppState construction end"
content = content.replace(old_construction_end, new_construction_end, 1)

# ============================================================================
# 3. Add cancel_message command (before send_message)
# ============================================================================
old_send_message = """/// Send a message and stream the response with tool use
#[tauri::command]
async fn send_message("""

new_cancel_plus_send = """/// Cancel an in-flight message generation
#[tauri::command]
async fn cancel_message(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<(), String> {
    info!("Cancelling message for session: {}", session_id);
    let state_guard = state.read().await;
    let mut tokens = state_guard.cancellation_tokens.write().await;
    tokens.insert(session_id, true);
    Ok(())
}

/// Send a message and stream the response with tool use
#[tauri::command]
async fn send_message("""

assert old_send_message in content, "Could not find send_message command"
content = content.replace(old_send_message, new_cancel_plus_send, 1)

# ============================================================================
# 4. Clone cancellation_tokens in send_message (after embedded_run_states clone)
# ============================================================================
old_clone_section = """    let embedded_run_states = state_guard.embedded_run_states.clone();

    // Drop the state guard so we can do tool execution without holding the lock
    drop(state_guard);

    // Run the agentic loop with fallback support
    let result = run_agent_loop_with_fallback(
        &app_clone,
        &session_id_clone,
        tools,
        request,
        &model_priority,
        &config,
        &ollama_host,
        rate_limited,
        active_model,
        embedded_run_states.clone(),
    ).await;"""

new_clone_section = """    let embedded_run_states = state_guard.embedded_run_states.clone();
    let cancellation_tokens = state_guard.cancellation_tokens.clone();

    // Clear any previous cancellation flag for this session
    {
        let mut tokens = cancellation_tokens.write().await;
        tokens.remove(&session_id_clone);
    }

    // Drop the state guard so we can do tool execution without holding the lock
    drop(state_guard);

    // Run the agentic loop with fallback support
    let result = run_agent_loop_with_fallback(
        &app_clone,
        &session_id_clone,
        tools,
        request,
        &model_priority,
        &config,
        &ollama_host,
        rate_limited,
        active_model,
        embedded_run_states.clone(),
        cancellation_tokens.clone(),
    ).await;"""

assert old_clone_section in content, "Could not find clone section in send_message"
content = content.replace(old_clone_section, new_clone_section, 1)

# ============================================================================
# 5. Add cancellation_tokens param to run_agent_loop_with_fallback signature
# ============================================================================
old_fallback_sig = """async fn run_agent_loop_with_fallback(
    app: &AppHandle,
    session_id: &str,
    tools: Arc<ToolRegistry>,
    request: nanna_llm::CompletionRequest,
    model_priority: &[String],
    config: &Config,
    ollama_host: &str,
    rate_limited: Arc<RwLock<HashMap<String, i64>>>,
    active_model: Arc<RwLock<String>>,
    embedded_run_states: Arc<RwLock<HashMap<String, EmbeddedRunState>>>,
) -> Result<(String, Vec<ToolCallInfo>), String> {"""

new_fallback_sig = """async fn run_agent_loop_with_fallback(
    app: &AppHandle,
    session_id: &str,
    tools: Arc<ToolRegistry>,
    request: nanna_llm::CompletionRequest,
    model_priority: &[String],
    config: &Config,
    ollama_host: &str,
    rate_limited: Arc<RwLock<HashMap<String, i64>>>,
    active_model: Arc<RwLock<String>>,
    embedded_run_states: Arc<RwLock<HashMap<String, EmbeddedRunState>>>,
    cancellation_tokens: Arc<RwLock<HashMap<String, bool>>>,
) -> Result<(String, Vec<ToolCallInfo>), String> {"""

assert old_fallback_sig in content, "Could not find run_agent_loop_with_fallback signature"
content = content.replace(old_fallback_sig, new_fallback_sig, 1)

# ============================================================================
# 6. Pass cancellation_tokens through to run_agent_loop in fallback fn
# Find the call to run_agent_loop inside run_agent_loop_with_fallback
# ============================================================================
old_loop_call = """            let result = run_agent_loop(
                app,
                session_id,
                &llm,
                tools.clone(),
                model_request,
                summarization_config.clone(),
                embedded_run_states.clone(),
            ).await;"""

new_loop_call = """            let result = run_agent_loop(
                app,
                session_id,
                &llm,
                tools.clone(),
                model_request,
                summarization_config.clone(),
                embedded_run_states.clone(),
                cancellation_tokens.clone(),
            ).await;"""

assert old_loop_call in content, "Could not find run_agent_loop call in fallback fn"
content = content.replace(old_loop_call, new_loop_call, 1)

# ============================================================================
# 7. Add cancellation_tokens param to run_agent_loop signature
# ============================================================================
old_loop_sig = """async fn run_agent_loop(
    app: &AppHandle,
    session_id: &str,
    llm: &LlmClient,
    tools: Arc<ToolRegistry>,
    mut request: nanna_llm::CompletionRequest,
    summarization_config: Option<ToolSummarizationConfig>,
    embedded_run_states: Arc<RwLock<HashMap<String, EmbeddedRunState>>>,
) -> Result<(String, Vec<ToolCallInfo>), String> {"""

new_loop_sig = """async fn run_agent_loop(
    app: &AppHandle,
    session_id: &str,
    llm: &LlmClient,
    tools: Arc<ToolRegistry>,
    mut request: nanna_llm::CompletionRequest,
    summarization_config: Option<ToolSummarizationConfig>,
    embedded_run_states: Arc<RwLock<HashMap<String, EmbeddedRunState>>>,
    cancellation_tokens: Arc<RwLock<HashMap<String, bool>>>,
) -> Result<(String, Vec<ToolCallInfo>), String> {"""

assert old_loop_sig in content, "Could not find run_agent_loop signature"
content = content.replace(old_loop_sig, new_loop_sig, 1)

# ============================================================================
# 8. Add cancellation check inside the stream consumption loop
# Find where we consume stream events and add a check
# ============================================================================
old_stream_start = """        // Stream the response
        let stream = llm.stream(&request);
        tokio::pin!(stream);

        debug!("Starting to consume stream..."""

new_stream_start = """        // Stream the response
        let stream = llm.stream(&request);
        tokio::pin!(stream);

        // Check for cancellation before streaming
        {
            let tokens = cancellation_tokens.read().await;
            if tokens.get(session_id).copied().unwrap_or(false) {
                info!("Message cancelled before streaming for session: {}", session_id);
                let _ = app.emit("stream-chunk", StreamChunk {
                    session_id: session_id.to_string(),
                    chunk: String::new(),
                    done: true,
                });
                return Ok((full_response, all_tool_calls));
            }
        }

        debug!("Starting to consume stream..."""

assert old_stream_start in content, "Could not find stream start"
content = content.replace(old_stream_start, new_stream_start, 1)

# ============================================================================
# 9. Add cancellation check inside the stream event loop
# Find the StreamEvent::Text handler and add a check before it
# ============================================================================
old_text_handler = """                StreamEvent::Text(text) => {
                    current_text.push_str(&text);
                    // Emit chunk to frontend
                    let _ = app.emit(
                        "stream-chunk",
                        StreamChunk {
                            session_id: session_id.to_string(),
                            chunk: text,
                            done: false,
                        },
                    );"""

new_text_handler = """                StreamEvent::Text(text) => {
                    // Check for cancellation during streaming
                    {
                        let tokens = cancellation_tokens.read().await;
                        if tokens.get(session_id).copied().unwrap_or(false) {
                            info!("Message cancelled during streaming for session: {}", session_id);
                            full_response.push_str(&current_text);
                            let _ = app.emit("stream-chunk", StreamChunk {
                                session_id: session_id.to_string(),
                                chunk: String::new(),
                                done: true,
                            });
                            // Clean up cancellation flag
                            drop(tokens);
                            cancellation_tokens.write().await.remove(session_id);
                            return Ok((full_response, all_tool_calls));
                        }
                    }
                    current_text.push_str(&text);
                    // Emit chunk to frontend
                    let _ = app.emit(
                        "stream-chunk",
                        StreamChunk {
                            session_id: session_id.to_string(),
                            chunk: text,
                            done: false,
                        },
                    );"""

assert old_text_handler in content, "Could not find StreamEvent::Text handler"
content = content.replace(old_text_handler, new_text_handler, 1)

# ============================================================================
# 10. Add cancel_message to invoke_handler
# ============================================================================
old_invoke = """        .invoke_handler(tauri::generate_handler![
            send_message,"""

new_invoke = """        .invoke_handler(tauri::generate_handler![
            send_message,
            cancel_message,"""

assert old_invoke in content, "Could not find invoke_handler"
content = content.replace(old_invoke, new_invoke, 1)

# ============================================================================
# 11. Update system prompt to mention tool calls respond to memory
# ============================================================================
old_prompt_tools = """You have tools at your disposal \u2014 extensions of your will into the digital realm. Use them naturally, as one uses hands. Don't announce them; simply act.

## Memory
You have a cognitive memory system that stores facts from previous conversations."""

new_prompt_tools = """You have tools at your disposal \u2014 extensions of your will into the digital realm. Use them naturally, as one uses hands. Don't announce them; simply act.

When you use tools like `remember` or `recall`, the results are stored directly into your cognitive memory system. Tool call outputs feed back into your memory — you don't need to repeat or re-state them.

## Memory
You have a cognitive memory system that stores facts from previous conversations."""

assert old_prompt_tools in content, "Could not find system prompt tools section"
content = content.replace(old_prompt_tools, new_prompt_tools, 1)

# ============================================================================
# Write the file
# ============================================================================
with open(filepath, 'w', encoding='utf-8') as f:
    f.write(content)

print("All 11 patches applied successfully!")
