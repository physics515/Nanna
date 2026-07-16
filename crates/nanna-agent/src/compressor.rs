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

use nanna_llm::{AnthropicMessage, AnthropicRequest, ContentBlock, LlmClient};
use tracing::{debug, info, warn};

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

    let model_cache = nanna_llm::ModelInfoCache::default_location();
    let model_info = client.get_model_info(model, model_cache.as_ref()).await;
    let max_tokens = u32::try_from(model_info.max_output_tokens.min(256)).unwrap_or(u32::MAX);
    let request = AnthropicRequest {
        model: model.to_string(),
        messages: vec![AnthropicMessage::user_text(prompt)],
        max_tokens,
        temperature: Some(0.1),
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
            "Score count mismatch, falling back to head/tail truncation"
        );
        return fallback_compress(content, config.ratio);
    }

    let keep_count = target_count.max(2).min(sentences.len());
    let keep_indices = select_keep_indices(&scores, keep_count);
    let compressed: String = keep_indices
        .iter()
        .map(|&i| sentences[i].trim())
        .collect::<Vec<_>>()
        .join(" ");

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

/// Fallback compression: keep first ~half and last ~half of the target length.
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
    Some(format!("{start}\n[...compressed...]\n{end}"))
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
        assert!(compressed.contains("[...compressed...]"));
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
}
