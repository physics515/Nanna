//! In-memory circular log buffer for recent process logs
//!
//! Captures log events via a tracing Layer and makes them available to clients
//! (the GUI's Logs page, the daemon's `system logs` action). Uses
//! `std::sync::Mutex` (not tokio) so it works before the Tokio runtime starts.
//!
//! This lives in `nanna-core` (rather than `nanna-daemon`) so a pure daemon
//! client such as the GUI can capture its own lines without linking the whole
//! daemon crate.

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

/// Which process produced a log line.
///
/// The GUI can show both at once: when it is attached to a daemon it still emits
/// its own in-process lines, so a merged view is ambiguous without this tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogSource {
    /// The in-process backend running inside the GUI (no daemon attached).
    Embedded,
    /// The standalone background daemon.
    Daemon,
}

impl LogSource {
    /// Stable identifier used on the wire and in the UI.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Embedded => "embedded",
            Self::Daemon => "daemon",
        }
    }
}

/// Wire compatibility: entries from a daemon predating the `source` field arrive
/// untagged, and they can only ever have come from a daemon.
impl Default for LogSource {
    fn default() -> Self {
        Self::Daemon
    }
}

/// A single log entry
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub target: String,
    pub message: String,
    /// Which process emitted this line. `#[serde(default)]` keeps a newer GUI
    /// readable against an older daemon that does not send the field.
    #[serde(default)]
    pub source: LogSource,
    /// The Nanna span chain the line was logged inside, outermost first —
    /// `chat_turn{session_id=… message_id=…}:harness_step{…}:tool_call{tool=exec …}`.
    /// Empty for a line logged outside any span. It is what lets a reader
    /// tell two overlapping conversations' lines apart, and filter one out.
    /// Omitted from the wire when empty; `#[serde(default)]` keeps entries
    /// from an older daemon readable.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scope: String,
}

/// Circular buffer for recent logs (last N entries)
#[derive(Clone)]
pub struct LogBuffer {
    entries: Arc<Mutex<Vec<LogEntry>>>,
    max_entries: usize,
    source: LogSource,
}

impl LogBuffer {
    /// Create a new log buffer with max capacity, tagging every captured entry
    /// with `source`.
    ///
    /// The source lives on the buffer rather than the individual push so a buffer
    /// cannot accidentally hold a mix of origins — one process, one buffer, one tag.
    ///
    /// # Panics
    ///
    /// If `max_entries` is zero. Capacity is a compile-time constant at every call
    /// site, so a zero here is a programmer error, not an operational one — and a
    /// zero-capacity buffer would silently discard every line it was asked to keep.
    #[must_use]
    pub fn new(max_entries: usize, source: LogSource) -> Self {
        assert!(max_entries > 0, "log buffer capacity must be non-zero");
        Self {
            entries: Arc::new(Mutex::new(Vec::with_capacity(max_entries))),
            max_entries,
            source,
        }
    }

    /// The origin tag applied to entries captured by this buffer.
    #[must_use]
    pub const fn source(&self) -> LogSource {
        self.source
    }

    /// Add a log entry
    pub fn push(&self, entry: LogEntry) {
        let mut entries = self.entries.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.push(entry);

        // Keep only the last N entries
        if entries.len() > self.max_entries {
            let excess = entries.len() - self.max_entries;
            entries.drain(0..excess);
        }
        debug_assert!(
            entries.len() <= self.max_entries,
            "log buffer must stay within its capacity"
        );
        drop(entries);
    }

    /// Get all entries
    #[must_use]
    pub fn get_all(&self) -> Vec<LogEntry> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Get last N entries
    #[must_use]
    pub fn get_recent(&self, limit: usize) -> Vec<LogEntry> {
        let entries = self.entries.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        entries
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    /// Clear all entries
    pub fn clear(&self) {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }
}

// =============================================================================
// Tracing Layer for capturing logs into the buffer
// =============================================================================

/// Visitor that extracts the message field from tracing events
#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        }
    }
}

