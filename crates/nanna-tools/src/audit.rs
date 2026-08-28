//! Per-call audit trail for tool execution.
//!
//! Nanna's daemon executes tools **unattended** — scheduled runs, channel
//! messages, heartbeats — and until now the only record a call left behind was
//! a pair of `debug!`/`warn!` lines interleaved with everything else in the
//! rotating daemon log. That is not an audit trail: it is off at the default
//! level, it is unstructured, and the aggregate counters that do exist
//! (`nanna-agent::ToolStatsTracker`) are recorded from the agent loop — so
//! calls made outside it (chat harness, task tool, the `nanna mcp serve`
//! bridge, scripted skills) leave no trace at all.
//!
//! This module records **one structured line per call, at the one chokepoint
//! every caller funnels through** ([`crate::ToolRegistry::execute`]).
//!
//! # What is recorded, and what deliberately is not
//!
//! A tool call's *arguments* routinely carry secrets — an API key pasted into a
//! request, a token in a URL, the contents of a file being written. An audit
//! log is a durable, plaintext, long-lived artifact, so writing raw arguments
//! into it by default would create a secret sink that outlives the process that
//! made the call. The record therefore carries the **parameter key names**
//! (sorted, bounded) and never their values, unless the operator explicitly
//! opts in with [`ToolAuditConfig::include_values`].
//!
//! Names alone still answer the questions an audit is for: which tool ran, what
//! the caller typed and what it resolved to, whether policy refused it, how long
//! it took, and whether it failed.
//!
//! # Bounds
//!
//! Every field a caller can influence is bounded, so one hostile or runaway call
//! cannot produce an unbounded line — and the file itself is size-capped with a
//! single generation of rollover, so the trail cannot fill a disk.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

/// Maximum number of parameter key names recorded for one call.
///
/// A tool's parameter list is its function signature; real tools declare a
/// handful and the widest in this tree declares well under twenty. `64` is a
/// generous ceiling that still bounds the line when a caller sends a map of
/// junk keys — unknown keys are not rejected before execution, they are simply
/// ignored by the tool, so the map's size is caller-controlled.
pub const AUDIT_PARAM_KEYS_MAX: usize = 64;

/// Maximum bytes of a single recorded parameter key name.
///
/// Identifiers in every tool schema in this tree are under 32 bytes; `64` is
/// twice that and keeps one absurd key from dominating the line.
pub const AUDIT_KEY_BYTES_MAX: usize = 64;

/// Maximum bytes of the optional argument-value preview, and of the error
/// preview.
///
/// Long enough to carry a path, a URL, or the first clause of an error message —
/// the identifying part of either — and short enough that the whole record stays
/// comfortably inside one filesystem block.
pub const AUDIT_PREVIEW_BYTES_MAX: usize = 512;

/// Maximum bytes the audit file may reach before it rolls over.
///
/// At roughly 300 bytes per record this holds on the order of 30 000 calls —
/// months of a personal daemon's history. One generation is kept (`<file>` plus
/// `<file>.1`), so the trail costs at most twice this on disk.
pub const AUDIT_FILE_BYTES_MAX: u64 = 8 * 1024 * 1024;

/// How a tool call ended.
///
/// Every call produces exactly one of these, including the calls that never
/// reached a tool at all — an audit that recorded only executions would be blind
/// to precisely the events worth reviewing: a refused call, or a model reaching
/// for a tool that does not exist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum ToolAuditOutcome {
    /// No tool matched the requested name, even after alias and fuzzy resolution.
    NotFound,
    /// The active [`crate::ToolPolicy`] refused the resolved name.
    Refused {
        /// The policy's reason, as surfaced to the model.
        reason: String,
    },
    /// The tool ran and reported success.
    Succeeded,
    /// The tool ran and reported failure, or returned an error.
    Failed {
        /// Bounded preview of the failure text.
        error: String,
    },
}

impl ToolAuditOutcome {
    /// Stable short label, for grepping a trail without a JSON parser.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Refused { .. } => "refused",
            Self::Succeeded => "succeeded",
            Self::Failed { .. } => "failed",
        }
    }

    /// True when the call never reached a tool body.
    ///
    /// Both short-circuit classes are the security-interesting ones, so they get
    /// a predicate rather than two match arms at every call site.
    #[must_use]
    pub const fn short_circuited(&self) -> bool {
        matches!(self, Self::NotFound | Self::Refused { .. })
    }
}

