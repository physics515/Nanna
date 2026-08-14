//! Terminal reason file: the daemon's own durable record of WHY it stopped.
//!
//! Motivation (2026-08-10 ministral bench leg): the daemon hard-died mid-turn
//! at 14:15:47Z and the only evidence was absence — the log simply ends, no
//! shutdown marker, no panic line, nothing on disk that says the process is
//! gone rather than thinking. The bench driver polled the corpse 14 times and
//! published a 0/42 score for a daemon that was dead for 96% of the window.
//!
//! Mechanism (a classic dirty bit):
//! - On startup, AFTER the PID file is acquired (so a losing duplicate can
//!   never clobber the live daemon's record), the daemon reads whatever the
//!   previous process left, logs it, and overwrites the file with a
//!   `state: running` marker.
//! - Every deliberate exit path (clean shutdown drain, panic hook, signal /
//!   ctrl handler, IPC-server hard exit) overwrites the marker with
//!   `state: exited` plus a reason. Last writer wins.
//! - A file still saying `running` for a process that no longer exists IS the
//!   unclean-exit verdict: the process died through a path no hook could see
//!   (`taskkill /F`, OOM kill, power loss, abort without unwinding).
//!
//! The writer is disarmed until the startup marker lands: `record_exit` and
//! the panic hook are no-ops in a process that never owned the file, so a
//! second instance that fails the PID race (or is Ctrl-C'd while losing it)
//! cannot overwrite the live daemon's record.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// File name under the daemon data directory.
pub const EXIT_REASON_FILE: &str = "nanna-daemon.exit.json";

/// Whether the recorded process believed itself alive or terminated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitState {
    /// Startup marker: the process was running when this was written.
    Running,
    /// A terminal reason was recorded on the way out.
    Exited,
}

/// One record, serialized as JSON. The whole file is a single record; every
/// write replaces it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExitReasonRecord {
    pub state: ExitState,
    /// PID of the process that wrote the record.
    pub pid: u32,
    /// Terminal reason (`clean_shutdown`, `panic`, `signal`, ...). None while running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Free-form detail: panic payload + location, signal name, error text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// RFC3339 timestamp of the write.
    pub at: String,
}

/// What the previous process left behind.
#[derive(Debug, Clone)]
pub enum PreviousExit {
    /// No file — first boot, or the file was deleted.
    Absent,
    /// The file exists but does not parse. A torn write during a hard death
    /// lands here, so this is treated as an unclean exit.
    Corrupt(String),
    /// A parseable record.
    Record(ExitReasonRecord),
}

impl PreviousExit {
    /// True when the previous process died without recording a terminal
    /// reason — the log-just-ends case this file exists to catch.
    pub fn is_unclean(&self) -> bool {
        match self {
            Self::Absent => false,
            Self::Corrupt(_) => true,
            Self::Record(r) => r.state == ExitState::Running,
        }
    }

    /// One line for the startup log describing the previous exit.
    pub fn describe(&self) -> String {
        match self {
            Self::Absent => "no previous exit record (first boot or record deleted)".to_string(),
            Self::Corrupt(err) => format!(
                "previous exit record is unreadable ({err}) — treating as an UNCLEAN exit \
                 (a torn write during a hard death lands here)"
            ),
            Self::Record(r) => match r.state {
                ExitState::Running => format!(
                    "previous daemon (PID {}) exited UNCLEANLY: it marked itself running at {} \
                     and never recorded a terminal reason — it died through a path no hook \
                     could see (hard kill, OOM, power loss)",
                    r.pid, r.at
                ),
                ExitState::Exited => format!(
                    "previous daemon (PID {}) exited at {}: {}{}",
                    r.pid,
                    r.at,
                    r.reason.as_deref().unwrap_or("<no reason recorded>"),
                    r.detail
                        .as_deref()
                        .map(|d| format!(" — {d}"))
                        .unwrap_or_default(),
                ),
            },
        }
    }
}

/// Handle to the reason file. Cheap to clone; clones share the armed flag, so
/// arming the handle held by `run()` also arms the clones captured by the
/// panic hook and the signal handlers.
#[derive(Clone)]
pub struct ExitReasonFile {
    path: PathBuf,
    armed: Arc<AtomicBool>,
}

impl ExitReasonFile {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join(EXIT_REASON_FILE),
            armed: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read whatever the previous process left. Never errors: absence and
    /// corruption are verdicts, not failures.
    pub fn read_previous(&self) -> PreviousExit {
        match std::fs::read_to_string(&self.path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => PreviousExit::Absent,
            Err(e) => PreviousExit::Corrupt(e.to_string()),
            Ok(text) => match serde_json::from_str::<ExitReasonRecord>(&text) {
                Ok(record) => PreviousExit::Record(record),
                Err(e) => PreviousExit::Corrupt(e.to_string()),
            },
        }
    }

    /// Write the `running` startup marker and arm the terminal writers.
    /// Call exactly once, after the PID file is acquired.
    pub fn mark_running(&self) {
        self.write(ExitReasonRecord {
            state: ExitState::Running,
            pid: std::process::id(),
            reason: None,
            detail: None,
            at: now_rfc3339(),
        });
        self.armed.store(true, Ordering::Release);
    }

