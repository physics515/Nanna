//! Memory Consolidation ("Dreaming")
//!
//! Periodic processing that mimics biological memory consolidation during sleep:
//! - Scores memories by a composite of similarity, recall frequency, importance, and age
//! - Clusters memories that should be merged together
//! - Compresses low-weight memories (summarization)
//! - Expands high-weight memories (enrichment)
//! - Merges related memories
//! - Respects a maximum compression ratio to avoid over-aggressive consolidation

use crate::{MemoryEntry, FsrsParameters, FsrsState};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Upper bound on how many memories may be folded into a single consolidation
/// cluster (and therefore a single summarization prompt). A degenerate weight
/// band of thousands of mutually-similar memories would otherwise collapse into
/// one cluster whose prompt overflows a small local model's context window
/// (P12). Over-cap members stay unassigned and re-cluster among themselves, so
/// no content is dropped — the band is just consolidated in several passes.
pub const DEFAULT_MAX_CLUSTER_MEMORIES: usize = 64;

/// Context window (tokens) assumed when the summarizer model is unknown.
///
/// The safe low end (a small 8k-token local model). Callers that know the model
/// should size the budget to it via
/// [`ConsolidationConfig::with_summarizer_context_window`].
pub const FALLBACK_SUMMARIZER_CONTEXT_WINDOW_TOKENS: usize = 8_192;

/// Fallback per-cluster content-byte budget when the summarizer model is unknown.
///
/// Derived from [`FALLBACK_SUMMARIZER_CONTEXT_WINDOW_TOKENS`] via
/// [`cluster_content_bytes_for_context`] — not an "8 GB tier" constant — so
/// `default()` and the model-aware path share one formula.
pub const DEFAULT_MAX_CLUSTER_CONTENT_BYTES: usize =
    cluster_content_bytes_for_context(FALLBACK_SUMMARIZER_CONTEXT_WINDOW_TOKENS);

/// Convert a summarizer model's context window (in tokens) into the per-cluster
/// content-byte budget, so the consolidation prompt built from a cluster always
/// fits that model's window.
///
/// Reserves headroom for the summarization instruction/framing and the generated
/// summary, then converts the remaining token budget to bytes at the token
/// estimator's **worst-case density**: `nanna_llm::estimate_tokens` counts every
/// non-ASCII char as one token, and the smallest non-ASCII UTF-8 char is 2 bytes,
/// so content can reach at most 0.5 tokens/byte. Budgeting **2 bytes per token**
/// therefore guarantees the concatenated content never exceeds the token budget
/// for any script (ASCII simply under-fills — safety over utilization).
#[must_use]
pub const fn cluster_content_bytes_for_context(context_window_tokens: usize) -> usize {
    /// Instruction + framing + per-memory `---` separators.
    const PROMPT_OVERHEAD_TOKENS: usize = 512;
    /// Room for the generated consolidated summary.
    const OUTPUT_RESERVE_TOKENS: usize = 1024;
    /// Worst-case bytes-per-token that cannot overflow the token budget.
    const SAFE_BYTES_PER_TOKEN: usize = 2;
    /// Never collapse to nothing on a tiny/misreported window.
    const MIN_CONTENT_BYTES: usize = 2_048;

    let content_tokens =
        context_window_tokens.saturating_sub(PROMPT_OVERHEAD_TOKENS + OUTPUT_RESERVE_TOKENS);
    let bytes = content_tokens * SAFE_BYTES_PER_TOKEN;
    if bytes > MIN_CONTENT_BYTES {
        bytes
    } else {
        MIN_CONTENT_BYTES
    }
}

const fn default_max_cluster_memories() -> usize {
    DEFAULT_MAX_CLUSTER_MEMORIES
}

const fn default_max_cluster_content_bytes() -> usize {
    DEFAULT_MAX_CLUSTER_CONTENT_BYTES
}

