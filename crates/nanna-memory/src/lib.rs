#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! Memory and embedding system for Nanna
//!
//! Provides vector storage and semantic search with SIMD and GPU acceleration.
//! Implements FSRS-6 for cognitive memory decay and the "dreaming" consolidation model.

mod activity;
pub mod chunk_rank;
pub mod chunking;
mod consolidation;
mod dreaming;
mod fsrs;
pub mod retention;
mod service;

pub use activity::ActivityClock;

pub use chunk_rank::{collapse_chunk_hits, ChunkHit};
pub use chunking::{chunk_text, derive_chunk_params, Chunk, ChunkParams, CHUNKER_VERSION};

pub use consolidation::{
    ConsolidationConfig, ConsolidationResult, CompressionLevel,
    WeightThresholds, ClusteringWeights, MemoryCluster, cluster_memories,
    create_consolidated_entry, composite_cluster_score,
    cluster_content_bytes_for_context, FALLBACK_SUMMARIZER_CONTEXT_WINDOW_TOKENS,
};
pub use dreaming::{
    dream_trigger, DreamOutcome, DreamTrigger, DreamingConfig, DreamingService, DreamingStats,
    MemoryFeedback, make_summarize_fn, LlmSummarizer,
};
pub use fsrs::{
    FsrsParameters, FsrsState, MemoryState, Rating, IngestAction,
    power_law_retrievability,
};
pub use service::{
    MemoryService, MemoryServiceConfig, RecallResult, EmbedFn,
    MemoryStats, MemoryListEntry, ConsolidationBands,
    // Exported so the episodic writer chunks against the same bound the merge
    // path caps against. Those two numbers drifting apart is precisely how a
    // 4096-byte ceiling ended up silently rejecting 3200-char chunks.
    MEMORY_CHUNK_MAX_CHARS, MEMORY_OBSERVATION_MAX_CHARS,
    MEMORY_CHUNK_TARGET_CHARS, chunk_max_chars_for_window,
};
pub use retention::{
    measure_gated_recall, measure_recall, run_retention_cycle, CorpusParams,
    RetentionCorpus, RetentionMeasurement, RetentionProbe, RetentionReport,
    topic_centroid, TOPIC_METADATA_KEY,
};

use async_trait::async_trait;
use nanna_gpu::{CosineSimilaritySearch, GpuContext};
use nanna_simd::{cosine_similarity_f32, normalize_f32};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

#[derive(Error, Debug)]
pub enum MemoryError {
    #[error("Embedding dimension mismatch: expected {expected}, got {got}")]
    DimensionMismatch { expected: usize, got: usize },
    #[error("Memory not found: {0}")]
    NotFound(String),
    /// No embedding provider is wired up, so anything needing a vector cannot run.
    ///
    /// This is its own variant because it is the one memory failure a *user* can
    /// fix, and the message is read by the agent: it used to be reported as
    /// `MemoryError::Io(NotConnected, "No embedding function configured")`, which
    /// surfaced to the model as `IO error: ...`. A model cannot act on an IO
    /// error — it retries or gives up — so `recall` looked broken rather than
    /// unconfigured. Keep the text actionable and addressed to the person.
    #[error(
        "no embedding provider configured — set one in Settings, or run a local Ollama \
         with an embedding model pulled, then memory search will work"
    )]
    NoEmbeddingProvider,
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
/// Result of loading the durable store, including any rows skipped as corrupt.
#[derive(Debug, Default)]
pub struct LoadReport {
    pub entries: Vec<MemoryEntry>,
    /// Rows that existed but could not be read (corrupt payload / overflow chain).
    pub corrupt_rows: usize,
    /// Total rows the table reported (`entries.len() + corrupt_rows` on a salvage).
    pub expected: usize,
}

/// Health of the durable memory store after the startup load. `degraded` is true
/// when any row was unreadable, so it can be surfaced instead of the silent
/// "loaded 0 of N" that a whole-table corruption used to cause.
#[derive(Debug, Clone, Copy, Default)]
pub struct MemoryStoreHealth {
    pub degraded: bool,
    pub corrupt_rows: usize,
    pub loaded: usize,
    pub expected: usize,
}

#[async_trait]
pub trait MemoryPersistence: Send + Sync {
    /// Persist (insert or update) a single entry.
    async fn save_entry(&self, entry: &MemoryEntry) -> Result<(), MemoryError>;
    /// Remove an entry by its ID.
    async fn remove_entry(&self, id: &str) -> Result<(), MemoryError>;
    /// Remove multiple entries by ID in one batch.
    ///
    /// The default loops [`remove_entry`](Self::remove_entry); a backing store
    /// that can amortize per-delete cost across the batch (e.g. a single WAL
    /// checkpoint for the whole set, rather than one per row) overrides this. The
    /// dream cycle removes a cluster's superseded sources through this path, so a
    /// per-row checkpoint there would fsync once per consolidated memory.
    async fn remove_entries(&self, ids: &[&str]) -> Result<(), MemoryError> {
        for id in ids {
            self.remove_entry(id).await?;
        }
        Ok(())
    }
    /// Update only the FSRS cognitive state for an existing entry.
    async fn update_entry_fsrs(&self, id: &str, fsrs: &FsrsState) -> Result<(), MemoryError>;
    /// Update only the text content for an existing entry.
    async fn update_entry_content(&self, id: &str, content: &str) -> Result<(), MemoryError>;
    /// Replace the stored chunk set for `memory_id` with `chunks`.
    ///
    /// Whole-set replacement, not a patch: chunk boundaries move when content
    /// changes, so an ordinal that meant one span before means a different span
    /// after, and merging the two would interleave text from two different
    /// splittings. Deleting and rewriting is the only operation whose result
    /// does not depend on what was there.
    ///
    /// The default is a no-op, which is correct for a store with no durable
    /// backing: chunks are derived data, reproducible from content at any time.
    async fn replace_chunks(
        &self,
        _memory_id: &str,
        _workspace_id: Option<&str>,
        _chunks: &[ChunkWrite],
    ) -> Result<(), MemoryError> {
        Ok(())
    }

    /// Chunks with no vector for `model` yet, oldest first, at most `limit`.
    ///
    /// "No vector" and "a vector from a different model" are the same state
    /// here, and that is what makes an embedding-model switch a resumable
    /// incremental backfill instead of a full rebuild: vectors from two models
    /// live in unrelated spaces, so a chunk carrying another model's vector is
    /// exactly as unsearchable under `model` as a chunk carrying none.
    ///
    /// Returns `(chunk_id, content)`. Default: nothing to do.
    async fn chunks_needing_embedding(
        &self,
        _model: &str,
        _limit: usize,
    ) -> Result<Vec<(i64, String)>, MemoryError> {
        Ok(Vec::new())
    }

    /// Attach `embedding` to the chunk with `chunk_id`, stamped with `model`.
    async fn set_chunk_embedding(
        &self,
        _chunk_id: i64,
        _embedding: &[f32],
        _model: &str,
    ) -> Result<(), MemoryError> {
        Ok(())
    }

