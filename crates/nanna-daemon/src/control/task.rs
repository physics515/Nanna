//! Task control actions (P15 store + P14 long-horizon runs).

#[allow(clippy::wildcard_imports)]
use super::*;
use crate::tasks::{AgentStepRunner, TursoTaskSource};
use nanna_agent::harness::LongHorizonConfig;
use nanna_storage::{NewTask, TaskPatch};

impl ControlPlane {
    /// Resolve `(scope, scope_id)` for a task action. Workspace scope binds
    /// to the active workspace; session scope requires an explicit id.
    async fn resolve_task_scope(
        &self,
        scope: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<(String, Option<String>), String> {
        let scope = scope.unwrap_or("session").to_lowercase();
        match scope.as_str() {
            "session" => {
                let session_id = session_id
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| "session scope requires session_id".to_string())?;
                Ok(("session".to_string(), Some(session_id.to_string())))
            }
            "workspace" => {
                let workspaces = self.workspaces.read().await;
                let active = workspaces
                    .active()
                    .ok_or_else(|| "workspace scope requires an active workspace".to_string())?;
                Ok(("workspace".to_string(), Some(active.id.clone())))
            }
            "global" => Ok(("global".to_string(), None)),
            other => Err(format!("unknown scope '{other}'")),
        }
    }

