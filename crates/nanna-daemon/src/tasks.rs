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
    AcceptanceCheck, Interjector, LongHorizonConfig, LongHorizonReport, LongHorizonRunner,
    StepOutcome, StepRequest, StepRunner, StepToolCall, StopReason, TaskSource, TaskStep,
};
use nanna_agent::planner::{Plan, build_plan_prompt, plan_or_fallback};
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

                    // Idempotent add: re-adding a title that is ALREADY open
                    // in this scope returns the existing item instead of a
                    // second copy. Observed live (lfm2.5 smoke): 5 seeded
                    // tasks became ~50 as the model re-decomposed the same
                    // work every step — "Write data file with header and 3
                    // rows" was created ten times — so the plan grew faster
                    // than it was worked and the real items drowned. Only
                    // OPEN items dedupe: a genuinely recurring chore
                    // ("run the tests") must still be addable after the
                    // previous one is closed, and reopening history would be
                    // the worse failure.
                    if let Some(existing) = storage
                        .tasks()
                        .list(&scope, scope_id.as_deref(), false)
                        .await
                        .map_err(err_str)?
                        .into_iter()
                        .find(|t| {
                            t.parent_id == parent_id
                                && t.title.trim().eq_ignore_ascii_case(title.trim())
                        })
                    {
                        return Ok(json!({
                            "task": task_to_json(&existing),
                            "deduplicated": true,
                            "note": format!(
                                "Task #{} \"{}\" already exists and is still open — reusing it \
                                 instead of creating a duplicate. Work on it rather than \
                                 planning it again.",
                                existing.id, existing.title
                            ),
                        }));
                    }
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

                    // A task carrying a machine-checkable acceptance check is
                    // a CONTRACT — someone (the planner, the eval, the user)
                    // defined what "done" means for it, and deleting it
                    // destroys the only objective record of the goal. Erasing
                    // it is never the right move: the honest outcomes are
                    // finish it or cancel it, both of which keep the item and
                    // its history. Observed live (lfm2.5 smoke): blocked on a
                    // refusing write_file and holding only `write_file` and
                    // `todo`, the model deleted a SEEDED plan item — the run
                    // then died on a task the harness still expected to
                    // verify. Scratch items the model invents carry no
                    // acceptance and stay freely removable.
                    let existing = storage.tasks().get(id).await.map_err(err_str)?;
                    if existing.acceptance.is_some() {
                        return Ok(json!({
                            "removed": false,
                            "refused": true,
                            "note": format!(
                                "Task #{} \"{}\" has a machine-checkable acceptance check, so it is \
                                 a commitment rather than scratch work and was NOT deleted — it is \
                                 fully intact. If the work is finished, complete it (the check runs \
                                 automatically). If it should not be done at all, set its status to \
                                 cancelled. Either way the record survives.",
                                existing.id, existing.title
                            ),
                        }));
                    }

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

                    // Same contract rule as tasks.remove, applied in bulk —
                    // and this is the path that actually bites. Observed live
                    // (lfm2.5 endurance, 2026-07-25): guarding only the
                    // per-id remove left `clear` wide open, and one call took
                    // the scope from 42 tasks to 6 mid-run, destroying 36
                    // seeded features the harness was still driving.
                    //
                    // Ancestors of a contract are protected too: `delete`
                    // removes whole SUBTREES, so clearing a scratch parent
                    // would take a contract-bearing child down with it.
                    let all = storage
                        .tasks()
                        .list(&scope, scope_id.as_deref(), true)
                        .await
                        .map_err(err_str)?;
                    let parents: HashMap<i64, Option<i64>> =
                        all.iter().map(|t| (t.id, t.parent_id)).collect();
                    let mut protected: std::collections::HashSet<i64> =
                        std::collections::HashSet::new();
                    for task in all.iter().filter(|t| t.acceptance.is_some()) {
                        let mut cursor = Some(task.id);
                        while let Some(id) = cursor {
                            if !protected.insert(id) {
                                break; // this ancestor chain is already marked
                            }
                            cursor = parents.get(&id).copied().flatten();
                        }
                    }

                    let mut removed = 0u64;
                    for task in &all {
                        if protected.contains(&task.id) {
                            continue;
                        }
                        if closed_only && task.status != "done" && task.status != "cancelled" {
                            continue;
                        }
                        // A subtree delete may already have taken this id;
                        // that is success, not an error.
                        if let Ok(count) = storage.tasks().delete(task.id, None).await {
                            removed += count;
                        }
                    }

                    let kept = protected.len();
                    Ok(json!({
                        "removed": removed,
                        "protected": kept,
                        "note": if kept > 0 {
                            format!(
                                "Cleared {removed} scratch task(s). {kept} task(s) carrying an \
                                 acceptance contract were KEPT — they define what \"done\" means \
                                 and are still intact. Complete or cancel those instead."
                            )
                        } else {
                            format!("Cleared {removed} task(s).")
                        },
                    }))
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
    /// Tools the model has discovered so far in THIS harness run.
    ///
    /// Shared across steps so `discover_tools` is paid once per tool rather
    /// than once per step — the runner is one object for the whole run, while
    /// each step gets a fresh `RunState`.
    pub discovered_tools: Arc<tokio::sync::RwLock<std::collections::HashSet<String>>>,
    pub router: Arc<LlmRouter>,
    pub tools: Arc<nanna_tools::ToolRegistry>,
    pub agent_config: nanna_agent::AgentConfig,
    pub system_prompt: String,
    pub workspace_root: Option<PathBuf>,
    pub stats: Option<nanna_agent::ModelStatsTracker>,
    /// Memory sink for tool results.
    ///
    /// The design is "a tool result goes to MEMORY, and only a stub goes to
    /// context — recall is how you get the full thing back". `loop_runner`
    /// implements exactly that, but only when `RunOptions::on_memory` is set,
    /// and it was set on the OLD direct-chat path only. When the harness
    /// became the single chat path this runner replaced that one without
    /// carrying the sink across, so the branch silently no-opped: a 4-hour
    /// run made 1315 exec + 501 read_file + 87 edit_file calls and stored
    /// NOTHING (observed 2026-07-27). Wiring it back restores the intended
    /// behaviour for every turn, since every turn is now a harness run.
    pub memory: Option<Arc<nanna_memory::MemoryService>>,
    /// Workspace scope for stored memories, so a run's observations belong to
    /// the workspace they happened in rather than leaking global.
    pub workspace_id: Option<String>,
    /// When set, every step streams its text and tool activity into the
    /// session's chat transcript ("show your work as you go"). None for
    /// background runs that have no transcript to show.
    pub chat_sink: Option<ChatSink>,
}

/// Streams a harness step into a chat session using the *existing* chat event
/// contract (`MessageDelta` / `ToolStart` / `ToolEnd`), so a long-horizon run
/// renders in the transcript with no protocol change on the GUI side.
///
/// When `run` is set (chat-backed runs), every callback ALSO fills the
/// registered [`crate::agent_service::ExternalRunHandle`] buffers, which is
/// what makes navigation recovery (`get_run_state`), Stop, and end-of-run
/// timeline persistence work — events alone vanish the moment the page
/// unmounts.
#[derive(Clone)]
pub struct ChatSink {
    pub session_id: String,
    pub message_id: String,
    pub event_tx: tokio::sync::broadcast::Sender<Event>,
    pub run: Option<crate::agent_service::ExternalRunHandle>,
    /// When set, every completed tool call is recorded to the shared stats
    /// tracker (and the Turso time-series when `storage` is also set) —
    /// parity with the retired direct chat path, which recorded after the
    /// run from the flat result.
    pub tool_stats: Option<nanna_agent::ToolStatsTracker>,
    pub storage: Option<Arc<Storage>>,
    /// The single item of a one-task (conversation-shaped) plan. Steps for
    /// this item render with no `**[working]**` banner so a plain question
    /// reads as a plain reply; items added later (interjections, replans)
    /// are announced — by then there IS a run to attribute work to.
    /// Shared (not per-clone) state: the finalizer sets it after seeding,
    /// and the step runner's clone must see it.
    pub quiet_item: Arc<std::sync::Mutex<Option<i64>>>,
}

impl ChatSink {
    pub(crate) fn delta(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        let _ = self.event_tx.send(Event::MessageDelta {
            session_id: self.session_id.clone(),
            message_id: self.message_id.clone(),
            delta: text.to_string(),
        });
        if let Some(run) = &self.run {
            // try_write mirrors the in-service path: a snapshot clone briefly
            // holding the lock must not block the stream thread.
            if let Ok(mut acc) = run.accumulated_text.try_write() {
                acc.push_str(text);
            }
            // The journal lock is std::sync and infallible by design (see
            // ActiveChat::timeline) — merge into the trailing Text item so a
            // token stream stays one item, not thousands.
            let mut journal = run.timeline.lock().expect("timeline lock poisoned");
            if let Some(crate::session::TimelineItem::Text { content, .. }) = journal.last_mut() {
                content.push_str(text);
            } else {
                journal.push(crate::session::TimelineItem::Text {
                    content: text.to_string(),
                    at: chrono::Utc::now().to_rfc3339(),
                });
            }
        }
    }

    fn thinking(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        let _ = self.event_tx.send(Event::ThinkingDelta {
            session_id: self.session_id.clone(),
            delta: text.to_string(),
        });
        if let Some(run) = &self.run {
            if let Ok(mut acc) = run.accumulated_thinking.try_write() {
                acc.push_str(text);
            }
            let mut journal = run.timeline.lock().expect("timeline lock poisoned");
            if let Some(crate::session::TimelineItem::Thinking { content, .. }) = journal.last_mut()
            {
                content.push_str(text);
            } else {
                journal.push(crate::session::TimelineItem::Thinking {
                    content: text.to_string(),
                    at: chrono::Utc::now().to_rfc3339(),
                });
            }
        }
    }

    fn tool_start(&self, call_id: &str, name: &str, input: &Value, model: Option<&str>) {
        let _ = self.event_tx.send(Event::ToolStart {
            session_id: self.session_id.clone(),
            call_id: call_id.to_string(),
            name: name.to_string(),
            input: input.clone(),
            model: model.map(String::from),
            tokens: None,
            total_tokens: None,
        });
        if let Some(run) = &self.run {
            if let Ok(mut active) = run.active_tool_calls.try_write() {
                active.push(crate::agent_service::ActiveToolCallInfo {
                    call_id: call_id.to_string(),
                    name: name.to_string(),
                    started_at: chrono::Utc::now(),
                });
            }
            run.timeline
                .lock()
                .expect("timeline lock poisoned")
                .push(crate::session::TimelineItem::Tool {
                    call_id: call_id.to_string(),
                    name: name.to_string(),
                    input: Some(input.clone()),
                    output: None,
                    success: None,
                    duration_ms: None,
                    tokens: None,
                    total_tokens: None,
                    at: chrono::Utc::now().to_rfc3339(),
                });
        }
    }

