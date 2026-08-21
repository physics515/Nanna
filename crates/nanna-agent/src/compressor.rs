//! LLMLingua-style prompt compression using the configured summarization model.
//!
//! Scores sentences by information density via an LLM (typically the user's
//! summarization model from settings — local Ollama / small GPU models first),
//! then keeps the highest-scoring sentences in original order until a target
//! compression ratio is hit.
//!
//! Unlike true LLMLingua (per-token perplexity via a local causal LM), this is a
//! sentence-level approach that works over the same chat API the rest of the
//! agent uses — no separate GPU tokenizer stack required.
//!
//! Applied **selectively** to large tool outputs and soak-level context bulk;
//! never to system prompts or the most recent user/assistant turns.

use nanna_llm::{estimate_tokens, AnthropicMessage, AnthropicRequest, ContentBlock, LlmClient};
use tracing::{debug, info, warn};

/// Ceiling on the scorer's output. The scorer emits one small number per
/// sentence and nothing else, so it never needs a large budget — but the cap
/// is also a hard capacity limit on how many sentences can be scored at all,
/// which is why [`scoring_is_futile`] checks against it before spending a
/// round-trip.
const SCORE_OUTPUT_CAP_TOKENS: usize = 256;

/// Worst-case scorer output for one sentence: the two-digit maximum score,
/// plus the line break that separates it from the next. Costed piecewise with
/// the estimator the rest of the agent budgets with
/// (`nanna_llm::estimate_tokens`) because the model emits them as separate
/// tokens — not as a guessed constant.
const WORST_CASE_SCORE: &str = "10";
const SCORE_LINE_TERMINATOR: &str = "\n";

/// Configuration for LLMLingua-style compression.
///
/// The model/client come from the agent’s summarization settings (`AgentConfig::
/// summarization_priority` + `create_client_for_model`) — this struct only tunes
/// ratio / length gates so compression never runs on tiny payloads.
#[derive(Debug, Clone)]
pub struct CompressionConfig {
    /// Target compression ratio (e.g., 4 = compress to ~1/4 of original size).
    pub ratio: usize,
    /// Minimum content length (chars) before compression is worth attempting.
    pub min_content_length: usize,
    /// Minimum number of sentences required before selective dropping helps.
    pub min_sentences: usize,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            ratio: 4,
            min_content_length: 500,
            min_sentences: 4,
        }
    }
}

/// Compress text by scoring sentences and dropping low-importance ones.
///
/// Returns the compressed text, or `None` if compression failed or wasn't worthwhile.
pub async fn compress_text(
    client: &LlmClient,
    model: &str,
    content: &str,
    target_ratio: usize,
) -> Option<String> {
    compress_text_with_config(
        client,
        model,
        content,
        &CompressionConfig {
            ratio: target_ratio.max(1),
            ..CompressionConfig::default()
        },
    )
    .await
}

