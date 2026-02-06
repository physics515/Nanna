//! Context summarization service
//!
//! Summarizes large content blocks using a configurable model (e.g., local Ollama).
//! This allows using a cheap/fast model to condense content before sending to
//! the main (expensive) model.
//!
//! For very large content, uses hierarchical summarization:
//! - Splits content into manageable chunks based on model context window
//! - Summarizes each chunk
//! - If combined summaries are still too large, recursively summarizes again
//! - Limits API calls per level to prevent timeouts
//!
//! Includes an in-memory cache to avoid re-summarizing identical content.

use futures::future::join_all;
use nanna_llm::{CompletionRequest, LlmClient, Message, RequestBuilder};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Maximum chunks per summarization level (limits API calls)
const MAX_CHUNKS_PER_LEVEL: usize = 20;

/// Maximum recursion depth for hierarchical summarization
const MAX_SUMMARIZATION_DEPTH: usize = 5;

/// Maximum number of cached summaries (LRU eviction)
const MAX_CACHE_ENTRIES: usize = 100;

/// A cached summary entry
#[derive(Clone, Debug)]
pub struct SummaryCacheEntry {
    /// The summarized text
    pub summary: String,
    /// Timestamp when this entry was created
    pub created_at: std::time::Instant,
}

/// Type alias for the summary cache
pub type SummaryCache = Arc<RwLock<HashMap<u64, SummaryCacheEntry>>>;

/// Create a new empty summary cache that can be shared across summarizers
pub fn new_summary_cache() -> SummaryCache {
    Arc::new(RwLock::new(HashMap::new()))
}

/// Compute a hash of content for cache key
fn content_hash(content: &str, context_hint: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    context_hint.hash(&mut hasher);
    hasher.finish()
}

/// Configuration for the summarizer
#[derive(Debug, Clone)]
pub struct SummarizerConfig {
    /// Model to use for summarization (e.g., "llama3.2", "mistral")
    pub model: String,
    /// Maximum chars per chunk to summarize (based on model context window)
    pub chunk_size: usize,
    /// Target output size (chars) for each chunk summary
    pub target_summary_size: usize,
    /// Overlap between chunks to preserve context
    pub chunk_overlap: usize,
    /// Context limit for Ollama (num_ctx). If set, limits the context window
    /// to reduce VRAM usage, allowing more model layers on GPU.
    /// Recommended: half of the model's reported context window.
    pub context_limit: Option<u32>,
}

impl Default for SummarizerConfig {
    fn default() -> Self {
        Self {
            model: "llama3.2".to_string(),
            chunk_size: 8000,        // ~2000 tokens per chunk
            target_summary_size: 2000, // Target ~500 tokens per summary
            chunk_overlap: 500,       // Small overlap for context
            context_limit: Some(4096), // Default to 4K context to fit in VRAM
        }
    }
}

/// Summarizes large content for context management
pub struct Summarizer {
    client: LlmClient,
    config: SummarizerConfig,
    /// Cache of summaries keyed by content hash
    cache: Arc<RwLock<HashMap<u64, SummaryCacheEntry>>>,
}

