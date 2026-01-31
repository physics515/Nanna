//! Multi-agent coordination and background tasks

use crate::{Agent, AgentConfig, AgentContext, AgentError, AgentResponse, RunOptions, ThinkingMode};
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

/// Critical path metrics for parallel execution analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriticalPathMetrics {
    /// Total wall-clock time (actual elapsed)
    pub wall_clock_ms: u64,
    /// Sum of all task durations (if sequential)
    pub sequential_ms: u64,
    /// Duration of the longest parallel branch (critical path)
    pub critical_path_ms: u64,
    /// Parallelism efficiency (sequential / wall_clock)
    pub parallelism_ratio: f32,
    /// Number of execution levels (dependency depth)
    pub execution_levels: usize,
    /// Tasks per level
    pub tasks_per_level: Vec<usize>,
    /// Critical path task IDs (the longest chain)
    pub critical_path_tasks: Vec<String>,
}

impl CriticalPathMetrics {
    /// Calculate metrics from task results and execution levels
    pub fn calculate(
        results: &[SwarmTaskResult],
        levels: &[Vec<String>],
        wall_clock_ms: u64,
    ) -> Self {
        let sequential_ms: u64 = results.iter().map(|r| r.duration_ms).sum();
        
        // Calculate critical path: max duration at each level, then sum
        let mut level_max_durations: Vec<u64> = Vec::new();
        let mut critical_path_tasks: Vec<String> = Vec::new();
        
        for level in levels {
            let mut max_duration = 0u64;
            let mut max_task_id = String::new();
            
            for task_id in level {
                if let Some(result) = results.iter().find(|r| &r.task_id == task_id) {
                    if result.duration_ms > max_duration {
                        max_duration = result.duration_ms;
                        max_task_id = task_id.clone();
                    }
                }
            }
            
            if max_duration > 0 {
                level_max_durations.push(max_duration);
                critical_path_tasks.push(max_task_id);
            }
        }
        
        let critical_path_ms: u64 = level_max_durations.iter().sum();
        let parallelism_ratio = if wall_clock_ms > 0 {
            sequential_ms as f32 / wall_clock_ms as f32
        } else {
            1.0
        };
        
        let tasks_per_level: Vec<usize> = levels.iter().map(Vec::len).collect();
        
        Self {
            wall_clock_ms,
            sequential_ms,
            critical_path_ms,
            parallelism_ratio,
            execution_levels: levels.len(),
            tasks_per_level,
            critical_path_tasks,
        }
    }
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
    /// Critical path analysis metrics
    pub critical_path: Option<CriticalPathMetrics>,
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
    /// Context isolation mode for sub-agents
    pub context_isolation: crate::context::ContextIsolation,
    /// Enable critical path metrics calculation
    pub calculate_critical_path: bool,
    /// Thinking mode for sub-agents (extended reasoning)
    pub thinking_mode: ThinkingMode,
    /// Total context budget to distribute across sub-agents (in tokens)
    pub context_budget: Option<usize>,
}

