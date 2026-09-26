//! Every card and every thread post becomes a workspace memory (P25 Stage 1).
//!
//! P25 decision 14: "Every task is a memory. Every thread post is a memory."
//! The thread stays the permanent record and is never compacted; what dreaming
//! consolidates is these *copies*, each carrying a pointer back to the card and
//! the post it came from (`source_task_id`, `source_note_id`), so a recalled
//! memory can always be followed back to the words it was made from.
//!
//! **Fed by the store's own event sink, not by a bus subscriber.** A
//! `broadcast` receiver that falls behind is told it lagged and the skipped
//! events are gone — for a notification that is a missed toast, here it is a
//! memory that silently never exists. So the sink that already sees every task
//! write ([`crate::task_event_bridge::TaskEventBridge`]) also hands the few
//! kinds that matter to a bounded `mpsc` queue, which one worker drains in
//! order. A full queue is reported, never waited on: the sink is called from
//! the task store's write path, and that path must not block.
//!
//! **Board cards only.** A `session`-scoped task is chat scaffolding — the
//! harness writes dozens per turn — and P25 decision 9 deletes that scope.
//! Copying those would flood memory with plan steps; they are skipped until
//! the session → workspace promotion makes every card a board card.

use nanna_memory::MemoryService;
use nanna_storage::{Storage, StorageError, Task, TaskEvent, TaskEventKind};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Events the write-through queue holds before the sink starts reporting drops.
///
/// Bound justification: one scope holds at most
/// [`nanna_storage::TASKS_PER_SCOPE_MAX`] cards, and the largest single burst
/// the store emits is a cascade over one scope, so a whole scope's worth of
/// events fits even if the worker has not started draining yet (the queue is
/// created with the store, before the memory service exists). Each queued
/// event is a few hundred bytes, so the bound is a few MiB at worst.
pub const WRITE_THROUGH_QUEUE_MAX: usize = nanna_storage::TASKS_PER_SCOPE_MAX;

/// Importance given to a board copy, on the memory service's 1–5 scale.
///
/// The middle of the scale: a card is neither an offhand remark nor a stated
/// fact about the user, and dreaming — not this module — is what decides which
/// copies matter.
const COPY_IMPORTANCE: f32 = 3.0;

/// What a store event becomes in memory, decided without any I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// A card was created.
    Created,
    /// A card was closed: completed (a verdict) or cancelled.
    Closed { cancelled: bool },
    /// A post was added to a card's thread.
    Posted { note_id: i64 },
}

impl Trigger {
    /// The trigger for `event`, or `None` when it produces no memory.
    #[must_use]
    pub fn of(event: &TaskEvent) -> Option<Self> {
        if event.scope == "session" {
            return None;
        }
        match event.kind {
            TaskEventKind::Created => Some(Self::Created),
            TaskEventKind::Verdict => Some(Self::Closed { cancelled: false }),
            TaskEventKind::StatusChanged
                if event.detail.get("status").and_then(|s| s.as_str()) == Some("cancelled") =>
            {
                Some(Self::Closed { cancelled: true })
            }
            TaskEventKind::Posted => event
                .detail
                .get("note_id")
                .and_then(serde_json::Value::as_i64)
                .map(|note_id| Self::Posted { note_id }),
            _ => None,
        }
    }
}

/// The memory a trigger writes: its text and its pointers back to the board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardCopy {
    pub content: String,
    pub metadata: HashMap<String, String>,
    /// `Some(id)` for a workspace card; `None` for a global one.
    pub workspace_id: Option<String>,
}