/// Most bytes of span context kept per line. The chain the agent loop opens
/// (turn → step → run → iteration → call) with ids renders in ~350 bytes; the
/// cap bounds the buffer at capacity × this even if a span records a huge
/// field, and a cut is marked.
const SCOPE_BYTES_MAX: usize = 512;

/// A span's fields rendered as `key=value key=value`, kept in the span's
/// extensions so each line renders its chain without re-visiting fields.
struct SpanFields(String);

/// Renders every field of a span as `key=value`, space-separated.
struct FieldsVisitor<'a>(&'a mut String);

impl Visit for FieldsVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if !self.0.is_empty() {
            self.0.push(' ');
        }
        let _ = write!(self.0, "{}={value:?}", field.name());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if !self.0.is_empty() {
            self.0.push(' ');
        }
        let _ = write!(self.0, "{}={value}", field.name());
    }
}

/// Whether a span belongs in a line's scope: Nanna's own, the same rule the
/// daemon's console and file layers apply (`own_spans_only`), so a
/// dependency's internal spans never crowd the context out.
fn is_own_span(metadata: &tracing::Metadata<'_>) -> bool {
    metadata.target().starts_with("nanna")
}

/// Tracing layer that captures events into a `LogBuffer`
pub struct LogBufferLayer {
    buffer: LogBuffer,
}

impl LogBufferLayer {
    #[must_use]
    pub const fn new(buffer: LogBuffer) -> Self {
        Self { buffer }
    }
}

impl LogBufferLayer {
    /// The event's own-span chain, outermost first, bounded by
    /// [`SCOPE_BYTES_MAX`].
    fn scope_of<S>(event: &tracing::Event<'_>, ctx: &Context<'_, S>) -> String
    where
        S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
    {
        let mut scope = String::new();
        let Some(spans) = ctx.event_scope(event) else {
            return scope;
        };
        for span in spans.from_root() {
            if !is_own_span(span.metadata()) {
                continue;
            }
            if !scope.is_empty() {
                scope.push(':');
            }
            scope.push_str(span.name());
            if let Some(fields) = span.extensions().get::<SpanFields>()
                && !fields.0.is_empty()
            {
                let _ = write!(scope, "{{{}}}", fields.0);
            }
            if scope.len() > SCOPE_BYTES_MAX {
                let cut = scope.floor_char_boundary(SCOPE_BYTES_MAX);
                scope.truncate(cut);
                scope.push('…');
                break;
            }
        }
        debug_assert!(
            scope.len() <= SCOPE_BYTES_MAX + '…'.len_utf8(),
            "the scope is bounded"
        );
        scope
    }
}

