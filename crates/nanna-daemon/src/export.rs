//! Session export: a conversation as a document the user owns.
//!
//! Until 2026-09-11 `PRIVACY.md` said "the Turso database files themselves are
//! the export". The daemon owns the session store (turso holds an exclusive
//! lock on it), so the document is rendered here and handed to any client over
//! IPC — `nanna export` today, a GUI button later — one renderer rather than
//! one per client re-deriving the transcript.
//!
//! **Markdown** is the readable transcript, laid out the way the chat page lays
//! out a message: an assistant message with a run journal renders the journal
//! (thinking, tool calls with input and output, edit diffs, healed faults,
//! steps) and its `content` only when the journal holds no text of its own; a
//! message without one renders reasoning, tool calls, then content. **JSON** is
//! the lossless one: the stored `Session` verbatim, in a versioned envelope.
//!
//! Writing into a `String` through `fmt::Write` cannot fail, so each
//! `let _ = write!(..)` below discards an `Ok(())`, never an error.

use std::fmt::Write as _;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::protocol::ExportFormat;
use crate::session::{
    EditDiff, MessageRole, Session, SessionMessage, TimelineItem, ToolCallRecord,
};
use nanna_memory::MemoryExportRecord;

/// Bumped when the JSON envelope changes incompatibly.
pub const EXPORT_FORMAT_VERSION: u32 = 1;

/// Longest filename stem an export suggests, in bytes (ASCII by construction).
const FILENAME_STEM_BYTES_MAX: usize = 64;

/// A rendered export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportDocument {
    /// Suggested file name, e.g. `fix-the-build.md`.
    pub filename: String,
    pub content: String,
}

#[derive(Serialize)]
struct JsonEnvelope<'a> {
    nanna_export: u32,
    exported_at: String,
    session: &'a Session,
}

/// Render `session` as a document in `format`.
///
/// # Errors
/// Only the JSON path can fail, and only if `serde_json` refuses a value in the
/// session's metadata; the Markdown path is infallible.
pub fn export_session(
    session: &Session,
    format: ExportFormat,
    exported_at: DateTime<Utc>,
) -> Result<ExportDocument, serde_json::Error> {
    let stem = filename_stem(session);
    let document = match format {
        ExportFormat::Markdown => ExportDocument {
            filename: format!("{stem}.md"),
            content: render_markdown(session, exported_at),
        },
        ExportFormat::Json => {
            let envelope = JsonEnvelope {
                nanna_export: EXPORT_FORMAT_VERSION,
                exported_at: exported_at.to_rfc3339(),
                session,
            };
            ExportDocument {
                filename: format!("{stem}.json"),
                content: serde_json::to_string_pretty(&envelope)?,
            }
        }
    };
    debug_assert!(!document.content.is_empty(), "an export always has content");
    debug_assert!(
        document.filename.len() <= FILENAME_STEM_BYTES_MAX + ".json".len(),
        "the suggested name is bounded"
    );
    Ok(document)
}

/// A filesystem-safe stem from the session name, falling back to the id when
/// the name yields nothing (unnamed, or all punctuation and emoji).
fn filename_stem(session: &Session) -> String {
    let stem = slug(session.name.as_deref().unwrap_or_default());
    if !stem.is_empty() {
        return stem;
    }
    let id: String = session
        .id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(8)
        .collect();
    format!("session-{id}")
}

/// Lowercase ASCII alphanumerics joined by single dashes, at most
/// [`FILENAME_STEM_BYTES_MAX`] bytes; empty when `source` has none.
fn slug(source: &str) -> String {
    let mut stem = String::with_capacity(FILENAME_STEM_BYTES_MAX);
    for ch in source.chars() {
        if stem.len() >= FILENAME_STEM_BYTES_MAX {
            break;
        }
        if ch.is_ascii_alphanumeric() {
            stem.push(ch.to_ascii_lowercase());
        } else if !stem.is_empty() && !stem.ends_with('-') {
            stem.push('-');
        }
    }
    let trimmed = stem.trim_end_matches('-');
    debug_assert!(
        trimmed.len() <= FILENAME_STEM_BYTES_MAX,
        "the stem is bounded"
    );
    debug_assert!(
        trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "the stem is filesystem-safe"
    );
    trimmed.to_string()
}

