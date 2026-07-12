//! Per-channel response formatting (P8 `ResponseFormatter`, first slice).
//!
//! Agent responses are Markdown by default. Channels that render Markdown
//! (Discord/Telegram/Slack — flagged with the `MARKDOWN` feature) receive the
//! text unchanged; channels that display raw text (Signal, `WhatsApp`) would
//! otherwise show literal `**bold**`, backticks, and `[label](url)` link
//! syntax. For those we down-convert to readable plain text before sending.
//!
//! The stripper is deliberately conservative: it only removes syntax that is
//! *essentially always* Markdown, so ordinary prose — and code identifiers like
//! `__init__`, `snake_case`, or arithmetic `2 * 3` — survive untouched.

use crate::{ChannelCapabilities, MessageContent};

/// Adapt outgoing `content` to what `caps` can render.
///
/// For a text message on a channel WITHOUT [`ChannelFeatures::MARKDOWN`], strip
/// Markdown to plain text. Non-text content and Markdown-capable channels are
/// returned unchanged.
#[must_use]
pub fn format_for_channel(content: MessageContent, caps: &ChannelCapabilities) -> MessageContent {
    match content {
        MessageContent::Text { text } if !caps.supports_markdown() => MessageContent::Text {
            text: strip_markdown(&text),
        },
        other => other,
    }
}

/// Down-convert common Markdown in `text` to readable plain text.
///
/// Handles ATX headers, blockquote markers, fenced-code fences, bold
/// (`**x**`), inline code backticks, and `[label](url)` links (kept as
/// `label (url)`). Single `*`/`_` (italic) and `__` are left alone so
/// arithmetic and identifiers survive.
#[must_use]
pub fn strip_markdown(text: &str) -> String {
    // Guard: this runs on agent output, which is already length-bounded upstream.
    debug_assert!(text.len() < 1 << 24, "strip_markdown on oversized text");

    let lines: Vec<&str> = text.lines().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < lines.len() {
        // Markdown table block: a row line immediately followed by a delimiter
        // row (`|---|---|`). Render each non-delimiter row as plain text and
        // drop the delimiter, so channels that don't render tables get readable
        // rows instead of literal pipes and dashes.
        if i + 1 < lines.len() && is_table_row(lines[i]) && is_table_delimiter(lines[i + 1]) {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&strip_table_row(lines[i]));
            i += 2; // consumed header + delimiter
            while i < lines.len() && is_table_row(lines[i]) && !is_table_delimiter(lines[i]) {
                out.push('\n');
                out.push_str(&strip_table_row(lines[i]));
                i += 1;
            }
            continue;
        }
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&strip_line(lines[i]));
        i += 1;
    }
    if text.ends_with('\n') {
        out.push('\n');
    }

    // Postcondition: stripping shrinks in general, but re-joining tight table
    // cells with " | " can add a few separator chars — cap at 2x as a
    // runaway-growth guard.
    debug_assert!(out.len() <= text.len() * 2 + 1, "strip_markdown grew unexpectedly");
    out
}

/// A line that looks like a Markdown table row: non-empty and contains a pipe.
/// Only consulted when the *next* line is a delimiter row (or we're already
/// inside a detected table), so stray prose pipes elsewhere aren't misread.
fn is_table_row(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty() && t.contains('|')
}

