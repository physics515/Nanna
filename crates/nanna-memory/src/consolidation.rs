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

/// Consolidation configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationConfig {
    /// Minimum composite score to consider two memories for the same cluster (0.0-1.0).
    /// This blends similarity, recall overlap, importance, and age — not just cosine similarity.
    pub cluster_threshold: f32,
    /// Minimum cluster size to consolidate (singletons below this are skipped or expanded)
    pub min_cluster_size: usize,
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
            max_compression_ratio: 0.50,
            min_remaining_memories: 20,
            weight_thresholds: WeightThresholds::default(),
            clustering_weights: ClusteringWeights::default(),
        }
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
    let recall_ratio = recall_a.min(recall_b) / max_recall; // 0..1, 1 = same count
    let recall_affinity = recall_ratio;

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
pub fn cluster_memories(
    memories: Vec<MemoryEntry>,
    config: &ConsolidationConfig,
) -> Vec<Vec<MemoryEntry>> {
    if memories.is_empty() {
        return Vec::new();
    }

    let mut clusters: Vec<Vec<MemoryEntry>> = Vec::new();
    let mut assigned = vec![false; memories.len()];

    for i in 0..memories.len() {
        if assigned[i] {
            continue;
        }

        let mut cluster = vec![memories[i].clone()];
        assigned[i] = true;

        for j in (i + 1)..memories.len() {
            if assigned[j] {
                continue;
            }

            let score = composite_cluster_score(
                &memories[i],
                &memories[j],
                &config.clustering_weights,
            );
            if score >= config.cluster_threshold {
                cluster.push(memories[j].clone());
                assigned[j] = true;
            }
        }

        clusters.push(cluster);
    }

    clusters
}

/// Cosine similarity between two vectors
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;

    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot / (norm_a.sqrt() * norm_b.sqrt())
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

    // Create new FSRS state inheriting the best traits from the cluster
    let mut fsrs = FsrsState::new();
    // Importance: take the max (don't average away a high-importance memory)
    fsrs.importance = cluster.memories.iter()
        .map(|m| m.fsrs.importance)
        .max_by(|a, b| a.partial_cmp(b).unwrap())
        .unwrap_or(1.0);
    fsrs.storage_strength = cluster.memories.iter()
        .map(|m| m.fsrs.storage_strength)
        .max_by(|a, b| a.partial_cmp(b).unwrap())
        .unwrap_or(0.1);
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

    MemoryEntry {
        id: uuid::Uuid::new_v4().to_string(),
        content: consolidated_content,
        embedding: new_embedding,
        metadata,
        timestamp: now(),
        fsrs,
        workspace_id,
        expires_at: None, // Consolidated memories don't expire
    }
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
            expires_at: None,
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
            expires_at: None,
        };
        let b = MemoryEntry {
            id: "2".into(),
            content: "B".into(),
            embedding: vec![0.0, 1.0, 0.0],
            metadata: HashMap::new(),
            timestamp: 86400 * 365, // 1 year apart
            fsrs: FsrsState { importance: 5.0, access_count: 100, ..FsrsState::default() },
            workspace_id: None,
            expires_at: None,
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
                expires_at: None,
            },
            MemoryEntry {
                id: "2".into(),
                content: "B".into(),
                embedding: vec![0.99, 0.1, 0.0],
                metadata: HashMap::new(),
                timestamp: 1100,
                fsrs: FsrsState { importance: 2.0, access_count: 4, ..FsrsState::default() },
                workspace_id: None,
                expires_at: None,
            },
            MemoryEntry {
                id: "3".into(),
                content: "C".into(),
                embedding: vec![0.0, 1.0, 0.0],
                metadata: HashMap::new(),
                timestamp: 86400 * 60,
                fsrs: FsrsState { importance: 5.0, access_count: 0, ..FsrsState::default() },
                workspace_id: None,
                expires_at: None,
            },
        ];

        let clusters = cluster_memories(memories, &config);

        // 1 and 2 are similar in embedding, importance, age, and recall → should cluster
        // 3 is different on every axis → separate
        assert_eq!(clusters.len(), 2);
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
                expires_at: None,
            },
            MemoryEntry {
                id: "2".to_string(),
                content: "User likes dark themes in apps".to_string(),
                embedding: vec![0.95, 0.1],
                metadata: HashMap::new(),
                timestamp: 0,
                fsrs: FsrsState::default(),
                workspace_id: None,
                expires_at: None,
            },
        ];

        let cluster = MemoryCluster::new(memories, CompressionLevel::Compressed, &FsrsParameters::default());
        let prompt = cluster.build_consolidation_prompt();
        
        assert!(prompt.contains("2 memories"));
        assert!(prompt.contains("dark mode"));
        assert!(prompt.contains("dark themes"));
    }
}
