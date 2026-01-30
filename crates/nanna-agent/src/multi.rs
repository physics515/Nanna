//! Multi-agent coordination and background tasks

use crate::{Agent, AgentConfig, AgentContext, AgentError, AgentResponse, RunOptions};
use nanna_llm::{LlmClient, RequestBuilder};
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

// =============================================================================
// Swarm Operations
// =============================================================================

/// Result from a swarm task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmTaskResult {
    pub task_id: String,
    pub prompt: String,
    pub success: bool,
    pub result: Option<String>,
    pub error: Option<String>,
    pub duration_ms: u64,
}

/// Aggregated results from a swarm run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmResult {
    pub swarm_id: String,
    pub total_tasks: usize,
    pub successful: usize,
    pub failed: usize,
    pub results: Vec<SwarmTaskResult>,
    pub aggregated: Option<String>,
    pub duration_ms: u64,
}

/// Swarm configuration
#[derive(Debug, Clone)]
pub struct SwarmConfig {
    /// Maximum parallel tasks
    pub max_parallel: usize,
    /// Timeout per task (seconds)
    pub task_timeout_secs: u64,
    /// Whether to aggregate results with LLM
    pub aggregate_results: bool,
    /// Aggregation prompt template (use {results} placeholder)
    pub aggregation_prompt: Option<String>,
}

impl Default for SwarmConfig {
    fn default() -> Self {
        Self {
            max_parallel: 10,
            task_timeout_secs: 60,
            aggregate_results: true,
            aggregation_prompt: None,
        }
    }
}

impl AgentCoordinator {
    /// Spawn a swarm of parallel tasks and wait for completion.
    ///
    /// All tasks run in parallel (up to max_parallel), then results are
    /// optionally aggregated using the LLM.
    ///
    /// # Arguments
    /// * `agent_id` - Agent configuration to use for all tasks
    /// * `prompts` - List of prompts to execute in parallel
    /// * `config` - Swarm configuration
    ///
    /// # Returns
    /// Aggregated results from all tasks.
    pub async fn spawn_swarm(
        &self,
        agent_id: &str,
        prompts: Vec<String>,
        config: SwarmConfig,
    ) -> Result<SwarmResult, AgentError> {
        let start = std::time::Instant::now();
        let swarm_id = Uuid::new_v4().to_string();
        let total_tasks = prompts.len();
        
        info!("Starting swarm {} with {} tasks (max parallel: {})", 
              swarm_id, total_tasks, config.max_parallel);

        // Get agent configuration
        let agents = self.agents.read().await;
        let entry = agents
            .get(agent_id)
            .ok_or_else(|| AgentError::Llm(nanna_llm::LlmError::MissingApiKey(
                format!("Agent not found: {}", agent_id)
            )))?;
        let agent_config = entry.config.clone();
        let system_prompt = entry.system_prompt.clone();
        drop(agents);

        // Spawn tasks in batches
        let mut all_results = Vec::with_capacity(total_tasks);
        let task_timeout = config.task_timeout_secs;
        
        for chunk in prompts.chunks(config.max_parallel) {
            let mut handles = Vec::with_capacity(chunk.len());
            
            for prompt in chunk {
                let llm = self.llm.clone();
                let tools = self.tools.clone();
                let agent_cfg = agent_config.clone();
                let system = system_prompt.clone();
                let prompt = prompt.clone();
                let timeout = task_timeout;
                let task_id = Uuid::new_v4().to_string();
                
                let handle = tokio::spawn(async move {
                    let task_start = std::time::Instant::now();
                    
                    let context = AgentContext::new(&task_id).with_system_prompt(system);
                    let agent = Agent::new(agent_cfg, llm, tools).with_context(context);
                    
                    // Run with timeout
                    let result = tokio::time::timeout(
                        std::time::Duration::from_secs(timeout),
                        agent.run(&prompt, RunOptions::default())
                    ).await;
                    
                    let duration_ms = task_start.elapsed().as_millis() as u64;
                    
                    match result {
                        Ok(Ok(response)) => SwarmTaskResult {
                            task_id,
                            prompt,
                            success: true,
                            result: Some(response.text),
                            error: None,
                            duration_ms,
                        },
                        Ok(Err(e)) => SwarmTaskResult {
                            task_id,
                            prompt,
                            success: false,
                            result: None,
                            error: Some(e.to_string()),
                            duration_ms,
                        },
                        Err(_) => SwarmTaskResult {
                            task_id,
                            prompt,
                            success: false,
                            result: None,
                            error: Some("Task timed out".to_string()),
                            duration_ms,
                        },
                    }
                });
                
                handles.push(handle);
            }
            
            // Wait for this batch
            for handle in handles {
                if let Ok(result) = handle.await {
                    all_results.push(result);
                }
            }
        }

        let successful = all_results.iter().filter(|r| r.success).count();
        let failed = all_results.iter().filter(|r| !r.success).count();
        
        info!("Swarm {} completed: {}/{} successful", swarm_id, successful, total_tasks);

        // Optionally aggregate results
        let aggregated = if config.aggregate_results && successful > 0 {
            let results_text = all_results
                .iter()
                .filter_map(|r| r.result.as_ref())
                .enumerate()
                .map(|(i, text)| format!("--- Result {} ---\n{}", i + 1, text))
                .collect::<Vec<_>>()
                .join("\n\n");
            
            let agg_prompt = config.aggregation_prompt.unwrap_or_else(|| {
                format!(
                    "You are aggregating results from {} parallel research tasks.\n\n\
                    Synthesize these results into a coherent summary. \
                    Identify key themes, resolve contradictions, and highlight the most important findings.\n\n\
                    {}\n\n\
                    Synthesized summary:",
                    successful, results_text
                )
            });
            
            match self.llm.complete(
                &nanna_llm::CompletionRequest::default()
                    .with_model(&agent_config.model)
                    .with_message(nanna_llm::Message::user(&agg_prompt))
            ).await {
                Ok(summary) => Some(summary),
                Err(e) => {
                    debug!("Aggregation failed: {}", e);
                    None
                }
            }
        } else {
            None
        };

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(SwarmResult {
            swarm_id,
            total_tasks,
            successful,
            failed,
            results: all_results,
            aggregated,
            duration_ms,
        })
    }

    /// Quick helper to run parallel research queries and aggregate results.
    pub async fn parallel_research(
        &self,
        agent_id: &str,
        queries: Vec<String>,
    ) -> Result<String, AgentError> {
        let result = self.spawn_swarm(
            agent_id,
            queries,
            SwarmConfig {
                max_parallel: 5,
                task_timeout_secs: 30,
                aggregate_results: true,
                aggregation_prompt: None,
            },
        ).await?;

        result.aggregated.ok_or_else(|| {
            AgentError::Llm(nanna_llm::LlmError::MissingApiKey(
                "No results to aggregate".to_string()
            ))
        })
    }
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