/// Consolidation configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationConfig {
    /// Minimum composite score to consider two memories for the same cluster (0.0-1.0).
    /// This blends similarity, recall overlap, importance, and age — not just cosine similarity.
    pub cluster_threshold: f32,
    /// Minimum cluster size to consolidate (singletons below this are skipped or expanded)
    pub min_cluster_size: usize,
    /// Maximum number of memories a single cluster may hold, bounding the size of
    /// the consolidation prompt handed to the (possibly small, local) summarizer.
    #[serde(default = "default_max_cluster_memories")]
    pub max_cluster_memories: usize,
    /// Maximum total `content` bytes a single cluster may hold, bounding the
    /// consolidation prompt to fit the summarizer model's context window. Defaults
    /// to the small-model fallback; size it to the real model with
    /// [`ConsolidationConfig::with_summarizer_context_window`].
    #[serde(default = "default_max_cluster_content_bytes")]
    pub max_cluster_content_bytes: usize,
    /// Maximum fraction of total memories that can be removed in a single run (0.0-1.0).
    /// E.g. 0.50 means at most 50% of memories can be merged away.
    pub max_compression_ratio: f32,
    /// Minimum number of memories to leave after consolidation (floor).
    /// Consolidation stops merging once the store would drop below this count.
    pub min_remaining_memories: usize,
    /// Weight thresholds for compression levels
    pub weight_thresholds: WeightThresholds,
    /// Weights for the composite clustering score
    pub clustering_weights: ClusteringWeights,
}

impl Default for ConsolidationConfig {
    fn default() -> Self {
        Self {
            cluster_threshold: 0.45,
            min_cluster_size: 2,
            max_cluster_memories: DEFAULT_MAX_CLUSTER_MEMORIES,
            max_cluster_content_bytes: DEFAULT_MAX_CLUSTER_CONTENT_BYTES,
            max_compression_ratio: 0.50,
            min_remaining_memories: 20,
            weight_thresholds: WeightThresholds::default(),
            clustering_weights: ClusteringWeights::default(),
        }
    }
}

impl ConsolidationConfig {
    /// Size [`Self::max_cluster_content_bytes`] to the summarizer model's context
    /// window (in tokens) so an automatically-built consolidation prompt always
    /// fits it — replacing the small-model fallback with the real budget. Leaves
    /// the member-count cap ([`Self::max_cluster_memories`], a coarse safety
    /// limit) untouched, since the byte budget is what maps to the context window.
    #[must_use]
    pub const fn with_summarizer_context_window(mut self, context_window_tokens: usize) -> Self {
        self.max_cluster_content_bytes = cluster_content_bytes_for_context(context_window_tokens);
        self
    }
}

/// Weights for the composite clustering score.
/// The final score is a weighted combination of these signals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusteringWeights {
    /// How much cosine similarity matters (semantic relatedness)
    pub similarity: f32,
    /// How much overlapping recall frequency matters (memories accessed together)
    pub recall_affinity: f32,
    /// How much similar importance levels matter (merge peers, not outliers)
    pub importance_proximity: f32,
    /// How much similar age matters (merge memories from the same era)
    pub age_proximity: f32,
}

impl Default for ClusteringWeights {
    fn default() -> Self {
        Self {
            similarity: 0.45,
            recall_affinity: 0.20,
            importance_proximity: 0.20,
            age_proximity: 0.15,
        }
    }
}

/// Weight thresholds that determine compression level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeightThresholds {
    /// Below this: compress to essence (1 word/sentence)
    pub essence: f32,
    /// Below this: moderate compression
    pub compressed: f32,
    /// Below this: standard detail
    pub standard: f32,
    /// Below this: full detail (no compression)
    pub detailed: f32,
    // Above detailed threshold: expand/research
}

impl Default for WeightThresholds {
    fn default() -> Self {
        Self {
            essence: 0.2,
            compressed: 0.5,
            standard: 0.8,
            detailed: 1.0,
        }
    }
}

/// Compression level for summarization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompressionLevel {
    /// Compress to absolute essence (1 word to 1 sentence)
    Essence,
    /// Moderate compression (key points only)
    Compressed,
    /// Standard detail (minor trimming)
    Standard,
    /// Keep full detail
    Detailed,
    /// Expand and enrich with more context
    Expand,
}

impl CompressionLevel {
    /// Determine compression level from weight
    #[must_use]
    pub fn from_weight(weight: f32, thresholds: &WeightThresholds) -> Self {
        if weight < thresholds.essence {
            Self::Essence
        } else if weight < thresholds.compressed {
            Self::Compressed
        } else if weight < thresholds.standard {
            Self::Standard
        } else if weight <= thresholds.detailed {
            Self::Detailed
        } else {
            Self::Expand
        }
    }

    /// Get the summarization prompt for this compression level
    #[must_use]
    pub fn summarization_prompt(&self) -> &'static str {
        match self {
            Self::Essence => {
                "Compress this memory to its absolute essence - a single word or very short phrase \
                that captures the core concept. Remove all detail, keep only the kernel of meaning."
            }
            Self::Compressed => {
                "Summarize this memory concisely. Keep only the key facts and main point. \
                Remove examples, elaboration, and secondary details. 1-2 sentences maximum."
            }
            Self::Standard => {
                "Lightly summarize this memory. Keep the main content but remove redundancy \
                and unnecessary verbosity. Preserve important details."
            }
            Self::Detailed => {
                "Keep this memory as-is. No compression needed."
            }
            Self::Expand => {
                "This is an important memory. If there are implicit connections, context, \
                or insights that could be made explicit, add them. Enrich but don't invent."
            }
        }
    }
}

