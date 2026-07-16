//! Memory management commands.

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

/// Search across all sessions
#[tauri::command]
pub async fn search_memory(
    state: State<'_, Arc<RwLock<AppState>>>,
    query: String,
    limit: Option<u32>,
) -> Result<Vec<MemorySearchResult>, String> {
    let state_guard = state.read().await;
    let max_results = limit.unwrap_or(50) as usize;
    let query_lower = query.to_lowercase();

    // Get all sessions as (id, name) — from local storage when we have it
    // open, otherwise from the daemon (which owns nanna.db in daemon mode)
    let sessions: Vec<(String, String)> = if let Some(storage) = &state_guard.storage {
        storage
            .list_gui_sessions(1000)
            .await
            .map_err(|e| format!("Failed to list sessions: {}", e))?
            .iter()
            .map(|s| (s.session_id.clone(), Storage::get_session_name(s)))
            .collect()
    } else {
        let result = state_guard.backend.sessions_list().await
            .map_err(|e| format!("Failed to list sessions: {}", e))?;
        result
            .get("sessions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| {
                        let id = s.get("id")?.as_str()?.to_string();
                        let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("Untitled").to_string();
                        Some((id, name))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let mut results = Vec::new();
    let is_daemon = state_guard.backend.is_daemon_mode().await;

    for (session_id, session_name) in &sessions {
        // Collect messages from daemon or local storage
        let messages: Vec<(String, String, String, String)> = if is_daemon {
            // Try daemon first
            if let Ok(result) = state_guard.backend.session_history(session_id, Some(1000)).await {
                if let Some(msgs) = result.get("messages").and_then(|v| v.as_array()) {
                    msgs.iter().filter_map(|m| {
                        Some((
                            m.get("id")?.as_str()?.to_string(),
                            m.get("role")?.as_str()?.to_string(),
                            m.get("content")?.as_str()?.to_string(),
                            m.get("timestamp")?.as_str()?.to_string(),
                        ))
                    }).collect()
                } else {
                    vec![]
                }
            } else if let Some(storage) = &state_guard.storage {
                // Fallback to local storage
                storage
                    .get_session_messages(session_id, 1000)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|m| (m.id.to_string(), m.role, m.content, m.created_at))
                    .collect()
            } else {
                vec![]
            }
        } else {
            state_guard
                .storage()?
                .get_session_messages(session_id, 1000)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|m| (m.id.to_string(), m.role, m.content, m.created_at))
                .collect()
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

/// Get statistics for memory browser
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

    // In daemon mode, the daemon's session list already carries message counts
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard.backend.sessions_list().await
            .map_err(|e| format!("Failed to list sessions: {}", e))?;
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

        return Ok(MemoryStats {
            total_sessions: sessions.len() as u32,
            total_messages,
            oldest_session: timestamps.first().cloned(),
            newest_session: timestamps.last().cloned(),
        });
    }

    // Embedded mode: direct storage access
    let storage = state_guard.storage()?;
    let sessions = storage
        .list_gui_sessions(10000)
        .await
        .map_err(|e| format!("Failed to list sessions: {}", e))?;

    let mut total_messages = 0u32;
    for session in &sessions {
        let count = storage
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

/// Set dreaming (memory consolidation) enabled
#[tauri::command]
pub async fn set_dreaming_enabled(
    state: State<'_, Arc<RwLock<AppState>>>,
    enabled: bool,
) -> Result<(), String> {
    let state_guard = state.read().await;
    *state_guard.dreaming_enabled.write().await = enabled;
    info!("Dreaming enabled: {}", enabled);
    Ok(())
}

/// Set max compression ratio for memory consolidation
#[tauri::command]
pub async fn set_max_compression_ratio(
    state: State<'_, Arc<RwLock<AppState>>>,
    ratio: f32,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    let clamped = ratio.clamp(0.1, 0.9);
    state_guard.config.memory.max_compression_ratio = clamped;
    state_guard.config.save().map_err(|e| format!("Failed to save config: {}", e))?;
    info!("Max compression ratio set: {}", clamped);
    Ok(())
}

/// Set minimum remaining memories floor for consolidation
#[tauri::command]
pub async fn set_min_remaining_memories(
    state: State<'_, Arc<RwLock<AppState>>>,
    count: usize,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    let clamped = count.max(5);
    state_guard.config.memory.min_remaining_memories = clamped;
    state_guard.config.save().map_err(|e| format!("Failed to save config: {}", e))?;
    info!("Min remaining memories set: {}", clamped);
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

/// Get cognitive memory statistics
#[tauri::command]
pub async fn get_cognitive_memory_stats(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<CognitiveMemoryStats, String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        if let Ok(result) = state_guard.backend.memory_stats().await {
            // Parse daemon response
            return Ok(CognitiveMemoryStats {
                total_memories: result.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                active: result.get("active").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                dormant: result.get("dormant").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                silent: result.get("silent").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                unavailable: result.get("unavailable").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                consolidation_enabled: result.get("consolidation_enabled").and_then(|v| v.as_bool()).unwrap_or(true),
                last_consolidation: result.get("last_consolidation").and_then(|v| v.as_str()).map(|s| s.to_string()),
            });
        }
        // Fall through to local if daemon fails
    }

    // Embedded mode / fallback: direct memory service access
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
pub async fn trigger_consolidation(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<ConsolidationResultInfo, String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        if let Ok(result) = state_guard.backend.memory_consolidate().await {
            // Parse daemon response
            return Ok(ConsolidationResultInfo {
                memories_processed: result.get("memories_processed").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                clusters_formed: result.get("clusters_formed").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                memories_merged: result.get("memories_merged").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                memories_expanded: result.get("memories_expanded").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                errors: result.get("errors")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default(),
            });
        }
        // Fall through to local if daemon fails
    }

    // Embedded mode / fallback: run consolidation locally
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
pub async fn apply_memory_updates(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    let state_guard = state.read().await;
    state_guard.memory.apply_pending_updates().await;
    Ok(())
}

/// Manually save memories to disk
#[tauri::command]
pub async fn save_memories(
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
    pub workspace_id: Option<String>,  // None = global, Some = workspace-scoped
}

/// List all semantic memories (with optional workspace scope filter)
///
/// scope: None = all memories, Some("global") = global only, Some(ws_id) = global + that workspace
#[tauri::command]
pub async fn list_memories(
    state: State<'_, Arc<RwLock<AppState>>>,
    scope: Option<String>,
) -> Result<Vec<MemoryItem>, String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        // Use proper memory_list action (scope filtering done by daemon)
        if let Ok(result) = state_guard.backend.memory_list(scope.as_deref()).await {
            if let Some(memories_array) = result.get("memories").and_then(|v: &serde_json::Value| v.as_array()) {
                let mut items: Vec<MemoryItem> = memories_array.iter().filter_map(|m: &serde_json::Value| {
                    Some(MemoryItem {
                        id: m.get("id")?.as_str()?.to_string(),
                        content: m.get("content")?.as_str()?.to_string(),
                        fact_type: m.get("fact_type").and_then(|v: &serde_json::Value| v.as_str()).unwrap_or("stated").to_string(),
                        importance: m.get("importance").and_then(|v: &serde_json::Value| v.as_f64()).unwrap_or(3.0) as f32,
                        state: m.get("state").and_then(|v: &serde_json::Value| v.as_str()).unwrap_or("active").to_string(),
                        weight: m.get("weight").and_then(|v: &serde_json::Value| v.as_f64()).unwrap_or(1.0) as f32,
                        retrievability: m.get("retrievability").and_then(|v: &serde_json::Value| v.as_f64()).unwrap_or(1.0) as f32,
                        access_count: m.get("access_count").and_then(|v: &serde_json::Value| v.as_u64()).unwrap_or(0) as u32,
                        created_at: m.get("created_at").and_then(|v: &serde_json::Value| v.as_str()).unwrap_or("").to_string(),
                        session_id: m.get("session_id").and_then(|v: &serde_json::Value| v.as_str()).map(String::from),
                        workspace_id: m.get("workspace_id").and_then(|v: &serde_json::Value| v.as_str()).map(String::from),
                    })
                }).collect();
                items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
                return Ok(items);
            }
        }
        // Fall through to local if daemon fails
    }

    // Embedded mode / fallback: direct memory service access
    let entries = state_guard.memory.list_all().await;

    // Filter by scope
    let filtered: Vec<_> = entries.into_iter().filter(|e| {
        match &scope {
            None => true, // No filter - show all
            Some(s) if s == "global" => e.workspace_id.is_none(), // Global only
            Some(ws_id) => {
                // Workspace scope: show global + that workspace
                e.workspace_id.is_none() || e.workspace_id.as_deref() == Some(ws_id)
            }
        }
    }).collect();

    let mut items: Vec<MemoryItem> = filtered.into_iter().map(|e| {
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
            workspace_id: e.workspace_id,
        }
    }).collect();
    items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(items)
}

/// Get a single memory by ID
#[tauri::command]
pub async fn get_memory(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: String,
) -> Result<Option<MemoryItem>, String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        if let Ok(result) = state_guard.backend.memory_get(&id).await {
            if let Some(m) = result.get("memory") {
                return Ok(Some(MemoryItem {
                    id: m.get("id").and_then(|v| v.as_str()).unwrap_or(&id).to_string(),
                    content: m.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    fact_type: m.get("fact_type").and_then(|v| v.as_str()).unwrap_or("stated").to_string(),
                    importance: m.get("importance").and_then(|v| v.as_f64()).unwrap_or(3.0) as f32,
                    state: m.get("state").and_then(|v| v.as_str()).unwrap_or("active").to_string(),
                    weight: m.get("weight").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
                    retrievability: m.get("retrievability").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
                    access_count: m.get("access_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                    created_at: m.get("created_at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    session_id: m.get("session_id").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    workspace_id: m.get("workspace_id").and_then(|v| v.as_str()).map(|s| s.to_string()),
                }));
            }
            return Ok(None);
        }
        // Fall through to local if daemon fails
    }

    // Embedded mode / fallback: direct memory service access
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
            workspace_id: e.workspace_id,
        }
    }))
}