/// Compress text using an explicit [`CompressionConfig`].
pub async fn compress_text_with_config(
    client: &LlmClient,
    model: &str,
    content: &str,
    config: &CompressionConfig,
) -> Option<String> {
    if content.len() < config.min_content_length {
        return None;
    }

    let sentences = split_sentences(content);
    if sentences.len() < config.min_sentences {
        return None;
    }

    let target_count = sentences.len() / config.ratio.max(1);
    if target_count < 2 {
        return None;
    }

    let model_cache = nanna_llm::ModelInfoCache::default_location();
    let model_info = client.get_model_info(model, model_cache.as_ref()).await;
    let max_tokens = u32::try_from(model_info.max_output_tokens.min(SCORE_OUTPUT_CAP_TOKENS))
        .unwrap_or(u32::MAX);

    if let Some(reason) = scoring_is_futile(content, &sentences, max_tokens as usize) {
        debug!(
            sentences = sentences.len(),
            max_tokens, reason, model = %model,
            "Skipping the scoring round-trip and reducing by whole lines"
        );
        return elide_by_lines(content, config.ratio);
    }

    let numbered: String = sentences
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{}: {}", i + 1, s.trim()))
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = format!(
        "Rate each numbered sentence by information importance (1-10). \
         1=filler/boilerplate, 10=critical information. \
         Output ONLY numbers, one per line, in order.\n\n{numbered}"
    );

    let request = AnthropicRequest {
        context_limit: None,
        model: model.to_string(),
        messages: vec![AnthropicMessage::user_text(prompt)],
        max_tokens,
        temperature: nanna_llm::sampling_temperature_for_model(model, 0.1),
        system: Some(
            "You are an information density scorer. Output ONLY one number (1-10) per line, nothing else."
                .to_string(),
        ),
        tools: None,
        stream: None,
        thinking: None,
        cache_control: None,
    };

    let response = match client.complete_anthropic(&request).await {
        Ok(r) => r,
        Err(e) => {
            debug!(error = %e, model = %model, "Compression scoring failed");
            return None;
        }
    };

    let scores_text: String = response
        .content
        .iter()
        .filter_map(|b| {
            if let ContentBlock::Text { text } = b {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let scores = parse_scores(&scores_text);
    if scores.len() != sentences.len() {
        debug!(
            expected = sentences.len(),
            got = scores.len(),
            model = %model,
            "Score count mismatch, falling back to head/tail reduction"
        );
        return elide_by_lines(content, config.ratio);
    }

    let keep_count = target_count.max(2).min(sentences.len());
    let keep_indices = select_keep_indices(&scores, keep_count);
    // Survivors keep the shape they arrived in. A "sentence" here can be a
    // whole LINE (`split_sentences` treats `\n` as a terminator), and joining
    // trimmed lines with spaces flattened a listing onto one line with its
    // line-number prefixes still attached — under a banner calling it a
    // summary. A line stays a line, indentation and all.
    let mut compressed = String::new();
    for &i in &keep_indices {
        let sentence = sentences[i];
        if sentence.ends_with('\n') {
            compressed.push_str(sentence.trim_end());
            compressed.push('\n');
        } else {
            compressed.push_str(sentence.trim());
            compressed.push(' ');
        }
    }
    let compressed = compressed.trim_end().to_string();

    let original_len = content.len();
    let compressed_len = compressed.len();
    if compressed_len == 0 || compressed_len >= original_len {
        return None;
    }
    let actual_ratio = original_len / compressed_len.max(1);

    info!(
        original_chars = original_len,
        compressed_chars = compressed_len,
        actual_ratio = actual_ratio,
        sentences_kept = keep_count,
        sentences_total = sentences.len(),
        model = %model,
        "🗜️ LLMLingua compression: {original_len} → {compressed_len} chars ({actual_ratio}x)"
    );

    Some(compressed)
}

/// Try each summarization model in priority order.
///
/// Walks `models` with the supplied client factory. The factory should map a
/// settings model spec (`"ollama/phi3:mini"`, `"openai/gpt-4o-mini"`, bare
/// model names, …) onto an [`LlmClient`] + bare model name. Mirrors how the
/// agent already builds clients for tool-output summarization.
pub async fn compress_with_priority<F>(
    content: &str,
    target_ratio: usize,
    models: &[String],
    mut make_client: F,
) -> Option<String>
where
    F: FnMut(&str) -> Result<(LlmClient, String), String>,
{
    if models.is_empty() {
        return None;
    }
    let config = CompressionConfig {
        ratio: target_ratio.max(1),
        ..CompressionConfig::default()
    };
    if content.len() < config.min_content_length {
        return None;
    }

    // Line-structured content needs no model at all: scoring it would ask a
    // summarizer to rank a listing's lines and then flatten the winners. Doing
    // this before the loop is what keeps the wasted round-trip from being
    // spent once per candidate.
    if is_line_structured(content) {
        debug!(
            chars = content.len(),
            "Line-structured content: reducing by whole lines, no scorer needed"
        );
        return elide_by_lines(content, config.ratio);
    }

    for model_spec in models {
        let (client, model_name) = match make_client(model_spec) {
            Ok(pair) => pair,
            Err(e) => {
                warn!(model = %model_spec, error = %e, "Skipping compression model");
                continue;
            }
        };
        match compress_text_with_config(&client, &model_name, content, &config).await {
            Some(compressed) if compressed.len() < content.len() => {
                return Some(compressed);
            }
            Some(_) => {
                debug!(
                    model = %model_spec,
                    "Compression returned non-shrinking result, trying next model"
                );
            }
            None => {
                debug!(
                    model = %model_spec,
                    "Compression returned None, trying next model"
                );
            }
        }
    }
    None
}

/// Parse one score per line from the scorer model output.
///
/// Accepts bare numbers (`7`), numbered lines (`1: 7`, `1. 7`, `1) 7`), and
/// surrounds of whitespace. Values are clamped to 1..=10; unparseable lines
/// are skipped so callers can detect count mismatches.
#[must_use]
pub fn parse_scores(scores_text: &str) -> Vec<u8> {
    scores_text
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            // Formats: "7", "1: 7", "1. 7", "1) 7", "- 7"
            let num_str = if let Some((_prefix, score)) = trimmed.split_once(':') {
                score.trim()
            } else if let Some((_prefix, score)) = trimmed.split_once(')') {
                score.trim()
            } else if let Some((first, rest)) = trimmed.split_once(|c: char| c == '.' || c == '-') {
                // Only treat as labeled if the left side is a pure index number
                if first.trim().chars().all(|c| c.is_ascii_digit()) && !rest.trim().is_empty() {
                    rest.trim()
                } else {
                    trimmed
                }
            } else {
                trimmed
            };
            // Take the first integer run (handles "7/10", "7 points", "score=9")
            let token: String = num_str
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if token.is_empty() {
                None
            } else {
                token.parse::<u8>().ok().map(|n| n.clamp(1, 10))
            }
        })
        .collect()
}

/// Pick the top-`keep_count` sentence indices by score, restoring original order.
///
/// Ties break toward earlier sentences (stable, deterministic).
#[must_use]
pub fn select_keep_indices(scores: &[u8], keep_count: usize) -> Vec<usize> {
    if scores.is_empty() || keep_count == 0 {
        return Vec::new();
    }
    let mut indexed: Vec<(usize, u8)> = scores.iter().copied().enumerate().collect();
    // Higher score first; stable index order for ties (enumerate is ascending).
    indexed.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let keep = keep_count.min(indexed.len());
    let mut indices: Vec<usize> = indexed[..keep].iter().map(|(i, _)| *i).collect();
    indices.sort_unstable();
    indices
}

/// Why sentence scoring cannot work on this content, or `None` if it can.
///
/// Two independent reasons, both decided before any request is sent:
///
/// 1. **The score vector provably cannot fit.** The scorer must return one
///    number per sentence, and its output is capped. Past
///    `cap / cost of one score line` sentences the reply can never match the
///    input count, for any model however capable — every attempt burns a
///    round-trip (3.3-3.7 s observed, 116 times across four models) and then
///    falls through anyway.
/// 2. **The content is line-structured, not prose.** `split_sentences` treats
///    `\n` as a terminator, so a file listing's "sentences" are its lines.
///    Scoring lines by "information importance" and keeping the winners
///    scatters a listing; whole-line reduction preserves what makes it
///    readable. Reserve sentence scoring for prose.
#[must_use]
pub fn scoring_is_futile(
    content: &str,
    sentences: &[&str],
    max_output_tokens: usize,
) -> Option<&'static str> {
    let per_score =
        (estimate_tokens(WORST_CASE_SCORE) + estimate_tokens(SCORE_LINE_TERMINATOR)).max(1);
    if sentences.len().saturating_mul(per_score) > max_output_tokens {
        return Some("score vector cannot fit the scorer's output cap");
    }
    if is_line_structured(content) {
        return Some("content is line-structured, not prose");
    }
    None
}

