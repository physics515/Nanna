//! Agent-grade task repository (P15).
//!
//! The task store is the control structure a long-horizon run is driven from
//! (P14): hierarchy is decomposition, dependencies make `next()` derivable
//! instead of guessed, `blocked` is derived state the model can never write,
//! and `done` on a parent is a harness-enforced invariant, not a prompt
//! convention.
//!
//! No SQL triggers or transactions are used (the migration runner splits on
//! `;` and the codebase serializes all access through one connection mutex);
//! multi-statement invariants hold because every method acquires the single
//! connection lock for the duration of its writes.

use crate::{NewTask, StorageError, Task, TaskActivityEntry, TaskNote, TaskPatch, task_filter};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::Mutex;
use turso::Connection;

/// Maximum title length in bytes.
///
/// Bound justification: the title is re-injected into the O(1) per-step
/// prompt on every harness iteration; 500 bytes (~125 tokens) keeps a single
/// title from ever dominating a small model's window.
pub const TASK_TITLE_MAX_BYTES: usize = 500;

/// Maximum note length in bytes.
///
/// Bound justification: notes carry sub-agent findings and are read back as a
/// bounded tail into prompts; 16 KiB (~4k tokens) is the most a note could
/// ever usefully contribute to a context injection.
pub const TASK_NOTE_MAX_BYTES: usize = 16 * 1024;

/// Maximum direct dependencies per task.
///
/// Bound justification: `next()` and the cycle check walk dependency edges;
/// an item depending on more than 100 siblings is degenerate decomposition
/// (that is what a parent is for), and the bound caps graph-walk work.
pub const TASK_DEPS_MAX: usize = 100;

/// Maximum hierarchy depth.
///
/// Bound justification: parent-chain walks (cycle check, auto-complete)
/// recurse once per level; 32 levels is far beyond meaningful decomposition
/// and keeps every walk trivially bounded.
pub const TASK_DEPTH_MAX: usize = 32;

/// Maximum tasks per scope.
///
/// Bound justification: `next()`, filters, and cycle checks load a scope into
/// memory (~1 KiB/task ⇒ ≤10 MiB); the bound also brakes a runaway agent
/// stuck in a task-creation loop long before the store degrades.
pub const TASKS_PER_SCOPE_MAX: usize = 10_000;

const TASK_COLUMNS: &str = "id, parent_id, scope, scope_id, project, title, description, status, \
     priority, labels, tool_scope, due_at, recurrence, depends_on, acceptance, assignee, \
     sort_order, created_at, updated_at, completed_at";

/// Outcome of completing a task.
#[derive(Debug, Clone)]
pub struct CompleteOutcome {
    pub task: Task,
    /// Ancestor ids auto-completed because all their children finished.
    pub auto_completed: Vec<i64>,
    /// True when the task was already done (idempotent no-op).
    pub already_done: bool,
}

/// Task repository over the shared Turso connection.
pub struct TaskRepository {
    conn: Arc<Mutex<Connection>>,
}

impl TaskRepository {
    #[must_use]
    pub const fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Create a task. Validates scope, title, priority, parent, and
    /// dependencies (existence + bounds); a new task cannot introduce a
    /// dependency cycle because nothing depends on it yet.
    pub async fn create(&self, new: NewTask) -> Result<Task, StorageError> {
        validate_scope(&new.scope, new.scope_id.as_deref())?;
        validate_title(&new.title)?;
        validate_priority(new.priority)?;
        if new.depends_on.len() > TASK_DEPS_MAX {
            return Err(StorageError::Invalid(format!(
                "too many dependencies: {} (max {TASK_DEPS_MAX})",
                new.depends_on.len()
            )));
        }
        if let Some(acceptance) = &new.acceptance {
            validate_acceptance(acceptance)?;
        }

        let scope_tasks = self.load_scope(&new.scope, new.scope_id.as_deref()).await?;
        if scope_tasks.len() >= TASKS_PER_SCOPE_MAX {
            return Err(StorageError::Invalid(format!(
                "scope already holds {TASKS_PER_SCOPE_MAX} tasks"
            )));
        }
        let by_id: HashMap<i64, &Task> = scope_tasks.iter().map(|t| (t.id, t)).collect();

        if let Some(parent_id) = new.parent_id {
            let parent = by_id.get(&parent_id).ok_or_else(|| {
                StorageError::Invalid(format!("parent task #{parent_id} not found in scope"))
            })?;
            if parent.status == "done" || parent.status == "cancelled" {
                return Err(StorageError::Invalid(format!(
                    "cannot add a child to {} task #{parent_id}",
                    parent.status
                )));
            }
            let depth = parent_depth(&by_id, parent_id)?;
            if depth + 1 >= TASK_DEPTH_MAX {
                return Err(StorageError::Invalid(format!(
                    "hierarchy depth limit {TASK_DEPTH_MAX} reached"
                )));
            }
        }
        for dep in &new.depends_on {
            if !by_id.contains_key(dep) {
                return Err(StorageError::Invalid(format!(
                    "dependency task #{dep} not found in scope"
                )));
            }
        }
        // A pure depends_on cycle is impossible for a new task (nothing
        // depends on it yet) — but the parent-completion invariant makes an
        // ancestor an implicit dependent, so depending on an ancestor
        // (directly or transitively) would wedge both sides forever.
        if new.parent_id.is_some() && !new.depends_on.is_empty() {
            let ancestors = ancestor_chain(&by_id, new.parent_id);
            check_ancestor_dependency(&by_id, &ancestors, &new.depends_on)?;
        }

        let labels_json = serde_json::to_string(&new.labels)?;
        let tool_scope_json = serde_json::to_string(&new.tool_scope)?;
        let depends_json = serde_json::to_string(&new.depends_on)?;
        let acceptance_json = new
            .acceptance
            .as_ref()
            .map(std::string::ToString::to_string);

        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO tasks (parent_id, scope, scope_id, project, title, description, status, \
             priority, labels, tool_scope, due_at, recurrence, depends_on, acceptance, assignee, \
             sort_order) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            turso::params![
                new.parent_id,
                new.scope.as_str(),
                new.scope_id.as_deref(),
                new.project.as_deref(),
                new.title.as_str(),
                new.description.as_deref(),
                new.priority,
                labels_json.as_str(),
                tool_scope_json.as_str(),
                new.due_at.as_deref(),
                new.recurrence.as_deref(),
                depends_json.as_str(),
                acceptance_json.as_deref(),
                new.assignee.as_deref(),
                new.sort_order,
            ],
        )
        .await?;

        let mut rows = conn
            .query(
                &format!("SELECT {TASK_COLUMNS} FROM tasks ORDER BY id DESC LIMIT 1"),
                (),
            )
            .await?;
        let task = match rows.next().await? {
            Some(row) => decode_task_row(&row)?,
            None => return Err(StorageError::NotFound("task just created".to_string())),
        };
        // Drop the open cursor before any further statement: an unfinished
        // Rows on the shared turso connection silently swallows later writes.
        drop(rows);
        drop(conn);

