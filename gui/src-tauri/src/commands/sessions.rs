//! Session management commands. The daemon owns nanna.db; these forward to it.

#[allow(clippy::wildcard_imports)]
use crate::*;

/// Create a new session.
#[tauri::command]
pub async fn create_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: Option<String>,
    workspace_id: Option<String>,
) -> Result<SessionInfo, String> {
    let state_guard = state.read().await;

    let session_name = name.unwrap_or_else(|| {
        format!("Chat {}", chrono::Utc::now().format("%Y-%m-%d %H:%M"))
    });

    let result = state_guard
        .backend
        .session_create_in_workspace(Some(&session_name), workspace_id.as_deref())
        .await?;
    let session = result
        .get("session")
        .ok_or("Invalid daemon response: missing 'session' field")?;

    Ok(SessionInfo {
        id: session
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or("Invalid daemon response: missing 'id'")?
            .to_string(),
        name: session.get("name").and_then(|v| v.as_str()).unwrap_or(&session_name).to_string(),
        created_at: session.get("created_at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        updated_at: session.get("updated_at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        message_count: 0,
        workspace_id: session
            .get("workspace_id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or(workspace_id),
        workspace_name: None,
    })
}

/// List sessions for the current context.
/// - `workspace_id = Some(id)`: sessions belonging to that workspace
/// - `workspace_id = None`: all sessions (global view)
#[tauri::command]
pub async fn list_sessions(
    state: State<'_, Arc<RwLock<AppState>>>,
    workspace_id: Option<String>,
) -> Result<Vec<SessionInfo>, String> {
    let state_guard = state.read().await;

    let result = if let Some(ref ws_id) = workspace_id {
        state_guard.backend.sessions_list_by_workspace(Some(ws_id.as_str())).await
    } else {
        state_guard.backend.sessions_list().await
    }?;

    let mut all_sessions: Vec<SessionInfo> = result
        .get("sessions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| {
                    let id = s.get("id").and_then(|v| v.as_str())?.to_string();
                    Some(SessionInfo {
                        id,
                        name: s.get("name").and_then(|v| v.as_str()).unwrap_or("Untitled").to_string(),
                        created_at: s.get("created_at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        updated_at: s.get("updated_at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        message_count: s.get("message_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                        workspace_id: s.get("workspace_id").and_then(|v| v.as_str()).map(String::from),
                        workspace_name: s.get("workspace_name").and_then(|v| v.as_str()).map(String::from),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    all_sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(all_sessions)
}

/// Get session history.
#[tauri::command]
pub async fn get_session_history(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<Vec<ChatMessage>, String> {
    let state_guard = state.read().await;
    let result = state_guard
        .backend
        .session_history(&session_id, Some(500))
        .await?;

    Ok(result
        .get("messages")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let tool_calls = m
                        .get("tool_calls")
                        .and_then(|tc| serde_json::from_value::<Vec<ToolCallInfo>>(tc.clone()).ok())
                        .unwrap_or_default();
                    let reasoning = m.get("reasoning").and_then(|r| r.as_str()).map(str::to_string);
                    Some(ChatMessage {
                        id: m.get("id")?.as_str()?.to_string(),
                        role: m.get("role")?.as_str()?.to_string(),
                        content: m.get("content")?.as_str()?.to_string(),
                        timestamp: m.get("timestamp")?.as_str()?.to_string(),
                        tool_calls,
                        reasoning,
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Delete a session.
#[tauri::command]
pub async fn delete_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<(), String> {
    let state_guard = state.read().await;
    state_guard.backend.session_delete(&session_id).await?;
    Ok(())
}

/// Delete all sessions.
#[tauri::command]
pub async fn clear_all_sessions(
    app: AppHandle,
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<usize, String> {
    let state_guard = state.read().await;
    let count = match state_guard.backend.sessions_delete_all().await {
        Ok(result) => result.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        Err(e) => {
            warn!("Failed to clear daemon sessions: {e}");
            return Err(format!("Failed to clear sessions: {e}"));
        }
    };
    info!("Cleared {count} sessions");
    let _ = app.emit("sessions-cleared", count);
    Ok(count)
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchiveResult {
    memories_created: usize,
    session_deleted: bool,
}

/// Archive a session and delete it.
///
/// The daemon auto-extracts memories from every turn as the conversation
/// happens, so there is nothing extra to archive on delete — this simply deletes
/// the session. (The client no longer runs its own extraction LLM pass.)
#[tauri::command]
pub async fn archive_and_delete_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<ArchiveResult, String> {
    info!("Deleting session {session_id} (memories are extracted per-turn by the daemon)");
    delete_session(state, session_id).await?;
    Ok(ArchiveResult {
        memories_created: 0,
        session_deleted: true,
    })
}

/// Rename a session.
#[tauri::command]
pub async fn rename_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
    name: String,
) -> Result<(), String> {
    let state_guard = state.read().await;
    state_guard.backend.session_rename(&session_id, &name).await?;
    Ok(())
}

/// Set or clear the workspace for a session.
#[tauri::command]
pub async fn set_session_workspace(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
    workspace_id: Option<String>,
) -> Result<(), String> {
    let state_guard = state.read().await;
    let result = state_guard
        .backend
        .session_set_workspace(&session_id, workspace_id.as_deref())
        .await?;
    if result.get("error").is_some() {
        return Err(result["message"].as_str().unwrap_or("Unknown error").to_string());
    }
    Ok(())
}

// =============================================================================
// Sub-Session Commands (#72)
// =============================================================================

/// Spawn a sub-agent session.
#[tauri::command]
pub async fn spawn_sub_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    task: String,
    label: Option<String>,
    parent_id: Option<String>,
    model: Option<String>,
    max_iterations: Option<usize>,
    timeout_secs: Option<u64>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.daemon_request(serde_json::json!({
        "type": "session",
        "action": "spawn_sub_session",
        "task": task,
        "label": label,
        "parent_id": parent_id,
        "model": model,
        "max_iterations": max_iterations,
        "timeout_secs": timeout_secs,
    })).await
}

/// List sub-sessions.
#[tauri::command]
pub async fn list_sub_sessions(
    state: State<'_, Arc<RwLock<AppState>>>,
    parent_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.daemon_request(serde_json::json!({
        "type": "session",
        "action": "list_sub_sessions",
        "parent_id": parent_id,
    })).await
}

/// Kill a sub-session.
#[tauri::command]
pub async fn kill_sub_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    target: String,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.daemon_request(serde_json::json!({
        "type": "session",
        "action": "kill_sub_session",
        "target": target,
    })).await
}

/// Get sub-session status.
#[tauri::command]
pub async fn get_sub_session_status(
    state: State<'_, Arc<RwLock<AppState>>>,
    target: String,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.daemon_request(serde_json::json!({
        "type": "session",
        "action": "get_sub_session_status",
        "target": target,
    })).await
}

/// Send a message to a sub-session.
#[tauri::command]
pub async fn send_to_sub_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    target: String,
    message: String,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.daemon_request(serde_json::json!({
        "type": "session",
        "action": "send_to_sub_session",
        "target": target,
        "message": message,
    })).await
}

/// Get session run state (in-flight streaming text, active tools).
#[tauri::command]
pub async fn get_session_run_state(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<serde_json::Value, String> {
    let state = state.read().await;
    state.backend.session_get_run_state(&session_id).await
}

// =============================================================================
// Cancellation
// =============================================================================

/// Cancel an active agent session.
#[tauri::command]
pub async fn cancel_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<bool, String> {
    let state_guard = state.read().await;
    state_guard.backend.chat_cancel(&session_id).await
}
