//! Nanna GUI - Tauri backend
//!
//! IPC bridge between the frontend and nanna-core.

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

/// Chat message from frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// Agent response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCallInfo>,
}

/// Tool call info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallInfo {
    pub name: String,
    pub input: serde_json::Value,
    pub output: String,
}

/// Send a chat message and get response
#[tauri::command]
async fn send_message(
    _app: AppHandle,
    message: String,
    _session_id: Option<String>,
) -> Result<AgentResponse, String> {
    // TODO: Connect to nanna-agent
    // For now, echo back
    Ok(AgentResponse {
        text: format!("Echo: {}", message),
        tool_calls: vec![],
    })
}

/// Get available sessions
#[tauri::command]
async fn list_sessions() -> Result<Vec<String>, String> {
    // TODO: Connect to nanna-storage
    Ok(vec!["default".to_string()])
}

/// Get session history
#[tauri::command]
async fn get_session_history(
    _session_id: String,
) -> Result<Vec<ChatMessage>, String> {
    // TODO: Connect to nanna-storage
    Ok(vec![])
}

/// Application config
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub theme: String,
    pub model: String,
    pub api_key_set: bool,
}

/// Get app config
#[tauri::command]
async fn get_config() -> Result<AppConfig, String> {
    Ok(AppConfig {
        theme: "dark".to_string(),
        model: "claude-sonnet-4-20250514".to_string(),
        api_key_set: std::env::var("ANTHROPIC_API_KEY").is_ok(),
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            send_message,
            list_sessions,
            get_session_history,
            get_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
