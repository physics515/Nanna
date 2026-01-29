//! Multi-agent coordination and background tasks

use crate::{Agent, AgentConfig, AgentContext, AgentError, AgentResponse, RunOptions};
use nanna_llm::LlmClient;
use nanna_tools::ToolRegistry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info};
use uuid::Uuid;

/// Message sent between agents or from spawner to agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub id: String,
    pub from: String,
    pub to: String,
    pub content: String,
    pub timestamp: i64,
}

/// Task status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Background task definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundTask {
    pub id: String,
    pub name: String,
    pub prompt: String,
    pub status: TaskStatus,
    pub result: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

/// Agent registry entry
#[derive(Debug, Clone)]
pub struct AgentEntry {
    pub id: String,
    pub config: AgentConfig,
    pub system_prompt: String,
}

/// Multi-agent coordinator
pub struct AgentCoordinator {
    /// Registered agent configurations
    agents: RwLock<HashMap<String, AgentEntry>>,
    /// Running background tasks
    tasks: RwLock<HashMap<String, BackgroundTask>>,
    /// Cross-agent message queues
    mailboxes: RwLock<HashMap<String, Vec<AgentMessage>>>,
    /// LLM client (shared)
    llm: Arc<LlmClient>,
    /// Tool registry (shared)
    tools: Arc<ToolRegistry>,
    /// Task completion notifier
    task_tx: mpsc::Sender<(String, Result<AgentResponse, AgentError>)>,
    task_rx: RwLock<Option<mpsc::Receiver<(String, Result<AgentResponse, AgentError>)>>>,
}

impl AgentCoordinator {
    /// Create a new coordinator.
    #[must_use]
    pub fn new(llm: Arc<LlmClient>, tools: Arc<ToolRegistry>) -> Self {
        let (tx, rx) = mpsc::channel(100);
        Self {
            agents: RwLock::new(HashMap::new()),
            tasks: RwLock::new(HashMap::new()),
            mailboxes: RwLock::new(HashMap::new()),
            llm,
            tools,
            task_tx: tx,
            task_rx: RwLock::new(Some(rx)),
        }
    }

    /// Register an agent configuration.
    pub async fn register_agent(&self, id: impl Into<String>, config: AgentConfig, system_prompt: impl Into<String>) {
        let id = id.into();
        let entry = AgentEntry {
            id: id.clone(),
            config,
            system_prompt: system_prompt.into(),
        };
        self.agents.write().await.insert(id.clone(), entry);
        self.mailboxes.write().await.insert(id.clone(), Vec::new());
        info!("Registered agent: {}", id);
    }

    /// Spawn a background task.
    ///
    /// Returns the task ID immediately. The task runs asynchronously.
    pub async fn spawn_task(
        &self,
        agent_id: &str,
        name: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Result<String, AgentError> {
        let agents = self.agents.read().await;
        let entry = agents
            .get(agent_id)
            .ok_or_else(|| AgentError::Llm(nanna_llm::LlmError::MissingApiKey(format!("Agent not found: {}", agent_id))))?;

        let task_id = Uuid::new_v4().to_string();
        let name: String = name.into();
        let prompt: String = prompt.into();
        let name_for_log = name.clone();

        let task = BackgroundTask {
            id: task_id.clone(),
            name: name.clone(),
            prompt: prompt.clone(),
            status: TaskStatus::Pending,
            result: None,
            error: None,
            created_at: chrono_timestamp(),
            completed_at: None,
        };

        self.tasks.write().await.insert(task_id.clone(), task);

        // Clone what we need for the spawned task
        let config = entry.config.clone();
        let system_prompt = entry.system_prompt.clone();
        let llm = self.llm.clone();
        let tools = self.tools.clone();
        let task_tx = self.task_tx.clone();
        let task_id_clone = task_id.clone();

        // Spawn the background task
        tokio::spawn(async move {
            debug!("Starting background task: {} ({})", name, task_id_clone);

            let context = AgentContext::new(&task_id_clone).with_system_prompt(system_prompt);
            let agent = Agent::new(config, llm, tools).with_context(context);

            let result = agent.run(&prompt, RunOptions::default()).await;

            debug!("Background task completed: {}", task_id_clone);
            let _ = task_tx.send((task_id_clone, result)).await;
        });

        // Update status to running
        if let Some(task) = self.tasks.write().await.get_mut(&task_id) {
            task.status = TaskStatus::Running;
        }

        info!("Spawned background task: {} ({})", name_for_log, task_id);
        Ok(task_id)
    }

    /// Get task status.
    pub async fn get_task(&self, task_id: &str) -> Option<BackgroundTask> {
        self.tasks.read().await.get(task_id).cloned()
    }

    /// List all tasks.
    pub async fn list_tasks(&self) -> Vec<BackgroundTask> {
        self.tasks.read().await.values().cloned().collect()
    }

    /// Cancel a task (if still running).
    pub async fn cancel_task(&self, task_id: &str) -> bool {
        if let Some(task) = self.tasks.write().await.get_mut(task_id) {
            if task.status == TaskStatus::Running || task.status == TaskStatus::Pending {
                task.status = TaskStatus::Cancelled;
                task.completed_at = Some(chrono_timestamp());
                return true;
            }
        }
        false
    }

    /// Send a message to an agent's mailbox.
    pub async fn send_message(&self, from: &str, to: &str, content: impl Into<String>) -> Result<String, AgentError> {
        let msg = AgentMessage {
            id: Uuid::new_v4().to_string(),
            from: from.to_string(),
            to: to.to_string(),
            content: content.into(),
            timestamp: chrono_timestamp(),
        };

        let msg_id = msg.id.clone();

        let mut mailboxes = self.mailboxes.write().await;
        if let Some(mailbox) = mailboxes.get_mut(to) {
            mailbox.push(msg);
            Ok(msg_id)
        } else {
            Err(AgentError::Llm(nanna_llm::LlmError::MissingApiKey(format!("Agent not found: {}", to))))
        }
    }

    /// Check mailbox for an agent.
    pub async fn check_mailbox(&self, agent_id: &str) -> Vec<AgentMessage> {
        let mut mailboxes = self.mailboxes.write().await;
        mailboxes
            .get_mut(agent_id)
            .map(std::mem::take)
            .unwrap_or_default()
    }

    /// Poll for completed tasks (non-blocking).
    pub async fn poll_completions(&self) -> Vec<(String, BackgroundTask)> {
        let mut rx_guard = self.task_rx.write().await;
        let rx = match rx_guard.as_mut() {
            Some(r) => r,
            None => return Vec::new(),
        };

        let mut completed = Vec::new();

        while let Ok((task_id, result)) = rx.try_recv() {
            if let Some(task) = self.tasks.write().await.get_mut(&task_id) {
                match result {
                    Ok(response) => {
                        task.status = TaskStatus::Completed;
                        task.result = Some(response.text);
                    }
                    Err(e) => {
                        task.status = TaskStatus::Failed;
                        task.error = Some(e.to_string());
                    }
                }
                task.completed_at = Some(chrono_timestamp());
                completed.push((task_id, task.clone()));
            }
        }

        completed
    }
}

fn chrono_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_agent_registration() {
        let llm = Arc::new(LlmClient::anthropic("test"));
        let tools = Arc::new(ToolRegistry::new());
        let coordinator = AgentCoordinator::new(llm, tools);

        coordinator
            .register_agent("test-agent", AgentConfig::default(), "Test system prompt")
            .await;

        let agents = coordinator.agents.read().await;
        assert!(agents.contains_key("test-agent"));
    }
}