/// Longest heading a memory gets in the Markdown export, in chars: a preview,
/// the full content follows it.
const MEMORY_HEADING_CHARS_MAX: usize = 72;

#[derive(Serialize)]
struct MemoryEnvelope<'a> {
    nanna_export: u32,
    kind: &'static str,
    exported_at: String,
    scope: Option<&'a str>,
    count: usize,
    memories: &'a [&'a MemoryExportRecord],
}

/// Render memory `records` — already filtered to `scope` — as a document.
///
/// Records are ordered oldest first with the id as tiebreak, so exporting the
/// same store twice yields the same document.
///
/// # Errors
/// Only the JSON path can fail, as for [`export_session`].
pub fn export_memories(
    records: &[MemoryExportRecord],
    scope: Option<&str>,
    format: ExportFormat,
    exported_at: DateTime<Utc>,
) -> Result<ExportDocument, serde_json::Error> {
    let mut ordered: Vec<&MemoryExportRecord> = records.iter().collect();
    ordered.sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then_with(|| a.id.cmp(&b.id)));
    let stem = scope.map(slug).filter(|s| !s.is_empty()).map_or_else(
        || "nanna-memories".to_string(),
        |scope_slug| format!("nanna-memories-{scope_slug}"),
    );
    let document = match format {
        ExportFormat::Markdown => ExportDocument {
            filename: format!("{stem}.md"),
            content: render_memories_markdown(&ordered, scope, exported_at),
        },
        ExportFormat::Json => {
            let envelope = MemoryEnvelope {
                nanna_export: EXPORT_FORMAT_VERSION,
                kind: "memories",
                exported_at: exported_at.to_rfc3339(),
                scope,
                count: ordered.len(),
                memories: &ordered,
            };
            ExportDocument {
                filename: format!("{stem}.json"),
                content: serde_json::to_string_pretty(&envelope)?,
            }
        }
    };
    debug_assert!(!document.content.is_empty(), "an export always has content");
    debug_assert!(
        document.filename.starts_with("nanna-memories"),
        "the name says what it holds"
    );
    Ok(document)
}

fn render_memories_markdown(
    records: &[&MemoryExportRecord],
    scope: Option<&str>,
    exported_at: DateTime<Utc>,
) -> String {
    let mut out = String::from("# Nanna memories\n\n");
    let scope_text = match scope {
        None => "every memory".to_string(),
        Some("global") => "global memories only".to_string(),
        Some(workspace) => format!("global memories and workspace `{workspace}`"),
    };
    let _ = writeln!(out, "- Scope: {scope_text}");
    let _ = writeln!(out, "- Memories: {}", records.len());
    let _ = writeln!(out, "- Exported: {} by Nanna", exported_at.to_rfc3339());
    out.push_str(
        "- Embedding vectors are not included: they are derived data, recomputed by a re-embed.\n",
    );
    for record in records {
        render_memory(&mut out, record);
    }
    debug_assert!(
        out.starts_with("# Nanna memories"),
        "the document says what it holds"
    );
    out
}

fn render_memory(out: &mut String, record: &MemoryExportRecord) {
    let preview: String = one_line(&record.content)
        .chars()
        .take(MEMORY_HEADING_CHARS_MAX)
        .collect();
    let heading = if preview.is_empty() {
        record.id.as_str()
    } else {
        preview.as_str()
    };
    let _ = write!(out, "\n---\n\n### {heading}\n\n");
    push_prose(out, &record.content, false);
    let workspace = record
        .workspace_id
        .as_deref()
        .map_or_else(|| "global".to_string(), |w| format!("`{w}`"));
    let _ = writeln!(out, "- Id: `{}`", record.id);
    let _ = writeln!(out, "- Provenance: {}", record.fact_type);
    let _ = writeln!(out, "- Workspace: {workspace}");
    let _ = writeln!(out, "- Stored: {}", unix_seconds_rfc3339(record.timestamp));
    let _ = writeln!(
        out,
        "- State: {} · importance {:.2} · retrievability {:.2} · weight {:.2}",
        record.state, record.fsrs.importance, record.retrievability, record.weight
    );
    let _ = writeln!(
        out,
        "- FSRS: stability {:.2} d · difficulty {:.2} · accessed {}× (last {}) · generation {}",
        record.fsrs.stability,
        record.fsrs.difficulty,
        record.fsrs.access_count,
        unix_seconds_rfc3339(record.fsrs.last_access),
        record.fsrs.generation
    );
}