/// A cluster of related memories to be consolidated
#[derive(Debug, Clone)]
pub struct MemoryCluster {
    /// Memories in this cluster
    pub memories: Vec<MemoryEntry>,
    /// Average embedding (centroid)
    pub centroid: Vec<f32>,
    /// Compression level for this cluster
    pub compression_level: CompressionLevel,
    /// Average weight of memories in cluster
    pub avg_weight: f32,
}

impl MemoryCluster {
    /// Create a new cluster from memories
    pub fn new(memories: Vec<MemoryEntry>, compression_level: CompressionLevel, fsrs_params: &FsrsParameters) -> Self {
        let centroid = Self::compute_centroid(&memories);
        let avg_weight = memories.iter()
            .map(|m| m.fsrs.weight(fsrs_params))
            .sum::<f32>() / memories.len().max(1) as f32;
        
        Self {
            memories,
            centroid,
            compression_level,
            avg_weight,
        }
    }

    /// Compute centroid (average) of embeddings
    fn compute_centroid(memories: &[MemoryEntry]) -> Vec<f32> {
        if memories.is_empty() {
            return Vec::new();
        }
        
        let dim = memories[0].embedding.len();
        let mut centroid = vec![0.0; dim];
        
        for memory in memories {
            for (i, &val) in memory.embedding.iter().enumerate() {
                if i < dim {
                    centroid[i] += val;
                }
            }
        }
        
        let count = memories.len() as f32;
        for val in &mut centroid {
            *val /= count;
        }
        
        centroid
    }

    /// Get concatenated content for summarization
    #[must_use]
    pub fn concatenated_content(&self) -> String {
        self.memories
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n---\n\n")
    }

    /// Build the prompt for LLM consolidation
    #[must_use]
    pub fn build_consolidation_prompt(&self) -> String {
        let instruction = self.compression_level.summarization_prompt();
        let content = self.concatenated_content();
        let count = self.memories.len();
        
        format!(
            "You are consolidating {count} memories into one.\n\n\
            Instruction: {instruction}\n\n\
            Memories to consolidate:\n\n{content}\n\n\
            Consolidated memory:"
        )
    }
}

/// Result of a consolidation run
#[derive(Debug, Clone, Default)]
pub struct ConsolidationResult {
    /// Number of memories processed
    pub memories_processed: usize,
    /// Number of clusters formed
    pub clusters_formed: usize,
    /// Number of memories merged (reduced by)
    pub memories_merged: usize,
    /// Number of memories expanded
    pub memories_expanded: usize,
    /// Errors encountered (non-fatal)
    pub errors: Vec<String>,
}

/// Compute a composite clustering score between two memories.
///
/// Blends semantic similarity, recall affinity, importance proximity, and age proximity.
/// Returns a value in [0, 1] where higher means more likely to cluster together.
pub fn composite_cluster_score(
    a: &MemoryEntry,
    b: &MemoryEntry,
    weights: &ClusteringWeights,
) -> f32 {
    // 1. Semantic similarity (cosine)
    let sim = cosine_similarity(&a.embedding, &b.embedding).max(0.0);

    // 2. Recall affinity: memories accessed a similar number of times are "peers"
    //    and memories both accessed recently are likely contextually related.
    let recall_a = a.fsrs.access_count as f32;
    let recall_b = b.fsrs.access_count as f32;
    let max_recall = recall_a.max(recall_b).max(1.0);
    // Equal access counts are peers (affinity 1), including two never-accessed
    // memories — `min/max` wrongly scored (0,0) as 0. Divergence lowers affinity.
    let recall_affinity = 1.0 - (recall_a - recall_b).abs() / max_recall; // 0..1

    // 3. Importance proximity: prefer merging memories of similar importance.
    //    Two low-importance memories should merge; a high+low pair should not.
    let imp_diff = (a.fsrs.importance - b.fsrs.importance).abs();
    let importance_prox = (1.0 - imp_diff / 5.0).max(0.0); // normalize: max diff ~5

    // 4. Age proximity: memories from the same time period are more likely related.
    //    Uses the gap between timestamps relative to the older memory's age.
    let age_diff_days = (a.timestamp - b.timestamp).unsigned_abs() as f32 / 86400.0;
    let age_prox = (-age_diff_days / 30.0).exp(); // half-life ~30 days

    // Weighted sum
    let total_weight = weights.similarity + weights.recall_affinity
        + weights.importance_proximity + weights.age_proximity;
    if total_weight <= 0.0 {
        return sim; // fallback to pure similarity
    }

    (weights.similarity * sim
        + weights.recall_affinity * recall_affinity
        + weights.importance_proximity * importance_prox
        + weights.age_proximity * age_prox)
        / total_weight
}