/// Build the copy for `trigger` on `task`. Pure.
///
/// The card's title leads every copy so a recalled post still says what it
/// was about — a bare "done, tests pass" is useless without its card.
#[must_use]
pub fn board_copy(
    task: &Task,
    trigger: Trigger,
    post: Option<&nanna_storage::TaskNote>,
) -> BoardCopy {
    let mut metadata = HashMap::from([
        ("source".to_string(), "task_board".to_string()),
        ("source_task_id".to_string(), task.id.to_string()),
    ]);
    if !task.labels.is_empty() {
        metadata.insert("labels".to_string(), task.labels.join(","));
    }
    let header = format!("Card #{} \"{}\"", task.id, task.title);
    let content = match trigger {
        Trigger::Created => {
            metadata.insert("board_event".to_string(), "created".to_string());
            match task.description.as_deref().map(str::trim) {
                Some(description) if !description.is_empty() => {
                    format!("{header} was created.\n{description}")
                }
                _ => format!("{header} was created."),
            }
        }
        Trigger::Closed { cancelled } => {
            let outcome = if cancelled { "cancelled" } else { "completed" };
            metadata.insert("board_event".to_string(), outcome.to_string());
            format!("{header} was {outcome}.")
        }
        Trigger::Posted { note_id } => {
            metadata.insert("board_event".to_string(), "posted".to_string());
            metadata.insert("source_note_id".to_string(), note_id.to_string());
            let (kind, by, text) = post.map_or(("post", None, ""), |note| {
                (
                    note.kind.as_str(),
                    note.author_member_id.as_deref().or(note.author.as_deref()),
                    note.content.as_str(),
                )
            });
            metadata.insert("post_kind".to_string(), kind.to_string());
            // Only a real member id is recorded as one; a pre-members actor
            // string (`gui`, `harness`) still names the author in the text.
            if let Some(member) = post.and_then(|note| note.author_member_id.as_deref()) {
                metadata.insert("author_member_id".to_string(), member.to_string());
            }
            let by = by.map(|by| format!(" by {by}")).unwrap_or_default();
            format!("{header} — {kind}{by}:\n{text}")
        }
    };
    let workspace_id = if task.scope == "workspace" {
        task.scope_id.clone()
    } else {
        None
    };
    debug_assert!(!content.is_empty(), "a copy always has its header");
    debug_assert!(
        metadata.contains_key("source_task_id"),
        "a copy points at its card"
    );
    BoardCopy {
        content,
        metadata,
        workspace_id,
    }
}

