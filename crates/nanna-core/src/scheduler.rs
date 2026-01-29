//! Task scheduler for autonomous behavior
//!
//! Provides heartbeats, cron jobs, and background task execution.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{interval, Instant};
use tracing::{debug, error, info, warn};

/// Scheduled task types
#[derive(Debug, Clone)]
pub enum TaskType {
    /// Heartbeat - periodic self-check
    Heartbeat,
    /// Cron job with schedule
    Cron { schedule: String },
    /// One-shot delayed task
    Delayed { delay: Duration },
    /// Recurring task
    Recurring { interval: Duration },
}

/// A scheduled task
#[derive(Debug, Clone)]
pub struct ScheduledTask {
    pub id: String,
    pub name: String,
    pub task_type: TaskType,
    pub payload: String,
    pub enabled: bool,
    pub last_run: Option<Instant>,
    pub run_count: u64,
}

/// Task execution result
#[derive(Debug)]
pub struct TaskResult {
    pub task_id: String,
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
    pub duration_ms: u64,
}

/// Callback for task execution
pub type TaskExecutor = Arc<dyn Fn(ScheduledTask) -> std::pin::Pin<Box<dyn std::future::Future<Output = TaskResult> + Send>> + Send + Sync>;

/// Scheduler configuration
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Heartbeat interval
    pub heartbeat_interval: Duration,
    /// Whether heartbeats are enabled
    pub heartbeat_enabled: bool,
    /// Maximum concurrent tasks
    pub max_concurrent: usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            heartbeat_interval: Duration::from_secs(300), // 5 minutes
            heartbeat_enabled: true,
            max_concurrent: 4,
        }
    }
}

/// The main scheduler
pub struct Scheduler {
    config: SchedulerConfig,
    tasks: Arc<RwLock<HashMap<String, ScheduledTask>>>,
    executor: Option<TaskExecutor>,
    shutdown_tx: Option<mpsc::Sender<()>>,
}

impl Scheduler {
    pub fn new(config: SchedulerConfig) -> Self {
        Self {
            config,
            tasks: Arc::new(RwLock::new(HashMap::new())),
            executor: None,
            shutdown_tx: None,
        }
    }

    /// Set the task executor callback
    pub fn with_executor(mut self, executor: TaskExecutor) -> Self {
        self.executor = Some(executor);
        self
    }

    /// Add a task
    pub async fn add_task(&self, task: ScheduledTask) {
        let mut tasks = self.tasks.write().await;
        info!("Scheduled task: {} ({})", task.name, task.id);
        tasks.insert(task.id.clone(), task);
    }

    /// Remove a task
    pub async fn remove_task(&self, task_id: &str) -> bool {
        let mut tasks = self.tasks.write().await;
        tasks.remove(task_id).is_some()
    }

    /// Enable/disable a task
    pub async fn set_task_enabled(&self, task_id: &str, enabled: bool) {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.enabled = enabled;
        }
    }

    /// Get all tasks
    pub async fn list_tasks(&self) -> Vec<ScheduledTask> {
        let tasks = self.tasks.read().await;
        tasks.values().cloned().collect()
    }

    /// Start the scheduler
    pub async fn start(&mut self) {
        let executor = match &self.executor {
            Some(e) => e.clone(),
            None => {
                warn!("No executor set, scheduler will not run tasks");
                return;
            }
        };

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        let config = self.config.clone();
        let tasks = self.tasks.clone();

        // Spawn the scheduler loop
        tokio::spawn(async move {
            let mut heartbeat_interval = interval(config.heartbeat_interval);
            let mut check_interval = interval(Duration::from_secs(10));

            info!("Scheduler started (heartbeat every {:?})", config.heartbeat_interval);

            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        info!("Scheduler shutting down");
                        break;
                    }
                    _ = heartbeat_interval.tick() => {
                        if config.heartbeat_enabled {
                            debug!("Heartbeat tick");
                            let task = ScheduledTask {
                                id: format!("heartbeat-{}", chrono_timestamp()),
                                name: "heartbeat".to_string(),
                                task_type: TaskType::Heartbeat,
                                payload: "Check in, review state, do proactive work if needed.".to_string(),
                                enabled: true,
                                last_run: None,
                                run_count: 0,
                            };
                            let result = executor(task).await;
                            if !result.success {
                                warn!("Heartbeat failed: {:?}", result.error);
                            }
                        }
                    }
                    _ = check_interval.tick() => {
                        // Check for due tasks
                        let tasks_snapshot = {
                            let tasks = tasks.read().await;
                            tasks.values().cloned().collect::<Vec<_>>()
                        };

                        for task in tasks_snapshot {
                            if !task.enabled {
                                continue;
                            }

                            let should_run = match &task.task_type {
                                TaskType::Recurring { interval: task_interval } => {
                                    task.last_run
                                        .map(|lr| lr.elapsed() >= *task_interval)
                                        .unwrap_or(true)
                                }
                                TaskType::Delayed { delay } => {
                                    task.last_run.is_none() && task.run_count == 0
                                }
                                _ => false,
                            };

                            if should_run {
                                let executor = executor.clone();
                                let tasks = tasks.clone();
                                let task_id = task.id.clone();

                                tokio::spawn(async move {
                                    let result = executor(task).await;
                                    
                                    // Update task state
                                    let mut tasks = tasks.write().await;
                                    if let Some(t) = tasks.get_mut(&task_id) {
                                        t.last_run = Some(Instant::now());
                                        t.run_count += 1;

                                        // Disable one-shot tasks after running
                                        if matches!(t.task_type, TaskType::Delayed { .. }) {
                                            t.enabled = false;
                                        }
                                    }

                                    if !result.success {
                                        warn!("Task {} failed: {:?}", task_id, result.error);
                                    }
                                });
                            }
                        }
                    }
                }
            }
        });
    }

    /// Stop the scheduler
    pub async fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(()).await;
        }
    }
}

fn chrono_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Helper to create a heartbeat task
pub fn heartbeat_task(prompt: &str) -> ScheduledTask {
    ScheduledTask {
        id: format!("heartbeat-{}", uuid::Uuid::new_v4()),
        name: "heartbeat".to_string(),
        task_type: TaskType::Heartbeat,
        payload: prompt.to_string(),
        enabled: true,
        last_run: None,
        run_count: 0,
    }
}

/// Helper to create a recurring task
pub fn recurring_task(name: &str, interval: Duration, payload: &str) -> ScheduledTask {
    ScheduledTask {
        id: format!("{}-{}", name, uuid::Uuid::new_v4()),
        name: name.to_string(),
        task_type: TaskType::Recurring { interval },
        payload: payload.to_string(),
        enabled: true,
        last_run: None,
        run_count: 0,
    }
}

/// Helper to create a delayed one-shot task
pub fn delayed_task(name: &str, delay: Duration, payload: &str) -> ScheduledTask {
    ScheduledTask {
        id: format!("{}-{}", name, uuid::Uuid::new_v4()),
        name: name.to_string(),
        task_type: TaskType::Delayed { delay },
        payload: payload.to_string(),
        enabled: true,
        last_run: None,
        run_count: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_scheduler_add_task() {
        let scheduler = Scheduler::new(SchedulerConfig::default());
        
        let task = recurring_task("test", Duration::from_secs(60), "test payload");
        scheduler.add_task(task.clone()).await;

        let tasks = scheduler.list_tasks().await;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name, "test");
    }
}