/// Whether `content` is a listing, diff, log, or serialized blob rather than
/// prose.
///
/// Only decisive signals, each a plain majority (or an unambiguous framing
/// marker) rather than a tuned fraction — a wrong "prose" verdict merely
/// spends a round-trip that would have failed anyway, while a wrong
/// "structured" verdict costs only the sentence-level selection.
#[must_use]
pub fn is_line_structured(content: &str) -> bool {
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() < 4 {
        return false;
    }

    let trimmed = content.trim();
    // Serialized framing: the whole payload is one structure.
    if (trimmed.starts_with('{') || trimmed.starts_with('['))
        && (trimmed.ends_with('}') || trimmed.ends_with(']'))
    {
        return true;
    }
    // Diff framing.
    if trimmed.starts_with("diff --git")
        || trimmed.starts_with("--- ")
        || trimmed.starts_with("@@ ")
    {
        return true;
    }
    // A numbered listing: the line number IS the structure, and it is what
    // survived into the flattened output that made this visible.
    let numbered = lines
        .iter()
        .filter(|l| leading_line_number(l).is_some())
        .count();
    if numbered * 2 > lines.len() {
        return true;
    }
    // Indented structure: code, YAML, tree output.
    let indented = lines
        .iter()
        .filter(|l| l.starts_with(' ') || l.starts_with('\t'))
        .count();
    indented * 2 > lines.len()
}

