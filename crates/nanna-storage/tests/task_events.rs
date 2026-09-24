//! Task lifecycle events (P25 Stage 1).
//!
//! These events are emitted from [`TaskRepository`] rather than from a caller
//! because the repository is the only layer that sees every writer. The tests
//! that matter here are therefore the ones a caller-level emitter would fail:
//! an ancestor auto-completed by somebody else's `complete`, and a change that
//! did not actually change anything.

#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

use nanna_storage::{
    HUMAN_MEMBER_ID, NewTask, Storage, TaskEvent, TaskEventKind, TaskEventSink, TaskNoteKind,
    TaskPatch,
};
use std::sync::{Arc, Mutex};

/// Records everything published, so a test can assert on the whole sequence
/// rather than on one event in isolation — ordering is part of the contract
/// the router will read.
#[derive(Default)]
struct Recorder {
    events: Mutex<Vec<TaskEvent>>,
}

impl Recorder {
    fn kinds(&self) -> Vec<TaskEventKind> {
        self.events
            .lock()
            .expect("recorder not poisoned")
            .iter()
            .map(|e| e.kind)
            .collect()
    }

    fn of_kind(&self, kind: TaskEventKind) -> Vec<TaskEvent> {
        self.events
            .lock()
            .expect("recorder not poisoned")
            .iter()
            .filter(|e| e.kind == kind)
            .cloned()
            .collect()
    }

    fn clear(&self) {
        self.events.lock().expect("recorder not poisoned").clear();
    }
}

impl TaskEventSink for Recorder {
    fn publish(&self, event: TaskEvent) {
        self.events
            .lock()
            .expect("recorder not poisoned")
            .push(event);
    }
}

async fn storage_with_recorder() -> (Storage, Arc<Recorder>) {
    let storage = Storage::in_memory().await.expect("in-memory storage");
    let recorder = Arc::new(Recorder::default());
    assert!(
        storage.set_task_events(recorder.clone()),
        "first sink attaches"
    );
    (storage, recorder)
}

fn card(title: &str) -> NewTask {
    NewTask {
        scope: "workspace".to_string(),
        scope_id: Some("ws1".to_string()),
        title: title.to_string(),
        priority: 3,
        ..NewTask::default()
    }
}

#[tokio::test]
async fn creating_a_card_announces_it() {
    let (storage, recorder) = storage_with_recorder().await;
    let task = storage
        .tasks()
        .create(card("write the thing"))
        .await
        .expect("created");

    assert_eq!(recorder.kinds(), vec![TaskEventKind::Created]);
    let events = recorder.of_kind(TaskEventKind::Created);
    let event = events.first().expect("one created event");
    assert_eq!(event.task_id, task.id);
    assert_eq!(event.scope, "workspace");
    assert_eq!(event.scope_id.as_deref(), Some("ws1"));
    assert_eq!(event.detail["title"], "write the thing");
}

#[tokio::test]
async fn a_status_change_is_announced_once_and_only_when_it_changes() {
    let (storage, recorder) = storage_with_recorder().await;
    let repo = storage.tasks();
    let task = repo.create(card("a card")).await.expect("created");
    recorder.clear();

    repo.update(
        task.id,
        TaskPatch {
            status: Some("in_progress".to_string()),
            ..TaskPatch::default()
        },
        Some("gui"),
    )
    .await
    .expect("status updated");
    assert_eq!(recorder.kinds(), vec![TaskEventKind::StatusChanged]);

    // Re-writing the same status is not a transition, and a consumer woken by
    // one would be reacting to nothing.
    recorder.clear();
    repo.update(
        task.id,
        TaskPatch {
            status: Some("in_progress".to_string()),
            ..TaskPatch::default()
        },
        Some("gui"),
    )
    .await
    .expect("no-op status update");
    assert_eq!(recorder.kinds(), Vec::<TaskEventKind>::new());
}

#[tokio::test]
async fn reassignment_is_announced_but_a_no_op_reassign_is_not() {
    let (storage, recorder) = storage_with_recorder().await;
    let repo = storage.tasks();
    let task = repo.create(card("a card")).await.expect("created");
    recorder.clear();

    repo.update(
        task.id,
        TaskPatch {
            assignee: Some(Some(HUMAN_MEMBER_ID.to_string())),
            ..TaskPatch::default()
        },
        Some("router"),
    )
    .await
    .expect("assigned");
    let events = recorder.of_kind(TaskEventKind::Assigned);
    assert_eq!(events.len(), 1, "one assignment");
    assert_eq!(events[0].detail["assignee"], HUMAN_MEMBER_ID);
    assert_eq!(events[0].actor.as_deref(), Some("router"));

    // The regression this guards: `apply_patch` used to record an assignee
    // change whenever the field was present, with no old/new comparison, so
    // re-writing the same member looked like a hand-off and would have woken
    // the router for nothing.
    recorder.clear();
    repo.update(
        task.id,
        TaskPatch {
            assignee: Some(Some(HUMAN_MEMBER_ID.to_string())),
            ..TaskPatch::default()
        },
        Some("router"),
    )
    .await
    .expect("no-op reassign");
    assert_eq!(recorder.kinds(), Vec::<TaskEventKind>::new());
}