/// A Markdown table delimiter row (`|---|:--:|`): only pipes, dashes, colons,
/// and spaces, with at least one dash AND one pipe. The pipe requirement rules
/// out a bare `---` horizontal rule that merely follows a line with a pipe.
fn is_table_delimiter(line: &str) -> bool {
    let t = line.trim();
    if !t.contains('-') || !t.contains('|') {
        return false;
    }
    t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// Convert a table row to plain text: drop the outer pipes, trim each cell,
/// strip inline Markdown inside cells, and join with " | ".
fn strip_table_row(line: &str) -> String {
    let t = line.trim();
    let inner = t.strip_prefix('|').unwrap_or(t);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    inner
        .split('|')
        .map(|cell| strip_inline(cell.trim()))
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Strip block- and inline-level Markdown from a single line.
fn strip_line(line: &str) -> String {
    let trimmed = line.trim_start();
    // Drop fenced-code delimiter lines (```lang / ```), keep their contents.
    if trimmed.starts_with("```") {
        return String::new();
    }

    let indent_len = line.len() - trimmed.len();
    let indent = &line[..indent_len];

    // Block markers apply to the post-indent body.
    let mut body = trimmed;
    body = body.strip_prefix("> ").unwrap_or(body); // blockquote
    body = strip_atx_header(body); // "# ", "## ", ...

    let inline = strip_inline(body);
    debug_assert!(
        inline.len() <= body.len() + indent.len(),
        "inline strip grew"
    );
    if indent.is_empty() {
        inline
    } else {
        format!("{indent}{inline}")
    }
}

/// Remove a leading ATX header marker (`#`..`######` then one space).
fn strip_atx_header(body: &str) -> &str {
    let hashes = body.bytes().take_while(|&b| b == b'#').count();
    if (1..=6).contains(&hashes) && body.as_bytes().get(hashes) == Some(&b' ') {
        // Skip the hashes and the single following space.
        &body[hashes + 1..]
    } else {
        body
    }
}

/// Remove inline Markdown: bold (`**`), inline-code backticks, and rewrite
/// `[label](url)` links to `label (url)`.
fn strip_inline(body: &str) -> String {
    debug_assert!(body.len() < 1 << 24, "strip_inline on oversized line");
    let without_links = rewrite_links(body);
    // Bold markers and stray backticks are near-never literal prose.
    let result = without_links.replace("**", "").replace('`', "");
    debug_assert!(result.len() <= without_links.len(), "inline strip grew");
    result
}

/// Replace every `[label](url)` with `label (url)`; leave all other text as-is.
fn rewrite_links(body: &str) -> String {
    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'['
            && let Some(rel_close) = body[i + 1..].find(']')
        {
            let close = i + 1 + rel_close;
            // Require the link form `](` immediately after the label.
            if body.as_bytes().get(close + 1) == Some(&b'(')
                && let Some(rel_paren) = body[close + 2..].find(')')
            {
                let paren = close + 2 + rel_paren;
                let label = &body[i + 1..close];
                let url = &body[close + 2..paren];
                out.push_str(label);
                out.push_str(" (");
                out.push_str(url);
                out.push(')');
                i = paren + 1;
                continue;
            }
        }
        // Not a link start: copy this char verbatim (handle UTF-8 boundaries).
        let ch = body[i..].chars().next().unwrap_or('\u{FFFD}');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Split `text` into chunks each at most `max_chars` Unicode scalar values,
/// for channels that cap message length (`ChannelCapabilities::max_message_length`).
///
/// Breaks are preferred at a newline, then a space, within the leading
/// `max_chars`-char window; a single token longer than `max_chars` is hard-split
/// mid-word as a last resort. Chunks concatenate back to the exact input (the
/// break character stays at the end of the preceding chunk), so no content is
/// lost. Returns a single chunk when the text already fits.
///
/// # Panics
///
/// Panics if `max_chars` is 0 (a zero-length limit can't bound any chunk).
#[must_use]
pub fn split_for_length(text: &str, max_chars: usize) -> Vec<String> {
    assert!(max_chars > 0, "max_chars must be positive");

    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return vec![text.to_string()];
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let remaining = chars.len() - start;
        if remaining <= max_chars {
            chunks.push(chars[start..].iter().collect());
            break;
        }
        // Prefer to break after the last newline, else the last space, within the
        // window; otherwise hard-break at the char limit. `+ 1` keeps the break
        // character at the end of this chunk so the pieces rejoin exactly.
        let hard_end = start + max_chars;
        let window = &chars[start..hard_end];
        let break_at = window
            .iter()
            .rposition(|&c| c == '\n')
            .or_else(|| window.iter().rposition(|&c| c == ' '))
            .map_or(hard_end, |rel| start + rel + 1);
        debug_assert!(break_at > start, "split must make progress");
        chunks.push(chars[start..break_at].iter().collect());
        start = break_at;
    }

    debug_assert!(chunks.iter().all(|c| c.chars().count() <= max_chars));
    debug_assert!(!chunks.is_empty());
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChannelFeatures;

    fn caps(markdown: bool) -> ChannelCapabilities {
        ChannelCapabilities {
            features: if markdown {
                ChannelFeatures::MARKDOWN
            } else {
                ChannelFeatures::empty()
            },
            max_message_length: None,
        }
    }

    #[test]
    fn markdown_channel_passes_text_through() {
        let content = MessageContent::text("**bold** and `code`");
        let out = format_for_channel(content, &caps(true));
        assert_eq!(out.as_text(), Some("**bold** and `code`"));
    }

    #[test]
    fn plain_channel_strips_bold_and_code() {
        let content = MessageContent::text("**bold** and `code`");
        let out = format_for_channel(content, &caps(false));
        assert_eq!(out.as_text(), Some("bold and code"));
    }

    #[test]
    fn strips_headers_and_blockquotes() {
        assert_eq!(strip_markdown("# Title\n> quoted"), "Title\nquoted");
        assert_eq!(strip_markdown("### Deep"), "Deep");
        // Not a header: no space after hashes.
        assert_eq!(strip_markdown("#hashtag"), "#hashtag");
    }

    #[test]
    fn rewrites_links_to_label_and_url() {
        assert_eq!(
            strip_markdown("see [docs](https://x.io/y) now"),
            "see docs (https://x.io/y) now"
        );
    }

    #[test]
    fn preserves_code_identifiers_and_arithmetic() {
        // The dangerous cases: dunders, snake_case, single-star math must survive.
        let s = "call __init__ on snake_case with 2 * 3";
        assert_eq!(strip_markdown(s), s);
    }

    #[test]
    fn drops_code_fences_keeps_body() {
        let input = "before\n```rust\nlet x = 1;\n```\nafter";
        assert_eq!(strip_markdown(input), "before\n\nlet x = 1;\n\nafter");
    }

    #[test]
    fn table_becomes_plain_rows_delimiter_dropped() {
        let input = "| Name | Age |\n|------|-----|\n| Alice | 30 |\n| Bob | 25 |";
        // Delimiter row is dropped; each row's outer pipes go, cells joined " | ".
        assert_eq!(strip_markdown(input), "Name | Age\nAlice | 30\nBob | 25");
    }

    #[test]
    fn table_with_alignment_and_surrounding_text() {
        let input = "Here:\n| A | B |\n|:--|--:|\n| 1 | 2 |\nDone";
        assert_eq!(strip_markdown(input), "Here:\nA | B\n1 | 2\nDone");
    }

    #[test]
    fn table_cells_get_inline_markdown_stripped() {
        let input = "| Tool | Note |\n|---|---|\n| `read` | **fast** |";
        assert_eq!(strip_markdown(input), "Tool | Note\nread | fast");
    }

    #[test]
    fn prose_pipe_and_horizontal_rule_are_not_tables() {
        // A pipe in prose with no delimiter row must survive unchanged.
        assert_eq!(strip_markdown("choose a | b here"), "choose a | b here");
        // A bare `---` horizontal rule after a pipe line is NOT a table
        // delimiter (no pipe in the rule), so nothing is reformatted.
        assert_eq!(strip_markdown("x | y\n---\nz"), "x | y\n---\nz");
    }

    #[test]
    fn table_output_does_not_trip_growth_guard() {
        // Tight, pipe-heavy table exercises the relaxed postcondition.
        let input = "|a|b|c|d|\n|-|-|-|-|\n|1|2|3|4|";
        let out = strip_markdown(input);
        assert_eq!(out, "a | b | c | d\n1 | 2 | 3 | 4");
    }

    #[test]
    fn non_text_content_is_untouched() {
        let img = MessageContent::Image {
            url: "u".into(),
            caption: Some("**c**".into()),
        };
        // Image passes through even on a plain channel (no text field to strip).
        let out = format_for_channel(img, &caps(false));
        assert!(matches!(out, MessageContent::Image { .. }));
    }

    #[test]
    fn split_returns_single_chunk_when_within_limit() {
        assert_eq!(split_for_length("hello", 10), vec!["hello".to_string()]);
        // Exactly at the limit is still one chunk.
        assert_eq!(split_for_length("hello", 5), vec!["hello".to_string()]);
    }

    #[test]
    fn split_breaks_at_whitespace_and_rejoins_exactly() {
        let text = "one two three four five";
        let parts = split_for_length(text, 8);
        // Every part is within the limit...
        assert!(parts.iter().all(|p| p.chars().count() <= 8));
        // ...more than one part was produced...
        assert!(parts.len() >= 2);
        // ...and concatenation reproduces the input exactly (no content lost).
        assert_eq!(parts.concat(), text);
        // Breaks landed on spaces, so no part starts with a space.
        assert!(parts.iter().all(|p| !p.starts_with(' ')));
    }

    #[test]
    fn split_prefers_newline_over_space() {
        // With a newline inside the window, the break lands there.
        let parts = split_for_length("aa bb\ncc dd", 7);
        assert_eq!(parts[0], "aa bb\n");
        assert_eq!(parts.concat(), "aa bb\ncc dd");
    }

    #[test]
    fn split_hard_breaks_an_oversized_token() {
        // A single 10-char token with no break point, limit 4 → ceil(10/4)=3 parts.
        let parts = split_for_length("abcdefghij", 4);
        assert_eq!(parts, vec!["abcd", "efgh", "ij"]);
        assert_eq!(parts.concat(), "abcdefghij");
    }

    #[test]
    fn split_counts_unicode_scalars_not_bytes() {
        // Four 2-byte chars; a byte-based splitter would mis-bound these.
        let parts = split_for_length("áéíóú", 2);
        assert!(parts.iter().all(|p| p.chars().count() <= 2));
        assert_eq!(parts.concat(), "áéíóú");
    }
}
