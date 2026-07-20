//! Daemon-side task-store integration (P15) and long-horizon run manager (P14).
//!
//! Three pieces live here:
//! - `build_task_services` — the `tasks.*` script services the `todo` skill
//!   calls via `Nanna.service(...)`. The store is the daemon's Turso DB; the
//!   JS tool never touches the filesystem for task state again.
//! - `TursoTaskSource` / `AgentStepRunner` — the production implementations of
//!   the harness traits: the P15 repository as [`nanna_agent::harness::TaskSource`],
//!   and a fresh `Agent` + empty context per step as
//!   [`nanna_agent::harness::StepRunner`] (the re-anchor: parent state never
//!   accumulates in a transcript).
//! - `TaskRunManager` — starts/cancels/reports background long-horizon runs
//!   and broadcasts their lifecycle as events. The task store itself is the
//!   checkpoint: resuming after a crash is just starting a run in the same
//!   scope again.

use crate::llm_router::LlmRouter;
use crate::protocol::Event;
use nanna_agent::harness::{
    AcceptanceCheck, LongHorizonConfig, LongHorizonReport, LongHorizonRunner, StepOutcome,
    StepRequest, StepRunner, StepToolCall, StopReason, TaskSource, TaskStep,
};
use nanna_scripting::ServiceFn;
use nanna_storage::{NewTask, Storage, StorageError, Task, TaskPatch};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::RwLock;
use tracing::info;

/// Notes injected into a step prompt.
///
/// Bound justification: each note is already capped at 16 KiB by the store,
/// but a prompt tail only has room for a handful of findings — 5 recent notes
/// keeps the O(1) prompt O(1).
const STEP_NOTES_TAIL: i64 = 5;

/// How many malformed-acceptance tasks `next()` will cancel before giving up.
///
/// Bound justification: each skip cancels one task (the open set strictly
/// shrinks), so this only trips when a scope is saturated with corrupt rows —
/// at that point stopping loudly beats grinding through thousands.
const TASK_NEXT_SKIP_MAX: usize = 100;

// ---------------------------------------------------------------------------
// Scope resolution
// ---------------------------------------------------------------------------

/// Resolve `(scope, scope_id)` from service params + the active workspace.
async fn resolve_scope(
    params: &Value,
    workspace_id: &Arc<RwLock<Option<String>>>,
) -> Result<(String, Option<String>), String> {
    let scope = params
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or("session")
        .to_lowercase();
    match scope.as_str() {
        "session" => {
            let session_id = params
                .get("session_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "session scope requires session_id".to_string())?;
            Ok(("session".to_string(), Some(session_id.to_string())))
        }
        "workspace" => {
            let ws = workspace_id.read().await.clone();
            let ws =
                ws.ok_or_else(|| "workspace scope requires an active workspace".to_string())?;
            Ok(("workspace".to_string(), Some(ws)))
        }
        "global" => Ok(("global".to_string(), None)),
        other => Err(format!("unknown scope '{other}'")),
    }
}

fn task_to_json(task: &Task) -> Value {
    json!({
        "id": task.id,
        "parent_id": task.parent_id,
        "scope": task.scope,
        "project": task.project,
        "title": task.title,
        "description": task.description,
        "status": task.status,
        "blocked": task.blocked,
        "priority": task.priority,
        "labels": task.labels,
        "tools": task.tool_scope,
        "due_at": task.due_at,
        "recurrence": task.recurrence,
        "depends_on": task.depends_on,
        "acceptance": task.acceptance,
        "assignee": task.assignee,
        "created_at": task.created_at,
        "completed_at": task.completed_at,
    })
}

fn string_vec(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Read an i64 that may arrive as a JS float (Boa numbers cross the bridge
/// as f64 once arithmetic touches them).
fn as_i64_lenient(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        value
            .as_f64()
            .filter(|f| f.fract() == 0.0)
            .map(|f| f as i64)
    })
}

fn get_i64(params: &Value, key: &str) -> Option<i64> {
    params.get(key).and_then(as_i64_lenient)
}

fn i64_vec(value: Option<&Value>) -> Vec<i64> {
    value
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(as_i64_lenient).collect())
        .unwrap_or_default()
}