/// One audited tool call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolAuditRecord {
    /// Wall-clock milliseconds since the Unix epoch.
    pub ts_unix_ms: u128,
    /// The call id the caller assigned, so a record joins back to a transcript.
    pub call_id: String,
    /// The name the caller (usually a model) actually typed.
    pub requested: String,
    /// The registry key it resolved to. `None` when nothing matched.
    pub resolved: Option<String>,
    /// The session the run belonged to, when one was bound.
    pub session_id: Option<String>,
    /// Sorted, bounded parameter key names. Values are excluded by default.
    pub param_keys: Vec<String>,
    /// Bounded preview of the serialized arguments. Present only when the
    /// operator set [`ToolAuditConfig::include_values`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params_preview: Option<String>,
    /// Wall-clock duration of the call, including resolution and policy.
    pub duration_ms: u64,
    /// How it ended.
    #[serde(flatten)]
    pub outcome: ToolAuditOutcome,
}

/// What the audit records, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolAuditConfig {
    /// Include a bounded preview of argument *values*.
    ///
    /// Off by default: arguments carry secrets, and the trail outlives the run.
    pub include_values: bool,
    /// Byte ceiling before the file rolls over to `<file>.1`.
    pub file_bytes_max: u64,
}

impl Default for ToolAuditConfig {
    fn default() -> Self {
        Self {
            include_values: false,
            file_bytes_max: AUDIT_FILE_BYTES_MAX,
        }
    }
}

/// A destination for audit records.
///
/// Deliberately synchronous and infallible from the caller's point of view: a
/// sink that could fail the call it is auditing would turn observability into an
/// outage, so a sink reports its own trouble and the tool call proceeds
/// regardless.
pub trait ToolAuditSink: Send + Sync {
    /// Record one call. Must not panic and must not block for long.
    fn record(&self, record: &ToolAuditRecord);

    /// Whether this sink wants [`ToolAuditRecord::params_preview`] filled in.
    ///
    /// The decision belongs to the sink, not the registry: only the sink knows
    /// where the trail lands and therefore what it is safe to keep. It defaults
    /// to `false` so a sink written later inherits the value-free posture rather
    /// than silently opting its operator into a secret sink.
    fn includes_values(&self) -> bool {
        false
    }
}

/// A shared sink handle, as the registry stores it.
pub type SharedAuditSink = Arc<dyn ToolAuditSink>;

/// Truncate `s` to at most `max_bytes`, never splitting a UTF-8 character.
///
/// Returns a borrowed slice, so the common (short) case allocates nothing.
#[must_use]
pub fn bounded(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    debug_assert!(end <= max_bytes, "truncation may only shrink");
    debug_assert!(
        s.is_char_boundary(end),
        "truncation must land on a boundary"
    );
    &s[..end]
}

/// Sorted, deduplicated, bounded parameter key names.
///
/// Sorting makes two records of the same call byte-identical in this field
/// regardless of map iteration order, which is what lets an operator diff a
/// trail. Both the count and each name are bounded because both are
/// caller-supplied.
#[must_use]
pub fn param_key_names(parameters: &HashMap<String, Value>) -> Vec<String> {
    let mut keys: Vec<String> = parameters
        .keys()
        .map(|k| bounded(k, AUDIT_KEY_BYTES_MAX).to_string())
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys.truncate(AUDIT_PARAM_KEYS_MAX);
    debug_assert!(
        keys.len() <= AUDIT_PARAM_KEYS_MAX,
        "recorded key count must stay within its ceiling"
    );
    debug_assert!(
        keys.windows(2).all(|w| w[0] < w[1]),
        "keys must be sorted and deduplicated"
    );
    keys
}

/// Bounded preview of the serialized arguments, for the opt-in value mode.
///
/// Serialization failure yields `None` rather than an error: a preview is a
/// convenience, and losing it must never cost the audit record itself.
#[must_use]
pub fn params_preview(parameters: &HashMap<String, Value>) -> Option<String> {
    let json = serde_json::to_string(parameters).ok()?;
    Some(bounded(&json, AUDIT_PREVIEW_BYTES_MAX).to_string())
}

/// Milliseconds since the Unix epoch, or `0` if the clock predates it.
///
/// A clock behind the epoch is a misconfigured machine, not a reason to lose the
/// record — so this saturates instead of failing.
#[must_use]
pub fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

