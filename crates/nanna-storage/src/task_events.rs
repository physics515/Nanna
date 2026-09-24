//! Task lifecycle events (P25 Stage 1).
//!
//! These are **store mutations**, not run progress. The `TaskRun*` family on
//! the daemon's event bus describes a harness run; these describe what
//! happened to a card, and the board's router is the consumer they exist for.
//!
//! # Why this lives in `nanna-storage`
//!
//! [`TaskRepository`](crate::TaskRepository) is the only place that sees every
//! writer. Above it there are five independent ingress families — the `tasks.*`
//! tool services, the control plane's IPC handlers, the harness's `TaskSource`,
//! the seeding and recurrence sweeps, and the GUI — and two of them have no
//! event bus at all. Worse, some mutations have no caller to instrument:
//! `cascade_cancel` closes a whole subtree and ancestor auto-completion closes
//! parents, both touching rows nobody named. Emitting from the layer above the
//! repository would therefore be emitting from *one* of five doors while
//! claiming to describe the store.
//!
//! `nanna-storage` does not know the daemon's wire protocol, so it emits its
//! own shape through [`TaskEventSink`] and the daemon adapts it to
//! `protocol::Event`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What happened to a task in the store.
///
/// **Only kinds that are actually emitted exist here.** P25 lists nine; the
/// four missing ones need machinery that does not exist yet — `blocked` and
/// `unblocked` are *derived* transitions (nothing walks reverse dependencies
/// today) and `due`/`overdue` are time-driven (they need a sweep). Declaring
/// them now would ship four variants that nothing ever sends, which is a dead
/// field wearing a feature's clothes: a consumer would match on them and wait
/// forever. They arrive with their emit points, not before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskEventKind {
    /// A card was created.
    Created,
    /// The assignee changed to a different member (never a no-op re-assign).
    Assigned,
    /// The status moved between two different values.
    StatusChanged,
    /// A thread post was appended.
    Posted,
    /// A card was completed — the recorded verdict.
    Verdict,
}

impl TaskEventKind {
    /// The wire name, matching `#[serde(rename_all = "snake_case")]` on the
    /// daemon's mirror of this enum.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Assigned => "assigned",
            Self::StatusChanged => "status_changed",
            Self::Posted => "posted",
            Self::Verdict => "verdict",
        }
    }
}

/// A single task store mutation.
///
/// `detail` carries only what the kind needs — never the whole task. A
/// consumer that wants the card reads it; the event says what changed, and a
/// full row on every mutation would make the bus a replication channel.
#[derive(Debug, Clone)]
pub struct TaskEvent {
    pub kind: TaskEventKind,
    pub task_id: i64,
    pub scope: String,
    pub scope_id: Option<String>,
    /// Who caused it, as the activity log records actors. `None` when the
    /// mutation had no named actor (a cascade, an internal sweep).
    pub actor: Option<String>,
    pub detail: Value,
}

/// Where task store mutations are published.
///
/// # Contract
///
/// `publish` **must not block and must not panic.** It is called from the
/// repository on the write path, and a sink that waits turns every task write
/// in the process into a queue behind whatever it is waiting for. The daemon's
/// implementation is a `broadcast::Sender::send`, which is non-blocking and
/// drops on lag rather than applying back-pressure — the event-bus rule that a
/// slow consumer must never stall a producer.
///
/// Implementations are also called while other tasks may be mid-write, so they
/// must be `Send + Sync`.
pub trait TaskEventSink: Send + Sync {
    /// Publish one event. Non-blocking; see the trait contract.
    fn publish(&self, event: TaskEvent);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every kind, so a new variant has to be added here too.
    const ALL: [TaskEventKind; 5] = [
        TaskEventKind::Created,
        TaskEventKind::Assigned,
        TaskEventKind::StatusChanged,
        TaskEventKind::Posted,
        TaskEventKind::Verdict,
    ];

    #[test]
    fn serde_name_matches_as_str() {
        // The daemon puts this enum straight on the wire, so serde's
        // snake_case renaming IS the wire name. `as_str` exists for logs and
        // for callers that want it without serializing; if the two disagree,
        // a consumer matching on one silently stops seeing the other.
        for kind in ALL {
            let wire = serde_json::to_value(kind).expect("kind serializes");
            assert_eq!(wire, serde_json::Value::String(kind.as_str().to_string()));
        }
    }

    #[test]
    fn kind_wire_names_are_snake_case() {
        assert_eq!(TaskEventKind::Created.as_str(), "created");
        assert_eq!(TaskEventKind::Assigned.as_str(), "assigned");
        assert_eq!(TaskEventKind::StatusChanged.as_str(), "status_changed");
        assert_eq!(TaskEventKind::Posted.as_str(), "posted");
        assert_eq!(TaskEventKind::Verdict.as_str(), "verdict");
    }

    #[test]
    fn kind_wire_names_are_distinct() {
        let names: Vec<&str> = ALL.iter().map(|k| k.as_str()).collect();
        let unique: std::collections::HashSet<&str> = names.iter().copied().collect();
        assert_eq!(unique.len(), names.len(), "two kinds share a wire name");
    }
}