    #[allow(clippy::too_many_lines)]
    pub(super) async fn handle_task(&self, _client_id: &str, action: TaskAction) -> Value {
        let Some(ref storage) = self.storage else {
            return json!({"error": "storage_unavailable", "message": "task store requires storage"});
        };
        let repo = storage.tasks();

        match action {
            TaskAction::List {
                scope,
                session_id,
                include_closed,
            } => {
                let (scope, scope_id) = match self
                    .resolve_task_scope(scope.as_deref(), session_id.as_deref())
                    .await
                {
                    Ok(resolved) => resolved,
                    Err(message) => return json!({"error": "bad_scope", "message": message}),
                };
                match repo
                    .list(&scope, scope_id.as_deref(), include_closed.unwrap_or(true))
                    .await
                {
                    Ok(tasks) => json!({"tasks": tasks}),
                    Err(e) => json!({"error": "task_list_failed", "message": e.to_string()}),
                }
            }

            TaskAction::Get { id } => {
                let task = match repo.get(id).await {
                    Ok(task) => task,
                    Err(e) => return json!({"error": "task_not_found", "message": e.to_string()}),
                };
                let notes = repo.notes(id, 50).await.unwrap_or_default();
                let activity = repo.activity(id, 50).await.unwrap_or_default();
                json!({"task": task, "notes": notes, "activity": activity})
            }

            TaskAction::Next { scope, session_id } => {
                let (scope, scope_id) = match self
                    .resolve_task_scope(scope.as_deref(), session_id.as_deref())
                    .await
                {
                    Ok(resolved) => resolved,
                    Err(message) => return json!({"error": "bad_scope", "message": message}),
                };
                match repo.next(&scope, scope_id.as_deref()).await {
                    Ok(task) => json!({"task": task}),
                    Err(e) => json!({"error": "task_next_failed", "message": e.to_string()}),
                }
            }

            TaskAction::Create {
                title,
                scope,
                session_id,
                parent_id,
                description,
                priority,
                labels,
                tools,
                due_at,
                recurrence,
                depends_on,
                acceptance,
                project,
                assignee,
            } => {
                // A subtask always lives in its parent's scope.
                let (scope, scope_id) = if let Some(parent_id) = parent_id {
                    match repo.get(parent_id).await {
                        Ok(parent) => (parent.scope, parent.scope_id),
                        Err(e) => {
                            return json!({"error": "task_not_found", "message": e.to_string()});
                        }
                    }
                } else {
                    match self
                        .resolve_task_scope(scope.as_deref(), session_id.as_deref())
                        .await
                    {
                        Ok(resolved) => resolved,
                        Err(message) => return json!({"error": "bad_scope", "message": message}),
                    }
                };
                // Canonicalize through the harness parser: a shape that would
                // fail at run time is rejected here, not mid-run.
                let acceptance = match acceptance {
                    Some(raw) => {
                        match nanna_agent::harness::AcceptanceCheck::from_json(&raw)
                            .and_then(|c| serde_json::to_value(&c).map_err(|e| e.to_string()))
                        {
                            Ok(canonical) => Some(canonical),
                            Err(message) => {
                                return json!({"error": "bad_acceptance", "message": message});
                            }
                        }
                    }
                    None => None,
                };
                let new = NewTask {
                    parent_id,
                    scope,
                    scope_id,
                    project,
                    title,
                    description,
                    priority: priority.unwrap_or(3),
                    labels: labels.unwrap_or_default(),
                    tool_scope: tools.unwrap_or_default(),
                    due_at,
                    recurrence,
                    depends_on: depends_on.unwrap_or_default(),
                    acceptance,
                    assignee,
                    sort_order: 0,
                };
                match repo.create(new).await {
                    Ok(task) => {
                        self.emit(Event::TaskRunProgress {
                            scope: task.scope.clone(),
                            scope_id: task.scope_id.clone(),
                            task_id: Some(task.id),
                            kind: "created".to_string(),
                            detail: json!({"title": task.title}),
                        });
                        json!({"task": task})
                    }
                    Err(e) => json!({"error": "task_create_failed", "message": e.to_string()}),
                }
            }

            TaskAction::Update { id, patch } => {
                let string_vec = |v: &Value| -> Vec<String> {
                    v.as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default()
                };
                // Null/absent/mistyped values SKIP a field, never wipe it —
                // partial patches must not clear what they did not mention.
                let acceptance = match patch.get("acceptance").filter(|v| !v.is_null()) {
                    Some(raw) => {
                        match nanna_agent::harness::AcceptanceCheck::from_json(raw)
                            .and_then(|c| serde_json::to_value(&c).map_err(|e| e.to_string()))
                        {
                            Ok(canonical) => Some(Some(canonical)),
                            Err(message) => {
                                return json!({"error": "bad_acceptance", "message": message});
                            }
                        }
                    }
                    None => None,
                };
                let task_patch = TaskPatch {
                    title: patch
                        .get("title")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    description: patch
                        .get("description")
                        .and_then(Value::as_str)
                        .map(|s| Some(s.to_string())),
                    status: patch
                        .get("status")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    priority: patch.get("priority").and_then(Value::as_i64),
                    labels: patch
                        .get("labels")
                        .filter(|v| v.is_array())
                        .map(&string_vec),
                    tool_scope: patch.get("tools").filter(|v| v.is_array()).map(&string_vec),
                    due_at: patch
                        .get("due_at")
                        .and_then(Value::as_str)
                        .map(|s| Some(s.to_string())),
                    recurrence: patch
                        .get("recurrence")
                        .and_then(Value::as_str)
                        .map(|s| Some(s.to_string())),
                    depends_on: patch.get("depends_on").filter(|v| v.is_array()).map(|v| {
                        v.as_array()
                            .map(|arr| arr.iter().filter_map(Value::as_i64).collect())
                            .unwrap_or_default()
                    }),
                    acceptance,
                    assignee: patch
                        .get("assignee")
                        .and_then(Value::as_str)
                        .map(|s| Some(s.to_string())),
                    parent_id: patch.get("parent_id").and_then(Value::as_i64).map(Some),
                    project: patch
                        .get("project")
                        .and_then(Value::as_str)
                        .map(|s| Some(s.to_string())),
                    sort_order: patch.get("sort_order").and_then(Value::as_i64),
                };
                match repo.update(id, task_patch, Some("gui")).await {
                    Ok(task) => json!({"task": task}),
                    Err(e) => json!({"error": "task_update_failed", "message": e.to_string()}),
                }
            }

            TaskAction::Done { id, workdir } => {
                // Same verdict-first flow as the tasks.done service: run the
                // acceptance check before completion is recorded.
                let task = match repo.get(id).await {
                    Ok(task) => task,
                    Err(e) => return json!({"error": "task_not_found", "message": e.to_string()}),
                };
                if let Some(acceptance) = &task.acceptance {
                    let check = match nanna_agent::harness::AcceptanceCheck::from_json(acceptance) {
                        Ok(check) => check,
                        Err(message) => {
                            return json!({"error": "bad_acceptance", "message": message});
                        }
                    };
                    // Default to the active workspace root — the daemon's own
                    // cwd is meaningless for workspace artifacts.
                    let dir = match workdir {
                        Some(dir) => PathBuf::from(dir),
                        None => self
                            .workspaces
                            .read()
                            .await
                            .active()
                            .map_or_else(|| PathBuf::from("."), |w| w.path.clone()),
                    };
                    let verdict = check.run(&dir).await;
                    let _ = repo
                        .log_activity(
                            id,
                            Some("gui"),
                            "acceptance_checked",
                            Some(json!({"passed": verdict.passed, "detail": verdict.detail})),
                        )
                        .await;
                    if !verdict.passed {
                        return json!({"done": false, "verdict": verdict.detail});
                    }
                }
                match repo.complete(id, Some("gui"), None).await {
                    Ok(outcome) => json!({
                        "done": true,
                        "already_done": outcome.already_done,
                        "auto_completed": outcome.auto_completed,
                    }),
                    Err(e) => json!({"error": "task_done_failed", "message": e.to_string()}),
                }
            }

            TaskAction::Delete { id } => match repo.delete(id, Some("gui")).await {
                Ok(removed) => json!({"removed": removed}),
                Err(e) => json!({"error": "task_delete_failed", "message": e.to_string()}),
            },

            TaskAction::Note { id, content } => {
                match repo.add_note(id, Some("gui"), &content).await {
                    Ok(note) => json!({"note": note}),
                    Err(e) => json!({"error": "task_note_failed", "message": e.to_string()}),
                }
            }

            TaskAction::Query {
                filter,
                scope,
                session_id,
            } => {
                let (scope, scope_id) = match self
                    .resolve_task_scope(scope.as_deref(), session_id.as_deref())
                    .await
                {
                    Ok(resolved) => resolved,
                    Err(message) => return json!({"error": "bad_scope", "message": message}),
                };
                match repo.query(&scope, scope_id.as_deref(), &filter).await {
                    Ok(tasks) => json!({"tasks": tasks}),
                    Err(e) => json!({"error": "task_query_failed", "message": e.to_string()}),
                }
            }

            TaskAction::StartRun {
                goal,
                scope,
                session_id,
                workdir,
                max_wall_clock_secs,
                max_total_tokens,
            } => {
                let Some(ref task_runs) = self.task_runs else {
                    return json!({"error": "task_runs_unavailable", "message": "run manager not attached"});
                };
                let (Some(agent), Some(router), Some(tools)) = (
                    self.agent.as_ref(),
                    self.router.as_ref(),
                    self.tools.as_ref(),
                ) else {
                    return json!({"error": "agent_unavailable", "message": "agent service required"});
                };
                let Some(event_tx) = self.event_tx.clone() else {
                    return json!({"error": "events_unavailable", "message": "event bus required"});
                };
                let (scope, scope_id) = match self
                    .resolve_task_scope(scope.as_deref(), session_id.as_deref())
                    .await
                {
                    Ok(resolved) => resolved,
                    Err(message) => return json!({"error": "bad_scope", "message": message}),
                };

                // Workdir: explicit > active workspace root > current dir.
                let workspace_root = self
                    .workspaces
                    .read()
                    .await
                    .active()
                    .map(|w| w.path.clone());
                let dir = workdir
                    .map(PathBuf::from)
                    .or_else(|| workspace_root.clone())
                    .unwrap_or_else(|| PathBuf::from("."));

                let source = TursoTaskSource::new(
                    storage.clone(),
                    scope.clone(),
                    scope_id.clone(),
                    "harness".to_string(),
                    Some(event_tx.clone()),
                );
                let runner = AgentStepRunner {
                    router: router.clone(),
                    tools: tools.clone(),
                    agent_config: agent.agent_config().await,
                    system_prompt: self.system_prompt.read().await.clone(),
                    workspace_root,
                    stats: Some(self.model_stats.clone()),
                };
                let mut config = LongHorizonConfig::default();
                if let Some(secs) = max_wall_clock_secs {
                    config.max_wall_clock = std::time::Duration::from_secs(secs);
                }
                config.max_total_tokens = max_total_tokens;

                match task_runs
                    .start(goal, source, runner, config, dir, event_tx)
                    .await
                {
                    Ok(()) => json!({"started": true, "scope": scope, "scope_id": scope_id}),
                    Err(message) => json!({"error": "run_start_failed", "message": message}),
                }
            }

            TaskAction::RunStatus { scope, session_id } => {
                let Some(ref task_runs) = self.task_runs else {
                    return json!({"error": "task_runs_unavailable", "message": "run manager not attached"});
                };
                let (scope, scope_id) = match self
                    .resolve_task_scope(scope.as_deref(), session_id.as_deref())
                    .await
                {
                    Ok(resolved) => resolved,
                    Err(message) => return json!({"error": "bad_scope", "message": message}),
                };
                let status = task_runs.status(&scope, scope_id.as_deref()).await;
                serde_json::to_value(&status).unwrap_or_else(|_| json!({"running": false}))
            }

            TaskAction::CancelRun { scope, session_id } => {
                let Some(ref task_runs) = self.task_runs else {
                    return json!({"error": "task_runs_unavailable", "message": "run manager not attached"});
                };
                let (scope, scope_id) = match self
                    .resolve_task_scope(scope.as_deref(), session_id.as_deref())
                    .await
                {
                    Ok(resolved) => resolved,
                    Err(message) => return json!({"error": "bad_scope", "message": message}),
                };
                let cancelled = task_runs.cancel(&scope, scope_id.as_deref()).await;
                json!({"cancelled": cancelled})
            }
        }
    }
}
