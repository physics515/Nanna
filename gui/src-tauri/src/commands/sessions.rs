//! Session management commands. The daemon owns nanna.db; these forward to it.

#[allow(clippy::wildcard_imports)]
use crate::*;

/// Create a new session.
///
/// # Errors
///
/// Fails when the daemon cannot be reached or the `session.create_in_workspace`
/// request is dropped or times out. Fails with `Invalid daemon response: …`
/// when the reply has no `session` object or that object has no `id` — which is
/// also how a refusal reported in the reply surfaces.
#[tauri::command]
pub async fn create_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: Option<String>,
    workspace_id: Option<String>,
) -> Result<SessionInfo, String> {
    let backend = backend_handle(&state).await;

    let session_name = name.unwrap_or_else(|| {
        format!("Chat {}", chrono::Utc::now().format("%Y-%m-%d %H:%M"))
    });

    let result = backend
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
        // A session created this instant cannot carry a pin: `set_model` is the
        // only thing that writes one, and it needs the id this call returns.
        chat_model: None,
        // Same for tool picks: `set_tools` is the only writer.
        chat_tools: Vec::new(),
    })
}

/// List sessions for the current context.
/// - `workspace_id = Some(id)`: sessions belonging to that workspace
/// - `workspace_id = None`: all sessions (global view)
///
/// # Errors
///
/// Fails when the daemon cannot be reached or the `session.list` (or, with a
/// workspace, `session.list_by_workspace`) request is dropped or times out. A
/// reply without a `sessions` array lists nothing.
#[tauri::command]
pub async fn list_sessions(
    state: State<'_, Arc<RwLock<AppState>>>,
    workspace_id: Option<String>,
) -> Result<Vec<SessionInfo>, String> {
    let backend = backend_handle(&state).await;

    let result = if let Some(ref ws_id) = workspace_id {
        backend.sessions_list_by_workspace(Some(ws_id.as_str())).await
    } else {
        backend.sessions_list().await
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
                        // Saturates where `as` wrapped; a count past u32::MAX is unreachable.
                        message_count: s
                            .get("message_count")
                            .and_then(serde_json::Value::as_u64)
                            .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX)),
                        workspace_id: s.get("workspace_id").and_then(|v| v.as_str()).map(String::from),
                        workspace_name: s.get("workspace_name").and_then(|v| v.as_str()).map(String::from),
                        // Absent (not null) on an unpinned session — the daemon's
                        // `SessionSummary` skips the key when there is no pin.
                        chat_model: s.get("chat_model").and_then(|v| v.as_str()).map(String::from),
                        // Absent when no tools were manually picked (the daemon
                        // skips the key on an empty selection).
                        chat_tools: s
                            .get("chat_tools")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|t| t.as_str())
                                    .map(String::from)
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    all_sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(all_sessions)
}