        self.log_activity(task.id, new.assignee.as_deref(), "created", None)
            .await?;
        Ok(task)
    }

    /// Get one task with its derived `blocked` flag.
    pub async fn get(&self, id: i64) -> Result<Task, StorageError> {
        let mut task = self.get_raw(id).await?;
        if !task.depends_on.is_empty() {
            let scope_tasks = self
                .load_scope(&task.scope, task.scope_id.as_deref())
                .await?;
            let by_id: HashMap<i64, &Task> = scope_tasks.iter().map(|t| (t.id, t)).collect();
            task.blocked = is_blocked(&task, &by_id);
        }
        Ok(task)
    }

    /// List tasks in a scope with derived `blocked` flags, ordered by
    /// hierarchy-friendly (`sort_order`, `id`).
    pub async fn list(
        &self,
        scope: &str,
        scope_id: Option<&str>,
        include_closed: bool,
    ) -> Result<Vec<Task>, StorageError> {
        let mut tasks = self.load_scope(&normalize_scope(scope), scope_id).await?;
        let statuses: HashMap<i64, String> =
            tasks.iter().map(|t| (t.id, t.status.clone())).collect();
        for task in &mut tasks {
            task.blocked = task.depends_on.iter().any(|dep| {
                statuses
                    .get(dep)
                    .is_some_and(|s| s != "done" && s != "cancelled")
            });
        }
        if !include_closed {
            tasks.retain(|t| t.status != "done" && t.status != "cancelled");
        }
        tasks.sort_by_key(|t| (t.sort_order, t.id));
        Ok(tasks)
    }

    /// The single most valuable call: return the one actionable item —
    /// open, unblocked, a leaf (no open children) — ordered by
    /// `in_progress` first (resume what you started), then priority, due
    /// date, explicit order, and id.
    pub async fn next(
        &self,
        scope: &str,
        scope_id: Option<&str>,
    ) -> Result<Option<Task>, StorageError> {
        let tasks = self.list(scope, scope_id, false).await?;
        let mut open_children: HashSet<i64> = HashSet::new();
        for task in &tasks {
            if let Some(parent) = task.parent_id {
                open_children.insert(parent);
            }
        }
        let mut candidates: Vec<&Task> = tasks
            .iter()
            .filter(|t| !t.blocked && !open_children.contains(&t.id))
            .collect();
        candidates.sort_by(|a, b| {
            let a_progress = i32::from(a.status != "in_progress");
            let b_progress = i32::from(b.status != "in_progress");
            a_progress
                .cmp(&b_progress)
                .then(a.priority.cmp(&b.priority))
                .then_with(|| match (&a.due_at, &b.due_at) {
                    (Some(x), Some(y)) => x.cmp(y),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                })
                .then(a.sort_order.cmp(&b.sort_order))
                .then(a.id.cmp(&b.id))
        });
        Ok(candidates.first().map(|t| (*t).clone()))
    }

    /// Apply a partial update. `status` accepts only
    /// `pending | in_progress | cancelled`: `done` must go through
    /// [`complete`](Self::complete) and `blocked` is derived, never written.
    pub async fn update(
        &self,
        id: i64,
        patch: TaskPatch,
        actor: Option<&str>,
    ) -> Result<Task, StorageError> {
        let mut task = self.get_raw(id).await?;
        let mut changed: Vec<&'static str> = Vec::new();

        if let Some(status) = &patch.status {
            match status.as_str() {
                "pending" | "in_progress" | "cancelled" => {
                    if task.status != *status {
                        changed.push("status");
                    }
                    task.status.clone_from(status);
                    // Reopening a closed task must clear its completion stamp.
                    if status != "cancelled" && task.completed_at.is_some() {
                        task.completed_at = None;
                    }
                }
                "done" => {
                    return Err(StorageError::Invalid(
                        "status 'done' is a verdict: use the complete/done action so the \
                         acceptance check can run"
                            .to_string(),
                    ));
                }
                "blocked" => {
                    return Err(StorageError::Invalid(
                        "'blocked' is derived from depends_on and cannot be set directly; \
                         add a dependency instead"
                            .to_string(),
                    ));
                }
                other => {
                    return Err(StorageError::Invalid(format!("unknown status '{other}'")));
                }
            }
        }
        if let Some(title) = patch.title {
            validate_title(&title)?;
            if task.title != title {
                changed.push("title");
            }
            task.title = title;
        }
        if let Some(description) = patch.description {
            changed.push("description");
            task.description = description;
        }
        if let Some(priority) = patch.priority {
            validate_priority(priority)?;
            if task.priority != priority {
                changed.push("priority");
            }
            task.priority = priority;
        }
        if let Some(labels) = patch.labels {
            changed.push("labels");
            task.labels = labels;
        }
        if let Some(tool_scope) = patch.tool_scope {
            changed.push("tool_scope");
            task.tool_scope = tool_scope;
        }
        if let Some(due_at) = patch.due_at {
            changed.push("due_at");
            task.due_at = due_at;
        }
        if let Some(recurrence) = patch.recurrence {
            changed.push("recurrence");
            task.recurrence = recurrence;
        }
        if let Some(acceptance) = patch.acceptance {
            if let Some(check) = &acceptance {
                validate_acceptance(check)?;
            }
            changed.push("acceptance");
            task.acceptance = acceptance;
        }
        if let Some(assignee) = patch.assignee {
            changed.push("assignee");
            task.assignee = assignee;
        }
        if let Some(project) = patch.project {
            changed.push("project");
            task.project = project;
        }
        if let Some(sort_order) = patch.sort_order {
            if task.sort_order != sort_order {
                changed.push("sort_order");
            }
            task.sort_order = sort_order;
        }

        let needs_graph_check = patch.depends_on.is_some() || patch.parent_id.is_some();
        let parent_changed = patch.parent_id.is_some();
        if let Some(depends_on) = patch.depends_on {
            if depends_on.len() > TASK_DEPS_MAX {
                return Err(StorageError::Invalid(format!(
                    "too many dependencies: {} (max {TASK_DEPS_MAX})",
                    depends_on.len()
                )));
            }
            changed.push("depends_on");
            task.depends_on = depends_on;
        }
        if let Some(parent_id) = patch.parent_id {
            changed.push("parent_id");
            task.parent_id = parent_id;
        }

        {
            // Validate and write under ONE connection guard: the mutex is the
            // transaction, so a concurrent writer cannot slip a conflicting
            // graph change between the checks and the write.
            let conn = self.conn.lock().await;
            if needs_graph_check {
                let scope_tasks =
                    load_scope_with(&conn, &task.scope, task.scope_id.as_deref()).await?;
                let mut by_id: HashMap<i64, Task> =
                    scope_tasks.into_iter().map(|t| (t.id, t)).collect();
                // Evaluate the graph as it would be after this update.
                by_id.insert(task.id, task.clone());
                let borrow: HashMap<i64, &Task> = by_id.iter().map(|(k, v)| (*k, v)).collect();

                for dep in &task.depends_on {
                    if *dep == task.id {
                        return Err(StorageError::Invalid(format!(
                            "task #{} cannot depend on itself",
                            task.id
                        )));
                    }
                    if !borrow.contains_key(dep) {
                        return Err(StorageError::Invalid(format!(
                            "dependency task #{dep} not found in scope"
                        )));
                    }
                }
                check_dependency_cycle(&borrow, task.id)?;

                if let Some(parent_id) = task.parent_id {
                    if parent_id == task.id {
                        return Err(StorageError::Invalid(format!(
                            "task #{} cannot be its own parent",
                            task.id
                        )));
                    }
                    let Some(parent) = borrow.get(&parent_id) else {
                        return Err(StorageError::Invalid(format!(
                            "parent task #{parent_id} not found in scope"
                        )));
                    };
                    if parent_changed && (parent.status == "done" || parent.status == "cancelled") {
                        return Err(StorageError::Invalid(format!(
                            "cannot move a task under {} task #{parent_id}",
                            parent.status
                        )));
                    }
                    check_parent_cycle(&borrow, task.id)?;

                    if parent_changed {
                        // Re-parenting must not push the moved subtree past
                        // the depth bound.
                        let new_depth = parent_depth(&borrow, parent_id)? + 1;
                        let height = subtree_height(&borrow, task.id);
                        if new_depth + height >= TASK_DEPTH_MAX {
                            return Err(StorageError::Invalid(format!(
                                "hierarchy depth limit {TASK_DEPTH_MAX} exceeded by re-parenting"
                            )));
                        }
                    }
                }

                // The parent-completion invariant makes ancestors implicit
                // dependents: no task in the (possibly re-parented) subtree
                // may depend on one of its ancestors, or both wedge forever.
                let ancestors = ancestor_chain(&borrow, task.parent_id);
                check_ancestor_dependency(&borrow, &ancestors, &task.depends_on)?;
                if parent_changed {
                    for member_id in subtree_ids_from(&borrow, task.id) {
                        if member_id == task.id {
                            continue;
                        }
                        if let Some(member) = borrow.get(&member_id) {
                            check_ancestor_dependency(&borrow, &ancestors, &member.depends_on)?;
                        }
                    }
                }
            }
            write_task_with(&conn, &task).await?;
        }
        if !changed.is_empty() {
            self.log_activity(
                task.id,
                actor,
                "updated",
                Some(serde_json::json!({ "fields": changed })),
            )
            .await?;
        }

        // Cancelling a branch closes its whole open subtree — children of a
        // cancelled plan must not surface from next().
        if changed.contains(&"status") && task.status == "cancelled" {
            let scope_tasks = self
                .load_scope(&task.scope, task.scope_id.as_deref())
                .await?;
            let mut queue: VecDeque<i64> = VecDeque::from([task.id]);
            // Bounded by scope size: each task enters the queue at most once.
            let mut seen: HashSet<i64> = HashSet::from([task.id]);
            while let Some(current) = queue.pop_front() {
                for t in &scope_tasks {
                    if t.parent_id == Some(current) && seen.insert(t.id) {
                        if t.status != "done" && t.status != "cancelled" {
                            let mut child = t.clone();
                            child.status = "cancelled".to_string();
                            self.write_task(&child).await?;
                            self.log_activity(
                                t.id,
                                actor,
                                "cancelled",
                                Some(serde_json::json!({ "cascade_from": task.id })),
                            )
                            .await?;
                        }
                        queue.push_back(t.id);
                    }
                }
            }
        }
        self.get(id).await
    }

    /// Complete a task. The harness (or service layer) runs the acceptance
    /// check *before* calling this — completion here is the recorded verdict.
    ///
    /// Enforced invariants: a parent cannot complete while a child is open,
    /// and ancestors without acceptance checks auto-complete when their last
    /// open child finishes (a parent *with* an acceptance check must be
    /// completed explicitly so its own check runs).
    pub async fn complete(
        &self,
        id: i64,
        actor: Option<&str>,
        detail: Option<serde_json::Value>,
    ) -> Result<CompleteOutcome, StorageError> {
        let task = self.get_raw(id).await?;
        if task.status == "done" {
            return Ok(CompleteOutcome {
                task,
                auto_completed: Vec::new(),
                already_done: true,
            });
        }

        // Check-then-mark under ONE connection guard so a child created
        // concurrently cannot slip in between the open-children check and the
        // completion write (the mutex is the transaction).
        let scope_tasks;
        {
            let conn = self.conn.lock().await;
            scope_tasks = load_scope_with(&conn, &task.scope, task.scope_id.as_deref()).await?;
            let open_children: Vec<i64> = scope_tasks
                .iter()
                .filter(|t| {
                    t.parent_id == Some(id) && t.status != "done" && t.status != "cancelled"
                })
                .map(|t| t.id)
                .collect();
            if !open_children.is_empty() {
                return Err(StorageError::Invalid(format!(
                    "task #{id} has open children {open_children:?}; complete them first"
                )));
            }
            mark_done_with(&conn, id).await?;
        }
        self.log_activity(id, actor, "completed", detail).await?;

        // Auto-complete ancestors whose children are now all closed.
        let by_id: HashMap<i64, &Task> = scope_tasks.iter().map(|t| (t.id, t)).collect();
        let mut closed: HashSet<i64> = scope_tasks
            .iter()
            .filter(|t| t.status == "done" || t.status == "cancelled")
            .map(|t| t.id)
            .collect();
        closed.insert(id);

        let mut auto_completed = Vec::new();
        let mut current = task.parent_id;
        // Bounded by the hierarchy depth limit enforced at write time.
        for _ in 0..TASK_DEPTH_MAX {
            let Some(parent_id) = current else { break };
            let Some(parent) = by_id.get(&parent_id) else {
                break;
            };
            if closed.contains(&parent_id) || parent.acceptance.is_some() {
                break;
            }
            let all_children_closed = scope_tasks
                .iter()
                .filter(|t| t.parent_id == Some(parent_id))
                .all(|t| closed.contains(&t.id));
            if !all_children_closed {
                break;
            }
            self.mark_done(parent_id).await?;
            self.log_activity(
                parent_id,
                actor,
                "auto_completed",
                Some(serde_json::json!({ "trigger": id })),
            )
            .await?;
            closed.insert(parent_id);
            auto_completed.push(parent_id);
            current = parent.parent_id;
        }

        let task = self.get_raw(id).await?;
        debug_assert!(task.status == "done", "complete() must leave status=done");
        Ok(CompleteOutcome {
            task,
            auto_completed,
            already_done: false,
        })
    }

    /// Reopen a closed task (recurrence firing, or replan of a wrong verdict).
    pub async fn reopen(&self, id: i64, actor: Option<&str>) -> Result<Task, StorageError> {
        let task = self.get_raw(id).await?;
        if task.status != "done" && task.status != "cancelled" {
            return Ok(task);
        }
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE tasks SET status = 'pending', completed_at = NULL, \
             updated_at = datetime('now') WHERE id = ?1",
            turso::params![id],
        )
        .await?;
        drop(conn);
        self.log_activity(id, actor, "reopened", None).await?;
        self.get_raw(id).await
    }

    /// Append a working note (the durable scratchpad sub-agents leave
    /// findings in).
    pub async fn add_note(
        &self,
        task_id: i64,
        author: Option<&str>,
        content: &str,
    ) -> Result<TaskNote, StorageError> {
        let trimmed = content.trim();
        if trimmed.is_empty() {
            return Err(StorageError::Invalid("note content is empty".to_string()));
        }
        if trimmed.len() > TASK_NOTE_MAX_BYTES {
            return Err(StorageError::Invalid(format!(
                "note exceeds {TASK_NOTE_MAX_BYTES} bytes (got {})",
                trimmed.len()
            )));
        }
        // Ensure the task exists before writing (FKs are unenforced).
        let _ = self.get_raw(task_id).await?;

        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO task_notes (task_id, author, content) VALUES (?1, ?2, ?3)",
            turso::params![task_id, author, trimmed],
        )
        .await?;
        let mut rows = conn
            .query(
                "SELECT id, task_id, author, content, created_at FROM task_notes \
                 ORDER BY id DESC LIMIT 1",
                (),
            )
            .await?;
        if let Some(row) = rows.next().await? {
            Ok(TaskNote {
                id: row.get(0)?,
                task_id: row.get(1)?,
                author: row.get(2)?,
                content: row.get(3)?,
                created_at: row.get(4)?,
            })
        } else {
            Err(StorageError::NotFound("note just created".to_string()))
        }
    }

    /// Last `limit` notes for a task, oldest first.
    pub async fn notes(&self, task_id: i64, limit: i64) -> Result<Vec<TaskNote>, StorageError> {
        let conn = self.conn.lock().await;
        let mut rows = conn
            .query(
                "SELECT id, task_id, author, content, created_at FROM task_notes \
                 WHERE task_id = ?1 ORDER BY id DESC LIMIT ?2",
                turso::params![task_id, limit],
            )
            .await?;
        let mut notes = Vec::new();
        while let Some(row) = rows.next().await? {
            notes.push(TaskNote {
                id: row.get(0)?,
                task_id: row.get(1)?,
                author: row.get(2)?,
                content: row.get(3)?,
                created_at: row.get(4)?,
            });
        }
        notes.reverse();
        Ok(notes)
    }

    /// Last `limit` activity entries for a task, oldest first.
    pub async fn activity(
        &self,
        task_id: i64,
        limit: i64,
    ) -> Result<Vec<TaskActivityEntry>, StorageError> {
        let conn = self.conn.lock().await;
        let mut rows = conn
            .query(
                "SELECT id, task_id, actor, action, detail, created_at FROM task_activity \
                 WHERE task_id = ?1 ORDER BY id DESC LIMIT ?2",
                turso::params![task_id, limit],
            )
            .await?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next().await? {
            let detail_str: Option<String> = row.get(4)?;
            entries.push(TaskActivityEntry {
                id: row.get(0)?,
                task_id: row.get(1)?,
                actor: row.get(2)?,
                action: row.get(3)?,
                detail: detail_str.and_then(|s| serde_json::from_str(&s).ok()),
                created_at: row.get(5)?,
            });
        }
        entries.reverse();
        Ok(entries)
    }

    /// Record an activity entry (public so the harness can log run events
    /// like acceptance verdicts against the task).
    pub async fn log_activity(
        &self,
        task_id: i64,
        actor: Option<&str>,
        action: &str,
        detail: Option<serde_json::Value>,
    ) -> Result<(), StorageError> {
        let detail_json = detail.as_ref().map(std::string::ToString::to_string);
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO task_activity (task_id, actor, action, detail) VALUES (?1, ?2, ?3, ?4)",
            turso::params![task_id, actor, action, detail_json.as_deref()],
        )
        .await?;
        Ok(())
    }

    /// Query tasks in a scope with the filter language
    /// (see [`task_filter`]). Evaluated in memory; `blocked` is derived
    /// before evaluation so `blocked` atoms see real state.
    pub async fn query(
        &self,
        scope: &str,
        scope_id: Option<&str>,
        filter: &str,
    ) -> Result<Vec<Task>, StorageError> {
        let expr = task_filter::parse(filter)
            .map_err(|e| StorageError::Invalid(format!("filter: {e}")))?;
        let tasks = self.list(scope, scope_id, true).await?;
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        Ok(tasks
            .into_iter()
            .filter(|t| expr.matches(t, &today))
            .collect())
    }

    /// Delete a task and its whole subtree (notes + activity included), and
    /// strip dangling references from other tasks' `depends_on`.
    pub async fn delete(&self, id: i64, actor: Option<&str>) -> Result<u64, StorageError> {
        let task = self.get_raw(id).await?;
        let scope_tasks = self
            .load_scope(&task.scope, task.scope_id.as_deref())
            .await?;

        // Collect the subtree, bounded by scope size.
        let mut doomed: Vec<i64> = vec![id];
        let mut queue: VecDeque<i64> = VecDeque::from([id]);
        while let Some(current) = queue.pop_front() {
            for t in &scope_tasks {
                if t.parent_id == Some(current) && !doomed.contains(&t.id) {
                    doomed.push(t.id);
                    queue.push_back(t.id);
                }
            }
        }

        let conn = self.conn.lock().await;
        for task_id in &doomed {
            conn.execute(
                "DELETE FROM task_notes WHERE task_id = ?1",
                turso::params![*task_id],
            )
            .await?;
            conn.execute(
                "DELETE FROM task_activity WHERE task_id = ?1",
                turso::params![*task_id],
            )
            .await?;
            conn.execute("DELETE FROM tasks WHERE id = ?1", turso::params![*task_id])
                .await?;
        }
        drop(conn);

        // Strip dangling dependency references.
        let doomed_set: HashSet<i64> = doomed.iter().copied().collect();
        for t in &scope_tasks {
            if doomed_set.contains(&t.id) {
                continue;
            }
            if t.depends_on.iter().any(|d| doomed_set.contains(d)) {
                let mut kept = t.clone();
                kept.depends_on.retain(|d| !doomed_set.contains(d));
                self.write_task(&kept).await?;
            }
        }

        let count = doomed.len() as u64;
        if let Some(actor) = actor {
            tracing::debug!("{actor} deleted task #{id} subtree ({count} tasks)");
        }
        Ok(count)
    }

    /// Clear closed tasks (or every task) in a scope. Returns deleted count.
    pub async fn clear(
        &self,
        scope: &str,
        scope_id: Option<&str>,
        closed_only: bool,
    ) -> Result<u64, StorageError> {
        let tasks = self.load_scope(&normalize_scope(scope), scope_id).await?;
        let by_id: HashMap<i64, &Task> = tasks.iter().map(|t| (t.id, t)).collect();
        let mut deleted = 0u64;
        // Delete children before parents so subtree deletes never double-count.
        let mut targets: Vec<&Task> = tasks
            .iter()
            .filter(|t| !closed_only || t.status == "done" || t.status == "cancelled")
            .collect();
        targets.sort_by_key(|t| std::cmp::Reverse(parent_depth_lenient(&tasks, t.id)));
        for target in targets {
            // delete() removes whole subtrees — clearing closed tasks must
            // never take a still-open descendant down with a closed parent.
            if closed_only {
                let has_open_descendant = subtree_ids_from(&by_id, target.id)
                    .into_iter()
                    .filter(|member_id| *member_id != target.id)
                    .any(|member_id| {
                        by_id
                            .get(&member_id)
                            .is_some_and(|t| t.status != "done" && t.status != "cancelled")
                    });
                if has_open_descendant {
                    continue;
                }
            }
            // The subtree may already be gone via an earlier parent delete.
            if self.get_raw(target.id).await.is_ok() {
                deleted += self.delete(target.id, None).await?;
            }
        }
        Ok(deleted)
    }

    /// Import v0.1 JSON todo items (session-scoped flat list). `blocked`
    /// items become `pending` — blocked is derived now — with an activity
    /// entry preserving the old label. Returns imported count.
    pub async fn import_v01(
        &self,
        session_id: &str,
        items: &[(String, String)],
    ) -> Result<usize, StorageError> {
        // Idempotence: a session being migrated has no store tasks yet. If
        // any exist, a previous (possibly partial) import or v0.2 usage
        // already populated the scope — never duplicate.
        let existing = self.load_scope("session", Some(session_id)).await?;
        if !existing.is_empty() {
            return Ok(0);
        }
        let mut imported = 0usize;
        for (index, (text, status)) in items.iter().enumerate() {
            if text.trim().is_empty() {
                continue;
            }
            let mut title = text.trim().to_string();
            if title.len() > TASK_TITLE_MAX_BYTES {
                // Cut on a char boundary: String::truncate panics mid-char.
                let mut cut = TASK_TITLE_MAX_BYTES;
                while cut > 0 && !title.is_char_boundary(cut) {
                    cut -= 1;
                }
                title.truncate(cut);
            }
            let task = self
                .create(NewTask {
                    scope: "session".to_string(),
                    scope_id: Some(session_id.to_string()),
                    title,
                    priority: 3,
                    sort_order: index as i64,
                    ..NewTask::default()
                })
                .await?;
            match status.as_str() {
                "done" => {
                    let _ = self
                        .complete(task.id, Some("todo-v0.1-migration"), None)
                        .await?;
                }
                "in_progress" => {
                    let _ = self
                        .update(
                            task.id,
                            TaskPatch {
                                status: Some("in_progress".to_string()),
                                ..TaskPatch::default()
                            },
                            Some("todo-v0.1-migration"),
                        )
                        .await?;
                }
                "blocked" => {
                    self.log_activity(
                        task.id,
                        Some("todo-v0.1-migration"),
                        "imported_blocked",
                        Some(serde_json::json!({
                            "note": "v0.1 'blocked' label dropped: blocked is derived from depends_on now"
                        })),
                    )
                    .await?;
                }
                _ => {}
            }
            imported += 1;
        }
        Ok(imported)
    }

    /// All completed tasks that carry a recurrence expression, across every
    /// scope. The daemon's recurrence sweep (one recurrence engine — the P8
    /// scheduler) computes the next occurrence and reopens due ones.
    pub async fn list_recurring_closed(&self) -> Result<Vec<Task>, StorageError> {
        let conn = self.conn.lock().await;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TASK_COLUMNS} FROM tasks \
                     WHERE recurrence IS NOT NULL AND status = 'done'"
                ),
                (),
            )
            .await?;
        let mut tasks = Vec::new();
        while let Some(row) = rows.next().await? {
            tasks.push(decode_task_row(&row)?);
        }
        Ok(tasks)
    }

    /// (open, closed) counts for a scope — the progress line.
    pub async fn counts(
        &self,
        scope: &str,
        scope_id: Option<&str>,
    ) -> Result<(u64, u64), StorageError> {
        let tasks = self.load_scope(&normalize_scope(scope), scope_id).await?;
        let closed = tasks
            .iter()
            .filter(|t| t.status == "done" || t.status == "cancelled")
            .count() as u64;
        let open = tasks.len() as u64 - closed;
        Ok((open, closed))
    }

    // ------------------------------------------------------------------
    // internals
    // ------------------------------------------------------------------

    async fn get_raw(&self, id: i64) -> Result<Task, StorageError> {
        let conn = self.conn.lock().await;
        let mut rows = conn
            .query(
                &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1"),
                turso::params![id],
            )
            .await?;
        match rows.next().await? {
            Some(row) => decode_task_row(&row),
            None => Err(StorageError::NotFound(format!("Task: #{id}"))),
        }
    }

    async fn load_scope(
        &self,
        scope: &str,
        scope_id: Option<&str>,
    ) -> Result<Vec<Task>, StorageError> {
        let conn = self.conn.lock().await;
        load_scope_with(&conn, scope, scope_id).await
    }

    async fn mark_done(&self, id: i64) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        mark_done_with(&conn, id).await
    }

    async fn write_task(&self, task: &Task) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        write_task_with(&conn, task).await
    }
}

