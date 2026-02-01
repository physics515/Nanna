//! Global Agent Registry with lifecycle events and workspace scoping
//!
//! Provides:
//! - Unique agent IDs across the system
//! - Parent-child relationship tracking
//! - Lifecycle events (spawn/running/complete/error/cancel)
//! - Per-workspace agent isolation
//! - Real-time state broadcasting for visualization

use crate::{Agent, AgentConfig, AgentContext, AgentError, AgentResponse, RunOptions};
use nanna_llm::LlmClient;
use nanna_tools::ToolRegistry;
use nanna_workspace::Workspace;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{info, warn};
use uuid::Uuid;

// =============================================================================
// Agent State & Lifecycle
// =============================================================================

/// Current state of an agent
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// Agent created but not yet running
    Spawned,
    /// Agent is idle, waiting for input
    Idle,
    /// Agent is thinking/reasoning
    Thinking,
    /// Agent is executing a tool
    ToolUse,
    /// Agent is waiting for external input
    Waiting,
    /// Agent completed successfully
    Completed,
    /// Agent encountered an error
    Error,
    /// Agent was cancelled
    Cancelled,
}

impl AgentState {
    /// Check if this is a terminal state
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Error | Self::Cancelled)
    }

    /// Check if agent is actively working
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(self, Self::Thinking | Self::ToolUse | Self::Waiting)
    }
}

/// Lifecycle event types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LifecycleEvent {
    /// Agent was spawned
    Spawned {
        agent_id: String,
        parent_id: Option<String>,
        workspace: Option<PathBuf>,
        config: AgentMetadata,
    },
    /// Agent state changed
    StateChanged {
        agent_id: String,
        old_state: AgentState,
        new_state: AgentState,
        tool_name: Option<String>,
    },
    /// Agent started a tool call
    ToolStarted {
        agent_id: String,
        tool_name: String,
        tool_id: String,
    },
    /// Agent finished a tool call
    ToolCompleted {
        agent_id: String,
        tool_name: String,
        tool_id: String,
        success: bool,
        duration_ms: u64,
    },
    /// Agent completed successfully
    Completed {
        agent_id: String,
        response: String,
        duration_ms: u64,
        tokens_in: u32,
        tokens_out: u32,
    },
    /// Agent encountered an error
    Error {
        agent_id: String,
        error: String,
        duration_ms: u64,
    },
    /// Agent was cancelled
    Cancelled {
        agent_id: String,
        reason: Option<String>,
    },
    /// Message sent between agents
    MessageSent {
        from_id: String,
        to_id: String,
        content: String,
    },
}

impl LifecycleEvent {
    /// Get the agent ID this event relates to
    #[must_use]
    pub fn agent_id(&self) -> &str {
        match self {
            Self::Spawned { agent_id, .. }
            | Self::StateChanged { agent_id, .. }
            | Self::ToolStarted { agent_id, .. }
            | Self::ToolCompleted { agent_id, .. }
            | Self::Completed { agent_id, .. }
            | Self::Error { agent_id, .. }
            | Self::Cancelled { agent_id, .. } => agent_id,
            Self::MessageSent { from_id, .. } => from_id,
        }
    }

    /// Get timestamp for this event
    #[must_use]
    pub fn timestamp(&self) -> i64 {
        chrono_timestamp()
    }
}

// =============================================================================
// Agent Metadata & Info
// =============================================================================

/// Metadata about an agent (serializable for events)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetadata {
    pub name: Option<String>,
    pub model: String,
    pub max_tokens: u32,
    pub role: AgentRole,
}

impl From<&AgentConfig> for AgentMetadata {
    fn from(config: &AgentConfig) -> Self {
        Self {
            name: None,
            model: config.model.clone(),
            max_tokens: config.max_tokens,
            role: AgentRole::SubAgent,
        }
    }
}

/// Role of an agent in the hierarchy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    /// Main agent for a workspace (one per workspace)
    Main,
    /// Sub-agent spawned by another agent
    SubAgent,
    /// Background/scheduled task agent
    Background,
    /// System agent (heartbeat, maintenance, etc.)
    System,
}

