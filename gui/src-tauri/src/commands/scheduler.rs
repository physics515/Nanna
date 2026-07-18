//! Scheduler and cron-job commands. The daemon runs heartbeat + cron and owns
//! the job store; these forward to it. The whole-scheduler / heartbeat runtime
//! toggles have no daemon control action yet, so they are no-ops.

#[allow(clippy::wildcard_imports)]
use crate::*;

/// Enable/disable the scheduler.
///
/// No-op: the daemon owns the scheduler and exposes no enable/disable action
/// over IPC. Kept for UI compatibility.
#[tauri::command]
pub async fn set_scheduler_enabled(
    _state: State<'_, Arc<RwLock<AppState>>>,
    enabled: bool,
) -> Result<(), String> {
    info!("set_scheduler_enabled({enabled}) is a no-op in daemon-only mode (the daemon owns the scheduler)");
    Ok(())
}

/// Enable/disable the heartbeat.
///
/// No-op: no daemon control action for the heartbeat toggle yet.
#[tauri::command]
pub async fn set_heartbeat_enabled(
    _state: State<'_, Arc<RwLock<AppState>>>,
    enabled: bool,
) -> Result<(), String> {
    info!("set_heartbeat_enabled({enabled}) is a no-op in daemon-only mode");
    Ok(())
}

/// Set the heartbeat interval.
///
/// No-op: no daemon control action for the heartbeat interval yet.
#[tauri::command]
pub async fn set_heartbeat_interval(
    _state: State<'_, Arc<RwLock<AppState>>>,
    seconds: u64,
) -> Result<(), String> {
    if seconds < 30 {
        return Err("Heartbeat interval must be at least 30 seconds".to_string());
    }
    info!("set_heartbeat_interval({seconds}) is a no-op in daemon-only mode");
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

/// Build a `CronJobInfo` from the daemon's `scheduler.list` job JSON.
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

/// Get all scheduled jobs.
#[tauri::command]
pub async fn list_cron_jobs(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<CronJobInfo>, String> {
    let state_guard = state.read().await;
    let result = state_guard.backend.scheduler_list().await?;
    let jobs = result
        .get("jobs")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(cron_job_info_from_daemon).collect())
        .unwrap_or_default();
    Ok(jobs)
}

/// Create a new cron job.
#[tauri::command]
pub async fn create_cron_job(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
    schedule: String,
    payload: String,
    timezone: Option<String>,
) -> Result<CronJobInfo, String> {
    use nanna_core::CronExpr;

    // Validate the cron expression up-front so we fail fast with a good message.
    let parsed = CronExpr::parse(&schedule).map_err(|e| e.to_string())?;
    let next_run = parsed.next_from_now();

    let state_guard = state.read().await;
    let result = state_guard
        .backend
        .scheduler_add(&schedule, &payload, Some(&name))
        .await?;
    if result.get("error").is_some() {
        let msg = result.get("message").and_then(|v| v.as_str()).unwrap_or("unknown error");
        return Err(format!("Daemon failed to create cron job: {msg}"));
    }
    let id = result
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("Daemon returned no job id")?
        .to_string();

    Ok(CronJobInfo {
        id,
        name,
        schedule: schedule.clone(),
        schedule_description: parsed.describe(),
        payload,
        enabled: true,
        last_run: None,
        next_run: next_run.map(|dt| dt.to_rfc3339()),
        run_count: 0,
        // The daemon's add defaults to UTC; a custom timezone is not forwarded yet.
        timezone: timezone.unwrap_or_else(|| "UTC".to_string()),
    })
}

/// Update a cron job's schedule.
#[tauri::command]
pub async fn update_cron_job(
    state: State<'_, Arc<RwLock<AppState>>>,
    job_id: String,
    schedule: String,
) -> Result<bool, String> {
    let state_guard = state.read().await;
    let result = state_guard
        .backend
        .scheduler_update(&job_id, Some(&schedule), None, None)
        .await?;
    match result.get("error").and_then(|v| v.as_str()) {
        None => Ok(true),
        Some("not_found") => Ok(false),
        Some(_) => Err(result
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error")
            .to_string()),
    }
}

/// Enable or disable a cron job.
#[tauri::command]
pub async fn set_cron_job_enabled(
    state: State<'_, Arc<RwLock<AppState>>>,
    job_id: String,
    enabled: bool,
) -> Result<(), String> {
    let state_guard = state.read().await;
    state_guard
        .backend
        .scheduler_update(&job_id, None, None, Some(enabled))
        .await?;
    Ok(())
}

/// Delete a cron job.
#[tauri::command]
pub async fn delete_cron_job(
    state: State<'_, Arc<RwLock<AppState>>>,
    job_id: String,
) -> Result<bool, String> {
    let state_guard = state.read().await;
    let result = state_guard.backend.scheduler_remove(&job_id).await?;
    Ok(result.get("status").and_then(|v| v.as_str()) == Some("deleted"))
}

/// Delete all cron jobs with a given name (useful for cleanup).
#[tauri::command]
pub async fn delete_cron_jobs_by_name(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
) -> Result<usize, String> {
    let state_guard = state.read().await;
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
    Ok(removed)
}

/// Run a cron job immediately.
#[tauri::command]
pub async fn run_cron_job_now(
    state: State<'_, Arc<RwLock<AppState>>>,
    job_id: String,
) -> Result<Option<JobRunInfo>, String> {
    let state_guard = state.read().await;
    let result = state_guard.backend.scheduler_run_now(&job_id).await?;
    if result.get("error").is_some() {
        return Ok(None);
    }
    let now = chrono::Utc::now().to_rfc3339();
    Ok(Some(JobRunInfo {
        id: 0,
        job_id,
        started_at: now.clone(),
        finished_at: Some(now),
        success: result.get("status").and_then(|v| v.as_str()) == Some("success"),
        output: result.get("output").and_then(|v| v.as_str()).map(str::to_string),
        error: result.get("error").and_then(|v| v.as_str()).map(str::to_string),
        duration_ms: result.get("duration_ms").and_then(serde_json::Value::as_i64),
    }))
}

/// Get job run history.
#[tauri::command]
pub async fn get_cron_job_history(
    state: State<'_, Arc<RwLock<AppState>>>,
    job_id: String,
    limit: Option<usize>,
) -> Result<Vec<JobRunInfo>, String> {
    let state_guard = state.read().await;
    let result = state_guard.backend.scheduler_history(&job_id, limit).await?;
    let runs = result
        .get("history")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|r| JobRunInfo {
                    id: r.get("run_id").and_then(serde_json::Value::as_i64).unwrap_or(0),
                    job_id: r.get("job_id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                    started_at: r.get("started_at").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                    finished_at: r.get("finished_at").and_then(|v| v.as_str()).map(str::to_string),
                    success: r.get("success").and_then(|v| v.as_bool()).unwrap_or(false),
                    output: r.get("output").and_then(|v| v.as_str()).map(str::to_string),
                    error: r.get("error").and_then(|v| v.as_str()).map(str::to_string),
                    duration_ms: None,
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(runs)
}

/// Validate a cron expression.
#[tauri::command]
pub async fn validate_cron_expression(expression: String) -> Result<(bool, String), String> {
    use nanna_core::CronExpr;

    match CronExpr::parse(&expression) {
        Ok(parsed) => {
            let description = parsed.describe();
            let next = parsed
                .next_from_now()
                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "N/A".to_string());
            Ok((true, format!("{description} (next: {next})")))
        }
        Err(e) => Ok((false, e.to_string())),
    }
}
