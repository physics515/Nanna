#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! Memory and embedding system for Nanna
//!
//! Provides vector storage and semantic search with SIMD and GPU acceleration.

mod service;

pub use service::{MemoryService, MemoryServiceConfig, RecallResult, EmbedFn};

use nanna_gpu::{CosineSimilaritySearch, GpuContext};
use nanna_simd::{cosine_similarity_f32, normalize_f32};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

#[derive(Error, Debug)]
pub enum MemoryError {
    #[error("Embedding dimension mismatch: expected {expected}, got {got}")]
    DimensionMismatch { expected: usize, got: usize },
    #[error("Memory not found: {0}")]
    NotFound(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// A memory entry with embedding and metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub content: String,
    pub embedding: Vec<f32>,
    pub metadata: HashMap<String, String>,
    pub timestamp: i64,
}

/// Vector store configuration
#[derive(Debug, Clone)]
pub struct VectorStoreConfig {
    pub dimension: usize,
    pub use_f16: bool,  // Store embeddings as f16 to save memory
}

impl Default for VectorStoreConfig {
    fn default() -> Self {
        Self {
            dimension: 1536,  // OpenAI ada-002 default
            use_f16: true,
        }
    }
}

/// In-memory vector store with SIMD and GPU-accelerated search
pub struct VectorStore {
    config: VectorStoreConfig,
    entries: RwLock<Vec<MemoryEntry>>,
    gpu: Option<Arc<GpuContext>>,
    gpu_pipeline: Option<CosineSimilaritySearch>,
}

impl VectorStore {
    #[must_use] 
    pub fn new(config: VectorStoreConfig) -> Self {
        Self {
            config,
            entries: RwLock::new(Vec::new()),
            gpu: None,
            gpu_pipeline: None,
        }
    }

    /// Create a vector store with GPU acceleration.
    ///
    /// Falls back to SIMD if GPU initialization fails.
    pub async fn with_gpu(config: VectorStoreConfig) -> Self {
        match GpuContext::new().await {
            Ok(ctx) => {
                let ctx = Arc::new(ctx);
                match CosineSimilaritySearch::new(&ctx) {
                    Ok(pipeline) => {
                        info!("VectorStore using GPU: {}", ctx.adapter_info.name);
                        Self {
                            config,
                            entries: RwLock::new(Vec::new()),
                            gpu: Some(ctx),
                            gpu_pipeline: Some(pipeline),
                        }
                    }
                    Err(e) => {
                        warn!("GPU pipeline creation failed, using SIMD: {}", e);
                        Self::new(config)
                    }
                }
            }
            Err(e) => {
                warn!("GPU initialization failed, using SIMD: {}", e);
                Self::new(config)
            }
        }
    }

    /// Check if GPU acceleration is available.
    #[must_use]
    pub fn has_gpu(&self) -> bool {
        self.gpu.is_some() && self.gpu_pipeline.is_some()
    }

    /// Add a memory entry.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError::DimensionMismatch` if the embedding dimension is wrong.
    pub async fn add(&self, mut entry: MemoryEntry) -> Result<(), MemoryError> {
        if entry.embedding.len() != self.config.dimension {
            return Err(MemoryError::DimensionMismatch {
                expected: self.config.dimension,
                got: entry.embedding.len(),
            });
        }

        // Normalize the embedding for cosine similarity
        normalize_f32(&mut entry.embedding);

        self.entries.write().await.push(entry);
        Ok(())
    }

    /// Search for similar memories using GPU or SIMD-accelerated cosine similarity.
    ///
    /// Uses GPU for large vector counts (>1000), SIMD for smaller sets.
    pub async fn search(&self, query_embedding: &[f32], top_k: usize) -> Vec<(MemoryEntry, f32)> {
        if query_embedding.len() != self.config.dimension {
            return Vec::new();
        }

        // Normalize query
        let mut query = query_embedding.to_vec();
        normalize_f32(&mut query);

        let entries = self.entries.read().await;
        let entry_count = entries.len();

        if entry_count == 0 {
            return Vec::new();
        }

        // Use GPU for large vector counts, SIMD for smaller sets
        const GPU_THRESHOLD: usize = 1000;

        let similarities: Vec<f32> = if entry_count >= GPU_THRESHOLD && self.has_gpu() {
            // GPU path: batch all vectors together
            debug!("Using GPU for {} vectors", entry_count);
            let gpu = self.gpu.as_ref().unwrap();
            let pipeline = self.gpu_pipeline.as_ref().unwrap();

            // Flatten all embeddings into a single buffer
            let vectors: Vec<f32> = entries
                .iter()
                .flat_map(|e| e.embedding.iter().copied())
                .collect();

            match pipeline.search(gpu, &query, &vectors).await {
                Ok(sims) => sims,
                Err(e) => {
                    warn!("GPU search failed, falling back to SIMD: {}", e);
                    // Fallback to SIMD
                    entries
                        .iter()
                        .map(|entry| cosine_similarity_f32(&query, &entry.embedding))
                        .collect()
                }
            }
        } else {
            // SIMD path
            debug!("Using SIMD for {} vectors", entry_count);
            entries
                .iter()
                .map(|entry| cosine_similarity_f32(&query, &entry.embedding))
                .collect()
        };

        // Pair entries with similarities
        let mut scored: Vec<(MemoryEntry, f32)> = entries
            .iter()
            .zip(similarities)
            .map(|(entry, sim)| (entry.clone(), sim))
            .collect();
        drop(entries);

        // Sort by similarity (descending) and take top-k
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        scored
    }