#[tokio::test]
async fn every_thread_post_is_announced() {
    let (storage, recorder) = storage_with_recorder().await;
    let repo = storage.tasks();
    let task = repo.create(card("a card")).await.expect("created");
    recorder.clear();

    let note = repo
        .post(
            task.id,
            Some("harness"),
            None,
            TaskNoteKind::Question,
            "which one?",
        )
        .await
        .expect("posted");
    let events = recorder.of_kind(TaskEventKind::Posted);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].detail["note_id"], note.id);
    assert_eq!(events[0].detail["kind"], "question");

    // `add_note` is the thin Comment wrapper over `post`; it must announce too,
    // or the harness's own findings would be the one silent writer.
    recorder.clear();
    repo.add_note(task.id, Some("harness"), "a finding")
        .await
        .expect("noted");
    assert_eq!(recorder.kinds(), vec![TaskEventKind::Posted]);
}

#[tokio::test]
async fn a_failed_post_announces_nothing() {
    let (storage, recorder) = storage_with_recorder().await;
    let repo = storage.tasks();
    let task = repo.create(card("a card")).await.expect("created");
    recorder.clear();

    repo.post(task.id, Some("harness"), None, TaskNoteKind::Comment, "   ")
        .await
        .expect_err("empty content is rejected");
    assert_eq!(
        recorder.kinds(),
        Vec::<TaskEventKind>::new(),
        "an event for a post nobody can read would be a lie"
    );
}

#[tokio::test]
async fn completing_announces_a_verdict() {
    let (storage, recorder) = storage_with_recorder().await;
    let repo = storage.tasks();
    let task = repo.create(card("a card")).await.expect("created");
    recorder.clear();

    repo.complete(task.id, Some("gui"), None)
        .await
        .expect("completed");
    let events = recorder.of_kind(TaskEventKind::Verdict);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].task_id, task.id);
    assert_eq!(events[0].detail["auto"], false);
}

/// The case that decides where these events live.
///
/// Completing the last child auto-completes the parent, and **no caller ever
/// names the parent**. An emitter sitting above the repository — in the IPC
/// handler or a tool service — would announce the child and let the parent go
/// done in silence.
#[tokio::test]
async fn an_ancestor_closed_by_a_cascade_gets_its_own_verdict() {
    let (storage, recorder) = storage_with_recorder().await;
    let repo = storage.tasks();
    let parent = repo
        .create(card("the parent"))
        .await
        .expect("parent created");
    let child = repo
        .create(NewTask {
            parent_id: Some(parent.id),
            ..card("the child")
        })
        .await
        .expect("child created");
    recorder.clear();

    let outcome = repo
        .complete(child.id, Some("harness"), None)
        .await
        .expect("completed");
    assert_eq!(
        outcome.auto_completed,
        vec![parent.id],
        "parent auto-completes"
    );

    let verdicts = recorder.of_kind(TaskEventKind::Verdict);
    assert_eq!(verdicts.len(), 2, "child and parent both announced");
    assert_eq!(verdicts[0].task_id, child.id);
    assert_eq!(verdicts[0].detail["auto"], false);
    assert_eq!(verdicts[1].task_id, parent.id);
    assert_eq!(verdicts[1].detail["auto"], true);
    assert_eq!(verdicts[1].detail["trigger"], child.id);
}

#[tokio::test]
async fn completing_an_already_done_card_announces_nothing() {
    let (storage, recorder) = storage_with_recorder().await;
    let repo = storage.tasks();
    let task = repo.create(card("a card")).await.expect("created");
    repo.complete(task.id, Some("gui"), None)
        .await
        .expect("completed");
    recorder.clear();

    let outcome = repo
        .complete(task.id, Some("gui"), None)
        .await
        .expect("idempotent");
    assert!(outcome.already_done);
    assert_eq!(
        recorder.kinds(),
        Vec::<TaskEventKind>::new(),
        "completion is idempotent, so its announcement must be too"
    );
}

#[tokio::test]
async fn a_store_with_no_sink_still_works() {
    // Every existing caller and every other test runs without a sink; the
    // events must be an addition, not a requirement.
    let storage = Storage::in_memory().await.expect("in-memory storage");
    let repo = storage.tasks();
    let task = repo.create(card("a card")).await.expect("created");
    repo.add_note(task.id, Some("harness"), "a note")
        .await
        .expect("noted");
    repo.complete(task.id, Some("gui"), None)
        .await
        .expect("completed");
}

#[tokio::test]
async fn a_second_sink_is_refused_rather_than_silently_replacing_the_first() {
    let (storage, recorder) = storage_with_recorder().await;
    let second = Arc::new(Recorder::default());
    assert!(
        !storage.set_task_events(second.clone()),
        "attaching twice is a wiring bug, and the first sink keeps receiving"
    );

    storage
        .tasks()
        .create(card("a card"))
        .await
        .expect("created");
    assert_eq!(recorder.kinds(), vec![TaskEventKind::Created]);
    assert_eq!(second.kinds(), Vec::<TaskEventKind>::new());
}