/// The line number a listing line carries, if any: optional indentation, a
/// digit run, then a separator that is not part of the content.
#[must_use]
pub fn leading_line_number(line: &str) -> Option<u64> {
    let rest = line.trim_start();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let after = &rest[digits.len()..];
    let separated = matches!(after.chars().next(), Some('\t' | ':' | '|' | ' '));
    if separated {
        digits.parse().ok()
    } else {
        None
    }
}

/// Shape-preserving reduction: keep a contiguous head and tail of WHOLE
/// lines, name the elided range where it happened.
///
/// Whole lines, indentation intact, line numbers contiguous within each kept
/// run, and the gap named — the reader can see exactly which lines are
/// missing instead of finding a listing silently 75% shorter.
#[must_use]
pub fn elide_by_lines(content: &str, ratio: usize) -> Option<String> {
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    if lines.len() < 4 {
        // Nothing line-shaped to preserve; the char-level path is honest here.
        return fallback_compress(content, ratio);
    }
    let target_len = content.len() / ratio.max(1);
    let half = target_len / 2;
    if half < 100 {
        return None;
    }

    let mut head_lines = 0usize;
    let mut head_chars = 0usize;
    while head_lines < lines.len() && head_chars + lines[head_lines].len() <= half {
        head_chars += lines[head_lines].len();
        head_lines += 1;
    }
    let mut tail_lines = 0usize;
    let mut tail_chars = 0usize;
    while head_lines + tail_lines < lines.len()
        && tail_chars + lines[lines.len() - 1 - tail_lines].len() <= half
    {
        tail_chars += lines[lines.len() - 1 - tail_lines].len();
        tail_lines += 1;
    }
    // A line longer than the whole half-budget would otherwise leave the head
    // or tail empty, reducing the content to a marker and nothing else. One
    // line each way is the floor; the shrink check below rejects the result if
    // that floor buys nothing.
    if head_lines == 0 {
        head_lines = 1;
    }
    if tail_lines == 0 && head_lines < lines.len() {
        tail_lines = 1;
    }

    if head_lines + tail_lines >= lines.len() {
        return None;
    }
    let elided_lines = lines.len() - head_lines - tail_lines;
    let head: String = lines[..head_lines].concat();
    let tail: String = lines[lines.len() - tail_lines..].concat();
    let elided_chars = content.len() - head.len() - tail.len();

    // Name the gap by the listing's OWN numbering when it has one, so the
    // reader can go back to the file and find exactly what is missing.
    let first_gap = head_lines;
    let last_gap = head_lines + elided_lines - 1;
    let range = match (
        leading_line_number(lines[first_gap]),
        leading_line_number(lines[last_gap]),
    ) {
        (Some(first), Some(last)) => format!("lines {first}-{last}"),
        _ => format!(
            "lines {}-{} of {}",
            first_gap + 1,
            last_gap + 1,
            lines.len()
        ),
    };
    let marker = format!(
        "\n[... {elided_lines} lines elided ({range}, {elided_chars} chars) to fit the \
         context window. The lines above and below are VERBATIM and complete — nothing \
         was reflowed or reordered. Read the file for the elided range.]\n"
    );

    let out = format!("{head}{marker}{tail}");
    if out.len() >= content.len() {
        return None;
    }
    Some(out)
}

