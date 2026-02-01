//! Task scheduler for autonomous behavior
//!
//! Provides heartbeats, cron jobs, and background task execution.
//! Supports persistent storage of cron jobs via nanna-storage.

use crate::cron::{CronError, CronExpr};
use chrono::{DateTime, Utc};
use nanna_storage::{NewCronJob, Storage};
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
    /// Cron job with parsed schedule
    Cron {
        schedule: String,
        parsed: Option<CronExpr>,
        next_run: Option<DateTime<Utc>>,
    },
    /// One-shot delayed task
    Delayed { delay: Duration, created: Instant },
    /// Recurring task at fixed interval
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
    pub last_run: Option<DateTime<Utc>>,
    pub run_count: u64,
    /// Timezone for cron evaluation (default: UTC)
    pub timezone: String,
    /// Channel to send results to (optional)
    pub target_channel: Option<String>,
    /// Session to run in (optional)
    pub target_session: Option<String>,
}

/// Task execution result
#[derive(Debug, Clone)]
pub struct TaskResult {
    pub task_id: String,
    pub task_name: String,
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
}

/// Job run history entry
#[derive(Debug, Clone)]
pub struct JobRun {
    pub id: i64,
    pub job_id: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
}

/// Callback for task execution
pub type TaskExecutor = Arc<
    dyn Fn(ScheduledTask) -> std::pin::Pin<Box<dyn std::future::Future<Output = TaskResult> + Send>>
        + Send
        + Sync,
>;