/// Delete a memory by ID
#[tauri::command]
pub async fn delete_memory(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: String,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        if let Ok(_) = state_guard.backend.memory_delete(&id).await {
            info!("Deleted memory via daemon: {}", id);
            return Ok(());
        }
        // Fall through to local if daemon fails
    }

    // Embedded mode / fallback: direct memory service access
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
pub async fn update_memory(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: String,
    content: String,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        if let Ok(_) = state_guard.backend.memory_update(&id, Some(&content), None).await {
            info!("Updated memory via daemon: {}", id);
            return Ok(());
        }
        // Fall through to local if daemon fails
    }

    // Embedded mode / fallback: direct memory service access
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
pub async fn clear_all_memories(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        state_guard.backend.memory_clear().await?;
        info!("Cleared all memories (via daemon)");
        return Ok(());
    }

    // Embedded mode: direct memory service access
    state_guard.memory.clear().await;

    // Save empty state
    state_guard.memory.save(&state_guard.memory_path).await
        .map_err(|e| format!("Failed to save after clear: {}", e))?;

    info!("Cleared all memories");
    Ok(())
}

// =============================================================================
// Similarity Threshold Configuration
// =============================================================================

/// Get the current similarity threshold
#[tauri::command]
pub async fn get_similarity_threshold(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<f32, String> {
    let state_guard = state.read().await;
    Ok(state_guard.memory.get_min_score())
}

/// Set the similarity threshold for memory recall
#[tauri::command]
pub async fn set_similarity_threshold(
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
