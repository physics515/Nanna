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

    let mut out = String::with_capacity(text.len());
    for (idx, line) in text.lines().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        out.push_str(&strip_line(line));
    }
    if text.ends_with('\n') {
        out.push('\n');
    }

    // Postcondition: stripping only ever removes/rewrites-shorter, never grows.
    debug_assert!(out.len() <= text.len(), "strip_markdown must not grow text");
    out
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
    fn non_text_content_is_untouched() {
        let img = MessageContent::Image {
            url: "u".into(),
            caption: Some("**c**".into()),
        };
        // Image passes through even on a plain channel (no text field to strip).
        let out = format_for_channel(img, &caps(false));
        assert!(matches!(out, MessageContent::Image { .. }));
    }
}