impl<S> Layer<S> for LogBufferLayer
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if !is_own_span(attrs.metadata()) {
            return;
        }
        let Some(span) = ctx.span(id) else {
            return;
        };
        let mut fields = String::new();
        attrs.record(&mut FieldsVisitor(&mut fields));
        span.extensions_mut().insert(SpanFields(fields));
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        if let Some(fields) = span.extensions_mut().get_mut::<SpanFields>() {
            values.record(&mut FieldsVisitor(&mut fields.0));
        }
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let level = metadata.level().as_str().to_lowercase();
        let target = metadata.target().to_string();

        // Extract message from event fields
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let entry = LogEntry {
            timestamp: chrono::Local::now()
                .format("%Y-%m-%d %H:%M:%S%.3f")
                .to_string(),
            level,
            target,
            message: visitor.message,
            source: self.buffer.source(),
            scope: Self::scope_of(event, &ctx),
        };

        self.buffer.push(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(index: usize, source: LogSource) -> LogEntry {
        LogEntry {
            timestamp: format!("2024-01-01 12:00:{index:02}"),
            level: "info".to_string(),
            target: "test".to_string(),
            message: format!("Message {index}"),
            source,
            scope: String::new(),
        }
    }

    #[test]
    fn test_log_buffer() {
        let buffer = LogBuffer::new(10, LogSource::Daemon);

        for i in 0..15 {
            buffer.push(entry(i, LogSource::Daemon));
        }

        let entries = buffer.get_all();
        assert_eq!(entries.len(), 10);
        assert_eq!(entries[0].message, "Message 5");
        assert_eq!(entries[9].message, "Message 14");
    }

    #[test]
    fn test_get_recent() {
        let buffer = LogBuffer::new(100, LogSource::Daemon);

        for i in 0..20 {
            buffer.push(entry(i, LogSource::Daemon));
        }

        let recent = buffer.get_recent(5);
        assert_eq!(recent.len(), 5);
        assert_eq!(recent[0].message, "Message 15");
        assert_eq!(recent[4].message, "Message 19");
    }

    /// The buffer's tag is what the capturing layer stamps, so an embedded buffer
    /// can never silently serve entries that look like they came from a daemon.
    #[test]
    fn buffer_reports_its_own_source() {
        assert_eq!(
            LogBuffer::new(4, LogSource::Embedded).source(),
            LogSource::Embedded
        );
        assert_eq!(
            LogBuffer::new(4, LogSource::Daemon).source(),
            LogSource::Daemon
        );
    }

    /// The GUI merges both origins into one view, so the tag must survive the wire.
    #[test]
    fn source_round_trips_through_json() {
        let json = serde_json::to_string(&entry(1, LogSource::Embedded)).expect("serialize");
        assert!(json.contains(r#""source":"embedded""#), "got: {json}");

        let back: LogEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.source, LogSource::Embedded);
    }

    /// A daemon predating the `source` field sends untagged entries. They must still
    /// parse (a newer GUI against an older daemon) and be attributed to the daemon —
    /// never mislabelled as the GUI's own embedded lines.
    #[test]
    fn untagged_wire_entry_defaults_to_daemon() {
        let legacy =
            r#"{"timestamp":"2024-01-01 12:00:00","level":"info","target":"t","message":"m"}"#;

        let parsed: LogEntry = serde_json::from_str(legacy).expect("legacy entry must parse");

        assert_eq!(parsed.source, LogSource::Daemon);
        assert_eq!(parsed.message, "m");
    }

    /// A line logged inside Nanna's spans carries the chain, outermost
    /// first, with each span's fields — including fields recorded after the
    /// span opened. Dependency spans are left out; a line outside any span
    /// has no scope, and the empty scope stays off the wire.
    #[test]
    fn lines_carry_their_own_span_chain() {
        use tracing_subscriber::layer::SubscriberExt as _;

        let buffer = LogBuffer::new(16, LogSource::Daemon);
        let subscriber = tracing_subscriber::registry().with(LogBufferLayer::new(buffer.clone()));
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "nanna_test", "outside");
            let turn = tracing::info_span!(target: "nanna_test", "chat_turn", session_id = "s-1");
            let _turn = turn.enter();
            let foreign = tracing::info_span!(target: "hyper", "request", uri = "/x");
            let _foreign = foreign.enter();
            let call = tracing::info_span!(
                target: "nanna_test",
                "tool_call",
                tool = "exec",
                success = tracing::field::Empty
            );
            let _call = call.enter();
            call.record("success", true);
            tracing::info!(target: "nanna_test", "inside");
        });

        let entries = buffer.get_all();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].scope, "", "a line outside any span has no scope");
        assert_eq!(
            entries[1].scope, "chat_turn{session_id=s-1}:tool_call{tool=exec success=true}",
            "own spans only, outermost first, recorded fields included"
        );
        let wire = serde_json::to_string(&entries[0]).expect("serialize");
        assert!(
            !wire.contains("scope"),
            "empty scope stays off the wire: {wire}"
        );
    }

    #[test]
    fn source_as_str_matches_wire_encoding() {
        assert_eq!(LogSource::Embedded.as_str(), "embedded");
        assert_eq!(LogSource::Daemon.as_str(), "daemon");
    }
}
