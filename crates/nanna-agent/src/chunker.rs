//! Content-Defined Chunking (CDC) for deduplication
//!
//! Uses a rolling hash to find deterministic chunk boundaries based on content.
//! This ensures the same content produces the same chunks regardless of how
//! it's split or where it appears in a larger document.
//!
//! Based on the FastCDC algorithm for efficient, content-aware chunking.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Minimum chunk size (bytes) - won't split smaller than this
const MIN_CHUNK_SIZE: usize = 2048; // 2KB

/// Target average chunk size (bytes)
const AVG_CHUNK_SIZE: usize = 8192; // 8KB

/// Maximum chunk size (bytes) - force split at this point
const MAX_CHUNK_SIZE: usize = 32768; // 32KB

/// Mask for determining chunk boundaries (affects average size)
/// Lower bits = larger average chunks
const CHUNK_MASK: u64 = 0x0000_0FFF; // ~4KB average with good distribution

/// Rolling hash window size
const WINDOW_SIZE: usize = 48;

/// Gear table for rolling hash (pre-computed random values)
/// This provides good distribution for the rolling hash
const GEAR: [u64; 256] = gear_table();

/// Generate gear table at compile time
const fn gear_table() -> [u64; 256] {
    let mut table = [0u64; 256];
    let mut i = 0;
    // Simple PRNG for deterministic table generation
    let mut state: u64 = 0x123456789ABCDEF0;
    while i < 256 {
        // xorshift64
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        table[i] = state;
        i += 1;
    }
    table
}

/// A content-defined chunk with its hash
#[derive(Debug, Clone)]
pub struct Chunk {
    /// Hash of this chunk's content
    pub hash: u64,
    /// Start offset in original content
    pub start: usize,
    /// Length of chunk
    pub len: usize,
}

/// Content-defined chunker using Gear rolling hash
pub struct ContentChunker<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> ContentChunker<'a> {
    /// Create a new chunker for the given data
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    /// Create a chunker from a string
    pub fn from_str(s: &'a str) -> Self {
        Self::new(s.as_bytes())
    }

    /// Find the next chunk boundary using Gear rolling hash
    fn find_boundary(&self, start: usize) -> usize {
        let end = self.data.len();

        // Don't go past max chunk size
        let max_pos = (start + MAX_CHUNK_SIZE).min(end);

        // Start looking for boundary after minimum chunk size
        let search_start = (start + MIN_CHUNK_SIZE).min(end);

        if search_start >= max_pos {
            return max_pos;
        }

        // Gear rolling hash
        let mut hash: u64 = 0;

        for pos in search_start..max_pos {
            // Update rolling hash with gear table
            hash = (hash << 1).wrapping_add(GEAR[self.data[pos] as usize]);

            // Check if we hit a boundary (low bits match mask pattern)
            if (hash & CHUNK_MASK) == 0 {
                return pos + 1;
            }
        }

        // No boundary found, use max size
        max_pos
    }

    /// Compute hash for a slice of data
    fn hash_slice(data: &[u8]) -> u64 {
        let mut hasher = DefaultHasher::new();
        data.hash(&mut hasher);
        hasher.finish()
    }
}

impl<'a> Iterator for ContentChunker<'a> {
    type Item = Chunk;

    fn next(&mut self) -> Option<Self::Item> {
        if self.position >= self.data.len() {
            return None;
        }

        let start = self.position;
        let boundary = self.find_boundary(start);
        let chunk_data = &self.data[start..boundary];

        self.position = boundary;

        Some(Chunk {
            hash: Self::hash_slice(chunk_data),
            start,
            len: chunk_data.len(),
        })
    }
}

/// Chunk content and return hashes of all chunks
pub fn chunk_and_hash(content: &str) -> Vec<u64> {
    ContentChunker::from_str(content)
        .map(|c| c.hash)
        .collect()
}