/// Load a scope's tasks using an already-held connection guard (the mutex is
/// the transaction: validate-then-write sequences hold one guard throughout).
async fn load_scope_with(
    conn: &Connection,
    scope: &str,
    scope_id: Option<&str>,
) -> Result<Vec<Task>, StorageError> {
    let mut rows = match scope_id {
        Some(sid) => {
            conn.query(
                &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE scope = ?1 AND scope_id = ?2"),
                turso::params![scope, sid],
            )
            .await?
        }
        None => {
            conn.query(
                &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE scope = ?1 AND scope_id IS NULL"),
                turso::params![scope],
            )
            .await?
        }
    };
    let mut tasks = Vec::new();
    while let Some(row) = rows.next().await? {
        tasks.push(decode_task_row(&row)?);
    }
    Ok(tasks)
}

async fn mark_done_with(conn: &Connection, id: i64) -> Result<(), StorageError> {
    conn.execute(
        "UPDATE tasks SET status = 'done', completed_at = datetime('now'), \
         updated_at = datetime('now') WHERE id = ?1",
        turso::params![id],
    )
    .await?;
    Ok(())
}

async fn write_task_with(conn: &Connection, task: &Task) -> Result<(), StorageError> {
    let labels_json = serde_json::to_string(&task.labels)?;
    let tool_scope_json = serde_json::to_string(&task.tool_scope)?;
    let depends_json = serde_json::to_string(&task.depends_on)?;
    let acceptance_json = task
        .acceptance
        .as_ref()
        .map(std::string::ToString::to_string);
    conn.execute(
        "UPDATE tasks SET parent_id = ?1, project = ?2, title = ?3, description = ?4, \
         status = ?5, priority = ?6, labels = ?7, tool_scope = ?8, due_at = ?9, \
         recurrence = ?10, depends_on = ?11, acceptance = ?12, assignee = ?13, \
         sort_order = ?14, completed_at = ?15, updated_at = datetime('now') WHERE id = ?16",
        turso::params![
            task.parent_id,
            task.project.as_deref(),
            task.title.as_str(),
            task.description.as_deref(),
            task.status.as_str(),
            task.priority,
            labels_json.as_str(),
            tool_scope_json.as_str(),
            task.due_at.as_deref(),
            task.recurrence.as_deref(),
            depends_json.as_str(),
            acceptance_json.as_deref(),
            task.assignee.as_deref(),
            task.sort_order,
            task.completed_at.as_deref(),
            task.id,
        ],
    )
    .await?;
    Ok(())
}