/// Full information about a registered agent
#[derive(Debug, Clone)]
pub struct RegisteredAgent {
    /// Unique agent ID
    pub id: String,
    /// Parent agent ID (if sub-agent)
    pub parent_id: Option<String>,
    /// Workspace this agent belongs to
    pub workspace: Option<PathBuf>,
    /// Agent configuration
    pub config: AgentConfig,
    /// System prompt
    pub system_prompt: String,
    /// Agent role
    pub role: AgentRole,
    /// Current state
    pub state: AgentState,
    /// When agent was spawned
    pub spawned_at: i64,
    /// When state last changed
    pub state_changed_at: i64,
    /// Child agent IDs
    pub children: Vec<String>,
    /// Current tool being executed (if any)
    pub current_tool: Option<String>,
    /// Accumulated token usage
    pub tokens_in: u32,
    pub tokens_out: u32,
    /// Custom metadata
    pub metadata: HashMap<String, String>,
}

impl RegisteredAgent {
    fn new(
        id: String,
        parent_id: Option<String>,
        workspace: Option<PathBuf>,
        config: AgentConfig,
        system_prompt: String,
        role: AgentRole,
    ) -> Self {
        let now = chrono_timestamp();
        Self {
            id,
            parent_id,
            workspace,
            config,
            system_prompt,
            role,
            state: AgentState::Spawned,
            spawned_at: now,
            state_changed_at: now,
            children: Vec::new(),
            current_tool: None,
            tokens_in: 0,
            tokens_out: 0,
            metadata: HashMap::new(),
        }
    }
}

// =============================================================================
// Agent Registry
// =============================================================================

/// Global counter for generating unique IDs
static AGENT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a unique agent ID
fn generate_agent_id() -> String {
    let count = AGENT_COUNTER.fetch_add(1, Ordering::SeqCst);
    let uuid = Uuid::new_v4();
    format!("agent-{count}-{}", &uuid.to_string()[..8])
}

/// Global agent registry
pub struct AgentRegistry {
    /// All registered agents (Arc for sharing with spawned tasks)
    agents: Arc<RwLock<HashMap<String, RegisteredAgent>>>,
    /// Agents grouped by workspace
    workspace_agents: RwLock<HashMap<PathBuf, Vec<String>>>,
    /// Parent-child relationships
    children: RwLock<HashMap<String, Vec<String>>>,
    /// Lifecycle event broadcaster
    event_tx: broadcast::Sender<LifecycleEvent>,
    /// Shared LLM client
    llm: Arc<LlmClient>,
    /// Shared tool registry
    tools: Arc<ToolRegistry>,
}

