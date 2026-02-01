//! Agent visualization commands for Tauri IPC
//!
//! Provides real-time agent state, workspace clustering, and lifecycle events
//! for the /agents visualization page.

use nanna_agent::{AgentRegistry, RegisteredAgent, LifecycleEvent};
use nanna_llm::LlmClient;
use nanna_tools::ToolRegistry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::RwLock;
use tracing::{debug, info};

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

impl From<&RegisteredAgent> for AgentInfo {
    fn from(agent: &RegisteredAgent) -> Self {
        Self {
            id: agent.id.clone(),
            parent_id: agent.parent_id.clone(),
            workspace_path: agent.workspace.as_ref().map(|p| p.to_string_lossy().to_string()),
            workspace_name: agent.workspace.as_ref().and_then(|p| {
                p.file_name().map(|n| n.to_string_lossy().to_string())
            }),
            model: agent.config.model.clone(),
            role: format!("{:?}", agent.role).to_lowercase(),
            state: format!("{:?}", agent.state).to_lowercase(),
            state_changed_at: agent.state_changed_at,
            spawned_at: agent.spawned_at,
            children: agent.children.clone(),
            current_tool: agent.current_tool.clone(),
            tokens_in: agent.tokens_in,
            tokens_out: agent.tokens_out,
        }
    }
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

/// Lifecycle event for frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEventPayload {
    pub event_type: String,
    pub agent_id: String,
    pub timestamp: i64,
    pub data: serde_json::Value,
}

impl From<&LifecycleEvent> for AgentEventPayload {
    fn from(event: &LifecycleEvent) -> Self {
        let (event_type, data) = match event {
            LifecycleEvent::Spawned { agent_id: _, parent_id, workspace, config } => {
                ("spawned", serde_json::json!({
                    "parent_id": parent_id,
                    "workspace": workspace.as_ref().map(|p| p.to_string_lossy().to_string()),
                    "model": config.model,
                    "role": format!("{:?}", config.role).to_lowercase(),
                }))
            }
            LifecycleEvent::StateChanged { agent_id: _, old_state, new_state, tool_name } => {
                ("state_changed", serde_json::json!({
                    "old_state": format!("{:?}", old_state).to_lowercase(),
                    "new_state": format!("{:?}", new_state).to_lowercase(),
                    "tool_name": tool_name,
                }))
            }
            LifecycleEvent::ToolStarted { agent_id: _, tool_name, tool_id } => {
                ("tool_started", serde_json::json!({
                    "tool_name": tool_name,
                    "tool_id": tool_id,
                }))
            }
            LifecycleEvent::ToolCompleted { agent_id: _, tool_name, tool_id, success, duration_ms } => {
                ("tool_completed", serde_json::json!({
                    "tool_name": tool_name,
                    "tool_id": tool_id,
                    "success": success,
                    "duration_ms": duration_ms,
                }))
            }
            LifecycleEvent::Completed { agent_id: _, response, duration_ms, tokens_in, tokens_out } => {
                ("completed", serde_json::json!({
                    "response_preview": response.chars().take(100).collect::<String>(),
                    "duration_ms": duration_ms,
                    "tokens_in": tokens_in,
                    "tokens_out": tokens_out,
                }))
            }
            LifecycleEvent::Error { agent_id: _, error, duration_ms } => {
                ("error", serde_json::json!({
                    "error": error,
                    "duration_ms": duration_ms,
                }))
            }
            LifecycleEvent::Cancelled { agent_id: _, reason } => {
                ("cancelled", serde_json::json!({
                    "reason": reason,
                }))
            }
            LifecycleEvent::MessageSent { from_id, to_id, content } => {
                ("message_sent", serde_json::json!({
                    "from_id": from_id,
                    "to_id": to_id,
                    "content_preview": content.chars().take(100).collect::<String>(),
                }))
            }
        };

        Self {
            event_type: event_type.to_string(),
            agent_id: event.agent_id().to_string(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            data,
        }
    }
}

/// Agent registry state for the GUI
pub struct AgentRegistryState {
    pub registry: Arc<AgentRegistry>,
    /// Handle for the event listener task
    listener_handle: Option<tokio::task::JoinHandle<()>>,
}

impl AgentRegistryState {
    pub fn new(llm: Arc<LlmClient>, tools: Arc<ToolRegistry>) -> Self {
        Self {
            registry: Arc::new(AgentRegistry::new(llm, tools)),
            listener_handle: None,
        }
    }