/// Get session history.
///
/// # Errors
///
/// Fails when the daemon cannot be reached or the `session.history` request is
/// dropped or times out. A reply without a `messages` array is an empty
/// history, and malformed messages are skipped.
#[tauri::command]
pub async fn get_session_history(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<Vec<ChatMessage>, String> {
    // Effectively no limit: the chat page must reload the WHOLE session on
    // remount — a cap here silently dropped history for long-horizon runs.
    // An explicit huge bound (rather than None) keeps one guarantee under
    // version skew: an older daemon defaulted None to 50.
    let result = backend_handle(&state)
        .await
        .session_history(&session_id, Some(1_000_000))
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
                    let timeline = m.get("timeline").filter(|t| t.is_array()).cloned();
                    let usage = m.get("usage").filter(|u| u.is_object()).cloned();
                    Some(ChatMessage {
                        id: m.get("id")?.as_str()?.to_string(),
                        role: m.get("role")?.as_str()?.to_string(),
                        content: m.get("content")?.as_str()?.to_string(),
                        timestamp: m.get("timestamp")?.as_str()?.to_string(),
                        tool_calls,
                        reasoning,
                        timeline,
                        usage,
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Delete a session.
///
/// # Errors
///
/// Fails when the daemon cannot be reached or the `session.delete` request is
/// dropped or times out. A refusal the daemon reports in its reply (an unknown
/// id, say) is not checked and still returns `Ok`.
#[tauri::command]
pub async fn delete_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<(), String> {
    backend_handle(&state).await.session_delete(&session_id).await?;
    Ok(())
}

/// Delete all sessions.
///
/// # Errors
///
/// Returns `Failed to clear sessions: …` when the daemon cannot be reached or
/// the `session.delete_all` request is dropped or times out.
#[tauri::command]
pub async fn clear_all_sessions(
    app: AppHandle,
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<usize, String> {
    let count = match backend_handle(&state).await.sessions_delete_all().await {
        // Lossless on the 64-bit targets this ships for; saturates otherwise.
        Ok(result) => result
            .get("count")
            .and_then(serde_json::Value::as_u64)
            .map_or(0, |n| usize::try_from(n).unwrap_or(usize::MAX)),
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
///
/// # Errors
///
/// Fails exactly as [`delete_session`] does.
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
///
/// # Errors
///
/// Fails when the daemon cannot be reached or the `session.rename` request is
/// dropped or times out. A refusal the daemon reports in its reply (an unknown
/// id, say) is not checked and still returns `Ok`.
#[tauri::command]
pub async fn rename_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
    name: String,
) -> Result<(), String> {
    backend_handle(&state).await.session_rename(&session_id, &name).await?;
    Ok(())
}

/// Set or clear the workspace for a session.
///
/// # Errors
///
/// Fails when the daemon cannot be reached or the `session.set_workspace`
/// request is dropped or times out. Also fails with the daemon's `message` when
/// its reply reports an `error`.
#[tauri::command]
pub async fn set_session_workspace(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
    workspace_id: Option<String>,
) -> Result<(), String> {
    let result = backend_handle(&state)
        .await
        .session_set_workspace(&session_id, workspace_id.as_deref())
        .await?;
    if result.get("error").is_some() {
        return Err(result["message"].as_str().unwrap_or("Unknown error").to_string());
    }
    Ok(())
}

/// Set or clear the chat-model pin for a session.
///
/// `model = None` clears the pin and the chat follows the global `[llm]`
/// default again. The pin covers chat replies only — the sub-agent,
/// summarization and embedding models stay global.
///
/// # Errors
///
/// Fails when the daemon cannot be reached or the `session.set_model` request
/// is dropped or times out. Also fails with the daemon's `message` when it
/// refuses the pin: an unknown session, or a model no live provider serves.
#[tauri::command]
pub async fn set_session_model(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
    model: Option<String>,
) -> Result<(), String> {
    let result = backend_handle(&state)
        .await
        .session_set_model(&session_id, model.as_deref())
        .await?;
    // A refused pin — unknown session, or a model no live provider serves —
    // comes back in the body, not as a transport error. Swallowing it here
    // would leave the picker showing a model that cannot answer a single turn,
    // so it is raised as an `Err` for the caller to surface and revert.
    if result.get("error").is_some() {
        return Err(result["message"].as_str().unwrap_or("Unknown error").to_string());
    }
    Ok(())
}

/// Set or clear the user-selected extra tools for a session.
///
/// Additive by contract: the daemon unions these into whatever active set the
/// turn would have built anyway. An empty list clears the selection and
/// restores byte-identical default tool behavior.
///
/// # Errors
///
/// Fails when the daemon cannot be reached or the `session.set_tools` request
/// is dropped or times out. Also fails with the daemon's `message` when it
/// refuses the selection (an unknown session).
#[tauri::command]
pub async fn set_session_tools(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
    tools: Vec<String>,
) -> Result<(), String> {
    let result = backend_handle(&state).await.session_set_tools(&session_id, tools).await?;
    // The only daemon-side refusal is an unknown session; surface it rather
    // than leaving the picker showing a selection that was never recorded.
    if result.get("error").is_some() {
        return Err(result["message"].as_str().unwrap_or("Unknown error").to_string());
    }
    Ok(())
}

// =============================================================================
// Sub-Session Commands (#72)
// =============================================================================

/// Spawn a sub-agent session.
///
/// # Errors
///
/// Fails when the daemon cannot be reached or the `session.spawn_sub_session`
/// request is dropped or times out. A refusal the daemon reports in its reply
/// is passed through inside `Ok`.
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
    backend_handle(&state).await.daemon_request(serde_json::json!({
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
///
/// # Errors
///
/// Fails when the daemon cannot be reached or the `session.list_sub_sessions`
/// request is dropped or times out. A refusal the daemon reports in its reply
/// is passed through inside `Ok`.
#[tauri::command]
pub async fn list_sub_sessions(
    state: State<'_, Arc<RwLock<AppState>>>,
    parent_id: Option<String>,
) -> Result<serde_json::Value, String> {
    backend_handle(&state).await.daemon_request(serde_json::json!({
        "type": "session",
        "action": "list_sub_sessions",
        "parent_id": parent_id,
    })).await
}

/// Kill a sub-session.
///
/// # Errors
///
/// Fails when the daemon cannot be reached or the `session.kill_sub_session`
/// request is dropped or times out. A refusal the daemon reports in its reply
/// is passed through inside `Ok`.
#[tauri::command]
pub async fn kill_sub_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    target: String,
) -> Result<serde_json::Value, String> {
    backend_handle(&state).await.daemon_request(serde_json::json!({
        "type": "session",
        "action": "kill_sub_session",
        "target": target,
    })).await
}

/// Get sub-session status.
///
/// # Errors
///
/// Fails when the daemon cannot be reached or the
/// `session.get_sub_session_status` request is dropped or times out. A refusal
/// the daemon reports in its reply is passed through inside `Ok`.
#[tauri::command]
pub async fn get_sub_session_status(
    state: State<'_, Arc<RwLock<AppState>>>,
    target: String,
) -> Result<serde_json::Value, String> {
    backend_handle(&state).await.daemon_request(serde_json::json!({
        "type": "session",
        "action": "get_sub_session_status",
        "target": target,
    })).await
}

/// Send a message to a sub-session.
///
/// # Errors
///
/// Fails when the daemon cannot be reached or the `session.send_to_sub_session`
/// request is dropped or times out. A refusal the daemon reports in its reply
/// is passed through inside `Ok`.
#[tauri::command]
pub async fn send_to_sub_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    target: String,
    message: String,
) -> Result<serde_json::Value, String> {
    backend_handle(&state).await.daemon_request(serde_json::json!({
        "type": "session",
        "action": "send_to_sub_session",
        "target": target,
        "message": message,
    })).await
}

/// Get session run state (in-flight streaming text, active tools).
/// `light: true` skips the run journal — for periodic counter polls.
///
/// # Errors
///
/// Fails when the daemon cannot be reached or the `session.get_run_state`
/// request is dropped or times out. A refusal the daemon reports in its reply
/// is passed through inside `Ok`.
#[tauri::command]
pub async fn get_session_run_state(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
    light: Option<bool>,
) -> Result<serde_json::Value, String> {
    backend_handle(&state)
        .await
        .session_get_run_state(&session_id, light.unwrap_or(false))
        .await
}

// =============================================================================
// Cancellation
// =============================================================================

/// Cancel an active agent session.
///
/// # Errors
///
/// Fails when the daemon cannot be reached or the `chat.cancel` request is
/// dropped or times out. `Ok(false)` means no turn was running.
#[tauri::command]
pub async fn cancel_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<bool, String> {
    backend_handle(&state).await.chat_cancel(&session_id).await
}