/// Check what percentage of content chunks are already known
pub fn dedup_coverage(content: &str, known_hashes: &std::collections::HashSet<u64>) -> f32 {
    if content.is_empty() {
        return 0.0;
    }

    let chunks: Vec<Chunk> = ContentChunker::from_str(content).collect();
    if chunks.is_empty() {
        return 0.0;
    }

    let total_bytes: usize = chunks.iter().map(|c| c.len).sum();
    let known_bytes: usize = chunks
        .iter()
        .filter(|c| known_hashes.contains(&c.hash))
        .map(|c| c.len)
        .sum();

    known_bytes as f32 / total_bytes as f32
}

/// Result of deduplicating content
#[derive(Debug)]
pub struct DedupResult {
    /// Chunks that are new (not in known_hashes)
    pub new_chunks: Vec<Chunk>,
    /// Chunks that were already known
    pub known_chunks: Vec<Chunk>,
    /// Total bytes in new chunks
    pub new_bytes: usize,
    /// Total bytes in known chunks
    pub known_bytes: usize,
}

/// Analyze content for deduplication
pub fn analyze_content(content: &str, known_hashes: &std::collections::HashSet<u64>) -> DedupResult {
    let mut new_chunks = Vec::new();
    let mut known_chunks = Vec::new();
    let mut new_bytes = 0;
    let mut known_bytes = 0;

    for chunk in ContentChunker::from_str(content) {
        if known_hashes.contains(&chunk.hash) {
            known_bytes += chunk.len;
            known_chunks.push(chunk);
        } else {
            new_bytes += chunk.len;
            new_chunks.push(chunk);
        }
    }

    DedupResult {
        new_chunks,
        known_chunks,
        new_bytes,
        known_bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunking_deterministic() {
        let content = "a".repeat(50000);
        let hashes1 = chunk_and_hash(&content);
        let hashes2 = chunk_and_hash(&content);
        assert_eq!(hashes1, hashes2, "Same content should produce same hashes");
    }

    #[test]
    fn test_chunking_different_content() {
        let content1 = "a".repeat(50000);
        let content2 = "b".repeat(50000);
        let hashes1 = chunk_and_hash(&content1);
        let hashes2 = chunk_and_hash(&content2);
        assert_ne!(hashes1, hashes2, "Different content should produce different hashes");
    }

    #[test]
    fn test_partial_overlap() {
        // Create two contents with shared middle section
        let shared = "SHARED_CONTENT".repeat(1000);
        let content1 = format!("PREFIX_A{}", shared);
        let content2 = format!("PREFIX_B{}", shared);

        let hashes1: std::collections::HashSet<_> = chunk_and_hash(&content1).into_iter().collect();
        let hashes2: std::collections::HashSet<_> = chunk_and_hash(&content2).into_iter().collect();

        // Should have some overlap in the shared section
        let overlap: Vec<_> = hashes1.intersection(&hashes2).collect();
        assert!(!overlap.is_empty(), "Should detect overlapping chunks");
    }

    #[test]
    fn test_chunk_sizes() {
        let content = "x".repeat(100000);
        let chunks: Vec<_> = ContentChunker::from_str(&content).collect();

        for chunk in &chunks[..chunks.len()-1] { // Last chunk can be smaller
            assert!(chunk.len >= MIN_CHUNK_SIZE, "Chunk too small: {}", chunk.len);
            assert!(chunk.len <= MAX_CHUNK_SIZE, "Chunk too large: {}", chunk.len);
        }
    }

    #[test]
    fn test_dedup_coverage() {
        let content = "test content for deduplication".repeat(100);
        let hashes: std::collections::HashSet<_> = chunk_and_hash(&content).into_iter().collect();

        // Same content should be 100% covered
        let coverage = dedup_coverage(&content, &hashes);
        assert!((coverage - 1.0).abs() < 0.01, "Same content should be ~100% covered");

        // Different content should have low coverage
        let different = "completely different content".repeat(100);
        let coverage2 = dedup_coverage(&different, &hashes);
        assert!(coverage2 < 0.5, "Different content should have low coverage");
    }
}