/// Cluster memories using the composite score (not just cosine similarity).
///
/// Uses greedy clustering: for each unassigned memory, find all unassigned memories
/// whose composite score exceeds the threshold and group them.
///
/// Each cluster is bounded by `config.max_cluster_memories` (member count) and
/// `config.max_cluster_content_bytes` (total content) so the consolidation prompt
/// built from it can never overflow a small local model's context window. A
/// candidate that would breach either bound is left unassigned and re-clustered
/// on a later seed — nothing is dropped, the band just consolidates in more passes.
pub fn cluster_memories(
    memories: Vec<MemoryEntry>,
    config: &ConsolidationConfig,
) -> Vec<Vec<MemoryEntry>> {
    if memories.is_empty() {
        return Vec::new();
    }
    // A cap of 0 would make every cluster a skipped singleton — a config bug.
    debug_assert!(config.max_cluster_memories >= 1, "max_cluster_memories must be >= 1");

    let cap_count = config.max_cluster_memories.max(1);
    let cap_bytes = config.max_cluster_content_bytes;

    let mut clusters: Vec<Vec<MemoryEntry>> = Vec::new();
    let mut assigned = vec![false; memories.len()];

    for i in 0..memories.len() {
        if assigned[i] {
            continue;
        }

        // The seed is always admitted (a single over-sized memory forms a lone,
        // sub-`min_cluster_size` cluster that consolidation then skips).
        let mut cluster = vec![memories[i].clone()];
        let mut cluster_bytes = memories[i].content.len();
        assigned[i] = true;

        for j in (i + 1)..memories.len() {
            if cluster.len() >= cap_count {
                break; // count bound reached — remaining matches seed later clusters
            }
            if assigned[j] {
                continue;
            }

            let score = composite_cluster_score(
                &memories[i],
                &memories[j],
                &config.clustering_weights,
            );
            if score < config.cluster_threshold {
                continue;
            }
            // Byte bound: skip (don't consume) a candidate that would overflow the
            // prompt budget; a smaller later candidate may still fit.
            if cluster_bytes.saturating_add(memories[j].content.len()) > cap_bytes {
                continue;
            }

            cluster_bytes += memories[j].content.len();
            cluster.push(memories[j].clone());
            assigned[j] = true;
        }

        clusters.push(cluster);
    }

    // Postconditions: every cluster honors the count bound, and every multi-member
    // cluster honors the byte bound (a lone seed may exceed it by itself).
    debug_assert!(
        clusters.iter().all(|c| c.len() <= cap_count),
        "a cluster exceeded max_cluster_memories"
    );
    debug_assert!(
        clusters.iter().all(|c| {
            c.len() == 1 || c.iter().map(|m| m.content.len()).sum::<usize>() <= cap_bytes
        }),
        "a multi-member cluster exceeded max_cluster_content_bytes"
    );

    clusters
}

/// Cosine similarity between two vectors
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    // Guards preserve this clusterer's "0.0 on mismatch/empty" contract:
    // `nanna_simd::cosine_similarity_f32` *panics* on unequal lengths (memories
    // from different embedding-dimension eras can co-occur during migration)
    // and yields NaN for a zero-magnitude vector.
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    // Delegate to the workspace SIMD primitive (AVX-512/AVX2/NEON) — the same
    // one the vector-search path uses — instead of a private scalar loop, since
    // this runs O(N^2) times per band during a dream cycle.
    let sim = nanna_simd::cosine_similarity_f32(a, b);
    if sim.is_finite() { sim } else { 0.0 }
}

