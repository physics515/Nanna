#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! Memory and embedding system for Nanna
//!
//! Provides vector storage and semantic search with SIMD and GPU acceleration.
//! Implements FSRS-6 for cognitive memory decay and the "dreaming" consolidation model.

mod consolidation;
mod dreaming;
mod fsrs;
mod service;

pub use consolidation::{
    ConsolidationConfig, ConsolidationResult, CompressionLevel,
    WeightThresholds, ClusteringWeights, MemoryCluster, cluster_memories,
    create_consolidated_entry, composite_cluster_score,
};
pub use dreaming::{
    DreamingConfig, DreamingService, DreamingStats, MemoryFeedback,
    make_summarize_fn, LlmSummarizer,
};
pub use fsrs::{
    FsrsParameters, FsrsState, MemoryState, Rating, IngestAction,
    power_law_retrievability,
};
pub use service::{
    MemoryService, MemoryServiceConfig, RecallResult, EmbedFn,
    MemoryStats, MemoryListEntry, ConsolidationBands,
};

use async_trait::async_trait;
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
    #[error("Persistence error: {0}")]
    Persistence(String),
}

/// Trait for pluggable persistence backends (Turso, etc.)
///
/// Implementors are responsible for durably storing and retrieving memory entries.
/// The in-memory vector cache remains the primary store for search; this layer
/// provides crash-safe persistence.
#[async_trait]
pub trait MemoryPersistence: Send + Sync {
    /// Persist (insert or update) a single entry.
    async fn save_entry(&self, entry: &MemoryEntry) -> Result<(), MemoryError>;
    /// Remove an entry by its ID.
    async fn remove_entry(&self, id: &str) -> Result<(), MemoryError>;
    /// Update only the FSRS cognitive state for an existing entry.
    async fn update_entry_fsrs(&self, id: &str, fsrs: &FsrsState) -> Result<(), MemoryError>;
    /// Update only the text content for an existing entry.
    async fn update_entry_content(&self, id: &str, content: &str) -> Result<(), MemoryError>;
    /// Load all persisted entries (called on startup to populate the in-memory cache).
    async fn load_all(&self) -> Result<Vec<MemoryEntry>, MemoryError>;
}

/// A memory entry with embedding, metadata, and FSRS cognitive state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub content: String,
    pub embedding: Vec<f32>,
    pub metadata: HashMap<String, String>,
    pub timestamp: i64,
    /// FSRS-6 cognitive state (stability, retrievability, etc.)
    #[serde(default)]
    pub fsrs: FsrsState,
    /// Workspace ID if this memory is scoped to a workspace (None = global)
    #[serde(default)]
    pub workspace_id: Option<String>,
    /// Optional expiration timestamp (epoch seconds). None = never expires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

impl MemoryEntry {
    /// Check if this entry has expired
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            now >= expires_at
        } else {
            false
        }
    }
}

/// Vector store configuration
#[derive(Debug)]
pub struct VectorStoreConfig {
    /// Expected embedding dimension. Stored as `AtomicUsize` so it can be
    /// updated at runtime (e.g., when the embedding model changes via fallback)
    /// without requiring `&mut self` on the `VectorStore`.
    pub dimension: std::sync::atomic::AtomicUsize,
    pub use_f16: bool,  // Store embeddings as f16 to save memory
}

impl Clone for VectorStoreConfig {
    fn clone(&self) -> Self {
        Self {
            dimension: std::sync::atomic::AtomicUsize::new(
                self.dimension.load(std::sync::atomic::Ordering::Relaxed)
            ),
            use_f16: self.use_f16,
        }
    }
}

impl Default for VectorStoreConfig {
    fn default() -> Self {
        Self {
            dimension: std::sync::atomic::AtomicUsize::new(1536),  // OpenAI ada-002 default
            use_f16: true,
        }
    }
}

impl VectorStoreConfig {
    /// Create config with specified dimension
    pub fn with_dimension(dim: usize) -> Self {
        Self {
            dimension: std::sync::atomic::AtomicUsize::new(dim),
            use_f16: true,
        }
    }