    fn tool_end(&self, call_id: &str, name: &str, output: &str, success: bool, duration_ms: u64) {
        let _ = self.event_tx.send(Event::ToolEnd {
            session_id: self.session_id.clone(),
            call_id: call_id.to_string(),
            output: output.to_string(),
            success,
            duration_ms,
            data: None,
        });
        if let Some(run) = &self.run {
            if let Ok(mut active) = run.active_tool_calls.try_write() {
                active.retain(|t| t.call_id != call_id);
            }
            if let Ok(mut done) = run.completed_tool_calls.try_write() {
                done.push(crate::agent_service::CompletedToolCallInfo {
                    call_id: call_id.to_string(),
                    name: name.to_string(),
                    output: output.to_string(),
                    success,
                    duration_ms,
                });
            }
            let mut journal = run.timeline.lock().expect("timeline lock poisoned");
            if let Some(crate::session::TimelineItem::Tool {
                output: slot_output,
                success: slot_success,
                duration_ms: slot_duration,
                ..
            }) = journal
                .iter_mut()
                .rev()
                .find(|item| matches!(item, crate::session::TimelineItem::Tool { call_id: id, .. } if id == call_id))
            {
                *slot_output = Some(output.to_string());
                *slot_success = Some(success);
                *slot_duration = Some(duration_ms);
            }
        }
        if let Some(stats) = &self.tool_stats {
            let observation = nanna_agent::ToolObservation {
                tool_name: name.to_string(),
                success,
                duration_ms,
                output_size: output.len(),
                error: (!success).then(|| output.to_string()),
                session_id: Some(self.session_id.clone()),
            };
            let stats = stats.clone();
            let storage = self.storage.clone();
            let session_id = self.session_id.clone();
            // Recording is async and this callback is sync — hand it off.
            tokio::spawn(async move {
                if let Some(storage) = storage {
                    if let Err(e) = storage
                        .log_tool_call(
                            &observation.tool_name,
                            observation.success,
                            observation.duration_ms,
                            observation.output_size,
                            observation.error.as_deref(),
                            Some(&session_id),
                        )
                        .await
                    {
                        tracing::warn!("Failed to log tool call to DB: {e}");
                    }
                }
                stats.record(observation).await;
            });
        }
    }

    /// Announce which item the run is starting, so the transcript reads as
    /// work-in-progress rather than a wall of unattributed text.
    /// The label is read back out of the step prompt (`build_step_prompt`
    /// writes `Task #id: title`); if that line is ever absent the header
    /// degrades to the bare item id rather than failing.
    fn step_header(&self, request: &StepRequest) {
        // Conversation-shaped turns (one-task plans) stay banner-free so the
        // transcript feels like chat — see the `quiet_item` field.
        if self
            .quiet_item
            .lock()
            .is_ok_and(|quiet| *quiet == Some(request.item_id))
        {
            return;
        }
        let kind = match request.step_kind {
            nanna_agent::harness::StepKind::Plan => "planning",
            nanna_agent::harness::StepKind::Verify => "verifying",
            nanna_agent::harness::StepKind::Execute => "working",
        };
        let label = if request.item_title.trim().is_empty() {
            format!("Task #{}", request.item_id)
        } else {
            request.item_title.clone()
        };

        // A step banner is run mechanics — it belongs in the journal the GUI
        // renders as status, NOT in the message body. Written as text it also
        // ended up in conversation history and was replayed to the model on
        // later turns as if it had said it.
        let _ = self.event_tx.send(Event::StepStarted {
            session_id: self.session_id.clone(),
            kind: kind.to_string(),
            label: label.clone(),
            item_id: request.item_id,
        });
        if let Some(run) = &self.run {
            run.timeline
                .lock()
                .expect("timeline lock poisoned")
                .push(crate::session::TimelineItem::Step {
                    phase: kind.to_string(),
                    label,
                    item_id: request.item_id,
                    at: chrono::Utc::now().to_rfc3339(),
                });
        }
    }
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

/// Transient provider faults worth retrying in place: 5xx (including the
/// synthesized 502 for aborted Ollama generations), client timeouts and
/// connection failures ("error sending request" is reqwest's send-phase
/// failure), and mid-stream drops ("Stream error:"). Shared by the step
/// runner and the chat path — both heal with the same ladder.
pub(crate) fn is_transient_llm_error(message: &str) -> bool {
    message.contains("API error: 5")
        || message.contains("timed out")
        || message.contains("connection")
        || message.contains("error sending request")
        || message.contains("Stream error:")
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

/// A step that neither acted nor said anything: reasoning tokens were spent,
/// but no tool ran and no text came back.
///
/// This is NOT a step result. A harness step exists to move an item, and
/// thinking alone moves nothing — yet it used to be accepted as an outcome
/// because the reasoning tokens put it over [`is_empty_completion`]'s bound.
/// Observed live 2026-07-27: the same item was re-entered SEVEN times, each
/// visit producing a "Thinking …" block and nothing else, and the run ended
/// "8 steps · 0 items completed" with the artifact untouched for 19 minutes.
///
/// Distinct from an empty completion (a degraded provider emitting ~nothing),
/// which is already handled: here the model is generating fine, it just did
/// not commit to an action, so the retry re-anchors it with an explicit nudge.
fn made_no_progress(outcome: &StepOutcome) -> bool {
    outcome.tool_calls.is_empty() && outcome.text.trim().is_empty()
}

/// Appended to a step prompt after a no-progress attempt.
///
/// Says what was missing and what the only two acceptable shapes are. Kept
/// short: the step prompt is already long, and a small model reads the tail.
const NO_PROGRESS_NUDGE: &str = "\n\n[SYSTEM: your previous attempt at this step produced only \
     reasoning — no tool call and no answer, so nothing changed. Do not think further about it. \
     Either CALL A TOOL now (that is how work happens), or, if the step is genuinely already \
     satisfied, reply with one short sentence saying so.]";

/// The only tool definitions shipped with a step. Everything else is reached
/// through `discover_tools`.
///
/// Two rules, and they are not in tension:
///
/// 1. **A plan never restricts capability.** Its `tool_scope` is a guess made
///    before the work starts, and a step sealed inside that guess cannot do
///    the job the step turned out to need. Observed 2026-07-27: a plan that
///    scoped steps to `exec` produced 141 exec calls and ZERO write_file /
///    read_file / edit_file — every file operation went through shell
///    heredocs, sailing past the anti-erosion ratchet, the syntax gate and
///    the fork refusals, which is why those guards never fired during the
///    `minidb.sh` forks we spent hours chasing.
///
/// 2. **Context is not free.** Shipping ~30 schemas on every request costs
///    thousands of tokens per step on a 32k window and measurably degrades
///    small-model tool selection. So the request carries the minimum, and the
///    model pulls in what it needs: `discover_tools` to reach any tool, and
///    `recall` because memory is how a step recovers what it already knows
///    (including the `[memory:…]` handles in its own context).
///
/// Anything activated stays active for the rest of the run, so discovery is
/// paid once per tool, not once per step.
/// ONE tool ships: the one that reaches every other tool.
///
/// Schemas are context, and on a 32k window the full set costs thousands of
/// tokens per step and measurably degrades small-model tool selection. So the
/// request carries `discover_tools` and nothing else, and the model pulls in
/// what the task actually needs.
///
/// The hazard this creates is real and worth naming: a model that cannot see a
/// file tool will write files by shell redirection instead, which bypasses the
/// anti-erosion ratchet, the suffixed-fork guard and the path repair — every
/// healing feature lives in the tools, so improvising around them silently
/// loses work. Two things prevent it. `stable_prefix` tells the model plainly
/// that the other tools exist behind discovery and that improvising around them
/// is how work gets lost; and activation now persists for the whole run, so
/// discovery is paid once rather than re-paid out of every step's iteration
/// budget.
///
/// What must NOT come back is the previous compromise, where the prompt ordered
/// the model to call `todo(action='note', …)` while `todo`'s schema was never
/// sent. That is not gating, it is the prompt and the request disagreeing about
/// what exists — and the model obeys blind. Measured 2026-07-28: malformed
/// acceptance checks went from 3.1% of `todo` calls to 53-66%, and `exec` took
/// 50 calls passing `query` (the only parameter name left in context) instead
/// of `command`. If a prompt ever needs to name a tool, the answer is to fix
/// the prompt, not to widen this list.
const CORE_TOOLS: &[&str] = &["discover_tools"];

/// Content not worth a memory: machine noise rather than an observation.
///
/// Deliberately narrow, and narrower than it used to be. This filter also
/// matched six "failure shapes" — `"Error:"`, `"Command failed"` and friends —
/// with `content.contains(s)` across the WHOLE body, not just the prefix.
/// Upstream, `loop_runner` rewrites every unsuccessful tool result to
/// `format!("Error: {…}")`, so the combination discarded **100% of failed tool
/// calls**: 704 of them in one 2-hour run, with not a single ingest line in the
/// whole day's log containing `FAILED`.
///
/// That is backwards. What went wrong is exactly what an agent must remember —
/// an agent that cannot recall its own failures repeats them, which is what a
/// long-horizon run looks like when it stalls. The substring form also ate
/// SUCCESSFUL calls whose output merely mentioned an error: `cat ./minidb`
/// stored nothing, twice, because the script contains its own error strings.
/// The agent could not remember reading its own source.
///
/// Failure is now carried structurally instead — the episodic writer stamps
/// `[tool → target — FAILED]` into the content and an `outcome` tag beside it —
/// so it can be filtered at RECALL time by anyone who wants only successes,
/// without being unwritable in the first place.
fn is_low_signal_memory(content: &str) -> bool {
    let trimmed = content.trim_start();
    if trimmed.is_empty() {
        return true;
    }
    // Binary/garbled output. Judged by CONTROL characters and decode failures,
    // not by "not ASCII" — the old test counted every non-ASCII char as noise,
    // so 40 box-drawing characters in `tree` output, or any text in a
    // non-Latin script, was classified as binary and deleted. It also flagged
    // this very writer's own `[exec → cmd — ok]` header punctuation.
    //
    // Real binary shows up as C0 control bytes and U+FFFD replacement
    // characters after a lossy decode; legitimate text does not.
    let noise = trimmed
        .chars()
        .take(200)
        .filter(|c| (c.is_control() && !c.is_whitespace()) || *c == '\u{FFFD}')
        .count();
    if noise > 40 {
        return true;
    }
    // Heartbeat chatter is the machinery talking to itself — not an observation.
    trimmed.starts_with("HEARTBEAT_OK")
}

/// Faults that mean the local runner's STATE is bad, not that the request
/// was unlucky. These are healed by unloading/reloading the model; another
/// identical request just re-hits the wedge.
///
/// - "empty completion": 200s with ~no generated tokens (stale KV checkpoint
///   restore sending generation straight to a stop token).
/// - "same token": the repetition watch caught a decoder emitting one token
///   forever. Both were surfacing as "no done=true" stream aborts before the
///   watch existed, and both come from the same degraded-runner condition.
fn wedged_runner_error(message: &str) -> bool {
    message.contains("empty completion") || message.contains("same token")
}

/// What one repetition abort looked like, kept so the next one can be
/// recognised as the same fault rather than a fresh unlucky generation.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WedgeFingerprint {
    /// The fragment the decoder was stuck on.
    token: String,
    /// Stream bytes received before the watch gave up.
    bytes: usize,
}

impl WedgeFingerprint {
    /// How far apart two abort lengths may sit and still be the same wedge,
    /// as a divisor of the longer one (1/64 ≈ 1.6%).
    ///
    /// Derived from the two distances it has to separate, not picked for
    /// feel. Same-wedge noise: a re-entered wedge reproduces its prefix to
    /// within a couple of bytes (≤3 on ~2780, ≈0.1%). Different-wedge
    /// distance: a different prompt's prefix has no relation to this one's,
    /// so it differs by hundreds or thousands of bytes. Any bound between
    /// those works; 1/64 sits ~15x above the noise and far below a real
    /// difference, which is what makes the result insensitive to the exact
    /// divisor rather than tuned to it.
    const LENGTH_TOLERANCE_DIVISOR: usize = 64;