/// Unix seconds as RFC 3339, or the raw number when out of chrono's range.
fn unix_seconds_rfc3339(seconds: i64) -> String {
    DateTime::from_timestamp(seconds, 0).map_or_else(|| seconds.to_string(), |at| at.to_rfc3339())
}

fn render_markdown(session: &Session, exported_at: DateTime<Utc>) -> String {
    let mut out = String::new();
    render_header(&mut out, session, exported_at);
    for message in &session.messages {
        render_message(&mut out, message);
    }
    debug_assert!(out.starts_with("# "), "the transcript opens with its title");
    out
}

fn render_header(out: &mut String, session: &Session, exported_at: DateTime<Utc>) {
    let title = session
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("Untitled session");
    let _ = writeln!(out, "# {}\n", one_line(title));
    let _ = writeln!(out, "- Session: `{}`", session.id);
    let _ = writeln!(out, "- Created: {}", session.created_at.to_rfc3339());
    let _ = writeln!(out, "- Updated: {}", session.updated_at.to_rfc3339());
    if let Some(workspace) = &session.workspace_id {
        let _ = writeln!(out, "- Workspace: `{workspace}`");
    }
    let _ = writeln!(out, "- Messages: {}", session.messages.len());
    let _ = writeln!(out, "- Exported: {} by Nanna", exported_at.to_rfc3339());
}

fn render_message(out: &mut String, message: &SessionMessage) {
    let _ = write!(
        out,
        "\n---\n\n## {} · {}\n\n",
        role_label(&message.role),
        message.timestamp.to_rfc3339()
    );
    let is_assistant = message.role == MessageRole::Assistant;
    if is_assistant && !message.timeline.is_empty() {
        for item in &message.timeline {
            render_timeline_item(out, item);
        }
        // Mirrors the chat page: the reply only when the journal carries no
        // text of its own (runs journaled before text capture existed).
        if !timeline_has_text(&message.timeline) {
            push_prose(out, &message.content, true);
        }
    } else {
        if is_assistant {
            if let Some(reasoning) = message.reasoning.as_deref() {
                render_thinking(out, reasoning);
            }
            for call in &message.tool_calls {
                render_tool_record(out, call);
            }
        }
        push_prose(out, &message.content, is_assistant);
    }
    for attachment in &message.attachments {
        let _ = writeln!(
            out,
            "- Attachment: `{}` ({})",
            attachment.filename, attachment.content_type
        );
    }
    if let Some(usage) = &message.usage {
        let _ = writeln!(
            out,
            "\n*{} · {} in / {} out tokens · {} ms*\n",
            usage.model, usage.input_tokens, usage.output_tokens, usage.duration_ms
        );
    }
}

const fn role_label(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::User => "You",
        MessageRole::Assistant => "Nanna",
        MessageRole::System => "System",
        MessageRole::Tool => "Tool",
    }
}

fn render_timeline_item(out: &mut String, item: &TimelineItem) {
    match item {
        TimelineItem::Thinking { content, .. } => render_thinking(out, content),
        TimelineItem::Text { content, .. } => push_prose(out, content, true),
        TimelineItem::Tool {
            name,
            input,
            output,
            success,
            duration_ms,
            short_circuited,
            diff,
            ..
        } => {
            let view = ToolView {
                name,
                input: input.as_ref(),
                output: output.as_deref(),
                outcome: tool_outcome(*success, *short_circuited),
                duration_ms: *duration_ms,
            };
            render_tool(out, &view);
            if let Some(diff) = diff {
                render_diff(out, diff);
            }
        }
        TimelineItem::Fault { message, .. } => {
            let _ = writeln!(
                out,
                "> ⚡ Stream fault, healed and continued: {}\n",
                one_line(message)
            );
        }
        TimelineItem::Step { phase, label, .. } => {
            let _ = writeln!(out, "*{}: {}*\n", one_line(phase), one_line(label));
        }
    }
}