    /// Get the current dimension
    pub fn get_dimension(&self) -> usize {
        self.dimension.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Set a new dimension at runtime
    pub fn set_dimension(&self, dim: usize) {
        self.dimension.store(dim, std::sync::atomic::Ordering::Relaxed);
    }
}

/// In-memory vector store with SIMD and GPU-accelerated search
pub struct VectorStore {
    config: VectorStoreConfig,
    entries: RwLock<Vec<MemoryEntry>>,
    gpu: Option<Arc<GpuContext>>,
    gpu_pipeline: Option<CosineSimilaritySearch>,
    /// Optional Turso (or other) backing store for durable persistence.
    /// When set, writes (add/remove/update) are mirrored to the backing store.
    /// Search always operates purely in-memory.
    db: Option<Arc<dyn MemoryPersistence + Send + Sync>>,
}

impl VectorStore {
    #[must_use] 
    pub fn new(config: VectorStoreConfig) -> Self {
        Self {
            config,
            entries: RwLock::new(Vec::new()),
            gpu: None,
            gpu_pipeline: None,
            db: None,
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
                            db: None,
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

    /// Attach a persistence backend.  Returns `self` for builder-style chaining.
    ///
    /// Once attached, every mutating operation (`add`, `remove`, `update_fsrs`,
    /// `update_content`) will also write through to the backing store.
    #[must_use]
    pub fn with_persistence(mut self, db: Arc<dyn MemoryPersistence + Send + Sync>) -> Self {
        self.db = Some(db);
        self
    }

    /// Load all entries from the persistence backend into the in-memory cache.
    ///
    /// This replaces any existing in-memory entries.  Call once on startup after
    /// attaching the persistence layer.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError::Persistence` if the backing store fails to load.
    pub async fn load_from_db(&self) -> Result<usize, MemoryError> {
        let Some(ref db) = self.db else {
            return Err(MemoryError::Persistence("No persistence backend attached".to_string()));
        };

        let loaded = db.load_all().await?;
        let count = loaded.len();

        let mismatched = loaded.iter()
            .filter(|e| e.embedding.len() != self.config.get_dimension())
            .count();

        if mismatched > 0 {
            let sample_dim = loaded.iter()
                .find(|e| e.embedding.len() != self.config.get_dimension())
                .map(|e| e.embedding.len())
                .unwrap_or(0);
            warn!(
                "Dimension mismatch loading from DB: {} of {} entries have {} dims (expected {}). \
                 They will be re-embedded.",
                mismatched, count, sample_dim, self.config.get_dimension()
            );
        }

        let mut entries = self.entries.write().await;
        *entries = loaded;
        info!("Loaded {} entries from persistence backend", count);
        Ok(count)
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
        if entry.embedding.len() != self.config.get_dimension() {
            return Err(MemoryError::DimensionMismatch {
                expected: self.config.get_dimension(),
                got: entry.embedding.len(),
            });
        }

        // Normalize the embedding for cosine similarity
        normalize_f32(&mut entry.embedding);

        // Write-through to persistence backend before updating in-memory cache
        if let Some(ref db) = self.db {
            if let Err(e) = db.save_entry(&entry).await {
                warn!("Failed to persist memory entry {}: {}", entry.id, e);
                // Non-fatal: continue with in-memory add
            }
        }

        self.entries.write().await.push(entry);
        Ok(())
    }

    /// Search for similar memories using SIMD-accelerated cosine similarity.
    ///
    /// GPU dispatch is available but only engaged for very large vector counts
    /// (>50,000) due to buffer upload/readback overhead dominating compute savings.
    ///
    /// ## Benchmark findings (RTX 4070 Ti SUPER + Zen 4 AVX-512)
    ///
    /// GPU fixed overhead: ~750us per dispatch (buffer alloc + upload + shader + readback).
    /// SIMD single cosine similarity (768-dim): ~0.1us (AVX-512).
    /// GPU never beat SIMD up to 10,000 vectors at any dimension tested (768/1536/3072).
    /// At 10,000 vectors: SIMD 1.5ms vs GPU 5.2ms (GPU still 3.5x slower).
    ///
    /// The crossover point with the current per-search buffer upload model is estimated
    /// at ~50,000+ vectors. To make GPU worthwhile at lower counts, we would need
    /// GPU-resident persistent buffers (upload once, search many times).
    ///
    /// See: `cargo bench -p nanna-gpu --bench gpu_vs_simd` for full results.
    pub async fn search(&self, query_embedding: &[f32], top_k: usize) -> Vec<(MemoryEntry, f32)> {
        if query_embedding.len() != self.config.get_dimension() {
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

        // Benchmark-calibrated threshold: GPU only wins with persistent buffers or
        // very large vector counts. With per-search buffer upload, the ~750us fixed
        // overhead means GPU needs >50k vectors to amortize the cost vs AVX-512.
        //
        // Previous threshold was 1000 — benchmarks showed GPU was 23x SLOWER there.
        // See: docs/benchmarks/gpu-vs-simd-analysis.md
        const GPU_THRESHOLD: usize = 50_000;

        let similarities: Vec<f32> = if entry_count >= GPU_THRESHOLD && self.has_gpu() {
            // GPU path: batch all vectors together
            debug!("Using GPU for {} vectors (above {} threshold)", entry_count, GPU_THRESHOLD);
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
            // SIMD path — fast for all practical memory store sizes
            debug!("Using SIMD ({}) for {} vectors", nanna_simd::simd_tier(), entry_count);
            entries
                .iter()
                .map(|entry| cosine_similarity_f32(&query, &entry.embedding))
                .collect()
        };

        // Pair entries with similarities, filtering out expired entries
        let mut scored: Vec<(MemoryEntry, f32)> = entries
            .iter()
            .zip(similarities)
            .filter(|(entry, _)| !entry.is_expired())
            .map(|(entry, sim)| (entry.clone(), sim))
            .collect();
        drop(entries);

        // Sort by similarity (descending) and take top-k
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        scored
    }

    /// Search for similar memories with workspace scope filtering.
    ///
    /// Scope rules:
    /// - `workspace_id = Some(id)`: returns global + that workspace's memories
    /// - `workspace_id = None` (global): returns all memories
    pub async fn search_scoped(
        &self,
        query_embedding: &[f32],
        top_k: usize,
        workspace_id: Option<&str>,
    ) -> Vec<(MemoryEntry, f32)> {
        let all_results = self.search(query_embedding, top_k * 3).await; // Get more to filter
        
        let filtered: Vec<(MemoryEntry, f32)> = match workspace_id {
            // Workspace scope: global + this workspace only
            Some(ws_id) => all_results
                .into_iter()
                .filter(|(entry, _)| {
                    entry.workspace_id.is_none() || entry.workspace_id.as_deref() == Some(ws_id)
                })
                .take(top_k)
                .collect(),
            // Global scope: all memories
            None => all_results.into_iter().take(top_k).collect(),
        };
        
        filtered
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
        drop(entries);

        // Write-through: remove from persistence backend
        if let Some(ref db) = self.db {
            if let Err(e) = db.remove_entry(id).await {
                warn!("Failed to remove memory entry {} from persistence: {}", id, e);
                // Non-fatal
            }
        }

        Ok(())
    }

    /// Remove all expired entries. Returns the count of entries purged.
    pub async fn purge_expired(&self) -> usize {
        let mut entries = self.entries.write().await;
        let before = entries.len();
        entries.retain(|e| !e.is_expired());
        let purged = before - entries.len();
        if purged > 0 {
            info!("Purged {} expired memory entries", purged);
        }
        purged
    }

    /// Update FSRS state for an entry.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError::NotFound` if no entry with the given ID exists.
    pub async fn update_fsrs<F>(&self, id: &str, f: F) -> Result<(), MemoryError>
    where
        F: FnOnce(&mut FsrsState),
    {
        let mut entries = self.entries.write().await;
        let entry = entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| MemoryError::NotFound(id.to_string()))?;
        f(&mut entry.fsrs);
        let new_fsrs = entry.fsrs.clone();
        drop(entries);

        // Write-through to persistence backend
        if let Some(ref db) = self.db {
            if let Err(e) = db.update_entry_fsrs(id, &new_fsrs).await {
                warn!("Failed to persist FSRS update for {}: {}", id, e);
                // Non-fatal
            }
        }

        Ok(())
    }

    /// Update content for an entry (used during expansion).
    ///
    /// # Errors
    ///
    /// Returns `MemoryError::NotFound` if no entry with the given ID exists.
    pub async fn update_content(&self, id: &str, content: &str) -> Result<(), MemoryError> {
        let mut entries = self.entries.write().await;
        let entry = entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| MemoryError::NotFound(id.to_string()))?;
        entry.content = content.to_string();
        drop(entries);

        // Write-through to persistence backend
        if let Some(ref db) = self.db {
            if let Err(e) = db.update_entry_content(id, content).await {
                warn!("Failed to persist content update for {}: {}", id, e);
                // Non-fatal
            }
        }

        Ok(())
    }

    /// Update both the content and embedding of an existing entry (used by
    /// dreaming/merge, where the merged text needs a matching embedding).
    ///
    /// The embedding is normalized in place and must match the store dimension.
    /// Persists the full entry write-through (content + embedding + FSRS).
    ///
    /// # Errors
    /// Returns `MemoryError::DimensionMismatch` if `embedding` has the wrong
    /// length, or `MemoryError::NotFound` if no entry has `id`.
    pub async fn update_content_and_embedding(
        &self,
        id: &str,
        content: &str,
        mut embedding: Vec<f32>,
    ) -> Result<(), MemoryError> {
        debug_assert!(!id.is_empty(), "id must not be empty");
        if embedding.len() != self.config.get_dimension() {
            return Err(MemoryError::DimensionMismatch {
                expected: self.config.get_dimension(),
                got: embedding.len(),
            });
        }
        normalize_f32(&mut embedding);

        let mut entries = self.entries.write().await;
        let entry = entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| MemoryError::NotFound(id.to_string()))?;
        entry.content = content.to_string();
        entry.embedding = embedding;
        let updated = entry.clone();
        drop(entries);

        // Write-through the full entry so content and embedding stay consistent.
        if let Some(ref db) = self.db
            && let Err(e) = db.save_entry(&updated).await
        {
            warn!("Failed to persist merged entry {}: {}", id, e);
            // Non-fatal
        }

        Ok(())
    }