    /// Get entry by ID
    pub async fn get(&self, id: &str) -> Option<MemoryEntry> {
        let entries = self.entries.read().await;
        entries.iter().find(|e| e.id == id).cloned()
    }

    /// Remove entry by ID.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError::NotFound` if no entry with the given ID exists.
    pub async fn remove(&self, id: &str) -> Result<(), MemoryError> {
        let mut entries = self.entries.write().await;
        let idx = entries
            .iter()
            .position(|e| e.id == id)
            .ok_or_else(|| MemoryError::NotFound(id.to_string()))?;
        entries.remove(idx);
        Ok(())
    }

    /// Get total number of entries
    pub async fn len(&self) -> usize {
        self.entries.read().await.len()
    }

    /// Check if store is empty
    pub async fn is_empty(&self) -> bool {
        self.entries.read().await.is_empty()
    }

    /// Clear all entries
    pub async fn clear(&self) {
        self.entries.write().await.clear();
    }

    /// Save to file
    pub async fn save(&self, path: &std::path::Path) -> Result<(), MemoryError> {
        let entries = self.entries.read().await;
        let json = serde_json::to_string_pretty(&*entries)?;
        tokio::fs::write(path, json).await?;
        info!("Saved {} entries to {:?}", entries.len(), path);
        Ok(())
    }

    /// Load from file
    pub async fn load(&self, path: &std::path::Path) -> Result<(), MemoryError> {
        let json = tokio::fs::read_to_string(path).await?;
        let loaded: Vec<MemoryEntry> = serde_json::from_str(&json)?;
        
        // Validate dimensions
        for entry in &loaded {
            if entry.embedding.len() != self.config.dimension {
                return Err(MemoryError::DimensionMismatch {
                    expected: self.config.dimension,
                    got: entry.embedding.len(),
                });
            }
        }

        let mut entries = self.entries.write().await;
        *entries = loaded;
        info!("Loaded {} entries from {:?}", entries.len(), path);
        Ok(())
    }
}

/// Conversation memory for maintaining chat context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMemory {
    pub session_id: String,
    pub messages: Vec<ConversationMessage>,
    pub max_messages: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub role: String,
    pub content: String,
    pub timestamp: i64,
}

impl ConversationMemory {
    pub fn new(session_id: impl Into<String>, max_messages: usize) -> Self {
        Self {
            session_id: session_id.into(),
            messages: Vec::new(),
            max_messages,
        }
    }

    pub fn add(&mut self, role: impl Into<String>, content: impl Into<String>) {
        let msg = ConversationMessage {
            role: role.into(),
            content: content.into(),
            timestamp: chrono_timestamp(),
        };
        self.messages.push(msg);

        // Trim old messages if over limit
        if self.messages.len() > self.max_messages {
            let to_remove = self.messages.len() - self.max_messages;
            self.messages.drain(0..to_remove);
        }
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }

    #[must_use] 
    pub const fn len(&self) -> usize {
        self.messages.len()
    }

    #[must_use] 
    pub const fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

fn chrono_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_vector_store() {
        let config = VectorStoreConfig {
            dimension: 8,
            use_f16: false,
        };
        let store = VectorStore::new(config);

        let entry = MemoryEntry {
            id: "test1".to_string(),
            content: "Hello world".to_string(),
            embedding: vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            metadata: HashMap::new(),
            timestamp: 0,
        };

        store.add(entry).await.unwrap();
        assert_eq!(store.len().await, 1);

        let results = store
            .search(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 10)
            .await;
        assert_eq!(results.len(), 1);
        assert!(results[0].1 > 0.99);  // Should be very similar
    }

    #[test]
    fn test_conversation_memory() {
        let mut memory = ConversationMemory::new("test", 3);
        memory.add("user", "Hello");
        memory.add("assistant", "Hi there!");
        memory.add("user", "How are you?");
        memory.add("assistant", "I'm good!");

        assert_eq!(memory.len(), 3);  // Trimmed to max
        assert_eq!(memory.messages[0].role, "assistant");  // First message was trimmed
    }
}
