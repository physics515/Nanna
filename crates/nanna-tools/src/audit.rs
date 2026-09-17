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

    /// Roll `<path>` aside to the previous generation when it has reached the
    /// ceiling.
    ///
    /// A missing file is not an error — it is the first call.
    fn roll_if_full(&self) {
        let Ok(meta) = std::fs::metadata(&self.path) else {
            return;
        };
        if meta.len() < self.config.file_bytes_max {
            return;
        }
        if let Err(e) = std::fs::rename(&self.path, rolled_path(&self.path)) {
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

// =============================================================================
// Reading the trail back
// =============================================================================

/// The path the previous generation rolls aside to.
///
/// One definition, used by the sink that creates it *and* the reader that looks
/// for it. Keeping the reader in this module is the point: a rollover naming
/// scheme that lived in two places would drift, and the failure would be silent
/// — a viewer that simply never finds the older half of the history.
#[must_use]
pub fn rolled_path(path: &Path) -> PathBuf {
    let mut rolled = path.to_path_buf().into_os_string();
    rolled.push(".1");
    PathBuf::from(rolled)
}

/// Largest page [`read_recent`] will return.
///
/// At roughly 300 bytes per record this is ~300 KB on the wire: enough for a
/// viewer to scroll a working session, and small enough that one request cannot
/// turn the whole 8 MB trail into a single control-plane response.
pub const AUDIT_PAGE_MAX: usize = 1000;

/// Page size when a caller does not ask for one.
pub const AUDIT_PAGE_DEFAULT: usize = 200;

/// One page of the trail, plus an honest account of what producing it cost.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditPage {
    /// Records, **newest first**.
    pub records: Vec<ToolAuditRecord>,
    /// Lines that did not parse and were skipped.
    ///
    /// Reported rather than swallowed. A trail is evidence, and a reader that
    /// silently drops what it cannot read presents a partial history as a
    /// complete one — the exact dishonesty the audit exists to prevent. A
    /// non-zero count here means the file was truncated mid-write, hand-edited,
    /// or written by a different version.
    pub unparseable: usize,
    /// Total non-empty lines examined across every generation read.
    pub scanned: usize,
    /// How many files were read: `1` for the live trail alone, `2` when the
    /// rolled generation was needed to fill the page.
    pub generations_read: usize,
    /// True when this page reaches the oldest record that still exists — the
    /// whole retained history, not a window into more.
    ///
    /// Without this a viewer cannot tell "that is everything Nanna has done"
    /// from "that is the most recent screenful", and those are very different
    /// answers to the question an audit gets asked.
    pub reached_oldest: bool,
}

/// Parse one generation's text into records in file order (oldest first),
/// accumulating the scan counters into `page`.
fn parse_generation(body: &str, page: &mut AuditPage) -> Vec<ToolAuditRecord> {
    let mut out = Vec::new();
    for line in body.lines() {
        if line.trim().is_empty() {
            continue;
        }
        page.scanned += 1;
        match serde_json::from_str::<ToolAuditRecord>(line) {
            Ok(record) => out.push(record),
            Err(_) => page.unparseable += 1,
        }
    }
    out
}

/// Read the most recent records from an audit trail, newest first.
///
/// Reads the live file and — only when that alone cannot fill the page — the
/// rolled generation as well. That fallback is not a nicety: the sink rolls at
/// a byte ceiling, so a viewer opened just after a rollover would otherwise show
/// a handful of records and present them as the entire history.
///
/// A missing file is an empty page, not an error: a daemon that has not yet run
/// a tool has nothing to show, and that is a legitimate answer.
///
/// Bounded without needing its own byte cap: [`JsonlAuditSink`] holds each
/// generation under [`ToolAuditConfig::file_bytes_max`], so the most this can
/// read is two generations of an already-capped file.
#[must_use]
pub fn read_recent(path: &Path, limit: usize) -> AuditPage {
    let limit = limit.clamp(1, AUDIT_PAGE_MAX);
    let mut page = AuditPage::default();

    let mut chronological = match std::fs::read_to_string(path) {
        Ok(body) => {
            page.generations_read += 1;
            parse_generation(&body, &mut page)
        }
        Err(_) => Vec::new(),
    };

    // Only pay for the rolled generation when the live one cannot fill the
    // page. On a long-running trail it never can't, so the common case reads
    // one file.
    let rolled = rolled_path(path);
    let mut rolled_read = false;
    if chronological.len() < limit
        && let Ok(body) = std::fs::read_to_string(&rolled)
    {
        rolled_read = true;
        page.generations_read += 1;
        // The rolled generation is strictly older, so it goes in front.
        let mut older = parse_generation(&body, &mut page);
        older.append(&mut chronological);
        chronological = older;
    }

    // "Oldest" means nothing older is retained anywhere: this page holds every
    // record we found, and there is no unread generation hiding more.
    let unread_generation = !rolled_read && rolled.exists();
    page.reached_oldest = !unread_generation && chronological.len() <= limit;

    if chronological.len() > limit {
        chronological.drain(..chronological.len() - limit);
    }
    chronological.reverse();
    page.records = chronological;

    debug_assert!(
        page.records.len() <= limit,
        "a page may never exceed the limit it was asked for"
    );
    debug_assert!(
        page.records.len() + page.unparseable <= page.scanned,
        "every returned record and every skipped line must have been scanned"
    );
    page
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

    // -------------------------------------------------------------------------
    // Reading the trail back
    // -------------------------------------------------------------------------

    /// A record tagged with `id`, so ordering assertions name a specific call
    /// rather than counting anonymous rows.
    fn rec_id(id: &str) -> ToolAuditRecord {
        ToolAuditRecord {
            call_id: id.to_string(),
            ..rec(ToolAuditOutcome::Succeeded)
        }
    }

    fn ids(page: &AuditPage) -> Vec<&str> {
        page.records.iter().map(|r| r.call_id.as_str()).collect()
    }

    #[test]
    fn a_page_is_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tool-audit.jsonl");
        let sink = JsonlAuditSink::new(&path, ToolAuditConfig::default());
        for id in ["a", "b", "c"] {
            sink.record(&rec_id(id));
        }

        let page = read_recent(&path, 10);
        assert_eq!(
            ids(&page),
            vec!["c", "b", "a"],
            "the most recent call must be the first thing a reviewer sees"
        );
        assert_eq!(page.scanned, 3);
        assert_eq!(page.unparseable, 0);
        assert!(
            page.reached_oldest,
            "three records under a limit of ten is the whole history"
        );
    }

    #[test]
    fn a_malformed_line_is_skipped_and_counted_not_swallowed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tool-audit.jsonl");
        let good = serde_json::to_string(&rec_id("good")).unwrap();
        // A line torn by a crash mid-write, between two intact records.
        std::fs::write(&path, format!("{good}\n{{\"requested\":\"tru\n{good}\n")).unwrap();

        let page = read_recent(&path, 10);
        assert_eq!(
            page.records.len(),
            2,
            "one bad line must not cost the records around it"
        );
        assert_eq!(
            page.unparseable, 1,
            "and it must be reported, or a partial history reads as a complete one"
        );
        assert_eq!(page.scanned, 3);
    }

    #[test]
    fn a_missing_trail_reads_as_an_empty_page_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let page = read_recent(&dir.path().join("never-written.jsonl"), 10);
        assert_eq!(page.records, Vec::<ToolAuditRecord>::new());
        assert_eq!(page.generations_read, 0);
        assert_eq!(page.scanned, 0);
        assert!(
            page.reached_oldest,
            "a daemon that has run no tools has shown its entire history"
        );
    }

    /// The property the rolled-generation fallback exists for: the sink rolls at
    /// a byte ceiling, so a viewer opened just afterwards would otherwise see
    /// only the handful of records written since — and present them as
    /// everything that ever happened.
    #[test]
    fn a_page_reaches_into_the_rolled_generation_when_the_live_file_is_short() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tool-audit.jsonl");
        let sink = JsonlAuditSink::new(
            &path,
            ToolAuditConfig {
                include_values: false,
                // One record exceeds this, so every write after the first rolls.
                file_bytes_max: 16,
            },
        );
        sink.record(&rec_id("older"));
        sink.record(&rec_id("newer"));

        let live = std::fs::read_to_string(&path).unwrap();
        assert_eq!(live.lines().count(), 1, "the live file really did roll");

        let page = read_recent(&path, 10);
        assert_eq!(
            ids(&page),
            vec!["newer", "older"],
            "history spans the rollover, newest first"
        );
        assert_eq!(page.generations_read, 2);
        assert!(page.reached_oldest);
    }

    #[test]
    fn a_full_page_skips_the_rolled_generation_and_says_it_is_not_the_whole_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tool-audit.jsonl");
        std::fs::write(
            rolled_path(&path),
            format!("{}\n", serde_json::to_string(&rec_id("ancient")).unwrap()),
        )
        .unwrap();
        let sink = JsonlAuditSink::new(&path, ToolAuditConfig::default());
        for id in ["x", "y"] {
            sink.record(&rec_id(id));
        }

        let page = read_recent(&path, 2);
        assert_eq!(ids(&page), vec!["y", "x"]);
        assert_eq!(
            page.generations_read, 1,
            "a page the live file can fill must not pay to read the older one"
        );
        assert!(
            !page.reached_oldest,
            "older records exist unread, and the viewer must not claim otherwise"
        );
    }

    #[test]
    fn a_page_is_clamped_to_its_ceiling() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tool-audit.jsonl");
        let sink = JsonlAuditSink::new(&path, ToolAuditConfig::default());
        for i in 0..5 {
            sink.record(&rec_id(&format!("r{i}")));
        }

        let page = read_recent(&path, 2);
        assert_eq!(
            ids(&page),
            vec!["r4", "r3"],
            "a bounded page keeps the newest, not the first written"
        );
        assert!(!page.reached_oldest);

        // A caller asking for more than the ceiling gets the ceiling, not a
        // refusal and not an unbounded response.
        assert!(read_recent(&path, usize::MAX).records.len() <= AUDIT_PAGE_MAX);
    }

    /// The reason the reader lives in this module: the sink names the rolled
    /// generation and the reader looks for it, and if those two ever disagree
    /// the older half of the history simply stops existing — silently. This
    /// drives the real sink's rollover and then asks the real reader to find it.
    #[test]
    fn the_reader_finds_what_the_sink_actually_rolled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tool-audit.jsonl");
        let sink = JsonlAuditSink::new(
            &path,
            ToolAuditConfig {
                include_values: false,
                file_bytes_max: 16,
            },
        );
        sink.record(&rec_id("rolled-away"));
        sink.record(&rec_id("still-live"));

        assert!(
            rolled_path(&path).exists(),
            "the sink must have produced the generation the reader looks for"
        );
        let page = read_recent(&path, 10);
        assert!(
            ids(&page).contains(&"rolled-away"),
            "a record the sink rolled aside must still be readable: {:?}",
            ids(&page)
        );
    }
}