fn normalize_scope(scope: &str) -> String {
    let lower = scope.trim().to_lowercase();
    if lower.is_empty() {
        "session".to_string()
    } else {
        lower
    }
}

fn validate_scope(scope: &str, scope_id: Option<&str>) -> Result<(), StorageError> {
    match scope {
        "session" | "workspace" => {
            if scope_id.is_none_or(str::is_empty) {
                return Err(StorageError::Invalid(format!(
                    "scope '{scope}' requires a scope_id"
                )));
            }
            Ok(())
        }
        "global" => {
            if scope_id.is_some() {
                return Err(StorageError::Invalid(
                    "scope 'global' must not carry a scope_id".to_string(),
                ));
            }
            Ok(())
        }
        other => Err(StorageError::Invalid(format!(
            "unknown scope '{other}' (expected session|workspace|global)"
        ))),
    }
}

fn validate_title(title: &str) -> Result<(), StorageError> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(StorageError::Invalid("title is empty".to_string()));
    }
    if trimmed.len() > TASK_TITLE_MAX_BYTES {
        return Err(StorageError::Invalid(format!(
            "title exceeds {TASK_TITLE_MAX_BYTES} bytes (got {})",
            trimmed.len()
        )));
    }
    Ok(())
}

fn validate_priority(priority: i64) -> Result<(), StorageError> {
    if (1..=4).contains(&priority) {
        Ok(())
    } else {
        Err(StorageError::Invalid(format!(
            "priority {priority} out of range 1..=4 (1 is highest)"
        )))
    }
}