impl Default for SwarmConfig {
    fn default() -> Self {
        Self {
            max_parallel: 10,
            task_timeout_secs: 60,
            aggregate_results: true,
            aggregation_prompt: None,
            context_isolation: crate::context::ContextIsolation::SystemOnly,
            calculate_critical_path: true,
            thinking_mode: ThinkingMode::Instant,
            context_budget: None,
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
        let swarm_thinking_mode = config.thinking_mode;
        
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
                let thinking_mode = swarm_thinking_mode;
                
                let handle = tokio::spawn(async move {
                    let task_start = std::time::Instant::now();
                    
                    let context = AgentContext::new(&task_id).with_system_prompt(system);
                    let agent = Agent::new(agent_cfg, llm, tools).with_context(context);
                    
                    // Run with timeout, passing thinking mode from swarm config
                    let run_options = RunOptions {
                        thinking_mode: Some(thinking_mode),
                        ..RunOptions::default()
                    };
                    let result = tokio::time::timeout(
                        std::time::Duration::from_secs(timeout),
                        agent.run(&prompt, run_options)
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

        // For simple swarm (no dependency levels), create single-level metrics
        let critical_path = if config.calculate_critical_path {
            let single_level = vec![all_results.iter().map(|r| r.task_id.clone()).collect()];
            Some(CriticalPathMetrics::calculate(&all_results, &single_level, duration_ms))
        } else {
            None
        };

        Ok(SwarmResult {
            swarm_id,
            total_tasks,
            successful,
            failed,
            results: all_results,
            aggregated,
            duration_ms,
            critical_path,
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
                context_isolation: crate::context::ContextIsolation::SystemOnly,
                calculate_critical_path: false,
                thinking_mode: ThinkingMode::Instant,
                context_budget: None,
            },
        ).await?;

        result.aggregated.ok_or_else(|| {
            AgentError::Llm(nanna_llm::LlmError::MissingApiKey(
                "No results to aggregate".to_string()
            ))
        })
    }
}

// =============================================================================
// Swarm Coordinator - Task Decomposition & Dynamic Agent Spawning
// =============================================================================

/// Subtask generated by task decomposition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subtask {
    pub id: String,
    pub description: String,
    pub domain: String,
    pub dependencies: Vec<String>,
    pub priority: u8,
}

/// Task decomposition result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecomposedTask {
    pub original_task: String,
    pub subtasks: Vec<Subtask>,
    pub execution_plan: String,
}

/// Domain-specific agent configuration
#[derive(Debug, Clone)]
pub struct DomainAgent {
    pub domain: String,
    pub system_prompt: String,
    pub tools: Vec<String>,
}

impl Default for DomainAgent {
    fn default() -> Self {
        Self {
            domain: "general".to_string(),
            system_prompt: "You are a helpful assistant.".to_string(),
            tools: vec![],
        }
    }
}

/// Swarm Coordinator - orchestrates complex tasks via decomposition and parallel execution
pub struct SwarmCoordinator {
    coordinator: Arc<AgentCoordinator>,
    domain_agents: RwLock<HashMap<String, DomainAgent>>,
    decomposition_model: String,
}

impl SwarmCoordinator {
    /// Create a new swarm coordinator.
    pub fn new(coordinator: Arc<AgentCoordinator>, decomposition_model: impl Into<String>) -> Self {
        let mut domain_agents = HashMap::new();
        
        // Register default domain agents
        domain_agents.insert("research".to_string(), DomainAgent {
            domain: "research".to_string(),
            system_prompt: "You are a research specialist. Find and synthesize information accurately. Cite sources when possible.".to_string(),
            tools: vec!["web_search".to_string(), "web_fetch".to_string()],
        });
        
        domain_agents.insert("code".to_string(), DomainAgent {
            domain: "code".to_string(),
            system_prompt: "You are a coding specialist. Write clean, efficient, well-documented code. Follow best practices.".to_string(),
            tools: vec!["read_file".to_string(), "write_file".to_string(), "exec".to_string()],
        });
        
        domain_agents.insert("analysis".to_string(), DomainAgent {
            domain: "analysis".to_string(),
            system_prompt: "You are a data analysis specialist. Analyze information critically, identify patterns, and provide insights.".to_string(),
            tools: vec!["read_file".to_string()],
        });
        
        domain_agents.insert("writing".to_string(), DomainAgent {
            domain: "writing".to_string(),
            system_prompt: "You are a writing specialist. Create clear, engaging, well-structured content.".to_string(),
            tools: vec!["write_file".to_string()],
        });

        domain_agents.insert("vision".to_string(), DomainAgent {
            domain: "vision".to_string(),
            system_prompt: "You are a vision specialist. Analyze images, extract text via OCR, and describe visual content.".to_string(),
            tools: vec!["analyze_image".to_string(), "ocr".to_string(), "describe_image".to_string()],
        });

        domain_agents.insert("general".to_string(), DomainAgent::default());

        Self {
            coordinator,
            domain_agents: RwLock::new(domain_agents),
            decomposition_model: decomposition_model.into(),
        }
    }

    /// Register a custom domain agent.
    pub async fn register_domain(&self, agent: DomainAgent) {
        self.domain_agents.write().await.insert(agent.domain.clone(), agent);
    }

    /// Decompose a complex task into subtasks using LLM.
    pub async fn decompose_task(&self, task: &str) -> Result<DecomposedTask, AgentError> {
        let decomposition_prompt = format!(
            r#"You are a task decomposition specialist. Break down the following task into smaller, independent subtasks that can be executed in parallel where possible.

TASK: {}

Analyze this task and output a JSON response with this exact structure:
{{
    "subtasks": [
        {{
            "id": "task_1",
            "description": "Description of what this subtask should accomplish",
            "domain": "research|code|analysis|writing|vision|general",
            "dependencies": ["task_id_this_depends_on"],
            "priority": 1
        }}
    ],
    "execution_plan": "Brief description of how these subtasks should be coordinated"
}}

Rules:
1. Each subtask should be self-contained and clearly scoped
2. Use "dependencies" only when a subtask MUST wait for another's output
3. Prefer parallel execution - minimize dependencies when possible
4. Domain should match the type of work: research (web search), code (programming), analysis (data), writing (content), vision (images), general (other)
5. Priority 1 is highest, 5 is lowest
6. Keep subtasks focused - each should take 1-3 minutes max
7. Aim for 2-6 subtasks for most tasks

Output ONLY valid JSON, no markdown or explanation."#,
            task
        );

        let llm = &self.coordinator.llm;
        let response = llm.complete(
            &nanna_llm::CompletionRequest::default()
                .with_model(&self.decomposition_model)
                .with_message(nanna_llm::Message::user(&decomposition_prompt))
        ).await.map_err(AgentError::Llm)?;

        // Parse the JSON response
        let parsed: serde_json::Value = serde_json::from_str(&response)
            .map_err(|e| AgentError::Llm(nanna_llm::LlmError::Stream(
                format!("Failed to parse decomposition: {}", e)
            )))?;

        let subtasks: Vec<Subtask> = parsed.get("subtasks")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let execution_plan = parsed.get("execution_plan")
            .and_then(|v| v.as_str())
            .unwrap_or("Execute subtasks based on dependencies")
            .to_string();

        Ok(DecomposedTask {
            original_task: task.to_string(),
            subtasks,
            execution_plan,
        })
    }

    /// Execute a complex task by decomposing it and running subtasks in parallel.
    pub async fn execute_task(&self, task: &str, config: SwarmConfig) -> Result<SwarmResult, AgentError> {
        let start = std::time::Instant::now();
        let swarm_id = Uuid::new_v4().to_string();

        info!("Swarm {} starting task decomposition", swarm_id);

        // Step 1: Decompose the task
        let decomposed = self.decompose_task(task).await?;
        
        info!("Swarm {} decomposed into {} subtasks", swarm_id, decomposed.subtasks.len());

        if decomposed.subtasks.is_empty() {
            // No decomposition needed - run as single task
            return self.coordinator.spawn_swarm(
                "general",
                vec![task.to_string()],
                config,
            ).await;
        }

        // Step 2: Group subtasks by dependency level
        let execution_levels = self.build_execution_levels(&decomposed.subtasks);

        // Step 3: Execute each level in parallel
        let mut all_results = Vec::new();
        let mut context: HashMap<String, String> = HashMap::new();

        for (level, subtask_ids) in execution_levels.iter().enumerate() {
            info!("Swarm {} executing level {} with {} tasks", swarm_id, level, subtask_ids.len());

            let subtasks: Vec<&Subtask> = decomposed.subtasks
                .iter()
                .filter(|s| subtask_ids.contains(&s.id))
                .collect();

            // Build prompts with context from completed dependencies
            let prompts: Vec<String> = subtasks.iter().map(|subtask| {
                let mut prompt = subtask.description.clone();
                
                // Add context from dependencies
                for dep_id in &subtask.dependencies {
                    if let Some(dep_result) = context.get(dep_id) {
                        prompt = format!(
                            "{}\n\n--- Context from previous task ({}) ---\n{}",
                            prompt, dep_id, dep_result
                        );
                    }
                }
                
                prompt
            }).collect();

            // Determine which agent to use (use first subtask's domain)
            let domain = subtasks.first().map(|s| s.domain.as_str()).unwrap_or("general");
            
            // Ensure agent is registered
            self.ensure_agent_registered(domain).await;

            // Run this level's tasks in parallel
            let level_result = self.coordinator.spawn_swarm(
                domain,
                prompts,
                SwarmConfig {
                    aggregate_results: false, // Don't aggregate intermediate levels
                    ..config.clone()
                },
            ).await?;

            // Store results for dependent tasks
            for (subtask, result) in subtasks.iter().zip(level_result.results.iter()) {
                if let Some(ref text) = result.result {
                    context.insert(subtask.id.clone(), text.clone());
                }
            }

            all_results.extend(level_result.results);
        }

        // Step 4: Aggregate final results
        let successful = all_results.iter().filter(|r| r.success).count();
        let failed = all_results.iter().filter(|r| !r.success).count();

        let aggregated = if config.aggregate_results && successful > 0 {
            let results_text = all_results
                .iter()
                .filter_map(|r| r.result.as_ref())
                .enumerate()
                .map(|(i, text)| format!("--- Subtask {} Result ---\n{}", i + 1, text))
                .collect::<Vec<_>>()
                .join("\n\n");

            let agg_prompt = format!(
                r#"You completed a complex task by breaking it into subtasks. Here are the results:

ORIGINAL TASK: {}

EXECUTION PLAN: {}

SUBTASK RESULTS:
{}

Synthesize these results into a coherent final response that addresses the original task.
Be comprehensive but concise. Highlight key findings and conclusions."#,
                decomposed.original_task,
                decomposed.execution_plan,
                results_text
            );

            match self.coordinator.llm.complete(
                &nanna_llm::CompletionRequest::default()
                    .with_model(&self.decomposition_model)
                    .with_message(nanna_llm::Message::user(&agg_prompt))
            ).await {
                Ok(summary) => Some(summary),
                Err(e) => {
                    debug!("Final aggregation failed: {}", e);
                    None
                }
            }
        } else {
            None
        };

        let duration_ms = start.elapsed().as_millis() as u64;

        // Calculate critical path metrics using actual execution levels
        let critical_path = if config.calculate_critical_path {
            Some(CriticalPathMetrics::calculate(&all_results, &execution_levels, duration_ms))
        } else {
            None
        };

        info!("Swarm {} completed: {}/{} successful in {}ms (critical path: {}ms, parallelism: {:.2}x)", 
              swarm_id, successful, all_results.len(), duration_ms,
              critical_path.as_ref().map_or(0, |cp| cp.critical_path_ms),
              critical_path.as_ref().map_or(1.0, |cp| cp.parallelism_ratio));

        Ok(SwarmResult {
            swarm_id,
            total_tasks: all_results.len(),
            successful,
            failed,
            results: all_results,
            aggregated,
            duration_ms,
            critical_path,
        })
    }

    /// Build execution levels based on dependencies (topological sort).
    fn build_execution_levels(&self, subtasks: &[Subtask]) -> Vec<Vec<String>> {
        let mut levels: Vec<Vec<String>> = Vec::new();
        let mut completed: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut remaining: Vec<&Subtask> = subtasks.iter().collect();

        while !remaining.is_empty() {
            let mut current_level = Vec::new();

            // Find all tasks whose dependencies are satisfied
            remaining.retain(|task| {
                let deps_satisfied = task.dependencies.iter().all(|d| completed.contains(d));
                if deps_satisfied {
                    current_level.push(task.id.clone());
                    false // Remove from remaining
                } else {
                    true // Keep in remaining
                }
            });

            if current_level.is_empty() && !remaining.is_empty() {
                // Circular dependency or missing dependency - just add remaining
                for task in &remaining {
                    current_level.push(task.id.clone());
                }
                remaining.clear();
            }

            for id in &current_level {
                completed.insert(id.clone());
            }

            if !current_level.is_empty() {
                levels.push(current_level);
            }
        }

        levels
    }

    /// Ensure a domain agent is registered with the coordinator.
    async fn ensure_agent_registered(&self, domain: &str) {
        let agents = self.coordinator.agents.read().await;
        if agents.contains_key(domain) {
            return;
        }
        drop(agents);

        let domain_agents = self.domain_agents.read().await;
        let domain_config = domain_agents.get(domain).cloned().unwrap_or_default();
        drop(domain_agents);

        self.coordinator.register_agent(
            domain,
            AgentConfig::default(),
            &domain_config.system_prompt,
        ).await;
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

    #[test]
    fn test_execution_levels_no_deps() {
        let subtasks = vec![
            Subtask { id: "a".to_string(), description: "A".to_string(), domain: "general".to_string(), dependencies: vec![], priority: 1 },
            Subtask { id: "b".to_string(), description: "B".to_string(), domain: "general".to_string(), dependencies: vec![], priority: 1 },
            Subtask { id: "c".to_string(), description: "C".to_string(), domain: "general".to_string(), dependencies: vec![], priority: 1 },
        ];

        let llm = std::sync::Arc::new(LlmClient::anthropic("test"));
        let tools = std::sync::Arc::new(ToolRegistry::new());
        let coordinator = std::sync::Arc::new(AgentCoordinator::new(llm, tools));
        let swarm = SwarmCoordinator::new(coordinator, "test");

        let levels = swarm.build_execution_levels(&subtasks);
        assert_eq!(levels.len(), 1);
        assert_eq!(levels[0].len(), 3);
    }

    #[test]
    fn test_execution_levels_with_deps() {
        let subtasks = vec![
            Subtask { id: "a".to_string(), description: "A".to_string(), domain: "general".to_string(), dependencies: vec![], priority: 1 },
            Subtask { id: "b".to_string(), description: "B".to_string(), domain: "general".to_string(), dependencies: vec!["a".to_string()], priority: 1 },
            Subtask { id: "c".to_string(), description: "C".to_string(), domain: "general".to_string(), dependencies: vec!["b".to_string()], priority: 1 },
        ];

        let llm = std::sync::Arc::new(LlmClient::anthropic("test"));
        let tools = std::sync::Arc::new(ToolRegistry::new());
        let coordinator = std::sync::Arc::new(AgentCoordinator::new(llm, tools));
        let swarm = SwarmCoordinator::new(coordinator, "test");

        let levels = swarm.build_execution_levels(&subtasks);
        assert_eq!(levels.len(), 3);
        assert_eq!(levels[0], vec!["a"]);
        assert_eq!(levels[1], vec!["b"]);
        assert_eq!(levels[2], vec!["c"]);
    }
}
