//! Scheduler and cron job commands.

#[allow(clippy::wildcard_imports)]
use crate::*;

/// Set scheduler enabled
#[tauri::command]
pub async fn set_scheduler_enabled(
    state: State<'_, Arc<RwLock<AppState>>>,
    enabled: bool,
) -> Result<(), String> {
    let state_guard = state.read().await;
    *state_guard.scheduler_enabled.write().await = enabled;

    // Start or stop the scheduler
    let mut scheduler = state_guard.scheduler.write().await;
    if enabled {
        scheduler.start();
        info!("Scheduler started");
    } else {
        scheduler.stop().await;
        info!("Scheduler stopped");
    }

    Ok(())
}

/// Set heartbeat enabled
#[tauri::command]
pub async fn set_heartbeat_enabled(
    state: State<'_, Arc<RwLock<AppState>>>,
    enabled: bool,
) -> Result<(), String> {
    let state_guard = state.read().await;
    *state_guard.heartbeat_enabled.write().await = enabled;
    info!("Heartbeat enabled: {}", enabled);
    Ok(())
}

/// Set heartbeat interval in seconds
#[tauri::command]
pub async fn set_heartbeat_interval(
    state: State<'_, Arc<RwLock<AppState>>>,
    seconds: u64,
) -> Result<(), String> {
    if seconds < 30 {
        return Err("Heartbeat interval must be at least 30 seconds".to_string());
    }

    let state_guard = state.read().await;
    *state_guard.heartbeat_interval_seconds.write().await = seconds;
    info!("Heartbeat interval set to {} seconds", seconds);
    Ok(())
}

// =============================================================================
// Scheduler / Cron Job Commands
// =============================================================================

/// Cron job info for the GUI
#[derive(Debug, Clone, serde::Serialize)]
pub struct CronJobInfo {
    pub id: String,
    pub name: String,
    pub schedule: String,
    pub schedule_description: String,
    pub payload: String,
    pub enabled: bool,
    pub last_run: Option<String>,
    pub next_run: Option<String>,
    pub run_count: u64,
    pub timezone: String,
}

/// Job run info for the GUI
#[derive(Debug, Clone, serde::Serialize)]
pub struct JobRunInfo {
    pub id: i64,
    pub job_id: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
    pub duration_ms: Option<i64>,
}

/// Build a CronJobInfo from the daemon's scheduler.list job JSON
pub(crate) fn cron_job_info_from_daemon(job: &serde_json::Value) -> Option<CronJobInfo> {
    let schedule = job.get("schedule").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let schedule_description = if schedule == "heartbeat" {
        "Periodic heartbeat".to_string()
    } else if let Ok(parsed) = nanna_core::CronExpr::parse(&schedule) {
        parsed.describe()
    } else {
        schedule.clone()
    };
    Some(CronJobInfo {
        id: job.get("id")?.as_str()?.to_string(),
        name: job.get("name").and_then(|v| v.as_str()).unwrap_or("unnamed").to_string(),
        schedule,
        schedule_description,
        payload: job.get("payload").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        enabled: job.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
        last_run: job.get("last_run").and_then(|v| v.as_str()).map(str::to_string),
        next_run: job.get("next_run").and_then(|v| v.as_str()).map(str::to_string),
        run_count: job.get("run_count").and_then(|v| v.as_u64()).unwrap_or(0),
        timezone: job.get("timezone").and_then(|v| v.as_str()).unwrap_or("UTC").to_string(),
    })
}

/// Get all scheduled jobs
#[tauri::command]
pub async fn list_cron_jobs(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<CronJobInfo>, String> {
    let state_guard = state.read().await;

    // Daemon mode: the daemon is the cron runner — its list is the truth
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard.backend.scheduler_list().await?;
        let jobs = result
            .get("jobs")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(cron_job_info_from_daemon).collect())
            .unwrap_or_default();
        return Ok(jobs);
    }

    let scheduler = state_guard.scheduler.read().await;
    let tasks = scheduler.list_tasks().await;

    let jobs: Vec<CronJobInfo> = tasks.into_iter().map(|t| {
        let (schedule, next_run, schedule_description) = match &t.task_type {
            nanna_core::TaskType::Heartbeat => {
                ("heartbeat".to_string(), None, "Periodic heartbeat".to_string())
            }
            nanna_core::TaskType::Cron { schedule, next_run, parsed } => {
                let desc = parsed.as_ref()
                    .map(|p| p.describe())
                    .unwrap_or_else(|| schedule.clone());
                (schedule.clone(), next_run.map(|dt| dt.to_rfc3339()), desc)
            }
            nanna_core::TaskType::Recurring { interval } => {
                let secs = interval.as_secs();
                let desc = if secs >= 3600 {
                    format!("Every {} hours", secs / 3600)
                } else if secs >= 60 {
                    format!("Every {} minutes", secs / 60)
                } else {
                    format!("Every {} seconds", secs)
                };
                (format!("every_{}s", secs), None, desc)
            }
            nanna_core::TaskType::Delayed { delay, .. } => {
                (format!("delay_{}s", delay.as_secs()), None, "One-shot delayed".to_string())
            }
        };

        CronJobInfo {
            id: t.id.clone(),
            name: t.name.clone(),
            schedule,
            schedule_description,
            payload: t.payload.clone(),
            enabled: t.enabled,
            last_run: t.last_run.map(|dt| dt.to_rfc3339()),
            next_run,
            run_count: t.run_count,
            timezone: t.timezone.clone(),
        }
    }).collect();

    Ok(jobs)
}

