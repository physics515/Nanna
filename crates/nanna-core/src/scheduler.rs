//! Task scheduler for autonomous behavior
//!
//! Provides heartbeats, cron jobs, and background task execution.
//! Supports persistent storage of cron jobs via nanna-storage.

use nanna_storage::{NewCronJob, Storage};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{interval, Instant};
use tracing::{debug, info, warn};

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
    storage: Option<Arc<Storage>>,
}

impl Scheduler {
    #[must_use] 
    pub fn new(config: SchedulerConfig) -> Self {
        Self {
            config,
            tasks: Arc::new(RwLock::new(HashMap::new())),
            executor: None,
            shutdown_tx: None,
            storage: None,
        }
    }

    /// Set persistent storage for cron jobs.
    #[must_use]
    pub fn with_storage(mut self, storage: Arc<Storage>) -> Self {
        self.storage = Some(storage);
        self
    }

    /// Load persisted cron jobs from storage.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage query fails.
    pub async fn load_jobs(&self) -> Result<usize, nanna_storage::StorageError> {
        let storage = match &self.storage {
            Some(s) => s,
            None => return Ok(0),
        };

        let jobs = storage.cron_jobs().list_enabled().await?;
        let count = jobs.len();

        let mut tasks = self.tasks.write().await;
        for job in jobs {
            let task_type = if job.schedule == "heartbeat" {
                TaskType::Heartbeat
            } else if let Some(interval_secs) = parse_interval(&job.schedule) {
                TaskType::Recurring {
                    interval: Duration::from_secs(interval_secs),
                }
            } else {
                TaskType::Cron {
                    schedule: job.schedule.clone(),
                }
            };

            let payload = job.task.get("payload")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let task = ScheduledTask {
                id: job.job_id.clone(),
                name: job.task.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&job.job_id)
                    .to_string(),
                task_type,
                payload,
                enabled: job.enabled,
                last_run: None,
                run_count: 0,
            };

            tasks.insert(job.job_id, task);
        }

        info!("Loaded {} cron jobs from storage", count);
        Ok(count)
    }

    /// Save a task to persistent storage.
    async fn persist_task(&self, task: &ScheduledTask) -> Result<(), nanna_storage::StorageError> {
        let storage = match &self.storage {
            Some(s) => s,
            None => return Ok(()),
        };

        let schedule = match &task.task_type {
            TaskType::Heartbeat => "heartbeat".to_string(),
            TaskType::Cron { schedule } => schedule.clone(),
            TaskType::Recurring { interval } => format!("every_{}s", interval.as_secs()),
            TaskType::Delayed { delay } => format!("delay_{}s", delay.as_secs()),
        };

        let job = NewCronJob {
            job_id: task.id.clone(),
            schedule,
            task: serde_json::json!({
                "name": task.name,
                "payload": task.payload,
            }),
            enabled: task.enabled,
            next_run: None,
            metadata: None,
        };

        storage.cron_jobs().create(job).await?;
        Ok(())
    }

    /// Set the task executor callback (builder pattern).
    #[must_use]
    pub fn with_executor(mut self, executor: TaskExecutor) -> Self {
        self.executor = Some(executor);
        self
    }

    /// Set the task executor callback (mutable reference).
    pub fn set_executor(&mut self, executor: TaskExecutor) {
        self.executor = Some(executor);
    }

    /// Add a task (persisted if storage is configured)
    pub async fn add_task(&self, task: ScheduledTask) {
        // Persist first if storage is available
        if let Err(e) = self.persist_task(&task).await {
            warn!("Failed to persist task {}: {}", task.id, e);
        }

        let mut tasks = self.tasks.write().await;
        info!("Scheduled task: {} ({})", task.name, task.id);
        tasks.insert(task.id.clone(), task);
    }

    /// Remove a task (from memory and storage)
    pub async fn remove_task(&self, task_id: &str) -> bool {
        // Remove from storage
        if let Some(storage) = &self.storage {
            if let Err(e) = storage.cron_jobs().delete(task_id).await {
                warn!("Failed to delete task {} from storage: {}", task_id, e);
            }
        }

        let mut tasks = self.tasks.write().await;
        tasks.remove(task_id).is_some()
    }

    /// Enable/disable a task (persisted)
    pub async fn set_task_enabled(&self, task_id: &str, enabled: bool) {
        // Update storage
        if let Some(storage) = &self.storage {
            if let Err(e) = storage.cron_jobs().set_enabled(task_id, enabled).await {
                warn!("Failed to update task {} enabled state: {}", task_id, e);
            }
        }

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

    /// Start the scheduler.
    pub fn start(&mut self) {
        let executor = if let Some(e) = &self.executor { e.clone() } else {
            warn!("No executor set, scheduler will not run tasks");
            return;
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
                                        .is_none_or(|lr| lr.elapsed() >= *task_interval)
                                }
                                TaskType::Delayed { delay: _ } => {
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

                                    // Update task state (scope the lock)
                                    {
                                        let mut tasks_guard = tasks.write().await;
                                        if let Some(t) = tasks_guard.get_mut(&task_id) {
                                            t.last_run = Some(Instant::now());
                                            t.run_count += 1;

                                            // Disable one-shot tasks after running
                                            if matches!(t.task_type, TaskType::Delayed { .. }) {
                                                t.enabled = false;
                                            }
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
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Helper to create a heartbeat task
#[must_use] 
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
#[must_use] 
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
#[must_use] 
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

/// Helper to create a memory consolidation ("dreaming") task
/// 
/// Runs periodically to consolidate memories based on weight/importance.
/// Default interval is 1 hour.
#[must_use]
pub fn consolidation_task(interval: Option<Duration>) -> ScheduledTask {
    let interval = interval.unwrap_or(Duration::from_secs(3600)); // 1 hour default
    ScheduledTask {
        id: format!("consolidation-{}", uuid::Uuid::new_v4()),
        name: "memory_consolidation".to_string(),
        task_type: TaskType::Recurring { interval },
        payload: "Run memory consolidation (dreaming): compress fading memories, expand important ones.".to_string(),
        enabled: true,
        last_run: None,
        run_count: 0,
    }
}

/// Task type marker for the dreaming task
pub const DREAMING_TASK_NAME: &str = "memory_consolidation";

/// Check if a task is the dreaming/consolidation task
#[must_use]
pub fn is_dreaming_task(task: &ScheduledTask) -> bool {
    task.name == DREAMING_TASK_NAME
}

/// Parse interval string like "every_300s" into seconds
fn parse_interval(schedule: &str) -> Option<u64> {
    if schedule.starts_with("every_") && schedule.ends_with('s') {
        let num_str = &schedule[6..schedule.len() - 1];
        num_str.parse().ok()
    } else {
        None
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