fn timeline_has_text(timeline: &[TimelineItem]) -> bool {
    timeline.iter().any(
        |item| matches!(item, TimelineItem::Text { content, .. } if has_renderable_text(content)),
    )
}

/// Ordinary message text, verbatim Markdown. An assistant's has the harness's
/// `TASK COMPLETE` plumbing lines dropped, exactly as the chat page does.
fn push_prose(out: &mut String, text: &str, is_assistant: bool) {
    let text = if is_assistant {
        strip_harness_markers(text)
    } else {
        text.to_string()
    };
    if text.trim().is_empty() {
        return;
    }
    let body = text.trim_end();
    out.push_str(body);
    if let Some(fence) = unclosed_fence(body) {
        let _ = write!(out, "\n{fence}");
        debug_assert!(
            unclosed_fence(&format!("{body}\n{fence}")).is_none(),
            "the appended fence closes the one left open"
        );
    }
    out.push_str("\n\n");
}

/// The fence that would close a code block `text` leaves open, if any.
///
/// Message prose is pasted verbatim, and the chat page renders each message on
/// its own — but the export concatenates them, so a reply cut off mid-block
/// would swallow every heading, tool block and diff after it. The rule, as
/// `CommonMark` has it: a fence is a run of 3+ backticks or tildes after at most 3 spaces;
/// a backtick fence's info string holds no backtick; it is closed by a run of
/// the same character at least as long, followed by nothing but spaces.
fn unclosed_fence(text: &str) -> Option<String> {
    let mut open: Option<(char, usize)> = None;
    for line in text.lines() {
        let indent = line.len() - line.trim_start_matches(' ').len();
        if indent > 3 {
            continue;
        }
        let rest = &line[indent..];
        let Some(ch) = rest.chars().next().filter(|c| *c == '`' || *c == '~') else {
            continue;
        };
        let run = rest.chars().take_while(|c| *c == ch).count();
        if run < 3 {
            continue;
        }
        // `ch` is ASCII, so `run` characters are `run` bytes.
        let after = &rest[run..];
        open = match open {
            None if ch == '`' && after.contains('`') => None,
            None => Some((ch, run)),
            Some((open_ch, open_run))
                if ch == open_ch && run >= open_run && after.trim().is_empty() =>
            {
                None
            }
            still_open => still_open,
        };
    }
    debug_assert!(
        open.is_none_or(|(_, run)| run >= 3),
        "only real fences open"
    );
    open.map(|(ch, run)| ch.to_string().repeat(run))
}

fn render_thinking(out: &mut String, thinking: &str) {
    let thinking = strip_harness_markers(thinking);
    let thinking = thinking.trim();
    if thinking.is_empty() {
        return;
    }
    out.push_str("> **Thinking**\n>\n");
    for line in thinking.lines() {
        if line.is_empty() {
            out.push_str(">\n");
        } else {
            let _ = writeln!(out, "> {line}");
        }
    }
    out.push('\n');
}

/// One tool call, from either the run journal or a legacy flat record.
struct ToolView<'a> {
    name: &'a str,
    input: Option<&'a serde_json::Value>,
    output: Option<&'a str>,
    outcome: &'static str,
    duration_ms: Option<u64>,
}

fn render_tool_record(out: &mut String, call: &ToolCallRecord) {
    let view = ToolView {
        name: &call.name,
        input: Some(&call.input),
        output: call.output.as_deref(),
        outcome: tool_outcome(call.success, None),
        duration_ms: call.duration_ms,
    };
    render_tool(out, &view);
}

fn render_tool(out: &mut String, tool: &ToolView<'_>) {
    let _ = write!(out, "**Tool `{}`** — {}", tool.name, tool.outcome);
    if let Some(ms) = tool.duration_ms {
        let _ = write!(out, " · {ms} ms");
    }
    out.push_str("\n\n");
    if let Some(input) = tool.input.filter(|value| !value.is_null()) {
        let text = serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string());
        push_fenced(out, &text, "json");
    }
    if let Some(output) = tool.output.filter(|text| !text.trim().is_empty()) {
        push_fenced(out, output, "text");
    }
}

