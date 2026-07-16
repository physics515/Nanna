//! Session management commands.

#[allow(clippy::wildcard_imports)]
use crate::*;

/// Create a new session
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

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard.backend.session_create_in_workspace(
            Some(&session_name),
            workspace_id.as_deref(),
        ).await?;
        // Daemon returns { "session": { ... } }
        let session = result.get("session")
            .ok_or("Invalid daemon response: missing 'session' field")?;
        return Ok(SessionInfo {
            id: session.get("id")
                .and_then(|v| v.as_str())
                .ok_or("Invalid daemon response: missing 'id'")?
                .to_string(),
            name: session.get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(&session_name)
                .to_string(),
            created_at: session.get("created_at")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            updated_at: session.get("updated_at")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            message_count: 0,
            workspace_id: session.get("workspace_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or(workspace_id),
            workspace_name: None,
        });
    }

    // Embedded mode: direct storage access

    // Get workspace name if workspace_id provided
    let workspace_name = if let Some(ref ws_id) = workspace_id {
        let registry = state_guard.workspaces.read().await;
        registry.get(ws_id).map(|ws| ws.name.clone())
    } else {
        None
    };

    let session = state_guard
        .storage()?
        .create_gui_session_with_workspace(&session_name, workspace_id.as_deref())
        .await
        .map_err(|e| format!("Failed to create session: {}", e))?;

    let name = Storage::get_session_name(&session);
    Ok(SessionInfo {
        id: session.session_id,
        name,
        created_at: session.created_at,
        updated_at: session.updated_at,
        message_count: 0,
        workspace_id: session.workspace_id,
        workspace_name,
    })
}

