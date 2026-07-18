//! Agent-visualization commands for the `/agents` page.
//!
//! In daemon-only mode the "agents" the GUI can visualize are the daemon's
//! **sub-sessions** (spawned via the `task` tool / `spawn_sub_session`). These
//! commands query the daemon's sub-session registry and shape it into the types
//! the frontend already renders. There is no in-process agent registry anymore.
//!
//! Live lifecycle events are not yet bridged to a Tauri `agent-event` feed, so
//! `subscribe_agent_events` is a no-op; the page polls the query commands.

use crate::AppState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;

/// Agent info for frontend display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub parent_id: Option<String>,
    pub workspace_path: Option<String>,
    pub workspace_name: Option<String>,
    pub model: String,
    pub role: String,
    pub state: String,
    pub state_changed_at: i64,
    pub spawned_at: i64,
    pub children: Vec<String>,
    pub current_tool: Option<String>,
    pub tokens_in: u32,
    pub tokens_out: u32,
}

/// Workspace cluster for frontend display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceCluster {
    pub path: String,
    pub name: String,
    pub agents: Vec<AgentInfo>,
    pub total_agents: usize,
    pub active_agents: usize,
    pub total_tokens_in: u32,
    pub total_tokens_out: u32,
}

/// Global agent stats
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStats {
    pub total_agents: usize,
    pub active_agents: usize,
    pub by_state: HashMap<String, usize>,
    pub by_role: HashMap<String, usize>,
    pub total_tokens_in: u32,
    pub total_tokens_out: u32,
    pub workspaces: usize,
}

/// Parse a timestamp field that may be an integer (unix seconds) or an RFC3339
/// string; `0` if absent/unparseable.
fn parse_ts(v: Option<&serde_json::Value>) -> i64 {
    match v {
        Some(serde_json::Value::Number(n)) => n.as_i64().unwrap_or(0),
        Some(serde_json::Value::String(s)) => chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.timestamp())
            .unwrap_or(0),
        _ => 0,
    }
}

/// Whether a sub-session state counts as "active" for stats/clustering.
fn state_is_active(state: &str) -> bool {
    !matches!(state, "completed" | "failed" | "killed" | "error" | "cancelled")
}

/// Map one daemon sub-session JSON object into an `AgentInfo`.
fn sub_session_to_agent(v: &serde_json::Value) -> Option<AgentInfo> {
    let id = v
        .get("session_id")
        .or_else(|| v.get("id"))
        .and_then(|x| x.as_str())?
        .to_string();
    let spawned_at = parse_ts(v.get("spawned_at"));
    Some(AgentInfo {
        id,
        parent_id: v.get("parent_id").and_then(|x| x.as_str()).map(String::from),
        workspace_path: None,
        workspace_name: None,
        model: v.get("model").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        role: "worker".to_string(),
        state: v.get("state").and_then(|x| x.as_str()).unwrap_or("unknown").to_string(),
        state_changed_at: parse_ts(v.get("finished_at")).max(spawned_at),
        spawned_at,
        children: Vec::new(),
        current_tool: v.get("label").and_then(|x| x.as_str()).map(String::from),
        tokens_in: 0,
        tokens_out: 0,
    })
}

/// Fetch the daemon's sub-sessions and map them to `AgentInfo`.
async fn fetch_agents(state: &AppState) -> Vec<AgentInfo> {
    let result = state
        .backend
        .daemon_request(serde_json::json!({
            "type": "session",
            "action": "list_sub_sessions",
            "parent_id": null,
        }))
        .await;

    let Ok(result) = result else {
        return Vec::new();
    };
    result
        .get("sub_sessions")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(sub_session_to_agent).collect())
        .unwrap_or_default()
}

// =============================================================================
// Tauri Commands
// =============================================================================

