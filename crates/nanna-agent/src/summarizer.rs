
use crate::chunker::analyze_content;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Configuration for the summarizer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SummarizerConfig {
    /// URL of Ollama instance for local summarization.
    pub ollama_url: Option<String>,
    /// Minimum content length (bytes) before summarization is triggered.
    pub threshold: usize,
    /// Maximum summary length (tokens).
    pub max_summary_tokens: usize,
}

impl Default for SummarizerConfig {
    fn default() -> Self {
        Self {
            ollama_url: None,
            threshold: 50_000,
            max_summary_tokens: 1000,
        }
    }
}

/// Summarizes long content using local or remote LLMs.
///
/// Breaks content into chunks, deduplicates against known content,
/// and generates summaries to reduce context window usage.
pub struct Summarizer {
    config: SummarizerConfig,
    known_hashes: HashSet<u64>,
}

impl Summarizer {
    /// Create a new summarizer with the given config.
    pub fn new(config: SummarizerConfig) -> Self {
        Self {
            config,
            known_hashes: HashSet::new(),
        }
    }

    /// Check if content should be summarized based on length threshold.
    pub fn should_summarize(&self, content: &str) -> bool {
        content.len() > self.config.threshold
    }

    /// Analyze content for redundancy.
    ///
    /// Returns analysis of novel vs. redundant chunks.
    /// Used to skip processing previously-seen content.
    pub fn analyze_redundancy(&self, content: &str) -> crate::chunker::DeduplicationAnalysis {
        analyze_content(content, &self.known_hashes)
    }

    /// Register content hashes as known (already processed).
    ///
    /// Prevents re-summarization of identical or similar content.
    pub fn register_hashes(&mut self, hashes: HashSet<u64>) {
        self.known_hashes.extend(hashes);
    }

    /// Get the current configuration.
    pub fn config(&self) -> &SummarizerConfig {
        &self.config
    }

    /// Update configuration.
    pub fn set_config(&mut self, config: SummarizerConfig) {
        self.config = config;
    }

    /// Estimate the token count for content.
    ///
    /// Rough heuristic: ~4 characters per token.
    pub fn estimate_tokens(&self, content: &str) -> usize {
        (content.len() + 3) / 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_summarizer_creation() {
        let config = SummarizerConfig::default();
        let summarizer = Summarizer::new(config);
        assert_eq!(summarizer.config.threshold, 50_000);
    }

    #[test]
    fn test_should_summarize() {
        let config = SummarizerConfig {
            threshold: 1000,
            ..Default::default()
        };
        let summarizer = Summarizer::new(config);

        let short = "short text";
        let long = "a".repeat(2000);

        assert!(!summarizer.should_summarize(short));
        assert!(summarizer.should_summarize(&long));
    }

    #[test]
    fn test_register_hashes() {
        let config = SummarizerConfig::default();
        let mut summarizer = Summarizer::new(config);

        let mut hashes = HashSet::new();
        hashes.insert(12345);
        hashes.insert(67890);

        summarizer.register_hashes(hashes);
        assert_eq!(summarizer.known_hashes.len(), 2);
    }

    #[test]
    fn test_estimate_tokens() {
        let config = SummarizerConfig::default();
        let summarizer = Summarizer::new(config);

        let content = "This is a test."; // ~15 chars
        let tokens = summarizer.estimate_tokens(content);
        assert!(tokens > 0);
    }
}