    /// Start listening for lifecycle events and emit to frontend
    pub fn start_event_listener(&mut self, app: AppHandle) {
        let mut receiver = self.registry.subscribe();
        
        let handle = tokio::spawn(async move {
            loop {
                match receiver.recv().await {
                    Ok(event) => {
                        let payload = AgentEventPayload::from(&event);
                        if let Err(e) = app.emit("agent-event", &payload) {
                            debug!("Failed to emit agent event: {}", e);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        debug!("Agent event listener lagged {} events", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        info!("Agent event channel closed");
                        break;
                    }
                }
            }
        });

        self.listener_handle = Some(handle);
    }
}

// =============================================================================
// Tauri Commands
// =============================================================================

/// Get all agents grouped by workspace
#[tauri::command]
pub async fn get_agent_clusters(
    state: State<'_, Arc<RwLock<AgentRegistryState>>>,
) -> Result<Vec<WorkspaceCluster>, String> {
    let state = state.read().await;
    let registry = &state.registry;
    
    let all_agents = registry.all().await;
    
    // Group by workspace
    let mut workspace_map: HashMap<Option<PathBuf>, Vec<&RegisteredAgent>> = HashMap::new();
    for agent in &all_agents {
        workspace_map
            .entry(agent.workspace.clone())
            .or_default()
            .push(agent);
    }
    
    let mut clusters = Vec::new();
    for (workspace, agents) in workspace_map {
        let path = workspace
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "(global)".to_string());
        
        let name = workspace
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Global".to_string());
        
        let agent_infos: Vec<AgentInfo> = agents.iter().map(|a| AgentInfo::from(*a)).collect();
        let active_count = agents.iter().filter(|a| a.state.is_active()).count();
        let total_in: u32 = agents.iter().map(|a| a.tokens_in).sum();
        let total_out: u32 = agents.iter().map(|a| a.tokens_out).sum();
        
        clusters.push(WorkspaceCluster {
            path,
            name,
            total_agents: agents.len(),
            active_agents: active_count,
            agents: agent_infos,
            total_tokens_in: total_in,
            total_tokens_out: total_out,
        });
    }
    
    // Sort by name, global last
    clusters.sort_by(|a, b| {
        if a.path == "(global)" { std::cmp::Ordering::Greater }
        else if b.path == "(global)" { std::cmp::Ordering::Less }
        else { a.name.cmp(&b.name) }
    });
    
    Ok(clusters)
}

/// Get all agents as a flat list
#[tauri::command]
pub async fn get_all_agents(
    state: State<'_, Arc<RwLock<AgentRegistryState>>>,
) -> Result<Vec<AgentInfo>, String> {
    let state = state.read().await;
    let agents = state.registry.all().await;
    Ok(agents.iter().map(AgentInfo::from).collect())
}

/// Get a specific agent by ID
#[tauri::command]
pub async fn get_agent(
    state: State<'_, Arc<RwLock<AgentRegistryState>>>,
    agent_id: String,
) -> Result<Option<AgentInfo>, String> {
    let state = state.read().await;
    let agent = state.registry.get(&agent_id).await;
    Ok(agent.as_ref().map(AgentInfo::from))
}

/// Get children of an agent
#[tauri::command]
pub async fn get_agent_children(
    state: State<'_, Arc<RwLock<AgentRegistryState>>>,
    agent_id: String,
) -> Result<Vec<AgentInfo>, String> {
    let state = state.read().await;
    let children = state.registry.children(&agent_id).await;
    Ok(children.iter().map(AgentInfo::from).collect())
}

/// Get global agent statistics
#[tauri::command]
pub async fn get_agent_stats(
    state: State<'_, Arc<RwLock<AgentRegistryState>>>,
) -> Result<AgentStats, String> {
    let state = state.read().await;
    let registry = &state.registry;
    
    let all_agents = registry.all().await;
    let active = registry.active().await;
    let (tokens_in, tokens_out) = registry.total_tokens().await;
    let state_counts = registry.count_by_state().await;
    
    // Count by role
    let mut role_counts: HashMap<String, usize> = HashMap::new();
    for agent in &all_agents {
        let role = format!("{:?}", agent.role).to_lowercase();
        *role_counts.entry(role).or_insert(0) += 1;
    }
    
    // Count unique workspaces
    let workspaces: std::collections::HashSet<_> = all_agents
        .iter()
        .filter_map(|a| a.workspace.as_ref())
        .collect();
    
    Ok(AgentStats {
        total_agents: all_agents.len(),
        active_agents: active.len(),
        by_state: state_counts.into_iter()
            .map(|(k, v)| (format!("{:?}", k).to_lowercase(), v))
            .collect(),
        by_role: role_counts,
        total_tokens_in: tokens_in,
        total_tokens_out: tokens_out,
        workspaces: workspaces.len(),
    })
}

/// Cancel an agent
#[tauri::command]
pub async fn cancel_agent(
    state: State<'_, Arc<RwLock<AgentRegistryState>>>,
    agent_id: String,
    reason: Option<String>,
) -> Result<(), String> {
    let state = state.read().await;
    state.registry.cancel(&agent_id, reason).await;
    Ok(())
}

/// Start listening for agent events (call once on page mount)
#[tauri::command]
pub async fn subscribe_agent_events(
    app: AppHandle,
    state: State<'_, Arc<RwLock<AgentRegistryState>>>,
) -> Result<(), String> {
    let mut state = state.write().await;
    
    // Only start if not already listening
    if state.listener_handle.is_none() {
        state.start_event_listener(app);
        info!("Started agent event listener");
    }
    
    Ok(())
}

/// Clean up completed agents older than threshold
#[tauri::command]
pub async fn cleanup_completed_agents(
    state: State<'_, Arc<RwLock<AgentRegistryState>>>,
    older_than_secs: i64,
) -> Result<(), String> {
    let state = state.read().await;
    state.registry.cleanup(older_than_secs).await;
    Ok(())
}

/// Get agents for a specific workspace
#[tauri::command]
pub async fn get_workspace_agents(
    state: State<'_, Arc<RwLock<AgentRegistryState>>>,
    workspace_path: String,
) -> Result<Vec<AgentInfo>, String> {
    let state = state.read().await;
    let path = PathBuf::from(workspace_path);
    let agents = state.registry.by_workspace(&path).await;
    Ok(agents.iter().map(AgentInfo::from).collect())
}
