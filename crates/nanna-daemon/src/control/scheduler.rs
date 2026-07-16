//! Scheduler handlers for the [`ControlPlane`].

use super::*;

impl ControlPlane {
    // =========================================================================
    // Scheduler Handlers
    // =========================================================================
    
    pub(super) async fn handle_scheduler(&self, _client_id: &str, action: SchedulerAction) -> Value {
        let Some(ref scheduler) = self.scheduler else {
            return json!({ "error": "scheduler_unavailable", "message": "Scheduler not configured" });
        };
        
        match action {
            SchedulerAction::List => {
                let scheduler = scheduler.read().await;
                let tasks = scheduler.list_tasks().await;
                let jobs: Vec<_> = tasks.into_iter()
                    .map(|t| {
                        let (schedule, next_run) = match &t.task_type {
                            nanna_core::TaskType::Heartbeat => ("heartbeat".to_string(), None),
                            nanna_core::TaskType::Cron { schedule, next_run, .. } => {
                                (schedule.clone(), next_run.map(|dt| dt.to_rfc3339()))
                            }
                            nanna_core::TaskType::Recurring { interval } => {
                                (format!("every_{}s", interval.as_secs()), None)
                            }
                            nanna_core::TaskType::Delayed { delay, .. } => {
                                (format!("delay_{}s", delay.as_secs()), None)
                            }
                        };
                        json!({
                            "id": t.id,
                            "name": t.name,
                            "schedule": schedule,
                            "payload": t.payload,
                            "enabled": t.enabled,
                            "last_run": t.last_run.map(|dt| dt.to_rfc3339()),
                            "next_run": next_run,
                            "run_count": t.run_count,
                            "timezone": t.timezone,
                            "target_channel": t.target_channel,
                            "target_session": t.target_session,
                        })
                    })
                    .collect();
                json!({ "jobs": jobs })
            }
            SchedulerAction::Get { id } => {
                let scheduler = scheduler.read().await;
                if let Some(task) = scheduler.get_task(&id).await {
                    let (schedule, next_run) = match &task.task_type {
                        nanna_core::TaskType::Heartbeat => ("heartbeat".to_string(), None),
                        nanna_core::TaskType::Cron { schedule, next_run, .. } => {
                            (schedule.clone(), next_run.map(|dt| dt.to_rfc3339()))
                        }
                        nanna_core::TaskType::Recurring { interval } => {
                            (format!("every_{}s", interval.as_secs()), None)
                        }
                        nanna_core::TaskType::Delayed { delay, .. } => {
                            (format!("delay_{}s", delay.as_secs()), None)
                        }
                    };
                    json!({
                        "job": {
                            "id": task.id,
                            "name": task.name,
                            "schedule": schedule,
                            "payload": task.payload,
                            "enabled": task.enabled,
                            "last_run": task.last_run.map(|dt| dt.to_rfc3339()),
                            "next_run": next_run,
                            "run_count": task.run_count,
                            "timezone": task.timezone,
                            "target_channel": task.target_channel,
                            "target_session": task.target_session,
                        }
                    })
                } else {
                    json!({ "error": "not_found", "id": id })
                }
            }
            SchedulerAction::Add { schedule, task, name } => {
                // Try to parse as cron expression
                match Scheduler::cron_task(
                    name.as_deref().unwrap_or("unnamed"),
                    &schedule,
                    &task,
                ) {
                    Ok(scheduled_task) => {
                        let id = scheduled_task.id.clone();
                        let scheduler = scheduler.read().await;
                        scheduler.add_task(scheduled_task).await;
                        info!("Added scheduled job: {}", id);
                        json!({ "status": "created", "id": id })
                    }
                    Err(e) => {
                        json!({ "error": "invalid_schedule", "message": e.to_string() })
                    }
                }
            }
            SchedulerAction::Update { id, schedule, task: _, enabled } => {
                let scheduler = scheduler.read().await;
                
                // Update schedule if provided
                if let Some(new_schedule) = schedule {
                    match scheduler.update_schedule(&id, &new_schedule).await {
                        Ok(true) => {}
                        Ok(false) => return json!({ "error": "not_found", "id": id }),
                        Err(e) => return json!({ "error": "invalid_schedule", "message": e.to_string() }),
                    }
                }
                
                // Update enabled state if provided
                if let Some(en) = enabled {
                    scheduler.set_task_enabled(&id, en).await;
                }
                
                // Note: task/payload update would require more logic
                json!({ "status": "updated", "id": id })
            }
            SchedulerAction::Remove { id } => {
                let scheduler = scheduler.read().await;
                if scheduler.remove_task(&id).await {
                    info!("Removed scheduled job: {}", id);
                    json!({ "status": "deleted", "id": id })
                } else {
                    json!({ "error": "not_found", "id": id })
                }
            }
            SchedulerAction::RunNow { id } => {
                let scheduler = scheduler.read().await;
                match scheduler.run_now(&id).await {
                    Some(result) => {
                        json!({
                            "status": if result.success { "success" } else { "failed" },
                            "id": id,
                            "output": result.output,
                            "error": result.error,
                            "duration_ms": result.duration_ms,
                        })
                    }
                    None => json!({ "error": "not_found_or_no_executor", "id": id })
                }
            }
            SchedulerAction::History { id, limit } => {
                let scheduler = scheduler.read().await;
                let runs = scheduler.get_history(&id, limit.unwrap_or(10)).await;
                let history: Vec<_> = runs.into_iter()
                    .map(|r| json!({
                        "run_id": r.id,
                        "job_id": r.job_id,
                        "started_at": r.started_at.to_rfc3339(),
                        "finished_at": r.finished_at.map(|dt| dt.to_rfc3339()),
                        "success": r.success,
                        "output": r.output,
                        "error": r.error,
                    }))
                    .collect();
                json!({ "history": history, "job_id": id })
            }
        }
    }
}