    /// Whether `self` is the same wedge as `previous` — same stuck token,
    /// same place it gave up.
    ///
    /// Compared as a band, not an equality, and that is the whole point.
    /// Live evidence (2026-07-28, qwen3.5:9b, 12 aborts across one day):
    /// every abort was stuck on `"0"` and every one landed between 2777 and
    /// 2780 bytes, but consecutive attempts of a single step went 2780→2779,
    /// 2779→2778, 2780→2779, 2780→2780 and 2777→2779. Sampling leaves the
    /// prefix a byte or two different before it collapses, so requiring
    /// equal byte counts would have recognised ONE of those five pairs and
    /// left the other four to pay a full generation apiece.
    ///
    /// The token is the real fingerprint — the watch trips precisely because
    /// the decoder is stuck on one specific token — and the length only has
    /// to tell "the same collapse again" from "a different generation that
    /// also collapsed".
    fn is_repeat_of(&self, previous: &Self) -> bool {
        if self.token != previous.token {
            return false;
        }
        let tolerance = self.bytes.max(previous.bytes) / Self::LENGTH_TOLERANCE_DIVISOR;
        self.bytes.abs_diff(previous.bytes) <= tolerance
    }
}

/// Why the ladder is clearing the runner before the next attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WedgeReset {
    /// This abort is the one we just saw. Nothing is learned by generating
    /// against it again.
    Confirmed,
    /// The fault repeated, but not identically — the original ladder.
    Repeated,
}

/// Whether to clear the runner before the next attempt, and why.
///
/// `current` is the wedge the last attempt aborted on; `previous` is the one
/// the runner produced before it (from an earlier attempt, or an earlier
/// step — see [`record_wedge`]). Both are `None` unless the last failure was
/// a repetition abort carrying a fingerprint.
fn wedge_reset_due(
    attempt: usize,
    last_err: &str,
    current: Option<&WedgeFingerprint>,
    previous: Option<&WedgeFingerprint>,
) -> Option<WedgeReset> {
    if !wedged_runner_error(last_err) {
        return None;
    }
    // Same token, same abort point: the retry re-entered the same wedge, so
    // the free first retry is not buying evidence — it already arrived.
    if let (Some(cur), Some(prev)) = (current, previous) {
        if cur.is_repeat_of(prev) {
            return Some(WedgeReset::Confirmed);
        }
    }
    // Otherwise the original rule: reset once the fault has repeated at all.
    // This still covers "empty completion", which has no fingerprint to
    // compare.
    (attempt >= 2).then_some(WedgeReset::Repeated)
}

/// The last wedge seen from a given model's runner.
///
/// Global, and keyed by model, because a wedge is a property of the RUNNER
/// rather than of the caller: one Ollama server serves every step and every
/// chat turn, and [`reset_ollama_runner_for`] clears it for all of them. Same
/// reasoning as the generation slot in `nanna-llm`, which is keyed by base URL.
///
/// Surviving across steps is what makes the comparison worth anything. Within
/// one step the second wedge does not exist until `attempt` reaches 2 —
/// exactly where the old ladder already reset — so a same-step-only check
/// could never fire earlier than the rule it exists to pre-empt. Across steps
/// it can, and that is the observed shape: 2026-07-28 18:53:53 aborted on one
/// step and 18:55:23 aborted identically on the FIRST try of the next, which
/// then spent a second full generation before the ladder acted at 18:55:32.
fn wedge_store() -> &'static std::sync::Mutex<HashMap<String, WedgeFingerprint>> {
    static WEDGES: std::sync::OnceLock<std::sync::Mutex<HashMap<String, WedgeFingerprint>>> =
        std::sync::OnceLock::new();
    WEDGES.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// Record `wedge` as the newest one for `model`, returning the one it
/// replaced — i.e. the wedge to judge it against.
///
/// A poisoned lock carries no invalid state here (the map is only inserted
/// into and removed from), so recover rather than propagate a panic into the
/// retry path.
fn record_wedge(model: &str, wedge: WedgeFingerprint) -> Option<WedgeFingerprint> {
    wedge_store()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(model.to_string(), wedge)
}

/// Forget a model's last wedge, because its runner was just cleared.
///
/// Without this a reset would leave its own fingerprint behind, and the first
/// wedge on the FRESH runner would match it and reset again immediately — the
/// free retry has to come back when the state it was spent on is gone.
fn forget_wedge(model: &str) {
    wedge_store()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(model);
}

/// Forget every recorded wedge: a server restart clears every model's runner,
/// not just the one that faulted.
fn forget_all_wedges() {
    wedge_store()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
}

/// A failed step attempt: the message the ladder logs and classifies, plus
/// the wedge fingerprint when the failure was one.
///
/// [`AgentStepRunner::try_run_step`] used to return a bare `String`, and that
/// was the one lossy boundary between the repetition watch — which knows the
/// stuck token and the abort length exactly — and the retry ladder that has
/// to act on them. Everything in between (`LlmError` → `AgentError`) already
/// carried the structure.
struct StepAttemptError {
    message: String,
    wedge: Option<WedgeFingerprint>,
}

impl From<String> for StepAttemptError {
    fn from(message: String) -> Self {
        Self { message, wedge: None }
    }
}

impl StepAttemptError {
    /// Flatten an agent error to its message, keeping the wedge fingerprint
    /// when there is one.
    fn from_agent(e: &nanna_agent::AgentError) -> Self {
        let wedge = match e {
            nanna_agent::AgentError::Llm(nanna_llm::LlmError::WedgedRunner {
                token,
                bytes_received,
                ..
            }) => Some(WedgeFingerprint { token: token.clone(), bytes: *bytes_received }),
            _ => None,
        };
        Self { message: format!("step error: {e}"), wedge }
    }
}

/// The runner ran out of GPU memory.
///
/// On CUDA this does not arrive as a clean allocation failure — the prefill
/// compute buffer overruns and the driver reports an illegal memory access, so
/// matching only on "out of memory" misses the form it actually takes here.
fn gpu_memory_error(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("illegal memory access")
        || m.contains("out of memory")
        || m.contains("cuda error")
        || m.contains("failed to allocate")
}

#[async_trait::async_trait]
impl StepRunner for AgentStepRunner {
    async fn run_step(&self, request: StepRequest) -> Result<StepOutcome, String> {
        let mut last_err = String::new();
        let mut nudge_pending = false;
        // The wedge the last attempt aborted on, and the one the runner
        // produced before it — the pair `wedge_reset_due` judges.
        let mut cur_wedge: Option<WedgeFingerprint> = None;
        let mut prev_wedge: Option<WedgeFingerprint> = None;
        for attempt in 0..=STEP_LLM_RETRIES {
            if attempt > 0 {
                tracing::warn!(attempt, error = %last_err, "retrying step after transient LLM error");
                let backoff = STEP_RETRY_BACKOFF_SECS[attempt - 1];
                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                // A WEDGED runner does not recover by being asked again: its
                // state has to be cleared. Retrying one three times against
                // the same wedge is how a run dies — observed 2026-07-27,
                // 17 repetition aborts collapsing into back-to-back
                // attempt=1,2,3 failures until the run ended.
                //
                // Two ways in. The fingerprint says this abort IS the last
                // one (same stuck token, same abort point), which is already
                // the evidence a retry would have bought — so act now.
                // Failing that, the original ladder: reset from the SECOND
                // retry, by which point the fault has repeated once, leaving
                // the first retry free for a genuinely transient drop.
                if let Some(reason) =
                    wedge_reset_due(attempt, &last_err, cur_wedge.as_ref(), prev_wedge.as_ref())
                {
                    tracing::warn!(
                        attempt,
                        ?reason,
                        "wedged runner — resetting it before the next attempt"
                    );
                    self.reset_ollama_runner().await;
                }
                // Out of VRAM is the one fault where repeating the request
                // unchanged is guaranteed to fail again: the context we asked
                // for does not fit. Shrink it and unload, so the retry brings
                // the model back at a size that does.
                //
                // Acted on from the FIRST retry, unlike the wedge above. A
                // wedge might be a blip worth one free retry; this cannot be —
                // and the evidence for waiting is bad, since three runs died to
                // exactly this fault repeating until the run ended.
                if gpu_memory_error(&last_err)
                    && nanna_llm::LlmClient::demote_context(&self.agent_config.model).is_some()
                {
                    self.reset_ollama_runner().await;
                }
            }
            // A fresh context per attempt: the re-anchor makes retries free —
            // there is no partial transcript worth salvaging. After a
            // no-progress attempt the prompt carries a nudge, so the retry is
            // never a verbatim repeat of the request that just stalled.
            let attempt_request = if nudge_pending {
                let mut nudged = request.clone();
                nudged.prompt.push_str(NO_PROGRESS_NUDGE);
                nudged
            } else {
                request.clone()
            };
            // Each attempt is judged on its own: only a repetition abort
            // refills these, so an interleaved network blip cannot leave a
            // stale pair behind for the next wedge to match against.
            cur_wedge = None;
            prev_wedge = None;
            match self.try_run_step(&attempt_request).await {
                Ok(outcome) if is_empty_completion(&outcome) => {
                    dump_empty_step(&request, attempt);
                    last_err =
                        "empty completion (no text, no tool calls, ~0 tokens) from provider"
                            .to_string();
                }
                // Thought but did not act: not a result. Retry with the nudge
                // rather than letting the item be marched forward on nothing.
                Ok(outcome) if made_no_progress(&outcome) => {
                    tracing::warn!(
                        attempt,
                        output_tokens = outcome.output_tokens,
                        "step produced reasoning but no tool call and no text — retrying with a nudge"
                    );
                    nudge_pending = true;
                    last_err =
                        "no-progress step (reasoning only: no tool call, no answer)".to_string();
                }
                Ok(outcome) => return Ok(outcome),
                Err(e) if is_transient_llm_error(&e.message) => {
                    // Remember this wedge so the NEXT one can be recognised
                    // as the same fault. `record_wedge` hands back what it
                    // replaced, which is the previous attempt's wedge within
                    // a step and the previous step's across one.
                    if let Some(wedge) = e.wedge {
                        prev_wedge = record_wedge(&self.agent_config.model, wedge.clone());
                        cur_wedge = Some(wedge);
                    }
                    last_err = e.message;
                }
                Err(e) => return Err(e.message),
            }
        }
        Err(last_err)
    }
}