/// List sessions for the current context
/// - workspace_id = Some(id): Show sessions belonging to that workspace
/// - workspace_id = None: Show only GLOBAL sessions (workspace_id IS NULL)
///
/// Memory access model:
/// - Global sessions: Access ALL memory (global + all workspaces) - omniscient
/// - Workspace sessions: Access global + their workspace's memory only - scoped
#[tauri::command]
pub async fn list_sessions(
    state: State<'_, Arc<RwLock<AppState>>>,
    workspace_id: Option<String>,
) -> Result<Vec<SessionInfo>, String> {
    let state_guard = state.read().await;

    // Route through daemon if available, but also merge with local SQLite sessions
    if state_guard.backend.is_daemon_mode().await {
        let mut all_sessions: Vec<SessionInfo> = Vec::new();

        // Get daemon sessions:
        // - workspace_id = None → show ALL sessions (global view)
        // - workspace_id = Some(id) → show only that workspace's sessions
        let result = if let Some(ref ws_id) = workspace_id {
            state_guard.backend.sessions_list_by_workspace(Some(ws_id.as_str())).await
        } else {
            // Global: fetch ALL sessions (no workspace filter)
            state_guard.backend.sessions_list().await
        };
        if let Ok(result) = result {
            if let Some(sessions_array) = result.get("sessions").and_then(|v| v.as_array()) {
                for s in sessions_array {
                    if let (Some(id), Some(name)) = (
                        s.get("id").and_then(|v| v.as_str()),
                        s.get("name").and_then(|v| v.as_str()).or(Some("Untitled"))
                    ) {
                        all_sessions.push(SessionInfo {
                            id: id.to_string(),
                            name: name.to_string(),
                            created_at: s.get("created_at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            updated_at: s.get("updated_at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            message_count: s.get("message_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                            workspace_id: s.get("workspace_id").and_then(|v| v.as_str()).map(|s| s.to_string()),
                            workspace_name: s.get("workspace_name").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        });
                    }
                }
            }
        }

        // Also merge sessions from local SQLite when we have it open (old
        // sessions from embedded mode). In daemon mode storage is None — the
        // daemon owns nanna.db, so its list already covers everything.
        let sqlite_result = match &state_guard.storage {
            Some(storage) if workspace_id.is_some() => {
                storage.sessions().list_by_workspace(workspace_id.as_deref(), 100).await
            }
            Some(storage) => storage.list_gui_sessions(100).await,
            None => Ok(Vec::new()),
        };
        if let Ok(sqlite_sessions) = sqlite_result {
            let daemon_ids: std::collections::HashSet<_> = all_sessions.iter().map(|s| s.id.clone()).collect();

            for session in sqlite_sessions {
                // Only add if not already in daemon list
                if !daemon_ids.contains(&session.session_id) {
                    let name = nanna_storage::Storage::get_session_name(&session);
                    all_sessions.push(SessionInfo {
                        id: session.session_id,
                        name,
                        created_at: session.created_at,
                        updated_at: session.updated_at,
                        message_count: 0, // Would need another query
                        workspace_id: session.workspace_id,
                        workspace_name: None,
                    });
                }
            }
        }

        // Sort by updated_at descending (newest first)
        all_sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        return Ok(all_sessions);
    }

    // Embedded mode: use direct storage access
    // workspace_id = None → show ALL sessions (global view)
    // workspace_id = Some(id) → show only that workspace's sessions
    let storage = state_guard.storage()?;
    let sessions = if let Some(ref ws_id) = workspace_id {
        storage
            .list_gui_sessions_by_workspace(Some(ws_id.as_str()), 100)
            .await
            .map_err(|e| format!("Failed to list sessions: {}", e))?
    } else {
        // Global: list ALL sessions regardless of workspace
        storage
            .list_gui_sessions(100)
            .await
            .map_err(|e| format!("Failed to list sessions: {}", e))?
    };

    // Build workspace name lookup
    let registry = state_guard.workspaces.read().await;

    let mut result = Vec::with_capacity(sessions.len());
    for s in sessions {
        let count = storage
            .count_session_messages(&s.session_id)
            .await
            .unwrap_or(0);

        // Get workspace name if session has workspace_id
        let workspace_name = s.workspace_id.as_ref()
            .and_then(|ws_id| registry.get(ws_id))
            .map(|ws| ws.name.clone());

        result.push(SessionInfo {
            id: s.session_id.clone(),
            name: Storage::get_session_name(&s),
            created_at: s.created_at,
            updated_at: s.updated_at,
            message_count: count as u32,
            workspace_id: s.workspace_id.clone(),
            workspace_name,
        });
    }

    Ok(result)
}

/// Get session history
#[tauri::command]
/// Parse tool calls from message metadata
pub fn parse_tool_calls_from_metadata(metadata: &Option<serde_json::Value>) -> Vec<ToolCallInfo> {
    metadata
        .as_ref()
        .and_then(|m| m.get("tool_calls"))
        .and_then(|tc| serde_json::from_value::<Vec<ToolCallInfo>>(tc.clone()).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub async fn get_session_history(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<Vec<ChatMessage>, String> {
    let state_guard = state.read().await;

    // Route through daemon if available, with fallback to SQLite for old sessions
    if state_guard.backend.is_daemon_mode().await {
        // Try daemon first
        if let Ok(result) = state_guard.backend.session_history(&session_id, Some(500)).await {
            if let Some(messages_array) = result.get("messages").and_then(|v| v.as_array()) {
                if !messages_array.is_empty() {
                    return Ok(messages_array.iter().filter_map(|m| {
                        // Parse tool calls from top-level field (daemon SessionMessage format)
                        let tool_calls = m.get("tool_calls")
                            .and_then(|tc| serde_json::from_value::<Vec<ToolCallInfo>>(tc.clone()).ok())
                            .unwrap_or_default();
                        // Parse reasoning from top-level field
                        let reasoning = m.get("reasoning")
                            .and_then(|r| r.as_str())
                            .map(|s| s.to_string());
                        Some(ChatMessage {
                            id: m.get("id")?.as_str()?.to_string(),
                            role: m.get("role")?.as_str()?.to_string(),
                            content: m.get("content")?.as_str()?.to_string(),
                            timestamp: m.get("timestamp")?.as_str()?.to_string(),
                            tool_calls,
                            reasoning,
                        })
                    }).collect());
                }
            }
        }

        // Fallback to SQLite for old sessions (from embedded mode) — only
        // possible when the GUI has the local DB open (storage is Some)
        if let Some(storage) = &state_guard.storage {
            if let Ok(messages) = storage.get_session_messages(&session_id, 500).await {
                return Ok(messages
                    .into_iter()
                    .map(|m| ChatMessage {
                        id: m.id.to_string(),
                        role: m.role,
                        content: m.content,
                        timestamp: m.created_at,
                        tool_calls: parse_tool_calls_from_metadata(&m.metadata),
                        reasoning: None,
                    })
                    .collect());
            }
        }

        // Session truly not found anywhere
        return Ok(vec![]);
    }

    // Embedded mode: direct storage access
    let messages = state_guard
        .storage()?
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
            tool_calls: parse_tool_calls_from_metadata(&m.metadata),
            reasoning: None,
        })
        .collect())
}

/// Delete a session
#[tauri::command]
pub async fn delete_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        // Try daemon first
        if let Ok(_) = state_guard.backend.session_delete(&session_id).await {
            return Ok(());
        }
        // Fall through to SQLite for backward compatibility
    }

    // Embedded mode / fallback: direct storage access
    state_guard
        .storage()?
        .delete_session(&session_id)
        .await
        .map_err(|e| format!("Failed to delete session: {}", e))?;

    Ok(())
}