    /// Chunk-level nearest neighbours, as `(memory_id, ordinal, similarity)`.
    ///
    /// Restricted to chunks embedded by `model`. Comparing across models is
    /// meaningless — equal width is not equal vector space — and it fails
    /// silently rather than loudly, so the model is a required argument here
    /// instead of an optional filter.
    ///
    /// Similarity, not distance — the caller compares it against the same
    /// calibrated cosine threshold every other score is compared against, and
    /// a path that returned distance here would silently invert every
    /// comparison downstream.
    async fn search_chunks(
        &self,
        _query: &[f32],
        _model: &str,
        _limit: usize,
        _workspace_id: Option<&str>,
    ) -> Result<Vec<(String, i64, f32)>, MemoryError> {
        Ok(Vec::new())
    }

    /// Memory ids that have no chunk rows at all, at most `limit`.
    ///
    /// These are memories written before chunking existed. They are searchable
    /// by their whole-row vector, so this is a quality backfill rather than a
    /// repair — but a store where only new memories are chunk-searchable
    /// retrieves inconsistently, which is worse than either extreme.
    async fn parents_without_chunks(&self, _limit: usize) -> Result<Vec<String>, MemoryError> {
        Ok(Vec::new())
    }

    /// Load all persisted entries (called on startup to populate the in-memory cache).
    async fn load_all(&self) -> Result<Vec<MemoryEntry>, MemoryError>;

    /// Load all entries with a corruption report (rows skipped as unreadable).
    ///
    /// The default delegates to [`load_all`](Self::load_all) and reports zero
    /// corruption; a backing store able to salvage individual rows overrides this
    /// so one bad row no longer aborts the whole load.
    async fn load_all_report(&self) -> Result<LoadReport, MemoryError> {
        let entries = self.load_all().await?;
        let expected = entries.len();
        Ok(LoadReport { entries, corrupt_rows: 0, expected })
    }
}

/// One chunk on its way to durable storage.
///
/// Mirrors the stored row minus the parent identity, which travels alongside as
/// an argument to [`MemoryPersistence::replace_chunks`].
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkWrite {
    pub ordinal: i64,
    pub content: String,
    pub char_start: i64,
    pub char_end: i64,
    /// Usually `None` — chunk vectors are filled by the backfill pass, not on
    /// the write path, because embedding N chunks inline would put N network
    /// round trips in front of every remember. The exception is a memory that
    /// fits in one chunk: its single chunk IS the parent text, so the parent's
    /// vector already describes it exactly and re-embedding would be waste.
    pub embedding: Option<Vec<f32>>,
    pub embedding_model: Option<String>,
    pub chunk_max_chars: i64,
    pub chunker_version: i64,
}

/// A memory entry with embedding, metadata, and FSRS cognitive state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub content: String,
    /// The embedding for the CURRENTLY active model — the one search compares
    /// against. Empty means "not embedded by the active model yet", which is a
    /// backfill job, not an error.
    pub embedding: Vec<f32>,
    /// Which model produced [`Self::embedding`].
    ///
    /// Dimension alone cannot answer this: two different 1024-dim models
    /// produce vectors that compare without asserting and mean nothing. Naming
    /// the model is the only way to tell "the same vector space" from "the same
    /// width".
    #[serde(default)]
    pub embedding_model: Option<String>,
    /// Every embedding ever computed for this entry, keyed by model identity
    /// (`provider:model`), including the active one.
    ///
    /// Providers flap — a rate-limited primary falls over and comes back on the
    /// next call. Throwing away the previous vector on every switch meant each
    /// flap cost a full re-embed of the store, and a flap back cost another.
    /// Retained buckets make a switch-back free: rebinding is a lookup, and only
    /// genuinely new (model, entry) pairs need work.
    #[serde(default)]
    pub embeddings: HashMap<String, Vec<f32>>,
    pub metadata: HashMap<String, String>,
    pub timestamp: i64,
    /// FSRS-6 cognitive state (stability, retrievability, etc.)
    #[serde(default)]
    pub fsrs: FsrsState,
    /// Workspace ID if this memory is scoped to a workspace (None = global)
    #[serde(default)]
    pub workspace_id: Option<String>,
}

/// Vector store configuration
#[derive(Debug)]
pub struct VectorStoreConfig {
    /// Expected embedding dimension. Stored as `AtomicUsize` so it can be
    /// updated at runtime (e.g., when the embedding model changes via fallback)
    /// without requiring `&mut self` on the `VectorStore`.
    pub dimension: std::sync::atomic::AtomicUsize,
    /// Chunk budget in chars for the currently bound embedding model, derived
    /// from that model's input window. Atomic for the same reason `dimension`
    /// is: a provider switch has to move it without `&mut self`.
    ///
    /// Zero is the unbound sentinel, NOT a bound of nothing — see
    /// [`Self::get_chunk_max_chars`].
    pub chunk_max_chars: std::sync::atomic::AtomicUsize,
    pub use_f16: bool,  // Store embeddings as f16 to save memory
}

impl Clone for VectorStoreConfig {
    fn clone(&self) -> Self {
        Self {
            dimension: std::sync::atomic::AtomicUsize::new(
                self.dimension.load(std::sync::atomic::Ordering::Relaxed)
            ),
            chunk_max_chars: std::sync::atomic::AtomicUsize::new(
                self.chunk_max_chars.load(std::sync::atomic::Ordering::Relaxed)
            ),
            use_f16: self.use_f16,
        }
    }
}

impl Default for VectorStoreConfig {
    fn default() -> Self {
        Self {
            dimension: std::sync::atomic::AtomicUsize::new(1536),  // OpenAI ada-002 default
            chunk_max_chars: std::sync::atomic::AtomicUsize::new(0),
            use_f16: true,
        }
    }
}

