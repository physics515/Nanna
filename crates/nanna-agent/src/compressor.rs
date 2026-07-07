//! LLMLingua-style prompt compression using local models.
//!
//! Uses a small local model (via Ollama) to score sentences by information density
//! and drops low-information sentences to achieve a target compression ratio.
//!
//! Unlike true LLMLingua (per-token perplexity), this uses a sentence-level approach
//! that is more practical with Ollama's API:
//! 1. Split content into sentences
//! 2. Ask the model to score each sentence's importance (0-10)
//! 3. Drop sentences below the threshold to hit the target ratio
//!
//! This is applied selectively to tool outputs and old conversation turns,
//! never to system prompts or recent messages.

use nanna_llm::{AnthropicMessage, AnthropicRequest, ContentBlock, LlmClient};
use tracing::{debug, info};

/// Configuration for LLMLingua-style compression.
#[derive(Debug, Clone)]
pub struct CompressionConfig {
    /// Target compression ratio (e.g., 4 = compress to 1/4 of original size).
    pub ratio: usize,
    /// Minimum content length (chars) to consider for compression.
    pub min_content_length: usize,
    /// Model to use for compression scoring (e.g., "phi3:mini").
    pub model: String,
    /// Ollama URL.
    pub ollama_url: String,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            ratio: 4,
            min_content_length: 2000,
            model: String::new(),
            ollama_url: "http://localhost:11434".to_string(),
        }
    }
}

/// Compress text by scoring sentences and dropping low-importance ones.
///
/// Returns the compressed text, or None if compression failed or wasn't worthwhile.
pub async fn compress_text(
    client: &LlmClient,
    model: &str,
    content: &str,
    target_ratio: usize,
) -> Option<String> {
    if content.len() < 500 {
        return None; // Too short to compress
    }

    // Split into sentences (simple heuristic)
    let sentences: Vec<&str> = split_sentences(content);
    if sentences.len() < 4 {
        return None; // Not enough sentences to compress
    }

    let target_count = sentences.len() / target_ratio.max(1);
    if target_count < 2 {
        return None;
    }

    // Build a scoring prompt — ask the model to rate each sentence
    let numbered: String = sentences.iter().enumerate()
        .map(|(i, s)| format!("{}: {}", i + 1, s.trim()))
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = format!(
        "Rate each numbered sentence by information importance (1-10). \
         1=filler/boilerplate, 10=critical information. \
         Output ONLY numbers, one per line, in order.\n\n{numbered}"
    );

    let request = AnthropicRequest {
        model: model.to_string(),
        messages: vec![AnthropicMessage::user_text(prompt)],
        max_tokens: 256,
        temperature: Some(0.1),
        system: Some("You are an information density scorer. Output ONLY one number (1-10) per line, nothing else.".to_string()),
        tools: None,
        stream: None,
        thinking: None,
        cache_control: None,
    };

    let response = match client.complete_anthropic(&request).await {
        Ok(r) => r,
        Err(e) => {
            debug!(error = %e, "Compression scoring failed");
            return None;
        }
    };

    let scores_text: String = response.content.iter().filter_map(|b| {
        if let ContentBlock::Text { text } = b { Some(text.as_str()) } else { None }
    }).collect::<Vec<_>>().join("\n");

    // Parse scores
    let scores: Vec<u8> = scores_text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            // Handle formats like "1: 7" or just "7"
            let num_str = if let Some((_prefix, score)) = trimmed.split_once(':') {
                score.trim()
            } else {
                trimmed
            };
            num_str.parse::<u8>().ok()
        })
        .collect();

    if scores.len() != sentences.len() {
        debug!(
            expected = sentences.len(),
            got = scores.len(),
            "Score count mismatch, falling back to simple truncation"
        );
        // Fallback: keep first and last portions
        return fallback_compress(content, target_ratio);
    }

    // Sort sentences by score and keep the top ones (preserving original order)
    let mut indexed_scores: Vec<(usize, u8)> = scores.iter().copied().enumerate().collect();
    indexed_scores.sort_by(|a, b| b.1.cmp(&a.1)); // Sort by score descending

    let keep_count = target_count.max(2);
    let mut keep_indices: Vec<usize> = indexed_scores[..keep_count.min(indexed_scores.len())]
        .iter()
        .map(|(i, _)| *i)
        .collect();
    keep_indices.sort_unstable(); // Restore original order

    let compressed: String = keep_indices.iter()
        .map(|&i| sentences[i].trim())
        .collect::<Vec<_>>()
        .join(" ");

    let original_len = content.len();
    let compressed_len = compressed.len();
    let actual_ratio = if compressed_len > 0 { original_len / compressed_len } else { 0 };

    info!(
        original_chars = original_len,
        compressed_chars = compressed_len,
        actual_ratio = actual_ratio,
        sentences_kept = keep_count,
        sentences_total = sentences.len(),
        "🗜️ LLMLingua compression: {original_len} → {compressed_len} chars ({actual_ratio}x)"
    );

    Some(compressed)
}

/// Fallback compression: keep first 25% and last 25% of content.
fn fallback_compress(content: &str, ratio: usize) -> Option<String> {
    let target_len = content.len() / ratio.max(1);
    let half = target_len / 2;
    if half < 100 {
        return None;
    }
    // Snap to char boundaries to avoid panicking on multi-byte UTF-8 (e.g. tree-drawing chars)
    let start_end = content.floor_char_boundary(half);
    // For the tail portion, floor to previous boundary then advance past any split char
    let tail_offset = content.len().saturating_sub(half);
    let end_start = if content.is_char_boundary(tail_offset) {
        tail_offset
    } else {
        // Find the next char boundary after tail_offset
        (tail_offset..content.len())
            .find(|&i| content.is_char_boundary(i))
            .unwrap_or(content.len())
    };
    let start = &content[..start_end];
    let end = &content[end_start..];
    Some(format!("{start}\n[...compressed...]\n{end}"))
}

/// Split text into sentences using simple heuristics.
fn split_sentences(text: &str) -> Vec<&str> {
    let mut sentences = Vec::new();
    let mut start = 0;

    for (i, c) in text.char_indices() {
        if (c == '.' || c == '!' || c == '?' || c == '\n') && i > start + 10 {
            // Check for sentence-ending pattern (not abbreviations)
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

    // Add remaining text
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
        assert!(sentences.len() >= 3, "Expected at least 3 sentences, got {}", sentences.len());
    }

    #[test]
    fn test_split_sentences_newlines() {
        let text = "Line one content here\nLine two content here\nLine three content here";
        let sentences = split_sentences(text);
        assert!(sentences.len() >= 2, "Expected at least 2 sentences, got {}", sentences.len());
    }

    #[test]
    fn test_fallback_compress() {
        let text = "A".repeat(1000);
        let result = fallback_compress(&text, 4);
        assert!(result.is_some());
        let compressed = result.unwrap();
        assert!(compressed.len() < text.len());
    }
}