    /// Get all entries (for consolidation)
    pub async fn all_entries(&self) -> Vec<MemoryEntry> {
        self.entries.read().await.clone()
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

    /// Save to file.
    ///
    /// # Deprecated
    ///
    /// This method is retained only for one-time JSON→Turso migration.
    /// Use [`VectorStore::with_persistence`] and [`VectorStore::load_from_db`] instead.
    pub async fn save(&self, path: &std::path::Path) -> Result<(), MemoryError> {
        warn!("VectorStore::save() is deprecated. Use Turso persistence instead.");
        let entries = self.entries.read().await;
        let json = serde_json::to_string_pretty(&*entries)?;
        tokio::fs::write(path, json).await?;
        info!("Saved {} entries to {:?}", entries.len(), path);
        Ok(())
    }

    /// Load from file.
    ///
    /// # Deprecated
    ///
    /// This method is retained only for one-time JSON→Turso migration.
    /// Use [`VectorStore::with_persistence`] and [`VectorStore::load_from_db`] instead.
    ///
    /// Loads all entries regardless of embedding dimension. If the embedding
    /// model has changed, call [`MemoryService::probe_and_align_dimension`]
    /// after loading to re-embed mismatched entries.
    pub async fn load(&self, path: &std::path::Path) -> Result<(), MemoryError> {
        let json = tokio::fs::read_to_string(path).await?;
        let loaded: Vec<MemoryEntry> = serde_json::from_str(&json)?;
        
        info!("Parsing {} entries from {:?}, expecting {} dimensions", 
              loaded.len(), path, self.config.get_dimension());

        let mismatched = loaded.iter()
            .filter(|e| e.embedding.len() != self.config.get_dimension())
            .count();

        if mismatched > 0 {
            let sample_dim = loaded.iter()
                .find(|e| e.embedding.len() != self.config.get_dimension())
                .map(|e| e.embedding.len())
                .unwrap_or(0);
            warn!(
                "Dimension mismatch: {} of {} entries have {} dims (expected {}). \
                 They will be re-embedded after dimension probe.",
                mismatched, loaded.len(), sample_dim, self.config.get_dimension()
            );
        }

        let mut entries = self.entries.write().await;
        *entries = loaded;
        info!("Loaded {} entries from {:?}", entries.len(), path);
        Ok(())
    }

    /// Flush all in-memory entries to the persistence backend.
    ///
    /// Used during one-time JSON → Turso migration: after `load()` populates
    /// the in-memory cache from JSON, call this to persist every entry to Turso.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError::Persistence` if the backing store is not set or a write fails.
    pub async fn flush_to_db(&self) -> Result<usize, MemoryError> {
        let db = self.db.as_ref().ok_or_else(|| {
            MemoryError::Persistence("No persistence backend attached".to_string())
        })?;

        let entries = self.entries.read().await;
        let total = entries.len();
        let mut saved = 0usize;

        for entry in entries.iter() {
            if let Err(e) = db.save_entry(entry).await {
                warn!("Failed to flush entry {} to DB: {}", entry.id, e);
            } else {
                saved += 1;
            }
        }

        info!("Flushed {}/{} entries to Turso", saved, total);
        Ok(saved)
    }

    /// Re-embed all entries whose dimension doesn't match the expected dimension.
    ///
    /// Returns the number of entries re-embedded. Entries that fail to re-embed
    /// are removed (content was likely empty or the embed function errored).
    pub async fn re_embed_mismatched<F, Fut>(
        &self,
        expected_dim: usize,
        embed_fn: F,
    ) -> usize
    where
        F: Fn(String) -> Fut,
        Fut: std::future::Future<Output = Result<Vec<f32>, String>>,
    {
        let mut entries = self.entries.write().await;
        let total = entries.len();
        let mismatched_count = entries.iter()
            .filter(|e| e.embedding.len() != expected_dim)
            .count();

        if mismatched_count == 0 {
            return 0;
        }

        info!(
            "Re-embedding {} of {} entries ({} dims → {} dims)...",
            mismatched_count, total,
            entries.iter().find(|e| e.embedding.len() != expected_dim)
                .map(|e| e.embedding.len()).unwrap_or(0),
            expected_dim
        );

        let mut re_embedded = 0usize;
        let mut failed = 0usize;

        for entry in entries.iter_mut() {
            if entry.embedding.len() == expected_dim {
                continue;
            }

            match (embed_fn)(entry.content.clone()).await {
                Ok(mut new_embedding) => {
                    if new_embedding.len() == expected_dim {
                        normalize_f32(&mut new_embedding);
                        entry.embedding = new_embedding;
                        re_embedded += 1;
                    } else {
                        warn!(
                            "Re-embed returned wrong dimension for '{}': expected {}, got {}",
                            &entry.content[..entry.content.len().min(40)],
                            expected_dim, new_embedding.len()
                        );
                        failed += 1;
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to re-embed '{}': {}",
                        &entry.content[..entry.content.len().min(40)], e
                    );
                    failed += 1;
                }
            }
        }

        // Remove entries that failed to re-embed
        if failed > 0 {
            entries.retain(|e| e.embedding.len() == expected_dim);
            warn!("Dropped {} entries that failed to re-embed", failed);
        }

        info!(
            "Re-embedding complete: {} succeeded, {} failed, {} total entries",
            re_embedded, failed, entries.len()
        );

        re_embedded
    }

    /// Get the current configured dimension
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.config.get_dimension()
    }

    /// Update the expected embedding dimension at runtime.
    ///
    /// Called by [`MemoryService::probe_and_align_dimension`] when the
    /// embedding model changes and returns a different dimension.
    /// After this call, `add()` accepts entries with the new dimension.
    pub fn set_dimension(&self, new_dim: usize) {
        self.config.set_dimension(new_dim);
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
            dimension: std::sync::atomic::AtomicUsize::new(8),
            use_f16: false,
        };
        let store = VectorStore::new(config);

        let entry = MemoryEntry {
            id: "test1".to_string(),
            content: "Hello world".to_string(),
            embedding: vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            metadata: HashMap::new(),
            timestamp: 0,
            fsrs: FsrsState::default(),
            workspace_id: None,
            expires_at: None,
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