/// Fallback compression: keep first ~half and last ~half of the target length.
///
/// Char-level, for content with no line structure to preserve. The marker
/// names how much went — a silently shorter blob reads as damaged data.
#[must_use]
pub fn fallback_compress(content: &str, ratio: usize) -> Option<String> {
    let target_len = content.len() / ratio.max(1);
    let half = target_len / 2;
    if half < 100 {
        return None;
    }
    // Snap to char boundaries to avoid panicking on multi-byte UTF-8.
    let start_end = content.floor_char_boundary(half);
    let tail_offset = content.len().saturating_sub(half);
    let end_start = if content.is_char_boundary(tail_offset) {
        tail_offset
    } else {
        (tail_offset..content.len())
            .find(|&i| content.is_char_boundary(i))
            .unwrap_or(content.len())
    };
    let start = &content[..start_end];
    let end = &content[end_start..];
    let elided = content.len() - start.len() - end.len();
    Some(format!(
        "{start}\n[...compressed: {elided} chars elided from the middle...]\n{end}"
    ))
}

/// Split text into sentences using simple heuristics.
#[must_use]
pub fn split_sentences(text: &str) -> Vec<&str> {
    let mut sentences = Vec::new();
    let mut start = 0;

    for (i, c) in text.char_indices() {
        if (c == '.' || c == '!' || c == '?' || c == '\n') && i > start + 10 {
            let next_char = text[i + c.len_utf8()..].chars().next();
            let is_sentence_end = match next_char {
                Some(' ') | Some('\n') | None => true,
                _ => c == '\n',
            };
            if is_sentence_end {
                let end = i + c.len_utf8();
                let sentence = &text[start..end];
                if sentence.trim().len() > 5 {
                    sentences.push(sentence);
                }
                start = end;
            }
        }
    }

    if start < text.len() {
        let remainder = &text[start..];
        if remainder.trim().len() > 5 {
            sentences.push(remainder);
        }
    }

    sentences
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The GATE, not the helper it delegates to. Every other test here calls
    /// `is_line_structured` / `elide_by_lines` directly, so removing the
    /// short-circuit from `compress_with_priority` left them all green while
    /// the wasted round-trip came back — and that round-trip (3.3-3.7 s a
    /// time, burned on four of five models) is the entire point of the item.
    ///
    /// The client factory panics: reaching it at all means the gate is gone.
    #[tokio::test]
    async fn line_structured_content_never_reaches_a_model() {
        let listing = (1..=200)
            .map(|n| format!("{n:>4}	src/module_{n}.rs"))
            .collect::<Vec<_>>()
            .join("
");
        assert!(
            is_line_structured(&listing),
            "fixture must be the shape the gate is for"
        );

        let out = compress_with_priority(&listing, 4, &["ollama/whatever".to_string()], |_| {
            panic!("line-structured content must be reduced without a scoring round-trip")
        })
        .await;

        let out = out.expect("the line reducer answers");
        assert!(
            out.len() < listing.len(),
            "it must actually reduce: {} -> {}",
            listing.len(),
            out.len()
        );
        assert!(
            out.contains('\n'),
            "and reduce by whole LINES, not flatten the listing onto one"
        );
    }

    #[test]
    fn test_split_sentences() {
        let text = "This is a sentence. This is another sentence! And a third one? Yes.";
        let sentences = split_sentences(text);
        assert!(
            sentences.len() >= 3,
            "Expected at least 3 sentences, got {}",
            sentences.len()
        );
    }

    #[test]
    fn test_split_sentences_newlines() {
        let text = "Line one content here\nLine two content here\nLine three content here";
        let sentences = split_sentences(text);
        assert!(
            sentences.len() >= 2,
            "Expected at least 2 sentences, got {}",
            sentences.len()
        );
    }

    #[test]
    fn test_fallback_compress() {
        let text = "A".repeat(1000);
        let result = fallback_compress(&text, 4);
        assert!(result.is_some());
        let compressed = result.unwrap();
        assert!(compressed.len() < text.len());
        assert!(
            compressed.contains("chars elided from the middle"),
            "a silently shorter blob reads as damaged data: {compressed}"
        );
    }

    #[test]
    fn test_fallback_compress_utf8_safe() {
        // Multi-byte chars at the snap boundary must not panic.
        let text = "日本語のテスト文章です。".repeat(80);
        let result = fallback_compress(&text, 4);
        assert!(result.is_some());
    }

    #[test]
    fn test_parse_scores_bare() {
        let text = "7\n3\n10\n1";
        assert_eq!(parse_scores(text), vec![7, 3, 10, 1]);
    }

    #[test]
    fn test_parse_scores_labeled() {
        let text = "1: 7\n2: 3\n3) 9\n4. 2\n5 - 8";
        assert_eq!(parse_scores(text), vec![7, 3, 9, 2, 8]);
    }

    #[test]
    fn test_parse_scores_clamped_and_skips_junk() {
        let text = "0\n15\nnot-a-score\n5\n7/10";
        // 0→1, 15→10, junk skipped, 5 kept, 7 from "7/10"
        assert_eq!(parse_scores(text), vec![1, 10, 5, 7]);
    }

    #[test]
    fn test_select_keep_indices_preserves_order() {
        // Scores: idx0=2, idx1=9, idx2=4, idx3=9 — keep top 2 → indices 1,3 (tie: earlier first)
        let scores = vec![2, 9, 4, 9];
        assert_eq!(select_keep_indices(&scores, 2), vec![1, 3]);
        assert_eq!(select_keep_indices(&scores, 3), vec![1, 2, 3]);
        assert_eq!(select_keep_indices(&scores, 0), Vec::<usize>::new());
        assert_eq!(select_keep_indices(&[], 2), Vec::<usize>::new());
    }

    #[test]
    fn test_select_keep_indices_rebuilds_compressed_text() {
        let text = "Alpha sentence here. Bravo content here! Charlie stuff here? Delta final here.";
        let sentences = split_sentences(text);
        assert!(sentences.len() >= 4, "got {}", sentences.len());
        // Prefer 1st and 3rd sentences by contrived scores
        let scores: Vec<u8> = (0..sentences.len())
            .map(|i| if i == 0 || i == 2 { 10 } else { 1 })
            .collect();
        let keep = select_keep_indices(&scores, 2);
        assert_eq!(keep, vec![0, 2]);
        let compressed: String = keep
            .iter()
            .map(|&i| sentences[i].trim())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(compressed.contains("Alpha"));
        assert!(compressed.contains("Charlie"));
        assert!(!compressed.contains("Bravo"));
    }

    #[test]
    fn test_default_config() {
        let cfg = CompressionConfig::default();
        assert_eq!(cfg.ratio, 4);
        assert_eq!(cfg.min_content_length, 500);
        assert_eq!(cfg.min_sentences, 4);
    }

    // -----------------------------------------------------------------
    // P24.15 — structured text is not sentence-scored
    // -----------------------------------------------------------------

    fn numbered_listing(lines: usize) -> String {
        (1..=lines)
            .map(|n| format!("{n}\tfn handler_{n}(req: Request) -> Response {{ todo!() }}\n"))
            .collect()
    }

    /// The scorer must return one number per sentence and its output is
    /// capped, so past `cap / cost-of-one-score-line` sentences the vector can
    /// never match — for any model, however capable. 116 rewrites across four
    /// models burned a 3.3-3.7 s round-trip before falling through anyway.
    #[test]
    fn scoring_is_refused_when_the_score_vector_provably_cannot_fit() {
        let content = numbered_listing(400);
        let sentences = split_sentences(&content);
        assert!(sentences.len() > 128, "got {}", sentences.len());
        assert_eq!(
            scoring_is_futile(&content, &sentences, 256),
            Some("score vector cannot fit the scorer's output cap")
        );

        // Prose that fits the cap still goes to the scorer.
        let prose = "This is a real sentence with content. Another follows it here. \
                     And a third makes the point. A fourth closes the paragraph."
            .to_string();
        let prose_sentences = split_sentences(&prose);
        assert_eq!(scoring_is_futile(&prose, &prose_sentences, 256), None);
    }

    #[test]
    fn line_structured_content_is_detected_and_prose_is_not() {
        assert!(is_line_structured(&numbered_listing(20)));
        assert!(is_line_structured(
            "{\n  \"a\": 1,\n  \"b\": 2,\n  \"c\": [3, 4]\n}"
        ));
        assert!(is_line_structured(
            "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n"
        ));
        assert!(is_line_structured(
            "fn main() {\n    let a = 1;\n    let b = 2;\n    println!(\"{a}{b}\");\n}"
        ));
        assert!(!is_line_structured(
            "The compressor scores sentences. It then keeps the highest ones. \
             That works for prose. It does not work for listings."
        ));
    }

    /// The damage the shape-preserving path replaces: survivors used to be
    /// trimmed and joined with spaces, flattening a listing onto one line with
    /// its line-number prefixes still attached, under a banner calling it a
    /// summary.
    #[test]
    fn a_listing_keeps_its_lines_indentation_and_named_gap() {
        let content = numbered_listing(400);
        let reduced = elide_by_lines(&content, 4).expect("a 400-line listing reduces");

        assert!(reduced.len() < content.len());
        assert!(
            reduced.lines().count() > 10,
            "whole lines survive as lines, not flattened onto one: {}",
            reduced.lines().count()
        );
        assert!(
            reduced.contains("lines elided"),
            "the gap must name itself: {reduced}"
        );
        assert!(
            reduced.contains("VERBATIM"),
            "and must say the survivors are literal: {reduced}"
        );
        // Kept runs are contiguous in the listing's own numbering.
        let kept: Vec<u64> = reduced
            .lines()
            .filter_map(leading_line_number)
            .collect();
        assert!(kept.contains(&1), "the head starts at the beginning");
        assert!(kept.contains(&400), "the tail ends at the end");
        let head_run = kept
            .iter()
            .take_while(|&&n| n <= 200)
            .copied()
            .collect::<Vec<_>>();
        assert!(
            head_run.windows(2).all(|w| w[1] == w[0] + 1),
            "the head run is contiguous: {head_run:?}"
        );
    }

    #[test]
    fn leading_line_numbers_are_recognised_only_when_separated() {
        assert_eq!(leading_line_number("42\tcontent"), Some(42));
        assert_eq!(leading_line_number("  7: content"), Some(7));
        assert_eq!(leading_line_number("103 | content"), Some(103));
        assert_eq!(leading_line_number("2026-08-20 an entry"), None);
        assert_eq!(leading_line_number("no number here"), None);
    }

    #[test]
    fn elision_refuses_when_there_is_nothing_to_gain() {
        // Everything fits inside the target: nothing is elided, so nothing
        // is claimed to be.
        assert!(elide_by_lines("a\nb\nc\nd\n", 4).is_none());
    }
}