/// Scheduler configuration
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Heartbeat interval
    pub heartbeat_interval: Duration,
    /// Whether heartbeats are enabled
    pub heartbeat_enabled: bool,
    /// Heartbeat prompt/payload
    pub heartbeat_prompt: String,
    /// Maximum concurrent tasks
    pub max_concurrent: usize,
    /// Check interval for cron jobs (how often to check for due jobs)
    pub check_interval: Duration,
    /// Default timezone for cron expressions
    pub default_timezone: String,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            heartbeat_interval: Duration::from_secs(1800), // 30 minutes
            heartbeat_enabled: true,
            heartbeat_prompt: "Read HEARTBEAT.md if it exists. Follow it strictly. If nothing needs attention, reply HEARTBEAT_OK.".to_string(),
            max_concurrent: 4,
            check_interval: Duration::from_secs(30),
            default_timezone: "UTC".to_string(),
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
    /// Job run history (in-memory, last N runs per job)
    history: Arc<RwLock<HashMap<String, Vec<JobRun>>>>,
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
            history: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Set persistent storage for cron jobs.
    #[must_use]
    pub fn with_storage(mut self, storage: Arc<Storage>) -> Self {
        self.storage = Some(storage);
        self
    }

    /// Load persisted cron jobs from storage.
    pub async fn load_jobs(&self) -> Result<usize, nanna_storage::StorageError> {
        let storage = match &self.storage {
            Some(s) => s,
            None => return Ok(0),
        };

        let jobs = storage.cron_jobs().list_all().await?;
        let count = jobs.len();

        let mut tasks = self.tasks.write().await;
        for job in jobs {
            let task = self.job_to_task(&job);
            tasks.insert(job.job_id, task);
        }

        info!("Loaded {} cron jobs from storage", count);
        Ok(count)
    }

    /// Convert storage job to scheduler task
    fn job_to_task(&self, job: &nanna_storage::CronJob) -> ScheduledTask {
        let task_type = if job.schedule == "heartbeat" {
            TaskType::Heartbeat
        } else if let Some(interval_secs) = parse_interval(&job.schedule) {
            TaskType::Recurring {
                interval: Duration::from_secs(interval_secs),
            }
        } else if let Some(delay_secs) = parse_delay(&job.schedule) {
            TaskType::Delayed {
                delay: Duration::from_secs(delay_secs),
                created: Instant::now(),
            }
        } else {
            // Try to parse as cron expression
            let parsed = CronExpr::parse(&job.schedule).ok();
            let next_run = parsed.as_ref().and_then(|p| p.next_from_now());
            TaskType::Cron {
                schedule: job.schedule.clone(),
                parsed,
                next_run,
            }
        };

        let payload = job
            .task
            .get("payload")
            .and_then(|v| v.as_str())
            .or_else(|| job.task.get("text").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();

        let name = job
            .task
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&job.job_id)
            .to_string();

        let target_channel = job
            .task
            .get("channel")
            .and_then(|v| v.as_str())
            .map(String::from);

        let target_session = job
            .task
            .get("session")
            .and_then(|v| v.as_str())
            .map(String::from);

        let timezone = job
            .task
            .get("timezone")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.config.default_timezone)
            .to_string();

        ScheduledTask {
            id: job.job_id.clone(),
            name,
            task_type,
            payload,
            enabled: job.enabled,
            last_run: job.last_run.as_ref().and_then(|s| s.parse().ok()),
            run_count: 0,
            timezone,
            target_channel,
            target_session,
        }
    }

    /// Save a task to persistent storage.
    async fn persist_task(&self, task: &ScheduledTask) -> Result<(), nanna_storage::StorageError> {
        let storage = match &self.storage {
            Some(s) => s,
            None => return Ok(()),
        };

        let schedule = match &task.task_type {
            TaskType::Heartbeat => "heartbeat".to_string(),
            TaskType::Cron { schedule, .. } => schedule.clone(),
            TaskType::Recurring { interval } => format!("every_{}s", interval.as_secs()),
            TaskType::Delayed { delay, .. } => format!("delay_{}s", delay.as_secs()),
        };

        let next_run = match &task.task_type {
            TaskType::Cron { next_run, .. } => next_run.map(|dt| dt.to_rfc3339()),
            _ => None,
        };

        let job = NewCronJob {
            job_id: task.id.clone(),
            schedule,
            task: serde_json::json!({
                "name": task.name,
                "payload": task.payload,
                "timezone": task.timezone,
                "channel": task.target_channel,
                "session": task.target_session,
            }),
            enabled: task.enabled,
            next_run,
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

    /// Create a cron task from a schedule expression
    pub fn cron_task(
        name: &str,
        schedule: &str,
        payload: &str,
    ) -> Result<ScheduledTask, CronError> {
        let parsed = CronExpr::parse(schedule)?;
        let next_run = parsed.next_from_now();

        Ok(ScheduledTask {
            id: format!("{}-{}", name, uuid::Uuid::new_v4()),
            name: name.to_string(),
            task_type: TaskType::Cron {
                schedule: schedule.to_string(),
                parsed: Some(parsed),
                next_run,
            },
            payload: payload.to_string(),
            enabled: true,
            last_run: None,
            run_count: 0,
            timezone: "UTC".to_string(),
            target_channel: None,
            target_session: None,
        })
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

    /// Update a task's schedule
    pub async fn update_schedule(
        &self,
        task_id: &str,
        schedule: &str,
    ) -> Result<bool, CronError> {
        let parsed = CronExpr::parse(schedule)?;
        let next_run = parsed.next_from_now();

        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.task_type = TaskType::Cron {
                schedule: schedule.to_string(),
                parsed: Some(parsed),
                next_run,
            };

            // Update storage
            if let Some(storage) = &self.storage {
                let next_run_str = next_run.map(|dt| dt.to_rfc3339());
                let _ = storage
                    .cron_jobs()
                    .update_last_run(task_id, "", next_run_str.as_deref())
                    .await;
            }

            Ok(true)
        } else {
            Ok(false)
        }
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

    /// Get a specific task
    pub async fn get_task(&self, task_id: &str) -> Option<ScheduledTask> {
        let tasks = self.tasks.read().await;
        tasks.get(task_id).cloned()
    }

    /// Get job run history
    pub async fn get_history(&self, job_id: &str, limit: usize) -> Vec<JobRun> {
        let history = self.history.read().await;
        history
            .get(job_id)
            .map(|runs| runs.iter().rev().take(limit).cloned().collect())
            .unwrap_or_default()
    }

    /// Record a job run
    async fn record_run(&self, result: &TaskResult) {
        let mut history = self.history.write().await;
        let runs = history.entry(result.task_id.clone()).or_default();

        runs.push(JobRun {
            id: runs.len() as i64 + 1,
            job_id: result.task_id.clone(),
            started_at: result.started_at,
            finished_at: Some(result.finished_at),
            success: result.success,
            output: result.output.clone(),
            error: result.error.clone(),
        });

        // Keep only last 100 runs per job
        if runs.len() > 100 {
            runs.remove(0);
        }
    }

    /// Run a task immediately (bypass schedule)
    pub async fn run_now(&self, task_id: &str) -> Option<TaskResult> {
        let executor = self.executor.as_ref()?;
        let task = {
            let tasks = self.tasks.read().await;
            tasks.get(task_id).cloned()?
        };

        let result = executor(task).await;

        // Record the run
        self.record_run(&result).await;

        // Update last_run
        {
            let mut tasks = self.tasks.write().await;
            if let Some(t) = tasks.get_mut(task_id) {
                t.last_run = Some(result.finished_at);
                t.run_count += 1;

                // Update next_run for cron tasks
                if let TaskType::Cron {
                    parsed: Some(ref p),
                    ref mut next_run,
                    ..
                } = t.task_type
                {
                    *next_run = p.next_from_now();
                }
            }
        }

        Some(result)
    }

    /// Start the scheduler.
    pub fn start(&mut self) {
        let executor = if let Some(e) = &self.executor {
            e.clone()
        } else {
            warn!("No executor set, scheduler will not run tasks");
            return;
        };

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        let config = self.config.clone();
        let tasks = self.tasks.clone();
        let storage = self.storage.clone();
        let history = self.history.clone();

        // Spawn the scheduler loop
        tokio::spawn(async move {
            let mut heartbeat_interval = interval(config.heartbeat_interval);
            let mut check_interval = interval(config.check_interval);

            info!(
                "Scheduler started (heartbeat every {:?}, check every {:?})",
                config.heartbeat_interval, config.check_interval
            );

            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        info!("Scheduler shutting down");
                        break;
                    }
                    _ = heartbeat_interval.tick() => {
                        if config.heartbeat_enabled {
                            debug!("Heartbeat tick");
                            let task = heartbeat_task(&config.heartbeat_prompt);
                            let result = executor(task).await;

                            // Record heartbeat run
                            {
                                let mut hist = history.write().await;
                                let runs = hist.entry("heartbeat".to_string()).or_default();
                                runs.push(JobRun {
                                    id: runs.len() as i64 + 1,
                                    job_id: "heartbeat".to_string(),
                                    started_at: result.started_at,
                                    finished_at: Some(result.finished_at),
                                    success: result.success,
                                    output: result.output.clone(),
                                    error: result.error.clone(),
                                });
                                if runs.len() > 100 {
                                    runs.remove(0);
                                }
                            }

                            if !result.success {
                                warn!("Heartbeat failed: {:?}", result.error);
                            }
                        }
                    }
                    _ = check_interval.tick() => {
                        // Check for due tasks
                        let now = Utc::now();
                        let tasks_snapshot = {
                            let tasks = tasks.read().await;
                            tasks.values().cloned().collect::<Vec<_>>()
                        };

                        for task in tasks_snapshot {
                            if !task.enabled {
                                continue;
                            }

                            let should_run = match &task.task_type {
                                TaskType::Cron { next_run, .. } => {
                                    next_run.is_some_and(|nr| nr <= now)
                                }
                                TaskType::Recurring { interval: task_interval } => {
                                    task.last_run.is_none_or(|lr| {
                                        let elapsed = now.signed_duration_since(lr);
                                        elapsed >= chrono::Duration::from_std(*task_interval).unwrap_or_default()
                                    })
                                }
                                TaskType::Delayed { delay, created } => {
                                    task.run_count == 0 && created.elapsed() >= *delay
                                }
                                TaskType::Heartbeat => false, // Handled separately
                            };

                            if should_run {
                                let executor = executor.clone();
                                let tasks = tasks.clone();
                                let storage = storage.clone();
                                let history = history.clone();
                                let task_id = task.id.clone();

                                tokio::spawn(async move {
                                    debug!("Running task: {} ({})", task.name, task_id);
                                    let result = executor(task).await;

                                    // Record run in history
                                    {
                                        let mut hist = history.write().await;
                                        let runs = hist.entry(task_id.clone()).or_default();
                                        runs.push(JobRun {
                                            id: runs.len() as i64 + 1,
                                            job_id: task_id.clone(),
                                            started_at: result.started_at,
                                            finished_at: Some(result.finished_at),
                                            success: result.success,
                                            output: result.output.clone(),
                                            error: result.error.clone(),
                                        });
                                        if runs.len() > 100 {
                                            runs.remove(0);
                                        }
                                    }

                                    // Update task state
                                    {
                                        let mut tasks_guard = tasks.write().await;
                                        if let Some(t) = tasks_guard.get_mut(&task_id) {
                                            t.last_run = Some(result.finished_at);
                                            t.run_count += 1;

                                            // Update next_run for cron tasks
                                            if let TaskType::Cron {
                                                parsed: Some(ref p),
                                                ref mut next_run,
                                                ..
                                            } = t.task_type
                                            {
                                                *next_run = p.next_from_now();
                                            }

                                            // Disable one-shot tasks after running
                                            if matches!(t.task_type, TaskType::Delayed { .. }) {
                                                t.enabled = false;
                                            }
                                        }
                                    }

                                    // Update storage
                                    if let Some(storage) = storage {
                                        let next_run = {
                                            let tasks = tasks.read().await;
                                            tasks.get(&task_id).and_then(|t| {
                                                if let TaskType::Cron { next_run, .. } = &t.task_type {
                                                    next_run.map(|dt| dt.to_rfc3339())
                                                } else {
                                                    None
                                                }
                                            })
                                        };

                                        let _ = storage.cron_jobs().update_last_run(
                                            &task_id,
                                            &result.finished_at.to_rfc3339(),
                                            next_run.as_deref(),
                                        ).await;
                                    }

                                    if !result.success {
                                        error!("Task {} failed: {:?}", task_id, result.error);
                                    } else {
                                        info!("Task {} completed in {}ms", task_id, result.duration_ms);
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

/// Parse interval string like "every_300s" into seconds
fn parse_interval(schedule: &str) -> Option<u64> {
    if schedule.starts_with("every_") && schedule.ends_with('s') {
        let num_str = &schedule[6..schedule.len() - 1];
        num_str.parse().ok()
    } else {
        None
    }
}

/// Parse delay string like "delay_60s" into seconds
fn parse_delay(schedule: &str) -> Option<u64> {
    if schedule.starts_with("delay_") && schedule.ends_with('s') {
        let num_str = &schedule[6..schedule.len() - 1];
        num_str.parse().ok()
    } else {
        None
    }
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
        timezone: "UTC".to_string(),
        target_channel: None,
        target_session: None,
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
        timezone: "UTC".to_string(),
        target_channel: None,
        target_session: None,
    }
}

/// Helper to create a delayed one-shot task
#[must_use]
pub fn delayed_task(name: &str, delay: Duration, payload: &str) -> ScheduledTask {
    ScheduledTask {
        id: format!("{}-{}", name, uuid::Uuid::new_v4()),
        name: name.to_string(),
        task_type: TaskType::Delayed {
            delay,
            created: Instant::now(),
        },
        payload: payload.to_string(),
        enabled: true,
        last_run: None,
        run_count: 0,
        timezone: "UTC".to_string(),
        target_channel: None,
        target_session: None,
    }
}

/// Helper to create a memory consolidation ("dreaming") task
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
        timezone: "UTC".to_string(),
        target_channel: None,
        target_session: None,
    }
}

/// Task type marker for the dreaming task
pub const DREAMING_TASK_NAME: &str = "memory_consolidation";

/// Check if a task is the dreaming/consolidation task
#[must_use]
pub fn is_dreaming_task(task: &ScheduledTask) -> bool {
    task.name == DREAMING_TASK_NAME
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

    #[tokio::test]
    async fn test_cron_task_creation() {
        let task = Scheduler::cron_task("daily-backup", "0 3 * * *", "Run backup").unwrap();
        assert_eq!(task.name, "daily-backup");
        assert!(matches!(task.task_type, TaskType::Cron { .. }));
    }

    #[test]
    fn test_parse_interval() {
        assert_eq!(parse_interval("every_300s"), Some(300));
        assert_eq!(parse_interval("every_60s"), Some(60));
        assert_eq!(parse_interval("invalid"), None);
    }
}