/// Delete all sessions
#[tauri::command]
pub async fn clear_all_sessions(
    app: AppHandle,
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<usize, String> {
    let state_guard = state.read().await;

    // Clear local SQLite storage when we have it open (GUI sessions live there
    // in embedded mode). Use list_gui_sessions to get ALL sessions (both
    // global and workspace-scoped).
    let mut local_count = 0;
    if let Some(storage) = &state_guard.storage {
        let sessions = storage.list_gui_sessions(1000)
            .await
            .map_err(|e| format!("Failed to list sessions: {}", e))?;

        local_count = sessions.len();
        for session in sessions {
            if let Err(e) = storage.delete_session(&session.session_id).await {
                warn!("Failed to delete local session {}: {}", session.session_id, e);
            }
        }
        info!("Cleared {} local sessions from SQLite", local_count);
    }

    // Also clear daemon sessions if in daemon mode
    if state_guard.backend.is_daemon_mode().await {
        match state_guard.backend.sessions_delete_all().await {
            Ok(result) => {
                let daemon_count = result.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
                info!("Cleared {} daemon sessions", daemon_count);
            }
            Err(e) => {
                warn!("Failed to clear daemon sessions: {}", e);
            }
        }
    }

    // Emit event to notify frontend to refresh sessions list
    let _ = app.emit("sessions-cleared", local_count);

    Ok(local_count)
}

/// Archive session to memory and delete
/// Extracts important information from the conversation before deletion
#[tauri::command]
pub async fn archive_and_delete_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<ArchiveResult, String> {
    info!("Archiving session {} to memory before deletion", session_id);

    // Get session history first
    let messages = get_session_history(state.clone(), session_id.clone()).await?;

    if messages.is_empty() {
        // Nothing to archive, just delete
        delete_session(state.clone(), session_id).await?;
        return Ok(ArchiveResult {
            memories_created: 0,
            session_deleted: true,
        });
    }

    // Build conversation text for analysis
    let conversation = messages
        .iter()
        .map(|m| format!("{}: {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n\n");

    let state_guard = state.read().await;

    // Get session name for context (best-effort; local storage only)
    let session_name = match &state_guard.storage {
        Some(storage) => storage
            .sessions()
            .get(&session_id)
            .await
            .ok()
            .map(|s| nanna_storage::Storage::get_session_name(&s)),
        None => None,
    }
    .unwrap_or_else(|| "Unnamed chat".to_string());

    // Check if memory is enabled
    let embedding_enabled = *state_guard.embedding_enabled.read().await;
    if !embedding_enabled {
        drop(state_guard);
        // Memory not enabled, just delete
        delete_session(state.clone(), session_id).await?;
        return Ok(ArchiveResult {
            memories_created: 0,
            session_deleted: true,
        });
    }

    // Create extraction prompt
    let extraction_prompt = format!(
        r#"Analyze this conversation titled "{}" and extract important information that should be remembered.

Focus on:
- User preferences, habits, or personal information shared
- Important decisions made
- Technical solutions or approaches discussed
- Action items or commitments
- Key insights or learnings
- Project context (if any)

For each piece of important information, output it on a separate line prefixed with "MEMORY:" followed by a clear, self-contained statement.

Only extract genuinely important or useful information. Do not extract trivial or temporary information.
If nothing is worth remembering, output "NO_MEMORIES".

Conversation:
{}
"#,
        session_name, conversation
    );

    // Use the LLM to extract memories
    let llm = state_guard.llm.clone();
    let extraction_model = state_guard.extraction_model.read().await.clone();
    let model = if extraction_model.is_empty() {
        state_guard.config.llm.model.clone()
    } else {
        extraction_model
    };

    drop(state_guard);

    // Call LLM to extract memories
    let request = CompletionRequest {
        model: model.clone(),
        messages: vec![LlmMessage {
            role: Role::User,
            content: extraction_prompt,
        }],
        max_tokens: Some(2000),
        ..Default::default()
    };

    let extraction_result = llm.complete(&request).await;

    let mut memories_created = 0;

    if let Ok(response) = extraction_result {
        // Parse extracted memories
        let extracted_memories: Vec<String> = response
            .lines()
            .filter(|line: &&str| line.starts_with("MEMORY:"))
            .map(|line: &str| line.trim_start_matches("MEMORY:").trim().to_string())
            .filter(|s: &String| !s.is_empty())
            .collect();

        if !extracted_memories.is_empty() {
            let state_guard = state.read().await;
            let memory = state_guard.memory.clone();
            drop(state_guard);

            // Store each extracted memory with metadata
            for memory_content in &extracted_memories {
                let tagged_content = format!("[ARCHIVED:{}] {}", session_name, memory_content);
                let mut metadata = std::collections::HashMap::new();
                metadata.insert("source".to_string(), "archived_session".to_string());
                metadata.insert("session_name".to_string(), session_name.clone());
                if let Err(e) = memory.remember(&tagged_content, metadata).await {
                    warn!("Failed to store archived memory: {}", e);
                } else {
                    memories_created += 1;
                }
            }

            info!("Archived {} memories from session {}", memories_created, session_id);
        }
    } else {
        warn!("Memory extraction failed, proceeding with deletion anyway");
    }

    // Now delete the session
    delete_session(state.clone(), session_id).await?;

    Ok(ArchiveResult {
        memories_created,
        session_deleted: true,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchiveResult {
    memories_created: usize,
    session_deleted: bool,
}

/// Rename a session
#[tauri::command]
pub async fn rename_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
    name: String,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        // Try daemon first
        if let Ok(_) = state_guard.backend.session_rename(&session_id, &name).await {
            return Ok(());
        }
        // Fall through to SQLite for backward compatibility
    }

    // Embedded mode / fallback: direct storage access
    state_guard
        .storage()?
        .rename_session(&session_id, &name)
        .await
        .map_err(|e| format!("Failed to rename session: {}", e))?;

    Ok(())
}

/// Set or clear the workspace for a session
#[tauri::command]
pub async fn set_session_workspace(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
    workspace_id: Option<String>,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard.backend.session_set_workspace(
            &session_id,
            workspace_id.as_deref(),
        ).await?;
        if result.get("error").is_some() {
            return Err(result["message"].as_str().unwrap_or("Unknown error").to_string());
        }
        return Ok(());
    }

    // Embedded mode: update storage directly
    state_guard
        .storage()?
        .set_session_workspace(&session_id, workspace_id.as_deref())
        .await
        .map_err(|e| format!("Failed to set session workspace: {}", e))?;

    Ok(())
}

// =============================================================================
// Sub-Session Commands (#72)
// =============================================================================

/// Spawn a sub-agent session
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

/// List sub-sessions
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

/// Kill a sub-session
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

/// Get sub-session status
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

/// Send a message to a sub-session
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

/// Get session run state (in-flight streaming text, active tools)
/// Works in both daemon mode (queries daemon) and embedded mode (local state).
#[tauri::command]
pub async fn get_session_run_state(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<serde_json::Value, String> {
    let state = state.read().await;

    // Try daemon mode first
    if state.backend.is_daemon_mode().await {
        return state.backend.session_get_run_state(&session_id).await;
    }

    // Embedded mode: query the in-process AgentService (same tracker the
    // daemon uses — accumulated text/thinking, active + completed tools).
    let msg_count = match &state.storage {
        Some(storage) => storage.count_session_messages(&session_id).await.unwrap_or(0) as usize,
        None => 0,
    };

    if let Some(ref agent_service) = state.agent_service {
        // Embedded sessions live in local SQLite, not a daemon SessionManager,
        // so pass an empty manager and patch the message count from storage.
        let sessions = nanna_daemon::SessionManager::new();
        let snapshot = agent_service.get_run_state(&session_id, &sessions).await;
        let mut value = serde_json::to_value(&snapshot)
            .map_err(|e| format!("Failed to serialize run state: {}", e))?;
        value["message_count"] = serde_json::json!(msg_count);
        Ok(value)
    } else {
        Ok(serde_json::json!({
            "is_running": false,
            "accumulated_text": "",
            "accumulated_thinking": "",
            "active_tool_calls": [],
            "completed_tool_calls": [],
            "started_at": null,
            "message_count": msg_count,
        }))
    }
}

// =============================================================================
// Cancellation & Logs
// =============================================================================

/// Cancel an active agent session
#[tauri::command]
pub async fn cancel_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<bool, String> {
    let state_guard = state.read().await;
    if state_guard.backend.is_daemon_mode().await {
        state_guard.backend.chat_cancel(&session_id).await
    } else if let Some(ref agent_service) = state_guard.agent_service {
        // Embedded mode: delegate to the in-process AgentService, which trips
        // the agent loop's cooperative cancellation flag. The loop finishes
        // gracefully with whatever it has so far (same behavior as the daemon).
        let cancelled = agent_service.cancel(&session_id).await;
        if cancelled {
            info!("Embedded run for session {} flagged for cancellation", session_id);
        }
        Ok(cancelled)
    } else {
        // No in-process agent service — nothing to cancel.
        Ok(false)
    }
}