/// Create a consolidated memory entry from a cluster
pub fn create_consolidated_entry(
    cluster: &MemoryCluster,
    consolidated_content: String,
    new_embedding: Vec<f32>,
) -> MemoryEntry {
    // Merge metadata from all memories
    let mut metadata: HashMap<String, String> = HashMap::new();
    for memory in &cluster.memories {
        for (k, v) in &memory.metadata {
            metadata.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    
    // Track consolidation
    let source_ids: Vec<_> = cluster.memories.iter().map(|m| m.id.clone()).collect();
    metadata.insert("consolidated_from".to_string(), source_ids.join(","));
    metadata.insert("consolidation_level".to_string(), format!("{:?}", cluster.compression_level));

    // Precondition: consolidation is only meaningful for a non-empty cluster.
    debug_assert!(
        !cluster.memories.is_empty(),
        "create_consolidated_entry on empty cluster"
    );

    // Create new FSRS state inheriting the best traits from the cluster.
    // Use `max_finite_or` (not `max_by(partial_cmp).unwrap()`) so a stray NaN in
    // a stored FSRS value can't panic the dreaming cycle.
    let mut fsrs = FsrsState::new();
    // Importance: take the max (don't average away a high-importance memory)
    fsrs.importance = max_finite_or(cluster.memories.iter().map(|m| m.fsrs.importance), 1.0);
    fsrs.storage_strength = max_finite_or(
        cluster.memories.iter().map(|m| m.fsrs.storage_strength),
        0.1,
    );
    // Sum access counts (consolidated memory inherits all recall history)
    fsrs.access_count = cluster.memories.iter()
        .map(|m| m.fsrs.access_count)
        .sum();
    fsrs.generation = cluster.memories.iter()
        .map(|m| m.fsrs.generation)
        .max()
        .unwrap_or(0) + 1;

    // Inherit workspace_id from cluster (all memories in cluster should have same workspace)
    let workspace_id = cluster.memories.first()
        .and_then(|m| m.workspace_id.clone());

    // Postcondition: the merged FSRS scalars are always finite (never NaN/inf),
    // so downstream weight math can't be poisoned.
    debug_assert!(
        fsrs.importance.is_finite() && fsrs.storage_strength.is_finite(),
        "consolidated FSRS scalars must be finite"
    );

    MemoryEntry {
        id: uuid::Uuid::new_v4().to_string(),
        content: consolidated_content,
        embedding: new_embedding,
        metadata,
        timestamp: now(),
        fsrs,
        workspace_id,
    }
}

/// Maximum of the finite values in `values`, or `default` when none are finite.
///
/// NaN-safe replacement for `values.max_by(|a, b| a.partial_cmp(b).unwrap())`,
/// whose `unwrap` panics the moment any value is NaN. Non-finite inputs
/// (NaN/±inf) are skipped rather than compared, so a single corrupt stored FSRS
/// scalar can't crash the dreaming cycle.
fn max_finite_or(values: impl Iterator<Item = f32>, default: f32) -> f32 {
    debug_assert!(default.is_finite(), "default must be finite");

    let result = values
        .filter(|v| v.is_finite())
        .fold(None, |acc: Option<f32>, v| {
            Some(acc.map_or(v, |a| a.max(v)))
        })
        .unwrap_or(default);

    debug_assert!(
        result.is_finite(),
        "max_finite_or must return a finite value"
    );
    result
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compression_levels() {
        let thresholds = WeightThresholds::default();
        
        assert_eq!(CompressionLevel::from_weight(0.1, &thresholds), CompressionLevel::Essence);
        assert_eq!(CompressionLevel::from_weight(0.3, &thresholds), CompressionLevel::Compressed);
        assert_eq!(CompressionLevel::from_weight(0.6, &thresholds), CompressionLevel::Standard);
        assert_eq!(CompressionLevel::from_weight(0.9, &thresholds), CompressionLevel::Detailed);
        assert_eq!(CompressionLevel::from_weight(1.5, &thresholds), CompressionLevel::Expand);
    }

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 0.001);

        let c = vec![0.0, 1.0, 0.0];
        assert!((cosine_similarity(&a, &c) - 0.0).abs() < 0.001);

        let d = vec![0.707, 0.707, 0.0];
        assert!((cosine_similarity(&a, &d) - 0.707).abs() < 0.01);
    }

    // Reference scalar implementation kept only in the test, to prove the
    // SIMD-backed `cosine_similarity` matches it across random vectors.
    fn cosine_reference(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() || a.is_empty() {
            return 0.0;
        }
        let mut dot = 0.0f32;
        let (mut na, mut nb) = (0.0f32, 0.0f32);
        for (x, y) in a.iter().zip(b.iter()) {
            dot += x * y;
            na += x * x;
            nb += y * y;
        }
        if na == 0.0 || nb == 0.0 {
            return 0.0;
        }
        dot / (na.sqrt() * nb.sqrt())
    }

    // PRNG + norm math is test-only; precision of the f32 casts is irrelevant.
    #[allow(clippy::cast_precision_loss, clippy::suboptimal_flops)]
    #[test]
    fn cosine_matches_scalar_reference_on_embedding_sized_vectors() {
        // Deterministic pseudo-random 768-dim pairs (typical embedding width).
        for seed in 0..8u32 {
            let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
            let mut next = || {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32) * 2.0 - 1.0
            };
            let a: Vec<f32> = (0..768).map(|_| next()).collect();
            let b: Vec<f32> = (0..768).map(|_| next()).collect();
            let simd = cosine_similarity(&a, &b);
            let scalar = cosine_reference(&a, &b);
            assert!(
                (simd - scalar).abs() < 1e-4,
                "seed {seed}: simd {simd} vs scalar {scalar}"
            );
        }
    }

    #[test]
    fn cosine_edge_cases_return_zero_not_nan() {
        // Zero-magnitude vector: SIMD yields NaN; the guard must return 0.0.
        let zero = vec![0.0f32; 4];
        let v = vec![1.0f32, 2.0, 3.0, 4.0];
        let z = cosine_similarity(&zero, &v);
        assert!(z.is_finite() && z.abs() < 1e-6, "zero-vector → 0.0, got {z}");
        // Length mismatch must not panic (SIMD would); return 0.0.
        assert!(cosine_similarity(&[1.0, 2.0], &[1.0, 2.0, 3.0]).abs() < f32::EPSILON);
        // Empty input.
        assert!(cosine_similarity(&[], &[]).abs() < f32::EPSILON);
    }

    #[test]
    fn test_composite_score_identical() {
        let weights = ClusteringWeights::default();
        let entry = MemoryEntry {
            id: "1".into(),
            content: "test".into(),
            embedding: vec![1.0, 0.0, 0.0],
            metadata: HashMap::new(),
            timestamp: 1000000,
            fsrs: FsrsState::default(),
            workspace_id: None,
        };
        let score = composite_cluster_score(&entry, &entry, &weights);
        assert!(score > 0.9, "identical memories should score high: {}", score);
    }

    #[test]
    fn test_composite_score_different() {
        let weights = ClusteringWeights::default();
        let a = MemoryEntry {
            id: "1".into(),
            content: "A".into(),
            embedding: vec![1.0, 0.0, 0.0],
            metadata: HashMap::new(),
            timestamp: 0,
            fsrs: FsrsState { importance: 1.0, access_count: 0, ..FsrsState::default() },
            workspace_id: None,
        };
        let b = MemoryEntry {
            id: "2".into(),
            content: "B".into(),
            embedding: vec![0.0, 1.0, 0.0],
            metadata: HashMap::new(),
            timestamp: 86400 * 365, // 1 year apart
            fsrs: FsrsState { importance: 5.0, access_count: 100, ..FsrsState::default() },
            workspace_id: None,
        };
        let score = composite_cluster_score(&a, &b, &weights);
        assert!(score < 0.3, "very different memories should score low: {}", score);
    }

    #[test]
    fn test_clustering_with_composite() {
        let config = ConsolidationConfig {
            cluster_threshold: 0.5,
            ..Default::default()
        };

        let memories = vec![
            MemoryEntry {
                id: "1".into(),
                content: "A".into(),
                embedding: vec![1.0, 0.0, 0.0],
                metadata: HashMap::new(),
                timestamp: 1000,
                fsrs: FsrsState { importance: 2.0, access_count: 5, ..FsrsState::default() },
                workspace_id: None,
            },
            MemoryEntry {
                id: "2".into(),
                content: "B".into(),
                embedding: vec![0.99, 0.1, 0.0],
                metadata: HashMap::new(),
                timestamp: 1100,
                fsrs: FsrsState { importance: 2.0, access_count: 4, ..FsrsState::default() },
                workspace_id: None,
            },
            MemoryEntry {
                id: "3".into(),
                content: "C".into(),
                embedding: vec![0.0, 1.0, 0.0],
                metadata: HashMap::new(),
                timestamp: 86400 * 60,
                fsrs: FsrsState { importance: 5.0, access_count: 0, ..FsrsState::default() },
                workspace_id: None,
            },
        ];

        let clusters = cluster_memories(memories, &config);

        // 1 and 2 are similar in embedding, importance, age, and recall → should cluster
        // 3 is different on every axis → separate
        assert_eq!(clusters.len(), 2);
    }

    /// A memory that clusters with every other `similar_entry` (identical
    /// embedding/importance/recall, adjacent timestamps → composite score ≈ 1).
    fn similar_entry(id: &str, content: &str) -> MemoryEntry {
        MemoryEntry {
            id: id.to_string(),
            content: content.to_string(),
            embedding: vec![1.0, 0.0, 0.0],
            metadata: HashMap::new(),
            timestamp: 1000,
            fsrs: FsrsState { importance: 2.0, access_count: 5, ..FsrsState::default() },
            workspace_id: None,
        }
    }

    #[test]
    fn cluster_member_count_is_bounded_and_lossless() {
        // 25 mutually-similar memories with a cap of 10 must split into bounded
        // clusters (10, 10, 5) with every memory preserved exactly once.
        let config = ConsolidationConfig {
            cluster_threshold: 0.3,
            max_cluster_memories: 10,
            ..Default::default()
        };
        let memories: Vec<_> = (0..25).map(|i| similar_entry(&i.to_string(), "x")).collect();

        let clusters = cluster_memories(memories, &config);

        assert!(
            clusters.iter().all(|c| c.len() <= 10),
            "no cluster may exceed the member cap"
        );
        let total: usize = clusters.iter().map(Vec::len).sum();
        assert_eq!(total, 25, "no memory may be dropped");
        let mut ids: Vec<_> = clusters.iter().flatten().map(|m| m.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 25, "every memory appears exactly once");
    }

    #[test]
    fn cluster_content_bytes_are_bounded_and_lossless() {
        // Ten 100-byte memories with a 300-byte budget → at most 3 per cluster.
        let config = ConsolidationConfig {
            cluster_threshold: 0.3,
            max_cluster_memories: 1000,
            max_cluster_content_bytes: 300,
            ..Default::default()
        };
        let body = "a".repeat(100);
        let memories: Vec<_> = (0..10).map(|i| similar_entry(&i.to_string(), &body)).collect();

        let clusters = cluster_memories(memories, &config);

        assert!(
            clusters
                .iter()
                .all(|c| c.len() == 1 || c.iter().map(|m| m.content.len()).sum::<usize>() <= 300),
            "a multi-member cluster exceeded the byte budget"
        );
        assert_eq!(
            clusters.iter().map(Vec::len).sum::<usize>(),
            10,
            "no memory may be dropped by the byte bound"
        );
    }

    #[test]
    fn config_deserializes_without_new_bound_fields() {
        // A ConsolidationConfig serialized before the bound fields existed must
        // still load, defaulting the new caps (serde backward compatibility).
        let legacy = r#"{
            "cluster_threshold": 0.45,
            "min_cluster_size": 2,
            "max_compression_ratio": 0.5,
            "min_remaining_memories": 20,
            "weight_thresholds": {"essence":0.2,"compressed":0.5,"standard":0.8,"detailed":1.0},
            "clustering_weights": {"similarity":0.45,"recall_affinity":0.2,"importance_proximity":0.2,"age_proximity":0.15}
        }"#;
        let cfg: ConsolidationConfig = serde_json::from_str(legacy).expect("legacy config must load");
        assert_eq!(cfg.max_cluster_memories, DEFAULT_MAX_CLUSTER_MEMORIES);
        assert_eq!(cfg.max_cluster_content_bytes, DEFAULT_MAX_CLUSTER_CONTENT_BYTES);
    }

    #[test]
    fn content_budget_scales_with_context_window() {
        // Bigger context → bigger (or equal) content budget, and each budget
        // fits the token window: at the estimator's worst-case 0.5 tok/byte the
        // content is at most `bytes / 2` tokens, which must stay under `window`.
        let small = cluster_content_bytes_for_context(8_192);
        let large = cluster_content_bytes_for_context(200_000);
        assert!(large > small, "budget must grow with the window");
        for window in [8_192_usize, 32_000, 128_000, 200_000] {
            let bytes = cluster_content_bytes_for_context(window);
            let worst_case_tokens = bytes / 2; // 0.5 tok/byte upper bound
            assert!(
                worst_case_tokens < window,
                "budget {bytes}B (~{worst_case_tokens} tok) must fit the {window}-tok window"
            );
        }
    }

    #[test]
    fn content_budget_floors_on_tiny_window() {
        // A tiny or misreported window must not collapse the budget to zero, or
        // consolidation would lock up (every candidate rejected).
        assert!(cluster_content_bytes_for_context(0) >= 2_048);
        assert!(cluster_content_bytes_for_context(100) >= 2_048);
    }

    #[test]
    fn with_summarizer_context_window_sizes_the_byte_budget() {
        // The builder swaps the small-model fallback for the real model's budget
        // and leaves the member-count cap alone.
        let base = ConsolidationConfig::default();
        let large = base.clone().with_summarizer_context_window(200_000);
        assert_eq!(
            large.max_cluster_content_bytes,
            cluster_content_bytes_for_context(200_000)
        );
        assert!(large.max_cluster_content_bytes > base.max_cluster_content_bytes);
        assert_eq!(large.max_cluster_memories, base.max_cluster_memories);
    }

    #[test]
    fn default_content_budget_matches_fallback_window() {
        // default() must equal the fallback-window formula (one source of truth).
        assert_eq!(
            DEFAULT_MAX_CLUSTER_CONTENT_BYTES,
            cluster_content_bytes_for_context(FALLBACK_SUMMARIZER_CONTEXT_WINDOW_TOKENS)
        );
    }

    #[test]
    fn test_cluster_prompt() {
        let memories = vec![
            MemoryEntry {
                id: "1".to_string(),
                content: "The user prefers dark mode".to_string(),
                embedding: vec![1.0, 0.0],
                metadata: HashMap::new(),
                timestamp: 0,
                fsrs: FsrsState::default(),
                workspace_id: None,
            },
            MemoryEntry {
                id: "2".to_string(),
                content: "User likes dark themes in apps".to_string(),
                embedding: vec![0.95, 0.1],
                metadata: HashMap::new(),
                timestamp: 0,
                fsrs: FsrsState::default(),
                workspace_id: None,
            },
        ];

        let cluster = MemoryCluster::new(memories, CompressionLevel::Compressed, &FsrsParameters::default());
        let prompt = cluster.build_consolidation_prompt();
        
        assert!(prompt.contains("2 memories"));
        assert!(prompt.contains("dark mode"));
        assert!(prompt.contains("dark themes"));
    }

    fn entry_with_fsrs(id: &str, importance: f32, storage: f32, access: u32) -> MemoryEntry {
        let fsrs = FsrsState {
            importance,
            storage_strength: storage,
            access_count: access,
            ..FsrsState::default()
        };
        MemoryEntry {
            id: id.to_string(),
            content: format!("memory {id}"),
            embedding: vec![1.0, 0.0],
            metadata: HashMap::new(),
            timestamp: 0,
            fsrs,
            workspace_id: None,
        }
    }

    #[test]
    fn max_finite_or_skips_nan_and_inf() {
        // A NaN or ±inf must be ignored, never compared (the old
        // `max_by(partial_cmp).unwrap()` panicked on NaN).
        let vals = [1.0_f32, f32::NAN, 3.5, f32::INFINITY, 2.0];
        assert!((max_finite_or(vals.into_iter(), 1.0) - 3.5).abs() < 1e-6);
        // All non-finite → fall back to the default.
        let none = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];
        assert!((max_finite_or(none.into_iter(), 0.1) - 0.1).abs() < 1e-6);
        // Empty → default.
        assert!((max_finite_or(std::iter::empty(), 1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn consolidated_entry_takes_max_importance_and_sums_access() {
        let cluster = MemoryCluster {
            memories: vec![
                entry_with_fsrs("a", 2.0, 0.3, 4),
                entry_with_fsrs("b", 5.0, 0.1, 7),
            ],
            centroid: vec![1.0, 0.0],
            compression_level: CompressionLevel::Standard,
            avg_weight: 0.5,
        };
        let out = create_consolidated_entry(&cluster, "merged".into(), vec![1.0, 0.0]);
        assert!((out.fsrs.importance - 5.0).abs() < 1e-6, "importance = max");
        assert!(
            (out.fsrs.storage_strength - 0.3).abs() < 1e-6,
            "storage = max"
        );
        assert_eq!(out.fsrs.access_count, 11, "access counts summed");
        assert_eq!(
            out.metadata.get("consolidated_from").map(String::as_str),
            Some("a,b")
        );
    }

    #[test]
    fn consolidated_entry_survives_nan_fsrs() {
        // A corrupt stored FSRS scalar must not panic dreaming; the result
        // still has finite scalars and inherits the finite sibling's importance.
        let cluster = MemoryCluster {
            memories: vec![
                entry_with_fsrs("a", f32::NAN, f32::NAN, 1),
                entry_with_fsrs("b", 3.0, 0.2, 2),
            ],
            centroid: vec![1.0, 0.0],
            compression_level: CompressionLevel::Standard,
            avg_weight: 0.5,
        };
        let out = create_consolidated_entry(&cluster, "merged".into(), vec![1.0, 0.0]);
        assert!(out.fsrs.importance.is_finite() && out.fsrs.storage_strength.is_finite());
        assert!((out.fsrs.importance - 3.0).abs() < 1e-6);
        assert!((out.fsrs.storage_strength - 0.2).abs() < 1e-6);
    }
}