/// Create a new cron job
#[tauri::command]
pub async fn create_cron_job(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
    schedule: String,
    payload: String,
    timezone: Option<String>,
) -> Result<CronJobInfo, String> {
    use nanna_core::CronExpr;

    // Validate the cron expression
    let parsed = CronExpr::parse(&schedule).map_err(|e| e.to_string())?;
    let next_run = parsed.next_from_now();

    let task = nanna_core::ScheduledTask {
        id: format!("{}-{}", name, uuid::Uuid::new_v4()),
        name: name.clone(),
        task_type: nanna_core::TaskType::Cron {
            schedule: schedule.clone(),
            parsed: Some(parsed.clone()),
            next_run,
        },
        payload: payload.clone(),
        enabled: true,
        last_run: None,
        run_count: 0,
        timezone: timezone.clone().unwrap_or_else(|| "UTC".to_string()),
        target_channel: None,
        target_session: None,
    };

    let state_guard = state.read().await;

    // Daemon mode: create the job on the daemon (it owns nanna.db and runs
    // the cron loop). Note: the daemon's add defaults to UTC — a custom
    // timezone is not forwarded over IPC yet.
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard
            .backend
            .scheduler_add(&schedule, &payload, Some(&name))
            .await?;
        if result.get("error").is_some() {
            let msg = result
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(format!("Daemon failed to create cron job: {msg}"));
        }
        let id = result
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or("Daemon returned no job id")?
            .to_string();
        return Ok(CronJobInfo {
            id,
            name,
            schedule: schedule.clone(),
            schedule_description: parsed.describe(),
            payload,
            enabled: true,
            last_run: None,
            next_run: next_run.map(|dt| dt.to_rfc3339()),
            run_count: 0,
            timezone: "UTC".to_string(),
        });
    }

    let scheduler = state_guard.scheduler.read().await;
    scheduler.add_task(task.clone()).await;

    Ok(CronJobInfo {
        id: task.id.clone(),
        name: task.name,
        schedule: schedule.clone(),
        schedule_description: parsed.describe(),
        payload,
        enabled: true,
        last_run: None,
        next_run: next_run.map(|dt| dt.to_rfc3339()),
        run_count: 0,
        timezone: timezone.unwrap_or_else(|| "UTC".to_string()),
    })
}

/// Update a cron job's schedule
#[tauri::command]
pub async fn update_cron_job(
    state: State<'_, Arc<RwLock<AppState>>>,
    job_id: String,
    schedule: String,
) -> Result<bool, String> {
    let state_guard = state.read().await;
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard
            .backend
            .scheduler_update(&job_id, Some(&schedule), None, None)
            .await?;
        return match result.get("error").and_then(|v| v.as_str()) {
            None => Ok(true),
            Some("not_found") => Ok(false),
            Some(_) => Err(result
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error")
                .to_string()),
        };
    }
    let scheduler = state_guard.scheduler.read().await;
    scheduler.update_schedule(&job_id, &schedule).await.map_err(|e| e.to_string())
}

/// Enable or disable a cron job
#[tauri::command]
pub async fn set_cron_job_enabled(
    state: State<'_, Arc<RwLock<AppState>>>,
    job_id: String,
    enabled: bool,
) -> Result<(), String> {
    let state_guard = state.read().await;
    if state_guard.backend.is_daemon_mode().await {
        state_guard
            .backend
            .scheduler_update(&job_id, None, None, Some(enabled))
            .await?;
        return Ok(());
    }
    let scheduler = state_guard.scheduler.read().await;
    scheduler.set_task_enabled(&job_id, enabled).await;
    Ok(())
}