fn opt_string(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Canonicalize an acceptance payload through the harness parser so a shape
/// that would fail at run time is rejected at write time instead of wedging
/// every future run in the scope.
fn canonical_acceptance(params: &Value) -> Result<Option<Value>, String> {
    match params.get("acceptance").filter(|v| !v.is_null()) {
        Some(raw) => {
            let check = AcceptanceCheck::from_json(raw)?;
            serde_json::to_value(&check)
                .map(Some)
                .map_err(|e| e.to_string())
        }
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// tasks.* script services
// ---------------------------------------------------------------------------

/// Build the `tasks.*` services the todo skill calls. Registered in
/// `build_script_services` when storage is available.
#[allow(clippy::too_many_lines)]
pub fn build_task_services(
    storage: Arc<Storage>,
    workspace_id: Arc<RwLock<Option<String>>>,
) -> HashMap<String, ServiceFn> {
    let mut services: HashMap<String, ServiceFn> = HashMap::new();

    let err_str = |e: StorageError| e.to_string();

    // tasks.next {scope?, session_id?}
    {
        let storage = storage.clone();
        let workspace_id = workspace_id.clone();
        services.insert(
            "tasks.next".to_string(),
            Arc::new(move |params: Value| {
                let storage = storage.clone();
                let workspace_id = workspace_id.clone();
                Box::pin(async move {
                    let (scope, scope_id) = resolve_scope(&params, &workspace_id).await?;
                    let next = storage
                        .tasks()
                        .next(&scope, scope_id.as_deref())
                        .await
                        .map_err(err_str)?;
                    match next {
                        Some(task) => {
                            let notes = storage
                                .tasks()
                                .notes(task.id, STEP_NOTES_TAIL)
                                .await
                                .map_err(err_str)?;
                            let mut value = task_to_json(&task);
                            value["notes"] =
                                json!(notes.iter().map(|n| n.content.clone()).collect::<Vec<_>>());
                            Ok(json!({ "task": value }))
                        }
                        None => Ok(json!({ "task": Value::Null })),
                    }
                })
            }),
        );
    }

    // tasks.add {title, scope?, session_id?, parent_id?, priority?, labels?,
    //            tools?, due_at?, recurrence?, depends_on?, acceptance?,
    //            project?, assignee?, description?}
    {
        let storage = storage.clone();
        let workspace_id = workspace_id.clone();
        services.insert(
            "tasks.add".to_string(),
            Arc::new(move |params: Value| {
                let storage = storage.clone();
                let workspace_id = workspace_id.clone();
                Box::pin(async move {
                    // A subtask always lives in its parent's scope — replan
                    // steps only know the parent id, not the run's scope.
                    let parent_id = get_i64(&params, "parent_id");
                    let (scope, scope_id, parent_sort) = if let Some(parent_id) = parent_id {
                        let parent = storage.tasks().get(parent_id).await.map_err(err_str)?;
                        (parent.scope, parent.scope_id, Some(parent.sort_order))
                    } else {
                        let (scope, scope_id) = resolve_scope(&params, &workspace_id).await?;
                        (scope, scope_id, None)
                    };
                    let title = opt_string(&params, "title")
                        .or_else(|| opt_string(&params, "text"))
                        .ok_or_else(|| "title is required".to_string())?;
                    // Ordering: a subtask inherits its parent's ladder
                    // position; a new root task appends AFTER everything
                    // (defaulting to 0 would jump the whole queue — observed
                    // live as a task explosion drowning the seeded plan).
                    let sort_order = match get_i64(&params, "sort_order") {
                        Some(explicit) => explicit,
                        None => match parent_sort {
                            Some(parent_sort) => parent_sort,
                            None => storage
                                .tasks()
                                .list(&scope, scope_id.as_deref(), true)
                                .await
                                .map_err(err_str)?
                                .iter()
                                .map(|t| t.sort_order)
                                .max()
                                .unwrap_or(0)
                                .saturating_add(1),
                        },
                    };
                    let new = NewTask {
                        parent_id,
                        scope,
                        scope_id,
                        project: opt_string(&params, "project"),
                        title,
                        description: opt_string(&params, "description"),
                        priority: get_i64(&params, "priority").unwrap_or(3),
                        labels: string_vec(params.get("labels")),
                        tool_scope: string_vec(params.get("tools")),
                        due_at: opt_string(&params, "due_at"),
                        recurrence: opt_string(&params, "recurrence"),
                        depends_on: i64_vec(params.get("depends_on")),
                        acceptance: canonical_acceptance(&params)?,
                        assignee: opt_string(&params, "assignee"),
                        sort_order,
                    };
                    let task = storage.tasks().create(new).await.map_err(err_str)?;
                    Ok(json!({ "task": task_to_json(&task) }))
                })
            }),
        );
    }

    // tasks.update {id, ...patch}
    {
        let storage = storage.clone();
        services.insert(
            "tasks.update".to_string(),
            Arc::new(move |params: Value| {
                let storage = storage.clone();
                Box::pin(async move {
                    let id = get_i64(&params, "id").ok_or_else(|| "id is required".to_string())?;
                    // Null/absent/mistyped values SKIP a field, never wipe it:
                    // the Boa bridge serializes `undefined` object members as
                    // null, so a partial update from the tool must not clear
                    // every field it did not mention. (Clearing a field is a
                    // deliberate op this service intentionally does not expose.)
                    let patch = TaskPatch {
                        title: opt_string(&params, "title").or_else(|| opt_string(&params, "text")),
                        description: params
                            .get("description")
                            .and_then(Value::as_str)
                            .map(|s| Some(s.to_string())),
                        status: opt_string(&params, "status"),
                        priority: get_i64(&params, "priority"),
                        labels: params
                            .get("labels")
                            .filter(|v| v.is_array())
                            .map(|v| string_vec(Some(v))),
                        tool_scope: params
                            .get("tools")
                            .filter(|v| v.is_array())
                            .map(|v| string_vec(Some(v))),
                        due_at: params
                            .get("due_at")
                            .and_then(Value::as_str)
                            .map(|s| Some(s.to_string())),
                        recurrence: params
                            .get("recurrence")
                            .and_then(Value::as_str)
                            .map(|s| Some(s.to_string())),
                        depends_on: params
                            .get("depends_on")
                            .filter(|v| v.is_array())
                            .map(|v| i64_vec(Some(v))),
                        acceptance: canonical_acceptance(&params)?.map(Some),
                        assignee: params
                            .get("assignee")
                            .and_then(Value::as_str)
                            .map(|s| Some(s.to_string())),
                        parent_id: get_i64(&params, "parent_id").map(Some),
                        project: params
                            .get("project")
                            .and_then(Value::as_str)
                            .map(|s| Some(s.to_string())),
                        sort_order: get_i64(&params, "sort_order"),
                    };
                    let actor = opt_string(&params, "actor");
                    let task = storage
                        .tasks()
                        .update(id, patch, actor.as_deref())
                        .await
                        .map_err(err_str)?;
                    Ok(json!({ "task": task_to_json(&task) }))
                })
            }),
        );
    }

    // tasks.done {id, actor?, workdir?} — runs the acceptance check first:
    // done is a verdict, not an assertion (the P14 anti-drift keystone).
    {
        let storage = storage.clone();
        services.insert(
            "tasks.done".to_string(),
            Arc::new(move |params: Value| {
                let storage = storage.clone();
                Box::pin(async move {
                    let id = get_i64(&params, "id").ok_or_else(|| "id is required".to_string())?;
                    let actor = opt_string(&params, "actor");
                    let task = storage.tasks().get(id).await.map_err(err_str)?;

                    let mut verified = false;
                    let mut verdict_detail = Value::Null;
                    if let Some(acceptance) = &task.acceptance {
                        let check = AcceptanceCheck::from_json(acceptance)?;
                        let workdir = opt_string(&params, "workdir")
                            .map_or_else(|| PathBuf::from("."), PathBuf::from);
                        let verdict = check.run(&workdir).await;
                        storage
                            .tasks()
                            .log_activity(
                                id,
                                actor.as_deref(),
                                "acceptance_checked",
                                Some(json!({
                                    "passed": verdict.passed,
                                    "detail": verdict.detail,
                                })),
                            )
                            .await
                            .map_err(err_str)?;
                        if !verdict.passed {
                            return Ok(json!({
                                "done": false,
                                "verdict": verdict.detail,
                                "message": format!(
                                    "Acceptance check failed — task #{id} is NOT done: {}",
                                    verdict.detail
                                ),
                            }));
                        }
                        verified = true;
                        verdict_detail = json!(verdict.detail);
                    }

                    let outcome = storage
                        .tasks()
                        .complete(
                            id,
                            actor.as_deref(),
                            Some(json!({ "verified": verified, "verdict": verdict_detail })),
                        )
                        .await
                        .map_err(err_str)?;
                    Ok(json!({
                        "done": true,
                        "verified": verified,
                        "already_done": outcome.already_done,
                        "auto_completed": outcome.auto_completed,
                    }))
                })
            }),
        );
    }

    // tasks.list {scope?, session_id?, include_done?}
    {
        let storage = storage.clone();
        let workspace_id = workspace_id.clone();
        services.insert(
            "tasks.list".to_string(),
            Arc::new(move |params: Value| {
                let storage = storage.clone();
                let workspace_id = workspace_id.clone();
                Box::pin(async move {
                    let (scope, scope_id) = resolve_scope(&params, &workspace_id).await?;
                    let include_done = params
                        .get("include_done")
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    let tasks = storage
                        .tasks()
                        .list(&scope, scope_id.as_deref(), include_done)
                        .await
                        .map_err(err_str)?;
                    Ok(json!({ "tasks": tasks.iter().map(task_to_json).collect::<Vec<_>>() }))
                })
            }),
        );
    }

    // tasks.query {filter, scope?, session_id?}
    {
        let storage = storage.clone();
        let workspace_id = workspace_id.clone();
        services.insert(
            "tasks.query".to_string(),
            Arc::new(move |params: Value| {
                let storage = storage.clone();
                let workspace_id = workspace_id.clone();
                Box::pin(async move {
                    let (scope, scope_id) = resolve_scope(&params, &workspace_id).await?;
                    let filter = opt_string(&params, "filter")
                        .ok_or_else(|| "filter is required".to_string())?;
                    let tasks = storage
                        .tasks()
                        .query(&scope, scope_id.as_deref(), &filter)
                        .await
                        .map_err(err_str)?;
                    Ok(json!({ "tasks": tasks.iter().map(task_to_json).collect::<Vec<_>>() }))
                })
            }),
        );
    }

    // tasks.note {id, content, author?}
    {
        let storage = storage.clone();
        services.insert(
            "tasks.note".to_string(),
            Arc::new(move |params: Value| {
                let storage = storage.clone();
                Box::pin(async move {
                    let id = get_i64(&params, "id").ok_or_else(|| "id is required".to_string())?;
                    let content = opt_string(&params, "content")
                        .or_else(|| opt_string(&params, "text"))
                        .ok_or_else(|| "content is required".to_string())?;
                    let author = opt_string(&params, "author");
                    let note = storage
                        .tasks()
                        .add_note(id, author.as_deref(), &content)
                        .await
                        .map_err(err_str)?;
                    Ok(json!({ "note_id": note.id }))
                })
            }),
        );
    }

    // tasks.remove {id}
    {
        let storage = storage.clone();
        services.insert(
            "tasks.remove".to_string(),
            Arc::new(move |params: Value| {
                let storage = storage.clone();
                Box::pin(async move {
                    let id = get_i64(&params, "id").ok_or_else(|| "id is required".to_string())?;
                    let actor = opt_string(&params, "actor");
                    let removed = storage
                        .tasks()
                        .delete(id, actor.as_deref())
                        .await
                        .map_err(err_str)?;
                    Ok(json!({ "removed": removed }))
                })
            }),
        );
    }

    // tasks.clear {scope?, session_id?, closed_only?}
    {
        let storage = storage.clone();
        let workspace_id = workspace_id.clone();
        services.insert(
            "tasks.clear".to_string(),
            Arc::new(move |params: Value| {
                let storage = storage.clone();
                let workspace_id = workspace_id.clone();
                Box::pin(async move {
                    let (scope, scope_id) = resolve_scope(&params, &workspace_id).await?;
                    let closed_only = params
                        .get("closed_only")
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    let removed = storage
                        .tasks()
                        .clear(&scope, scope_id.as_deref(), closed_only)
                        .await
                        .map_err(err_str)?;
                    Ok(json!({ "removed": removed }))
                })
            }),
        );
    }

    // tasks.counts {scope?, session_id?}
    {
        let storage = storage.clone();
        let workspace_id = workspace_id;
        services.insert(
            "tasks.counts".to_string(),
            Arc::new(move |params: Value| {
                let storage = storage.clone();
                let workspace_id = workspace_id.clone();
                Box::pin(async move {
                    let (scope, scope_id) = resolve_scope(&params, &workspace_id).await?;
                    let (open, closed) = storage
                        .tasks()
                        .counts(&scope, scope_id.as_deref())
                        .await
                        .map_err(err_str)?;
                    Ok(json!({ "open": open, "closed": closed }))
                })
            }),
        );
    }

    // tasks.import {session_id, items: [{text, status}]} — v0.1 JSON migration
    {
        let storage = storage;
        services.insert(
            "tasks.import".to_string(),
            Arc::new(move |params: Value| {
                let storage = storage.clone();
                Box::pin(async move {
                    let session_id = opt_string(&params, "session_id")
                        .ok_or_else(|| "session_id is required".to_string())?;
                    let items: Vec<(String, String)> = params
                        .get("items")
                        .and_then(Value::as_array)
                        .map(|arr| {
                            arr.iter()
                                .map(|item| {
                                    (
                                        item.get("text")
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                            .to_string(),
                                        item.get("status")
                                            .and_then(Value::as_str)
                                            .unwrap_or("pending")
                                            .to_string(),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let imported = storage
                        .tasks()
                        .import_v01(&session_id, &items)
                        .await
                        .map_err(err_str)?;
                    info!(session_id = %session_id, imported, "Migrated v0.1 todo JSON into task store");
                    Ok(json!({ "imported": imported }))
                })
            }),
        );
    }

    services
}

// ---------------------------------------------------------------------------
// Harness trait implementations
// ---------------------------------------------------------------------------

/// The P15 store as the harness's task source, scoped to one run.
pub struct TursoTaskSource {
    storage: Arc<Storage>,
    scope: String,
    scope_id: Option<String>,
    actor: String,
    event_tx: Option<tokio::sync::broadcast::Sender<Event>>,
}

impl TursoTaskSource {
    #[must_use]
    pub const fn new(
        storage: Arc<Storage>,
        scope: String,
        scope_id: Option<String>,
        actor: String,
        event_tx: Option<tokio::sync::broadcast::Sender<Event>>,
    ) -> Self {
        Self {
            storage,
            scope,
            scope_id,
            actor,
            event_tx,
        }
    }

    fn emit(&self, task_id: i64, kind: &str, detail: Value) {
        if let Some(tx) = &self.event_tx {
            let _ = tx.send(Event::TaskRunProgress {
                scope: self.scope.clone(),
                scope_id: self.scope_id.clone(),
                task_id: Some(task_id),
                kind: kind.to_string(),
                detail,
            });
        }
    }
}

#[async_trait::async_trait]
impl TaskSource for TursoTaskSource {
    async fn next(&self) -> Result<Option<TaskStep>, String> {
        let repo = self.storage.tasks();
        // Write-time canonicalization should make malformed acceptance JSON
        // impossible, but legacy or hand-edited rows must not wedge the run:
        // close them visibly and move on. Bounded: every malformed item is
        // cancelled, strictly shrinking the open set.
        for _ in 0..TASK_NEXT_SKIP_MAX {
            let task = repo
                .next(&self.scope, self.scope_id.as_deref())
                .await
                .map_err(|e| e.to_string())?;
            let Some(task) = task else { return Ok(None) };
            let acceptance = match &task.acceptance {
                Some(value) => match AcceptanceCheck::from_json(value) {
                    Ok(check) => Some(check),
                    Err(e) => {
                        let _ = repo
                            .log_activity(
                                task.id,
                                Some(&self.actor),
                                "acceptance_invalid",
                                Some(json!({ "error": e })),
                            )
                            .await;
                        let _ = repo
                            .update(
                                task.id,
                                TaskPatch {
                                    status: Some("cancelled".to_string()),
                                    ..TaskPatch::default()
                                },
                                Some(&self.actor),
                            )
                            .await;
                        self.emit(
                            task.id,
                            "abandoned",
                            json!({ "reason": format!("invalid acceptance check: {e}") }),
                        );
                        continue;
                    }
                },
                None => None,
            };
            let notes = repo
                .notes(task.id, STEP_NOTES_TAIL)
                .await
                .map_err(|e| e.to_string())?;
            return Ok(Some(TaskStep {
                id: task.id,
                title: task.title,
                description: task.description,
                acceptance,
                tool_scope: task.tool_scope,
                notes_tail: notes.into_iter().map(|n| n.content).collect(),
            }));
        }
        Err(format!(
            "gave up after cancelling {TASK_NEXT_SKIP_MAX} tasks with malformed acceptance checks"
        ))
    }

    async fn start(&self, id: i64) -> Result<(), String> {
        let repo = self.storage.tasks();
        let task = repo.get(id).await.map_err(|e| e.to_string())?;
        if task.status == "pending" {
            repo.update(
                id,
                TaskPatch {
                    status: Some("in_progress".to_string()),
                    ..TaskPatch::default()
                },
                Some(&self.actor),
            )
            .await
            .map_err(|e| e.to_string())?;
            self.emit(id, "started", Value::Null);
        }
        Ok(())
    }

    async fn complete(&self, id: i64, detail: Value) -> Result<(), String> {
        self.storage
            .tasks()
            .complete(id, Some(&self.actor), Some(detail.clone()))
            .await
            .map_err(|e| e.to_string())?;
        self.emit(id, "completed", detail);
        Ok(())
    }

    async fn add_note(&self, id: i64, content: &str) -> Result<(), String> {
        self.storage
            .tasks()
            .add_note(id, Some(&self.actor), content)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn log(&self, id: i64, action: &str, detail: Value) -> Result<(), String> {
        self.storage
            .tasks()
            .log_activity(id, Some(&self.actor), action, Some(detail.clone()))
            .await
            .map_err(|e| e.to_string())?;
        self.emit(id, action, detail);
        Ok(())
    }

    async fn abandon(&self, id: i64, reason: &str) -> Result<(), String> {
        let repo = self.storage.tasks();
        repo.update(
            id,
            TaskPatch {
                status: Some("cancelled".to_string()),
                ..TaskPatch::default()
            },
            Some(&self.actor),
        )
        .await
        .map_err(|e| e.to_string())?;
        repo.log_activity(
            id,
            Some(&self.actor),
            "abandoned",
            Some(json!({ "reason": reason })),
        )
        .await
        .map_err(|e| e.to_string())?;
        self.emit(id, "abandoned", json!({ "reason": reason }));
        Ok(())
    }
}

/// Runs one harness step as a fresh `Agent` with an isolated context — the
/// re-anchor. Mirrors `AgentSpawnerImpl` construction.
pub struct AgentStepRunner {
    pub router: Arc<LlmRouter>,
    pub tools: Arc<nanna_tools::ToolRegistry>,
    pub agent_config: nanna_agent::AgentConfig,
    pub system_prompt: String,
    pub workspace_root: Option<PathBuf>,
    pub stats: Option<nanna_agent::ModelStatsTracker>,
}

/// In-step retries for transient provider errors.
///
/// Bound justification: local models corrupt their own tool-call template
/// mid-generation (observed: Ollama 500s from qwen3.5), and Ollama's runner
/// intermittently enters a degraded state where a stale KV "context
/// checkpoint" restore sends generation straight to a stop token (observed:
/// 200s with ~33 generated tokens and empty output, recurring ~1h into a
/// sustained run). Three retries with escalating backoff — and a runner
/// reset before the last — absorb both without masking a dead endpoint; the
/// harness circuit breaker still sees persistent failure.
const STEP_LLM_RETRIES: usize = 3;

/// Backoff before retry attempts 1..=3.
const STEP_RETRY_BACKOFF_SECS: [u64; 3] = [2, 5, 10];

fn is_transient_llm_error(message: &str) -> bool {
    message.contains("API error: 5")
        || message.contains("timed out")
        || message.contains("connection")
}

/// Forensics: append the exact prompt of an empty-completion step to a temp
/// file so the deterministic trigger can be replayed and minimized offline.
fn dump_empty_step(request: &StepRequest, attempt: usize) {
    use std::io::Write;
    let path = std::env::temp_dir().join("nanna_empty_step_prompts.log");
    let entry = format!(
        "==== {} item#{} kind={:?} attempt={attempt} ====\n{}\n\n",
        chrono::Utc::now().to_rfc3339(),
        request.item_id,
        request.step_kind,
        request.prompt,
    );
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| f.write_all(entry.as_bytes()));
}

/// An "empty completion": HTTP success but no text, no tool calls, and ~no
/// generated tokens. Observed live from Ollama (a whole 42-item plan was
/// burned by 462 such no-op steps in 9 minutes — each one "succeeded", made
/// no progress, and marched every item to abandonment). Treat as a transient
/// provider failure, never as a step result. The token bound distinguishes
/// this from a legitimate thinking-only step, whose reasoning tokens count.
fn is_empty_completion(outcome: &StepOutcome) -> bool {
    outcome.tool_calls.is_empty() && outcome.text.trim().is_empty() && outcome.output_tokens <= 8
}

#[async_trait::async_trait]
impl StepRunner for AgentStepRunner {
    async fn run_step(&self, request: StepRequest) -> Result<StepOutcome, String> {
        let mut last_err = String::new();
        for attempt in 0..=STEP_LLM_RETRIES {
            if attempt > 0 {
                tracing::warn!(attempt, error = %last_err, "retrying step after transient LLM error");
                let backoff = STEP_RETRY_BACKOFF_SECS[attempt - 1];
                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                // The degraded-runner state (empty 200s) survives plain
                // retries — force a model unload/reload before the last try.
                if attempt == STEP_LLM_RETRIES && last_err.contains("empty completion") {
                    self.reset_ollama_runner().await;
                }
            }
            // A fresh context per attempt: the re-anchor makes retries free —
            // there is no partial transcript worth salvaging.
            match self.try_run_step(&request).await {
                Ok(outcome) if is_empty_completion(&outcome) => {
                    dump_empty_step(&request, attempt);
                    last_err =
                        "empty completion (no text, no tool calls, ~0 tokens) from provider"
                            .to_string();
                }
                Ok(outcome) => return Ok(outcome),
                Err(e) if is_transient_llm_error(&e) => last_err = e,
                Err(e) => return Err(e),
            }
        }
        Err(last_err)
    }
}

impl AgentStepRunner {
    /// Whether this runner's model is served by the local Ollama instance.
    /// Healing must be provider-aware: a `:free` suffix on an OpenRouter
    /// model id must never trigger local-server surgery.
    pub fn is_ollama_model(&self) -> bool {
        crate::llm_router::ProviderId::from_model(&self.agent_config.model)
            == crate::llm_router::ProviderId::Ollama
    }

    /// Force Ollama to unload the model (`keep_alive: 0`), clearing runner
    /// state — the observed degraded mode restores a stale KV context
    /// checkpoint that sends every generation straight to a stop token, and
    /// only a fresh runner clears it. No-op for non-Ollama models.
    pub async fn reset_ollama_runner(&self) {
        let model = &self.agent_config.model;
        if !self.is_ollama_model() {
            return;
        }
        let base =
            std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string());
        tracing::warn!(model = %model, "resetting Ollama runner (keep_alive=0) after empty completions");
        let client = reqwest::Client::new();
        let _ = client
            .post(format!("{base}/api/generate"))
            .json(&serde_json::json!({
                "model": LlmRouter::strip_model_prefix(model),
                "keep_alive": 0
            }))
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await;
        // Give the runner a moment to tear down before the reload request.
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }

    async fn try_run_step(&self, request: &StepRequest) -> Result<StepOutcome, String> {
        use nanna_agent::{Agent, AgentContext, RunOptions};

        let mut context = AgentContext::new(uuid::Uuid::new_v4().to_string())
            .with_system_prompt(&self.system_prompt);
        if let Some(ws_root) = &self.workspace_root {
            context.workspace_root = Some(ws_root.clone());
        }

        let mut config = self.agent_config.clone();
        // Execution/verification wants determinism (pass^k reliability on a
        // small model); planning keeps the configured creative temperature.
        if matches!(
            request.step_kind,
            nanna_agent::harness::StepKind::Execute | nanna_agent::harness::StepKind::Verify
        ) {
            config.temperature = config.temperature.min(0.3);
        }
        let model_display = config.model.clone();
        let llm_client = self
            .router
            .client_for_model(&config.model)
            .ok_or_else(|| format!("No provider available for model '{model_display}'"))?;
        config.model = LlmRouter::strip_model_prefix(&config.model);

        let mut agent = Agent::new(config, llm_client, self.tools.clone()).with_context(context);
        if let Some(tracker) = &self.stats {
            agent = agent.with_stats(tracker.clone());
        }

        // The step is always allowed the todo tool (notes are its only
        // memory) on top of the item's scoped tools.
        let mut active = request.tool_scope.clone();
        if !active.iter().any(|t| t == "todo") {
            active.push("todo".to_string());
        }

        let options = RunOptions {
            max_iterations: request.max_iterations,
            token_budget: request.token_budget,
            max_wall_clock: request.max_wall_clock,
            step_kind: Some(request.step_kind),
            initial_active_tools: active,
            is_sub_agent: true,
            ..Default::default()
        };

        let result = agent
            .run(&request.prompt, options)
            .await
            .map_err(|e| format!("step error: {e}"))?;

        let tool_calls = result
            .tool_calls
            .iter()
            .map(|record| StepToolCall {
                name: record.name.clone(),
                input_digest: digest(&record.input.to_string()),
                output_digest: digest(&record.output),
            })
            .collect();

        Ok(StepOutcome {
            text: result.text,
            input_tokens: u64::from(result.input_tokens),
            output_tokens: u64::from(result.output_tokens),
            tool_calls,
        })
    }
}

// ---------------------------------------------------------------------------
// Recurrence sweep
// ---------------------------------------------------------------------------

/// Reopen completed recurring tasks whose next cron occurrence has arrived.
///
/// Driven by the daemon scheduler (P8) — one recurrence engine, not two: the
/// task stores the cron expression, the scheduler provides the clock.
/// Returns the number of tasks reopened.
pub async fn sweep_recurrences(storage: &Arc<Storage>) -> usize {
    let repo = storage.tasks();
    let tasks = match repo.list_recurring_closed().await {
        Ok(tasks) => tasks,
        Err(e) => {
            tracing::warn!("recurrence sweep failed to list tasks: {e}");
            return 0;
        }
    };
    let now = chrono::Utc::now();
    let mut reopened = 0usize;
    for task in tasks {
        let Some(expr_str) = &task.recurrence else {
            continue;
        };
        let expr = match nanna_core::CronExpr::parse(expr_str) {
            Ok(expr) => expr,
            Err(e) => {
                tracing::warn!(task_id = task.id, "invalid recurrence '{expr_str}': {e}");
                continue;
            }
        };
        let completed = task
            .completed_at
            .as_deref()
            .and_then(parse_db_time)
            .unwrap_or(now);
        if expr.next(&completed).is_some_and(|next| next <= now)
            && repo.reopen(task.id, Some("recurrence")).await.is_ok()
        {
            info!(task_id = task.id, "recurring task reopened");
            reopened += 1;
        }
    }
    reopened
}

/// Parse a stored timestamp: RFC3339 first, then turso's
/// `datetime('now')` format (`YYYY-MM-DD HH:MM:SS`, UTC).
fn parse_db_time(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|naive| naive.and_utc())
        })
}

/// Cheap stable digest for repetition comparison (not cryptographic).
fn digest(content: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

// ---------------------------------------------------------------------------
// Run manager
// ---------------------------------------------------------------------------

/// Restart the local Ollama server: kill `ollama.exe` only (the tray
/// supervisor respawns it) and wait for the API to come back.
///
/// This is the cure for the sticky degraded-runner state (every generation
/// aborted with `done:false`; model unloads do not clear it — verified live).
/// Callers gate it: bouncing a shared local service is an operator decision.
pub async fn restart_ollama_server() -> bool {
    tracing::warn!("restarting the Ollama server (degraded runner state)");
    #[cfg(windows)]
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/IM", "ollama.exe"])
        .output();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("pkill")
        .args(["-x", "ollama"])
        .output();
    let base =
        std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string());
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let up = reqwest::Client::new()
            .get(format!("{base}/api/version"))
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success());
        if up {
            info!("Ollama is back after restart");
            return true;
        }
    }
    tracing::warn!("Ollama did not come back within 60s of restart");
    false
}

/// Whether runs may restart the Ollama server as last-resort healing.
/// ON by default (owner decision): the degraded state bricks every client of
/// the server anyway, and a restart is the only known cure. Set
/// `NANNA_OLLAMA_RESTART_ON_DEGRADED=0` to opt out on setups where bouncing
/// the shared server is unacceptable.
fn ollama_restart_allowed() -> bool {
    std::env::var("NANNA_OLLAMA_RESTART_ON_DEGRADED").as_deref() != Ok("0")
}

/// Fold per-segment reports into one run-level report: counters sum, the
/// stop reason is the final segment's, and tokens-per-item is recomputed
/// over the aggregate.
fn fold_reports(segments: &[LongHorizonReport]) -> LongHorizonReport {
    let mut folded = segments.last().cloned().unwrap_or(LongHorizonReport {
        stop: StopReason::AllTasksDone,
        steps_taken: 0,
        items_completed: 0,
        items_completed_unverified: 0,
        items_abandoned: 0,
        replans: 0,
        false_success_claims: 0,
        input_tokens: 0,
        output_tokens: 0,
        wall_clock_secs: 0,
        tokens_per_completed_item: None,
    });
    folded.steps_taken = segments.iter().map(|r| r.steps_taken).sum();
    folded.items_completed = segments.iter().map(|r| r.items_completed).sum();
    folded.items_completed_unverified =
        segments.iter().map(|r| r.items_completed_unverified).sum();
    folded.items_abandoned = segments.iter().map(|r| r.items_abandoned).sum();
    folded.replans = segments.iter().map(|r| r.replans).sum();
    folded.false_success_claims = segments.iter().map(|r| r.false_success_claims).sum();
    folded.input_tokens = segments.iter().map(|r| r.input_tokens).sum();
    folded.output_tokens = segments.iter().map(|r| r.output_tokens).sum();
    folded.wall_clock_secs = segments.iter().map(|r| r.wall_clock_secs).sum();
    folded.tokens_per_completed_item = if folded.items_completed > 0 {
        Some((folded.input_tokens + folded.output_tokens) / folded.items_completed as u64)
    } else {
        None
    };
    folded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(
        steps: usize,
        completed: usize,
        tokens_in: u64,
        tokens_out: u64,
        stop: StopReason,
    ) -> LongHorizonReport {
        LongHorizonReport {
            stop,
            steps_taken: steps,
            items_completed: completed,
            items_completed_unverified: 0,
            items_abandoned: 0,
            replans: 0,
            false_success_claims: 0,
            input_tokens: tokens_in,
            output_tokens: tokens_out,
            wall_clock_secs: 60,
            tokens_per_completed_item: None,
        }
    }

    #[test]
    fn fold_sums_counters_and_keeps_the_final_stop() {
        let folded = fold_reports(&[
            seg(
                10,
                3,
                1000,
                100,
                StopReason::RunnerErrors {
                    message: "x".to_string(),
                },
            ),
            seg(5, 2, 500, 50, StopReason::AllTasksDone),
        ]);
        assert_eq!(folded.steps_taken, 15);
        assert_eq!(folded.items_completed, 5);
        assert_eq!(folded.input_tokens, 1500);
        assert_eq!(folded.wall_clock_secs, 120);
        assert_eq!(folded.stop, StopReason::AllTasksDone, "final segment's stop wins");
        assert_eq!(
            folded.tokens_per_completed_item,
            Some(330),
            "recomputed over the aggregate (1650/5)"
        );
    }

    #[test]
    fn fold_with_zero_completions_has_no_per_item_rate() {
        let folded = fold_reports(&[seg(4, 0, 100, 10, StopReason::WallClockExhausted)]);
        assert_eq!(folded.tokens_per_completed_item, None);
    }

    #[test]
    fn fold_of_nothing_is_a_benign_empty_report() {
        let folded = fold_reports(&[]);
        assert_eq!(folded.steps_taken, 0);
        assert_eq!(folded.stop, StopReason::AllTasksDone);
    }
}

/// A live long-horizon run.
struct ActiveRun {
    cancel: Arc<AtomicBool>,
    goal: String,
    started_at: chrono::DateTime<chrono::Utc>,
}

/// Status snapshot returned over IPC.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RunStatus {
    pub running: bool,
    pub goal: Option<String>,
    pub started_at: Option<String>,
    pub last_report: Option<LongHorizonReport>,
    /// Provider incidents healed by resuming the plan (0 when none).
    pub resumes: usize,
}

/// Starts, cancels, and reports background long-horizon runs — one per scope
/// key at a time (the store serializes the plan; two runners over one plan
/// would race next()).
#[derive(Default)]
pub struct TaskRunManager {
    runs: RwLock<HashMap<String, ActiveRun>>,
    reports: RwLock<HashMap<String, (LongHorizonReport, usize)>>,
}

impl TaskRunManager {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn scope_key(scope: &str, scope_id: Option<&str>) -> String {
        format!("{scope}:{}", scope_id.unwrap_or(""))
    }

    /// Spawn a background run. Errors if one is already active for the scope.
    #[allow(clippy::too_many_arguments)]
    pub async fn start(
        self: &Arc<Self>,
        goal: String,
        source: TursoTaskSource,
        runner: AgentStepRunner,
        config: LongHorizonConfig,
        workdir: PathBuf,
        event_tx: tokio::sync::broadcast::Sender<Event>,
    ) -> Result<(), String> {
        let key = Self::scope_key(&source.scope, source.scope_id.as_deref());
        {
            let mut runs = self.runs.write().await;
            if runs.contains_key(&key) {
                return Err(format!("a task run is already active for scope {key}"));
            }
            runs.insert(
                key.clone(),
                ActiveRun {
                    cancel: Arc::new(AtomicBool::new(false)),
                    goal: goal.clone(),
                    started_at: chrono::Utc::now(),
                },
            );
        }
        let cancel = {
            let runs = self.runs.read().await;
            runs.get(&key).map(|r| r.cancel.clone())
        }
        .ok_or_else(|| "run vanished during start".to_string())?;

        let scope = source.scope.clone();
        let scope_id = source.scope_id.clone();
        let _ = event_tx.send(Event::TaskRunStarted {
            scope: scope.clone(),
            scope_id: scope_id.clone(),
            goal: goal.clone(),
        });

        let manager = self.clone();
        tokio::spawn(async move {
            // Bounded auto-resume (standard for every model): the task store
            // IS the checkpoint, so a run stopped by a provider incident is
            // resumed by simply starting again — next() picks up exactly
            // where the plan stands. Bound: 8 resumes tolerates an incident
            // every ~40 minutes of a long run without letting a permanently
            // dead provider spin forever.
            const RUN_RESUMES_MAX: usize = 8;
            let started = std::time::Instant::now();
            let mut segments: Vec<LongHorizonReport> = Vec::new();
            let mut resumes = 0usize;
            loop {
                let remaining = config.max_wall_clock.saturating_sub(started.elapsed());
                let segment_config = LongHorizonConfig {
                    max_wall_clock: remaining,
                    ..config.clone()
                };
                let report = LongHorizonRunner::new(segment_config)
                    .run(&goal, &source, &runner, &workdir, Some(cancel.clone()))
                    .await;
                let provider_died = matches!(report.stop, StopReason::RunnerErrors { .. });
                segments.push(report);
                if !provider_died
                    || resumes >= RUN_RESUMES_MAX
                    || started.elapsed() >= config.max_wall_clock
                {
                    break;
                }
                resumes += 1;
                tracing::warn!(
                    scope = %scope,
                    resumes,
                    "provider incident — healing and resuming the plan"
                );
                let _ = event_tx.send(Event::TaskRunProgress {
                    scope: scope.clone(),
                    scope_id: scope_id.clone(),
                    task_id: None,
                    kind: "resumed".to_string(),
                    detail: serde_json::json!({ "resumes": resumes }),
                });
                // Healing ladder — provider-aware. Local Ollama: server
                // restart (unless opted out), else runner reset. Cloud
                // providers (incl. the openrouter/free auto-router, where the
                // serving model varies per request): nothing local to heal —
                // the pause + resume + in-step retries ARE the healing.
                if runner.is_ollama_model()
                    && !(ollama_restart_allowed() && restart_ollama_server().await)
                {
                    runner.reset_ollama_runner().await;
                }
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            }
            let report = fold_reports(&segments);
            info!(
                scope = %scope,
                stop = ?report.stop,
                items_completed = report.items_completed,
                tokens_per_item = ?report.tokens_per_completed_item,
                resumes,
                "Long-horizon run finished"
            );
            let _ = event_tx.send(Event::TaskRunCompleted {
                scope: scope.clone(),
                scope_id: scope_id.clone(),
                report: serde_json::to_value(&report).unwrap_or(Value::Null),
            });
            manager.runs.write().await.remove(&key);
            manager.reports.write().await.insert(key, (report, resumes));
        });
        Ok(())
    }

    /// Request cancellation. Returns false when no run is active.
    pub async fn cancel(&self, scope: &str, scope_id: Option<&str>) -> bool {
        let key = Self::scope_key(scope, scope_id);
        let runs = self.runs.read().await;
        runs.get(&key).is_some_and(|run| {
            run.cancel.store(true, Ordering::Relaxed);
            true
        })
    }

    /// Current status for a scope.
    pub async fn status(&self, scope: &str, scope_id: Option<&str>) -> RunStatus {
        let key = Self::scope_key(scope, scope_id);
        let runs = self.runs.read().await;
        let reports = self.reports.read().await;
        let (last_report, resumes) = reports
            .get(&key)
            .map_or((None, 0), |(report, resumes)| (Some(report.clone()), *resumes));
        runs.get(&key).map_or_else(
            || RunStatus {
                running: false,
                goal: None,
                started_at: None,
                last_report: last_report.clone(),
                resumes,
            },
            |run| RunStatus {
                running: true,
                goal: Some(run.goal.clone()),
                started_at: Some(run.started_at.to_rfc3339()),
                last_report: last_report.clone(),
                resumes,
            },
        )
    }
}