impl AgentStepRunner {
    /// Put the step's own output — its answer, and each reasoning block — into
    /// memory, in the same `[kind → subject]` shape the episodic tool writer
    /// uses, so answers, thoughts and actions all read alike on recall.
    ///
    /// Reasoning is stored per BLOCK rather than as one lump: the blocks are
    /// already interleaved between tool calls, so one block is one thought
    /// about one action — which is the unit a later step actually wants back.
    async fn remember_step_narration(
        &self,
        request: &StepRequest,
        result: &nanna_agent::AgentResponse,
    ) {
        let Some(sink) = self.memory_sink() else {
            return;
        };
        let label = &request.item_title;

        let text = result.text.trim();
        if !text.is_empty() {
            let mut tags = HashMap::new();
            tags.insert("kind".to_string(), "assistant_text".to_string());
            tags.insert("step".to_string(), label.clone());
            sink(nanna_agent::ExtractedMemory {
                content: format!("[said → {label}] {text}"),
                category: "assistant_text".to_string(),
                provenance: nanna_agent::MemoryProvenance::Observed,
                tags: Some(tags),
            })
            .await;
        }

        let Some(reasoning) = result.reasoning.as_ref() else {
            return;
        };
        // Providers that do not split reasoning still fill `content`; treat
        // that as a single block rather than storing nothing.
        let blocks: Vec<&str> = if reasoning.blocks.is_empty() {
            vec![reasoning.content.as_str()]
        } else {
            reasoning.blocks.iter().map(|b| b.content.as_str()).collect()
        };
        let total = blocks.len();
        for (idx, block) in blocks.into_iter().enumerate() {
            let thought = block.trim();
            if thought.is_empty() {
                continue;
            }
            let mut tags = HashMap::new();
            tags.insert("kind".to_string(), "thinking".to_string());
            tags.insert("step".to_string(), label.clone());
            tags.insert("block".to_string(), format!("{}/{total}", idx + 1));
            sink(nanna_agent::ExtractedMemory {
                content: format!("[thought → {label}] {thought}"),
                category: "thinking".to_string(),
                provenance: nanna_agent::MemoryProvenance::Observed,
                tags: Some(tags),
            })
            .await;
        }
    }

    /// The sink that puts tool results into memory, or `None` when no memory
    /// service exists (then `loop_runner` keeps results in context as before,
    /// which is the correct degradation — never silently lose the output).
    fn memory_sink(&self) -> Option<nanna_agent::MemoryCallback> {
        let service = self.memory.clone()?;
        let workspace_id = self.workspace_id.clone();
        Some(Box::new(move |memory: nanna_agent::ExtractedMemory| {
            let service = service.clone();
            let workspace_id = workspace_id.clone();
            Box::pin(async move {
                if is_low_signal_memory(&memory.content) {
                    // INFO, not DEBUG. The daemon runs at INFO, so the old
                    // debug! meant 704 discarded writes left no operator-visible
                    // trace at all in a run whose store held 90 rows — the
                    // shortfall was invisible until someone counted by hand.
                    // A dropped memory is a fact about the run, not a detail.
                    tracing::info!(
                        category = %memory.category,
                        bytes = memory.content.len(),
                        "dropping low-signal memory (machine noise)"
                    );
                    return;
                }
                let mut metadata = memory.tags.unwrap_or_default();
                metadata.insert("category".to_string(), memory.category.clone());
                metadata.insert(
                    "fact_type".to_string(),
                    memory.provenance.as_str().to_string(),
                );
                // A tool result is raw episodic material — worth keeping, but
                // it must not outrank a stated preference when recall ranks.
                let importance: f32 = match memory.category.as_str() {
                    "tool_result" => 1.5,
                    "preference" | "identity" => 4.0,
                    "fact" | "insight" => 3.5,
                    _ => 3.0,
                };
                let stored = match &workspace_id {
                    Some(ws) => {
                        service
                            .remember_scoped(
                                &memory.content,
                                metadata,
                                importance,
                                Some(ws.clone()),
                            )
                            .await
                    }
                    None => {
                        service
                            .remember_with_importance(&memory.content, metadata, importance)
                            .await
                    }
                };
                if let Err(e) = stored {
                    tracing::warn!(error = %e, "failed to store tool result in memory");
                }
            })
        }))
    }

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
        reset_ollama_runner_for(&self.agent_config.model).await;
    }

    async fn try_run_step(
        &self,
        request: &StepRequest,
    ) -> Result<StepOutcome, StepAttemptError> {
        use nanna_agent::{Agent, AgentContext, RunOptions};

        // Name the item before its output starts arriving, so a multi-step
        // run reads as a sequence of labelled pieces of work rather than one
        // undifferentiated stream.
        if let Some(sink) = &self.chat_sink {
            sink.step_header(request);
        }

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

        // The plan's `tool_scope` does NOT gate anything. It is a guess made
        // before the work started, and every time it has been enforced it has
        // cost a run: scoped to a hallucinated "file" the model had nothing to
        // work with (7/42), scoped to "exec" it shell-heredoc'd around every
        // write guard (141 exec / 0 writes). A step reaches whatever the work
        // turns out to need, through `discover_tools`.
        if !request.tool_scope.is_empty() {
            tracing::debug!(
                scope = %request.tool_scope.join(", "),
                "plan named tools for this step — treated as a hint, not a restriction"
            );
        }

        // Context is the scarce resource, so the REQUEST carries only the core
        // pair; the model activates the rest by discovering them, and what it
        // activates persists for the run.
        // Carry forward everything discovered earlier in THIS run. The doc
        // above promised activation persists; `RunState::new()` per step meant
        // it never did, so each step re-paid discovery out of its 8-iteration
        // budget and then ran out — 40 of 45 steps in one run died at the cap
        // with no text at all. Now discovery is paid once per tool per run,
        // which is what the design said all along.
        let active: Vec<String> = {
            let discovered = self.discovered_tools.read().await;
            CORE_TOOLS
                .iter()
                .map(|t| (*t).to_string())
                .chain(discovered.iter().cloned())
                .collect()
        };
        let restrict_to_active = true;

        // Show the work as it happens: the step's text, thinking, and tool
        // activity stream through the same events a plain chat turn uses,
        // and fill the registered run buffers for recovery/persistence.
        let (on_text, on_thinking, on_tool_start, on_tool_end) = match &self.chat_sink {
            None => (None, None, None, None),
            Some(sink) => {
                let text_sink = sink.clone();
                let think_sink = sink.clone();
                let start_sink = sink.clone();
                let end_sink = sink.clone();
                (
                    Some(Box::new(move |chunk: &str| text_sink.delta(chunk))
                        as Box<dyn Fn(&str) + Send + Sync>),
                    Some(Box::new(move |chunk: &str| think_sink.thinking(chunk))
                        as Box<dyn Fn(&str) + Send + Sync>),
                    Some(Box::new(
                        move |call_id: &str, name: &str, input: &Value, model: Option<&str>| {
                            start_sink.tool_start(call_id, name, input, model);
                        },
                    )
                        as Box<dyn Fn(&str, &str, &Value, Option<&str>) + Send + Sync>),
                    Some(Box::new(
                        move |call_id: &str,
                              name: &str,
                              output: &str,
                              success: bool,
                              duration_ms: u64,
                              _data: Option<&Value>| {
                            end_sink.tool_end(call_id, name, output, success, duration_ms);
                        },
                    )
                        as Box<
                            dyn Fn(&str, &str, &str, bool, u64, Option<&Value>) + Send + Sync,
                        >),
                )
            }
        };

        let options = RunOptions {
            max_iterations: request.max_iterations,
            token_budget: request.token_budget,
            max_wall_clock: request.max_wall_clock,
            step_kind: Some(request.step_kind),
            initial_active_tools: active,
            restrict_to_active_tools: restrict_to_active,
            is_sub_agent: true,
            on_text,
            on_thinking,
            on_tool_start,
            on_tool_end,
            // Tool results land in memory; context keeps only the stub.
            on_memory: self.memory_sink(),
            ..Default::default()
        };

        let result = agent
            .run(&request.prompt, options)
            .await
            // The one place the structured LLM error used to be flattened to
            // prose. The wedge fingerprint is lifted out here instead.
            .map_err(|e| StepAttemptError::from_agent(&e))?;

        // The step's OWN words belong in memory too. Tool results were being
        // captured; the model's answer and its reasoning were not captured
        // anywhere. Assistant text reached memory at most once per user `Send`,
        // from a tail block that runs only after the whole harness run ends —
        // which a multi-hour run never reaches — and thinking blocks had no
        // path at all: ~2500 produced, 0 stored, in one measured run.
        //
        // Captured verbatim and directly rather than by flipping
        // `auto_extract_memories`, which spends an extra LLM call per step to
        // produce a lossy semantic digest. What is wanted here is the record,
        // not a précis of it — compressing is dreaming's job, done later with
        // the whole corpus in view rather than one step at a time.
        // Fold this step's discoveries into the run so the next step starts
        // with them already in hand.
        if !result.active_tools.is_empty() {
            let mut discovered = self.discovered_tools.write().await;
            for tool in &result.active_tools {
                discovered.insert(tool.clone());
            }
        }

        self.remember_step_narration(request, &result).await;

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
// Planning (P19): every chat turn becomes a plan the harness can execute
// ---------------------------------------------------------------------------

/// Wall-clock ceiling for one planning call.
///
/// Bound justification: planning sits in front of *every* chat turn, so it is
/// on the latency path of "hi". The reference local tier answers a short
/// structured prompt in single-digit seconds; 30s covers a cold model load
/// without letting a wedged provider hold a turn hostage — on timeout the
/// caller falls back to the single-task plan and the turn proceeds.
const PLAN_TIMEOUT_SECS: u64 = 30;

/// Iterations allowed inside a planning step.
///
/// Bound justification: planning emits one JSON array and calls no tools.
/// More than one iteration means the model is looping, not planning.
const PLAN_ITERATIONS: usize = 1;

/// Turns a request into a plan using the configured model.
///
/// Wraps [`AgentStepRunner`] rather than duplicating its provider handling:
/// planning is just a step whose prompt asks for JSON and whose tool scope is
/// empty.
pub struct AgentPlanner {
    pub runner: Arc<AgentStepRunner>,
}

impl AgentPlanner {
    #[must_use]
    pub const fn new(runner: Arc<AgentStepRunner>) -> Self {
        Self { runner }
    }

    /// Plan `goal`, degrading to the single-task plan on any failure.
    ///
    /// Never returns an error: a planner problem must not cost the user a
    /// turn. The returned `Plan::origin` records which path was taken.
    pub async fn plan(&self, goal: &str, context: Option<&str>) -> Plan {
        let request = StepRequest {
            // Planning is not attached to an item yet; the store assigns ids
            // when the plan is seeded.
            item_id: 0,
            step_index: 0,
            step_kind: nanna_agent::harness::StepKind::Plan,
            item_title: "Planning".to_string(),
            prompt: build_plan_prompt(goal, context),
            tool_scope: Vec::new(),
            token_budget: None,
            max_iterations: Some(PLAN_ITERATIONS),
            max_wall_clock: Some(std::time::Duration::from_secs(PLAN_TIMEOUT_SECS)),
        };

        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(PLAN_TIMEOUT_SECS),
            self.runner.run_step(request),
        )
        .await;

        match outcome {
            Ok(Ok(step)) => plan_or_fallback(goal, &step.text),
            Ok(Err(message)) => {
                tracing::warn!(%message, "planner failed - falling back to a single-task plan");
                Plan::single(goal)
            }
            Err(_) => {
                tracing::warn!(
                    timeout_secs = PLAN_TIMEOUT_SECS,
                    "planner timed out - falling back to a single-task plan"
                );
                Plan::single(goal)
            }
        }
    }
}