/// Get all agents grouped by workspace. Sub-sessions are not workspace-tagged,
/// so they land in a single global cluster.
#[tauri::command]
pub async fn get_agent_clusters(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<WorkspaceCluster>, String> {
    let state = state.read().await;
    let agents = fetch_agents(&state).await;
    if agents.is_empty() {
        return Ok(Vec::new());
    }

    let active_agents = agents.iter().filter(|a| state_is_active(&a.state)).count();
    let total_tokens_in = agents.iter().map(|a| a.tokens_in).sum();
    let total_tokens_out = agents.iter().map(|a| a.tokens_out).sum();
    let total_agents = agents.len();

    Ok(vec![WorkspaceCluster {
        path: "(global)".to_string(),
        name: "Global".to_string(),
        agents,
        total_agents,
        active_agents,
        total_tokens_in,
        total_tokens_out,
    }])
}

/// Get all agents as a flat list.
#[tauri::command]
pub async fn get_all_agents(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<AgentInfo>, String> {
    let state = state.read().await;
    Ok(fetch_agents(&state).await)
}

/// Get a specific agent by ID.
#[tauri::command]
pub async fn get_agent(
    state: State<'_, Arc<RwLock<AppState>>>,
    agent_id: String,
) -> Result<Option<AgentInfo>, String> {
    let state = state.read().await;
    Ok(fetch_agents(&state).await.into_iter().find(|a| a.id == agent_id))
}

/// Get children of an agent.
#[tauri::command]
pub async fn get_agent_children(
    state: State<'_, Arc<RwLock<AppState>>>,
    agent_id: String,
) -> Result<Vec<AgentInfo>, String> {
    let state = state.read().await;
    Ok(fetch_agents(&state)
        .await
        .into_iter()
        .filter(|a| a.parent_id.as_deref() == Some(agent_id.as_str()))
        .collect())
}

/// Get global agent statistics.
#[tauri::command]
pub async fn get_agent_stats(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<AgentStats, String> {
    let state = state.read().await;
    let agents = fetch_agents(&state).await;

    let mut by_state: HashMap<String, usize> = HashMap::new();
    let mut by_role: HashMap<String, usize> = HashMap::new();
    for a in &agents {
        *by_state.entry(a.state.clone()).or_insert(0) += 1;
        *by_role.entry(a.role.clone()).or_insert(0) += 1;
    }

    Ok(AgentStats {
        total_agents: agents.len(),
        active_agents: agents.iter().filter(|a| state_is_active(&a.state)).count(),
        by_state,
        by_role,
        total_tokens_in: agents.iter().map(|a| a.tokens_in).sum(),
        total_tokens_out: agents.iter().map(|a| a.tokens_out).sum(),
        workspaces: 0,
    })
}

/// Cancel an agent (kills the underlying daemon sub-session).
#[tauri::command]
pub async fn cancel_agent(
    state: State<'_, Arc<RwLock<AppState>>>,
    agent_id: String,
    reason: Option<String>,
) -> Result<(), String> {
    let _ = reason;
    let state = state.read().await;
    state
        .backend
        .daemon_request(serde_json::json!({
            "type": "session",
            "action": "kill_sub_session",
            "target": agent_id,
        }))
        .await?;
    Ok(())
}

/// Start listening for agent events.
///
/// No-op: the daemon's sub-session lifecycle events are not yet bridged to a
/// Tauri `agent-event` feed. The page polls the query commands instead.
#[tauri::command]
pub async fn subscribe_agent_events(
    _state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    Ok(())
}

/// Clean up completed agents. No-op: the daemon manages sub-session lifetime.
#[tauri::command]
pub async fn cleanup_completed_agents(
    _state: State<'_, Arc<RwLock<AppState>>>,
    older_than_secs: i64,
) -> Result<(), String> {
    let _ = older_than_secs;
    Ok(())
}

/// Get agents for a specific workspace. Sub-sessions are not workspace-tagged,
/// so this returns an empty list.
#[tauri::command]
pub async fn get_workspace_agents(
    _state: State<'_, Arc<RwLock<AppState>>>,
    workspace_path: String,
) -> Result<Vec<AgentInfo>, String> {
    let _ = workspace_path;
    Ok(Vec::new())
}