/// A [`ToolAuditSink`] that appends JSON Lines to a file, rolling over at a byte
/// ceiling.
///
/// One generation is kept: at rollover `<path>` becomes `<path>.1` (replacing any
/// previous `.1`) and a fresh `<path>` starts. That bounds the trail at
/// `2 x file_bytes_max` without a background reaper.
pub struct JsonlAuditSink {
    path: PathBuf,
    config: ToolAuditConfig,
    /// Serializes the size-check / rollover / append sequence. Without it two
    /// concurrent calls could both observe an under-limit size and both append
    /// past it, or interleave a rename with a write.
    gate: std::sync::Mutex<()>,
}

impl JsonlAuditSink {
    /// Create a sink writing to `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, config: ToolAuditConfig) -> Self {
        Self {
            path: path.into(),
            config,
            gate: std::sync::Mutex::new(()),
        }
    }

    /// The file this sink appends to.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The configuration in force.
    #[must_use]
    pub const fn config(&self) -> &ToolAuditConfig {
        &self.config
    }

    /// Roll `<path>` aside to `<path>.1` when it has reached the ceiling.
    ///
    /// A missing file is not an error — it is the first call.
    fn roll_if_full(&self) {
        let Ok(meta) = std::fs::metadata(&self.path) else {
            return;
        };
        if meta.len() < self.config.file_bytes_max {
            return;
        }
        let mut rolled = self.path.clone().into_os_string();
        rolled.push(".1");
        if let Err(e) = std::fs::rename(&self.path, PathBuf::from(rolled)) {
            warn!(path = %self.path.display(), error = %e, "Could not roll the tool audit log");
        }
    }
}

impl ToolAuditSink for JsonlAuditSink {
    fn includes_values(&self) -> bool {
        self.config.include_values
    }

    fn record(&self, record: &ToolAuditRecord) {
        let Ok(mut line) = serde_json::to_string(record) else {
            warn!(call_id = %record.call_id, "Could not serialize a tool audit record");
            return;
        };
        line.push('\n');

        // A poisoned gate still describes a valid file, so recovering the guard
        // keeps auditing alive after an unrelated panic elsewhere.
        let _guard = self
            .gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            warn!(
                path = %parent.display(),
                error = %e,
                "Could not create the tool audit directory"
            );
            return;
        }

        self.roll_if_full();

        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(mut file) => {
                if let Err(e) = file.write_all(line.as_bytes()) {
                    warn!(
                        path = %self.path.display(),
                        error = %e,
                        "Could not append a tool audit record"
                    );
                }
            }
            Err(e) => {
                warn!(
                    path = %self.path.display(),
                    error = %e,
                    "Could not open the tool audit log"
                );
            }
        }
    }
}

/// A [`ToolAuditSink`] that emits one `INFO` tracing event per call.
///
/// For operators who already ship the daemon log somewhere and do not want a
/// second file. The fields are the record's, so the event is machine-readable
/// under a JSON tracing subscriber.
pub struct TracingAuditSink;