/// Validate the acceptance-check JSON shape at write time so the harness
/// never meets a malformed check at run time.
fn validate_acceptance(value: &serde_json::Value) -> Result<(), StorageError> {
    let kind = value
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            StorageError::Invalid(
                "acceptance requires a 'kind' of command|file_exists|regex".to_string(),
            )
        })?;
    let require_str = |field: &str| -> Result<(), StorageError> {
        if value
            .get(field)
            .and_then(serde_json::Value::as_str)
            .is_none_or(str::is_empty)
        {
            return Err(StorageError::Invalid(format!(
                "acceptance kind '{kind}' requires a non-empty '{field}'"
            )));
        }
        Ok(())
    };
    // timeout_secs must be an unsigned integer when present — the harness
    // deserializes it strictly, and a shape that validates here but fails to
    // parse there would wedge every run in the scope.
    if let Some(timeout) = value.get("timeout_secs") {
        if !timeout.is_u64() {
            return Err(StorageError::Invalid(
                "acceptance 'timeout_secs' must be an unsigned integer (seconds)".to_string(),
            ));
        }
    }
    match kind {
        "command" => require_str("command"),
        "file_exists" => require_str("path"),
        "regex" => {
            require_str("pattern")?;
            // regex needs a target: a file path or a command whose output is matched
            if value
                .get("path")
                .and_then(serde_json::Value::as_str)
                .is_none()
                && value
                    .get("command")
                    .and_then(serde_json::Value::as_str)
                    .is_none()
            {
                return Err(StorageError::Invalid(
                    "acceptance kind 'regex' requires 'path' or 'command' to match against"
                        .to_string(),
                ));
            }
            Ok(())
        }
        other => Err(StorageError::Invalid(format!(
            "unknown acceptance kind '{other}' (expected command|file_exists|regex)"
        ))),
    }
}

fn is_blocked(task: &Task, by_id: &HashMap<i64, &Task>) -> bool {
    task.depends_on.iter().any(|dep| {
        by_id
            .get(dep)
            .is_some_and(|t| t.status != "done" && t.status != "cancelled")
    })
}

/// Depth of a task counted from the root (root = 0). Errors when the parent
/// chain exceeds the depth bound (which would indicate a corrupted store).
fn parent_depth(by_id: &HashMap<i64, &Task>, id: i64) -> Result<usize, StorageError> {
    let mut depth = 0usize;
    let mut current = Some(id);
    while let Some(task_id) = current {
        if depth >= TASK_DEPTH_MAX {
            return Err(StorageError::Invalid(format!(
                "hierarchy depth limit {TASK_DEPTH_MAX} exceeded at task #{task_id}"
            )));
        }
        current = by_id.get(&task_id).and_then(|t| t.parent_id);
        if current.is_some() {
            depth += 1;
        }
    }
    Ok(depth)
}

/// Lenient depth for delete ordering (never errors; saturates at the bound).
fn parent_depth_lenient(tasks: &[Task], id: i64) -> usize {
    let by_id: HashMap<i64, &Task> = tasks.iter().map(|t| (t.id, t)).collect();
    let mut depth = 0usize;
    let mut current = by_id.get(&id).and_then(|t| t.parent_id);
    while let Some(parent_id) = current {
        depth += 1;
        if depth >= TASK_DEPTH_MAX {
            break;
        }
        current = by_id.get(&parent_id).and_then(|t| t.parent_id);
    }
    depth
}

/// The set of a task's ancestors: the parent chain starting at
/// `start_parent`, bounded by the depth limit.
fn ancestor_chain(by_id: &HashMap<i64, &Task>, start_parent: Option<i64>) -> HashSet<i64> {
    let mut ancestors = HashSet::new();
    let mut current = start_parent;
    while let Some(id) = current {
        if !ancestors.insert(id) || ancestors.len() >= TASK_DEPTH_MAX {
            break;
        }
        current = by_id.get(&id).and_then(|t| t.parent_id);
    }
    ancestors
}

/// Reject dependencies that (transitively) reach one of the task's ancestors.
/// The parent-completion invariant makes an ancestor an implicit dependent of
/// this task — depending on it creates an unresolvable mutual wait that
/// `next()` would report as an (untrue) empty plan.
fn check_ancestor_dependency(
    by_id: &HashMap<i64, &Task>,
    ancestors: &HashSet<i64>,
    deps: &[i64],
) -> Result<(), StorageError> {
    if ancestors.is_empty() || deps.is_empty() {
        return Ok(());
    }
    let mut visited: HashSet<i64> = HashSet::new();
    let mut queue: VecDeque<i64> = deps.iter().copied().collect();
    while let Some(current) = queue.pop_front() {
        if ancestors.contains(&current) {
            return Err(StorageError::Invalid(format!(
                "task #{current} is an ancestor of this task; depending on it (directly or \
                 transitively) would wedge both — an ancestor cannot complete while its \
                 children are open"
            )));
        }
        if !visited.insert(current) {
            continue;
        }
        if let Some(task) = by_id.get(&current) {
            queue.extend(task.depends_on.iter().copied());
        }
    }
    Ok(())
}