impl Summarizer {
    /// Create a new summarizer with an Ollama client
    pub fn ollama(base_url: &str, config: SummarizerConfig) -> Self {
        Self {
            client: LlmClient::ollama(base_url),
            config,
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new summarizer with any LLM client
    pub fn new(client: LlmClient, config: SummarizerConfig) -> Self {
        Self {
            client,
            config,
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a summarizer with a shared cache (for reuse across requests)
    pub fn with_shared_cache(
        client: LlmClient,
        config: SummarizerConfig,
        cache: Arc<RwLock<HashMap<u64, SummaryCacheEntry>>>,
    ) -> Self {
        Self {
            client,
            config,
            cache,
        }
    }

    /// Get the cache for sharing with other summarizers
    pub fn cache(&self) -> Arc<RwLock<HashMap<u64, SummaryCacheEntry>>> {
        self.cache.clone()
    }

    /// Clear the cache
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
        info!("Summarization cache cleared");
    }

    /// Get cache statistics
    pub async fn cache_stats(&self) -> (usize, usize) {
        let cache = self.cache.read().await;
        let total_size: usize = cache.values().map(|e| e.summary.len()).sum();
        (cache.len(), total_size)
    }

    /// Summarize a large piece of content
    ///
    /// If content is small enough, returns it unchanged.
    /// For large content, uses hierarchical summarization:
    /// - Chunks content into model-sized pieces (max MAX_CHUNKS_PER_LEVEL)
    /// - Summarizes each chunk
    /// - If combined result is still large, recursively summarizes
    ///
    /// Results are cached by content hash to avoid re-summarizing identical content.
    pub async fn summarize(&self, content: &str, context_hint: &str) -> Result<String, String> {
        // Check cache first
        let hash = content_hash(content, context_hint);
        {
            let cache = self.cache.read().await;
            if let Some(entry) = cache.get(&hash) {
                info!(
                    hash = hash,
                    age_secs = entry.created_at.elapsed().as_secs(),
                    summary_len = entry.summary.len(),
                    "Cache hit for summarization"
                );
                return Ok(entry.summary.clone());
            }
        }

        // Not in cache, perform summarization
        let result = self.summarize_recursive(content, context_hint, 0).await?;

        // Store in cache
        {
            let mut cache = self.cache.write().await;

            // Evict oldest entries if cache is full
            if cache.len() >= MAX_CACHE_ENTRIES {
                // Find and remove the oldest entry
                if let Some(oldest_key) = cache
                    .iter()
                    .min_by_key(|(_, e)| e.created_at)
                    .map(|(k, _)| *k)
                {
                    cache.remove(&oldest_key);
                    debug!(evicted_hash = oldest_key, "Evicted oldest cache entry");
                }
            }

            cache.insert(
                hash,
                SummaryCacheEntry {
                    summary: result.clone(),
                    created_at: std::time::Instant::now(),
                },
            );
            info!(
                hash = hash,
                cache_size = cache.len(),
                summary_len = result.len(),
                "Cached summarization result"
            );
        }

        Ok(result)
    }

    /// Recursive hierarchical summarization
    fn summarize_recursive<'a>(
        &'a self,
        content: &'a str,
        context_hint: &'a str,
        depth: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'a>> {
        Box::pin(async move {
        // If content is already small, return as-is
        if content.len() <= self.config.target_summary_size * 2 {
            return Ok(content.to_string());
        }

        // Prevent infinite recursion
        if depth >= MAX_SUMMARIZATION_DEPTH {
            warn!(
                depth = depth,
                content_len = content.len(),
                "Max summarization depth reached, truncating"
            );
            return Ok(content.chars().take(self.config.target_summary_size * 2).collect());
        }

        // Calculate chunk size to stay within MAX_CHUNKS_PER_LEVEL
        let naive_chunks = (content.len() + self.config.chunk_size - 1) / self.config.chunk_size;
        let effective_chunk_size = if naive_chunks > MAX_CHUNKS_PER_LEVEL {
            // Need larger chunks to stay within limit
            (content.len() + MAX_CHUNKS_PER_LEVEL - 1) / MAX_CHUNKS_PER_LEVEL
        } else {
            self.config.chunk_size
        };

        info!(
            content_len = content.len(),
            chunk_size = effective_chunk_size,
            depth = depth,
            "Summarizing large content (hierarchical)"
        );

        // Split into chunks with the effective size
        let chunks = self.chunk_content_with_size(content, effective_chunk_size);
        debug!(num_chunks = chunks.len(), depth = depth, "Split content into chunks");

        // Summarize each chunk in parallel for speed
        let chunk_count = chunks.len();
        let futures: Vec<_> = chunks
            .iter()
            .enumerate()
            .map(|(i, chunk)| {
                self.summarize_chunk(chunk, context_hint, i + 1, chunk_count)
            })
            .collect();

        let results = join_all(futures).await;

        // Collect results, preserving order
        let mut summaries = Vec::new();
        for (i, result) in results.into_iter().enumerate() {
            match result {
                Ok(summary) => summaries.push(summary),
                Err(e) => {
                    warn!(chunk = i + 1, error = %e, "Chunk summarization failed, using truncated original");
                    // Fall back to truncated original for failed chunks
                    let chunk = &chunks[i];
                    let truncated = chunk.chars().take(self.config.target_summary_size / chunk_count.max(1)).collect();
                    summaries.push(truncated);
                }
            }
        }

        // Combine summaries
        let combined = if summaries.len() == 1 {
            summaries.into_iter().next().unwrap()
        } else {
            summaries
                .into_iter()
                .enumerate()
                .map(|(i, s)| format!("[Part {}]\n{}", i + 1, s))
                .collect::<Vec<_>>()
                .join("\n\n")
        };

        info!(
            original_len = content.len(),
            summary_len = combined.len(),
            compression_ratio = format!("{:.1}x", content.len() as f64 / combined.len().max(1) as f64),
            depth = depth,
            "Level {} summarization complete",
            depth
        );

        // If combined is still too large, recursively summarize
        if combined.len() > self.config.target_summary_size * 4 {
            info!(
                combined_len = combined.len(),
                target = self.config.target_summary_size * 4,
                "Combined summaries still large, recursing to level {}",
                depth + 1
            );
            return self
                .summarize_recursive(&combined, context_hint, depth + 1)
                .await;
        }

        Ok(combined)
        }) // Close Box::pin(async move {
    }

    /// Summarize a tool result specifically
    pub async fn summarize_tool_result(
        &self,
        tool_name: &str,
        content: &str,
    ) -> Result<String, String> {
        let context = format!("This is the output from a tool called '{}'. Preserve key information, file paths, error messages, and important data.", tool_name);
        self.summarize(content, &context).await
    }

    /// Split content into overlapping chunks using configured chunk_size
    fn chunk_content(&self, content: &str) -> Vec<String> {
        self.chunk_content_with_size(content, self.config.chunk_size)
    }

    /// Split content into overlapping chunks with a specified chunk size
    fn chunk_content_with_size(&self, content: &str, chunk_size: usize) -> Vec<String> {
        let mut chunks = Vec::new();
        let chars: Vec<char> = content.chars().collect();
        let mut start = 0;

        // Scale overlap proportionally to chunk size
        let overlap = (self.config.chunk_overlap * chunk_size) / self.config.chunk_size;
        let overlap = overlap.max(100).min(chunk_size / 4); // Between 100 and 25% of chunk

        while start < chars.len() {
            let end = (start + chunk_size).min(chars.len());
            let chunk: String = chars[start..end].iter().collect();
            chunks.push(chunk);

            // Move start, accounting for overlap
            if end >= chars.len() {
                break;
            }
            start = end.saturating_sub(overlap);
        }

        chunks
    }

    /// Summarize a single chunk
    async fn summarize_chunk(
        &self,
        chunk: &str,
        context_hint: &str,
        part_num: usize,
        total_parts: usize,
    ) -> Result<String, String> {
        let prompt = format!(
            r#"Summarize the following content concisely. {context_hint}

Focus on:
- Key facts, names, paths, and identifiers
- Important data, numbers, and error messages
- Main conclusions or results
- Preserve technical details that would be needed to understand the content

This is part {part_num} of {total_parts}.

Content to summarize:
---
{chunk}
---

Provide a concise summary (target: ~{target} characters):"#,
            context_hint = context_hint,
            part_num = part_num,
            total_parts = total_parts,
            chunk = chunk,
            target = self.config.target_summary_size,
        );

        let mut request = CompletionRequest::default()
            .with_model(&self.config.model)
            .with_message(Message::user(&prompt))
            .with_max_tokens(1024)
            .with_temperature(0.3); // Low temperature for factual summarization

        // Apply context limit for Ollama to reduce VRAM usage
        if let Some(limit) = self.config.context_limit {
            request = request.with_context_limit(limit);
        }

        match self.client.complete(&request).await {
            Ok(summary) => Ok(summary.trim().to_string()),
            Err(e) => {
                warn!(error = %e, "Summarization failed, returning truncated original");
                // Fallback: return truncated original
                Ok(chunk.chars().take(self.config.target_summary_size).collect())
            }
        }
    }
}

/// Summarize content if it exceeds a threshold
///
/// This is a convenience function for use in context management.
pub async fn summarize_if_large(
    client: &LlmClient,
    model: &str,
    content: &str,
    threshold: usize,
    context_hint: &str,
) -> Result<String, String> {
    if content.len() <= threshold {
        return Ok(content.to_string());
    }

    let config = SummarizerConfig {
        model: model.to_string(),
        ..Default::default()
    };
    let summarizer = Summarizer::new(client.clone(), config);
    summarizer.summarize(content, context_hint).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_content() {
        let config = SummarizerConfig {
            chunk_size: 100,
            chunk_overlap: 20,
            ..Default::default()
        };
        let summarizer = Summarizer::new(LlmClient::ollama_default(), config);

        let content = "a".repeat(250);
        let chunks = summarizer.chunk_content(&content);

        assert!(chunks.len() >= 2);
        assert!(chunks[0].len() <= 100);
    }

    #[test]
    fn test_small_content_unchanged() {
        let config = SummarizerConfig::default();
        let small_content = "This is small content";

        // Small content should be returned as-is (sync check only)
        assert!(small_content.len() <= config.target_summary_size * 2);
    }
}