impl ToolAuditSink for TracingAuditSink {
    fn record(&self, record: &ToolAuditRecord) {
        tracing::info!(
            target: "nanna::tool_audit",
            ts_unix_ms = %record.ts_unix_ms,
            call_id = %record.call_id,
            requested = %record.requested,
            resolved = record.resolved.as_deref().unwrap_or("-"),
            session_id = record.session_id.as_deref().unwrap_or("-"),
            param_keys = %record.param_keys.join(","),
            duration_ms = record.duration_ms,
            outcome = record.outcome.label(),
            "tool call"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn rec(outcome: ToolAuditOutcome) -> ToolAuditRecord {
        ToolAuditRecord {
            ts_unix_ms: 1_700_000_000_000,
            call_id: "call-1".into(),
            requested: "Bash".into(),
            resolved: Some("exec".into()),
            session_id: Some("s1".into()),
            param_keys: vec!["command".into()],
            params_preview: None,
            duration_ms: 12,
            outcome,
        }
    }

    /// A sink that only remembers, for the registry-side tests.
    #[derive(Default)]
    pub struct CountingSink {
        pub records: Mutex<Vec<ToolAuditRecord>>,
    }

    impl ToolAuditSink for CountingSink {
        fn record(&self, record: &ToolAuditRecord) {
            self.records
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(record.clone());
        }
    }

    #[test]
    fn bounded_never_splits_a_character() {
        // "é" is two bytes; cutting at 1 must fall back to 0, not panic.
        assert_eq!(bounded("é", 1), "");
        assert_eq!(bounded("aé", 2), "a");
        assert_eq!(bounded("abc", 10), "abc");
    }

    #[test]
    fn param_keys_are_sorted_deduped_and_bounded() {
        let mut params = HashMap::new();
        for i in 0..(AUDIT_PARAM_KEYS_MAX * 2) {
            params.insert(format!("k{i:04}"), Value::Null);
        }
        let keys = param_key_names(&params);
        assert_eq!(keys.len(), AUDIT_PARAM_KEYS_MAX);
        assert!(keys.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn long_param_names_are_clipped_to_the_key_ceiling() {
        let mut params = HashMap::new();
        params.insert("x".repeat(AUDIT_KEY_BYTES_MAX * 3), Value::Null);
        let keys = param_key_names(&params);
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].len(), AUDIT_KEY_BYTES_MAX);
    }

    #[test]
    fn values_are_absent_from_the_record_by_default() {
        let line = serde_json::to_string(&rec(ToolAuditOutcome::Succeeded)).unwrap();
        assert!(
            !line.contains("params_preview"),
            "a value-free record must not carry the key at all: {line}"
        );
    }

    #[test]
    fn params_preview_is_bounded() {
        let mut params = HashMap::new();
        params.insert("big".to_string(), Value::String("z".repeat(10_000)));
        let preview = params_preview(&params).expect("a map of strings always serializes");
        assert!(preview.len() <= AUDIT_PREVIEW_BYTES_MAX);
    }

    #[test]
    fn outcomes_round_trip_and_label_stably() {
        for outcome in [
            ToolAuditOutcome::NotFound,
            ToolAuditOutcome::Refused {
                reason: "blocked by tool policy".into(),
            },
            ToolAuditOutcome::Succeeded,
            ToolAuditOutcome::Failed {
                error: "boom".into(),
            },
        ] {
            let label = outcome.label();
            let json = serde_json::to_string(&rec(outcome.clone())).unwrap();
            let back: ToolAuditRecord = serde_json::from_str(&json).unwrap();
            assert_eq!(back.outcome, outcome);
            assert_eq!(back.outcome.label(), label);
        }
    }

    #[test]
    fn short_circuit_predicate_covers_exactly_the_pre_execution_exits() {
        assert!(ToolAuditOutcome::NotFound.short_circuited());
        assert!(
            ToolAuditOutcome::Refused {
                reason: String::new()
            }
            .short_circuited()
        );
        assert!(!ToolAuditOutcome::Succeeded.short_circuited());
        assert!(
            !ToolAuditOutcome::Failed {
                error: String::new()
            }
            .short_circuited()
        );
    }

    #[test]
    fn jsonl_sink_appends_one_line_per_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("tool-audit.jsonl");
        let sink = JsonlAuditSink::new(&path, ToolAuditConfig::default());
        sink.record(&rec(ToolAuditOutcome::Succeeded));
        sink.record(&rec(ToolAuditOutcome::NotFound));

        let body = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 2, "one line per record: {body}");
        for line in lines {
            serde_json::from_str::<ToolAuditRecord>(line).expect("each line parses on its own");
        }
    }

    #[test]
    fn jsonl_sink_rolls_over_at_the_ceiling_and_keeps_one_generation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tool-audit.jsonl");
        let sink = JsonlAuditSink::new(
            &path,
            ToolAuditConfig {
                include_values: false,
                // One record comfortably exceeds this, so the second write rolls.
                file_bytes_max: 16,
            },
        );
        sink.record(&rec(ToolAuditOutcome::Succeeded));
        sink.record(&rec(ToolAuditOutcome::Failed {
            error: "second".into(),
        }));

        let mut rolled_path = path.clone().into_os_string();
        rolled_path.push(".1");
        let rolled = std::fs::read_to_string(PathBuf::from(rolled_path)).unwrap();
        let live = std::fs::read_to_string(&path).unwrap();
        assert_eq!(rolled.lines().count(), 1, "the first record rolled aside");
        assert_eq!(live.lines().count(), 1, "the live file restarted");
        assert!(
            live.contains("second"),
            "the live file holds the newer record"
        );
    }

    #[test]
    fn now_is_after_the_epoch() {
        assert!(now_unix_ms() > 1_600_000_000_000, "clock must be sane");
    }

    #[test]
    fn a_sink_sees_exactly_what_it_was_handed() {
        let sink = CountingSink::default();
        sink.record(&rec(ToolAuditOutcome::Succeeded));
        let seen = sink.records.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].requested, "Bash");
        assert_eq!(seen[0].resolved.as_deref(), Some("exec"));
    }
}
