//! Memory management commands. The daemon owns the memory store; these forward
//! to it. Tuning knobs that have a config home are persisted to `config.toml`
//! and pushed to the daemon; knobs the daemon manages internally are no-ops.

#[allow(clippy::wildcard_imports)]
use crate::*;

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

/// Search across all sessions (substring match over daemon-stored history).
#[tauri::command]
pub async fn search_memory(
    state: State<'_, Arc<RwLock<AppState>>>,
    query: String,
    limit: Option<u32>,
) -> Result<Vec<MemorySearchResult>, String> {
    let state_guard = state.read().await;
    let max_results = limit.unwrap_or(50) as usize;
    let query_lower = query.to_lowercase();

    // Sessions come from the daemon (it owns nanna.db).
    let sessions: Vec<(String, String)> = {
        let result = state_guard
            .backend
            .sessions_list()
            .await
            .map_err(|e| format!("Failed to list sessions: {e}"))?;
        result
            .get("sessions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| {
                        let id = s.get("id")?.as_str()?.to_string();
                        let name = s
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Untitled")
                            .to_string();
                        Some((id, name))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let mut results = Vec::new();

    for (session_id, session_name) in &sessions {
        let messages: Vec<(String, String, String, String)> =
            if let Ok(result) = state_guard.backend.session_history(session_id, Some(1000)).await {
                result
                    .get("messages")
                    .and_then(|v| v.as_array())
                    .map(|msgs| {
                        msgs.iter()
                            .filter_map(|m| {
                                Some((
                                    m.get("id")?.as_str()?.to_string(),
                                    m.get("role")?.as_str()?.to_string(),
                                    m.get("content")?.as_str()?.to_string(),
                                    m.get("timestamp")?.as_str()?.to_string(),
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                vec![]
            };

        for (msg_id, role, content, timestamp) in messages {
            let content_lower = content.to_lowercase();
            if content_lower.contains(&query_lower) {
                let pos = content_lower.find(&query_lower).unwrap_or(0);
                let start = pos.saturating_sub(50);
                let end = (pos + query.len() + 50).min(content.len());
                let snippet = if start > 0 || end < content.len() {
                    let prefix = if start > 0 { "..." } else { "" };
                    let suffix = if end < content.len() { "..." } else { "" };
                    format!("{}{}{}", prefix, &content[start..end], suffix)
                } else {
                    content.clone()
                };

                let matches = content_lower.matches(&query_lower).count();
                let relevance = (matches as f32 / content.len().max(1) as f32).min(1.0);

                results.push(MemorySearchResult {
                    session_id: session_id.clone(),
                    session_name: session_name.clone(),
                    message_id: msg_id,
                    role,
                    content,
                    timestamp,
                    snippet,
                    relevance,
                });
            }
        }
    }

    results.sort_by(|a, b| b.relevance.partial_cmp(&a.relevance).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(max_results);

    Ok(results)
}

/// Statistics for the memory browser
#[derive(Debug, Clone, Serialize)]
pub struct MemoryStats {
    pub total_sessions: u32,
    pub total_messages: u32,
    pub oldest_session: Option<String>,
    pub newest_session: Option<String>,
}

#[tauri::command]
pub async fn get_memory_stats(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<MemoryStats, String> {
    let state_guard = state.read().await;

    let result = state_guard
        .backend
        .sessions_list()
        .await
        .map_err(|e| format!("Failed to list sessions: {e}"))?;
    let sessions = result
        .get("sessions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut total_messages = 0u32;
    let mut timestamps: Vec<String> = Vec::new();
    for session in &sessions {
        total_messages += session
            .get("message_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        if let Some(created) = session.get("created_at").and_then(|v| v.as_str()) {
            timestamps.push(created.to_string());
        }
    }
    timestamps.sort();

    Ok(MemoryStats {
        total_sessions: sessions.len() as u32,
        total_messages,
        oldest_session: timestamps.first().cloned(),
        newest_session: timestamps.last().cloned(),
    })
}

/// Set dreaming (memory consolidation) enabled.
///
/// The daemon runs consolidation on its own schedule; there is no runtime toggle
/// for it over IPC yet, so this is a no-op accepted for UI compatibility.
#[tauri::command]
pub async fn set_dreaming_enabled(
    _state: State<'_, Arc<RwLock<AppState>>>,
    enabled: bool,
) -> Result<(), String> {
    info!("set_dreaming_enabled({enabled}) is a no-op in daemon-only mode (daemon manages consolidation scheduling)");
    Ok(())
}

/// Set max compression ratio for memory consolidation (persisted to config +
/// pushed to the daemon).
#[tauri::command]
pub async fn set_max_compression_ratio(
    state: State<'_, Arc<RwLock<AppState>>>,
    ratio: f32,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    let clamped = ratio.clamp(0.1, 0.9);
    state_guard.config.memory.max_compression_ratio = clamped;
    state_guard.config.save().map_err(|e| format!("Failed to save config: {e}"))?;
    let _ = state_guard
        .backend
        .config_set("memory.max_compression_ratio", serde_json::json!(clamped))
        .await;
    info!("Max compression ratio set: {clamped}");
    Ok(())
}

/// Set minimum remaining memories floor for consolidation (persisted to config +
/// pushed to the daemon).
#[tauri::command]
pub async fn set_min_remaining_memories(
    state: State<'_, Arc<RwLock<AppState>>>,
    count: usize,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    let clamped = count.max(5);
    state_guard.config.memory.min_remaining_memories = clamped;
    state_guard.config.save().map_err(|e| format!("Failed to save config: {e}"))?;
    let _ = state_guard
        .backend
        .config_set("memory.min_remaining_memories", serde_json::json!(clamped))
        .await;
    info!("Min remaining memories set: {clamped}");
    Ok(())
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

#[tauri::command]
pub async fn get_cognitive_memory_stats(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<CognitiveMemoryStats, String> {
    let state_guard = state.read().await;
    let result = state_guard
        .backend
        .memory_stats()
        .await
        .map_err(|e| format!("Failed to get memory stats: {e}"))?;

    Ok(CognitiveMemoryStats {
        total_memories: result.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        active: result.get("active").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        dormant: result.get("dormant").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        silent: result.get("silent").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        unavailable: result.get("unavailable").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        consolidation_enabled: result
            .get("consolidation_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        last_consolidation: result
            .get("last_consolidation")
            .and_then(|v| v.as_str())
            .map(str::to_string),
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

/// Manually trigger memory consolidation ("dream").
#[tauri::command]
pub async fn trigger_consolidation(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<ConsolidationResultInfo, String> {
    let state_guard = state.read().await;
    let result = state_guard
        .backend
        .memory_consolidate()
        .await
        .map_err(|e| format!("Consolidation failed: {e}"))?;

    Ok(ConsolidationResultInfo {
        memories_processed: result.get("memories_processed").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        clusters_formed: result.get("clusters_formed").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        memories_merged: result.get("memories_merged").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        memories_expanded: result.get("memories_expanded").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        errors: result
            .get("errors")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default(),
    })
}

/// Apply pending FSRS updates. The daemon applies these itself during recall, so
/// this is a no-op accepted for UI compatibility.
#[tauri::command]
pub async fn apply_memory_updates(
    _state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    Ok(())
}

/// Manually save memories. The daemon persists via Turso write-through on every
/// mutation, so there is nothing to flush from the client — a no-op.
#[tauri::command]
pub async fn save_memories(
    _state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
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
    pub fact_type: String,
    pub importance: f32,
    pub state: String,
    pub weight: f32,
    pub retrievability: f32,
    pub access_count: u32,
    pub created_at: String,
    pub session_id: Option<String>,
    pub workspace_id: Option<String>,
}

fn memory_item_from_json(m: &serde_json::Value) -> Option<MemoryItem> {
    Some(MemoryItem {
        id: m.get("id")?.as_str()?.to_string(),
        content: m.get("content")?.as_str()?.to_string(),
        fact_type: m.get("fact_type").and_then(|v| v.as_str()).unwrap_or("stated").to_string(),
        importance: m.get("importance").and_then(serde_json::Value::as_f64).unwrap_or(3.0) as f32,
        state: m.get("state").and_then(|v| v.as_str()).unwrap_or("active").to_string(),
        weight: m.get("weight").and_then(serde_json::Value::as_f64).unwrap_or(1.0) as f32,
        retrievability: m.get("retrievability").and_then(serde_json::Value::as_f64).unwrap_or(1.0) as f32,
        access_count: m.get("access_count").and_then(serde_json::Value::as_u64).unwrap_or(0) as u32,
        created_at: m.get("created_at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        session_id: m.get("session_id").and_then(|v| v.as_str()).map(String::from),
        workspace_id: m.get("workspace_id").and_then(|v| v.as_str()).map(String::from),
    })
}

/// Resolve the page's tab keyword into what the daemon expects: the daemon
/// filter takes "global" or an actual workspace ID. The page sends the
/// literal "workspace" plus the active workspace's id — forwarding the
/// literal matched a workspace named "workspace" (nothing) and showed the
/// global set on both tabs (observed live).
fn resolve_memory_scope(scope: Option<String>, workspace_id: Option<String>) -> Option<String> {
    match scope.as_deref() {
        Some("workspace") => workspace_id.or(scope),
        _ => scope,
    }
}

/// List semantic memories. Scope semantics: "global" = global-only;
/// a workspace id = global + that workspace (what the agent sees there).
#[tauri::command]
pub async fn list_memories(
    state: State<'_, Arc<RwLock<AppState>>>,
    scope: Option<String>,
    workspace_id: Option<String>,
) -> Result<Vec<MemoryItem>, String> {
    let effective = resolve_memory_scope(scope, workspace_id);
    let state_guard = state.read().await;
    let result = state_guard
        .backend
        .memory_list(effective.as_deref())
        .await
        .map_err(|e| format!("Failed to list memories: {e}"))?;

    let mut items: Vec<MemoryItem> = result
        .get("memories")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(memory_item_from_json).collect())
        .unwrap_or_default();
    items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(items)
}

/// Get a single memory by ID.
#[tauri::command]
pub async fn get_memory(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: String,
) -> Result<Option<MemoryItem>, String> {
    let state_guard = state.read().await;
    let result = state_guard
        .backend
        .memory_get(&id)
        .await
        .map_err(|e| format!("Failed to get memory: {e}"))?;
    Ok(result.get("memory").and_then(memory_item_from_json))
}

/// Delete a memory by ID.
#[tauri::command]
pub async fn delete_memory(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: String,
) -> Result<(), String> {
    let state_guard = state.read().await;
    state_guard
        .backend
        .memory_delete(&id)
        .await
        .map_err(|e| format!("Failed to delete memory: {e}"))?;
    info!("Deleted memory: {id}");
    Ok(())
}

/// Update a memory's content.
#[tauri::command]
pub async fn update_memory(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: String,
    content: String,
) -> Result<(), String> {
    let state_guard = state.read().await;
    state_guard
        .backend
        .memory_update(&id, Some(&content), None)
        .await
        .map_err(|e| format!("Failed to update memory: {e}"))?;
    info!("Updated memory: {id}");
    Ok(())
}

/// Clear memories in a scope. "global" clears global-only; a workspace id
/// clears ONLY that workspace's entries (never the globals its tab also
/// displays — destructive ops stay conservative); no scope clears all.
/// (This is the command the memory page actually invokes — the old
/// `clear_all_memories` name was never called, so the button was dead.)
#[tauri::command]
pub async fn clear_memories(
    state: State<'_, Arc<RwLock<AppState>>>,
    scope: Option<String>,
    workspace_id: Option<String>,
) -> Result<(), String> {
    let effective = resolve_memory_scope(scope, workspace_id);
    let state_guard = state.read().await;
    state_guard
        .backend
        .memory_clear(effective.as_deref())
        .await
        .map_err(|e| format!("Failed to clear memories: {e}"))?;
    info!("Cleared memories (scope: {:?}, via daemon)", effective);
    Ok(())
}

// =============================================================================
// Similarity Threshold Configuration
// =============================================================================

/// Get the current similarity threshold.
///
/// The daemon owns the memory service and does not expose this over IPC yet, so
/// the client reports the neutral default.
#[tauri::command]
pub async fn get_similarity_threshold(
    _state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<f32, String> {
    Ok(0.0)
}

/// Set the similarity threshold for memory recall.
///
/// No daemon control action exists for this yet; accepted for UI compatibility
/// (validates the range) but does not change daemon behavior.
#[tauri::command]
pub async fn set_similarity_threshold(
    _state: State<'_, Arc<RwLock<AppState>>>,
    threshold: f32,
) -> Result<String, String> {
    if !(0.0..=1.0).contains(&threshold) {
        return Err("Threshold must be between 0.0 and 1.0".to_string());
    }
    info!("set_similarity_threshold({threshold}) is a no-op in daemon-only mode");
    Ok(format!("Similarity threshold set to {threshold:.2}"))
}