/// Delete a cron job
#[tauri::command]
pub async fn delete_cron_job(
    state: State<'_, Arc<RwLock<AppState>>>,
    job_id: String,
) -> Result<bool, String> {
    let state_guard = state.read().await;
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard.backend.scheduler_remove(&job_id).await?;
        return Ok(result.get("status").and_then(|v| v.as_str()) == Some("deleted"));
    }
    let scheduler = state_guard.scheduler.read().await;
    Ok(scheduler.remove_task(&job_id).await)
}

/// Delete all cron jobs with a given name (useful for cleanup)
#[tauri::command]
pub async fn delete_cron_jobs_by_name(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
) -> Result<usize, String> {
    let state_guard = state.read().await;
    if state_guard.backend.is_daemon_mode().await {
        // No by-name removal over IPC — list, filter, remove each.
        let result = state_guard.backend.scheduler_list().await?;
        let ids: Vec<String> = result
            .get("jobs")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter(|j| j.get("name").and_then(|v| v.as_str()) == Some(name.as_str()))
                    .filter_map(|j| j.get("id").and_then(|v| v.as_str()).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let mut removed = 0;
        for id in &ids {
            let result = state_guard.backend.scheduler_remove(id).await?;
            if result.get("status").and_then(|v| v.as_str()) == Some("deleted") {
                removed += 1;
            }
        }
        return Ok(removed);
    }
    let scheduler = state_guard.scheduler.read().await;
    Ok(scheduler.remove_tasks_by_name(&name).await)
}

/// Run a cron job immediately
#[tauri::command]
pub async fn run_cron_job_now(
    state: State<'_, Arc<RwLock<AppState>>>,
    job_id: String,
) -> Result<Option<JobRunInfo>, String> {
    let state_guard = state.read().await;

    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard.backend.scheduler_run_now(&job_id).await?;
        if result.get("error").is_some() {
            return Ok(None);
        }
        let now = chrono::Utc::now().to_rfc3339();
        return Ok(Some(JobRunInfo {
            id: 0,
            job_id,
            started_at: now.clone(),
            finished_at: Some(now),
            success: result.get("status").and_then(|v| v.as_str()) == Some("success"),
            output: result.get("output").and_then(|v| v.as_str()).map(str::to_string),
            error: result.get("error").and_then(|v| v.as_str()).map(str::to_string),
            duration_ms: result.get("duration_ms").and_then(serde_json::Value::as_i64),
        }));
    }

    let scheduler = state_guard.scheduler.read().await;

    if let Some(result) = scheduler.run_now(&job_id).await {
        Ok(Some(JobRunInfo {
            id: 0,
            job_id: result.task_id,
            started_at: result.started_at.to_rfc3339(),
            finished_at: Some(result.finished_at.to_rfc3339()),
            success: result.success,
            output: result.output,
            error: result.error,
            duration_ms: Some(result.duration_ms as i64),
        }))
    } else {
        Ok(None)
    }
}

/// Get job run history
#[tauri::command]
pub async fn get_cron_job_history(
    state: State<'_, Arc<RwLock<AppState>>>,
    job_id: String,
    limit: Option<usize>,
) -> Result<Vec<JobRunInfo>, String> {
    let state_guard = state.read().await;

    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard
            .backend
            .scheduler_history(&job_id, limit)
            .await?;
        let runs = result
            .get("history")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|r| JobRunInfo {
                        id: r.get("run_id").and_then(serde_json::Value::as_i64).unwrap_or(0),
                        job_id: r
                            .get("job_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        started_at: r
                            .get("started_at")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        finished_at: r
                            .get("finished_at")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        success: r.get("success").and_then(|v| v.as_bool()).unwrap_or(false),
                        output: r.get("output").and_then(|v| v.as_str()).map(str::to_string),
                        error: r.get("error").and_then(|v| v.as_str()).map(str::to_string),
                        duration_ms: None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        return Ok(runs);
    }

    let scheduler = state_guard.scheduler.read().await;

    let runs = scheduler.get_history(&job_id, limit.unwrap_or(20)).await;

    Ok(runs.into_iter().map(|r| JobRunInfo {
        id: r.id,
        job_id: r.job_id,
        started_at: r.started_at.to_rfc3339(),
        finished_at: r.finished_at.map(|dt| dt.to_rfc3339()),
        success: r.success,
        output: r.output,
        error: r.error,
        duration_ms: None, // JobRun from scheduler doesn't track this yet
    }).collect())
}

/// Validate a cron expression
#[tauri::command]
pub async fn validate_cron_expression(
    expression: String,
) -> Result<(bool, String), String> {
    use nanna_core::CronExpr;

    match CronExpr::parse(&expression) {
        Ok(parsed) => {
            let description = parsed.describe();
            let next = parsed.next_from_now()
                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "N/A".to_string());
            Ok((true, format!("{} (next: {})", description, next)))
        }
        Err(e) => Ok((false, e.to_string())),
    }
}
