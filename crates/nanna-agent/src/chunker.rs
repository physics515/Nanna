#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! Content chunking with deduplication.
//!
//! Splits large documents into semantic chunks with hash-based deduplication.
//! Used by the summarizer to reduce redundant processing.

use std::collections::HashSet;

/// Chunk size target (bytes). Actual chunks may vary based on semantic boundaries.
/// Used in `Chunk::split_on_boundaries` to balance granularity vs. overhead.
const TARGET_CHUNK_SIZE: usize = 8192; // 8KB

/// Represents a contiguous section of text with metadata.
#[derive(Clone, Debug)]
pub struct Chunk {
    /// FNV-1a hash of the content for deduplication.
    pub hash: u64,
    /// Token count estimate (for LLM planning).
    pub token_count: usize,
    /// Byte length of the original content.
    pub byte_len: usize,
}

impl Chunk {
    /// Create a new chunk from content.
    pub fn new(content: String, _offset: usize) -> Self {
        let hash = Self::hash_content(&content);
        let token_count = Self::estimate_tokens(&content);
        let byte_len = content.len();
        Self {
            hash,
            token_count,
            byte_len,
        }
    }

    /// Hash content using FNV-1a for fast deduplication.
    fn hash_content(content: &str) -> u64 {
        const FNV_PRIME: u64 = 1_099_511_628_211;
        const FNV_OFFSET_BASIS: u64 = 14_695_981_039_346_656_037;

        content.bytes().fold(FNV_OFFSET_BASIS, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME)
        })
    }

    /// Estimate token count (family-aware heuristic via nanna-llm).
    fn estimate_tokens(content: &str) -> usize {
        nanna_llm::estimate_tokens(content)
    }

    /// Split content on semantic boundaries (sentences, paragraphs).
    /// Returns chunks that respect `TARGET_CHUNK_SIZE` and `MAX_TOKENS_PER_CHUNK`.
    pub fn split_on_boundaries(content: &str) -> Vec<Self> {
        let mut chunks = Vec::new();
        let mut current = String::new();
        let mut offset = 0;

        for sentence in content.split_terminator(|c: char| c == '.' || c == '\n') {
            let sentence_with_term = if sentence.ends_with('\n') {
                sentence.to_string()
            } else {
                format!("{sentence}.")
            };

            if current.len() + sentence_with_term.len() > TARGET_CHUNK_SIZE {
                if !current.is_empty() {
                    chunks.push(Self::new(current.clone(), offset));
                    offset += current.len();
                    current.clear();
                }
            }

            current.push_str(&sentence_with_term);
        }

        if !current.is_empty() {
            chunks.push(Self::new(current, offset));
        }

        chunks
    }
}

/// Result of content deduplication analysis.
/// Identifies which chunks are novel vs. redundant.
#[derive(Clone, Debug)]
pub struct DeduplicationAnalysis {
    /// Hashes of novel chunks (not seen before).
    pub novel_hashes: HashSet<u64>,
    /// Count of redundant chunks filtered out.
    pub redundant_count: usize,
    /// Total estimated tokens across all novel chunks.
    pub novel_token_count: usize,
    /// Total bytes across all novel chunks.
    pub novel_byte_count: usize,
}

/// Analyze content for redundancy against known hashes.
///
/// Used by the summarizer to skip processing of previously-seen content.
/// Returns both novel hashes and redundancy statistics.
pub fn analyze_content(
    content: &str,
    known_hashes: &HashSet<u64>,
) -> DeduplicationAnalysis {
    let chunks = Chunk::split_on_boundaries(content);
    let mut novel_hashes = HashSet::new();
    let mut redundant_count = 0;
    let mut novel_token_count = 0;
    let mut novel_byte_count = 0;

    for chunk in chunks {
        if known_hashes.contains(&chunk.hash) {
            redundant_count += 1;
        } else {
            novel_hashes.insert(chunk.hash);
            novel_token_count += chunk.token_count;
            novel_byte_count += chunk.byte_len;
        }
    }

    DeduplicationAnalysis {
        novel_hashes,
        redundant_count,
        novel_token_count,
        novel_byte_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_creation() {
        let chunk = Chunk::new("Hello world".to_string(), 0);
        assert!(chunk.hash > 0);
        assert!(chunk.token_count > 0);
        assert_eq!(chunk.byte_len, 11);
    }

    #[test]
    fn test_split_on_boundaries() {
        let content = "First sentence. Second sentence. Third sentence.";
        let chunks = Chunk::split_on_boundaries(content);
        assert!(!chunks.is_empty());
        for chunk in &chunks {
            assert!(chunk.byte_len > 0);
        }
    }

    #[test]
    fn test_deduplication() {
        let content = "Test content here.";
        let chunk = Chunk::new(content.to_string(), 0);
        let mut known = HashSet::new();
        known.insert(chunk.hash);

        let analysis = analyze_content(content, &known);
        assert_eq!(analysis.redundant_count, 1);
        assert!(analysis.novel_hashes.is_empty());
    }
}