impl VectorStoreConfig {
    /// Create config with specified dimension
    pub fn with_dimension(dim: usize) -> Self {
        Self {
            dimension: std::sync::atomic::AtomicUsize::new(dim),
            chunk_max_chars: std::sync::atomic::AtomicUsize::new(0),
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

    /// The chunk budget for the currently bound embedding model, in chars.
    ///
    /// Zero means nothing has bound yet, which resolves to the
    /// retrieval-granularity default rather than to "no limit" — see
    /// [`crate::chunking::derive_chunk_params`].
    pub fn get_chunk_max_chars(&self) -> usize {
        let latched = self.chunk_max_chars.load(std::sync::atomic::Ordering::Relaxed);
        if latched == 0 {
            crate::chunking::derive_chunk_params(None).max_chars
        } else {
            latched
        }
    }

    /// Set the chunk budget at runtime.
    ///
    /// Lives beside the dimension latch, and under the same discipline: both
    /// are only ever moved while `MemoryService`'s binding write-lock is held,
    /// so no reader can observe a new model with an old geometry.
    pub fn set_chunk_max_chars(&self, max_chars: usize) {
        self.chunk_max_chars
            .store(max_chars, std::sync::atomic::Ordering::Relaxed);
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
    /// Durable-store health from the last `load_from_db` (degraded if any row was
    /// unreadable). Surfaced so a corrupt store isn't a silent empty one.
    store_health: RwLock<MemoryStoreHealth>,
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
            store_health: RwLock::new(MemoryStoreHealth::default()),
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
                            store_health: RwLock::new(MemoryStoreHealth::default()),
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

        let report = match db.load_all_report().await {
            Ok(r) => r,
            Err(e) => {
                // Whole-store load failed (e.g. btree corruption the salvage could
                // not localize to a single row). Mark the store degraded so
                // `/status` doesn't report a corrupt store as healthy-but-empty,
                // then surface the error as before.
                *self.store_health.write().await = MemoryStoreHealth {
                    degraded: true,
                    corrupt_rows: 0,
                    loaded: 0,
                    expected: 0,
                };
                error!("Memory store load FAILED ({e}); marked degraded (0 loaded)");
                return Err(e);
            }
        };
        let count = report.entries.len();

        // Record + surface durable-store health. A salvage load skips only the
        // corrupt rows rather than dropping the whole table, so make any loss
        // loud instead of the silent "loaded 0 of N" a corrupt store used to give.
        // `degraded` tracks genuine corruption (data loss); benign no-embedding
        // rows (readable, just not searchable) don't count.
        let degraded = report.corrupt_rows > 0;
        *self.store_health.write().await = MemoryStoreHealth {
            degraded,
            corrupt_rows: report.corrupt_rows,
            loaded: count,
            expected: report.expected,
        };
        if degraded {
            error!(
                "Memory store DEGRADED: loaded {} of {} rows; {} unreadable (corrupt). \
                 Those memories are lost until the store is repaired.",
                count, report.expected, report.corrupt_rows
            );
        }

        let mismatched = report.entries.iter()
            .filter(|e| e.embedding.len() != self.config.get_dimension())
            .count();

        if mismatched > 0 {
            let sample_dim = report.entries.iter()
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
        *entries = report.entries;
        info!("Loaded {} entries from persistence backend", count);
        Ok(count)
    }

    /// Durable-store health recorded by the last [`load_from_db`](Self::load_from_db).
    /// `degraded` is true if any row was unreadable (corrupt).
    pub async fn store_health(&self) -> MemoryStoreHealth {
        *self.store_health.read().await
    }

    /// Check if GPU acceleration is available.
    #[must_use]
    pub fn has_gpu(&self) -> bool {
        self.gpu.is_some() && self.gpu_pipeline.is_some()
    }

    /// Add a memory entry.
    ///
    /// An EMPTY active embedding is legal: it is the queued-for-backfill state
    /// ([`MemoryEntry::embedding`] documents it) that a write racing a provider
    /// switch lands in — the entry is kept, unsearchable until backfill embeds
    /// it under the current binding. Only a non-empty vector claims to be
    /// searchable, so only that is held to the bound width.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError::DimensionMismatch` if a non-empty embedding has
    /// the wrong dimension.
    pub async fn add(&self, mut entry: MemoryEntry) -> Result<(), MemoryError> {
        if !entry.embedding.is_empty() && entry.embedding.len() != self.config.get_dimension() {
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

        // Chunks follow the content, on EVERY path that writes content — not
        // just `remember`. The consolidation and dream paths mutate entries
        // through these primitives directly, and a chunk set that only tracked
        // the ingest API would go stale exactly where memories change most.
        self.write_chunks(
            &entry.id,
            &entry.content,
            entry.workspace_id.as_deref(),
            entry
                .embedding_model
                .as_deref()
                .filter(|_| !entry.embedding.is_empty())
                .map(|m| (m, entry.embedding.as_slice())),
        )
        .await;

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
    /// Cosine against a stored embedding, tolerating one of the wrong width.
    ///
    /// `cosine_similarity_f32` opens with `assert_eq!(a.len(), b.len())` — a
    /// hard assert, and this crate's release profile sets `panic = "abort"`, so
    /// a single stale row takes the whole daemon down. The search guard above
    /// only validates the QUERY against config; it says nothing about what is
    /// in the store, and the two genuinely can differ: changing embedding
    /// provider changes the width, while every row already persisted keeps the
    /// old one.
    ///
    /// A stale row is not a non-match, it is a row that cannot be compared yet.
    /// Scoring it at the cosine floor keeps it out of every result without
    /// pretending it was evaluated — and, unlike the assert, leaves the process
    /// alive to re-embed it.
    fn cosine_or_stale(query: &[f32], embedding: &[f32]) -> f32 {
        if query.len() == embedding.len() {
            cosine_similarity_f32(query, embedding)
        } else {
            -1.0
        }
    }

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

        // The GPU path flattens every embedding into one width-uniform buffer,
        // so a single stale or queued-for-backfill row (empty vector after a
        // rebind, old width after a provider switch) would misalign the whole
        // batch. Those rows can exist by design; when any is present, take the
        // SIMD path, which scores them at the stale floor row-by-row.
        let similarities: Vec<f32> = if entry_count >= GPU_THRESHOLD
            && self.has_gpu()
            && entries.iter().all(|e| e.embedding.len() == query.len())
        {
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
                        .map(|entry| Self::cosine_or_stale(&query, &entry.embedding))
                        .collect()
                }
            }
        } else {
            // SIMD path — fast for all practical memory store sizes.
            // `cosine_or_stale`, not the raw cosine: the raw kernel asserts
            // equal widths and this profile aborts on panic, so one stale or
            // queued row would take the daemon down mid-search.
            debug!("Using SIMD ({}) for {} vectors", nanna_simd::simd_tier(), entry_count);
            entries
                .iter()
                .map(|entry| Self::cosine_or_stale(&query, &entry.embedding))
                .collect()
        };

        // Rank by INDEX, then clone only the winners.
        //
        // This used to clone EVERY entry — embeddings included — into a Vec,
        // sort that, and then throw all but `top_k` away. At 2048 dimensions
        // each discarded clone is ~8 KB of memcpy, and `remember_scoped` runs a
        // search on every single ingest, so the cost was O(store) per write and
        // O(store²) across a run. Capturing every tool call raises the write
        // count roughly forty-fold, which would have turned that from wasteful
        // into a stall.
        let mut ranked: Vec<(usize, f32)> = similarities.into_iter().enumerate().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(top_k);
        let scored: Vec<(MemoryEntry, f32)> = ranked
            .into_iter()
            .map(|(idx, sim)| (entries[idx].clone(), sim))
            .collect();
        drop(entries);
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

    /// Remove many entries by ID in one batch.
    ///
    /// Best-effort like [`remove`](Self::remove): an id absent from the in-RAM
    /// cache is skipped rather than erroring, and a persistence failure is logged
    /// but non-fatal (the dream cycle that drives this must not abort a whole
    /// consolidation because one source removal failed). The batch takes the
    /// entries write-lock **once** and issues **one** persistence
    /// [`remove_entries`](MemoryPersistence::remove_entries) call, so a backing
    /// store that batches (Turso) checkpoints once for the set instead of per row.
    ///
    /// Returns the number of entries actually removed from the in-RAM cache.
    pub async fn remove_many(&self, ids: &[&str]) -> usize {
        if ids.is_empty() {
            return 0;
        }
        let requested = ids.len();
        let id_set: std::collections::HashSet<&str> = ids.iter().copied().collect();

        let mut entries = self.entries.write().await;
        let before = entries.len();
        entries.retain(|e| !id_set.contains(e.id.as_str()));
        let removed = before - entries.len();
        drop(entries);

        debug_assert!(
            removed <= requested,
            "removed more entries than ids requested"
        );

        // Write-through: one batched persistence call for the whole set.
        if let Some(ref db) = self.db {
            if let Err(e) = db.remove_entries(ids).await {
                warn!(
                    "Failed to batch-remove {} memory entries from persistence: {}",
                    requested, e
                );
                // Non-fatal
            }
        }

        removed
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
        let workspace_id = entry.workspace_id.clone();
        drop(entries);

        // Write-through to persistence backend
        if let Some(ref db) = self.db {
            if let Err(e) = db.update_entry_content(id, content).await {
                warn!("Failed to persist content update for {}: {}", id, e);
                // Non-fatal
            }
        }

        // New text, new boundaries. The parent's vector is NOT reused as a
        // single-chunk seed here: this path changes content without touching
        // the embedding, so that vector still describes the OLD text and
        // seeding it would assert a chunk had been embedded when it had not.
        self.write_chunks(id, content, workspace_id.as_deref(), None).await;

        Ok(())
    }

    /// Replace an entry's content **and** embedding in place (merge/dreaming path).
    ///
    /// Keeps the entry's ID and FSRS history; the caller reinforces FSRS
    /// separately. Persists the full updated entry (upsert) so the durable
    /// content and embedding never diverge from the in-memory cache.
    ///
    /// `model` names the binding that produced `embedding`. Rewriting content
    /// invalidates EVERY previously computed vector for this entry, so the
    /// bucket map is reset to just the incoming vector — leaving other models'
    /// buckets in place would let a later rebind resurrect a vector for text
    /// that no longer exists.
    ///
    /// An empty `embedding` is the queued-for-backfill state (same convention
    /// as [`Self::add`]): the content lands, the vectors are cleared, and the
    /// entry is unsearchable until backfill re-embeds it under the current
    /// binding.
    ///
    /// # Errors
    ///
    /// Returns `DimensionMismatch` if a non-empty `embedding` has the wrong
    /// dimension, or `NotFound` if no entry has the given `id`.
    pub async fn update_content_and_embedding(
        &self,
        id: &str,
        content: &str,
        mut embedding: Vec<f32>,
        model: Option<&str>,
    ) -> Result<(), MemoryError> {
        debug_assert!(!id.is_empty(), "memory id must not be empty");
        debug_assert!(!content.is_empty(), "merged content must not be empty");
        if !embedding.is_empty() && embedding.len() != self.config.get_dimension() {
            return Err(MemoryError::DimensionMismatch {
                expected: self.config.get_dimension(),
                got: embedding.len(),
            });
        }

        // Normalize for cosine similarity, matching `add`.
        normalize_f32(&mut embedding);

        // Update the in-memory entry, snapshot it, then release the lock.
        let mut entries = self.entries.write().await;
        let entry = entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| MemoryError::NotFound(id.to_string()))?;
        entry.content = content.to_string();
        entry.embeddings.clear();
        if embedding.is_empty() {
            entry.embedding_model = None;
        } else {
            entry.embedding_model = model.map(str::to_string);
            if let Some(model) = model {
                entry.embeddings.insert(model.to_string(), embedding.clone());
            }
        }
        entry.embedding = embedding;
        let snapshot = entry.clone();
        drop(entries);

        // Write-through the whole entry (content + embedding stay consistent).
        if let Some(ref db) = self.db
            && let Err(e) = db.save_entry(&snapshot).await
        {
            warn!("Failed to persist merged memory {}: {}", id, e);
            // Non-fatal: in-memory cache already updated.
        }

        self.write_chunks(
            id,
            content,
            snapshot.workspace_id.as_deref(),
            snapshot
                .embedding_model
                .as_deref()
                .filter(|_| !snapshot.embedding.is_empty())
                .map(|m| (m, snapshot.embedding.as_slice())),
        )
        .await;

        debug_assert_eq!(snapshot.id, id, "merged entry id must be unchanged");
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
    ///
    /// # Errors
    /// Returns `MemoryError` if serialization or the file write/rename fails.
    pub async fn save(&self, path: &std::path::Path) -> Result<(), MemoryError> {
        warn!("VectorStore::save() is deprecated. Use Turso persistence instead.");
        // Serialize under the read lock, then release it before the file IO so the
        // lock is never held across an `.await`.
        let (json, count) = {
            let entries = self.entries.read().await;
            (serde_json::to_string_pretty(&*entries)?, entries.len())
        };
        // Atomic write: write to a sibling temp file, then rename over the target.
        // `fs::write` truncates in place, so a crash mid-write would leave a
        // corrupt/empty store; rename-into-place is atomic on the same filesystem.
        let tmp_path = path.with_extension("json.tmp");
        tokio::fs::write(&tmp_path, json).await?;
        tokio::fs::rename(&tmp_path, path).await?;
        info!("Saved {} entries to {:?}", count, path);
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
    /// Point every entry at its bucket for `model`, keeping all other buckets.
    ///
    /// This is what a provider switch costs now: a hash lookup per entry, no
    /// network, no LLM. An entry that has been embedded by this model before —
    /// because the router used it earlier and flapped away — comes back
    /// instantly. An entry that has not is left with an empty active vector,
    /// which search skips and [`Self::entries_missing_model`] reports for
    /// backfill.
    ///
    /// Returns `(rebound, missing)`.
    pub async fn rebind_to_model(&self, model: &str) -> (usize, usize) {
        let mut entries = self.entries.write().await;
        let (mut rebound, mut missing) = (0usize, 0usize);
        for entry in entries.iter_mut() {
            // Retain whatever the active vector currently is, so switching AWAY
            // from a model never discards the work that produced it.
            //
            // Rows written before buckets existed carry a vector but no model
            // name. Dropping those on the first rebind would throw away the
            // entire store's existing embeddings — so they are kept under a
            // width-tagged legacy key. That key can never be *matched* by a real
            // provider (we do not know which model made it, and equal width is
            // not equal vector space), but keeping it costs almost nothing and
            // discarding it is irreversible.
            if !entry.embedding.is_empty() {
                let key = entry
                    .embedding_model
                    .clone()
                    .unwrap_or_else(|| format!("legacy-{}d", entry.embedding.len()));
                entry.embeddings.entry(key).or_insert_with(|| entry.embedding.clone());
            }
            match entry.embeddings.get(model) {
                Some(vector) => {
                    entry.embedding = vector.clone();
                    entry.embedding_model = Some(model.to_string());
                    rebound += 1;
                }
                None => {
                    entry.embedding.clear();
                    entry.embedding_model = None;
                    missing += 1;
                }
            }
        }
        (rebound, missing)
    }

    /// Ids and content of entries with no bucket for `model`, oldest first.
    ///
    /// `limit` bounds one backfill pass so the filler yields between batches
    /// instead of monopolising a rate-limited provider.
    pub async fn entries_missing_model(&self, model: &str, limit: usize) -> Vec<(String, String)> {
        let entries = self.entries.read().await;
        entries
            .iter()
            .filter(|e| !e.embeddings.contains_key(model))
            .take(limit)
            .map(|e| (e.id.clone(), e.content.clone()))
            .collect()
    }

    /// Record `embedding` as `model`'s vector for `id`, activating it when
    /// `model` is the one currently bound.
    ///
    /// # Errors
    ///
    /// Returns `DimensionMismatch` when `activate` is set and the vector does
    /// not match the bound width — an activated vector goes straight into the
    /// search path, so this is the last gate against a backfill racing a
    /// provider switch.
    pub async fn set_embedding_for_model(
        &self,
        id: &str,
        model: &str,
        embedding: Vec<f32>,
        activate: bool,
    ) -> Result<(), MemoryError> {
        if activate && embedding.len() != self.config.get_dimension() {
            return Err(MemoryError::DimensionMismatch {
                expected: self.config.get_dimension(),
                got: embedding.len(),
            });
        }
        let mut entries = self.entries.write().await;
        let Some(entry) = entries.iter_mut().find(|e| e.id == id) else {
            return Err(MemoryError::NotFound(id.to_string()));
        };
        entry.embeddings.insert(model.to_string(), embedding.clone());
        if activate {
            entry.embedding = embedding;
            entry.embedding_model = Some(model.to_string());
        }
        let snapshot = entry.clone();
        drop(entries);
        // Non-fatal, like every other write-through here: the in-memory cache is
        // already correct, and a backfill that fails to persist just repeats on
        // the next restart rather than losing the memory.
        if let Some(ref db) = self.db
            && let Err(e) = db.save_entry(&snapshot).await
        {
            warn!("Failed to persist backfilled embedding for {}: {}", snapshot.id, e);
        }
        Ok(())
    }

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

    /// Chunk geometry for the currently bound embedding model.
    #[must_use]
    pub fn chunk_params(&self) -> crate::chunking::ChunkParams {
        crate::chunking::ChunkParams {
            max_chars: self.config.get_chunk_max_chars(),
            version: crate::chunking::CHUNKER_VERSION,
        }
    }

    /// Move the chunk budget. Only called under `MemoryService`'s binding
    /// write-lock, beside `set_dimension`.
    pub fn set_chunk_params(&self, params: crate::chunking::ChunkParams) {
        self.config.set_chunk_max_chars(params.max_chars);
    }

    /// Chunk-level nearest neighbours for `query`, collapsed to one entry per
    /// parent memory.
    ///
    /// Over-fetches deliberately: `limit` is the number of MEMORIES wanted, but
    /// several of the top chunks routinely belong to the same memory, so asking
    /// the index for exactly `limit` chunks would return fewer memories than
    /// asked for whenever a memory matched more than once — and would do it
    /// most often for exactly the memories that matched best.
    pub async fn search_chunks(
        &self,
        query: &[f32],
        model: &str,
        limit: usize,
        workspace_id: Option<&str>,
        min_score: f32,
    ) -> HashMap<String, crate::chunk_rank::ChunkHit> {
        let Some(ref db) = self.db else { return HashMap::new() };
        // No bound model means no way to say which vector space the query
        // lives in, and chunk vectors carry the model that made them. Refusing
        // to search is the only honest answer; guessing would compare across
        // spaces.
        if query.is_empty() || model.is_empty() || limit == 0 {
            return HashMap::new();
        }
        /// Chunks fetched per memory wanted. Four is the point past which the
        /// over-fetch stops changing which memories come back in practice,
        /// while keeping the SQL scan bounded.
        const CHUNK_OVERFETCH: usize = 4;
        let hits = db
            .search_chunks(query, model, limit.saturating_mul(CHUNK_OVERFETCH), workspace_id)
            .await
            .unwrap_or_default();
        crate::chunk_rank::collapse_chunk_hits(&hits, min_score)
    }

    /// Chunks awaiting a vector under `model`, as `(chunk_id, content)`.
    pub async fn chunks_needing_embedding(
        &self,
        model: &str,
        limit: usize,
    ) -> Vec<(i64, String)> {
        match self.db {
            Some(ref db) => db.chunks_needing_embedding(model, limit).await.unwrap_or_default(),
            None => Vec::new(),
        }
    }

    /// Attach a backfilled vector to a chunk.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError::Persistence` if the write fails.
    pub async fn set_chunk_embedding(
        &self,
        chunk_id: i64,
        embedding: &[f32],
        model: &str,
    ) -> Result<(), MemoryError> {
        match self.db {
            Some(ref db) => db.set_chunk_embedding(chunk_id, embedding, model).await,
            None => Ok(()),
        }
    }

    /// Memories with no chunk rows yet — written before chunking existed.
    pub async fn parents_without_chunks(&self, limit: usize) -> Vec<String> {
        match self.db {
            Some(ref db) => db.parents_without_chunks(limit).await.unwrap_or_default(),
            None => Vec::new(),
        }
    }

    /// Re-split and store the chunk set for the in-RAM entry `id`.
    ///
    /// Used by the backfill to chunk memories that predate chunking. Reads the
    /// entry from the cache rather than the database because the cache is the
    /// authority on current content — a memory updated in this session has its
    /// new text here before the write-through completes.
    pub async fn rechunk(&self, id: &str) -> Result<(), MemoryError> {
        let snapshot = {
            let entries = self.entries.read().await;
            entries
                .iter()
                .find(|e| e.id == id)
                .cloned()
                .ok_or_else(|| MemoryError::NotFound(id.to_string()))?
        };
        self.write_chunks(
            &snapshot.id,
            &snapshot.content,
            snapshot.workspace_id.as_deref(),
            snapshot
                .embedding_model
                .as_deref()
                .filter(|_| !snapshot.embedding.is_empty())
                .map(|m| (m, snapshot.embedding.as_slice())),
        )
        .await;
        Ok(())
    }

    /// Re-split `content` and replace the stored chunk set for `id`.
    ///
    /// Called after every mutation that changes a memory's text. Chunks are
    /// derived data, so a failure here is logged and swallowed exactly like the
    /// entry write-throughs around it: losing the retrieval index for one
    /// memory must not fail the write that produced it, and the backfill pass
    /// rebuilds what is missing.
    ///
    /// `parent_vector` is the entry's own embedding, if it has one under the
    /// active model. When the content fits in a single chunk that vector
    /// already describes the chunk exactly — same text, same model — so it is
    /// reused and the backfill has nothing to do. Multi-chunk memories get
    /// text-only rows and are embedded by the backfill pass; doing it inline
    /// would put one network round trip per chunk in front of every write.
    async fn write_chunks(
        &self,
        id: &str,
        content: &str,
        workspace_id: Option<&str>,
        parent_vector: Option<(&str, &[f32])>,
    ) {
        let Some(ref db) = self.db else { return };
        let params = self.chunk_params();
        let pieces = crate::chunking::chunk_text(content, &params);

        let single = pieces.len() == 1;
        let writes: Vec<ChunkWrite> = pieces
            .into_iter()
            .map(|c| {
                let (embedding, embedding_model) = match parent_vector {
                    Some((model, vector)) if single => {
                        (Some(vector.to_vec()), Some(model.to_string()))
                    }
                    _ => (None, None),
                };
                ChunkWrite {
                    ordinal: c.ordinal,
                    content: c.content,
                    char_start: c.char_start,
                    char_end: c.char_end,
                    embedding,
                    embedding_model,
                    chunk_max_chars: i64::try_from(params.max_chars).unwrap_or(i64::MAX),
                    chunker_version: params.version,
                }
            })
            .collect();

        if let Err(e) = db.replace_chunks(id, workspace_id, &writes).await {
            warn!("Failed to write chunks for memory {}: {} (retrieval for this memory falls back to its whole-row vector until the backfill rebuilds them)", id, e);
        }
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
            chunk_max_chars: std::sync::atomic::AtomicUsize::new(0),
            use_f16: false,
        };
        let store = VectorStore::new(config);

        let entry = MemoryEntry {
            embedding_model: None,
            embeddings: HashMap::new(),
            id: "test1".to_string(),
            content: "Hello world".to_string(),
            embedding: vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            metadata: HashMap::new(),
            timestamp: 0,
            fsrs: FsrsState::default(),
            workspace_id: None,
        };

        store.add(entry).await.unwrap();
        assert_eq!(store.len().await, 1);

        let results = store
            .search(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 10)
            .await;
        assert_eq!(results.len(), 1);
        assert!(results[0].1 > 0.99);  // Should be very similar
    }

    fn health_test_entry(id: &str) -> MemoryEntry {
        MemoryEntry {
            embedding_model: None,
            embeddings: HashMap::new(),
            id: id.to_string(),
            content: "c".to_string(),
            embedding: vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            metadata: HashMap::new(),
            timestamp: 0,
            fsrs: FsrsState::default(),
            workspace_id: None,
        }
    }

    // A persistence whose salvage load reports corruption — proves store_health
    // surfacing without needing a real corrupt DB.
    struct DegradedDb;
    #[async_trait]
    impl MemoryPersistence for DegradedDb {
        async fn save_entry(&self, _e: &MemoryEntry) -> Result<(), MemoryError> { Ok(()) }
        async fn remove_entry(&self, _id: &str) -> Result<(), MemoryError> { Ok(()) }
        async fn update_entry_fsrs(&self, _id: &str, _f: &FsrsState) -> Result<(), MemoryError> { Ok(()) }
        async fn update_entry_content(&self, _id: &str, _c: &str) -> Result<(), MemoryError> { Ok(()) }
        async fn load_all(&self) -> Result<Vec<MemoryEntry>, MemoryError> {
            Ok(vec![health_test_entry("ok0"), health_test_entry("ok1")])
        }
        async fn load_all_report(&self) -> Result<LoadReport, MemoryError> {
            Ok(LoadReport {
                entries: vec![health_test_entry("ok0"), health_test_entry("ok1")],
                corrupt_rows: 3,
                expected: 5,
            })
        }
    }

    // A persistence with no salvage override — the default load_all_report reports
    // zero corruption.
    struct CleanDb;
    #[async_trait]
    impl MemoryPersistence for CleanDb {
        async fn save_entry(&self, _e: &MemoryEntry) -> Result<(), MemoryError> { Ok(()) }
        async fn remove_entry(&self, _id: &str) -> Result<(), MemoryError> { Ok(()) }
        async fn update_entry_fsrs(&self, _id: &str, _f: &FsrsState) -> Result<(), MemoryError> { Ok(()) }
        async fn update_entry_content(&self, _id: &str, _c: &str) -> Result<(), MemoryError> { Ok(()) }
        async fn load_all(&self) -> Result<Vec<MemoryEntry>, MemoryError> {
            Ok(vec![health_test_entry("ok")])
        }
    }

    /// Records every chunk-set replacement so a test can assert what the
    /// mutation paths actually wrote.
    struct ChunkRecordingDb {
        calls: std::sync::Mutex<Vec<(String, Vec<ChunkWrite>)>>,
    }

    impl ChunkRecordingDb {
        fn new() -> Self {
            Self { calls: std::sync::Mutex::new(Vec::new()) }
        }
        fn last(&self) -> (String, Vec<ChunkWrite>) {
            self.calls.lock().unwrap().last().cloned().expect("a chunk write happened")
        }
        fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    #[async_trait::async_trait]
    impl MemoryPersistence for ChunkRecordingDb {
        async fn save_entry(&self, _e: &MemoryEntry) -> Result<(), MemoryError> { Ok(()) }
        async fn remove_entry(&self, _id: &str) -> Result<(), MemoryError> { Ok(()) }
        async fn update_entry_fsrs(&self, _id: &str, _f: &FsrsState) -> Result<(), MemoryError> { Ok(()) }
        async fn update_entry_content(&self, _id: &str, _c: &str) -> Result<(), MemoryError> { Ok(()) }
        async fn load_all(&self) -> Result<Vec<MemoryEntry>, MemoryError> { Ok(Vec::new()) }
        async fn replace_chunks(
            &self,
            memory_id: &str,
            _workspace_id: Option<&str>,
            chunks: &[ChunkWrite],
        ) -> Result<(), MemoryError> {
            self.calls.lock().unwrap().push((memory_id.to_string(), chunks.to_vec()));
            Ok(())
        }
    }

    fn chunking_store(db: &Arc<ChunkRecordingDb>, max_chars: usize) -> VectorStore {
        let store = VectorStore::new(VectorStoreConfig {
            dimension: std::sync::atomic::AtomicUsize::new(4),
            chunk_max_chars: std::sync::atomic::AtomicUsize::new(max_chars),
            use_f16: false,
        })
        .with_persistence(db.clone());
        store
    }

    fn chunked_entry(id: &str, content: &str, embedding: Vec<f32>, model: Option<&str>) -> MemoryEntry {
        MemoryEntry {
            id: id.to_string(),
            content: content.to_string(),
            embeddings: HashMap::new(),
            embedding_model: model.map(str::to_string),
            embedding,
            metadata: HashMap::new(),
            timestamp: 0,
            fsrs: FsrsState::default(),
            workspace_id: None,
        }
    }

    /// Chunks have to be written by the STORE primitive, not by the ingest
    /// API. Consolidation and dreaming mutate entries through these directly,
    /// so a chunk set maintained only by `remember` would go stale exactly
    /// where memories change most.
    #[tokio::test]
    async fn adding_an_entry_writes_its_chunks() {
        let db = Arc::new(ChunkRecordingDb::new());
        let store = chunking_store(&db, 40);
        let content = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu";
        store
            .add(chunked_entry("m1", content, vec![1.0, 0.0, 0.0, 0.0], Some("prov:a")))
            .await
            .unwrap();

        let (id, chunks) = db.last();
        assert_eq!(id, "m1");
        assert!(chunks.len() > 1, "content longer than the budget must split");
        let rebuilt: String = chunks.iter().map(|c| c.content.as_str()).collect();
        assert_eq!(rebuilt, content, "the stored chunks must rebuild the memory");
        assert!(
            chunks.iter().all(|c| c.embedding.is_none()),
            "multi-chunk vectors are backfill work, not inline network calls"
        );
        assert!(chunks.iter().all(|c| c.chunk_max_chars == 40));
    }

    /// A memory that fits in one chunk already has a vector for exactly that
    /// text under exactly that model. Re-embedding it would be pure waste, so
    /// the parent vector seeds the single chunk.
    #[tokio::test]
    async fn a_single_chunk_memory_reuses_the_parent_vector() {
        let db = Arc::new(ChunkRecordingDb::new());
        let store = chunking_store(&db, 4000);
        store
            .add(chunked_entry("m2", "short enough", vec![1.0, 0.0, 0.0, 0.0], Some("prov:a")))
            .await
            .unwrap();

        let (_, chunks) = db.last();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].embedding.as_deref(), Some(&[1.0, 0.0, 0.0, 0.0][..]));
        assert_eq!(chunks[0].embedding_model.as_deref(), Some("prov:a"));
    }

    /// `update_content` changes text WITHOUT touching the embedding, so the
    /// parent vector now describes the old text. Seeding it into the new
    /// single chunk would assert that chunk had been embedded when it had not
    /// — a vector claiming to describe text it never saw.
    #[tokio::test]
    async fn a_content_only_update_rechunks_without_seeding_a_stale_vector() {
        let db = Arc::new(ChunkRecordingDb::new());
        let store = chunking_store(&db, 4000);
        store
            .add(chunked_entry("m3", "the harbor light was green", vec![1.0, 0.0, 0.0, 0.0], Some("prov:a")))
            .await
            .unwrap();
        store.update_content("m3", "the harbor light was red").await.unwrap();

        let (id, chunks) = db.last();
        assert_eq!(id, "m3");
        assert_eq!(db.call_count(), 2, "the update must rewrite the chunk set");
        assert_eq!(chunks[0].content, "the harbor light was red");
        assert!(
            chunks[0].embedding.is_none(),
            "the parent vector describes the OLD text and must not be reused"
        );
    }

    /// The merge/dream path replaces content and embedding together, so the
    /// incoming vector genuinely describes the new text.
    #[tokio::test]
    async fn a_merge_rechunks_and_may_seed_the_new_vector() {
        let db = Arc::new(ChunkRecordingDb::new());
        let store = chunking_store(&db, 4000);
        store
            .add(chunked_entry("m4", "first", vec![1.0, 0.0, 0.0, 0.0], Some("prov:a")))
            .await
            .unwrap();
        store
            .update_content_and_embedding("m4", "merged text", vec![0.0, 1.0, 0.0, 0.0], Some("prov:a"))
            .await
            .unwrap();

        let (_, chunks) = db.last();
        assert_eq!(db.call_count(), 2);
        assert_eq!(chunks[0].content, "merged text");
        assert_eq!(chunks[0].embedding_model.as_deref(), Some("prov:a"));
    }

    #[tokio::test]
    async fn store_health_reflects_salvage_report() {
        let config = VectorStoreConfig {
            dimension: std::sync::atomic::AtomicUsize::new(8),
            chunk_max_chars: std::sync::atomic::AtomicUsize::new(0),
            use_f16: false,
        };
        let store = VectorStore::new(config).with_persistence(Arc::new(DegradedDb));
        let loaded = store.load_from_db().await.unwrap();
        assert_eq!(loaded, 2);
        let h = store.store_health().await;
        assert!(h.degraded);
        assert_eq!(h.corrupt_rows, 3);
        assert_eq!(h.loaded, 2);
        assert_eq!(h.expected, 5);
    }

    #[tokio::test]
    async fn store_health_clean_when_no_corruption() {
        let config = VectorStoreConfig {
            dimension: std::sync::atomic::AtomicUsize::new(8),
            chunk_max_chars: std::sync::atomic::AtomicUsize::new(0),
            use_f16: false,
        };
        let store = VectorStore::new(config).with_persistence(Arc::new(CleanDb));
        store.load_from_db().await.unwrap();
        let h = store.store_health().await;
        assert!(!h.degraded);
        assert_eq!(h.corrupt_rows, 0);
        assert_eq!(h.loaded, 1);
        assert_eq!(h.expected, 1);
    }

    // A persistence whose whole load fails (e.g. btree corruption the salvage
    // can't localize to a row).
    struct FailingDb;
    #[async_trait]
    impl MemoryPersistence for FailingDb {
        async fn save_entry(&self, _e: &MemoryEntry) -> Result<(), MemoryError> { Ok(()) }
        async fn remove_entry(&self, _id: &str) -> Result<(), MemoryError> { Ok(()) }
        async fn update_entry_fsrs(&self, _id: &str, _f: &FsrsState) -> Result<(), MemoryError> { Ok(()) }
        async fn update_entry_content(&self, _id: &str, _c: &str) -> Result<(), MemoryError> { Ok(()) }
        async fn load_all(&self) -> Result<Vec<MemoryEntry>, MemoryError> {
            Err(MemoryError::Persistence("inconsistent overflow chain".into()))
        }
    }

    #[tokio::test]
    async fn store_health_degraded_when_whole_load_fails() {
        // A whole-store load failure must still mark the store degraded, so
        // /status can't report a corrupt store as healthy-but-empty.
        let config = VectorStoreConfig {
            dimension: std::sync::atomic::AtomicUsize::new(8),
            chunk_max_chars: std::sync::atomic::AtomicUsize::new(0),
            use_f16: false,
        };
        let store = VectorStore::new(config).with_persistence(Arc::new(FailingDb));
        assert!(store.load_from_db().await.is_err());
        let h = store.store_health().await;
        assert!(h.degraded, "a whole-store load failure must mark degraded");
        assert_eq!(h.loaded, 0);
    }

    // Counts single vs batched removals to prove `remove_many` routes through the
    // batched persistence path (one call) instead of N single `remove_entry`s.
    #[derive(Default)]
    struct CountingRemovalDb {
        single_calls: std::sync::atomic::AtomicUsize,
        batch_calls: std::sync::atomic::AtomicUsize,
        batch_ids_total: std::sync::atomic::AtomicUsize,
    }
    #[async_trait]
    impl MemoryPersistence for CountingRemovalDb {
        async fn save_entry(&self, _e: &MemoryEntry) -> Result<(), MemoryError> { Ok(()) }
        async fn remove_entry(&self, _id: &str) -> Result<(), MemoryError> {
            self.single_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        async fn remove_entries(&self, ids: &[&str]) -> Result<(), MemoryError> {
            self.batch_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.batch_ids_total
                .fetch_add(ids.len(), std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        async fn update_entry_fsrs(&self, _id: &str, _f: &FsrsState) -> Result<(), MemoryError> { Ok(()) }
        async fn update_entry_content(&self, _id: &str, _c: &str) -> Result<(), MemoryError> { Ok(()) }
        async fn load_all(&self) -> Result<Vec<MemoryEntry>, MemoryError> { Ok(vec![]) }
    }

    fn entry_dim8(id: &str) -> MemoryEntry {
        MemoryEntry {
            embedding_model: None,
            embeddings: HashMap::new(),
            id: id.to_string(),
            content: format!("content {id}"),
            embedding: vec![0.0; 8],
            metadata: HashMap::new(),
            timestamp: 0,
            fsrs: FsrsState::default(),
            workspace_id: None,
        }
    }

    #[tokio::test]
    async fn remove_many_uses_one_batched_persistence_call() {
        use std::sync::atomic::Ordering::SeqCst;
        let db = Arc::new(CountingRemovalDb::default());
        let store = VectorStore::new(VectorStoreConfig {
            dimension: std::sync::atomic::AtomicUsize::new(8),
            chunk_max_chars: std::sync::atomic::AtomicUsize::new(0),
            use_f16: false,
        })
        .with_persistence(db.clone());
        for i in 0..3 {
            store.add(entry_dim8(&format!("m{i}"))).await.unwrap();
        }

        let removed = store.remove_many(&["m0", "m1", "m2"]).await;

        assert_eq!(removed, 3, "all three present entries removed from cache");
        assert_eq!(
            db.batch_calls.load(SeqCst),
            1,
            "exactly one batched persistence call"
        );
        assert_eq!(
            db.batch_ids_total.load(SeqCst),
            3,
            "the batch carried all three ids"
        );
        assert_eq!(
            db.single_calls.load(SeqCst),
            0,
            "no per-row remove_entry calls"
        );
        assert_eq!(store.len().await, 0);
    }

    #[tokio::test]
    async fn remove_many_empty_is_a_noop() {
        let db = Arc::new(CountingRemovalDb::default());
        let store = VectorStore::new(VectorStoreConfig {
            dimension: std::sync::atomic::AtomicUsize::new(8),
            chunk_max_chars: std::sync::atomic::AtomicUsize::new(0),
            use_f16: false,
        })
        .with_persistence(db.clone());
        let removed = store.remove_many(&[]).await;
        assert_eq!(removed, 0);
        assert_eq!(
            db.batch_calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "an empty batch makes no persistence call"
        );
    }

    #[tokio::test]
    async fn test_save_is_atomic_and_roundtrips() {
        let config = VectorStoreConfig {
            dimension: std::sync::atomic::AtomicUsize::new(8),
            chunk_max_chars: std::sync::atomic::AtomicUsize::new(0),
            use_f16: false,
        };
        let store = VectorStore::new(config);
        store
            .add(MemoryEntry {
                embedding_model: None,
                embeddings: HashMap::new(),
                id: "s1".to_string(),
                content: "persist me".to_string(),
                embedding: vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                metadata: HashMap::new(),
                timestamp: 0,
                fsrs: FsrsState::default(),
                workspace_id: None,
            })
            .await
            .unwrap();

        let dir = std::env::temp_dir().join(format!("nanna_mem_save_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("memories.json");

        store.save(&path).await.unwrap();
        // Target written; the temp file was renamed away (not left behind).
        assert!(path.exists());
        assert!(!dir.join("memories.json.tmp").exists());

        // The store round-trips through load into a fresh store.
        let reloaded = VectorStore::new(VectorStoreConfig {
            dimension: std::sync::atomic::AtomicUsize::new(8),
            chunk_max_chars: std::sync::atomic::AtomicUsize::new(0),
            use_f16: false,
        });
        reloaded.load(&path).await.unwrap();
        assert_eq!(reloaded.len().await, 1);

        let _ = std::fs::remove_dir_all(&dir);
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

    /// The queued-for-backfill state must be writable: an entry whose vector
    /// raced a provider switch arrives with an empty embedding, and rejecting
    /// it (the old `0 != dim` mismatch) is exactly the write-failure loop the
    /// incident produced.
    #[tokio::test]
    async fn add_accepts_a_queued_entry_awaiting_backfill() {
        let store = VectorStore::new(VectorStoreConfig {
            dimension: std::sync::atomic::AtomicUsize::new(8),
            chunk_max_chars: std::sync::atomic::AtomicUsize::new(0),
            use_f16: false,
        });
        let mut queued = entry_dim8("queued");
        queued.embedding = Vec::new();
        store.add(queued).await.expect("a queued entry is a legal write");

        // A non-empty wrong-width vector is still a hard error — only the
        // explicit queued state is exempt from the width gate.
        let mut wrong = entry_dim8("wrong");
        wrong.embedding = vec![0.5; 3];
        let err = store.add(wrong).await.expect_err("wrong width still rejected");
        assert!(matches!(err, MemoryError::DimensionMismatch { expected: 8, got: 3 }));
    }

    /// A queued row in the store must be skipped by search, not panic it: the
    /// raw SIMD cosine asserts equal widths and this crate aborts on panic, so
    /// before the fix one queued row took the daemon down on the next recall.
    #[tokio::test]
    async fn search_skips_queued_rows_instead_of_panicking() {
        let store = VectorStore::new(VectorStoreConfig {
            dimension: std::sync::atomic::AtomicUsize::new(8),
            chunk_max_chars: std::sync::atomic::AtomicUsize::new(0),
            use_f16: false,
        });
        let good = MemoryEntry {
            embedding: vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            ..entry_dim8("good")
        };
        store.add(good).await.unwrap();
        let mut queued = entry_dim8("queued");
        queued.embedding = Vec::new();
        store.add(queued).await.unwrap();

        let results = store
            .search(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 10)
            .await;
        assert_eq!(results.len(), 2, "both rows are ranked");
        assert_eq!(results[0].0.id, "good");
        assert!(results[0].1 > 0.99);
        assert!(
            results[1].1 <= -1.0 + f32::EPSILON,
            "the queued row scores at the stale floor, below any min_score"
        );
    }

    /// Rewriting content invalidates every previously computed vector, so the
    /// bucket map must reset to just the producing model's — a later rebind
    /// must never resurrect a vector for text that no longer exists.
    #[tokio::test]
    async fn update_content_and_embedding_resets_stale_buckets() {
        let store = VectorStore::new(VectorStoreConfig {
            dimension: std::sync::atomic::AtomicUsize::new(8),
            chunk_max_chars: std::sync::atomic::AtomicUsize::new(0),
            use_f16: false,
        });
        let mut entry = entry_dim8("m");
        entry.embedding_model = Some("prov:a".into());
        entry.embeddings.insert("prov:a".into(), vec![0.0; 8]);
        entry.embeddings.insert("prov:old".into(), vec![0.0; 4]);
        store.add(entry).await.unwrap();

        store
            .update_content_and_embedding("m", "rewritten", vec![1.0; 8], Some("prov:a"))
            .await
            .unwrap();
        let updated = store.get("m").await.unwrap();
        assert_eq!(updated.content, "rewritten");
        assert_eq!(updated.embedding_model.as_deref(), Some("prov:a"));
        assert_eq!(
            updated.embeddings.keys().collect::<Vec<_>>(),
            vec!["prov:a"],
            "only the producing model's bucket survives a rewrite"
        );

        // The queued form of the same rewrite: content lands, vectors clear.
        store
            .update_content_and_embedding("m", "rewritten again", Vec::new(), None)
            .await
            .unwrap();
        let queued = store.get("m").await.unwrap();
        assert_eq!(queued.content, "rewritten again");
        assert!(queued.embedding.is_empty(), "queued for backfill");
        assert!(queued.embeddings.is_empty());
        assert_eq!(queued.embedding_model, None);
    }
}
