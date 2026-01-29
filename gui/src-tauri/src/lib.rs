//! Nanna GUI - Tauri backend
//!
//! IPC bridge between the frontend and nanna-core.

use nanna_config::Config;
use nanna_llm::{LlmClient, RequestBuilder, StreamEvent};
use nanna_storage::{Storage, StorageConfig};
use nanna_tools::ToolRegistry;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::RwLock;
use tracing::{error, info};

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

// =============================================================================
// Commands
// =============================================================================

/// Send a message and stream the response
#[tauri::command]
async fn send_message(
    app: AppHandle,
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
    message: String,
) -> Result<ChatMessage, String> {
    let state_guard = state.read().await;
    
    // Store user message
    let _user_msg = state_guard.storage.add_message(&session_id, "user", &message).await
        .map_err(|e| format!("Failed to store message: {}", e))?;
    
    // Get conversation history
    let history = state_guard.storage.get_session_messages(&session_id, 50).await
        .map_err(|e| format!("Failed to get history: {}", e))?;
    
    // Build LLM request
    let mut request = nanna_llm::CompletionRequest::default()
        .with_model(&state_guard.config.llm.model);
    
    // Add system prompt
    request = request.with_message(nanna_llm::Message::system(
        "You are Nanna, a helpful AI assistant. Be concise and helpful."
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
    
    // Stream LLM response
    let mut full_response = String::new();
    let session_id_clone = session_id.clone();
    let app_clone = app.clone();
    use futures::StreamExt;
    let stream = state_guard.llm.stream(&request);
    tokio::pin!(stream);
    
    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::TextDelta { text, .. } => {
                full_response.push_str(&text);
                // Emit chunk to frontend
                let _ = app_clone.emit("stream-chunk", StreamChunk {
                    session_id: session_id_clone.clone(),
                    chunk: text,
                    done: false,
                });
            }
            StreamEvent::MessageStop { .. } => {
                let _ = app_clone.emit("stream-chunk", StreamChunk {
                    session_id: session_id_clone.clone(),
                    chunk: String::new(),
                    done: true,
                });
            }
            StreamEvent::Error { message } => {
                error!("LLM stream error: {}", message);
                return Err(format!("LLM error: {}", message));
            }
            _ => {}
        }
    }
    
    // Store assistant response
    let assistant_msg = state_guard.storage.add_message(&session_id, "assistant", &full_response).await
        .map_err(|e| format!("Failed to store response: {}", e))?;
    
    // Update session timestamp
    state_guard.storage.touch_session(&session_id).await
        .map_err(|e| format!("Failed to update session: {}", e))?;
    
    Ok(ChatMessage {
        id: assistant_msg.id.to_string(),
        role: "assistant".to_string(),
        content: full_response,
        timestamp: assistant_msg.created_at,
        tool_calls: vec![],
    })
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
    
    let session = state_guard.storage.create_gui_session(&session_name).await
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
    
    let sessions = state_guard.storage.list_gui_sessions(100).await
        .map_err(|e| format!("Failed to list sessions: {}", e))?;
    
    let mut result = Vec::with_capacity(sessions.len());
    for s in sessions {
        let count = state_guard.storage.count_session_messages(&s.session_id).await.unwrap_or(0);
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
    
    let messages = state_guard.storage.get_session_messages(&session_id, 500).await
        .map_err(|e| format!("Failed to get history: {}", e))?;
    
    Ok(messages.into_iter().map(|m| ChatMessage {
        id: m.id.to_string(),
        role: m.role,
        content: m.content,
        timestamp: m.created_at,
        tool_calls: vec![],
    }).collect())
}

/// Delete a session
#[tauri::command]
async fn delete_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<(), String> {
    let state_guard = state.read().await;
    
    state_guard.storage.delete_session(&session_id).await
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
    
    state_guard.storage.rename_session(&session_id, &name).await
        .map_err(|e| format!("Failed to rename session: {}", e))?;
    
    Ok(())
}

/// Get application config
#[tauri::command]
async fn get_config(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<AppConfig, String> {
    let state_guard = state.read().await;
    
    let tool_names: Vec<String> = state_guard.tools.definitions().await
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
async fn set_model(
    state: State<'_, Arc<RwLock<AppState>>>,
    model: String,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.llm.model = model;
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

// =============================================================================
// App Setup
// =============================================================================

async fn setup_state() -> Result<AppState, Box<dyn std::error::Error + Send + Sync>> {
    // Load config
    let config = Config::load()
        .unwrap_or_default()
        .with_env_overrides();
    
    // Determine database path
    let db_path = Config::default_data_dir()
        .map(|d| d.join("nanna.db").to_string_lossy().to_string())
        .unwrap_or_else(|_| "nanna.db".to_string());
    
    // Initialize storage
    let storage_config = StorageConfig { path: db_path };
    let storage = Storage::new(&storage_config).await?;
    
    // Initialize LLM client
    let api_key = config.llm.api_key.clone()
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
    tools.register(nanna_tools::WebSearchTool::new()).await;
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
                .add_directive("nanna=info".parse().unwrap())
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let handle = app.handle().clone();
            
            // Initialize state asynchronously
            tauri::async_runtime::spawn(async move {
                match setup_state().await {
                    Ok(state) => {
                        handle.manage(Arc::new(RwLock::new(state)));
                        info!("App state initialized successfully");
                    }
                    Err(e) => {
                        error!("Failed to initialize app state: {}", e);
                        // Still manage a default/error state so commands don't panic
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