/// Every task in the subtree rooted at `root` (including `root`), bounded by
/// the number of tasks in the map.
fn subtree_ids_from(by_id: &HashMap<i64, &Task>, root: i64) -> Vec<i64> {
    let mut ids = vec![root];
    let mut queue: VecDeque<i64> = VecDeque::from([root]);
    let mut seen: HashSet<i64> = HashSet::from([root]);
    while let Some(current) = queue.pop_front() {
        for task in by_id.values() {
            if task.parent_id == Some(current) && seen.insert(task.id) {
                ids.push(task.id);
                queue.push_back(task.id);
            }
        }
    }
    ids
}

/// Height of the subtree below `root` (0 = leaf), saturating at the depth
/// bound.
fn subtree_height(by_id: &HashMap<i64, &Task>, root: i64) -> usize {
    let mut height = 0usize;
    let mut frontier = vec![root];
    let mut seen: HashSet<i64> = HashSet::from([root]);
    for _ in 0..TASK_DEPTH_MAX {
        let mut next_frontier = Vec::new();
        for current in &frontier {
            for task in by_id.values() {
                if task.parent_id == Some(*current) && seen.insert(task.id) {
                    next_frontier.push(task.id);
                }
            }
        }
        if next_frontier.is_empty() {
            break;
        }
        height += 1;
        frontier = next_frontier;
    }
    height
}

/// Reject dependency graphs where following `depends_on` edges from `start`
/// can reach `start` again (cycle check on write — reject, don't detect
/// later). Bounded by the number of tasks in scope.
fn check_dependency_cycle(by_id: &HashMap<i64, &Task>, start: i64) -> Result<(), StorageError> {
    let Some(start_task) = by_id.get(&start) else {
        return Ok(());
    };
    let mut visited: HashSet<i64> = HashSet::new();
    let mut queue: VecDeque<i64> = start_task.depends_on.iter().copied().collect();
    while let Some(current) = queue.pop_front() {
        if current == start {
            return Err(StorageError::Invalid(format!(
                "dependency cycle: task #{start} would (transitively) depend on itself"
            )));
        }
        if !visited.insert(current) {
            continue;
        }
        if let Some(task) = by_id.get(&current) {
            queue.extend(task.depends_on.iter().copied());
        }
    }
    Ok(())
}

/// Reject parent chains that loop back to `start`.
fn check_parent_cycle(by_id: &HashMap<i64, &Task>, start: i64) -> Result<(), StorageError> {
    let mut current = by_id.get(&start).and_then(|t| t.parent_id);
    let mut steps = 0usize;
    while let Some(parent_id) = current {
        if parent_id == start {
            return Err(StorageError::Invalid(format!(
                "hierarchy cycle: task #{start} would be its own ancestor"
            )));
        }
        steps += 1;
        if steps >= TASK_DEPTH_MAX {
            return Err(StorageError::Invalid(format!(
                "hierarchy depth limit {TASK_DEPTH_MAX} exceeded"
            )));
        }
        current = by_id.get(&parent_id).and_then(|t| t.parent_id);
    }
    Ok(())
}