/// How a call ended, in words. A breaker replay is steering, not failure —
/// the same distinction the journal and the chat page draw.
const fn tool_outcome(success: Option<bool>, short_circuited: Option<bool>) -> &'static str {
    match (short_circuited, success) {
        (Some(true), _) => "steering (the breaker answered; the tool did not run)",
        (_, Some(true)) => "ok",
        (_, Some(false)) => "failed",
        (_, None) => "did not finish",
    }
}

fn render_diff(out: &mut String, diff: &EditDiff) {
    let mut body = format!("@@ line {} @@\n", diff.start_line);
    for line in &diff.removed {
        body.push('-');
        body.push_str(line);
        body.push('\n');
    }
    for line in &diff.added {
        body.push('+');
        body.push_str(line);
        body.push('\n');
    }
    if diff.truncated {
        body.push_str("… (longer than shown)\n");
    }
    push_fenced(out, &body, "diff");
}

/// Put `body` in a fenced block it cannot close: the fence is one backtick
/// longer than the longest backtick run inside it, and never under three —
/// the Markdown spec's rule, so a tool output quoting Markdown stays in its
/// block.
fn push_fenced(out: &mut String, body: &str, lang: &str) {
    let fence_len = longest_backtick_run(body).max(2) + 1;
    let fence = "`".repeat(fence_len);
    let _ = write!(
        out,
        "{fence}{lang}\n{}\n{fence}\n\n",
        body.trim_end_matches('\n')
    );
    debug_assert!(fence_len >= 3, "a fence is at least three backticks");
    debug_assert!(
        fence_len > longest_backtick_run(body),
        "…and longer than any run inside"
    );
}