/// Seed a plan into the store, returning the created task ids in plan order.
///
/// `jump_queue` is how an interjected message reaches the front: the store
/// orders `next()` by `in_progress` first, then priority, then `sort_order`.
/// A task inserted mid-run therefore has to (a) sort ahead on `sort_order`
/// and (b) not be outranked by an item the harness already marked
/// `in_progress` - see `SessionInterjector::yield_current_item`.
pub async fn seed_plan(
    storage: &Arc<Storage>,
    scope: &str,
    scope_id: Option<&str>,
    plan: &Plan,
    jump_queue: bool,
) -> Result<Vec<i64>, String> {
    let repo = storage.tasks();
    let base_sort = if jump_queue {
        // Strictly below every existing item in the scope so the new work is
        // selected next. Explicitly computed rather than hardcoded to 0 -
        // 0 collides with the default and merely ties.
        let existing = repo
            .list(scope, scope_id, false)
            .await
            .map_err(|e| e.to_string())?;
        existing.iter().map(|t| t.sort_order).min().unwrap_or(0) - plan.tasks.len() as i64 - 1
    } else {
        0
    };

    let mut ids = Vec::with_capacity(plan.tasks.len());
    for (index, task) in plan.tasks.iter().enumerate() {
        let new = NewTask {
            parent_id: None,
            scope: scope.to_string(),
            scope_id: scope_id.map(String::from),
            project: None,
            title: task.title.clone(),
            description: task.description.clone(),
            // 1 is the highest the store accepts; an interjection is the
            // user speaking, which outranks anything already planned.
            priority: if jump_queue { 1 } else { 2 },
            labels: vec!["chat".to_string()],
            tool_scope: task.tool_scope.clone(),
            due_at: None,
            recurrence: None,
            depends_on: Vec::new(),
            acceptance: task.acceptance.clone(),
            assignee: None,
            sort_order: base_sort + index as i64,
        };
        match repo.create(new).await {
            Ok(created) => ids.push(created.id),
            // One rejected task must not sink the plan - the rest still runs.
            Err(e) => {
                tracing::warn!(title = %task.title, error = %e, "failed to seed planned task");
            }
        }
    }
    if ids.is_empty() {
        return Err("no planned task could be created".to_string());
    }
    Ok(ids)
}

// ---------------------------------------------------------------------------
// Interjection: mid-run messages join the plan at the next step boundary
// ---------------------------------------------------------------------------

/// Messages held for one session before admission.
///
/// Bound justification: these are messages a human typed while watching a run
/// - an unbounded queue here is a memory leak fed by a stuck run. 64 is far
/// past any realistic burst of human typing between two step boundaries, and
/// overflow drops the OLDEST so the most recent intent always survives.
pub const PENDING_MESSAGES_MAX: usize = 64;

/// Pending user messages for one session, drained by `SessionInterjector`.
///
/// This replaces "queue behind the mutex until the run ends" for the chat
/// path. A run can be hours long; blocking a follow-up message for that long
/// is the behaviour this exists to remove.
#[derive(Debug, Default)]
pub struct PendingMessages {
    inner: RwLock<Vec<String>>,
}

impl PendingMessages {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a message for admission at the next step boundary.
    /// Returns the queue depth after the push.
    pub async fn push(&self, message: String) -> usize {
        let mut queue = self.inner.write().await;
        if queue.len() >= PENDING_MESSAGES_MAX {
            queue.remove(0);
        }
        queue.push(message);
        queue.len()
    }

    /// Take everything waiting.
    pub async fn drain(&self) -> Vec<String> {
        let mut queue = self.inner.write().await;
        std::mem::take(&mut *queue)
    }

    /// Cheap check used on the hot path - the overwhelmingly common answer
    /// is "nothing waiting", and that must not cost a write lock.
    pub async fn is_empty(&self) -> bool {
        self.inner.read().await.is_empty()
    }

    /// Current depth.
    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }
}

/// Admits pending chat messages into a live run at step boundaries.
pub struct SessionInterjector {
    pub storage: Arc<Storage>,
    pub scope: String,
    pub scope_id: Option<String>,
    pub pending: Arc<PendingMessages>,
    pub planner: Arc<AgentPlanner>,
    pub actor: String,
    pub event_tx: Option<tokio::sync::broadcast::Sender<Event>>,
}