fn decode_task_row(row: &turso::Row) -> Result<Task, StorageError> {
    let labels_str: String = row.get(9)?;
    let tool_scope_str: String = row.get(10)?;
    let depends_str: String = row.get(13)?;
    let acceptance_str: Option<String> = row.get(14)?;
    Ok(Task {
        id: row.get(0)?,
        parent_id: row.get(1)?,
        scope: row.get(2)?,
        scope_id: row.get(3)?,
        project: row.get(4)?,
        title: row.get(5)?,
        description: row.get(6)?,
        status: row.get(7)?,
        priority: row.get(8)?,
        labels: serde_json::from_str(&labels_str).unwrap_or_default(),
        tool_scope: serde_json::from_str(&tool_scope_str).unwrap_or_default(),
        due_at: row.get(11)?,
        recurrence: row.get(12)?,
        depends_on: serde_json::from_str(&depends_str).unwrap_or_default(),
        acceptance: acceptance_str.and_then(|s| serde_json::from_str(&s).ok()),
        assignee: row.get(15)?,
        sort_order: row.get(16)?,
        created_at: row.get(17)?,
        updated_at: row.get(18)?,
        completed_at: row.get(19)?,
        blocked: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Storage;

    async fn repo() -> (Storage, TaskRepository) {
        let storage = Storage::in_memory().await.unwrap();
        let tasks = storage.tasks();
        (storage, tasks)
    }

    fn new_task(title: &str) -> NewTask {
        NewTask {
            scope: "session".to_string(),
            scope_id: Some("s1".to_string()),
            title: title.to_string(),
            priority: 3,
            ..NewTask::default()
        }
    }

    #[tokio::test]
    async fn create_assigns_id_and_logs_activity() {
        let (_s, repo) = repo().await;
        let task = repo.create(new_task("first")).await.unwrap();
        assert!(task.id >= 1, "expected positive id, got {}", task.id);
        assert_eq!(task.status, "pending");
        let activity = repo.activity(task.id, 10).await.unwrap();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].action, "created");
    }

    #[tokio::test]
    async fn create_rejects_empty_title_and_bad_priority_and_bad_scope() {
        let (_s, repo) = repo().await;
        let mut t = new_task("  ");
        assert!(matches!(
            repo.create(t).await,
            Err(StorageError::Invalid(_))
        ));
        t = new_task("ok");
        t.priority = 0;
        assert!(matches!(
            repo.create(t).await,
            Err(StorageError::Invalid(_))
        ));
        t = new_task("ok");
        t.priority = 5;
        assert!(matches!(
            repo.create(t).await,
            Err(StorageError::Invalid(_))
        ));
        t = new_task("ok");
        t.scope = "galaxy".to_string();
        assert!(matches!(
            repo.create(t).await,
            Err(StorageError::Invalid(_))
        ));
        t = new_task("ok");
        t.scope_id = None;
        assert!(
            matches!(repo.create(t).await, Err(StorageError::Invalid(_))),
            "session scope requires scope_id"
        );
    }

    #[tokio::test]
    async fn scopes_are_disjoint() {
        let (_s, repo) = repo().await;
        repo.create(new_task("session task")).await.unwrap();
        let mut g = new_task("global task");
        g.scope = "global".to_string();
        g.scope_id = None;
        repo.create(g).await.unwrap();

        let session = repo.list("session", Some("s1"), true).await.unwrap();
        let global = repo.list("global", None, true).await.unwrap();
        assert_eq!(session.len(), 1);
        assert_eq!(global.len(), 1);
        assert_eq!(session[0].title, "session task");
        assert_eq!(global[0].title, "global task");
    }

    #[tokio::test]
    async fn blocked_is_derived_from_open_dependencies() {
        let (_s, repo) = repo().await;
        let dep = repo.create(new_task("dep")).await.unwrap();
        let mut nt = new_task("dependent");
        nt.depends_on = vec![dep.id];
        let dependent = repo.create(nt).await.unwrap();

        let fetched = repo.get(dependent.id).await.unwrap();
        assert!(fetched.blocked, "open dependency must derive blocked=true");

        repo.complete(dep.id, None, None).await.unwrap();
        let fetched = repo.get(dependent.id).await.unwrap();
        assert!(!fetched.blocked, "completed dependency must unblock");
    }

    #[tokio::test]
    async fn blocked_cannot_be_written_directly() {
        let (_s, repo) = repo().await;
        let task = repo.create(new_task("t")).await.unwrap();
        let err = repo
            .update(
                task.id,
                TaskPatch {
                    status: Some("blocked".to_string()),
                    ..TaskPatch::default()
                },
                None,
            )
            .await;
        assert!(matches!(err, Err(StorageError::Invalid(_))));
    }

    #[tokio::test]
    async fn done_cannot_be_written_via_update() {
        let (_s, repo) = repo().await;
        let task = repo.create(new_task("t")).await.unwrap();
        let err = repo
            .update(
                task.id,
                TaskPatch {
                    status: Some("done".to_string()),
                    ..TaskPatch::default()
                },
                None,
            )
            .await;
        assert!(
            matches!(err, Err(StorageError::Invalid(_))),
            "done must go through complete()"
        );
    }

    #[tokio::test]
    async fn dependency_cycle_is_rejected_on_write() {
        let (_s, repo) = repo().await;
        let a = repo.create(new_task("a")).await.unwrap();
        let mut nt = new_task("b");
        nt.depends_on = vec![a.id];
        let b = repo.create(nt).await.unwrap();

        // a -> b would close the loop a -> b -> a
        let err = repo
            .update(
                a.id,
                TaskPatch {
                    depends_on: Some(vec![b.id]),
                    ..TaskPatch::default()
                },
                None,
            )
            .await;
        assert!(
            matches!(err, Err(StorageError::Invalid(_))),
            "cycle must be rejected"
        );

        // self-dependency is the smallest cycle
        let err = repo
            .update(
                a.id,
                TaskPatch {
                    depends_on: Some(vec![a.id]),
                    ..TaskPatch::default()
                },
                None,
            )
            .await;
        assert!(matches!(err, Err(StorageError::Invalid(_))));
    }

    #[tokio::test]
    async fn parent_cycle_is_rejected_on_write() {
        let (_s, repo) = repo().await;
        let a = repo.create(new_task("a")).await.unwrap();
        let mut nt = new_task("b");
        nt.parent_id = Some(a.id);
        let b = repo.create(nt).await.unwrap();
        let err = repo
            .update(
                a.id,
                TaskPatch {
                    parent_id: Some(Some(b.id)),
                    ..TaskPatch::default()
                },
                None,
            )
            .await;
        assert!(matches!(err, Err(StorageError::Invalid(_))));
    }

    #[tokio::test]
    async fn parent_cannot_complete_with_open_children() {
        let (_s, repo) = repo().await;
        let parent = repo.create(new_task("parent")).await.unwrap();
        let mut nt = new_task("child");
        nt.parent_id = Some(parent.id);
        let child = repo.create(nt).await.unwrap();

        let err = repo.complete(parent.id, None, None).await;
        assert!(
            matches!(err, Err(StorageError::Invalid(_))),
            "open child must block parent completion"
        );

        repo.complete(child.id, None, None).await.unwrap();
        // Parent without acceptance auto-completed with its last child.
        let parent = repo.get(parent.id).await.unwrap();
        assert_eq!(parent.status, "done", "parent should auto-complete");
    }

    #[tokio::test]
    async fn parent_with_acceptance_does_not_auto_complete() {
        let (_s, repo) = repo().await;
        let mut np = new_task("parent");
        np.acceptance = Some(serde_json::json!({"kind": "file_exists", "path": "out.txt"}));
        let parent = repo.create(np).await.unwrap();
        let mut nc = new_task("child");
        nc.parent_id = Some(parent.id);
        let child = repo.create(nc).await.unwrap();

        repo.complete(child.id, None, None).await.unwrap();
        let parent = repo.get(parent.id).await.unwrap();
        assert_eq!(
            parent.status, "pending",
            "a parent with an acceptance check must be completed explicitly"
        );
    }

    #[tokio::test]
    async fn complete_is_idempotent() {
        let (_s, repo) = repo().await;
        let task = repo.create(new_task("t")).await.unwrap();
        let first = repo.complete(task.id, Some("agent"), None).await.unwrap();
        assert!(!first.already_done);
        let second = repo.complete(task.id, Some("agent"), None).await.unwrap();
        assert!(second.already_done);
    }

    #[tokio::test]
    async fn next_prefers_in_progress_then_priority_then_due() {
        let (_s, repo) = repo().await;
        let mut a = new_task("low prio");
        a.priority = 4;
        let a = repo.create(a).await.unwrap();
        let mut b = new_task("high prio");
        b.priority = 1;
        let b = repo.create(b).await.unwrap();

        let next = repo.next("session", Some("s1")).await.unwrap().unwrap();
        assert_eq!(next.id, b.id, "p1 beats p4");

        // An in_progress item beats a higher-priority pending one.
        repo.update(
            a.id,
            TaskPatch {
                status: Some("in_progress".to_string()),
                ..TaskPatch::default()
            },
            None,
        )
        .await
        .unwrap();
        let next = repo.next("session", Some("s1")).await.unwrap().unwrap();
        assert_eq!(next.id, a.id, "in_progress resumes before pending");
    }

    #[tokio::test]
    async fn next_skips_blocked_and_parents_with_open_children() {
        let (_s, repo) = repo().await;
        let parent = repo.create(new_task("parent")).await.unwrap();
        let mut nc = new_task("child");
        nc.parent_id = Some(parent.id);
        nc.priority = 2;
        let child = repo.create(nc).await.unwrap();
        let mut nb = new_task("blocked one");
        nb.priority = 1;
        nb.depends_on = vec![child.id];
        repo.create(nb).await.unwrap();

        let next = repo.next("session", Some("s1")).await.unwrap().unwrap();
        assert_eq!(
            next.id, child.id,
            "parent has open children and p1 item is blocked; the child is the one actionable item"
        );
    }

    #[tokio::test]
    async fn next_returns_none_when_everything_is_closed() {
        let (_s, repo) = repo().await;
        let t = repo.create(new_task("only")).await.unwrap();
        repo.complete(t.id, None, None).await.unwrap();
        assert!(repo.next("session", Some("s1")).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn cancelling_a_parent_cascades_to_open_descendants() {
        let (_s, repo) = repo().await;
        let parent = repo.create(new_task("parent")).await.unwrap();
        let mut nc = new_task("child");
        nc.parent_id = Some(parent.id);
        let child = repo.create(nc).await.unwrap();
        let mut ng = new_task("grandchild");
        ng.parent_id = Some(child.id);
        let grandchild = repo.create(ng).await.unwrap();

        repo.update(
            parent.id,
            TaskPatch {
                status: Some("cancelled".to_string()),
                ..TaskPatch::default()
            },
            None,
        )
        .await
        .unwrap();

        assert_eq!(repo.get(child.id).await.unwrap().status, "cancelled");
        assert_eq!(repo.get(grandchild.id).await.unwrap().status, "cancelled");
        assert!(repo.next("session", Some("s1")).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn cancelled_dependency_does_not_block() {
        let (_s, repo) = repo().await;
        let dep = repo.create(new_task("dep")).await.unwrap();
        let mut nt = new_task("dependent");
        nt.depends_on = vec![dep.id];
        let dependent = repo.create(nt).await.unwrap();
        repo.update(
            dep.id,
            TaskPatch {
                status: Some("cancelled".to_string()),
                ..TaskPatch::default()
            },
            None,
        )
        .await
        .unwrap();
        assert!(!repo.get(dependent.id).await.unwrap().blocked);
    }

    #[tokio::test]
    async fn notes_are_append_only_and_bounded() {
        let (_s, repo) = repo().await;
        let task = repo.create(new_task("t")).await.unwrap();
        repo.add_note(task.id, Some("agent-a"), "found the bug")
            .await
            .unwrap();
        repo.add_note(task.id, Some("agent-b"), "fixed in commit")
            .await
            .unwrap();
        let notes = repo.notes(task.id, 10).await.unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].content, "found the bug", "oldest first");

        assert!(matches!(
            repo.add_note(task.id, None, "  ").await,
            Err(StorageError::Invalid(_))
        ));
        let oversized = "x".repeat(TASK_NOTE_MAX_BYTES + 1);
        assert!(matches!(
            repo.add_note(task.id, None, &oversized).await,
            Err(StorageError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn query_filters_with_derived_blocked() {
        let (_s, repo) = repo().await;
        let dep = repo.create(new_task("dep")).await.unwrap();
        let mut nt = new_task("dependent");
        nt.depends_on = vec![dep.id];
        nt.priority = 1;
        repo.create(nt).await.unwrap();

        let blocked = repo.query("session", Some("s1"), "blocked").await.unwrap();
        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0].title, "dependent");

        let actionable = repo
            .query("session", Some("s1"), "!blocked & !done")
            .await
            .unwrap();
        assert_eq!(actionable.len(), 1);
        assert_eq!(actionable[0].title, "dep");
    }

    #[tokio::test]
    async fn query_rejects_malformed_filters() {
        let (_s, repo) = repo().await;
        assert!(matches!(
            repo.query("session", Some("s1"), "banana &").await,
            Err(StorageError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn delete_removes_subtree_and_dangling_deps() {
        let (_s, repo) = repo().await;
        let parent = repo.create(new_task("parent")).await.unwrap();
        let mut nc = new_task("child");
        nc.parent_id = Some(parent.id);
        let child = repo.create(nc).await.unwrap();
        let mut no = new_task("outsider");
        no.depends_on = vec![child.id];
        let outsider = repo.create(no).await.unwrap();

        let deleted = repo.delete(parent.id, None).await.unwrap();
        assert_eq!(deleted, 2, "parent + child");
        assert!(repo.get(parent.id).await.is_err());
        assert!(repo.get(child.id).await.is_err());
        let outsider = repo.get(outsider.id).await.unwrap();
        assert!(
            outsider.depends_on.is_empty(),
            "dangling dependency must be stripped, got {:?}",
            outsider.depends_on
        );
        assert!(!outsider.blocked);
    }

    #[tokio::test]
    async fn clear_closed_keeps_open_tasks() {
        let (_s, repo) = repo().await;
        let done = repo.create(new_task("done one")).await.unwrap();
        repo.complete(done.id, None, None).await.unwrap();
        repo.create(new_task("open one")).await.unwrap();

        let removed = repo.clear("session", Some("s1"), true).await.unwrap();
        assert_eq!(removed, 1);
        let remaining = repo.list("session", Some("s1"), true).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].title, "open one");
    }

    #[tokio::test]
    async fn import_v01_preserves_order_and_maps_statuses() {
        let (_s, repo) = repo().await;
        let items = vec![
            ("first".to_string(), "done".to_string()),
            ("second".to_string(), "in_progress".to_string()),
            ("third".to_string(), "blocked".to_string()),
            (String::new(), "pending".to_string()),
        ];
        let imported = repo.import_v01("s1", &items).await.unwrap();
        assert_eq!(imported, 3, "empty titles are skipped");

        let tasks = repo.list("session", Some("s1"), true).await.unwrap();
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].title, "first");
        assert_eq!(tasks[0].status, "done");
        assert_eq!(tasks[1].status, "in_progress");
        assert_eq!(
            tasks[2].status, "pending",
            "v0.1 'blocked' becomes pending (blocked is derived now)"
        );
    }

    #[tokio::test]
    async fn acceptance_shape_is_validated_on_write() {
        let (_s, repo) = repo().await;
        let mut nt = new_task("bad acceptance");
        nt.acceptance = Some(serde_json::json!({"kind": "vibes"}));
        assert!(matches!(
            repo.create(nt).await,
            Err(StorageError::Invalid(_))
        ));

        let mut nt = new_task("regex needs target");
        nt.acceptance = Some(serde_json::json!({"kind": "regex", "pattern": "ok"}));
        assert!(matches!(
            repo.create(nt).await,
            Err(StorageError::Invalid(_))
        ));

        let mut nt = new_task("good");
        nt.acceptance = Some(serde_json::json!({
            "kind": "command", "command": "cargo test -p nanna-storage"
        }));
        assert!(repo.create(nt).await.is_ok());
    }

    #[tokio::test]
    async fn dependency_on_ancestor_is_rejected_on_create_and_update() {
        let (_s, repo) = repo().await;
        let grandparent = repo.create(new_task("grandparent")).await.unwrap();
        let mut np = new_task("parent");
        np.parent_id = Some(grandparent.id);
        let parent = repo.create(np).await.unwrap();

        // create: child depending on its direct parent wedges both forever
        let mut nc = new_task("child");
        nc.parent_id = Some(parent.id);
        nc.depends_on = vec![parent.id];
        assert!(
            matches!(repo.create(nc).await, Err(StorageError::Invalid(_))),
            "dep on direct parent must be rejected"
        );

        // create: dep on a grandparent is the same wedge, transitively
        let mut nc = new_task("child");
        nc.parent_id = Some(parent.id);
        nc.depends_on = vec![grandparent.id];
        assert!(matches!(
            repo.create(nc).await,
            Err(StorageError::Invalid(_))
        ));

        // update: adding the ancestor dep later must be rejected too
        let mut nc = new_task("child");
        nc.parent_id = Some(parent.id);
        let child = repo.create(nc).await.unwrap();
        let err = repo
            .update(
                child.id,
                TaskPatch {
                    depends_on: Some(vec![grandparent.id]),
                    ..TaskPatch::default()
                },
                None,
            )
            .await;
        assert!(matches!(err, Err(StorageError::Invalid(_))));
    }

    #[tokio::test]
    async fn reparent_under_closed_parent_is_rejected() {
        let (_s, repo) = repo().await;
        let done_parent = repo.create(new_task("done parent")).await.unwrap();
        repo.complete(done_parent.id, None, None).await.unwrap();
        let orphan = repo.create(new_task("orphan")).await.unwrap();
        let err = repo
            .update(
                orphan.id,
                TaskPatch {
                    parent_id: Some(Some(done_parent.id)),
                    ..TaskPatch::default()
                },
                None,
            )
            .await;
        assert!(
            matches!(err, Err(StorageError::Invalid(_))),
            "re-parenting under a done parent breaks the parent-done invariant"
        );
    }

    #[tokio::test]
    async fn reopen_via_update_clears_completed_at() {
        let (_s, repo) = repo().await;
        let task = repo.create(new_task("t")).await.unwrap();
        repo.complete(task.id, None, None).await.unwrap();
        assert!(repo.get(task.id).await.unwrap().completed_at.is_some());
        let reopened = repo
            .update(
                task.id,
                TaskPatch {
                    status: Some("pending".to_string()),
                    ..TaskPatch::default()
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(reopened.status, "pending");
        assert!(
            reopened.completed_at.is_none(),
            "a reopened task must not keep a completion stamp"
        );
    }

    #[tokio::test]
    async fn clear_closed_never_deletes_open_descendants() {
        let (_s, repo) = repo().await;
        let parent = repo.create(new_task("parent")).await.unwrap();
        let mut nc = new_task("child");
        nc.parent_id = Some(parent.id);
        let child = repo.create(nc).await.unwrap();
        // Close the subtree (parent auto-completes), then reopen the child.
        repo.complete(child.id, None, None).await.unwrap();
        repo.update(
            child.id,
            TaskPatch {
                status: Some("pending".to_string()),
                ..TaskPatch::default()
            },
            None,
        )
        .await
        .unwrap();

        let removed = repo.clear("session", Some("s1"), true).await.unwrap();
        assert_eq!(
            removed, 0,
            "closed parent with an open child must be skipped"
        );
        assert!(
            repo.get(child.id).await.is_ok(),
            "open child must survive clear"
        );
    }

    #[tokio::test]
    async fn import_v01_truncates_multibyte_titles_on_char_boundary() {
        let (_s, repo) = repo().await;
        // 200 x 4-byte chars = 800 bytes; byte 500 is mid-char.
        let long_title = "🌀".repeat(200);
        let items = vec![(long_title, "pending".to_string())];
        let imported = repo.import_v01("s1", &items).await.unwrap();
        assert_eq!(imported, 1);
        let tasks = repo.list("session", Some("s1"), true).await.unwrap();
        assert!(tasks[0].title.len() <= TASK_TITLE_MAX_BYTES);
        assert!(tasks[0].title.chars().all(|c| c == '🌀'));
    }

    #[tokio::test]
    async fn import_v01_is_idempotent() {
        let (_s, repo) = repo().await;
        let items = vec![("only".to_string(), "pending".to_string())];
        assert_eq!(repo.import_v01("s1", &items).await.unwrap(), 1);
        assert_eq!(
            repo.import_v01("s1", &items).await.unwrap(),
            0,
            "a second import must not duplicate"
        );
        assert_eq!(
            repo.list("session", Some("s1"), true).await.unwrap().len(),
            1
        );
    }

    #[tokio::test]
    async fn acceptance_timeout_type_is_validated() {
        let (_s, repo) = repo().await;
        let mut nt = new_task("bad timeout");
        nt.acceptance = Some(serde_json::json!({
            "kind": "command", "command": "exit 0", "timeout_secs": "five"
        }));
        assert!(matches!(
            repo.create(nt).await,
            Err(StorageError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn counts_reports_open_and_closed() {
        let (_s, repo) = repo().await;
        let a = repo.create(new_task("a")).await.unwrap();
        repo.create(new_task("b")).await.unwrap();
        repo.complete(a.id, None, None).await.unwrap();
        let (open, closed) = repo.counts("session", Some("s1")).await.unwrap();
        assert_eq!((open, closed), (1, 1));
    }
}