    /// Record a terminal reason. No-op until `mark_running` has armed the
    /// handle (a process that never owned the file must not write it).
    /// Best-effort and panic-free: safe to call from a panic hook.
    pub fn record_exit(&self, reason: &str, detail: Option<&str>) {
        if !self.armed.load(Ordering::Acquire) {
            return;
        }
        self.write(ExitReasonRecord {
            state: ExitState::Exited,
            pid: std::process::id(),
            reason: Some(reason.to_string()),
            detail: detail.map(str::to_string),
            at: now_rfc3339(),
        });
    }

    /// Atomic-ish replace: write a sibling temp file, then rename over the
    /// destination (Windows rename refuses to clobber, so remove first). A
    /// death inside this window leaves either the old record or a temp file
    /// beside an old record — never a half-written destination. If the temp
    /// path itself is unwritable, fall back to a direct write: a torn record
    /// reads as Corrupt, which the reader already treats as unclean.
    fn write(&self, record: ExitReasonRecord) {
        let json = match serde_json::to_string_pretty(&record) {
            Ok(j) => j,
            Err(_) => return,
        };
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, &json).is_ok() {
            let _ = std::fs::remove_file(&self.path);
            if std::fs::rename(&tmp, &self.path).is_ok() {
                return;
            }
        }
        let _ = std::fs::write(&self.path, &json);
    }
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_in(dir: &tempfile::TempDir) -> ExitReasonFile {
        ExitReasonFile::new(dir.path())
    }

    #[test]
    fn absent_on_first_boot() {
        let dir = tempfile::tempdir().unwrap();
        let f = file_in(&dir);
        let prev = f.read_previous();
        assert!(matches!(prev, PreviousExit::Absent));
        assert!(!prev.is_unclean());
    }

    #[test]
    fn running_marker_round_trips_with_our_pid() {
        let dir = tempfile::tempdir().unwrap();
        let f = file_in(&dir);
        f.mark_running();
        match f.read_previous() {
            PreviousExit::Record(r) => {
                assert_eq!(r.state, ExitState::Running);
                assert_eq!(r.pid, std::process::id());
                assert!(r.reason.is_none());
            }
            other => panic!("expected record, got {other:?}"),
        }
    }

    #[test]
    fn clean_exit_supersedes_running_marker() {
        let dir = tempfile::tempdir().unwrap();
        let f = file_in(&dir);
        f.mark_running();
        f.record_exit("clean_shutdown", None);
        let prev = f.read_previous();
        assert!(!prev.is_unclean());
        match prev {
            PreviousExit::Record(r) => {
                assert_eq!(r.state, ExitState::Exited);
                assert_eq!(r.reason.as_deref(), Some("clean_shutdown"));
            }
            other => panic!("expected record, got {other:?}"),
        }
    }

    #[test]
    fn stale_running_record_is_the_unclean_verdict() {
        // Simulate the ministral death: a previous process marked itself
        // running and then vanished. The next boot must call it unclean.
        let dir = tempfile::tempdir().unwrap();
        file_in(&dir).mark_running();
        let next_boot = file_in(&dir);
        let prev = next_boot.read_previous();
        assert!(prev.is_unclean());
        assert!(prev.describe().contains("UNCLEAN"));
    }

    #[test]
    fn corrupt_file_reads_as_unclean() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(EXIT_REASON_FILE), "{ torn wri").unwrap();
        let prev = file_in(&dir).read_previous();
        assert!(matches!(prev, PreviousExit::Corrupt(_)));
        assert!(prev.is_unclean());
        assert!(prev.describe().contains("UNCLEAN"));
    }

    #[test]
    fn record_exit_is_a_noop_until_armed() {
        // A duplicate instance that loses the PID race (or is Ctrl-C'd while
        // losing it) must not clobber the live daemon's record.
        let dir = tempfile::tempdir().unwrap();
        let live = file_in(&dir);
        live.mark_running();
        let loser = file_in(&dir); // separate handle: never armed
        loser.record_exit("signal", Some("ctrl_c"));
        match live.read_previous() {
            PreviousExit::Record(r) => assert_eq!(r.state, ExitState::Running),
            other => panic!("expected the running marker to survive, got {other:?}"),
        }
    }

    #[test]
    fn clones_share_the_armed_flag() {
        let dir = tempfile::tempdir().unwrap();
        let f = file_in(&dir);
        let hook_clone = f.clone(); // captured by a panic hook before arming
        f.mark_running();
        hook_clone.record_exit("panic", Some("boom at src/x.rs:1:1"));
        match f.read_previous() {
            PreviousExit::Record(r) => {
                assert_eq!(r.reason.as_deref(), Some("panic"));
                assert_eq!(r.detail.as_deref(), Some("boom at src/x.rs:1:1"));
            }
            other => panic!("expected panic record, got {other:?}"),
        }
    }

    #[test]
    fn describe_names_the_reason_for_a_clean_exit() {
        let dir = tempfile::tempdir().unwrap();
        let f = file_in(&dir);
        f.mark_running();
        f.record_exit("signal", Some("SIGTERM"));
        let text = f.read_previous().describe();
        assert!(text.contains("signal"), "got: {text}");
        assert!(text.contains("SIGTERM"), "got: {text}");
    }
}