/// Write one event's copy. `Ok(None)` when there was nothing to write — the
/// card or post was deleted before the worker reached it.
///
/// # Errors
/// Returns a message when the store cannot be read or the memory write fails.
pub async fn write_one(
    storage: &Storage,
    memory: &MemoryService,
    event: &TaskEvent,
) -> Result<Option<String>, String> {
    let Some(trigger) = Trigger::of(event) else {
        return Ok(None);
    };
    let task = match storage.tasks().get(event.task_id).await {
        Ok(task) => task,
        Err(StorageError::NotFound(_)) => return Ok(None),
        Err(e) => return Err(format!("reading card #{}: {e}", event.task_id)),
    };
    let post = match trigger {
        Trigger::Posted { note_id } => match storage.tasks().note(note_id).await {
            Ok(note) => Some(note),
            Err(StorageError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(format!("reading post #{note_id}: {e}")),
        },
        Trigger::Created | Trigger::Closed { .. } => None,
    };
    let copy = board_copy(&task, trigger, post.as_ref());
    memory
        .remember_deferred_vector(
            &copy.content,
            copy.metadata,
            COPY_IMPORTANCE,
            copy.workspace_id,
        )
        .await
        .map(|(id, _)| Some(id))
        .map_err(|e| format!("writing the memory copy of card #{}: {e}", task.id))
}

/// Drain the write-through queue until every sender is gone.
///
/// Sequential on purpose: one card's created → posted → completed copies land
/// in the order they happened, and each write is a single durable insert.
pub async fn run(
    mut events: mpsc::Receiver<TaskEvent>,
    storage: Arc<Storage>,
    memory: Arc<MemoryService>,
) {
    while let Some(event) = events.recv().await {
        match write_one(&storage, &memory, &event).await {
            Ok(Some(id)) => debug!("board copy {id} written for card #{}", event.task_id),
            Ok(None) => {}
            Err(e) => warn!("board memory write-through failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nanna_storage::{NewTask, TaskEventSink, TaskNoteKind};

    fn event(kind: TaskEventKind, scope: &str, detail: serde_json::Value) -> TaskEvent {
        TaskEvent {
            kind,
            task_id: 1,
            scope: scope.to_string(),
            scope_id: Some("ws1".to_string()),
            actor: None,
            detail,
        }
    }

    #[test]
    fn only_board_creations_closures_and_posts_become_memories() {
        let none = serde_json::json!({});
        assert_eq!(
            Trigger::of(&event(TaskEventKind::Created, "workspace", none.clone())),
            Some(Trigger::Created)
        );
        assert_eq!(
            Trigger::of(&event(TaskEventKind::Verdict, "global", none.clone())),
            Some(Trigger::Closed { cancelled: false })
        );
        let cancelled = serde_json::json!({ "status": "cancelled" });
        assert_eq!(
            Trigger::of(&event(TaskEventKind::StatusChanged, "workspace", cancelled)),
            Some(Trigger::Closed { cancelled: true })
        );
        let started = serde_json::json!({ "status": "in_progress" });
        assert_eq!(
            Trigger::of(&event(TaskEventKind::StatusChanged, "workspace", started)),
            None
        );
        let posted = serde_json::json!({ "note_id": 9 });
        assert_eq!(
            Trigger::of(&event(TaskEventKind::Posted, "workspace", posted.clone())),
            Some(Trigger::Posted { note_id: 9 })
        );
        assert_eq!(
            Trigger::of(&event(TaskEventKind::Posted, "session", posted)),
            None,
            "chat scaffolding is not a board card"
        );
        assert_eq!(
            Trigger::of(&event(TaskEventKind::Assigned, "workspace", none)),
            None
        );
    }

    async fn wait_for_count(memory: &MemoryService, want: usize) -> usize {
        for _ in 0..200 {
            let have = memory.count().await;
            if have >= want {
                return have;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        memory.count().await
    }

    /// A store whose task events feed a running write-through worker.
    async fn board_with_write_through() -> (Arc<Storage>, Arc<MemoryService>) {
        let storage = Arc::new(Storage::in_memory().await.expect("storage"));
        let (bus, _keep) = tokio::sync::broadcast::channel(64);
        let (queue_tx, queue_rx) = mpsc::channel(WRITE_THROUGH_QUEUE_MAX);
        let bridge =
            crate::task_event_bridge::TaskEventBridge::new(bus).with_memory_write_through(queue_tx);
        assert!(storage.set_task_events(Arc::new(bridge) as Arc<dyn TaskEventSink>));
        let memory = Arc::new(MemoryService::new(
            nanna_memory::MemoryServiceConfig::default(),
        ));
        tokio::spawn(run(queue_rx, Arc::clone(&storage), Arc::clone(&memory)));

        (storage, memory)
    }

    /// The whole path: a card's life on the board lands in memory as three
    /// copies, in order, each pointing back at the card — and the post's copy
    /// at the post. A chat-scaffolding card on the same store leaves none.
    #[tokio::test]
    async fn a_cards_life_becomes_memories_that_point_back_at_it() {
        let (storage, memory) = board_with_write_through().await;
        let tasks = storage.tasks();
        let scaffolding = tasks
            .create(NewTask {
                scope: "session".to_string(),
                scope_id: Some("chat-1".to_string()),
                title: "plan step".to_string(),
                priority: 3,
                ..NewTask::default()
            })
            .await
            .expect("session card");
        let card = tasks
            .create(NewTask {
                scope: "workspace".to_string(),
                scope_id: Some("ws1".to_string()),
                title: "Ship the release".to_string(),
                description: Some("Tag, build, publish.".to_string()),
                priority: 2,
                labels: vec!["release".to_string()],
                ..NewTask::default()
            })
            .await
            .expect("board card");
        let note = tasks
            .post(
                card.id,
                Some("gui"),
                Some(nanna_storage::HUMAN_MEMBER_ID),
                TaskNoteKind::Question,
                "Which tag?",
            )
            .await
            .expect("post");
        tasks
            .complete(card.id, Some("gui"), None)
            .await
            .expect("complete");
        tasks
            .complete(scaffolding.id, Some("harness"), None)
            .await
            .expect("complete");

        assert_eq!(
            wait_for_count(&memory, 3).await,
            3,
            "three copies, none for the scaffolding"
        );
        let copies = memory.list_all().await;
        assert!(
            copies
                .iter()
                .all(|m| m.workspace_id.as_deref() == Some("ws1")),
            "{copies:?}"
        );
        assert!(
            copies
                .iter()
                .all(|m| m.metadata.get("source_task_id") == Some(&card.id.to_string())),
            "every copy points at its card: {copies:?}"
        );
        let post_copy = copies
            .iter()
            .find(|m| m.metadata.get("board_event").map(String::as_str) == Some("posted"))
            .expect("the post's copy");
        assert_eq!(
            post_copy.metadata.get("source_note_id"),
            Some(&note.id.to_string())
        );
        assert_eq!(
            post_copy
                .metadata
                .get("author_member_id")
                .map(String::as_str),
            Some(nanna_storage::HUMAN_MEMBER_ID)
        );
        assert!(
            post_copy.content.contains("Ship the release"),
            "{}",
            post_copy.content
        );
        assert!(
            post_copy.content.contains("Which tag?"),
            "{}",
            post_copy.content
        );
        let created = copies
            .iter()
            .find(|m| m.metadata.get("board_event").map(String::as_str) == Some("created"))
            .expect("the creation's copy");
        assert!(
            created.content.contains("Tag, build, publish."),
            "{}",
            created.content
        );
        assert_eq!(
            created.metadata.get("labels").map(String::as_str),
            Some("release")
        );
    }
}