fn longest_backtick_run(text: &str) -> usize {
    let mut longest = 0_usize;
    let mut run = 0_usize;
    for byte in text.bytes() {
        if byte == b'`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    longest
}

/// Drop whole `TASK COMPLETE` lines — the harness's completion marker, which is
/// plumbing, not conversation. Mirrors `stripHarnessMarkers` in
/// `gui/app/lib/harnessMarkers.ts`: whole lines only, inline mentions kept.
fn strip_harness_markers(text: &str) -> String {
    if !text.to_ascii_lowercase().contains("task complete") {
        return text.to_string();
    }
    text.lines()
        .filter(|line| !line.trim().eq_ignore_ascii_case("task complete"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_renderable_text(text: &str) -> bool {
    !strip_harness_markers(text).trim().is_empty()
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{AttachmentRecord, RunUsage};

    fn at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-11T10:00:00Z")
            .expect("fixed timestamp parses")
            .with_timezone(&Utc)
    }

    fn message(role: MessageRole, content: &str) -> SessionMessage {
        SessionMessage {
            id: format!("m-{content}"),
            role,
            content: content.to_string(),
            timestamp: at(),
            tool_calls: Vec::new(),
            attachments: Vec::new(),
            reasoning: None,
            timeline: Vec::new(),
            usage: None,
        }
    }

    fn tool_item(name: &str, output: &str, diff: Option<EditDiff>) -> TimelineItem {
        TimelineItem::Tool {
            call_id: "c1".to_string(),
            name: name.to_string(),
            input: Some(serde_json::json!({ "file_path": "a.rs" })),
            output: Some(output.to_string()),
            success: Some(true),
            duration_ms: Some(4),
            tokens: None,
            total_tokens: None,
            short_circuited: Some(false),
            diff,
            at: at().to_rfc3339(),
        }
    }

    fn edit_session() -> Session {
        let mut session = Session::new(Some("Fix the Build! 🚀".to_string()));
        session
            .messages
            .push(message(MessageRole::User, "please fix it"));
        let mut reply = message(MessageRole::Assistant, "Fixed it.");
        reply.timeline = vec![
            TimelineItem::Thinking {
                content: "the error is on line 3".to_string(),
                at: at().to_rfc3339(),
            },
            tool_item(
                "edit_file",
                "Edited a.rs",
                Some(EditDiff {
                    start_line: 3,
                    removed: vec!["let x = 1;".to_string()],
                    added: vec!["let x = 2;".to_string()],
                    truncated: false,
                }),
            ),
            TimelineItem::Text {
                content: "Fixed it.\nTASK COMPLETE\n".to_string(),
                at: at().to_rfc3339(),
            },
        ];
        reply.usage = Some(RunUsage {
            input_tokens: 120,
            output_tokens: 30,
            duration_ms: 2500,
            model: "qwen3.5:9b".to_string(),
        });
        session.messages.push(reply);
        session
    }

    fn markdown(session: &Session) -> String {
        export_session(session, ExportFormat::Markdown, at())
            .expect("markdown export is infallible")
            .content
    }

    #[test]
    fn markdown_renders_the_conversation_in_order() {
        let md = markdown(&edit_session());
        assert!(md.starts_with("# Fix the Build! 🚀\n"), "{md}");
        let order = [
            "## You",
            "please fix it",
            "> **Thinking**",
            "**Tool `edit_file`** — ok",
            "@@ line 3 @@",
            "Fixed it.",
        ];
        let positions: Vec<usize> = order
            .iter()
            .map(|needle| {
                md.find(needle)
                    .unwrap_or_else(|| panic!("missing {needle:?} in:\n{md}"))
            })
            .collect();
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "out of order:\n{md}"
        );
        assert!(
            md.contains("```diff\n@@ line 3 @@\n-let x = 1;\n+let x = 2;\n```"),
            "{md}"
        );
        assert!(
            md.contains("*qwen3.5:9b · 120 in / 30 out tokens · 2500 ms*"),
            "{md}"
        );
    }

    /// The chat page shows `content` beside a journal only when the journal has
    /// no text; the export follows it, so the reply is neither lost nor doubled.
    #[test]
    fn the_reply_appears_exactly_once() {
        let md = markdown(&edit_session());
        assert_eq!(md.matches("Fixed it.").count(), 1, "{md}");
        assert!(
            !md.contains("TASK COMPLETE"),
            "harness plumbing is not conversation:\n{md}"
        );

        let mut session = Session::new(Some("tools only".to_string()));
        let mut reply = message(MessageRole::Assistant, "Done.");
        reply.timeline = vec![tool_item("exec", "ok", None)];
        session.messages.push(reply);
        assert!(
            markdown(&session).contains("Done."),
            "a journal with no text keeps the reply"
        );
    }

    #[test]
    fn a_legacy_message_renders_reasoning_tools_and_content() {
        let mut session = Session::new(None);
        let mut reply = message(MessageRole::Assistant, "Sorry, that failed.");
        reply.reasoning = Some("try the fast path".to_string());
        reply.tool_calls = vec![ToolCallRecord {
            id: "t1".to_string(),
            name: "exec".to_string(),
            input: serde_json::json!({ "cmd": "make" }),
            output: Some("make: *** No rule".to_string()),
            success: Some(false),
            duration_ms: Some(9),
        }];
        reply.attachments = vec![AttachmentRecord {
            id: "a1".to_string(),
            filename: "log.txt".to_string(),
            content_type: "text/plain".to_string(),
            url: None,
        }];
        session.messages.push(reply);
        let md = markdown(&session);
        assert!(md.contains("> try the fast path"), "{md}");
        assert!(md.contains("**Tool `exec`** — failed · 9 ms"), "{md}");
        assert!(md.contains("Sorry, that failed."), "{md}");
        assert!(md.contains("- Attachment: `log.txt` (text/plain)"), "{md}");
    }

    /// A tool output that itself contains a Markdown fence must not close the
    /// block it is quoted in — the rest of the transcript would render as code.
    #[test]
    fn a_fence_inside_a_tool_output_cannot_escape_its_block() {
        let mut session = Session::new(Some("fences".to_string()));
        let mut reply = message(MessageRole::Assistant, "");
        reply.timeline = vec![tool_item("read_file", "```\nrm -rf /\n```", None)];
        session.messages.push(reply);
        let md = markdown(&session);
        assert!(md.contains("````text\n```\nrm -rf /\n```\n````"), "{md}");
    }

    #[test]
    fn tool_outcomes_distinguish_steering_from_failure() {
        assert_eq!(
            tool_outcome(Some(false), Some(true)),
            "steering (the breaker answered; the tool did not run)"
        );
        assert_eq!(tool_outcome(Some(false), Some(false)), "failed");
        assert_eq!(tool_outcome(Some(true), None), "ok");
        assert_eq!(tool_outcome(None, None), "did not finish");
    }

    /// JSON is the lossless export: the stored session, versioned, and it
    /// deserializes back into the same session — diff included.
    #[test]
    fn json_export_is_lossless_and_versioned() {
        let session = edit_session();
        let document = export_session(&session, ExportFormat::Json, at()).expect("json export");
        let value: serde_json::Value = serde_json::from_str(&document.content).expect("valid JSON");
        assert_eq!(
            value["nanna_export"],
            serde_json::json!(EXPORT_FORMAT_VERSION)
        );
        assert_eq!(value["exported_at"], serde_json::json!(at().to_rfc3339()));
        let restored: Session =
            serde_json::from_value(value["session"].clone()).expect("the session round-trips");
        assert_eq!(restored.id, session.id);
        assert_eq!(restored.messages.len(), 2);
        match &restored.messages[1].timeline[1] {
            TimelineItem::Tool {
                diff: Some(diff), ..
            } => assert_eq!(diff.start_line, 3),
            other => panic!("expected the edit with its diff, got {other:?}"),
        }
    }

    #[test]
    fn the_suggested_filename_is_filesystem_safe() {
        let named = export_session(&edit_session(), ExportFormat::Markdown, at()).expect("md");
        assert_eq!(named.filename, "fix-the-build.md");

        let emoji_only = Session::new(Some("🚀 🚀".to_string()));
        let document = export_session(&emoji_only, ExportFormat::Json, at()).expect("json");
        assert!(
            document.filename.starts_with("session-"),
            "{}",
            document.filename
        );
        assert!(
            std::path::Path::new(&document.filename)
                .extension()
                .is_some_and(|ext| ext == "json"),
            "{}",
            document.filename
        );

        let long = Session::new(Some("x".repeat(500)));
        let document = export_session(&long, ExportFormat::Markdown, at()).expect("md");
        assert_eq!(
            document.filename.len(),
            FILENAME_STEM_BYTES_MAX + ".md".len()
        );
    }

    fn record(
        id: &str,
        content: &str,
        timestamp: i64,
        workspace: Option<&str>,
    ) -> MemoryExportRecord {
        MemoryExportRecord {
            id: id.to_string(),
            content: content.to_string(),
            fact_type: "stated".to_string(),
            metadata: std::collections::HashMap::new(),
            timestamp,
            workspace_id: workspace.map(str::to_string),
            embedding_model: Some("ollama:nomic-embed-text".to_string()),
            state: "active".to_string(),
            weight: 0.9,
            retrievability: 0.95,
            fsrs: nanna_memory::FsrsState::default(),
        }
    }

    fn memories_markdown(records: &[MemoryExportRecord], scope: Option<&str>) -> String {
        export_memories(records, scope, ExportFormat::Markdown, at())
            .expect("markdown export is infallible")
            .content
    }

    #[test]
    fn memory_markdown_lists_each_memory_oldest_first() {
        let records = [
            record("b", "prefers tea in the afternoon", 200, Some("ws1")),
            record("a", "the user prefers dark roast", 100, None),
        ];
        let md = memories_markdown(&records, None);
        assert!(md.starts_with("# Nanna memories\n"), "{md}");
        assert!(md.contains("- Memories: 2"), "{md}");
        assert!(md.contains("Embedding vectors are not included"), "{md}");
        let first = md
            .find("the user prefers dark roast")
            .expect("memory a is exported");
        let second = md
            .find("prefers tea in the afternoon")
            .expect("memory b is exported");
        assert!(first < second, "oldest first:\n{md}");
        assert!(md.contains("- Provenance: stated"), "{md}");
        assert!(md.contains("- Workspace: global"), "{md}");
        assert!(md.contains("- Workspace: `ws1`"), "{md}");
        assert!(
            md.contains("- FSRS: stability 1.00 d · difficulty 5.00"),
            "{md}"
        );
    }

    /// The lossless memory export is versioned, says what it holds, and
    /// carries FSRS state but no embedding vectors.
    #[test]
    fn memory_json_is_versioned_and_carries_no_vectors() {
        let records = [
            record("a", "x", 100, None),
            record("b", "y", 200, Some("ws1")),
        ];
        let document =
            export_memories(&records, Some("ws1"), ExportFormat::Json, at()).expect("json export");
        let value: serde_json::Value = serde_json::from_str(&document.content).expect("valid JSON");
        assert_eq!(
            value["nanna_export"],
            serde_json::json!(EXPORT_FORMAT_VERSION)
        );
        assert_eq!(value["kind"], "memories");
        assert_eq!(value["scope"], "ws1");
        assert_eq!(value["count"], 2);
        assert_eq!(value["memories"][0]["id"], "a", "oldest first");
        assert_eq!(
            value["memories"][0]["fsrs"]["stability"],
            serde_json::json!(1.0)
        );
        assert!(!document.content.contains("\"embedding\":"), "no vector");
        assert!(
            !document.content.contains("\"embeddings\":"),
            "no vector buckets"
        );
    }

    #[test]
    fn memory_export_filenames_follow_the_scope() {
        let name = |scope: Option<&str>, format| {
            export_memories(&[], scope, format, at())
                .expect("export")
                .filename
        };
        assert_eq!(name(None, ExportFormat::Markdown), "nanna-memories.md");
        assert_eq!(
            name(Some("global"), ExportFormat::Json),
            "nanna-memories-global.json"
        );
        assert_eq!(
            name(Some("../My Workspace"), ExportFormat::Markdown),
            "nanna-memories-my-workspace.md",
            "a scope is slugged, never used as a path"
        );
    }

    #[test]
    fn an_empty_store_still_exports_a_document() {
        let md = memories_markdown(&[], Some("global"));
        assert!(md.contains("- Memories: 0"), "{md}");
        assert!(md.contains("- Scope: global memories only"), "{md}");
    }

    #[test]
    fn an_unclosed_fence_is_found_by_commonmark_rules() {
        assert_eq!(unclosed_fence("plain prose"), None);
        assert_eq!(unclosed_fence("```rust\nfn main() {}\n```"), None);
        assert_eq!(
            unclosed_fence("cut off:\n```rust\nfn main() {"),
            Some("```".to_string())
        );
        assert_eq!(unclosed_fence("~~~~\ncode"), Some("~~~~".to_string()));
        // Closed only by the same character, at least as long, alone on its line.
        assert_eq!(
            unclosed_fence("````md\n```\ninner\n```\n"),
            Some("````".to_string())
        );
        assert_eq!(unclosed_fence("~~~\n```\n~~~"), None);
        assert_eq!(
            unclosed_fence("```\ncode\n``` trailing"),
            Some("```".to_string())
        );
        // Not fences: four spaces of indent, a backtick in a backtick info
        // string, an inline run mid-line, a run shorter than three.
        assert_eq!(unclosed_fence("    ```\ncode"), None);
        assert_eq!(unclosed_fence("```a`b\ncode"), None);
        assert_eq!(unclosed_fence("see ```x``` here"), None);
        assert_eq!(unclosed_fence("``\ncode"), None);
        // Multi-byte text around a fence cannot split a character.
        assert_eq!(
            unclosed_fence("  ```é\nnaïve — code"),
            Some("```".to_string())
        );
    }

    /// The reviewed failure: a reply cut off mid-block used to swallow every
    /// heading and tool block after it in the concatenated export.
    #[test]
    fn prose_that_leaves_a_fence_open_is_closed_before_the_next_block() {
        let mut out = String::new();
        push_prose(&mut out, "Here:\n```rust\nfn main() {", true);
        push_prose(&mut out, "## Next message", false);
        assert_eq!(unclosed_fence(&out), None, "{out}");
        assert!(
            out.contains("fn main() {\n```\n\n## Next message"),
            "the fence closes right after the cut-off text: {out}"
        );
    }
}