impl AgentRegistry {
    /// Create a new agent registry
    #[must_use]
    pub fn new(llm: Arc<LlmClient>, tools: Arc<ToolRegistry>) -> Self {
        let (event_tx, _) = broadcast::channel(1000);
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
            workspace_agents: RwLock::new(HashMap::new()),
            children: RwLock::new(HashMap::new()),
            event_tx,
            llm,
            tools,
        }
    }

    /// Subscribe to lifecycle events
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<LifecycleEvent> {
        self.event_tx.subscribe()
    }

    /// Broadcast a lifecycle event
    fn emit(&self, event: LifecycleEvent) {
        // Don't care if no one is listening
        let _ = self.event_tx.send(event);
    }

    // -------------------------------------------------------------------------
    // Agent Registration
    // -------------------------------------------------------------------------

    /// Register a new main agent for a workspace
    pub async fn register_main(
        &self,
        workspace: &Workspace,
        config: AgentConfig,
    ) -> String {
        let id = generate_agent_id();
        let system_prompt = workspace.system_context();

        let agent = RegisteredAgent::new(
            id.clone(),
            None,
            Some(workspace.root.clone()),
            config.clone(),
            system_prompt,
            AgentRole::Main,
        );

        // Store in registry
        {
            let mut agents = self.agents.write().await;
            agents.insert(id.clone(), agent);
        }

        // Track by workspace
        {
            let mut ws_agents = self.workspace_agents.write().await;
            ws_agents
                .entry(workspace.root.clone())
                .or_default()
                .push(id.clone());
        }

        // Emit event
        self.emit(LifecycleEvent::Spawned {
            agent_id: id.clone(),
            parent_id: None,
            workspace: Some(workspace.root.clone()),
            config: AgentMetadata {
                name: Some(workspace.name()),
                model: config.model,
                max_tokens: config.max_tokens,
                role: AgentRole::Main,
            },
        });

        info!("Registered main agent {} for workspace {}", id, workspace.name());
        id
    }

    /// Spawn a sub-agent from a parent agent
    pub async fn spawn_sub_agent(
        &self,
        parent_id: &str,
        config: AgentConfig,
        system_prompt: impl Into<String>,
        role: AgentRole,
    ) -> Result<String, AgentError> {
        let id = generate_agent_id();
        let system_prompt = system_prompt.into();

        // Get parent info
        let (workspace, _) = {
            let agents = self.agents.read().await;
            let parent = agents.get(parent_id).ok_or_else(|| {
                AgentError::Llm(nanna_llm::LlmError::MissingApiKey(format!(
                    "Parent agent not found: {parent_id}"
                )))
            })?;
            (parent.workspace.clone(), parent.config.clone())
        };

        let agent = RegisteredAgent::new(
            id.clone(),
            Some(parent_id.to_string()),
            workspace.clone(),
            config.clone(),
            system_prompt,
            role,
        );

        // Store in registry
        {
            let mut agents = self.agents.write().await;
            agents.insert(id.clone(), agent);
        }

        // Track parent-child relationship
        {
            let mut children = self.children.write().await;
            children
                .entry(parent_id.to_string())
                .or_default()
                .push(id.clone());
        }

        // Also update parent's children list
        {
            let mut agents = self.agents.write().await;
            if let Some(parent) = agents.get_mut(parent_id) {
                parent.children.push(id.clone());
            }
        }

        // Track by workspace if applicable
        if let Some(ref ws) = workspace {
            let mut ws_agents = self.workspace_agents.write().await;
            ws_agents.entry(ws.clone()).or_default().push(id.clone());
        }

        // Emit event
        self.emit(LifecycleEvent::Spawned {
            agent_id: id.clone(),
            parent_id: Some(parent_id.to_string()),
            workspace,
            config: AgentMetadata {
                name: None,
                model: config.model,
                max_tokens: config.max_tokens,
                role,
            },
        });

        info!("Spawned sub-agent {} from parent {}", id, parent_id);
        Ok(id)
    }

    /// Register a standalone agent (no workspace, no parent)
    pub async fn register_standalone(
        &self,
        config: AgentConfig,
        system_prompt: impl Into<String>,
        role: AgentRole,
    ) -> String {
        let id = generate_agent_id();
        let system_prompt = system_prompt.into();

        let agent = RegisteredAgent::new(
            id.clone(),
            None,
            None,
            config.clone(),
            system_prompt,
            role,
        );

        {
            let mut agents = self.agents.write().await;
            agents.insert(id.clone(), agent);
        }

        self.emit(LifecycleEvent::Spawned {
            agent_id: id.clone(),
            parent_id: None,
            workspace: None,
            config: AgentMetadata {
                name: None,
                model: config.model,
                max_tokens: config.max_tokens,
                role,
            },
        });

        info!("Registered standalone agent {}", id);
        id
    }

    // -------------------------------------------------------------------------
    // State Management
    // -------------------------------------------------------------------------

    /// Update agent state
    pub async fn set_state(&self, agent_id: &str, new_state: AgentState, tool_name: Option<String>) {
        let old_state = {
            let mut agents = self.agents.write().await;
            if let Some(agent) = agents.get_mut(agent_id) {
                let old = agent.state;
                agent.state = new_state;
                agent.state_changed_at = chrono_timestamp();
                agent.current_tool = tool_name.clone();
                old
            } else {
                return;
            }
        };

        self.emit(LifecycleEvent::StateChanged {
            agent_id: agent_id.to_string(),
            old_state,
            new_state,
            tool_name,
        });
    }

    /// Record tool execution start
    pub async fn tool_started(&self, agent_id: &str, tool_name: &str, tool_id: &str) {
        self.set_state(agent_id, AgentState::ToolUse, Some(tool_name.to_string()))
            .await;

        self.emit(LifecycleEvent::ToolStarted {
            agent_id: agent_id.to_string(),
            tool_name: tool_name.to_string(),
            tool_id: tool_id.to_string(),
        });
    }

    /// Record tool execution completion
    pub async fn tool_completed(
        &self,
        agent_id: &str,
        tool_name: &str,
        tool_id: &str,
        success: bool,
        duration_ms: u64,
    ) {
        self.set_state(agent_id, AgentState::Thinking, None).await;

        self.emit(LifecycleEvent::ToolCompleted {
            agent_id: agent_id.to_string(),
            tool_name: tool_name.to_string(),
            tool_id: tool_id.to_string(),
            success,
            duration_ms,
        });
    }

    /// Record agent completion
    pub async fn complete(
        &self,
        agent_id: &str,
        response: &str,
        duration_ms: u64,
        tokens_in: u32,
        tokens_out: u32,
    ) {
        {
            let mut agents = self.agents.write().await;
            if let Some(agent) = agents.get_mut(agent_id) {
                agent.state = AgentState::Completed;
                agent.state_changed_at = chrono_timestamp();
                agent.tokens_in += tokens_in;
                agent.tokens_out += tokens_out;
            }
        }

        self.emit(LifecycleEvent::Completed {
            agent_id: agent_id.to_string(),
            response: response.to_string(),
            duration_ms,
            tokens_in,
            tokens_out,
        });

        info!("Agent {} completed in {}ms", agent_id, duration_ms);
    }

    /// Record agent error
    pub async fn error(&self, agent_id: &str, error: &str, duration_ms: u64) {
        self.set_state(agent_id, AgentState::Error, None).await;

        self.emit(LifecycleEvent::Error {
            agent_id: agent_id.to_string(),
            error: error.to_string(),
            duration_ms,
        });

        warn!("Agent {} errored: {}", agent_id, error);
    }

    /// Cancel an agent
    pub async fn cancel(&self, agent_id: &str, reason: Option<String>) {
        self.set_state(agent_id, AgentState::Cancelled, None).await;

        self.emit(LifecycleEvent::Cancelled {
            agent_id: agent_id.to_string(),
            reason: reason.clone(),
        });

        info!("Agent {} cancelled: {:?}", agent_id, reason);
    }

    // -------------------------------------------------------------------------
    // Queries
    // -------------------------------------------------------------------------

    /// Get agent info by ID
    pub async fn get(&self, agent_id: &str) -> Option<RegisteredAgent> {
        self.agents.read().await.get(agent_id).cloned()
    }

    /// Get all agents
    pub async fn all(&self) -> Vec<RegisteredAgent> {
        self.agents.read().await.values().cloned().collect()
    }

    /// Get agents for a specific workspace
    pub async fn by_workspace(&self, workspace: &PathBuf) -> Vec<RegisteredAgent> {
        let ws_agents = self.workspace_agents.read().await;
        let agents = self.agents.read().await;

        ws_agents
            .get(workspace)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| agents.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get children of an agent
    pub async fn children(&self, agent_id: &str) -> Vec<RegisteredAgent> {
        let children = self.children.read().await;
        let agents = self.agents.read().await;

        children
            .get(agent_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| agents.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get all active (non-terminal) agents
    pub async fn active(&self) -> Vec<RegisteredAgent> {
        self.agents
            .read()
            .await
            .values()
            .filter(|a| !a.state.is_terminal())
            .cloned()
            .collect()
    }

    /// Get main agent for a workspace
    pub async fn main_for_workspace(&self, workspace: &PathBuf) -> Option<RegisteredAgent> {
        let ws_agents = self.workspace_agents.read().await;
        let agents = self.agents.read().await;

        ws_agents.get(workspace).and_then(|ids| {
            ids.iter()
                .filter_map(|id| agents.get(id))
                .find(|a| a.role == AgentRole::Main)
                .cloned()
        })
    }

    /// Count agents by state
    pub async fn count_by_state(&self) -> HashMap<AgentState, usize> {
        let agents = self.agents.read().await;
        let mut counts = HashMap::new();

        for agent in agents.values() {
            *counts.entry(agent.state).or_insert(0) += 1;
        }

        counts
    }

    /// Get total token usage across all agents
    pub async fn total_tokens(&self) -> (u32, u32) {
        let agents = self.agents.read().await;
        agents
            .values()
            .fold((0, 0), |(i, o), a| (i + a.tokens_in, o + a.tokens_out))
    }

    // -------------------------------------------------------------------------
    // Agent Execution
    // -------------------------------------------------------------------------

    /// Create and run an agent, tracking its lifecycle
    pub async fn run_agent(
        &self,
        agent_id: &str,
        message: &str,
        options: RunOptions,
    ) -> Result<AgentResponse, AgentError> {
        let start = std::time::Instant::now();

        // Get agent info
        let agent_info = {
            self.agents
                .read()
                .await
                .get(agent_id)
                .cloned()
                .ok_or_else(|| {
                    AgentError::Llm(nanna_llm::LlmError::MissingApiKey(format!(
                        "Agent not found: {agent_id}"
                    )))
                })?
        };

        // Update state to thinking
        self.set_state(agent_id, AgentState::Thinking, None).await;

        // Create the agent
        let context = AgentContext::new(agent_id).with_system_prompt(&agent_info.system_prompt);

        // If workspace is set, inject workspace context
        let context = if let Some(ref ws_path) = agent_info.workspace {
            if let Ok(workspace) = nanna_workspace::Workspace::load(ws_path.clone()).await {
                context.with_workspace(&workspace)
            } else {
                context
            }
        } else {
            context
        };

        let agent = Agent::new(agent_info.config.clone(), self.llm.clone(), self.tools.clone())
            .with_context(context);

        // Run agent
        match agent.run(message, options).await {
            Ok(response) => {
                let duration_ms = start.elapsed().as_millis() as u64;
                self.complete(
                    agent_id,
                    &response.text,
                    duration_ms,
                    response.input_tokens,
                    response.output_tokens,
                )
                .await;
                Ok(response)
            }
            Err(e) => {
                let duration_ms = start.elapsed().as_millis() as u64;
                self.error(agent_id, &e.to_string(), duration_ms).await;
                Err(e)
            }
        }
    }

    /// Spawn and run a sub-agent asynchronously
    pub async fn spawn_and_run(
        &self,
        parent_id: &str,
        prompt: &str,
        config: AgentConfig,
        system_prompt: impl Into<String>,
    ) -> Result<mpsc::Receiver<Result<AgentResponse, AgentError>>, AgentError> {
        let agent_id = self
            .spawn_sub_agent(parent_id, config, system_prompt, AgentRole::SubAgent)
            .await?;

        let (tx, rx) = mpsc::channel(1);
        let shared = self.shared_state();
        let prompt = prompt.to_string();

        tokio::spawn(async move {
            let result = shared.run_agent(&agent_id, &prompt, RunOptions::default()).await;
            let _ = tx.send(result).await;
        });

        Ok(rx)
    }

    /// Get shared state for spawning tasks
    fn shared_state(&self) -> SharedRegistryState {
        SharedRegistryState {
            agents: self.agents.clone(),
            event_tx: self.event_tx.clone(),
            llm: self.llm.clone(),
            tools: self.tools.clone(),
        }
    }

    // -------------------------------------------------------------------------
    // Cleanup
    // -------------------------------------------------------------------------

    /// Remove completed/errored/cancelled agents older than the given threshold
    pub async fn cleanup(&self, older_than_secs: i64) {
        let cutoff = chrono_timestamp() - older_than_secs;
        let mut to_remove = Vec::new();

        {
            let agents = self.agents.read().await;
            for (id, agent) in agents.iter() {
                if agent.state.is_terminal() && agent.state_changed_at < cutoff {
                    to_remove.push(id.clone());
                }
            }
        }

        if !to_remove.is_empty() {
            let mut agents = self.agents.write().await;
            let mut ws_agents = self.workspace_agents.write().await;
            let mut children = self.children.write().await;

            for id in &to_remove {
                if let Some(agent) = agents.remove(id) {
                    // Remove from workspace tracking
                    if let Some(ws) = &agent.workspace {
                        if let Some(ws_list) = ws_agents.get_mut(ws) {
                            ws_list.retain(|i| i != id);
                        }
                    }

                    // Remove from children tracking
                    if let Some(parent_id) = &agent.parent_id {
                        if let Some(child_list) = children.get_mut(parent_id) {
                            child_list.retain(|i| i != id);
                        }
                    }

                    // Remove this agent's children entry
                    children.remove(id);
                }
            }

            info!("Cleaned up {} completed agents", to_remove.len());
        }
    }
}

/// Shared state for spawned tasks
#[derive(Clone)]
struct SharedRegistryState {
    agents: Arc<RwLock<HashMap<String, RegisteredAgent>>>,
    event_tx: broadcast::Sender<LifecycleEvent>,
    llm: Arc<LlmClient>,
    tools: Arc<ToolRegistry>,
}

impl SharedRegistryState {
    async fn run_agent(
        &self,
        agent_id: &str,
        message: &str,
        options: RunOptions,
    ) -> Result<AgentResponse, AgentError> {
        let start = std::time::Instant::now();

        let agent_info = {
            self.agents
                .read()
                .await
                .get(agent_id)
                .cloned()
                .ok_or_else(|| {
                    AgentError::Llm(nanna_llm::LlmError::MissingApiKey(format!(
                        "Agent not found: {agent_id}"
                    )))
                })?
        };

        // Update state
        {
            let mut agents = self.agents.write().await;
            if let Some(agent) = agents.get_mut(agent_id) {
                agent.state = AgentState::Thinking;
                agent.state_changed_at = chrono_timestamp();
            }
        }

        let context = AgentContext::new(agent_id).with_system_prompt(&agent_info.system_prompt);

        let context = if let Some(ref ws_path) = agent_info.workspace {
            if let Ok(workspace) = nanna_workspace::Workspace::load(ws_path.clone()).await {
                context.with_workspace(&workspace)
            } else {
                context
            }
        } else {
            context
        };

        let agent = Agent::new(agent_info.config.clone(), self.llm.clone(), self.tools.clone())
            .with_context(context);

        match agent.run(message, options).await {
            Ok(response) => {
                let duration_ms = start.elapsed().as_millis() as u64;
                
                {
                    let mut agents = self.agents.write().await;
                    if let Some(agent) = agents.get_mut(agent_id) {
                        agent.state = AgentState::Completed;
                        agent.state_changed_at = chrono_timestamp();
                        agent.tokens_in += response.input_tokens;
                        agent.tokens_out += response.output_tokens;
                    }
                }

                let _ = self.event_tx.send(LifecycleEvent::Completed {
                    agent_id: agent_id.to_string(),
                    response: response.text.clone(),
                    duration_ms,
                    tokens_in: response.input_tokens,
                    tokens_out: response.output_tokens,
                });

                Ok(response)
            }
            Err(e) => {
                let duration_ms = start.elapsed().as_millis() as u64;

                {
                    let mut agents = self.agents.write().await;
                    if let Some(agent) = agents.get_mut(agent_id) {
                        agent.state = AgentState::Error;
                        agent.state_changed_at = chrono_timestamp();
                    }
                }

                let _ = self.event_tx.send(LifecycleEvent::Error {
                    agent_id: agent_id.to_string(),
                    error: e.to_string(),
                    duration_ms,
                });

                Err(e)
            }
        }
    }
}

fn chrono_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_state_terminal() {
        assert!(AgentState::Completed.is_terminal());
        assert!(AgentState::Error.is_terminal());
        assert!(AgentState::Cancelled.is_terminal());
        assert!(!AgentState::Thinking.is_terminal());
        assert!(!AgentState::Idle.is_terminal());
    }

    #[test]
    fn test_agent_state_active() {
        assert!(AgentState::Thinking.is_active());
        assert!(AgentState::ToolUse.is_active());
        assert!(AgentState::Waiting.is_active());
        assert!(!AgentState::Idle.is_active());
        assert!(!AgentState::Completed.is_active());
    }

    #[test]
    fn test_generate_agent_id() {
        let id1 = generate_agent_id();
        let id2 = generate_agent_id();
        assert_ne!(id1, id2);
        assert!(id1.starts_with("agent-"));
    }
}