impl SessionInterjector {
    /// Put the in-flight item back to `pending` so the interjected task wins
    /// the next selection.
    ///
    /// `TaskRepository::next` sorts `in_progress` ahead of everything else
    /// ("resume what you started"), which would otherwise starve an
    /// interjection behind a long multi-step item. Yielding is safe: the
    /// harness re-marks an item `in_progress` via `start()` whenever it
    /// selects it, notes are untouched, and the harness's in-memory progress
    /// counters are keyed by item id, so the original resumes exactly where
    /// it stood once the user's request is done.
    async fn yield_current_item(&self) -> Result<(), String> {
        let repo = self.storage.tasks();
        let tasks = repo
            .list(&self.scope, self.scope_id.as_deref(), false)
            .await
            .map_err(|e| e.to_string())?;
        for task in tasks.iter().filter(|t| t.status == "in_progress") {
            repo.update(
                task.id,
                TaskPatch {
                    status: Some("pending".to_string()),
                    ..TaskPatch::default()
                },
                Some(&self.actor),
            )
            .await
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl Interjector for SessionInterjector {
    async fn interject(&self) -> Result<usize, String> {
        // Hot path: a read lock and an is_empty, nothing more.
        if self.pending.is_empty().await {
            return Ok(0);
        }
        let messages = self.pending.drain().await;
        if messages.is_empty() {
            return Ok(0);
        }

        let mut admitted = 0usize;
        for message in messages {
            let plan = self.planner.plan(&message, None).await;
            // Yield before seeding so the new tasks are the only candidates
            // not outranked by an in-progress item.
            if let Err(e) = self.yield_current_item().await {
                tracing::warn!(error = %e, "could not yield the in-flight item for an interjection");
            }
            let ids = seed_plan(
                &self.storage,
                &self.scope,
                self.scope_id.as_deref(),
                &plan,
                true,
            )
            .await?;
            admitted += ids.len();
            if let Some(tx) = &self.event_tx {
                let _ = tx.send(Event::TaskRunProgress {
                    scope: self.scope.clone(),
                    scope_id: self.scope_id.clone(),
                    task_id: ids.first().copied(),
                    kind: "interjected".to_string(),
                    detail: json!({
                        "message": message,
                        "tasks": ids,
                        "plan_origin": plan.origin,
                    }),
                });
            }
        }
        Ok(admitted)
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

/// Minimum spacing between Ollama server restarts across ALL callers —
/// chat sessions, heartbeats, and task runs share one local server. Bounds
/// the blast radius when a model is persistently broken (e.g. OOM on every
/// load): without it, every failing chat re-kills the server out from under
/// every other client.
const OLLAMA_RESTART_COOLDOWN_SECS: u64 = 600;

/// Unix-epoch seconds of the last restart this process performed.
static LAST_OLLAMA_RESTART_EPOCH_SECS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Normalize a raw `OLLAMA_HOST` value into a CLIENT-usable base URL.
///
/// `OLLAMA_HOST` is a SERVER bind address: `0.0.0.0` / `[::]` (all
/// interfaces) still mean THIS machine, but as connect targets they must
/// become loopback; a bare `host[:port]` needs the http scheme; a missing
/// port gets Ollama's default 11434. Observed live: with
/// `OLLAMA_HOST=0.0.0.0` the runner reset posted to a scheme-less URL
/// (silently failed) and the server restart refused as "not local" — the
/// entire healing ladder was neutered by one env var.
pub(crate) fn normalize_ollama_base(raw: &str) -> String {
    let with_scheme = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("http://{raw}")
    };
    let mapped = with_scheme
        .replace("0.0.0.0", "127.0.0.1")
        .replace("[::]", "[::1]");
    match reqwest::Url::parse(&mapped) {
        Ok(mut url) => {
            if url.port().is_none() && url.set_port(Some(11434)).is_err() {
                return "http://127.0.0.1:11434".to_string();
            }
            url.to_string().trim_end_matches('/').to_string()
        }
        Err(_) => "http://127.0.0.1:11434".to_string(),
    }
}

/// The client base URL for the local Ollama server (env-driven, normalized).
pub(crate) fn ollama_local_base() -> String {
    let raw =
        std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string());
    normalize_ollama_base(&raw)
}

/// Restart the local Ollama server: kill `ollama.exe` only (the tray
/// supervisor respawns it) and wait for the API to come back.
///
/// This is the cure for the sticky degraded-runner state (every generation
/// aborted with `done:false`; model unloads do not clear it — verified live).
/// Callers gate it: bouncing a shared local service is an operator decision.
/// Refuses to act when `OLLAMA_HOST` points at a non-local server, and at
/// most once per [`OLLAMA_RESTART_COOLDOWN_SECS`] process-wide.
pub async fn restart_ollama_server() -> bool {
    // Normalization maps bind-all addresses to loopback, so after it a
    // genuinely remote host is the only thing that won't look local.
    let base = ollama_local_base();
    let is_local = ["localhost", "127.0.0.1", "[::1]"]
        .iter()
        .any(|h| base.contains(h));
    if !is_local {
        tracing::warn!(host = %base, "refusing Ollama restart: OLLAMA_HOST is not a local server");
        return false;
    }

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let last = LAST_OLLAMA_RESTART_EPOCH_SECS.load(std::sync::atomic::Ordering::Relaxed);
    if now_secs.saturating_sub(last) < OLLAMA_RESTART_COOLDOWN_SECS
        || LAST_OLLAMA_RESTART_EPOCH_SECS
            .compare_exchange(
                last,
                now_secs,
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
            )
            .is_err()
    {
        tracing::warn!(
            "skipping Ollama restart — one already ran {}s ago (cooldown {}s)",
            now_secs.saturating_sub(last),
            OLLAMA_RESTART_COOLDOWN_SECS
        );
        return false;
    }

    tracing::warn!("restarting the Ollama server (degraded runner state)");
    // Every model's runner dies with the server, so every fingerprint
    // describing one is now stale.
    forget_all_wedges();
    #[cfg(windows)]
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/IM", "ollama.exe"])
        .output();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("pkill")
        .args(["-x", "ollama"])
        .output();
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
pub(crate) fn ollama_restart_allowed() -> bool {
    std::env::var("NANNA_OLLAMA_RESTART_ON_DEGRADED").as_deref() != Ok("0")
}

/// Poll the local Ollama server until it answers `/api/version`, up to
/// `max_secs`. The chat healer uses this to WAIT OUT a server-down window
/// (our own runner-surgery restart, or any external restart) instead of
/// burning retry budget: attempts against a down server complete zero tool
/// calls, so the progress-based replenishment can never refill the budget,
/// and the 2/5/10s backoffs burn out inside a ~20-60s restart — observed
/// live killing a 4h55m mission two minutes after the surgery that cured
/// its fault storm.
pub(crate) async fn wait_for_ollama_ready(max_secs: u64) -> bool {
    let base = ollama_local_base();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(max_secs);
    loop {
        let up = reqwest::Client::new()
            .get(format!("{base}/api/version"))
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success());
        if up {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}

/// Unload an Ollama-served model (`keep_alive: 0`) to clear runner state —
/// the observed degraded mode restores a stale KV context checkpoint that
/// sends every generation straight to a stop token, and only a fresh runner
/// clears it. Free-function variant so the chat path can heal with the same
/// ladder as the step runner. No-op for non-Ollama models: healing is
/// provider-gated, a `:free` `OpenRouter` suffix must never trigger it.
pub(crate) async fn reset_ollama_runner_for(model: &str) {
    if crate::llm_router::ProviderId::from_model(model) != crate::llm_router::ProviderId::Ollama {
        return;
    }
    let base = ollama_local_base();
    tracing::warn!(model = %model, "resetting Ollama runner (keep_alive=0) after transient failures");
    // The runner this fingerprint described is about to stop existing, so the
    // next wedge must be judged fresh rather than matched against it.
    forget_wedge(model);
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

/// Fold per-segment reports into one run-level report: counters sum, the
/// stop reason is the final segment's, and tokens-per-item is recomputed
/// over the aggregate.
fn fold_reports(segments: &[LongHorizonReport]) -> LongHorizonReport {
    let mut folded = segments.last().cloned().unwrap_or(LongHorizonReport {
        tool_calls: 0,
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
        interjected_items: 0,
    });
    folded.steps_taken = segments.iter().map(|r| r.steps_taken).sum();
    folded.tool_calls = segments.iter().map(|r| r.tool_calls).sum();
    folded.items_completed = segments.iter().map(|r| r.items_completed).sum();
    folded.items_completed_unverified =
        segments.iter().map(|r| r.items_completed_unverified).sum();
    folded.items_abandoned = segments.iter().map(|r| r.items_abandoned).sum();
    folded.replans = segments.iter().map(|r| r.replans).sum();
    folded.false_success_claims = segments.iter().map(|r| r.false_success_claims).sum();
    folded.input_tokens = segments.iter().map(|r| r.input_tokens).sum();
    folded.output_tokens = segments.iter().map(|r| r.output_tokens).sum();
    folded.wall_clock_secs = segments.iter().map(|r| r.wall_clock_secs).sum();
    folded.interjected_items = segments.iter().map(|r| r.interjected_items).sum();
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

    // -----------------------------------------------------------------
    // Step outcomes: what counts as progress
    // -----------------------------------------------------------------

    fn outcome(text: &str, tools: usize, output_tokens: u64) -> StepOutcome {
        StepOutcome {
            text: text.to_string(),
            input_tokens: 100,
            output_tokens,
            tool_calls: (0..tools)
                .map(|i| StepToolCall {
                    name: format!("tool_{i}"),
                    input_digest: String::new(),
                    output_digest: String::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn reasoning_without_action_is_not_progress() {
        // The live failure: 50 words of thinking, no tool call, no answer.
        // Reasoning tokens put it over the empty-completion bound, so it was
        // accepted as a step result and the item was re-entered seven times.
        let thinking_only = outcome("", 0, 240);
        assert!(made_no_progress(&thinking_only));
        assert!(
            !is_empty_completion(&thinking_only),
            "still not an empty completion — the provider is generating fine"
        );
    }

    #[test]
    fn acting_or_answering_counts_as_progress() {
        assert!(!made_no_progress(&outcome("", 1, 300)), "a tool call is progress");
        assert!(!made_no_progress(&outcome("done: 7/42 pass", 0, 300)), "an answer is progress");
        assert!(
            !made_no_progress(&outcome("  \n ", 1, 300)),
            "blank text with a tool call is still progress"
        );
    }

    #[test]
    fn the_no_progress_nudge_demands_an_action() {
        // The retry must not be a verbatim repeat of what just stalled.
        assert!(NO_PROGRESS_NUDGE.contains("CALL A TOOL"));
        assert!(NO_PROGRESS_NUDGE.contains("nothing changed"));
    }

    // -----------------------------------------------------------------
    // Pending messages (interjection intake)
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn pending_messages_drain_in_arrival_order_and_empty_the_queue() {
        let pending = PendingMessages::new();
        assert!(pending.is_empty().await);
        pending.push("first".to_string()).await;
        pending.push("second".to_string()).await;
        assert_eq!(pending.len().await, 2);
        assert!(!pending.is_empty().await);

        assert_eq!(pending.drain().await, vec!["first", "second"]);
        assert!(pending.is_empty().await, "drain must empty the queue");
        assert!(pending.drain().await.is_empty(), "second drain is a no-op");
    }

    #[tokio::test]
    async fn pending_messages_overflow_drops_the_oldest_not_the_newest() {
        // The bound exists so a stuck run cannot leak memory; when it bites,
        // the user's most recent intent is what must survive.
        let pending = PendingMessages::new();
        for i in 0..(PENDING_MESSAGES_MAX + 5) {
            pending.push(format!("m{i}")).await;
        }
        assert_eq!(pending.len().await, PENDING_MESSAGES_MAX);
        let drained = pending.drain().await;
        assert_eq!(drained.last().unwrap(), &format!("m{}", PENDING_MESSAGES_MAX + 4));
        assert_eq!(drained.first().unwrap(), "m5");
    }

    // -----------------------------------------------------------------
    // Plan seeding + queue jumping
    // -----------------------------------------------------------------

    fn plan_of(titles: &[&str]) -> Plan {
        Plan {
            tasks: titles
                .iter()
                .map(|t| nanna_agent::planner::PlannedTask {
                    title: (*t).to_string(),
                    description: None,
                    acceptance: None,
                    tool_scope: Vec::new(),
                })
                .collect(),
            origin: nanna_agent::planner::PlanOrigin::Model,
        }
    }

    #[tokio::test]
    async fn seeded_plan_is_executed_in_plan_order() {
        let storage = Arc::new(Storage::in_memory().await.expect("storage"));
        let ids = seed_plan(&storage, "session", Some("s1"), &plan_of(&["a", "b", "c"]), false)
            .await
            .expect("seeded");
        assert_eq!(ids.len(), 3);

        let repo = storage.tasks();
        let next = repo.next("session", Some("s1")).await.unwrap().unwrap();
        assert_eq!(next.title, "a", "plan order decides what runs first");
    }

    #[tokio::test]
    async fn an_interjected_plan_preempts_pending_work() {
        let storage = Arc::new(Storage::in_memory().await.expect("storage"));
        seed_plan(&storage, "session", Some("s1"), &plan_of(&["original"]), false)
            .await
            .expect("seeded");
        seed_plan(&storage, "session", Some("s1"), &plan_of(&["urgent"]), true)
            .await
            .expect("interjected");

        let next = storage
            .tasks()
            .next("session", Some("s1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(next.title, "urgent", "the user's new message goes first");
    }

    #[tokio::test]
    async fn an_in_progress_item_outranks_an_interjection_until_it_yields() {
        // This is the reason SessionInterjector::yield_current_item exists:
        // TaskRepository::next sorts in_progress ahead of priority, so a
        // mid-run interjection would otherwise starve behind a long item.
        let storage = Arc::new(Storage::in_memory().await.expect("storage"));
        let original = seed_plan(&storage, "session", Some("s1"), &plan_of(&["original"]), false)
            .await
            .expect("seeded")[0];
        let repo = storage.tasks();
        repo.update(
            original,
            TaskPatch {
                status: Some("in_progress".to_string()),
                ..TaskPatch::default()
            },
            Some("harness"),
        )
        .await
        .expect("marked in progress");

        seed_plan(&storage, "session", Some("s1"), &plan_of(&["urgent"]), true)
            .await
            .expect("interjected");

        // Without yielding, the in-flight item still wins.
        let blocked = repo.next("session", Some("s1")).await.unwrap().unwrap();
        assert_eq!(blocked.title, "original");

        // Yielding it back to pending is what lets the interjection through.
        repo.update(
            original,
            TaskPatch {
                status: Some("pending".to_string()),
                ..TaskPatch::default()
            },
            Some("harness"),
        )
        .await
        .expect("yielded");
        let freed = repo.next("session", Some("s1")).await.unwrap().unwrap();
        assert_eq!(freed.title, "urgent");
    }

    #[tokio::test]
    async fn the_yielded_item_resumes_once_the_interjection_completes() {
        let storage = Arc::new(Storage::in_memory().await.expect("storage"));
        seed_plan(&storage, "session", Some("s1"), &plan_of(&["original"]), false)
            .await
            .expect("seeded");
        let urgent = seed_plan(&storage, "session", Some("s1"), &plan_of(&["urgent"]), true)
            .await
            .expect("interjected")[0];

        let repo = storage.tasks();
        repo.complete(urgent, Some("harness"), Some(json!({"verified": false})))
            .await
            .expect("completed");

        let resumed = repo.next("session", Some("s1")).await.unwrap().unwrap();
        assert_eq!(
            resumed.title, "original",
            "the interrupted plan continues where it stood"
        );
    }

    #[tokio::test]
    async fn repeated_interjections_stay_in_arrival_order() {
        let storage = Arc::new(Storage::in_memory().await.expect("storage"));
        seed_plan(&storage, "session", Some("s1"), &plan_of(&["original"]), false)
            .await
            .expect("seeded");
        seed_plan(&storage, "session", Some("s1"), &plan_of(&["first ask"]), true)
            .await
            .expect("first");
        seed_plan(&storage, "session", Some("s1"), &plan_of(&["second ask"]), true)
            .await
            .expect("second");

        // The later interjection sorts ahead — a user who speaks twice means
        // the most recent thing most.
        let next = storage
            .tasks()
            .next("session", Some("s1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(next.title, "second ask");
    }

    #[test]
    fn ollama_base_normalization_handles_bind_addresses() {
        // The live regression: OLLAMA_HOST=0.0.0.0 (a server BIND address)
        // must normalize to a loopback CLIENT url — it neutered the whole
        // healing ladder when treated verbatim.
        assert_eq!(normalize_ollama_base("0.0.0.0"), "http://127.0.0.1:11434");
        assert_eq!(
            normalize_ollama_base("0.0.0.0:11434"),
            "http://127.0.0.1:11434"
        );
        assert_eq!(
            normalize_ollama_base("http://localhost:11434"),
            "http://localhost:11434"
        );
        assert_eq!(
            normalize_ollama_base("localhost"),
            "http://localhost:11434"
        );
        // A genuinely remote host stays remote (and stays refusable).
        let remote = normalize_ollama_base("http://gpubox.lan:11434");
        assert!(remote.contains("gpubox.lan"));
        assert!(!remote.contains("127.0.0.1"));
    }

    #[test]
    fn transient_classifier_matches_the_live_failure_strings() {
        // The exact strings observed in the daemon log the chat-path healing
        // was built from — each one must be retried, not fatal.
        assert!(is_transient_llm_error(
            "LLM error: HTTP error: error sending request for url (http://localhost:11434/api/chat)"
        ));
        assert!(is_transient_llm_error(
            "LLM error: Stream error: error decoding response body"
        ));
        assert!(is_transient_llm_error(
            "API error: 502 - Ollama aborted generation mid-response (done=false)"
        ));
        assert!(is_transient_llm_error(
            "API error: 502 - Ollama stream ended without completion (no done=true)"
        ));
        assert!(is_transient_llm_error("request timed out"));
        // Non-transient failures must fall through to the next model.
        assert!(!is_transient_llm_error("API error: 401 - invalid api key"));
        assert!(!is_transient_llm_error("API error: 400 - context length exceeded"));
    }

    fn seg(
        steps: usize,
        completed: usize,
        tokens_in: u64,
        tokens_out: u64,
        stop: StopReason,
    ) -> LongHorizonReport {
        LongHorizonReport {
            tool_calls: 0,
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
            interjected_items: 0,
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

    // -----------------------------------------------------------------
    // todo-tool re-decomposition guard
    // -----------------------------------------------------------------

    async fn add_task(services: &HashMap<String, ServiceFn>, params: Value) -> Value {
        services
            .get("tasks.add")
            .expect("tasks.add service")(params)
        .await
        .expect("add succeeds")
    }

    /// REGRESSION (lfm2.5 smoke, 2026-07-25): the model re-planned the same
    /// work every step, turning 5 seeded tasks into ~50 — "Write data file
    /// with header and 3 rows" was created ten times — so the plan grew
    /// faster than it was worked. Re-adding an OPEN title reuses that item.
    #[tokio::test]
    async fn re_adding_an_open_title_reuses_the_existing_task() {
        let storage = Arc::new(Storage::in_memory().await.expect("storage"));
        let workspace_id = Arc::new(RwLock::new(None));
        let services = build_task_services(storage.clone(), workspace_id);

        let first = add_task(
            &services,
            json!({"title": "Write data file", "scope": "session", "session_id": "s1"}),
        )
        .await;
        let again = add_task(
            &services,
            json!({"title": "  write DATA file  ", "scope": "session", "session_id": "s1"}),
        )
        .await;

        assert_eq!(
            first["task"]["id"], again["task"]["id"],
            "a re-add of an open title must reuse the item, not clone it"
        );
        assert_eq!(again["deduplicated"], json!(true));
        let open = storage
            .tasks()
            .list("session", Some("s1"), false)
            .await
            .expect("list");
        assert_eq!(open.len(), 1, "exactly one task exists");
    }

    /// The dedupe must not swallow a genuinely recurring chore: once the
    /// previous one is closed, the same title is addable again.
    #[tokio::test]
    async fn the_same_title_is_addable_again_once_the_previous_one_is_closed() {
        let storage = Arc::new(Storage::in_memory().await.expect("storage"));
        let workspace_id = Arc::new(RwLock::new(None));
        let services = build_task_services(storage.clone(), workspace_id);

        let first = add_task(
            &services,
            json!({"title": "run the tests", "scope": "session", "session_id": "s1"}),
        )
        .await;
        let id = first["task"]["id"].as_i64().expect("id");
        storage
            .tasks()
            .complete(id, Some("test"), None)
            .await
            .expect("complete");

        let second = add_task(
            &services,
            json!({"title": "run the tests", "scope": "session", "session_id": "s1"}),
        )
        .await;
        assert_ne!(
            second["task"]["id"].as_i64(),
            Some(id),
            "a closed chore may be scheduled again"
        );
    }

    /// Sibling scoping: the same subtask title under DIFFERENT parents is
    /// legitimate ("write the header" for two different files).
    #[tokio::test]
    async fn the_same_title_under_a_different_parent_is_not_a_duplicate() {
        let storage = Arc::new(Storage::in_memory().await.expect("storage"));
        let workspace_id = Arc::new(RwLock::new(None));
        let services = build_task_services(storage.clone(), workspace_id);

        let a = add_task(
            &services,
            json!({"title": "feature A", "scope": "session", "session_id": "s1"}),
        )
        .await;
        let b = add_task(
            &services,
            json!({"title": "feature B", "scope": "session", "session_id": "s1"}),
        )
        .await;
        let sub_a = add_task(
            &services,
            json!({"title": "write the header", "parent_id": a["task"]["id"]}),
        )
        .await;
        let sub_b = add_task(
            &services,
            json!({"title": "write the header", "parent_id": b["task"]["id"]}),
        )
        .await;

        assert_ne!(
            sub_a["task"]["id"], sub_b["task"]["id"],
            "same title under different parents is different work"
        );
    }

    /// REGRESSION (lfm2.5 smoke, 2026-07-25): the model deleted a SEEDED
    /// plan item it could not finish, and the run then panicked on a task the
    /// harness still expected to verify. A task with an acceptance contract
    /// is never the model's to erase.
    #[tokio::test]
    async fn a_task_with_an_acceptance_contract_cannot_be_deleted() {
        let storage = Arc::new(Storage::in_memory().await.expect("storage"));
        let workspace_id = Arc::new(RwLock::new(None));
        let services = build_task_services(storage.clone(), workspace_id);

        let seeded = storage
            .tasks()
            .create(NewTask {
                scope: "session".to_string(),
                scope_id: Some("s1".to_string()),
                title: "Create the greeting file".to_string(),
                priority: 3,
                acceptance: Some(json!({
                    "kind": "regex", "path": "greeting.txt", "pattern": "hello"
                })),
                ..NewTask::default()
            })
            .await
            .expect("seed");

        let out = services
            .get("tasks.remove")
            .expect("tasks.remove service")(json!({ "id": seeded.id }))
        .await
        .expect("call succeeds");

        assert_eq!(out["removed"], json!(false));
        assert_eq!(out["refused"], json!(true));
        assert!(
            storage.tasks().get(seeded.id).await.is_ok(),
            "the seeded task must still exist"
        );
    }

    /// REGRESSION (lfm2.5 endurance, 2026-07-25): guarding only the per-id
    /// remove left `clear` open, and one call took the scope from 42 tasks to
    /// 6 mid-run — destroying 36 seeded features the harness was still
    /// driving. Bulk clear obeys the same contract rule, including for
    /// ancestors (delete removes whole subtrees).
    #[tokio::test]
    async fn clear_keeps_every_task_carrying_a_contract() {
        let storage = Arc::new(Storage::in_memory().await.expect("storage"));
        let workspace_id = Arc::new(RwLock::new(None));
        let services = build_task_services(storage.clone(), workspace_id);

        let seeded = |title: &str| NewTask {
            scope: "session".to_string(),
            scope_id: Some("s1".to_string()),
            title: title.to_string(),
            priority: 3,
            acceptance: Some(json!({"kind": "regex", "path": "f", "pattern": "x"})),
            ..NewTask::default()
        };
        let a = storage.tasks().create(seeded("feature A")).await.expect("a");
        let b = storage.tasks().create(seeded("feature B")).await.expect("b");

        // A scratch parent whose CHILD holds a contract: clearing the parent
        // would take the child with it, so the parent is protected too.
        let scratch_parent = add_task(
            &services,
            json!({"title": "scratch parent", "scope": "session", "session_id": "s1"}),
        )
        .await;
        let parent_id = scratch_parent["task"]["id"].as_i64().expect("id");
        let mut child = seeded("contract child");
        child.parent_id = Some(parent_id);
        let child = storage.tasks().create(child).await.expect("child");

        let free = add_task(
            &services,
            json!({"title": "pure scratch", "scope": "session", "session_id": "s1"}),
        )
        .await;
        let free_id = free["task"]["id"].as_i64().expect("id");

        let out = services
            .get("tasks.clear")
            .expect("tasks.clear service")(
            json!({"scope": "session", "session_id": "s1", "closed_only": false}),
        )
        .await
        .expect("clear succeeds");

        assert_eq!(
            out["protected"],
            json!(4),
            "A, B, the contract child, and the scratch parent that would take it down"
        );
        for id in [a.id, b.id, child.id, parent_id] {
            assert!(
                storage.tasks().get(id).await.is_ok(),
                "task #{id} must survive a bulk clear"
            );
        }
        assert!(
            storage.tasks().get(free_id).await.is_err(),
            "contract-free scratch is still cleared"
        );
    }

    /// The guard must not make the store un-tidyable: scratch items the model
    /// invents for itself carry no acceptance check and stay removable.
    #[tokio::test]
    async fn a_scratch_task_without_acceptance_is_still_removable() {
        let storage = Arc::new(Storage::in_memory().await.expect("storage"));
        let workspace_id = Arc::new(RwLock::new(None));
        let services = build_task_services(storage.clone(), workspace_id);

        let scratch = add_task(
            &services,
            json!({"title": "think about it", "scope": "session", "session_id": "s1"}),
        )
        .await;
        let id = scratch["task"]["id"].as_i64().expect("id");

        let out = services
            .get("tasks.remove")
            .expect("tasks.remove service")(json!({ "id": id }))
        .await
        .expect("call succeeds");

        assert_ne!(
            out["removed"],
            json!(false),
            "a scratch item must actually be removed (the store reports a count)"
        );
        assert!(out.get("refused").is_none(), "no contract, so no refusal");
        assert!(storage.tasks().get(id).await.is_err(), "scratch item is gone");
    }

    // -----------------------------------------------------------------
    // ChatSink → ExternalRunHandle (P19 navigation recovery)
    // -----------------------------------------------------------------

    fn external_run_handle() -> crate::agent_service::ExternalRunHandle {
        crate::agent_service::ExternalRunHandle {
            cancellation_flag: Arc::new(AtomicBool::new(false)),
            accumulated_text: Arc::new(RwLock::new(String::new())),
            accumulated_thinking: Arc::new(RwLock::new(String::new())),
            active_tool_calls: Arc::new(RwLock::new(Vec::new())),
            completed_tool_calls: Arc::new(RwLock::new(Vec::new())),
            timeline: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    fn chat_sink(run: crate::agent_service::ExternalRunHandle) -> ChatSink {
        let (event_tx, _) = tokio::sync::broadcast::channel(64);
        ChatSink {
            session_id: "s1".to_string(),
            message_id: "m1".to_string(),
            event_tx,
            run: Some(run),
            tool_stats: None,
            storage: None,
            quiet_item: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// REGRESSION (P19 live drive, 2026-07-24): tool calls vanished when the
    /// user navigated away mid-run, because the harness streamed events only —
    /// `get_run_state` recovery reads the ActiveChat buffers, and nothing was
    /// filling them. Every sink callback must land in the shared run handle,
    /// not just on the event bus.
    #[tokio::test]
    async fn tool_calls_survive_navigation_via_run_buffers() {
        let run = external_run_handle();
        let sink = chat_sink(run.clone());

        sink.delta("running ls ");
        sink.thinking("what files exist?");
        sink.tool_start("c1", "exec", &json!({"cmd": "ls"}), None);
        sink.tool_end("c1", "exec", "file.txt", true, 12);
        sink.delta("done");

        // What get_run_state serves after navigating away and back:
        assert_eq!(run.accumulated_text.read().await.as_str(), "running ls done");
        assert_eq!(
            run.accumulated_thinking.read().await.as_str(),
            "what files exist?"
        );
        assert!(
            run.active_tool_calls.read().await.is_empty(),
            "the call completed — it must not linger as active"
        );
        {
            let done = run.completed_tool_calls.read().await;
            assert_eq!(done.len(), 1);
            assert_eq!(done[0].name, "exec");
            assert_eq!(done[0].output, "file.txt");
            assert!(done[0].success);
        }

        // And the journal persisted with the final message: text merged per
        // burst, the tool call back-filled in place with its result.
        let journal = run.timeline.lock().unwrap().clone();
        assert_eq!(journal.len(), 4, "text, thinking, tool, text — in order");
        assert!(matches!(
            &journal[0],
            crate::session::TimelineItem::Text { content, .. } if content == "running ls "
        ));
        assert!(matches!(
            &journal[1],
            crate::session::TimelineItem::Thinking { content, .. } if content == "what files exist?"
        ));
        assert!(matches!(
            &journal[2],
            crate::session::TimelineItem::Tool {
                call_id,
                output: Some(output),
                success: Some(true),
                duration_ms: Some(12),
                ..
            } if call_id == "c1" && output == "file.txt"
        ));
        assert!(matches!(
            &journal[3],
            crate::session::TimelineItem::Text { content, .. } if content == "done"
        ));
    }

    /// "It should feel like the chat did before": a one-task plan is a
    /// conversation-shaped turn and is announced not at all. Items that join
    /// later (interjection, replan) ARE announced — as a timeline Step, the
    /// status row the GUI renders, never as text in the reply. Asserting on
    /// `accumulated_text` here would pass again the moment a banner leaked
    /// back into the message, so the test pins both halves: nothing in the
    /// text, an entry in the journal.
    #[tokio::test]
    async fn a_one_task_plan_streams_with_no_step_banner() {
        let run = external_run_handle();
        let sink = chat_sink(run.clone());
        *sink.quiet_item.lock().unwrap() = Some(7);

        let request = |item_id: i64| StepRequest {
            item_title: format!("test item {item_id}"),
            item_id,
            step_index: 0,
            step_kind: nanna_agent::harness::StepKind::Execute,
            prompt: format!("Task #{item_id}: reply to the user"),
            tool_scope: Vec::new(),
            token_budget: None,
            max_iterations: None,
            max_wall_clock: None,
        };

        sink.step_header(&request(7));
        assert!(
            run.accumulated_text.read().await.is_empty(),
            "conversational turn: no banner"
        );

        assert!(
            run.timeline.lock().unwrap().is_empty(),
            "the quiet item must not even reach the journal"
        );

        sink.step_header(&request(9));
        let streamed = run.accumulated_text.read().await.clone();
        assert!(
            streamed.is_empty(),
            "run mechanics must NEVER be message text (got: {streamed:?})"
        );
        let journal = run.timeline.lock().unwrap().clone();
        match journal.as_slice() {
            [crate::session::TimelineItem::Step { phase, label, item_id, .. }] => {
                assert_eq!(phase, "working");
                assert_eq!(label, "test item 9", "the label is the item TITLE");
                assert_eq!(*item_id, 9);
            }
            other => panic!("expected one Step entry, got: {other:?}"),
        }
    }

    /// Parity with the retired direct chat path: completed tool calls feed
    /// the shared stats tracker (the recording is handed off to a task, so
    /// poll briefly).
    #[tokio::test]
    async fn chat_tool_calls_are_recorded_to_tool_stats() {
        let stats = nanna_agent::ToolStatsTracker::new();
        let mut sink = chat_sink(external_run_handle());
        sink.tool_stats = Some(stats.clone());

        sink.tool_end("c1", "exec", "ok", true, 5);

        for _ in 0..200 {
            if stats.export_json().await.to_string().contains("exec") {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("tool call was never recorded to stats");
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
        self.start_with_interjector(goal, source, runner, config, workdir, event_tx, None)
            .await
    }

    /// [`Self::start`], plus the hook that lets user messages join this run
    /// at a step boundary instead of queueing behind it. Chat-backed runs
    /// always pass one; background runs do not.
    #[allow(clippy::too_many_arguments)]
    pub async fn start_with_interjector(
        self: &Arc<Self>,
        goal: String,
        source: TursoTaskSource,
        runner: AgentStepRunner,
        config: LongHorizonConfig,
        workdir: PathBuf,
        event_tx: tokio::sync::broadcast::Sender<Event>,
        interjector: Option<Arc<SessionInterjector>>,
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
                    .run_with_interjector(
                        &goal,
                        &source,
                        &runner,
                        &workdir,
                        Some(cancel.clone()),
                        interjector.as_deref().map(|i| i as &dyn Interjector),
                    )
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

#[cfg(test)]
mod wedged_runner_tests {
    use super::wedged_runner_error;

    /// REGRESSION (2026-07-27): the repetition watch started aborting wedged
    /// streams, but the retry ladder only reset the runner when the message
    /// said "empty completion" — so the new abort never triggered a reset and
    /// three retries hit the same wedge. 17 aborts ended the run.
    #[test]
    fn both_wedge_shapes_trigger_a_reset() {
        assert!(wedged_runner_error(
            "empty completion (no text, no tool calls, ~0 tokens) from provider"
        ));
        assert!(wedged_runner_error(
            "Ollama emitted the same token 20x in a row (\"0\") — a wedged runner, not a generation."
        ));
    }

    use super::{wedge_reset_due, WedgeFingerprint, WedgeReset};

    /// The wedge message as it actually arrives, so the tests below exercise
    /// the same classification path the ladder does.
    fn wedge_msg(bytes: usize) -> String {
        format!(
            "step error: LLM error: API error: 502 - Ollama emitted the same token 20x in a \
             row (\"0\") — a wedged runner, not a generation. Aborted after {bytes} bytes so \
             the step can be retried instead of waiting out the loop."
        )
    }

    fn wedge(bytes: usize) -> WedgeFingerprint {
        WedgeFingerprint { token: "0".to_string(), bytes }
    }

    /// The pair the change exists for: the retry re-entered the SAME wedge,
    /// so the reset happens on the first retry instead of costing another
    /// generation to re-learn it.
    #[test]
    fn an_identical_wedge_is_confirmed_and_reset_immediately() {
        assert_eq!(
            wedge_reset_due(1, &wedge_msg(2780), Some(&wedge(2780)), Some(&wedge(2780))),
            Some(WedgeReset::Confirmed),
            "equal abort lengths on the same token must not wait for attempt 2"
        );
    }

    /// And the counter-case: a wedge that gave up somewhere else is a
    /// different fault, so the free first retry stands.
    #[test]
    fn a_differently_sized_wedge_is_not_confirmed() {
        assert_eq!(
            wedge_reset_due(1, &wedge_msg(2780), Some(&wedge(2780)), Some(&wedge(400))),
            None,
            "unrelated abort lengths must leave the first retry free"
        );
        // A different stuck token is a different wedge even at the same length.
        let other_token = WedgeFingerprint { token: "\n".to_string(), bytes: 2780 };
        assert_eq!(
            wedge_reset_due(1, &wedge_msg(2780), Some(&wedge(2780)), Some(&other_token)),
            None,
            "the stuck token is the fingerprint — a different one is a different fault"
        );
    }

    /// REGRESSION (2026-07-28): the byte counts do NOT repeat exactly.
    ///
    /// Twelve aborts in one day, every one stuck on "0" and every one landing
    /// in 2777-2780 bytes, but consecutive attempts of a single step went
    /// 2780→2779, 2779→2778, 2780→2779, 2780→2780 and 2777→2779 — sampling
    /// leaves the prefix a byte or two different before it collapses. An
    /// equality test would have recognised one of these five and left the
    /// other four to pay a full generation each, which is the entire cost the
    /// confirmation exists to avoid.
    #[test]
    fn the_observed_near_miss_pairs_are_all_the_same_wedge() {
        for (prev, cur) in [(2780, 2779), (2779, 2778), (2780, 2779), (2780, 2780), (2777, 2779)] {
            assert_eq!(
                wedge_reset_due(1, &wedge_msg(cur), Some(&wedge(cur)), Some(&wedge(prev))),
                Some(WedgeReset::Confirmed),
                "live pair {prev}→{cur} is the same wedge re-entered"
            );
        }
    }

    /// The original ladder still covers everything the fingerprint cannot:
    /// a first wedge with nothing to compare against, and "empty completion",
    /// which carries no fingerprint at all.
    #[test]
    fn the_repeat_ladder_still_backs_the_fingerprint() {
        // First wedge of a run: no predecessor, so wait as before.
        assert_eq!(wedge_reset_due(1, &wedge_msg(2780), Some(&wedge(2780)), None), None);
        assert_eq!(
            wedge_reset_due(2, &wedge_msg(2780), Some(&wedge(2780)), None),
            Some(WedgeReset::Repeated)
        );
        // The other wedge shape has no fingerprint and is judged on repeats.
        let empty = "empty completion (no text, no tool calls, ~0 tokens) from provider";
        assert_eq!(wedge_reset_due(1, empty, None, None), None);
        assert_eq!(wedge_reset_due(2, empty, None, None), Some(WedgeReset::Repeated));
    }

    /// A reset is an unload/reload. It must never fire for a fault that is
    /// not the runner's state, however the fingerprints happen to line up.
    #[test]
    fn a_non_wedge_failure_never_resets() {
        assert_eq!(
            wedge_reset_due(3, "Stream error: connection reset", Some(&wedge(2780)), Some(&wedge(2780))),
            None
        );
    }

    #[test]
    fn ordinary_transient_faults_do_not_reset_the_runner() {
        // Resetting unloads/reloads the model — far too expensive for a
        // network blip, and it would add seconds to every recoverable drop.
        for msg in [
            "error sending request for url (http://127.0.0.1:11434/api/chat)",
            "Stream error: connection reset",
            "API error: 500 - internal",
            "step error: LLM error: API error: 429 - rate limited",
        ] {
            assert!(!wedged_runner_error(msg), "must not reset for: {msg}");
        }
    }
}

#[cfg(test)]
mod core_tool_gating_tests {
    use super::CORE_TOOLS;

    /// The contract, in three directions.
    ///
    /// A plan must never restrict capability (enforcing its guess cost us two
    /// runs: 7/42 when it scoped to a nonexistent "file", and 141-exec/0-write
    /// when it scoped to "exec" and the model shell-heredoc'd around every
    /// write guard).
    ///
    /// Exactly ONE tool ships, because schemas are context.
    ///
    /// And the prompt must not name tools it does not ship. `stable_prefix`
    /// used to order a `todo(action='note', …)` call every step while `todo`'s
    /// schema was never sent; the model obeyed blind and malformed acceptance
    /// checks went from 3.1% to 53-66%. The fix was to stop the prompt naming
    /// tools — not to widen this list. If this list grows again, check whether
    /// a prompt is making promises the request does not keep.
    #[test]
    fn only_discovery_ships_by_default() {
        assert_eq!(CORE_TOOLS, &["discover_tools"]);
        assert!(
            !CORE_TOOLS.contains(&"exec"),
            "exec must be discovered, not shipped — schemas are context"
        );
        assert!(
            !CORE_TOOLS.contains(&"write_file"),
            "write_file must be discovered, not shipped"
        );
        assert!(
            CORE_TOOLS.contains(&"discover_tools"),
            "without discovery a step cannot reach anything at all"
        );
        assert!(
            !CORE_TOOLS.contains(&"recall"),
            "even memory search is discovered — one tool ships, and it is the one \
             that reaches the others"
        );
        assert_eq!(
            CORE_TOOLS.len(),
            1,
            "the list is one tool. Every past widening started as a good local \
             reason and ended as the prompt and the request disagreeing about \
             what the model can call"
        );
    }
}
