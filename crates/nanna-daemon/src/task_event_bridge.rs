//! Carries task store mutations from `nanna-storage` onto the daemon's event bus.
//!
//! `nanna-storage` emits [`nanna_storage::TaskEvent`] because the repository is
//! the only layer that sees every writer (see that module for why). It does not
//! know the daemon's wire protocol, so this is the adapter: one struct holding
//! the broadcast sender, translating each store event into
//! [`Event::TaskEvent`].

use crate::protocol::Event;
use nanna_storage::{TaskEvent as StoreTaskEvent, TaskEventSink};
use tokio::sync::broadcast;

/// Publishes storage task events onto the daemon event bus.
pub struct TaskEventBridge {
    events: broadcast::Sender<Event>,
}

impl TaskEventBridge {
    #[must_use]
    pub const fn new(events: broadcast::Sender<Event>) -> Self {
        Self { events }
    }
}

impl TaskEventSink for TaskEventBridge {
    /// Forward one store mutation to the bus.
    ///
    /// `send` on a `broadcast::Sender` is non-blocking and returns `Err` when
    /// there is no subscriber — which is the normal state of a daemon nobody
    /// has attached a client to, not a failure. The result is deliberately
    /// discarded: the sink contract forbids blocking, and a task write must
    /// never fail because nothing was listening to its announcement.
    fn publish(&self, event: StoreTaskEvent) {
        let _ = self.events.send(Event::TaskEvent {
            kind: event.kind,
            task_id: event.task_id,
            scope: event.scope,
            scope_id: event.scope_id,
            actor: event.actor,
            detail: event.detail,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nanna_storage::TaskEventKind;

    fn store_event(kind: TaskEventKind) -> StoreTaskEvent {
        StoreTaskEvent {
            kind,
            task_id: 7,
            scope: "workspace".to_string(),
            scope_id: Some("ws1".to_string()),
            actor: Some("gui".to_string()),
            detail: serde_json::json!({"title": "a card"}),
        }
    }

    #[test]
    fn publishes_every_field_onto_the_bus() {
        let (tx, mut rx) = broadcast::channel(4);
        let bridge = TaskEventBridge::new(tx);
        bridge.publish(store_event(TaskEventKind::Created));

        let event = rx.try_recv().expect("event reached the bus");
        let Event::TaskEvent {
            kind,
            task_id,
            scope,
            scope_id,
            actor,
            detail,
        } = event
        else {
            panic!("wrong variant: {event:?}");
        };
        assert_eq!(kind, TaskEventKind::Created);
        assert_eq!(task_id, 7);
        assert_eq!(scope, "workspace");
        assert_eq!(scope_id.as_deref(), Some("ws1"));
        assert_eq!(actor.as_deref(), Some("gui"));
        assert_eq!(detail, serde_json::json!({"title": "a card"}));
    }

    #[test]
    fn publish_with_no_subscriber_is_not_an_error() {
        // A daemon with no attached client is the ordinary case; a task write
        // must not fail because nobody was listening.
        let (tx, rx) = broadcast::channel(4);
        drop(rx);
        let bridge = TaskEventBridge::new(tx);
        bridge.publish(store_event(TaskEventKind::Verdict));
    }

    #[test]
    fn task_events_belong_to_no_session() {
        // P25 decision 9: a card outlives every session, so a per-session
        // subscriber must not receive board events as if they were its own.
        let event = Event::TaskEvent {
            kind: TaskEventKind::Posted,
            task_id: 1,
            scope: "workspace".to_string(),
            scope_id: None,
            actor: None,
            detail: serde_json::Value::Null,
        };
        assert_eq!(event.session_id(), None);
    }
}
