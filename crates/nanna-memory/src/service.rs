//! Memory service - ties together embeddings, storage, and search
//!
//! Integrates FSRS-6 cognitive memory model with vector storage.

use crate::{
    MemoryEntry, MemoryError, VectorStore, VectorStoreConfig,
    FsrsParameters, FsrsState, MemoryState, Rating, IngestAction,
    ConsolidationConfig, ConsolidationResult, CompressionLevel,
    MemoryCluster, cluster_memories, create_consolidated_entry,
};
use crate::chunking::{derive_chunk_params, ChunkParams};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Memory service configuration
#[derive(Debug, Clone)]
pub struct MemoryServiceConfig {
    /// Embedding dimension
    pub dimension: usize,
    /// Minimum similarity score to return results
    pub min_score: f32,
    /// Maximum memories to return from search
    pub max_results: usize,
    /// Minimum weight (retrievability × importance) to return results
    pub min_weight: f32,
    /// FSRS-6 parameters for memory decay
    pub fsrs: FsrsParameters,
}

impl Default for MemoryServiceConfig {
    fn default() -> Self {
        Self {
            dimension: 1536,
            min_score: 0.40, // Lower threshold for semantic matching (0.7 was too strict)
            max_results: 10,
            min_weight: 0.1, // Filter out effectively forgotten memories
            fsrs: FsrsParameters::default(),
        }
    }
}

/// Callback for generating embeddings (injected dependency)
pub type EmbedFn = Arc<dyn Fn(&str) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<f32>, String>> + Send>> + Send + Sync>;

/// What a [`MemoryService::fold_into_memory`] attempt actually did.
///
/// Three outcomes, because two is what caused the loss: the old `bool`
/// collapsed "the content is already there" and "the write was declined" into
/// one value, and the caller could not tell which fallback keeps the content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FoldResult {
    /// The merge was written; `incoming` now lives inside the target entry.
    Merged,
    /// The merge left the target byte-identical — `incoming` is either a true
    /// subset of it or the byte bound declined the append. The caller decides
    /// (by a containment check) whether the content is already present or
    /// must become its own row.
    Subset,
    /// The target's content changed between the caller's read and the write:
    /// the compare-and-swap declined, nothing was touched, and `incoming`
    /// landed NOWHERE. The caller must store it another way.
    Contended,
}

/// What one dream-dedup fold commit ([`MemoryService::commit_duplicate_fold`])
/// actually did.
#[derive(Debug, Clone, PartialEq)]
enum DedupCommit {
    /// The fold committed. Payload: the survivor's new embedding when its
    /// content was rewritten (`None` for a pure superset fold that changed
    /// nothing). The source is now safe to remove — via a MATCHED removal,
    /// so a source rewritten after this commit is still kept.
    Committed(Option<Vec<f32>>),
    /// A concurrent write changed the pair under the fold; nothing was
    /// touched. Both memories stay for the next cycle.
    Stale,
}

/// Memory service for semantic search and recall with FSRS-6 cognitive model
pub struct MemoryService {
    config: MemoryServiceConfig,
    store: VectorStore,
    embed_fn: Option<EmbedFn>,
    /// Track memory IDs that need FSRS updates (for async batch updates)
    pending_updates: RwLock<Vec<(String, Rating)>>,
    /// Runtime-adjustable minimum score (overrides config)
    min_score_override: RwLock<Option<f32>>,
    /// Identity (`provider:model`) of the embedding model currently in use.
    ///
    /// The service only ever sees an opaque `embed_fn`, so it cannot name the
    /// model that produced a vector — but a bucket keyed by anything less than
    /// model identity is unsound, because two models of equal width produce
    /// vectors that compare happily and mean nothing. The daemon owns the
    /// router and stamps the name here on every switch.
    ///
    /// This lock is ALSO the binding lock for the expected dimension: the
    /// store's width latch only ever changes while the write half is held
    /// ([`Self::rebind_embeddings`], [`Self::probe_and_align_dimension`]), so a
    /// reader holding the read half sees `(model, width)` as one consistent
    /// pair. The failover incident this encodes: the router switched 2048→768
    /// and rebound the model, but the width latch stayed at 2048 — every write
    /// for the next several minutes failed `DimensionMismatch` while the log
    /// said the store had been rebound.
    binding: RwLock<EmbeddingBinding>,
    /// Consecutive writes whose vector width disagreed with the binding, and
    /// the width they all reported. See [`MemoryService::note_width_mismatch`].
    width_mismatch_streak: RwLock<(usize, usize)>,
}

/// The identity half of the embedding binding.
///
/// The other two halves — vector width and chunk geometry — are latches on the
/// store, moved ONLY while this lock's write half is held. That is not an
/// accident of layout: a reader that sees the new model with the old width gets
/// `DimensionMismatch` on every write (the 2026-08-02 incident), and a reader
/// that sees the new model with the old chunk size writes chunks the new
/// embedder silently truncates — same split-brain, quieter symptom. Holding the
/// read half is what makes all three consistent.
#[derive(Clone, Debug, Default)]
struct EmbeddingBinding {
    /// `provider:model` of the embedder currently in use, once the daemon says.
    model: Option<String>,
}

impl MemoryService {
    /// Create new memory service
    #[must_use] 
    pub fn new(config: MemoryServiceConfig) -> Self {
        let store_config = VectorStoreConfig {
            dimension: std::sync::atomic::AtomicUsize::new(config.dimension),
            // Unbound until the daemon names an embedding model; reads resolve
            // to the retrieval-granularity default meanwhile.
            chunk_max_chars: std::sync::atomic::AtomicUsize::new(0),
            use_f16: true,
        };
        Self {
            config,
            store: VectorStore::new(store_config),
            embed_fn: None,
            pending_updates: RwLock::new(Vec::new()),
            min_score_override: RwLock::new(None),
            binding: RwLock::new(EmbeddingBinding::default()),
            width_mismatch_streak: RwLock::new((0, 0)),
        }
    }

    /// Set the embedding function (consuming builder form).
    #[must_use]
    pub fn with_embed_fn(mut self, f: EmbedFn) -> Self {
        self.set_embed_fn(f);
        self
    }

    /// Set the embedding function in place.
    ///
    /// The `&mut self` form lets a caller configure a `MemoryService` reached
    /// through a uniquely-held `Arc` (via `Arc::get_mut`) without moving it —
    /// used by [`crate::DreamingService::with_embed_fn`].
    pub fn set_embed_fn(&mut self, f: EmbedFn) {
        self.embed_fn = Some(f);
        debug_assert!(self.embed_fn.is_some(), "embed_fn must be set after assignment");
    }

    /// Attach a Turso (or other) persistence backend to the underlying store.
    ///
    /// Must be called before [`MemoryService::load_from_db`].
    #[must_use]
    pub fn with_persistence(mut self, db: Arc<dyn crate::MemoryPersistence + Send + Sync>) -> Self {
        self.store = self.store.with_persistence(db);
        self
    }

    /// Load all entries from the persistence backend into the in-memory cache.
    ///
    /// Replaces any existing in-memory entries.  Call once on startup after
    /// attaching the persistence layer with [`MemoryService::with_persistence`].
    ///
    /// # Errors
    ///
    /// Returns `MemoryError::Persistence` if the backing store fails.
    pub async fn load_from_db(&self) -> Result<usize, MemoryError> {
        self.store.load_from_db().await
    }

    /// Durable-store health from the last [`load_from_db`](Self::load_from_db) —
    /// `degraded` is true when a corrupt row was skipped. Lets the daemon surface
    /// a degraded/corrupt store via `/status` instead of a silent empty one.
    pub async fn store_health(&self) -> crate::MemoryStoreHealth {
        self.store.store_health().await
    }

    /// Probe the actual embedding dimension and re-embed any mismatched entries.
    ///
    /// Generates a test embedding to detect the real model dimension. If it
    /// differs from config, updates the runtime dimension and re-embeds all
    /// stored entries that have the wrong dimension. This preserves memory
    /// content across embedding model changes.
    ///
    /// This method takes `&self` (not `&mut self`) so it can be called on
    /// `Arc<MemoryService>` at runtime — e.g., when an embedding provider
    /// fallback changes the active model mid-session.
    ///
    /// Returns the actual dimension.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError` if no embedding function is configured or the
    /// probe embedding call fails.
    pub async fn probe_and_align_dimension(&self) -> Result<usize, MemoryError> {
        let embed_fn = self.embed_fn.as_ref().ok_or(MemoryError::NoEmbeddingProvider)?;

        // Generate a test embedding with a short probe string
        let test_embedding = (embed_fn)("dimension probe")
            .await
            .map_err(|e| MemoryError::Io(std::io::Error::other(e)))?;

        let actual_dim = test_embedding.len();
        // Take the binding lock AFTER the embed call returns (never across the
        // await — the daemon's embed_fn rebinds on a provider switch and would
        // deadlock), so the width correction cannot tear a rebind in half.
        let binding = self.binding.write().await;
        let expected_dim = self.store.dimension();
        if actual_dim == expected_dim {
            info!("Embedding dimension confirmed: {}", actual_dim);
        } else {
            warn!(
                "Embedding model returns {} dims, expected {}. Reconfiguring; mismatched \
                 entries will be backfilled.",
                actual_dim, expected_dim
            );
            self.store.set_dimension(actual_dim);
        }
        drop(binding);

        // Vector repair is NOT done here any more — `rebind_embeddings` plus
        // `backfill_embeddings` own it. This function's only remaining job is to
        // keep the configured width honest.
        //
        // It used to re-embed every mismatched row inline, which stopped the
        // world to duplicate work the backfill was already doing. And
        // re-embedding is the wrong answer to a width change regardless: it
        // DISCARDS the previous provider's vectors, so a provider that flaps
        // away and back — precisely what a rate-limited primary does — paid two
        // full passes over the store to end up where it started. Buckets make
        // the return trip a lookup.

        Ok(actual_dim)
    }

    /// Get the current embedding dimension (may differ from initial config
    /// if the embedding model changed at runtime).
    ///
    /// Delegates to the store's latch — the ONE width every write is checked
    /// against. A second copy here is exactly what went stale in the failover
    /// incident, so there isn't one.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.store.dimension()
    }

    /// Get FSRS parameters
    #[must_use]
    pub fn fsrs_params(&self) -> &FsrsParameters {
        &self.config.fsrs
    }

    /// Switch the store onto `model` at `dimension`, reusing every bucket
    /// already computed for it and reporting how many entries still need one.
    ///
    /// This replaces re-embedding on provider switch. Providers flap — a
    /// rate-limited primary fails over and is back on the next call — and
    /// re-embedding the whole store in each direction made a flap cost two full
    /// passes over every memory. Buckets make the return trip free.
    ///
    /// Model, dimension, and chunk geometry move as ONE binding, under one
    /// lock. The 2026-08-02 failover left the first two split: the model
    /// rebound 2048→768 but the width latch did not, so every subsequent write
    /// failed `DimensionMismatch` until a restart. Chunk geometry joins them
    /// because it fails the same way and more quietly — chunks sized for the
    /// old model's window are not rejected by the new one, they are truncated
    /// by it, and the resulting vector describes a prefix while claiming to
    /// describe the whole chunk.
    ///
    /// `dimension` is the width of the vector the new provider just produced —
    /// the caller always has one in hand, because a switch is only ever
    /// observed on a successful embed. `window_tokens` is that model's input
    /// limit, or `None` when the provider does not publish one.
    pub async fn rebind_embeddings(
        &self,
        model: &str,
        dimension: usize,
        window_tokens: Option<usize>,
    ) -> (usize, usize) {
        assert!(dimension > 0, "a provider that produced a vector has a positive width");
        let params = derive_chunk_params(window_tokens);
        let mut binding = self.binding.write().await;
        let previous = self.store.chunk_params();
        binding.model = Some(model.to_string());
        self.store.set_active_model(model).await;
        self.store.set_chunk_params(params);
        self.store.set_dimension(dimension);
        let (rebound, missing) = self.store.rebind_to_model(model).await;
        drop(binding);
        // Chunks have no in-RAM bucket map — their vectors live only on disk —
        // so rebinding them is a durable operation, not a lookup. Restoring
        // this model's stored chunk vectors is what stops a provider that
        // flaps away and back from re-embedding every chunk it had already
        // done, and it clears the matching queue entries so the drain does not
        // redo the work either.
        let restored = self.store.restore_chunk_vectors(model).await;
        if restored > 0 {
            info!("Re-activated {restored} chunk vectors already embedded by '{model}'");
        }
        if missing > 0 {
            info!(
                "Rebound {rebound} memories to '{model}' ({dimension} dims, {} char chunks); \
                 {missing} awaiting backfill (they are unsearchable until then, not lost)",
                params.max_chars
            );
        } else {
            info!(
                "Rebound {rebound} memories to '{model}' ({dimension} dims, {} char chunks) — \
                 nothing to backfill",
                params.max_chars
            );
        }
        // A changed chunk size invalidates every chunk already written, and
        // silently: the rows stay, the vectors stay, and they now describe
        // pieces of a size this model was never asked about. Say so — an
        // unannounced re-chunk is indistinguishable from corruption to whoever
        // reads the store next.
        if params.supersedes(&previous) {
            info!(
                "Chunk geometry changed {} → {} chars (chunker v{}); existing chunks will be \
                 rewritten as their parents are backfilled",
                previous.max_chars, params.max_chars, params.version
            );
        }
        (rebound, missing)
    }

    /// Embed up to `batch` entries that have no vector for `model` yet.
    ///
    /// Lazy on purpose. A switch must not stall the agent behind a full re-embed
    /// of the store, and on a rate-limited provider it could not finish anyway —
    /// so the gap is filled in bounded passes, newest work first, while the run
    /// continues. Returns how many were filled; zero means the store is complete
    /// for this model.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError` if no embedding provider is configured.
    pub async fn backfill_embeddings(&self, model: &str, batch: usize) -> Result<usize, MemoryError> {
        let embed_fn = self.embed_fn.as_ref().ok_or(MemoryError::NoEmbeddingProvider)?;

        // The embed_fn produces vectors from whatever provider is CURRENTLY
        // active — so filling `model`'s bucket is only sound while `model` IS
        // the active binding. Filling any other bucket from here poisons it
        // with another model's vectors (wrong space, usually wrong width).
        // Not hypothetical: the startup backfill kept "completing" a dead
        // primary's bucket with fallback vectors after a mid-run failover.
        if self.active_embedding_model().await.as_deref() != Some(model) {
            debug!("Backfill for '{model}' skipped — it is not the active binding");
            return Ok(0);
        }
        let pending = self.store.entries_missing_model(model, batch).await;
        if pending.is_empty() {
            return Ok(0);
        }

        let mut filled = 0usize;
        for (id, content) in pending {
            match (embed_fn)(&content).await {
                Ok(embedding) => {
                    // Re-check AFTER the embed, not before it: a provider
                    // switch lands inside the embed call (the daemon's
                    // embed_fn rebinds before returning), and the vector in
                    // hand then belongs to the new provider, not `model`.
                    if self.active_embedding_model().await.as_deref() != Some(model) {
                        debug!("Backfill for '{model}' abandoned — provider changed underneath it");
                        break;
                    }
                    if let Err(e) = self
                        .store
                        .set_embedding_for_model(&id, model, embedding, true)
                        .await
                    {
                        debug!("Backfill could not store embedding for {id}: {e}");
                    } else {
                        filled += 1;
                    }
                }
                Err(e) => {
                    // The router already waited out congestion before returning
                    // an error, so this provider is genuinely unavailable. Leave
                    // the rest for the next pass.
                    warn!("Backfill for '{model}' stopped after {filled}: {e}");
                    break;
                }
            }
        }
        if filled > 0 {
            info!("Backfilled {filled} embeddings for '{model}'");
        }
        Ok(filled)
    }

    /// Embed up to `batch` chunks that have no vector for `model` yet, and
    /// chunk up to `batch` memories that have no chunks at all.
    ///
    /// The chunk analogue of [`Self::backfill_embeddings`], with the same
    /// discipline and for the same reasons: the binding is re-checked AFTER
    /// every embed call, because the daemon's `embed_fn` rebinds mid-call on a
    /// provider switch and the vector then in hand belongs to the new provider,
    /// not to `model`. Writing it under `model` would poison that bucket with
    /// another model's vectors — wrong space, usually wrong width — and nothing
    /// downstream could tell.
    ///
    /// Returns `(chunks_embedded, parents_chunked)`. Both zero means the store
    /// is complete for this model.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError` if no embedding provider is configured.
    pub async fn backfill_chunks(
        &self,
        model: &str,
        batch: usize,
    ) -> Result<(usize, usize), MemoryError> {
        let embed_fn = self.embed_fn.as_ref().ok_or(MemoryError::NoEmbeddingProvider)?;

        if self.active_embedding_model().await.as_deref() != Some(model) {
            debug!("Chunk backfill for '{model}' skipped — it is not the active binding");
            return Ok((0, 0));
        }

        // Chunk the unchunked FIRST. Those memories have no chunk rows at all,
        // so they contribute nothing to the embedding queue until they do —
        // running the queue first would report "complete" while whole memories
        // sat outside the index.
        let mut chunked = 0usize;
        for id in self.store.parents_without_chunks(batch).await {
            match self.store.rechunk(&id).await {
                Ok(()) => chunked += 1,
                // A memory in the database but not in the RAM cache is not an
                // error worth stopping for: the cache is authoritative on
                // content, so chunking it from a stale row would write chunks
                // that disagree with the memory.
                Err(e) => debug!("Could not chunk memory {id} yet: {e}"),
            }
        }

        let mut embedded = 0usize;
        for pending in self.store.chunks_needing_embedding(model, batch).await {
            match (embed_fn)(&pending.content).await {
                Ok(embedding) => {
                    if self.active_embedding_model().await.as_deref() != Some(model) {
                        debug!("Chunk backfill for '{model}' abandoned — provider changed underneath it");
                        break;
                    }
                    if embedding.is_empty() {
                        debug!(
                            "Chunk {} embedded to an empty vector; leaving it queued",
                            pending.chunk_id
                        );
                        self.store
                            .record_embedding_failure(
                                &pending.memory_id,
                                pending.ordinal,
                                model,
                                "provider returned an empty vector",
                            )
                            .await;
                        continue;
                    }
                    if let Err(e) = self
                        .store
                        .set_chunk_embedding(pending.chunk_id, &embedding, model)
                        .await
                    {
                        debug!(
                            "Chunk backfill could not store embedding for {}: {e}",
                            pending.chunk_id
                        );
                        self.store
                            .record_embedding_failure(
                                &pending.memory_id,
                                pending.ordinal,
                                model,
                                &e.to_string(),
                            )
                            .await;
                    } else {
                        self.store
                            .dequeue_embedding(&pending.memory_id, pending.ordinal, model)
                            .await;
                        embedded += 1;
                    }
                }
                Err(e) => {
                    // The router already waited out congestion before erroring,
                    // so this provider is genuinely unavailable. Leave the rest
                    // — but WRITE DOWN WHY. A queue that stops with no reason
                    // attached is indistinguishable from a queue that is merely
                    // slow, and that ambiguity is exactly what let 2167
                    // memories sit unembedded for a day.
                    self.store
                        .record_embedding_failure(
                            &pending.memory_id,
                            pending.ordinal,
                            model,
                            &e.to_string(),
                        )
                        .await;
                    warn!("Chunk backfill for '{model}' stopped after {embedded}: {e}");
                    break;
                }
            }
        }

        if embedded > 0 || chunked > 0 {
            info!("Chunk backfill for '{model}': {embedded} chunks embedded, {chunked} memories chunked");
        }
        Ok((embedded, chunked))
    }

    /// How many consecutive width mismatches make a wrong binding rather than a
    /// race.
    ///
    /// A provider switch corrects itself on the very call that observes it —
    /// the router hands back `switched_to` and the store rebinds before the
    /// write lands — so a mismatch is normally a one-shot: writes already in
    /// flight when the switch happened, carrying the previous provider's width.
    /// Concurrency bounds how many of those there can be. Nothing bounds how
    /// long a genuinely wrong binding persists, which is the difference this
    /// number is drawing. Eight is comfortably above plausible in-flight
    /// concurrency and far below "all day".
    const WIDTH_MISMATCH_ESCALATION: usize = 8;

    /// Record that a write arrived with `observed` width against the current
    /// binding, and re-probe the provider when that stops looking like a race.
    ///
    /// The 2026-08-04 incident this exists for: the store was latched at 1536
    /// (a paid embedder returning 402) while the live provider produced 768.
    /// Every write for an entire day queued for backfill — 2167 of them — and
    /// the backfill could not drain them either, because activating a
    /// backfilled vector runs the same width gate that rejected them. Nothing
    /// escalated, because queueing a write is *correct* behaviour for the race
    /// it was written for, and a correct-looking line repeated 2167 times still
    /// reads as correct.
    ///
    /// The re-probe is authoritative rather than a guess: it asks the live
    /// provider for a vector and takes its width. Inferring the width from the
    /// rejected vector instead would be wrong in exactly the race case, where
    /// that vector is the OLD provider's and the binding is already right.
    async fn note_width_mismatch(&self, observed: usize) {
        let escalate = {
            let mut streak = self.width_mismatch_streak.write().await;
            if streak.1 == observed {
                streak.0 += 1;
            } else {
                *streak = (1, observed);
            }
            streak.0 == Self::WIDTH_MISMATCH_ESCALATION
        };
        if !escalate {
            return;
        }
        warn!(
            "{} consecutive writes queued at {observed} dims against a {} dim binding — that is \
             no longer a provider switch in flight, it is a stale binding. Re-probing the live \
             embedding provider.",
            Self::WIDTH_MISMATCH_ESCALATION,
            self.dimension()
        );
        match self.probe_and_align_dimension().await {
            Ok(actual) => {
                info!("Re-probe settled the binding at {actual} dims; queued writes can now backfill");
                *self.width_mismatch_streak.write().await = (0, 0);
            }
            // Leave the streak intact: the next write re-escalates, which is
            // the behaviour we want while the provider is still unreachable.
            Err(e) => warn!("Re-probe could not reach the embedding provider: {e}"),
        }
    }

    /// Reset the mismatch streak — a write whose width agreed with the binding
    /// is direct evidence the binding is fine.
    async fn note_width_agreement(&self) {
        let mut streak = self.width_mismatch_streak.write().await;
        if streak.0 != 0 {
            *streak = (0, 0);
        }
    }

    /// Identity of the embedding model currently in use, if the daemon has said.
    pub async fn active_embedding_model(&self) -> Option<String> {
        self.binding.read().await.model.clone()
    }

    /// Chunk geometry for the active embedding model.
    ///
    /// Read this immediately before splitting content, never cached across an
    /// await — a provider switch mid-write would otherwise chunk to the old
    /// model's window and hand the new embedder text it silently truncates.
    pub async fn chunk_params(&self) -> ChunkParams {
        let _binding = self.binding.read().await;
        self.store.chunk_params()
    }

    /// The current binding as one consistent `(model, dimension)` pair.
    ///
    /// The width latch only moves under the model write-lock, so reading it
    /// while holding the read half cannot observe a torn (new model, old
    /// width) state — which is precisely the split-brain this exists to
    /// prevent.
    pub async fn active_binding(&self) -> (Option<String>, usize) {
        let binding = self.binding.read().await;
        (binding.model.clone(), self.store.dimension())
    }

    /// Get the minimum similarity score threshold for recall
    pub fn get_min_score(&self) -> f32 {
        // Check for runtime override first
        if let Ok(guard) = self.min_score_override.try_read() {
            if let Some(score) = *guard {
                return score;
            }
        }
        self.config.min_score
    }

    /// Set the minimum similarity score threshold for recall (runtime override)
    pub fn set_min_score(&self, score: f32) {
        if let Ok(mut guard) = self.min_score_override.try_write() {
            *guard = Some(score.clamp(0.0, 1.0));
        }
    }

    /// Smart ingest - handles duplicates via prediction error gating.
    ///
    /// Returns (id, action) where action is Reinforce/Update/Create.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError` if embedding or storage fails.
    pub async fn smart_ingest(
        &self,
        content: &str,
        metadata: HashMap<String, String>,
    ) -> Result<(String, IngestAction), MemoryError> {
        let embed_fn = self.embed_fn.as_ref().ok_or(MemoryError::NoEmbeddingProvider)?;

        // Generate embedding
        let embedding = (embed_fn)(content)
            .await
            .map_err(|e| MemoryError::Io(std::io::Error::other(e)))?;

        // Check for similar existing memories
        let results = self.store.search(&embedding, 1).await;
        
        if let Some((existing, similarity)) = results.first() {
            let action = IngestAction::from_similarity(*similarity);
            
            match action {
                IngestAction::Reinforce => {
                    // Just strengthen existing memory
                    self.pending_updates.write().await.push((existing.id.clone(), Rating::Good));
                    info!("Reinforced: {} (sim: {:.3})", truncate(&existing.content, 30), similarity);
                    return Ok((existing.id.clone(), action));
                }
                IngestAction::Update => {
                    // Related-but-distinct: fold new information into the existing
                    // memory (dedup) rather than accreting a near-duplicate.
                    let folded = self
                        .fold_into_memory(embed_fn, &existing.id, &existing.content, content)
                        .await?;
                    // A related mention is a successful recall — reinforce FSRS.
                    self.pending_updates
                        .write()
                        .await
                        .push((existing.id.clone(), Rating::Good));
                    match folded {
                        FoldResult::Merged => {
                            info!(
                                "Merged into: {} (sim: {:.3})",
                                truncate(&existing.content, 30),
                                similarity
                            );
                            return Ok((existing.id.clone(), IngestAction::Update));
                        }
                        FoldResult::Subset => {
                            info!(
                                "Update no-op (subset): {} (sim: {:.3})",
                                truncate(&existing.content, 30),
                                similarity
                            );
                            return Ok((existing.id.clone(), IngestAction::Update));
                        }
                        // The neighbour changed under the fold; the content
                        // landed nowhere, so it must still become a row.
                        FoldResult::Contended => {
                            debug!(
                                "Fold contended (memory changed mid-merge); \
                                 storing separately (sim: {:.3})",
                                similarity
                            );
                            // fall through to create
                        }
                    }
                }
                IngestAction::Create => {
                    // Novel content, create new
                }
            }
        }

        // Create new memory
        let id = uuid::Uuid::new_v4().to_string();
        let (active_model, bound_dim) = self.active_binding().await;
        if embedding.len() == bound_dim {
            self.note_width_agreement().await;
        } else {
            self.note_width_mismatch(embedding.len()).await;
        }
        let (embedding_model, embedding, embeddings) =
            resolve_entry_vectors(active_model, bound_dim, embedding);
        let entry = MemoryEntry {
            id: id.clone(),
            content: content.to_string(),
            embedding_model,
            embeddings,
            embedding,
            metadata,
            timestamp: chrono_timestamp(),
            fsrs: FsrsState::new(),
            workspace_id: None, // smart_ingest creates global memories
        };

        self.store.add(entry).await?;
        info!("Remembered: {} (id: {})", truncate(content, 50), id);
        Ok((id, IngestAction::Create))
    }

    /// Fold `incoming` content into an existing memory (`existing_id`), re-embedding
    /// the merged text. Shared by every ingest path that lands in the `Update` band.
    ///
    /// Returns [`FoldResult::Merged`] when the merge was written,
    /// [`FoldResult::Subset`] when `incoming` added nothing new (the merge
    /// left the existing content byte-identical — a true subset, or a merge
    /// the byte bound declined), and [`FoldResult::Contended`] when the
    /// target's content changed between the caller's read and the write — the
    /// rewrite is compare-and-swapped against `existing_content`, because
    /// blindly writing a merge of a STALE snapshot would erase whatever the
    /// concurrent writer (a dream-cycle fold, another ingest) just stored.
    /// On `Contended` the caller must land `incoming` some other way (its own
    /// row); it provably did NOT land here. The caller is responsible for FSRS
    /// reinforcement — this only rewrites content + embedding.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError` if re-embedding or the store update fails.
    async fn fold_into_memory(
        &self,
        embed_fn: &EmbedFn,
        existing_id: &str,
        existing_content: &str,
        incoming: &str,
    ) -> Result<FoldResult, MemoryError> {
        debug_assert!(
            !existing_id.is_empty(),
            "existing memory id must not be empty"
        );
        let merged = merge_memory_content(existing_content, incoming);
        if merged == existing_content {
            return Ok(FoldResult::Subset);
        }
        let merged_embedding = (embed_fn)(&merged)
            .await
            .map_err(|e| MemoryError::Io(std::io::Error::other(e)))?;
        match self
            .rewrite_with_binding(existing_id, &merged, merged_embedding, Some(existing_content))
            .await?
        {
            Some(_) => Ok(FoldResult::Merged),
            None => Ok(FoldResult::Contended),
        }
    }

    /// Rewrite `id` to `content` with `embedding`, resolved against the
    /// CURRENT binding: a vector whose width no longer matches (the provider
    /// switched while it was being computed) is discarded and the entry queued
    /// for backfill instead of failing the rewrite — the merged content must
    /// land either way.
    ///
    /// `expected_content`, when set, makes the rewrite a compare-and-swap on
    /// the entry's live content (see
    /// [`VectorStore::update_content_and_embedding_if`]): every caller here
    /// computed `content` by merging FROM a snapshot, and applying that merge
    /// over an entry someone else rewrote in between would silently drop the
    /// concurrent write. `None` from this method means the swap was declined
    /// and nothing was touched; `Some(vector)` is the vector actually stored
    /// (empty when queued for backfill), so callers can keep in-memory copies
    /// in step.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError::NotFound` if no entry has the given `id`.
    async fn rewrite_with_binding(
        &self,
        id: &str,
        content: &str,
        embedding: Vec<f32>,
        expected_content: Option<&str>,
    ) -> Result<Option<Vec<f32>>, MemoryError> {
        let (model, bound_dim) = self.active_binding().await;
        if embedding.len() == bound_dim {
            let applied = self
                .store
                .update_content_and_embedding_if(
                    id,
                    expected_content,
                    content,
                    embedding.clone(),
                    model.as_deref(),
                )
                .await?;
            return Ok(applied.then_some(embedding));
        }
        info!(
            "Rewrite of {id} raced a provider switch ({} dims vs bound {bound_dim}) — \
             content updated, vector queued for backfill",
            embedding.len()
        );
        let applied = self
            .store
            .update_content_and_embedding_if(id, expected_content, content, Vec::new(), None)
            .await?;
        Ok(applied.then_some(Vec::new()))
    }

    /// Remember something - store with embedding.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError` if no embedding function is configured or storage fails.
    pub async fn remember(
        &self,
        content: &str,
        metadata: HashMap<String, String>,
    ) -> Result<String, MemoryError> {
        let (id, _action) = self.smart_ingest(content, metadata).await?;
        Ok(id)
    }

    /// Remember something with explicit importance rating.
    ///
    /// Importance affects FSRS weight calculation and consolidation priority.
    /// Scale: 1.0 (minor) to 5.0 (critical identity info)
    ///
    /// # Errors
    ///
    /// Returns `MemoryError` if no embedding function is configured or storage fails.
    pub async fn remember_with_importance(
        &self,
        content: &str,
        metadata: HashMap<String, String>,
        importance: f32,
    ) -> Result<(String, IngestAction), MemoryError> {
        let embed_fn = self.embed_fn.as_ref().ok_or(MemoryError::NoEmbeddingProvider)?;

        // Truncate oversized content to avoid embedding model context overflow
        // Most embedding models have 8192 token context; ~4 chars/token ≈ 30k chars safe limit
        let content = if content.len() > 30_000 {
            warn!("Truncating oversized memory content ({} chars) to 30000", content.len());
            // `&content[..30_000]` panics unless 30_000 lands on a char boundary,
            // and `len()` is BYTES while the message says "chars". Any memory
            // whose 30_000th byte falls inside a multi-byte sequence took the
            // process down — one box-drawing character in `tree` output, one
            // emoji in a log line, one accented word. Walk back to a boundary.
            let mut end = 30_000;
            while end > 0 && !content.is_char_boundary(end) {
                end -= 1;
            }
            &content[..end]
        } else {
            content
        };

        // Generate embedding
        let embedding = (embed_fn)(content)
            .await
            .map_err(|e| MemoryError::Io(std::io::Error::other(e)))?;

        // Check for similar existing memories (duplicate detection)
        let results = self.store.search(&embedding, 1).await;
        
        if let Some((existing, similarity)) = results.first() {
            let action = IngestAction::from_similarity(*similarity);
            
            // Don't reinforce error memories — let them die
            let skip_reinforce = existing.content.contains("Error:")
                || existing.content.contains("Command failed");

            match action {
                IngestAction::Reinforce if !skip_reinforce => {
                    // Just strengthen existing memory (testing effect)
                    self.pending_updates.write().await.push((existing.id.clone(), Rating::Good));
                    // Also boost importance if new fact has higher importance
                    let importance_normalized = (importance / 5.0).clamp(0.5, 1.5);
                    if let Err(e) = self.store.update_fsrs(&existing.id, |fsrs| {
                        if importance_normalized > fsrs.importance {
                            fsrs.importance = importance_normalized;
                        }
                    }).await {
                        debug!("Failed to update importance: {}", e);
                    }
                    info!("Reinforced: {} (sim: {:.3})", truncate(&existing.content, 30), similarity);
                    return Ok((existing.id.clone(), action));
                }
                IngestAction::Update if !skip_reinforce => {
                    // Related-but-distinct: fold new information in (dedup) and reinforce.
                    let folded = self
                        .fold_into_memory(embed_fn, &existing.id, &existing.content, content)
                        .await?;
                    self.pending_updates.write().await.push((existing.id.clone(), Rating::Good));
                    // Same invariant as `remember_scoped`: an unabsorbed fold
                    // means the content landed nowhere, so it must still become
                    // a row. Only a provable superset may discard.
                    //
                    // This is the path taken whenever no workspace is active, so
                    // leaving it on the old behaviour would have made whether a
                    // memory survives depend on whether a workspace happened to
                    // be selected — the same bug, hiding one branch over.
                    match folded {
                        FoldResult::Merged => {
                            info!(
                                "Merged (importance): {} (sim: {:.3})",
                                truncate(&existing.content, 30),
                                similarity
                            );
                            return Ok((existing.id.clone(), action));
                        }
                        FoldResult::Subset => {
                            if existing.content.trim().contains(content.trim()) {
                                info!(
                                    "Related memory exists (already present): {} (sim: {:.3})",
                                    truncate(&existing.content, 30),
                                    similarity
                                );
                                return Ok((existing.id.clone(), action));
                            }
                            debug!(
                                "Fold declined (bound); storing separately (sim: {:.3})",
                                similarity
                            );
                            // fall through to create
                        }
                        // The neighbour changed under the fold — the CAS
                        // declined and the content landed nowhere.
                        FoldResult::Contended => {
                            debug!(
                                "Fold contended (memory changed mid-merge); \
                                 storing separately (sim: {:.3})",
                                similarity
                            );
                            // fall through to create
                        }
                    }
                }
                _ => {
                    // Novel content or skipped reinforcement — fall through to create
                    if skip_reinforce {
                        info!("Skipping reinforcement of error memory: {} (sim: {:.3})", truncate(&existing.content, 30), similarity);
                    }
                }
            }
        }

        // Create new memory with importance
        let id = uuid::Uuid::new_v4().to_string();
        let mut fsrs = FsrsState::new();
        // Normalize importance from 1-5 scale to 0.5-1.5 multiplier
        fsrs.importance = (importance / 5.0).clamp(0.5, 1.5);
        
        let (active_model, bound_dim) = self.active_binding().await;
        if embedding.len() == bound_dim {
            self.note_width_agreement().await;
        } else {
            self.note_width_mismatch(embedding.len()).await;
        }
        let (embedding_model, embedding, embeddings) =
            resolve_entry_vectors(active_model, bound_dim, embedding);
        let entry = MemoryEntry {
            id: id.clone(),
            content: content.to_string(),
            embedding_model,
            embeddings,
            embedding,
            metadata: metadata.clone(),
            timestamp: chrono_timestamp(),
            fsrs,
            workspace_id: None, // Global memory
        };

        self.store.add(entry).await?;
        info!("Remembered (importance {}): {} (id: {})", importance, truncate(content, 50), id);
        Ok((id, IngestAction::Create))
    }

    /// Remember something with workspace scope.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError` if no embedding function is configured or storage fails.
    pub async fn remember_scoped(
        &self,
        content: &str,
        metadata: HashMap<String, String>,
        importance: f32,
        workspace_id: Option<String>,
    ) -> Result<(String, IngestAction), MemoryError> {
        let embed_fn = self.embed_fn.as_ref().ok_or(MemoryError::NoEmbeddingProvider)?;

        // Generate embedding
        let embedding = (embed_fn)(content)
            .await
            .map_err(|e| MemoryError::Io(std::io::Error::other(e)))?;

        // Check for similar existing memories (within same scope)
        let results = self.store.search_scoped(&embedding, 1, workspace_id.as_deref()).await;
        
        if let Some((existing, similarity)) = results.first() {
            let action = IngestAction::from_similarity(*similarity);
            
            // A near neighbour is a reason to STRENGTHEN it, never on its own a
            // reason to throw the new observation away.
            //
            // The old code returned here for both the Reinforce and Update
            // bands, so the incoming content survived only if `fold_into_memory`
            // happened to absorb it. Measured over one 2-hour run
            // (2026-07-27): 1307 ingests reached these bands and 1028 of them
            // were folds that returned `false` — content written nowhere, and
            // logged as "Reinforced", indistinguishable from a real duplicate.
            // That single early return is most of why 2314 memory-target tool
            // calls produced 54 rows.
            //
            // The invariant now: content is dropped ONLY when it is provably
            // already in the store. Everything else falls through and creates a
            // row. Squeezing the store is dreaming's job — it can see the whole
            // corpus and narrate across it; the write path sees one neighbour
            // and cannot tell redundancy from recurrence.
            // An ADDRESSABLE record always gets its own row.
            //
            // Episodic writes carry a `source_id`, and the agent has already
            // been handed `[memory:{source_id}]` in its context as a promise it
            // can dereference later. Folding such a write into a neighbour keeps
            // the text but destroys the address: the merged row's metadata still
            // names the OLD source_id, so `recall("{new}")` resolves to nothing
            // and the model is told its own handle is dead — for the majority of
            // tool calls, since repeated commands are exactly what lands in the
            // fold bands.
            //
            // So the fold bands are skipped whenever a handle was issued. This
            // also happens to be the honest reading of "memory holds
            // everything": deduplication is dreaming's job, done later with the
            // whole corpus in view, not the write path's guess from one
            // neighbour.
            let addressable = metadata.contains_key("source_id");

            if addressable && matches!(action, IngestAction::Reinforce | IngestAction::Update) {
                // Still a genuine hit: strengthen the neighbour. Just do not
                // absorb the new record into it and orphan its handle.
                self.pending_updates.write().await.push((existing.id.clone(), Rating::Good));
            }

            if !addressable && matches!(action, IngestAction::Reinforce | IngestAction::Update) {
                // The neighbour was a genuine hit either way: rate it.
                self.pending_updates.write().await.push((existing.id.clone(), Rating::Good));

                // Provably redundant: the existing entry already contains this
                // text verbatim. Only this case may discard.
                if existing.content.trim().contains(content.trim()) {
                    info!(
                        "Reinforced (scoped, already present): {} (sim: {:.3})",
                        truncate(&existing.content, 30),
                        similarity
                    );
                    return Ok((existing.id.clone(), action));
                }

                // Otherwise try to fold it in. Only `Merged` means the content
                // actually landed somewhere; `Subset` is the byte bound
                // declining the append and `Contended` is the CAS declining a
                // stale merge — both fall through to a row of its own.
                match self
                    .fold_into_memory(embed_fn, &existing.id, &existing.content, content)
                    .await?
                {
                    FoldResult::Merged => {
                        info!(
                            "Merged (scoped): {} (sim: {:.3})",
                            truncate(&existing.content, 30),
                            similarity
                        );
                        return Ok((existing.id.clone(), action));
                    }
                    FoldResult::Subset => {
                        // Not absorbed — the merge would have breached the byte
                        // bound. Keep the observation as its own entry rather
                        // than losing it.
                        debug!(
                            "Fold declined (bound); storing separately (sim: {:.3})",
                            similarity
                        );
                    }
                    FoldResult::Contended => {
                        debug!(
                            "Fold contended (memory changed mid-merge); \
                             storing separately (sim: {:.3})",
                            similarity
                        );
                    }
                }
            }
        }

        // Create new memory with workspace scope.
        //
        // The row id CARRIES the handle when one was issued. `resolve_memory_handle`
        // already falls back to `id.starts_with(handle)`, but with a bare fresh
        // uuid that fallback could only ever fire by a 1-in-4-billion accident —
        // and if it did fire it would return an unrelated memory, which is worse
        // than failing. Prefixing makes it a guarantee.
        let id = match metadata.get("source_id") {
            Some(source_id) => format!("{source_id}-{}", uuid::Uuid::new_v4()),
            None => uuid::Uuid::new_v4().to_string(),
        };
        let mut fsrs = FsrsState::new();
        fsrs.importance = (importance / 5.0).clamp(0.5, 1.5);

        let (active_model, bound_dim) = self.active_binding().await;
        if embedding.len() == bound_dim {
            self.note_width_agreement().await;
        } else {
            self.note_width_mismatch(embedding.len()).await;
        }
        let (embedding_model, embedding, embeddings) =
            resolve_entry_vectors(active_model, bound_dim, embedding);
        let entry = MemoryEntry {
            id: id.clone(),
            content: content.to_string(),
            embedding_model,
            embeddings,
            embedding,
            metadata,
            timestamp: chrono_timestamp(),
            fsrs,
            workspace_id,
        };

        self.store.add(entry).await?;
        info!("Remembered (scoped, importance {}): {} (id: {})", importance, truncate(content, 50), id);
        Ok((id, IngestAction::Create))
    }

    /// Recall memories similar to a query.
    ///
    /// Applies the testing effect: recalled memories get strengthened.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError` if no embedding function is configured.
    pub async fn recall(&self, query: &str) -> Result<Vec<RecallResult>, MemoryError> {
        // Delegates rather than duplicating. This used to be a second,
        // near-identical search loop, and it drifted the moment chunk-level
        // recall was added: the scoped path gained it, this one silently did
        // not, so whether a memory could be found at all depended on which
        // entry point the caller happened to use. `search_scoped(.., None)` is
        // the unscoped search exactly — it takes the same `top_k` off the same
        // ranking with no workspace filter — so there is nothing here for a
        // separate implementation to be more correct about.
        self.recall_scoped(query, None).await
    }

    /// Recall memories with workspace scope filtering.
    ///
    /// Scope rules:
    /// - `workspace_id = Some(id)`: returns global + that workspace's memories
    /// - `workspace_id = None` (global): returns all memories
    ///
    /// Searches row vectors and chunk vectors together, keeping the stronger
    /// evidence per memory — see [`crate::chunk_rank`] for why the two are
    /// merged rather than one replacing the other.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError` if no embedding function is configured.
    pub async fn recall_scoped(
        &self,
        query: &str,
        workspace_id: Option<&str>,
    ) -> Result<Vec<RecallResult>, MemoryError> {
        let embed_fn = self.embed_fn.as_ref().ok_or(MemoryError::NoEmbeddingProvider)?;

        let store_count = self.store.len().await;
        info!("Recall (scoped {:?}): generating embedding for query (store has {} entries)", 
              workspace_id, store_count);

        let query_embedding = (embed_fn)(query)
            .await
            .map_err(|e| MemoryError::Io(std::io::Error::other(e)))?;
        
        info!("Recall: embedding generated ({} dims), searching...", query_embedding.len());

        // Scoped search
        let results = self.store.search_scoped(&query_embedding, self.config.max_results * 2, workspace_id).await;
        let min_score = self.get_min_score();

        // Chunk-level hits, collapsed to one per parent. A long memory whose
        // whole-row vector is a centroid of everything it contains can score
        // poorly on a query that one of its paragraphs answers exactly — that
        // is the failure chunking exists to fix, so the two signals are merged
        // rather than one replacing the other, and each memory keeps whichever
        // evidence is stronger.
        // The model that produced `query_embedding` is the only space its
        // chunk neighbours can legitimately be drawn from. Read once, here, and
        // pass it down — re-reading it inside the search could straddle a
        // provider switch and compare the query against another model's chunks.
        let active_model = self.active_embedding_model().await.unwrap_or_default();
        let chunk_hits = self
            .store
            .search_chunks(
                &query_embedding,
                &active_model,
                self.config.max_results * 2,
                workspace_id,
                min_score,
            )
            .await;

        // Filter by min score and weight, apply testing effect
        let mut filtered = Vec::new();
        let mut updates = Vec::new();
        let mut scored: Vec<(MemoryEntry, f32, f32, Option<i64>)> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

        for (entry, score) in results {
            let hit = chunk_hits.get(&entry.id);
            // Admitted on the better of the two TRUE similarities. Neither is
            // inflated for the test: `rank` carries the corroboration bonus and
            // is used only for ordering, so the threshold keeps the exact
            // meaning it was calibrated with.
            let best = hit.map_or(score, |h| h.best.max(score));
            if best < min_score {
                continue;
            }
            let rank = hit.map_or(score, |h| h.rank().max(score));
            // Only name a chunk when the chunk is what actually won; a row-level
            // hit has no "part that matched" to point at.
            let best_chunk = hit.filter(|h| h.best >= score).map(|h| h.best_ordinal);
            seen.insert(entry.id.clone());
            scored.push((entry, best, rank, best_chunk));
        }

        // Memories whose chunks matched but whose whole-row vector did not
        // clear the top-k cut. These are the ones chunking is FOR: without
        // this pass, splitting a memory would improve nothing, because the
        // row-level search would still be the only way in.
        for (id, hit) in &chunk_hits {
            if seen.contains(id) {
                continue;
            }
            if let Some(entry) = self.store.get(id).await {
                if workspace_id.is_some() && entry.workspace_id.as_deref() != workspace_id {
                    continue;
                }
                scored.push((entry, hit.best, hit.rank(), Some(hit.best_ordinal)));
            }
        }

        scored.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

        for (entry, score, _rank, best_chunk) in scored {
            let weight = entry.fsrs.weight(&self.config.fsrs);
            let state = entry.fsrs.state(&self.config.fsrs);

            if weight < self.config.min_weight {
                continue;
            }

            updates.push((entry.id.clone(), Rating::Good));

            filtered.push(RecallResult {
                id: entry.id,
                content: entry.content,
                best_chunk,
                score,
                weight,
                state,
                metadata: entry.metadata,
                workspace_id: entry.workspace_id,
            });

            if filtered.len() >= self.config.max_results {
                break;
            }
        }

        if !updates.is_empty() {
            self.pending_updates.write().await.extend(updates);
        }

        debug!("Recall (scoped) '{}' found {} results", truncate(query, 30), filtered.len());
        Ok(filtered)
    }

    /// How many FSRS testing-effect reviews are queued and not yet applied.
    ///
    /// Every `recall`/`recall_scoped` hit enqueues one review, and
    /// [`Self::apply_pending_updates`] is the only drain — which in the running
    /// daemon happens exclusively inside a dream cycle. A number that only ever
    /// climbs therefore means nothing is dreaming, and the queue is growing
    /// unbounded. Exposed so that condition is observable (and assertable in
    /// tests) rather than silent.
    pub async fn pending_update_count(&self) -> usize {
        self.pending_updates.read().await.len()
    }

    /// Apply pending FSRS updates (testing effect).
    ///
    /// Call this periodically to batch-apply memory strengthening.
    pub async fn apply_pending_updates(&self) {
        let updates: Vec<_> = self.pending_updates.write().await.drain(..).collect();
        
        if updates.is_empty() {
            return;
        }

        let count = updates.len();
        for (id, rating) in updates {
            if let Err(e) = self.store.update_fsrs(&id, |fsrs| {
                fsrs.record_access(&self.config.fsrs, rating);
            }).await {
                debug!("Failed to update FSRS for {}: {}", id, e);
            }
        }
        
        debug!("Applied {} FSRS updates", count);
    }

    /// Promote a memory (mark as helpful/important)
    ///
    /// # Errors
    ///
    /// Returns `MemoryError` if the memory is not found.
    pub async fn promote(&self, id: &str, boost: f32) -> Result<(), MemoryError> {
        self.store.update_fsrs(id, |fsrs| {
            fsrs.promote(boost);
        }).await?;
        info!("Promoted memory: {} (boost: {})", id, boost);
        Ok(())
    }

    /// Demote a memory (mark as wrong/unhelpful)
    ///
    /// # Errors
    ///
    /// Returns `MemoryError` if the memory is not found.
    pub async fn demote(&self, id: &str, penalty: f32) -> Result<(), MemoryError> {
        self.store.update_fsrs(id, |fsrs| {
            fsrs.demote(penalty);
        }).await?;
        info!("Demoted memory: {} (penalty: {})", id, penalty);
        Ok(())
    }

    /// Get memories grouped by weight bands for consolidation.
    ///
    /// Returns memories in bands: essence (<0.2), compressed (0.2-0.5), 
    /// standard (0.5-0.8), detailed (0.8-1.0), expand (>1.0)
    pub async fn get_consolidation_bands(&self) -> ConsolidationBands {
        let entries = self.store.all_entries().await;
        let params = &self.config.fsrs;
        
        let mut bands = ConsolidationBands::default();
        
        for entry in entries {
            let weight = entry.fsrs.weight(params);
            
            if weight < 0.2 {
                bands.essence.push(entry);
            } else if weight < 0.5 {
                bands.compressed.push(entry);
            } else if weight < 0.8 {
                bands.standard.push(entry);
            } else if weight <= 1.0 {
                bands.detailed.push(entry);
            } else {
                bands.expand.push(entry);
            }
        }
        
        bands
    }

    /// Forget a memory by ID.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError` if the memory cannot be removed.
    pub async fn forget(&self, id: &str) -> Result<(), MemoryError> {
        self.store.remove(id).await?;
        info!("Forgot memory: {}", id);
        Ok(())
    }

    /// Get memory count
    pub async fn count(&self) -> usize {
        self.store.len().await
    }

    /// Add a fully-formed [`MemoryEntry`] straight to the store, bypassing
    /// embedding generation and FSRS initialization.
    ///
    /// This is for callers that already hold a complete entry with a controlled
    /// embedding and FSRS state — notably the retention harness
    /// ([`crate::retention`]), which needs deterministic vectors and aged
    /// timestamps that the `remember` path (fresh FSRS, model-generated
    /// embeddings) can't express. The store still normalizes the embedding.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError::DimensionMismatch`] if the embedding's length does
    /// not match the store dimension.
    pub async fn add_entry(&self, entry: MemoryEntry) -> Result<(), MemoryError> {
        debug_assert!(!entry.embedding.is_empty(), "add_entry with empty embedding");
        self.store.add(entry).await
    }

    /// Raw top-`k` vector search by a pre-computed query embedding.
    ///
    /// Unlike [`Self::recall`], this bypasses the string→embedding step and the
    /// `min_score`/`min_weight` recall gating, returning the store's raw
    /// cosine-nearest entries. It exists so the retention harness
    /// ([`crate::retention`]) can measure vector recall directly — isolating a
    /// dream cycle's effect on retrievability from the recall gating and any
    /// embedding-model noise.
    ///
    /// Returns `(entry, score)` pairs, at most `top_k` of them, score-descending.
    pub async fn search_by_embedding(
        &self,
        query_embedding: &[f32],
        top_k: usize,
    ) -> Vec<(MemoryEntry, f32)> {
        debug_assert!(top_k >= 1, "search_by_embedding needs a positive top_k");
        let results = self.store.search(query_embedding, top_k).await;
        debug_assert!(results.len() <= top_k, "store returned more than top_k results");
        results
    }

    /// Get memory statistics
    pub async fn stats(&self) -> MemoryStats {
        let entries = self.store.all_entries().await;
        let params = &self.config.fsrs;
        
        let mut stats = MemoryStats::default();
        stats.total = entries.len();
        
        for entry in entries {
            match entry.fsrs.state(params) {
                MemoryState::Active => stats.active += 1,
                MemoryState::Dormant => stats.dormant += 1,
                MemoryState::Silent => stats.silent += 1,
                MemoryState::Unavailable => stats.unavailable += 1,
            }
        }
        
        stats
    }

    /// Get all memories with their FSRS state
    pub async fn list_all(&self) -> Vec<MemoryListEntry> {
        let entries = self.store.all_entries().await;
        let params = &self.config.fsrs;
        
        entries.into_iter().map(|e| {
            let weight = e.fsrs.weight(params);
            let state = e.fsrs.state(params);
            let retrievability = e.fsrs.retrievability(params);
            
            MemoryListEntry {
                id: e.id,
                content: e.content,
                metadata: e.metadata,
                timestamp: e.timestamp,
                state,
                weight,
                retrievability,
                importance: e.fsrs.importance,
                access_count: e.fsrs.access_count,
                workspace_id: e.workspace_id,
            }
        }).collect()
    }

    /// Get a single memory by ID
    pub async fn get(&self, id: &str) -> Option<MemoryListEntry> {
        let entry = self.store.get(id).await?;
        let params = &self.config.fsrs;
        
        Some(MemoryListEntry {
            id: entry.id,
            content: entry.content,
            metadata: entry.metadata,
            timestamp: entry.timestamp,
            state: entry.fsrs.state(params),
            weight: entry.fsrs.weight(params),
            retrievability: entry.fsrs.retrievability(params),
            importance: entry.fsrs.importance,
            access_count: entry.fsrs.access_count,
            workspace_id: entry.workspace_id,
        })
    }

    /// Update a memory's content
    pub async fn update_content(&self, id: &str, content: &str) -> Result<(), MemoryError> {
        self.store.update_content(id, content).await?;
        info!("Updated memory content: {}", id);
        Ok(())
    }

    /// Clear all memories
    pub async fn clear(&self) {
        self.store.clear().await;
        info!("Cleared all memories");
    }

    /// Save memories to file.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError` if the file cannot be written.
    pub async fn save(&self, path: &std::path::Path) -> Result<(), MemoryError> {
        self.store.save(path).await
    }

    /// Load memories from file.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError` if the file cannot be read or parsed.
    pub async fn load(&self, path: &std::path::Path) -> Result<(), MemoryError> {
        self.store.load(path).await
    }

    /// Flush all in-memory entries to the persistence backend (for JSON → Turso migration).
    pub async fn flush_to_db(&self) -> Result<usize, MemoryError> {
        self.store.flush_to_db().await
    }

    /// Run memory consolidation ("dreaming").
    ///
    /// This is the core of the cognitive memory model:
    /// 1. Groups memories by weight bands
    /// 2. Clusters memories using a composite score (similarity + recall + importance + age)
    /// 3. Uses LLM to summarize/compress based on weight
    /// 4. Replaces clusters with consolidated memories
    /// 5. Respects compression limits (max ratio and min remaining count)
    ///
    /// # Arguments
    /// * `config` - Consolidation parameters
    /// * `summarize_fn` - Async function that takes a prompt and returns summarized text
    ///
    /// # Errors
    ///
    /// Returns `MemoryError` if storage operations fail.
    pub async fn consolidate<F, Fut>(
        &self,
        config: &ConsolidationConfig,
        summarize_fn: F,
    ) -> Result<ConsolidationResult, MemoryError>
    where
        F: Fn(String) -> Fut + Send + Sync,
        Fut: Future<Output = Result<String, String>> + Send,
    {
        let mut result = ConsolidationResult::default();
        let total_memories = self.count().await;

        // Compute budget: how many memories we're allowed to remove
        let max_removable = ((total_memories as f32) * config.max_compression_ratio) as usize;
        let floor_headroom = total_memories.saturating_sub(config.min_remaining_memories);
        let mut removal_budget = max_removable.min(floor_headroom);

        info!(
            "Consolidation starting: {} memories, removal budget {} (max_ratio={}, floor={})",
            total_memories, removal_budget, config.max_compression_ratio, config.min_remaining_memories
        );

        if removal_budget == 0 {
            info!("Consolidation: nothing to do (at or below floor)");
            return Ok(result);
        }

        // Measure how long this store has actually been accumulating, and judge
        // "close in time" against that rather than against a constant.
        //
        // Derived from the memories themselves rather than plumbed in from the
        // workspace record: the observed span IS the store's lifetime for
        // clustering purposes, it needs no new dependency, and it stays correct
        // for a workspace that sat idle for a month or was seeded from an import.
        let config = {
            let mut config = config.clone();
            let span_minutes = {
                let all = self.store.all_entries().await;
                match (
                    all.iter().map(|e| e.timestamp).min(),
                    all.iter().map(|e| e.timestamp).max(),
                ) {
                    (Some(first), Some(last)) => (last - first) as f32 / 60.0,
                    _ => 0.0,
                }
            };
            config.clustering_weights.time_span_minutes = span_minutes.max(1.0);
            info!(
                "Consolidation timescale: store spans {:.1} hours",
                span_minutes / 60.0
            );
            config
        };
        let config = &config;

        // Get all memories grouped by weight
        let bands = self.get_consolidation_bands().await;

        // Process each band
        let band_entries = [
            (CompressionLevel::Essence, bands.essence),
            (CompressionLevel::Compressed, bands.compressed),
            (CompressionLevel::Standard, bands.standard),
            (CompressionLevel::Detailed, bands.detailed),
            (CompressionLevel::Expand, bands.expand),
        ];

        for (compression_level, memories) in band_entries {
            if memories.is_empty() || removal_budget == 0 {
                continue;
            }

            // Phase (b): fold true duplicates deterministically FIRST, so the
            // summarizer is only ever paid for genuinely distinct content.
            //
            // This runs in EVERY band, including `Detailed`. Detailed is skipped
            // for *compression* below because those are the highest-weight
            // memories and paraphrasing your best-established facts is exactly
            // what you do not want — but deduplication is **lossless** (guarded:
            // a fold is committed only when no content is dropped), so there is
            // no reason to let exact restatements of your most important facts
            // accumulate forever. It also removes the most valuable memories
            // from the drift-exposed path entirely.
            let deduped_before = result.memories_deduped;
            let memories = self
                .fold_near_duplicates(memories, removal_budget, &mut result)
                .await;
            let folded_here = result.memories_deduped - deduped_before;
            removal_budget = removal_budget.saturating_sub(folded_here);
            result.memories_processed += folded_here;
            if removal_budget == 0 {
                continue;
            }

            // `Detailed` used to end here for EVERYTHING, and that is why a long
            // run's memories were never narrated while the run was happening:
            // freshness holds them at the top of the weight bands for hours, so
            // dreaming only ever reached them once they had decayed — by which
            // time the day they belonged to was over. The owner's model is the
            // opposite. Dreaming is what finds the throughline through a day,
            // while the day is still in progress.
            //
            // The drift argument still stands, but only for the memories it was
            // ever about. Paraphrasing a durable fact ("the port is 5149")
            // degrades it, and that is worth protecting at high weight. An
            // episodic record — a tool result, a thought — is high-volume and
            // individually cheap, and is exactly the material meant to be
            // compressed into narrative. So at `Detailed`, episodes consolidate
            // and durable facts are left alone.
            let memories = if compression_level == CompressionLevel::Detailed {
                let (episodic, durable): (Vec<_>, Vec<_>) =
                    memories.into_iter().partition(is_episodic_memory);
                result.memories_processed += durable.len();
                if episodic.is_empty() {
                    continue;
                }
                episodic
            } else {
                memories
            };

            // Cluster using composite score (similarity + recall + importance + age)
            let clusters = cluster_memories(memories, config);

            for cluster_memories in clusters {
                if removal_budget == 0 {
                    break;
                }

                if cluster_memories.len() < config.min_cluster_size {
                    // Singleton or small cluster - process individually if Expand
                    if compression_level == CompressionLevel::Expand {
                        for memory in &cluster_memories {
                            if let Err(e) = self.expand_memory(memory, &summarize_fn).await {
                                result.errors.push(format!("Expand failed for {}: {}", memory.id, e));
                            } else {
                                result.memories_expanded += 1;
                            }
                        }
                    }
                    result.memories_processed += cluster_memories.len();
                    continue;
                }

                // How many would this cluster merge away?
                let would_remove = cluster_memories.len() - 1; // -1 because we create 1 new
                if would_remove > removal_budget {
                    // Skip this cluster — it would exceed our budget
                    result.memories_processed += cluster_memories.len();
                    continue;
                }

                // Create cluster and consolidate
                let cluster = MemoryCluster::new(
                    cluster_memories.clone(),
                    compression_level,
                    &self.config.fsrs,
                );

                match self.consolidate_cluster(&cluster, &summarize_fn).await {
                    Ok(()) => {
                        result.clusters_formed += 1;
                        result.memories_merged += would_remove;
                        result.memories_processed += cluster_memories.len();
                        removal_budget = removal_budget.saturating_sub(would_remove);
                    }
                    Err(e) => {
                        result.errors.push(format!("Cluster consolidation failed: {}", e));
                        result.memories_processed += cluster_memories.len();
                    }
                }
            }
        }

        // `deduped` belongs in the summary: it is the only line that REMOVES
        // rows, and leaving it out made a cycle that deleted 13 memories read
        // as "0 merged" — compression looked like a no-op while the store
        // visibly shrank underneath it.
        info!(
            "Consolidation complete: {} processed, {} clusters, {} merged, {} deduped, \
             {} expanded, {} errors",
            result.memories_processed,
            result.clusters_formed,
            result.memories_merged,
            result.memories_deduped,
            result.memories_expanded,
            result.errors.len()
        );

        // A COUNT of errors is not a report of them. One observed cycle logged
        // "89 processed, 0 clusters, 0 merged, 0 expanded, 5 errors" — a total
        // no-op with five unexplained failures, and the strings describing them
        // were built, stored on the result, and then dropped on the floor. Say
        // what actually broke.
        for error in &result.errors {
            warn!("Consolidation error: {error}");
        }

        Ok(result)
    }

    /// Dream phase (b): fold near-duplicate memories together **deterministically,
    /// with no LLM call**, returning the survivors for the summarizing clusterer.
    ///
    /// A dream cycle otherwise pays one summarizer call for every cluster —
    /// including clusters that are nothing but restatements of the same fact
    /// ("user prefers dark mode", stored three times from three sessions).
    /// Paraphrasing those through a model is both wasted tokens (the scarcest
    /// resource on a single-GPU local tier) and *lossier* than a deterministic
    /// fold. This pass removes them from the clusterer's input first.
    ///
    /// Rules, in order of importance:
    /// - **Scope is absolute**: never fold across a workspace boundary
    ///   ([`same_scope`]) — that would leak or re-home a memory.
    /// - **Only truly-duplicate pairs**: the pair must reach
    ///   [`IngestAction::Reinforce`] (cosine > 0.92), the project's existing
    ///   "this is the same fact" line, reused rather than inventing a threshold.
    /// - **Never lose content**: `merge_memory_content` keeps `existing`
    ///   unchanged when the append would breach its byte bound, so folding then
    ///   removing the source would silently drop text. A fold is only committed
    ///   when the merged content demonstrably still contains the incoming
    ///   content; otherwise both memories are left for the clusterer.
    /// - **Strongest survives**: the band is processed in descending FSRS
    ///   weight, so duplicates always fold *into* the best-established memory,
    ///   which then inherits `max` importance and the summed access count
    ///   (mirroring `create_consolidated_entry`).
    /// - **Update before remove**: the survivor is rewritten first; only then is
    ///   the source dropped. A failure part-way leaves a transient duplicate
    ///   that the next cycle re-folds, never a hole.
    ///
    /// Bounded by `removal_budget_count` (the cycle's compression allowance) and
    /// by the band size; the pairwise scan is the same O(N²) shape the existing
    /// clusterer already has, so this adds no new complexity class.
    async fn fold_near_duplicates(
        &self,
        memories: Vec<MemoryEntry>,
        removal_budget_count: usize,
        result: &mut ConsolidationResult,
    ) -> Vec<MemoryEntry> {
        let memories_count_before = memories.len();
        if removal_budget_count == 0 || memories_count_before < 2 {
            return memories;
        }

        let Some(embed_fn) = self.embed_fn.as_ref() else {
            // No embedder: folding would be unable to re-embed the survivor and
            // would leave content and vector inconsistent. Skip, don't guess.
            return memories;
        };

        // Strongest first, so a duplicate always folds into the best-established
        // memory rather than whichever happened to come back first.
        let mut ranked = memories;
        ranked.sort_by(|a, b| {
            let weight_b = b.fsrs.weight(&self.config.fsrs);
            let weight_a = a.fsrs.weight(&self.config.fsrs);
            weight_b
                .partial_cmp(&weight_a)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut survivors: Vec<MemoryEntry> = Vec::with_capacity(ranked.len());
        let mut folded_count = 0_usize;
        // `(id, content-at-fold-time)` of successful folds, removed in one
        // MATCHED batch after the pass: a source that a live write rewrote
        // after its fold committed no longer holds only the text the survivor
        // absorbed, so the matched removal keeps it (transient duplicate,
        // re-folded next cycle) instead of deleting the fresh write.
        let mut folded_sources: Vec<(String, String)> = Vec::new();

        for candidate in ranked {
            let target = if folded_count < removal_budget_count {
                find_duplicate_target(&survivors, &candidate)
            } else {
                None
            };

            let Some(target_index) = target else {
                survivors.push(candidate);
                continue;
            };

            let survivor = &survivors[target_index];
            let incoming = candidate.content.trim();
            let merged = merge_memory_content(&survivor.content, incoming);

            // The losslessness guard: a merge that dropped the incoming text
            // must not be followed by removing its source.
            if !merged.contains(incoming) {
                survivors.push(candidate);
                continue;
            }

            match self
                .commit_duplicate_fold(embed_fn, &survivors[target_index], &candidate, &merged)
                .await
            {
                Ok(DedupCommit::Committed(new_embedding)) => {
                    // Keep the in-memory copy in step with the store — including
                    // the vector. A survivor left holding its pre-merge embedding
                    // would be compared (and later clustered) on a vector that no
                    // longer describes its content.
                    survivors[target_index].content = merged;
                    if let Some(embedding) = new_embedding {
                        survivors[target_index].embedding = embedding;
                    }
                    survivors[target_index].fsrs.importance = survivors[target_index]
                        .fsrs
                        .importance
                        .max(candidate.fsrs.importance);
                    survivors[target_index].fsrs.access_count += candidate.fsrs.access_count;
                    folded_sources.push((candidate.id.clone(), candidate.content.clone()));
                    folded_count += 1;
                    result.memories_deduped += 1;
                }
                // A live write moved the pair under the fold. Nothing was
                // written, so BOTH memories stay; the next cycle re-examines
                // them against the store as it is then.
                Ok(DedupCommit::Stale) => {
                    debug!(
                        "Dedup fold of {} skipped — the pair changed mid-fold",
                        candidate.id
                    );
                    survivors.push(candidate);
                }
                Err(e) => {
                    result
                        .errors
                        .push(format!("Dedup fold failed for {}: {e}", candidate.id));
                    survivors.push(candidate);
                }
            }
        }

        debug_assert!(
            folded_count <= removal_budget_count,
            "dedup must respect the removal budget"
        );
        debug_assert_eq!(
            survivors.len() + folded_count,
            memories_count_before,
            "every memory must either survive or be folded — none may vanish"
        );
        debug_assert_eq!(
            folded_sources.len(),
            folded_count,
            "one source recorded per fold"
        );

        // Drop every folded source in a single batch — one WAL checkpoint for the
        // whole dedup pass instead of one per fold. Deferred to here (not inside
        // `commit_duplicate_fold`) precisely so it can be batched; the sources are
        // already absent from `survivors`, so this is pure write-through cleanup.
        // MATCHED on the content each fold absorbed: see `folded_sources` above.
        if !folded_sources.is_empty() {
            let pairs: Vec<(&str, &str)> = folded_sources
                .iter()
                .map(|(id, content)| (id.as_str(), content.as_str()))
                .collect();
            let removed = self.store.remove_many_matching(&pairs).await;
            if removed == folded_count {
                info!("Dream dedup: folded {folded_count} near-duplicate memories (no LLM call)");
            } else {
                info!(
                    "Dream dedup: folded {folded_count} near-duplicate memories, removed \
                     {removed} source rows ({} kept — rewritten mid-fold; no LLM call)",
                    folded_count - removed
                );
            }
        }
        survivors
    }

    /// Persist one duplicate fold: rewrite the survivor (merged content, fresh
    /// embedding, inherited FSRS). The source row is **not** dropped here — the
    /// caller batches all folded sources into a single matched removal at the
    /// end of the pass (one WAL checkpoint for the whole batch, not one per
    /// fold). Update-before-remove still holds at the pass level: every
    /// survivor rewrite commits before any source is removed, so a partial
    /// failure can only leave a transient duplicate (re-folded next cycle),
    /// never lose content.
    ///
    /// The dedup pass folds from a SNAPSHOT of the band, and a live `remember`
    /// can rewrite either half of the pair while the pass runs — this is the
    /// fold-vs-step interleave that used to lose the live write. Both halves
    /// are therefore guarded here: the survivor rewrite is a compare-and-swap
    /// on the snapshot content ([`Self::rewrite_with_binding`]), and a pure
    /// superset fold re-verifies against the survivor's LIVE content that the
    /// source's text is really present before authorizing its removal. When
    /// either check declines, [`DedupCommit::Stale`] is returned with nothing
    /// touched, and the caller keeps both memories for the next cycle.
    ///
    /// On [`DedupCommit::Committed`], the payload is the survivor's **new
    /// embedding** when the content changed (so the caller's in-memory copy
    /// can be kept in step with the store — a stale vector on a rewritten
    /// entry is the recurring bug class in this path), or `None` when a pure
    /// superset fold left the content, and thus the vector, already correct.
    async fn commit_duplicate_fold(
        &self,
        embed_fn: &EmbedFn,
        survivor: &MemoryEntry,
        source: &MemoryEntry,
        merged: &str,
    ) -> Result<DedupCommit, MemoryError> {
        debug_assert_ne!(survivor.id, source.id, "a memory cannot fold into itself");
        debug_assert!(!merged.is_empty(), "merged content must not be empty");

        // Re-embed only when the text actually changed; a pure superset fold
        // leaves the survivor's content (and therefore its vector) correct.
        let new_embedding = if merged == survivor.content {
            // No rewrite happens on this arm, so the CAS below never runs —
            // but the removal this fold authorizes is only sound while the
            // LIVE survivor still contains the source's text. The snapshot
            // said so; verify the store still does. (The residual window
            // between this check and the batch removal is closed by the
            // matched removal itself, which re-checks the SOURCE, and bounded
            // on the survivor side by every merge path being
            // content-preserving.)
            let live_contains = self
                .store
                .get(&survivor.id)
                .await
                .is_some_and(|live| live.content.contains(source.content.trim()));
            if !live_contains {
                return Ok(DedupCommit::Stale);
            }
            None
        } else {
            let embedding = (embed_fn)(merged)
                .await
                .map_err(|e| MemoryError::Io(std::io::Error::other(e)))?;
            // Empty when the rewrite raced a provider switch and queued for
            // backfill — the caller's copy must reflect that, not the stale
            // vector. `None` when the survivor changed under the fold: the
            // CAS declined, nothing was written, and the pair stays.
            match self
                .rewrite_with_binding(&survivor.id, merged, embedding, Some(&survivor.content))
                .await?
            {
                Some(stored) => Some(stored),
                None => return Ok(DedupCommit::Stale),
            }
        };

        let inherited_importance = survivor.fsrs.importance.max(source.fsrs.importance);
        let inherited_access = source.fsrs.access_count;
        self.store
            .update_fsrs(&survivor.id, |fsrs| {
                fsrs.importance = inherited_importance;
                fsrs.access_count += inherited_access;
            })
            .await?;

        // NB: the source is removed by the caller in one batch, not here.
        Ok(DedupCommit::Committed(new_embedding))
    }

    /// Consolidate a single cluster of memories
    async fn consolidate_cluster<F, Fut>(
        &self,
        cluster: &MemoryCluster,
        summarize_fn: &F,
    ) -> Result<(), MemoryError>
    where
        F: Fn(String) -> Fut + Send + Sync,
        Fut: Future<Output = Result<String, String>> + Send,
    {
        // Build prompt and get summary
        let prompt = cluster.build_consolidation_prompt();
        let summary = summarize_fn(prompt)
            .await
            .map_err(|e| MemoryError::Io(std::io::Error::other(e)))?;

        // Generate embedding for the summary
        let embed_fn = self.embed_fn.as_ref().ok_or(MemoryError::NoEmbeddingProvider)?;

        let embedding = (embed_fn)(&summary)
            .await
            .map_err(|e| MemoryError::Io(std::io::Error::other(e)))?;

        // Create consolidated entry (a fresh uuid, so it never collides with a
        // source memory).
        let consolidated = create_consolidated_entry(cluster, summary, embedding);

        // Add the consolidated memory FIRST, then remove the sources. If the add
        // fails (e.g. a dimension mismatch), `?` returns before any removal, so
        // the cluster's memories are never lost — the worst case is a transient
        // duplicate (consolidated + a source whose removal failed), which a later
        // dream cycle re-consolidates. The reverse order (remove-then-add) would
        // delete every source and then lose them all on a failed add.
        self.store.add(consolidated).await?;

        // Remove the now-superseded source memories in ONE batch (best-effort):
        // a single persistence call / WAL checkpoint for the whole cluster rather
        // than one fsync per source.
        //
        // MATCHED on each source's snapshot content, not by bare id. The
        // summary above was built from the cluster as it stood when the bands
        // were snapshotted, and the summarize call between there and here
        // takes SECONDS on the local tier — plenty of time for a live
        // `remember` to fold fresh content into a source. That fresh content
        // is not in the summary, so removing the rewritten source would
        // silently delete it. A source whose content moved is kept (transient
        // duplicate beside the summary; a later cycle re-consolidates it),
        // which is the same lossless fallback every other dream path uses.
        let source_pairs: Vec<(&str, &str)> = cluster
            .memories
            .iter()
            .map(|m| (m.id.as_str(), m.content.as_str()))
            .collect();
        let removed = self.store.remove_many_matching(&source_pairs).await;
        debug_assert!(
            removed <= source_pairs.len(),
            "removed more sources than the cluster held"
        );

        debug!(
            "Consolidated {} memories into 1 ({:?})",
            cluster.memories.len(),
            cluster.compression_level
        );

        Ok(())
    }

    /// Expand a high-importance memory with more context
    async fn expand_memory<F, Fut>(
        &self,
        memory: &MemoryEntry,
        summarize_fn: &F,
    ) -> Result<(), MemoryError>
    where
        F: Fn(String) -> Fut + Send + Sync,
        Fut: Future<Output = Result<String, String>> + Send,
    {
        let prompt = format!(
            "{}\n\nOriginal memory:\n{}\n\nEnriched memory:",
            CompressionLevel::Expand.summarization_prompt(),
            memory.content
        );

        let expanded = summarize_fn(prompt)
            .await
            .map_err(|e| MemoryError::Io(std::io::Error::other(e)))?;

        // Only update if expansion added meaningful content
        if expanded.len() > memory.content.len() {
            // Re-embed the enriched content so the vector matches it. Updating
            // content alone (the old `update_content` path) left the embedding
            // pointing at the pre-expansion text, so recall matched the stale
            // vector and the enrichment made the memory *harder* to find — the
            // opposite of expansion's intent. Mirror the merge path, which
            // re-embeds via `update_content_and_embedding` to keep content and
            // embedding consistent. A failed re-embed returns before any write,
            // so content and embedding never diverge.
            let embed_fn = self
                .embed_fn
                .as_ref()
                .ok_or(MemoryError::NoEmbeddingProvider)?;
            let embedding = (embed_fn)(&expanded)
                .await
                .map_err(|e| MemoryError::Io(std::io::Error::other(e)))?;
            // CAS on the snapshot content: the summarize call above takes
            // seconds, and enrichment of a stale snapshot must not overwrite
            // what a live write stored in the meantime. A declined swap just
            // skips this expansion; the next cycle expands the fresh text.
            if self
                .rewrite_with_binding(&memory.id, &expanded, embedding, Some(&memory.content))
                .await?
                .is_none()
            {
                debug!(
                    "Expansion of {} skipped — the memory changed mid-expansion",
                    memory.id
                );
                return Ok(());
            }
            debug!("Expanded memory {}: {} -> {} chars", memory.id, memory.content.len(), expanded.len());
        }

        Ok(())
    }
}

/// Memory statistics
#[derive(Debug, Clone, Default)]
pub struct MemoryStats {
    pub total: usize,
    pub active: usize,
    pub dormant: usize,
    pub silent: usize,
    pub unavailable: usize,
}

/// Memory entry for listing (includes computed FSRS state)
#[derive(Debug, Clone)]
pub struct MemoryListEntry {
    pub id: String,
    pub content: String,
    pub metadata: HashMap<String, String>,
    pub timestamp: i64,
    pub state: MemoryState,
    pub weight: f32,
    pub retrievability: f32,
    pub importance: f32,
    pub access_count: u32,
    pub workspace_id: Option<String>,
}

/// Memories grouped by weight bands for consolidation
#[derive(Debug, Clone, Default)]
pub struct ConsolidationBands {
    /// Weight < 0.2: compress to essence
    pub essence: Vec<MemoryEntry>,
    /// Weight 0.2-0.5: moderate compression
    pub compressed: Vec<MemoryEntry>,
    /// Weight 0.5-0.8: standard detail
    pub standard: Vec<MemoryEntry>,
    /// Weight 0.8-1.0: full detail
    pub detailed: Vec<MemoryEntry>,
    /// Weight > 1.0: expand/research
    pub expand: Vec<MemoryEntry>,
}

/// Result from memory recall
#[derive(Debug, Clone)]
pub struct RecallResult {
    pub id: String,
    /// The memory's full content.
    ///
    /// Full, because a `RecallResult` is a value, not a rendering — the caller
    /// that puts this in front of a model is the one that must page it (see
    /// [`Self::excerpt`]), and a caller that needs the whole thing (dreaming,
    /// consolidation, export) must not be silently handed a prefix.
    pub content: String,
    /// Ordinal of the chunk that produced [`Self::score`], when the hit came
    /// from chunk search. Lets a paged read open at the part that matched
    /// rather than at the beginning of a long memory.
    pub best_chunk: Option<i64>,
    /// Similarity score from vector search
    pub score: f32,
    /// FSRS weight (retrievability × importance)
    pub weight: f32,
    /// Current memory state
    pub state: MemoryState,
    pub metadata: HashMap<String, String>,
    /// Workspace ID if scoped (None = global)
    pub workspace_id: Option<String>,
}

impl RecallResult {
    /// A bounded window of [`Self::content`], as `(text, start, total)` in
    /// characters.
    ///
    /// Storage is unbounded by design, so a recall that returned whole
    /// memories would put an arbitrarily large payload into a fixed context
    /// window — and would do it `limit` times over. Paging is therefore a
    /// property of *reading*, not of storing.
    ///
    /// The window opens at `start`, clamped so it never begins past the end,
    /// and is cut on a character boundary. It reports `total` so the caller can
    /// say what fraction is shown and compute the next offset; a window that
    /// silently showed a prefix would read as the whole memory, which is the
    /// unannounced-truncation failure this codebase keeps paying for.
    #[must_use]
    pub fn excerpt(&self, start: usize, budget: usize) -> (String, usize, usize) {
        let total = self.content.chars().count();
        let start = start.min(total);
        let text: String = self.content.chars().skip(start).take(budget.max(1)).collect();
        (text, start, total)
    }
}

fn chrono_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        // Find a valid char boundary at or before max_len
        let mut end = max_len;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

/// The largest single observation the episodic writer can hand us: one semantic
/// chunk plus its `[tool → target — outcome] ` header (the target is itself
/// capped at 120 chars, so 192 covers the header whole).
///
/// Exported so the producer chunks against the same number the consumer bounds
/// against. Them drifting apart IS the bug this documents.
pub const MEMORY_OBSERVATION_MAX_CHARS: usize = MEMORY_CHUNK_MAX_CHARS + MEMORY_OBSERVATION_HEADER_CHARS;

/// The `[tool → target — outcome] ` header the episodic writer prepends. The
/// target is itself capped at 120 chars, so this covers the header whole.
pub const MEMORY_OBSERVATION_HEADER_CHARS: usize = 192;

/// Retrieval-granularity target for one embedded chunk, in chars.
///
/// This is the SOFT bound — what size makes a good retrieval unit, independent
/// of any model. A chunk far larger than a few paragraphs embeds to a centroid
/// of everything it contains: it matches most queries weakly and locates
/// nothing precisely, so recall degrades as chunks grow even when the model
/// could accept them. Use [`chunk_max_chars_for_window`] to combine this with
/// the hard bound.
pub const MEMORY_CHUNK_TARGET_CHARS: usize = 3200;

/// Conservative chars-per-token, matching the ratio the budget estimators use
/// for code (which tokenizes worse than prose). Under-estimating chars per
/// token makes the derived chunk SMALLER, which is the safe direction.
const CHARS_PER_TOKEN: usize = 3;

/// The chunk budget for an embedding model whose input window is
/// `embedding_window_tokens`.
///
/// Two independent bounds, smaller wins:
///
/// - **Hard — the model's window.** A chunk larger than the embedder accepts is
///   truncated by the embedder, and the resulting vector describes only a
///   prefix while claiming to describe the whole chunk. Nothing errors; recall
///   just quietly degrades. A 512-token embedding model (~1.5k chars) handed
///   the 3200-char target was losing half of every chunk this way.
/// - **Soft — [`MEMORY_CHUNK_TARGET_CHARS`].** Retrieval granularity, above.
///   A 32k-token embedder would otherwise permit ~100k-char chunks, which fit
///   fine and retrieve badly.
///
/// Room is left for the `[tool → target — outcome] ` header the writer prepends
/// (see [`MEMORY_OBSERVATION_MAX_CHARS`]), because the header is embedded too.
///
/// `0` (window unknown) yields the target: no information about the model is a
/// reason to keep the previous behaviour, not to guess a bigger number.
#[must_use]
pub fn chunk_max_chars_for_window(embedding_window_tokens: usize) -> usize {
    if embedding_window_tokens == 0 {
        return MEMORY_CHUNK_TARGET_CHARS;
    }
    let window_chars = embedding_window_tokens.saturating_mul(CHARS_PER_TOKEN);
    let usable = window_chars.saturating_sub(MEMORY_OBSERVATION_HEADER_CHARS);
    // Never zero: a pathologically small window still has to chunk into
    // something, and one char at a time is the floor that keeps progress.
    usable.min(MEMORY_CHUNK_TARGET_CHARS).max(1)
}

/// Kept for callers that want the compile-time target rather than a
/// model-derived budget — notably the output-reserve derivation, which is
/// about the largest routine `write_file` payload and only ever coincided with
/// the embedding chunk size.
pub const MEMORY_CHUNK_MAX_CHARS: usize = MEMORY_CHUNK_TARGET_CHARS;



/// Merge `incoming` content into `existing`, deduplicating and bounding length.
///
/// Used when an ingest lands in the `Update` similarity band (related-but-distinct):
/// instead of storing a near-duplicate, fold the new information into the existing
/// memory. Rules:
/// - If either string contains the other (after trimming), keep the superset.
/// - Otherwise append `incoming` after `existing` with a `"; "` separator.
/// - The result is capped at `MEMORY_MERGE_MAX_BYTES`; if appending would exceed
///   the cap, the existing content is kept unchanged (no unbounded growth).
///
/// The returned string is never empty when `existing` is non-empty.
/// Index of the first memory in `survivors` that `candidate` is a true duplicate
/// of, or `None` if it is genuinely new information.
///
/// Pure (no IO, no clock) so the fold policy is exhaustively testable. Two gates,
/// cheapest first: identical workspace scope, then cosine similarity past the
/// [`IngestAction::Reinforce`] line. Embeddings are stored normalized, so the
/// SIMD cosine is exact here; a dimension mismatch or empty vector yields no
/// match rather than a panic.
#[must_use]
fn find_duplicate_target(survivors: &[MemoryEntry], candidate: &MemoryEntry) -> Option<usize> {
    if candidate.embedding.is_empty() {
        return None;
    }

    survivors.iter().position(|survivor| {
        if !crate::consolidation::same_scope(survivor, candidate) {
            return false;
        }
        if survivor.embedding.len() != candidate.embedding.len() {
            return false;
        }
        let similarity =
            nanna_simd::cosine_similarity_f32(&survivor.embedding, &candidate.embedding);
        similarity.is_finite()
            && IngestAction::from_similarity(similarity) == IngestAction::Reinforce
    })
}

/// An episodic record — something the agent DID or THOUGHT — as opposed to a
/// durable fact it was told.
///
/// Episodes come from the tool and step capture paths, which stamp a
/// `source_id` on tool results and a `kind` on assistant text and reasoning
/// blocks. They arrive in volume, each one is individually cheap, and they are
/// the material dreaming exists to narrate into a throughline. Durable facts
/// are the ones drift protection is for, so the two are treated differently at
/// high weight.
/// Active vector, model stamp, and bucket map for a fresh entry, resolved
/// against the CURRENT binding.
///
/// A vector whose width differs from the binding's was produced under a
/// binding that changed while the write was in flight (provider failover
/// mid-call). It cannot be kept: as the active vector it would fail the
/// store's width gate, and bucketed under the new model it would poison that
/// bucket with another model's space. So the vector is discarded and the
/// entry QUEUED — empty active vector, no buckets — for backfill under the
/// current binding. The write always lands; losing a vector costs temporary
/// searchability, losing the write cost the memory (the 2026-08-02 incident
/// WARN-failed every write for minutes).
///
/// Pure so the race policy is testable without a store.
#[must_use]
fn resolve_entry_vectors(
    model: Option<String>,
    bound_dim: usize,
    embedding: Vec<f32>,
) -> (Option<String>, Vec<f32>, HashMap<String, Vec<f32>>) {
    if embedding.len() != bound_dim {
        info!(
            "Embedding width {} does not match the current binding ({} dims) — \
             entry queued for backfill instead of failing the write",
            embedding.len(),
            bound_dim
        );
        return (None, Vec::new(), HashMap::new());
    }
    let mut buckets = HashMap::new();
    if let Some(ref model) = model {
        buckets.insert(model.clone(), embedding.clone());
    }
    (model, embedding, buckets)
}

fn is_episodic_memory(entry: &MemoryEntry) -> bool {
    entry.metadata.contains_key("source_id") || entry.metadata.contains_key("kind")
}

fn merge_memory_content(existing: &str, incoming: &str) -> String {
    // Separator is ASCII so byte math for the length bound is exact.
    const SEPARATOR: &str = "; ";

    let existing = existing.trim();
    let incoming = incoming.trim();

    // Superset dedup: keep the longer when one already contains the other.
    // (An empty `incoming` is contained by everything, so `existing` is kept.)
    if existing.contains(incoming) {
        return existing.to_string();
    }
    if incoming.contains(existing) {
        return incoming.to_string();
    }

    // Unbounded append. There was a byte cap here, and it did not merely bound
    // growth: past the ceiling it returned `existing` and DISCARDED the
    // incoming observation — a silent write failure reported as a successful
    // merge. Storage has no size limit now, and the constraint the cap was
    // standing in for (an embedding model's finite input window) is handled
    // where it actually applies, by chunking the content for embedding rather
    // than by refusing to remember it.
    format!("{existing}{SEPARATOR}{incoming}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_memory_service_no_embed() {
        let service = MemoryService::new(MemoryServiceConfig::default());

        // Should fail without embed function
        let result = service.remember("test", HashMap::new()).await;
        assert!(result.is_err());
    }

    /// The user-reported bug: with no embedding provider, `recall` failed with
    /// `IO error: No embedding function configured`. That reaches the *model*,
    /// which cannot act on an IO error — so memory looked broken rather than
    /// unconfigured. It must name the real condition instead.
    #[tokio::test]
    async fn recall_without_an_embedding_provider_names_the_real_problem() {
        let service = MemoryService::new(MemoryServiceConfig::default());

        let err = service
            .recall("anything")
            .await
            .expect_err("recall cannot run without an embedding provider");

        assert!(
            matches!(err, MemoryError::NoEmbeddingProvider),
            "expected the dedicated variant, got: {err:?}"
        );

        let message = err.to_string();
        assert!(
            !message.starts_with("IO error"),
            "an unconfigured provider is not an IO failure: {message}"
        );
        assert!(
            message.contains("embedding provider"),
            "the message must name what is missing: {message}"
        );
        assert!(
            message.contains("Settings") || message.contains("Ollama"),
            "the message must say how to fix it: {message}"
        );
    }

    /// Every embed-dependent entry point should report the same actionable
    /// condition, not just `recall` — the reporter hit it through `recall`, but
    /// `remember` is the same latent trap.
    #[tokio::test]
    async fn remember_without_an_embedding_provider_uses_the_same_variant() {
        let service = MemoryService::new(MemoryServiceConfig::default());

        let err = service
            .remember("test", HashMap::new())
            .await
            .expect_err("remember cannot run without an embedding provider");

        assert!(
            matches!(err, MemoryError::NoEmbeddingProvider),
            "expected the dedicated variant, got: {err:?}"
        );
    }

    #[test]
    fn test_merge_memory_content_superset_dedup() {
        // When one string contains the other, keep the superset (no duplication).
        assert_eq!(
            merge_memory_content("sky is blue", "sky is blue"),
            "sky is blue"
        );
        assert_eq!(
            merge_memory_content("the sky is blue today", "sky is blue"),
            "the sky is blue today"
        );
        assert_eq!(
            merge_memory_content("sky is blue", "the sky is blue today"),
            "the sky is blue today"
        );
        // Trimming: surrounding whitespace does not defeat the containment check.
        assert_eq!(
            merge_memory_content("  sky is blue  ", "sky is blue"),
            "sky is blue"
        );
    }

    #[test]
    fn test_merge_memory_content_append_distinct() {
        // Distinct-but-related content is appended with a separator.
        assert_eq!(
            merge_memory_content("sky is blue", "grass is green"),
            "sky is blue; grass is green"
        );
    }

    /// Storage is unbounded. The byte cap this replaces did not bound growth so
    /// much as drop writes: past the ceiling it returned the existing content
    /// and discarded the incoming observation, reporting success. A merge must
    /// never lose the thing being merged in.
    #[test]
    fn test_merge_memory_content_is_unbounded() {
        let existing = "a".repeat(64_000);
        let incoming = "b".repeat(64_000);
        let merged = merge_memory_content(&existing, &incoming);
        assert!(
            merged.contains(&incoming),
            "the incoming observation must survive the merge at any size"
        );
        assert!(merged.starts_with(&existing), "and so must the existing one");
        assert_eq!(merged.len(), existing.len() + 2 + incoming.len());
    }

    #[tokio::test]
    async fn consolidate_cluster_never_loses_memories_on_failed_add() {
        use std::sync::Arc;

        // The consolidated summary embeds to the WRONG dimension (2, not the
        // store's 3), so `store.add(consolidated)` fails with a dimension
        // mismatch — the exact partial failure the add-then-remove ordering
        // guards against. The sources must survive: losing them would be
        // irreversible data loss.
        const SUMMARY: &str = "CONSOLIDATED";
        let embed: EmbedFn = Arc::new(|text: &str| {
            let v = if text == SUMMARY {
                vec![1.0_f32, 0.0] // wrong dimension → add fails
            } else {
                vec![1.0_f32, 0.0, 0.0]
            };
            Box::pin(async move { Ok(v) })
        });
        let config = MemoryServiceConfig {
            dimension: 3,
            ..Default::default()
        };
        let service = MemoryService::new(config).with_embed_fn(embed);

        let mk = |id: &str| MemoryEntry {
            embedding_model: None,
            embeddings: HashMap::new(),
            id: id.to_string(),
            content: format!("memory {id}"),
            embedding: vec![1.0, 0.0, 0.0],
            metadata: HashMap::new(),
            timestamp: 0,
            fsrs: FsrsState::default(),
            workspace_id: None,
        };
        service.add_entry(mk("a")).await.unwrap();
        service.add_entry(mk("b")).await.unwrap();
        assert_eq!(service.count().await, 2);

        let cluster = MemoryCluster::new(
            vec![mk("a"), mk("b")],
            CompressionLevel::Compressed,
            &service.config.fsrs,
        );
        let summarize = |_p: String| async move { Ok::<_, String>(SUMMARY.to_string()) };

        let result = service.consolidate_cluster(&cluster, &summarize).await;
        assert!(
            result.is_err(),
            "a dimension-mismatched summary embedding must fail the add"
        );

        // The critical assertion: a failed add must NOT have deleted the sources.
        assert_eq!(
            service.count().await,
            2,
            "consolidation must never lose memories when the add fails"
        );
        assert!(service.get("a").await.is_some(), "source a survives");
        assert!(service.get("b").await.is_some(), "source b survives");
    }

    #[tokio::test]
    async fn expand_memory_reembeds_the_enriched_content() {
        use std::sync::Arc;

        // The original content embeds to [1,0,0]; the enriched content embeds to
        // an ORTHOGONAL [0,1,0]. After expansion the stored embedding must be the
        // enriched vector, not the stale original — otherwise recall matches the
        // pre-expansion text and the enrichment hurts retrievability.
        const ORIGINAL: &str = "short";
        let embed: EmbedFn = Arc::new(|text: &str| {
            let v = if text == ORIGINAL {
                vec![1.0_f32, 0.0, 0.0]
            } else {
                vec![0.0_f32, 1.0, 0.0] // any enriched text
            };
            Box::pin(async move { Ok(v) })
        });
        let config = MemoryServiceConfig {
            dimension: 3,
            ..Default::default()
        };
        let service = MemoryService::new(config).with_embed_fn(embed);

        let entry = MemoryEntry {
            embedding_model: None,
            embeddings: HashMap::new(),
            id: "m".to_string(),
            content: ORIGINAL.to_string(),
            embedding: vec![1.0, 0.0, 0.0],
            metadata: HashMap::new(),
            timestamp: 0,
            fsrs: FsrsState::default(),
            workspace_id: None,
        };
        service.add_entry(entry.clone()).await.unwrap();

        // summarize_fn returns a strictly longer, enriched version.
        let enriched = format!("{ORIGINAL} with a lot more explanatory context added");
        let summarize = {
            let enriched = enriched.clone();
            move |_p: String| {
                let enriched = enriched.clone();
                async move { Ok::<_, String>(enriched) }
            }
        };

        service
            .expand_memory(&entry, &summarize)
            .await
            .expect("expansion must succeed");

        // A raw search by the ENRICHED vector [0,1,0] must strongly match the
        // memory — proving its stored embedding was re-generated from the
        // enriched content, not left at the stale original [1,0,0].
        let by_enriched = service.search_by_embedding(&[0.0, 1.0, 0.0], 1).await;
        assert_eq!(by_enriched.len(), 1);
        let (hit, score) = &by_enriched[0];
        assert_eq!(hit.content, enriched, "content must be the enriched text");
        assert!(
            *score > 0.99,
            "enriched vector must strongly match the re-embedded memory, got {score}"
        );

        // Conversely, the stale original vector [1,0,0] must NOT match well — if
        // it did, the embedding was never refreshed (the bug this guards).
        let by_stale = service.search_by_embedding(&[1.0, 0.0, 0.0], 1).await;
        assert!(
            by_stale[0].1 < 0.01,
            "the pre-expansion vector must no longer match, got {}",
            by_stale[0].1
        );
    }

    // ---- Dream phase (b): deterministic near-duplicate folding -------------

    /// Build a dim-3 service whose embedder is irrelevant to the fold decision
    /// (entries are inserted with explicit vectors) but is needed to re-embed a
    /// survivor whose content changed.
    fn dedup_service() -> MemoryService {
        let embed: EmbedFn =
            Arc::new(|_text: &str| Box::pin(async { Ok(vec![1.0_f32, 0.0, 0.0]) }));
        MemoryService::new(MemoryServiceConfig {
            dimension: 3,
            ..Default::default()
        })
        .with_embed_fn(embed)
    }

    fn dedup_entry(
        id: &str,
        content: &str,
        embedding: Vec<f32>,
        workspace_id: Option<&str>,
    ) -> MemoryEntry {
        MemoryEntry {
            id: id.to_string(),
            content: content.to_string(),
            embedding_model: None,
            embeddings: HashMap::new(),
            embedding,
            metadata: HashMap::new(),
            timestamp: 0,
            fsrs: FsrsState::new(),
            workspace_id: workspace_id.map(str::to_string),
        }
    }

    #[test]
    fn duplicate_target_requires_the_reinforce_similarity_band() {
        // Identical vector -> cosine 1.0 -> Reinforce -> a duplicate.
        let survivor = dedup_entry("a", "x", vec![1.0, 0.0, 0.0], None);
        let same = dedup_entry("b", "x", vec![1.0, 0.0, 0.0], None);
        assert_eq!(
            find_duplicate_target(std::slice::from_ref(&survivor), &same),
            Some(0)
        );

        // Merely *related* (~0.85, the Update band) is NOT a duplicate — that
        // content is distinct and must still reach the summarizer.
        let related = dedup_entry("c", "y", vec![0.85, 0.53, 0.0], None);
        assert_eq!(
            find_duplicate_target(std::slice::from_ref(&survivor), &related),
            None
        );
    }

    #[test]
    fn duplicate_target_never_crosses_a_workspace_boundary() {
        // Byte-identical content and vector, different scope: folding these
        // would leak private content or re-home a memory.
        let survivor = dedup_entry("a", "same", vec![1.0, 0.0, 0.0], Some("ws-1"));
        let other_ws = dedup_entry("b", "same", vec![1.0, 0.0, 0.0], Some("ws-2"));
        assert_eq!(
            find_duplicate_target(std::slice::from_ref(&survivor), &other_ws),
            None
        );

        // Global vs workspace is also a boundary.
        let global = dedup_entry("c", "same", vec![1.0, 0.0, 0.0], None);
        assert_eq!(
            find_duplicate_target(std::slice::from_ref(&survivor), &global),
            None
        );
    }

    #[test]
    fn duplicate_target_tolerates_degenerate_embeddings() {
        // Negative space: neither a dimension mismatch nor an empty vector may
        // match (or panic) — both mean "cannot judge", so never fold.
        let survivor = dedup_entry("a", "x", vec![1.0, 0.0, 0.0], None);
        let wrong_dims = dedup_entry("b", "x", vec![1.0, 0.0], None);
        assert_eq!(
            find_duplicate_target(std::slice::from_ref(&survivor), &wrong_dims),
            None
        );

        let empty = dedup_entry("c", "x", Vec::new(), None);
        assert_eq!(
            find_duplicate_target(std::slice::from_ref(&survivor), &empty),
            None
        );
    }

    /// The headline: true duplicates are folded away with **no summarizer call**.
    #[tokio::test]
    async fn dedup_folds_duplicates_without_paying_the_summarizer() {
        let service = dedup_service();
        let mut result = ConsolidationResult::default();

        // Three restatements of one fact (identical vectors => Reinforce band).
        let band = vec![
            dedup_entry("m1", "prefers dark mode", vec![1.0, 0.0, 0.0], None),
            dedup_entry("m2", "prefers dark mode", vec![1.0, 0.0, 0.0], None),
            dedup_entry("m3", "likes dark themes", vec![1.0, 0.0, 0.0], None),
        ];
        for entry in &band {
            service.add_entry(entry.clone()).await.expect("seed");
        }
        assert_eq!(service.count().await, 3);

        let survivors = service.fold_near_duplicates(band, 10, &mut result).await;

        assert_eq!(survivors.len(), 1, "three duplicates collapse to one");
        assert_eq!(result.memories_deduped, 2);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            service.count().await,
            1,
            "sources must be removed from the store"
        );

        // Losslessness: the exact-duplicate is absorbed by superset dedup, and
        // the differently-worded restatement is appended, not discarded.
        let survivor = &survivors[0];
        assert!(survivor.content.contains("prefers dark mode"));
        assert!(survivor.content.contains("likes dark themes"));
    }

    #[tokio::test]
    async fn dedup_respects_the_removal_budget() {
        let service = dedup_service();
        let mut result = ConsolidationResult::default();

        let band: Vec<MemoryEntry> = (0..4)
            .map(|i| dedup_entry(&format!("m{i}"), "same fact", vec![1.0, 0.0, 0.0], None))
            .collect();
        for entry in &band {
            service.add_entry(entry.clone()).await.expect("seed");
        }

        // Budget of 1 => exactly one fold, three memories left standing.
        let survivors = service.fold_near_duplicates(band, 1, &mut result).await;
        assert_eq!(result.memories_deduped, 1);
        assert_eq!(survivors.len(), 3);
        assert_eq!(service.count().await, 3);
    }

    #[tokio::test]
    async fn dedup_leaves_distinct_memories_for_the_summarizer() {
        let service = dedup_service();
        let mut result = ConsolidationResult::default();

        // Orthogonal vectors: nowhere near the Reinforce band.
        let band = vec![
            dedup_entry("m1", "likes tea", vec![1.0, 0.0, 0.0], None),
            dedup_entry("m2", "owns a bike", vec![0.0, 1.0, 0.0], None),
        ];
        for entry in &band {
            service.add_entry(entry.clone()).await.expect("seed");
        }

        let survivors = service.fold_near_duplicates(band, 10, &mut result).await;
        assert_eq!(survivors.len(), 2, "distinct content must not be folded");
        assert_eq!(result.memories_deduped, 0);
        assert_eq!(service.count().await, 2);
    }

    #[tokio::test]
    async fn dedup_survivor_inherits_importance_and_access_count() {
        let service = dedup_service();
        let mut result = ConsolidationResult::default();

        let mut weak = dedup_entry("weak", "fact", vec![1.0, 0.0, 0.0], None);
        weak.fsrs.importance = 1.0;
        weak.fsrs.access_count = 2;
        let mut strong = dedup_entry("strong", "fact restated", vec![1.0, 0.0, 0.0], None);
        strong.fsrs.importance = 5.0;
        strong.fsrs.access_count = 7;

        service.add_entry(weak.clone()).await.expect("seed");
        service.add_entry(strong.clone()).await.expect("seed");

        let survivors = service
            .fold_near_duplicates(vec![weak, strong], 10, &mut result)
            .await;

        assert_eq!(result.memories_deduped, 1);
        assert_eq!(survivors.len(), 1);
        let survivor = &survivors[0];
        // The strongest memory survives (ranked first by FSRS weight)…
        assert_eq!(survivor.id, "strong");
        // …and inherits the union of the recall history, not just its own.
        assert!((survivor.fsrs.importance - 5.0).abs() < f32::EPSILON);
        assert_eq!(survivor.fsrs.access_count, 9);
    }

    /// The `Detailed` band (weight 0.8..=1.0 — the freshest, most important
    /// memories) is never summarized, but it **is** deduplicated: exact
    /// restatements of your best-established facts are precisely the ones worth
    /// collapsing, and the fold is lossless so it cannot cost anything.
    #[tokio::test]
    async fn detailed_band_is_deduplicated_but_never_summarized() {
        let service = dedup_service();

        // Fresh + importance 1.0 => weight lands in the Detailed band.
        for index in 0..4 {
            let mut entry = dedup_entry(
                &format!("hot{index}"),
                "the deploy key lives in the vault",
                vec![1.0, 0.0, 0.0],
                None,
            );
            entry.fsrs.importance = 1.0;
            entry.fsrs.stability = 100.0;
            service.add_entry(entry).await.expect("seed");
        }

        // Sanity: these really are in Detailed, not a compressible band.
        let bands = service.get_consolidation_bands().await;
        assert_eq!(bands.detailed.len(), 4, "fixture must populate Detailed");

        let summarizer_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&summarizer_calls);
        let result = service
            .consolidate(
                &ConsolidationConfig {
                    min_remaining_memories: 1,
                    max_compression_ratio: 0.9,
                    ..ConsolidationConfig::default()
                },
                move |_prompt| {
                    let calls = Arc::clone(&calls);
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(String::from("summary"))
                    }
                },
            )
            .await
            .expect("consolidate");

        assert!(
            result.memories_deduped > 0,
            "Detailed duplicates must still be folded: {result:?}"
        );
        assert_eq!(
            result.clusters_formed, 0,
            "Detailed must never be summarized: {result:?}"
        );
        assert_eq!(
            summarizer_calls.load(Ordering::SeqCst),
            0,
            "no LLM call may be made for the Detailed band"
        );
        assert!(service.count().await < 4, "the band must have compressed");
    }

    /// A rewritten survivor must not keep its pre-merge vector. Stale embeddings
    /// on rewritten entries are the recurring bug class in this path: the
    /// returned survivor is fed straight into `cluster_memories`, so a stale
    /// vector would cluster it by content it no longer holds.
    #[tokio::test]
    async fn dedup_survivor_embedding_tracks_its_rewritten_content() {
        // An embedder that returns a *distinct* vector for merged content, so a
        // stale copy is detectable rather than coincidentally equal.
        let embed: EmbedFn = Arc::new(|text: &str| {
            let v = if text.contains(';') {
                vec![0.0_f32, 1.0, 0.0] // merged (the fold joins with "; ")
            } else {
                vec![1.0_f32, 0.0, 0.0] // original
            };
            Box::pin(async move { Ok(v) })
        });
        let service = MemoryService::new(MemoryServiceConfig {
            dimension: 3,
            ..Default::default()
        })
        .with_embed_fn(embed);
        let mut result = ConsolidationResult::default();

        // Distinct wording => the fold appends, so the content really changes.
        let band = vec![
            dedup_entry("a", "deploy key in vault", vec![1.0, 0.0, 0.0], None),
            dedup_entry("b", "key stored in the vault", vec![1.0, 0.0, 0.0], None),
        ];
        for entry in &band {
            service.add_entry(entry.clone()).await.expect("seed");
        }

        let survivors = service.fold_near_duplicates(band, 10, &mut result).await;
        assert_eq!(result.memories_deduped, 1);

        let survivor = &survivors[0];
        assert!(survivor.content.contains(';'), "content must have merged");
        assert_ne!(
            survivor.embedding,
            vec![1.0_f32, 0.0, 0.0],
            "survivor must not keep its pre-merge embedding"
        );

        // …and it must match what the store actually persisted.
        let stored = service
            .search_by_embedding(&[0.0_f32, 1.0, 0.0], 1)
            .await
            .first()
            .map(|(entry, _)| entry.clone())
            .expect("survivor must be in the store");
        assert_eq!(stored.id, survivor.id);
        assert_eq!(stored.content, survivor.content);
    }

    #[tokio::test]
    async fn dedup_is_a_no_op_without_an_embedder() {
        // Without an embedder a survivor could not be re-embedded, leaving its
        // content and vector inconsistent — so the pass must decline entirely.
        let service = MemoryService::new(MemoryServiceConfig {
            dimension: 3,
            ..Default::default()
        });
        let mut result = ConsolidationResult::default();
        let band = vec![
            dedup_entry("m1", "same", vec![1.0, 0.0, 0.0], None),
            dedup_entry("m2", "same", vec![1.0, 0.0, 0.0], None),
        ];

        let survivors = service.fold_near_duplicates(band, 10, &mut result).await;
        assert_eq!(survivors.len(), 2);
        assert_eq!(result.memories_deduped, 0);
    }

    #[tokio::test]
    async fn test_smart_ingest_merges_related_memory() {
        use std::sync::Arc;

        // Dimension-3 embeddings crafted so the two related contents land in the
        // Update similarity band (0.75..=0.92): cosine([1,0,0],[0.85,0.53,0]) ~= 0.85.
        let embed: EmbedFn = Arc::new(|text: &str| {
            let v = match text {
                "sky is blue" => vec![1.0_f32, 0.0, 0.0],
                "sky is deep blue" => vec![0.85_f32, 0.53, 0.0],
                _ => vec![0.30_f32, 0.30, 0.90], // merged/other content
            };
            Box::pin(async move { Ok(v) })
        });
        let config = MemoryServiceConfig {
            dimension: 3,
            ..Default::default()
        };
        let service = MemoryService::new(config).with_embed_fn(embed);

        // First ingest creates a memory.
        let (id1, action1) = service
            .smart_ingest("sky is blue", HashMap::new())
            .await
            .unwrap();
        assert_eq!(action1, IngestAction::Create);
        assert_eq!(service.count().await, 1);

        // Second, related-but-distinct ingest merges instead of creating a duplicate.
        let (id2, action2) = service
            .smart_ingest("sky is deep blue", HashMap::new())
            .await
            .unwrap();
        assert_eq!(action2, IngestAction::Update);
        assert_eq!(id2, id1, "merge must reuse the existing memory id");
        assert_eq!(
            service.count().await,
            1,
            "merge must not create a second memory"
        );

        // The surviving memory carries both mentions.
        let merged = service.get(&id1).await.expect("memory still present");
        assert!(merged.content.contains("sky is blue"));
        assert!(merged.content.contains("deep blue"));
    }

    #[tokio::test]
    async fn test_remember_with_importance_merges_related_memory() {
        use std::sync::Arc;

        // Same Update-band construction as the smart_ingest test.
        let embed: EmbedFn = Arc::new(|text: &str| {
            let v = match text {
                "sky is blue" => vec![1.0_f32, 0.0, 0.0],
                "sky is deep blue" => vec![0.85_f32, 0.53, 0.0],
                _ => vec![0.30_f32, 0.30, 0.90],
            };
            Box::pin(async move { Ok(v) })
        });
        let config = MemoryServiceConfig {
            dimension: 3,
            ..Default::default()
        };
        let service = MemoryService::new(config).with_embed_fn(embed);

        let (id1, action1) = service
            .remember_with_importance("sky is blue", HashMap::new(), 3.0)
            .await
            .unwrap();
        assert_eq!(action1, IngestAction::Create);

        // The explicit-importance path also merges rather than duplicating.
        let (id2, action2) = service
            .remember_with_importance("sky is deep blue", HashMap::new(), 3.0)
            .await
            .unwrap();
        assert_eq!(action2, IngestAction::Update);
        assert_eq!(id2, id1);
        assert_eq!(service.count().await, 1, "merge must not create a second memory");

        let merged = service.get(&id1).await.expect("memory still present");
        assert!(merged.content.contains("sky is blue"));
        assert!(merged.content.contains("deep blue"));
    }

    /// An embed stub whose vector width is switchable mid-test — the shape of
    /// a provider failover as the service sees it.
    fn switchable_width_embed(width: &Arc<AtomicUsize>) -> EmbedFn {
        let width = width.clone();
        Arc::new(move |_text: &str| {
            let n = width.load(Ordering::SeqCst);
            Box::pin(async move { Ok(vec![0.5_f32; n]) })
        })
    }

    /// A store that records chunk state so the backfill can be driven without
    /// a database.
    #[derive(Default)]
    struct ChunkBackfillDb {
        /// (chunk_id, content, embedded_by)
        chunks: std::sync::Mutex<Vec<(i64, String, Option<String>)>>,
        unchunked: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl crate::MemoryPersistence for ChunkBackfillDb {
        async fn save_entry(&self, _e: &MemoryEntry) -> Result<(), MemoryError> { Ok(()) }
        async fn remove_entry(&self, _id: &str) -> Result<(), MemoryError> { Ok(()) }
        async fn update_entry_fsrs(&self, _id: &str, _f: &crate::FsrsState) -> Result<(), MemoryError> { Ok(()) }
        async fn update_entry_content(&self, _id: &str, _c: &str) -> Result<(), MemoryError> { Ok(()) }
        async fn load_all(&self) -> Result<Vec<MemoryEntry>, MemoryError> { Ok(Vec::new()) }

        async fn chunks_needing_embedding(
            &self,
            model: &str,
            limit: usize,
            _seed: bool,
        ) -> Result<Vec<crate::PendingChunk>, MemoryError> {
            Ok(self
                .chunks
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, _, by)| by.as_deref() != Some(model))
                .take(limit)
                .map(|(id, content, _)| crate::PendingChunk {
                    chunk_id: *id,
                    memory_id: format!("m{id}"),
                    ordinal: 0,
                    content: content.clone(),
                })
                .collect())
        }

        async fn set_chunk_embedding(
            &self,
            chunk_id: i64,
            _embedding: &[f32],
            model: &str,
        ) -> Result<(), MemoryError> {
            for (id, _, by) in self.chunks.lock().unwrap().iter_mut() {
                if *id == chunk_id {
                    *by = Some(model.to_string());
                }
            }
            Ok(())
        }

        async fn parents_without_chunks(&self, limit: usize) -> Result<Vec<String>, MemoryError> {
            Ok(self.unchunked.lock().unwrap().iter().take(limit).cloned().collect())
        }

        async fn replace_chunks(
            &self,
            memory_id: &str,
            _workspace_id: Option<&str>,
            _model: Option<&str>,
            chunks: &[crate::ChunkWrite],
        ) -> Result<(), MemoryError> {
            self.unchunked.lock().unwrap().retain(|id| id != memory_id);
            let mut store = self.chunks.lock().unwrap();
            let next = store.iter().map(|(id, _, _)| *id).max().unwrap_or(0) + 1;
            for (i, c) in chunks.iter().enumerate() {
                store.push((
                    next + i as i64,
                    c.content.clone(),
                    c.embedding_model.clone().filter(|_| c.embedding.is_some()),
                ));
            }
            Ok(())
        }
    }

    /// The 2026-08-04 incident: the store latched at 1536 (a paid embedder
    /// answering 402) while the live provider produced 768. Every write for a
    /// day queued for backfill — 2167 of them — and the backfill could not
    /// drain them either, because activating a backfilled vector runs the same
    /// width gate. Queueing is CORRECT for the race it was written for, which
    /// is why nothing escalated: a correct-looking line repeated 2167 times
    /// still reads as correct.
    #[tokio::test]
    async fn a_persistent_width_mismatch_re_probes_the_binding() {
        // Binding says 1536; the live provider produces 768, consistently.
        let width = Arc::new(AtomicUsize::new(768));
        let service = MemoryService::new(MemoryServiceConfig {
            dimension: 1536,
            ..Default::default()
        })
        .with_embed_fn(switchable_width_embed(&width));

        assert_eq!(service.dimension(), 1536, "precondition — a stale binding");

        for i in 0..MemoryService::WIDTH_MISMATCH_ESCALATION {
            service
                .remember_with_importance(&format!("observation {i}"), HashMap::new(), 3.0)
                .await
                .expect("the write must always land, vector or not");
        }

        assert_eq!(
            service.dimension(),
            768,
            "a mismatch that repeats is a stale binding, and must re-probe rather than queue \
             writes forever"
        );
    }

    /// A handful of mismatches IS the race the queue policy was written for —
    /// writes already in flight when a switch landed, carrying the previous
    /// provider's width. Re-probing on those would flap the latch.
    #[tokio::test]
    async fn a_brief_width_mismatch_does_not_disturb_the_binding() {
        let width = Arc::new(AtomicUsize::new(768));
        let service = MemoryService::new(MemoryServiceConfig {
            dimension: 1536,
            ..Default::default()
        })
        .with_embed_fn(switchable_width_embed(&width));

        for i in 0..(MemoryService::WIDTH_MISMATCH_ESCALATION - 1) {
            service
                .remember_with_importance(&format!("in flight {i}"), HashMap::new(), 3.0)
                .await
                .expect("write");
        }
        assert_eq!(service.dimension(), 1536, "a short streak must not rebind");
    }

    /// A width that agrees clears the streak, so mismatches have to be
    /// genuinely consecutive — otherwise a slow drip of races across an
    /// otherwise healthy day would eventually escalate on its own.
    #[tokio::test]
    async fn an_agreeing_write_clears_the_streak() {
        let width = Arc::new(AtomicUsize::new(4));
        let service = MemoryService::new(MemoryServiceConfig {
            dimension: 4,
            ..Default::default()
        })
        .with_embed_fn(switchable_width_embed(&width));

        for _ in 0..(MemoryService::WIDTH_MISMATCH_ESCALATION - 1) {
            width.store(9, Ordering::SeqCst);
            service.remember_with_importance("mismatch", HashMap::new(), 3.0).await.expect("write");
            width.store(4, Ordering::SeqCst);
            service.remember_with_importance("agreement", HashMap::new(), 3.0).await.expect("write");
        }
        assert_eq!(service.dimension(), 4, "interleaved agreement must reset the streak");
    }

    fn recall_of(content: &str) -> RecallResult {
        RecallResult {
            id: "m".to_string(),
            content: content.to_string(),
            best_chunk: None,
            score: 0.9,
            weight: 1.0,
            state: MemoryState::Active,
            metadata: HashMap::new(),
            workspace_id: None,
        }
    }

    /// Storage is unbounded, so reading has to be paged — and every page has
    /// to report the whole size, or a prefix is indistinguishable from the
    /// complete memory and the reader stops early.
    #[test]
    fn an_excerpt_reports_the_true_total_not_the_page_size() {
        let long = "x".repeat(10_000);
        let (text, start, total) = recall_of(&long).excerpt(0, 100);
        assert_eq!(text.chars().count(), 100);
        assert_eq!(start, 0);
        assert_eq!(total, 10_000, "the page must not hide how much there is");
    }

    /// Paging must terminate: the last page is short, the page after the end
    /// is empty, and neither panics. Offsets past the end are what a naive
    /// continuation loop produces.
    #[test]
    fn paging_terminates_at_the_end_instead_of_panicking() {
        let content = "abcdefghij";
        let r = recall_of(content);

        let (last, start, total) = r.excerpt(8, 100);
        assert_eq!(last, "ij");
        assert_eq!((start, total), (8, 10));

        let (past, start, total) = r.excerpt(10, 100);
        assert!(past.is_empty(), "a page at the end is empty, not an error");
        assert_eq!((start, total), (10, 10));

        let (way_past, start, _) = r.excerpt(9_999, 100);
        assert!(way_past.is_empty());
        assert_eq!(start, 10, "the offset is clamped to the end, not echoed back");
    }

    /// Walking the pages must reproduce the memory exactly — the same
    /// losslessness the chunker guarantees, on the read side.
    #[test]
    fn walking_the_pages_rebuilds_the_memory() {
        let content = "héllo wörld, 日本語もある. ".repeat(200);
        let r = recall_of(&content);
        let mut rebuilt = String::new();
        let mut offset = 0usize;
        loop {
            let (page, start, total) = r.excerpt(offset, 37);
            if page.is_empty() {
                assert_eq!(start, total);
                break;
            }
            offset = start + page.chars().count();
            rebuilt.push_str(&page);
        }
        assert_eq!(rebuilt, content, "paging must not drop or duplicate characters");
    }

    /// A zero budget would otherwise make no progress and hang any
    /// continuation loop built on the returned offset.
    #[test]
    fn a_zero_page_budget_still_advances() {
        let (page, _, _) = recall_of("abc").excerpt(0, 0);
        assert_eq!(page.chars().count(), 1, "a page must contain at least one character");
    }

    /// Returns fixed chunk hits so recall's merge can be driven without an
    /// embedding index.
    struct StubChunkSearch {
        hits: Vec<(String, i64, f32)>,
        /// The model these chunk vectors were produced by.
        model: String,
    }

    #[async_trait::async_trait]
    impl crate::MemoryPersistence for StubChunkSearch {
        async fn save_entry(&self, _e: &MemoryEntry) -> Result<(), MemoryError> { Ok(()) }
        async fn remove_entry(&self, _id: &str) -> Result<(), MemoryError> { Ok(()) }
        async fn update_entry_fsrs(&self, _id: &str, _f: &crate::FsrsState) -> Result<(), MemoryError> { Ok(()) }
        async fn update_entry_content(&self, _id: &str, _c: &str) -> Result<(), MemoryError> { Ok(()) }
        async fn load_all(&self) -> Result<Vec<MemoryEntry>, MemoryError> { Ok(Vec::new()) }
        async fn search_chunks(
            &self,
            _query: &[f32],
            model: &str,
            _limit: usize,
            _workspace_id: Option<&str>,
        ) -> Result<Vec<(String, i64, f32)>, MemoryError> {
            // Mirrors the real filter: only this model's chunks are visible.
            // A stub that ignored the model would let a cross-space regression
            // pass its own test.
            if model != self.model {
                return Ok(Vec::new());
            }
            Ok(self.hits.clone())
        }
    }

    /// The whole justification for chunking. A long memory's row vector is a
    /// centroid of everything it contains, so a query answered exactly by one
    /// paragraph can score below the recall threshold at row level. If chunk
    /// hits could only re-rank memories the row search already returned,
    /// splitting a memory would improve nothing — the row vector would still
    /// be the only way in.
    #[tokio::test]
    async fn a_chunk_hit_surfaces_a_memory_the_row_vector_missed() {
        // Orthogonal to the query, so the row-level search cannot reach it.
        let embed: EmbedFn = Arc::new(|_text: &str| {
            Box::pin(async move { Ok(vec![1.0_f32, 0.0, 0.0, 0.0]) })
        });
        let service = MemoryService::new(MemoryServiceConfig {
            dimension: 4,
            min_score: 0.4,
            ..Default::default()
        })
        .with_embed_fn(embed);

        let buried = MemoryEntry {
            id: "buried".to_string(),
            content: "a long memory whose centroid says nothing about the query".to_string(),
            embeddings: HashMap::new(),
            embedding_model: Some("prov:a".to_string()),
            embedding: vec![0.0, 1.0, 0.0, 0.0],
            metadata: HashMap::new(),
            timestamp: 0,
            fsrs: crate::FsrsState::default(),
            workspace_id: None,
        };
        service.store.add(buried).await.expect("add");

        // Row search alone finds nothing: the vectors are orthogonal.
        assert!(
            service.recall_scoped("the query", None).await.unwrap().is_empty(),
            "precondition — the row vector must not reach this memory"
        );

        // Now the same store, with a chunk of that memory matching strongly.
        service.rebind_embeddings("prov:a", 4, None).await;
        let service = service.with_persistence(Arc::new(StubChunkSearch {
            hits: vec![("buried".to_string(), 7, 0.88)],
            model: "prov:a".to_string(),
        }));
        let found = service.recall_scoped("the query", None).await.unwrap();
        assert_eq!(found.len(), 1, "the chunk hit must surface its parent");
        assert_eq!(found[0].id, "buried");
        assert!(
            (found[0].score - 0.88).abs() < 1e-6,
            "the reported score is the chunk's true similarity, not a synthetic rank: {}",
            found[0].score
        );
    }

    /// Chunk vectors carry the model that produced them, and equal width is
    /// NOT equal vector space — two different 1024-dim models compare without
    /// complaint and mean nothing. Because a chunk hit can admit its parent on
    /// chunk evidence alone, an unfiltered chunk index would let every
    /// not-yet-backfilled chunk after a same-width failover rank its memory
    /// against a query from a different space, silently and with no floor.
    ///
    /// The whole-row path never had this hole, but only by accident:
    /// `rebind_to_model` clears an entry's active vector when the new model has
    /// no bucket for it. Chunks have no equivalent, so the filter is the gate.
    #[tokio::test]
    async fn chunks_from_another_model_are_never_scored() {
        let embed: EmbedFn = Arc::new(|_text: &str| {
            Box::pin(async move { Ok(vec![1.0_f32, 0.0, 0.0, 0.0]) })
        });
        let service = MemoryService::new(MemoryServiceConfig {
            dimension: 4,
            min_score: 0.4,
            ..Default::default()
        })
        .with_embed_fn(embed)
        // The chunk index holds vectors from the PREVIOUS provider, at a
        // strong-looking similarity and the very same width.
        .with_persistence(Arc::new(StubChunkSearch {
            hits: vec![("stale".to_string(), 0, 0.97)],
            model: "prov:old".to_string(),
        }));

        service
            .store
            .add(MemoryEntry {
                id: "stale".to_string(),
                content: "embedded by the provider we just failed away from".to_string(),
                embeddings: HashMap::new(),
                embedding_model: Some("prov:old".to_string()),
                embedding: vec![0.0, 1.0, 0.0, 0.0],
                metadata: HashMap::new(),
                timestamp: 0,
                fsrs: crate::FsrsState::default(),
                workspace_id: None,
            })
            .await
            .expect("add");

        // Now bound to a DIFFERENT model of the same width.
        service.rebind_embeddings("prov:new", 4, None).await;

        assert!(
            service.recall_scoped("the query", None).await.unwrap().is_empty(),
            "a 0.97 chunk hit from another model's vector space must not surface its parent"
        );

        // Sanity: the same store DOES return it once the binding matches, so
        // the assertion above is testing the model filter and not a dead path.
        let matching = MemoryService::new(MemoryServiceConfig {
            dimension: 4,
            min_score: 0.4,
            ..Default::default()
        })
        .with_embed_fn(Arc::new(|_t: &str| Box::pin(async move { Ok(vec![1.0_f32, 0.0, 0.0, 0.0]) })))
        .with_persistence(Arc::new(StubChunkSearch {
            hits: vec![("stale".to_string(), 0, 0.97)],
            model: "prov:old".to_string(),
        }));
        matching
            .store
            .add(MemoryEntry {
                id: "stale".to_string(),
                content: "embedded by the provider we just failed away from".to_string(),
                embeddings: HashMap::new(),
                embedding_model: Some("prov:old".to_string()),
                embedding: vec![0.0, 1.0, 0.0, 0.0],
                metadata: HashMap::new(),
                timestamp: 0,
                fsrs: crate::FsrsState::default(),
                workspace_id: None,
            })
            .await
            .expect("add");
        matching.rebind_embeddings("prov:old", 4, None).await;
        assert_eq!(
            matching.recall_scoped("the query", None).await.unwrap().len(),
            1,
            "with the binding matching, the very same chunk hit must surface"
        );
    }

    /// Corroboration reorders; it never admits. A memory whose every chunk is
    /// just below the bar must stay out however many chunks it has, or the
    /// threshold would mean something weaker for long memories than short ones.
    #[tokio::test]
    async fn many_near_miss_chunks_do_not_add_up_to_a_recall() {
        let embed: EmbedFn = Arc::new(|_text: &str| {
            Box::pin(async move { Ok(vec![1.0_f32, 0.0, 0.0, 0.0]) })
        });
        let hits: Vec<(String, i64, f32)> =
            (0..30).map(|i| ("long".to_string(), i, 0.39)).collect();
        let service = MemoryService::new(MemoryServiceConfig {
            dimension: 4,
            min_score: 0.4,
            ..Default::default()
        })
        .with_embed_fn(embed)
        .with_persistence(Arc::new(StubChunkSearch { hits, model: "prov:a".to_string() }));
        // Without a bound model the chunk search refuses to run at all, which
        // would make the assertion below pass for the wrong reason.
        service.rebind_embeddings("prov:a", 4, None).await;

        service
            .store
            .add(MemoryEntry {
                id: "long".to_string(),
                content: "thirty paragraphs that each nearly match".to_string(),
                embeddings: HashMap::new(),
                embedding_model: Some("prov:a".to_string()),
                embedding: vec![0.0, 1.0, 0.0, 0.0],
                metadata: HashMap::new(),
                timestamp: 0,
                fsrs: crate::FsrsState::default(),
                workspace_id: None,
            })
            .await
            .expect("add");

        assert!(
            service.recall_scoped("the query", None).await.unwrap().is_empty(),
            "thirty near-misses must not clear a threshold none of them clears"
        );
    }

    /// A chunk whose vector came from another model is exactly as unsearchable
    /// as a chunk with no vector, so both are one queue — which is what makes
    /// an embedding-model switch a resumable incremental backfill rather than a
    /// full rebuild.
    #[tokio::test]
    async fn a_model_switch_requeues_chunks_without_rebuilding_them() {
        let db = Arc::new(ChunkBackfillDb::default());
        *db.chunks.lock().unwrap() = vec![
            (1, "alpha".into(), Some("prov:a".into())),
            (2, "beta".into(), None),
            (3, "gamma".into(), Some("prov:b".into())),
        ];
        let width = Arc::new(AtomicUsize::new(4));
        let service = MemoryService::new(MemoryServiceConfig { dimension: 4, ..Default::default() })
            .with_embed_fn(switchable_width_embed(&width))
            .with_persistence(db.clone());

        service.rebind_embeddings("prov:a", 4, None).await;
        let (embedded, _) = service.backfill_chunks("prov:a", 64).await.unwrap();
        assert_eq!(embedded, 2, "the unembedded one and the other model's one");
        assert!(
            db.chunks.lock().unwrap().iter().all(|(_, _, by)| by.as_deref() == Some("prov:a")),
            "every chunk now carries the active model's vector"
        );

        // Nothing left to do: a completed queue must report zero, or the
        // daemon's drain loop never terminates.
        assert_eq!(service.backfill_chunks("prov:a", 64).await.unwrap(), (0, 0));
    }

    /// Memories that predate chunking have no chunk rows, so they contribute
    /// nothing to the embedding queue until they are split. Running the
    /// embedding queue first would report the store complete while whole
    /// memories sat outside the index entirely.
    #[tokio::test]
    async fn unchunked_memories_are_split_before_the_queue_is_measured() {
        let db = Arc::new(ChunkBackfillDb::default());
        *db.unchunked.lock().unwrap() = vec!["old-1".into()];
        let width = Arc::new(AtomicUsize::new(4));
        let service = MemoryService::new(MemoryServiceConfig { dimension: 4, ..Default::default() })
            .with_embed_fn(switchable_width_embed(&width))
            .with_persistence(db.clone());
        service.rebind_embeddings("prov:a", 4, None).await;

        // The memory has to be in the RAM cache — it is the authority on
        // content, and chunking from a stale row would disagree with it.
        service
            .remember_with_importance("a memory written before chunking existed", HashMap::new(), 3.0)
            .await
            .expect("write");
        let id = service.store.all_entries().await[0].id.clone();
        *db.unchunked.lock().unwrap() = vec![id];
        db.chunks.lock().unwrap().clear();

        let (_, chunked) = service.backfill_chunks("prov:a", 64).await.unwrap();
        assert_eq!(chunked, 1, "the unchunked memory was split");
        assert!(
            !db.chunks.lock().unwrap().is_empty(),
            "splitting it must have produced chunk rows"
        );
    }

    /// Backfilling a bucket that is no longer the active binding poisons it
    /// with another model's vectors — wrong space, usually wrong width, and
    /// nothing downstream can tell. The guard is a skip, not an error.
    #[tokio::test]
    async fn a_chunk_backfill_for_an_inactive_binding_does_nothing() {
        let db = Arc::new(ChunkBackfillDb::default());
        *db.chunks.lock().unwrap() = vec![(1, "alpha".into(), None)];
        let width = Arc::new(AtomicUsize::new(4));
        let service = MemoryService::new(MemoryServiceConfig { dimension: 4, ..Default::default() })
            .with_embed_fn(switchable_width_embed(&width))
            .with_persistence(db.clone());

        service.rebind_embeddings("prov:a", 4, None).await;
        assert_eq!(service.backfill_chunks("prov:b", 64).await.unwrap(), (0, 0));
        assert_eq!(
            db.chunks.lock().unwrap()[0].2, None,
            "the inactive model must not have written a vector"
        );
    }

    /// Chunk geometry is the third leg of the binding, and it fails more
    /// quietly than the width: an over-long chunk is not rejected by the new
    /// embedder, it is truncated by it, and the vector then describes a prefix
    /// while claiming to describe the whole chunk. So a switch onto a
    /// narrower model must shrink the chunk size in the same breath it moves
    /// the model and the width — never one request later.
    #[tokio::test]
    async fn chunk_geometry_follows_a_provider_switch() {
        let width = Arc::new(AtomicUsize::new(4));
        let service = MemoryService::new(MemoryServiceConfig {
            dimension: 4,
            ..Default::default()
        })
        .with_embed_fn(switchable_width_embed(&width));

        // A roomy embedder: the retrieval-granularity target binds, not the
        // window.
        service.rebind_embeddings("prov:a", 4, Some(32_768)).await;
        assert_eq!(
            service.chunk_params().await.max_chars,
            MEMORY_CHUNK_TARGET_CHARS,
            "a wide window must not inflate chunks past the retrieval target"
        );

        // Failover onto a 512-token embedder. Its window is now the binding
        // constraint, and the chunk size has to have already moved.
        width.store(8, Ordering::SeqCst);
        service.rebind_embeddings("prov:b", 8, Some(512)).await;

        let (model, dimension) = service.active_binding().await;
        let params = service.chunk_params().await;
        assert_eq!(model.as_deref(), Some("prov:b"), "model moved");
        assert_eq!(dimension, 8, "width moved");
        assert!(
            params.max_chars < MEMORY_CHUNK_TARGET_CHARS,
            "chunk size did not move: {} chars still assumes the old window",
            params.max_chars
        );
        assert!(
            params.max_chars <= 512 * 3,
            "chunk of {} chars cannot fit a 512-token window",
            params.max_chars
        );
    }

    /// An undiscoverable window is not permission to send unbounded text — it
    /// means no ceiling was learned, so the retrieval target stands. Every
    /// OpenAI-compatible embeddings endpoint lands here.
    #[tokio::test]
    async fn an_unknown_window_falls_back_rather_than_unbounding() {
        let width = Arc::new(AtomicUsize::new(4));
        let service = MemoryService::new(MemoryServiceConfig {
            dimension: 4,
            ..Default::default()
        })
        .with_embed_fn(switchable_width_embed(&width));

        service.rebind_embeddings("prov:a", 4, Some(512)).await;
        let narrow = service.chunk_params().await.max_chars;
        service.rebind_embeddings("prov:a", 4, None).await;
        let unknown = service.chunk_params().await.max_chars;

        assert!(unknown > narrow, "an unknown window should relax the 512-token clamp");
        assert_eq!(
            unknown, MEMORY_CHUNK_TARGET_CHARS,
            "…but only as far as the retrieval target, never to unbounded"
        );
    }

    /// The 2026-08-02 incident: the router failed over 2048→768 and rebound
    /// the model, but the width latch stayed at 2048 — every write after the
    /// switch failed "Embedding dimension mismatch" for minutes. The binding
    /// is one pair now: after a rebind the write path must accept the new
    /// width, and the pre-switch entries must flow back in through backfill.
    #[tokio::test]
    async fn dimension_follows_a_provider_switch() {
        let width = Arc::new(AtomicUsize::new(4));
        let config = MemoryServiceConfig {
            dimension: 4,
            ..Default::default()
        };
        let service = MemoryService::new(config).with_embed_fn(switchable_width_embed(&width));

        service.rebind_embeddings("prov:a", 4, None).await;
        let (first_id, _) = service
            .remember_with_importance("the harbor light was green", HashMap::new(), 3.0)
            .await
            .expect("write under the first binding");

        // Failover: the fallback provider embeds at width 8.
        width.store(8, Ordering::SeqCst);
        service.rebind_embeddings("prov:b", 8, None).await;
        assert_eq!(service.dimension(), 8, "the width latch moved with the model");

        let (second_id, _) = service
            .remember_with_importance("the tide table was wrong", HashMap::new(), 3.0)
            .await
            .expect("a write AFTER the switch must store at the new width, not error");
        let second = service.store.get(&second_id).await.expect("stored");
        assert_eq!(second.embedding.len(), 8);
        assert_eq!(second.embedding_model.as_deref(), Some("prov:b"));

        // The pre-switch entry has no bucket for prov:b: unsearchable, not
        // lost — and backfill (targeting the CURRENT binding) completes it.
        let first = service.store.get(&first_id).await.expect("still present");
        assert!(first.embedding.is_empty(), "awaiting backfill after the rebind");
        let filled = service
            .backfill_embeddings("prov:b", 16)
            .await
            .expect("backfill runs");
        assert_eq!(filled, 1);
        let first = service.store.get(&first_id).await.expect("still present");
        assert_eq!(first.embedding.len(), 8, "re-embedded under the new binding");
        assert_eq!(first.embedding_model.as_deref(), Some("prov:b"));
    }

    /// A write whose embed call itself carries the switch (the daemon's
    /// embed_fn rebinds before returning) hands back a vector of the OLD
    /// width. That write must land as a queued-for-backfill entry — never
    /// error, which is what produced the per-write WARN loop.
    #[tokio::test]
    async fn a_write_racing_the_switch_queues_for_backfill() {
        let cell: Arc<tokio::sync::OnceCell<Arc<MemoryService>>> =
            Arc::new(tokio::sync::OnceCell::new());
        let flip = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let width = Arc::new(AtomicUsize::new(4));
        let cell_for_fn = cell.clone();
        let flip_for_fn = flip.clone();
        let width_for_fn = width.clone();
        let embed: EmbedFn = Arc::new(move |_text: &str| {
            let cell = cell_for_fn.clone();
            let flip = flip_for_fn.clone();
            let width = width_for_fn.clone();
            Box::pin(async move {
                if flip.swap(false, Ordering::SeqCst) {
                    // The failover lands INSIDE the embed call, like the
                    // daemon's embed_fn rebinding before it returns — every
                    // LATER embed comes from the new provider at width 8…
                    let service = cell.get().expect("service registered").clone();
                    service.rebind_embeddings("prov:b", 8, None).await;
                    width.store(8, Ordering::SeqCst);
                    // …but THIS call's vector was produced at the old width.
                    return Ok(vec![0.5_f32; 4]);
                }
                Ok(vec![0.5_f32; width.load(Ordering::SeqCst)])
            })
        });
        let config = MemoryServiceConfig {
            dimension: 4,
            ..Default::default()
        };
        let service = Arc::new(MemoryService::new(config).with_embed_fn(embed));
        assert!(cell.set(service.clone()).is_ok(), "first set");
        service.rebind_embeddings("prov:a", 4, None).await;

        flip.store(true, Ordering::SeqCst);
        let (id, action) = service
            .remember_with_importance("the ferry left early", HashMap::new(), 3.0)
            .await
            .expect("a write racing the switch must land, not fail");
        assert!(matches!(action, IngestAction::Create));

        let entry = service.store.get(&id).await.expect("the write landed");
        assert!(entry.embedding.is_empty(), "the stale vector was not stored");
        assert!(entry.embeddings.is_empty(), "no bucket poisoned with it either");

        // Backfill under the new binding makes it searchable again.
        let filled = service
            .backfill_embeddings("prov:b", 16)
            .await
            .expect("backfill runs");
        assert_eq!(filled, 1);
        let entry = service.store.get(&id).await.expect("still present");
        assert_eq!(entry.embedding.len(), 8);
    }

    /// The post-switch corruption from the incident log: "Backfilled N
    /// embeddings for 'openrouter:…'" AFTER the router had moved to ollama —
    /// the drain kept filling the dead provider's bucket with the new
    /// provider's vectors. Backfill must refuse a binding that is not active.
    #[tokio::test]
    async fn backfill_refuses_a_binding_that_is_not_active() {
        let width = Arc::new(AtomicUsize::new(4));
        let config = MemoryServiceConfig {
            dimension: 4,
            ..Default::default()
        };
        let service = MemoryService::new(config).with_embed_fn(switchable_width_embed(&width));

        // Written before any binding was stamped: no bucket at all, so it is
        // pending for whichever model binds next.
        let (id, _) = service
            .remember_with_importance("the anchor chain rattled", HashMap::new(), 3.0)
            .await
            .expect("write");
        service.rebind_embeddings("prov:a", 4, None).await;

        // Provider fails over before the drain for prov:a gets there.
        width.store(8, Ordering::SeqCst);
        service.rebind_embeddings("prov:b", 8, None).await;

        let filled = service
            .backfill_embeddings("prov:a", 16)
            .await
            .expect("the call itself succeeds");
        assert_eq!(filled, 0, "a non-active binding must not be filled");
        let entry = service.store.get(&id).await.expect("present");
        assert!(
            !entry.embeddings.contains_key("prov:a"),
            "the dead binding's bucket was not poisoned with the new provider's vectors"
        );

        // The ACTIVE binding drains normally.
        let filled = service
            .backfill_embeddings("prov:b", 16)
            .await
            .expect("backfill runs");
        assert_eq!(filled, 1);
        let entry = service.store.get(&id).await.expect("present");
        assert_eq!(entry.embedding.len(), 8);
        assert_eq!(entry.embedding_model.as_deref(), Some("prov:b"));
    }

    // ---- Fold vs live-write concurrency (the 2026-08-10 incident class) ----

    /// Shared slot letting a test embed_fn call back INTO the service — the
    /// deterministic hook point for "a live write lands between the fold's
    /// snapshot and its commit" (the embed of the merged text sits exactly
    /// there).
    type ServiceSlot = Arc<tokio::sync::Mutex<Option<Arc<MemoryService>>>>;

    fn hooked_dedup_service(
        hook_markers: (&'static str, &'static str),
        rewrite_target: &'static str,
        rewrite_content: &'static str,
    ) -> (Arc<MemoryService>, ServiceSlot) {
        let slot: ServiceSlot = Arc::new(tokio::sync::Mutex::new(None));
        let embed: EmbedFn = {
            let slot = Arc::clone(&slot);
            Arc::new(move |text: &str| {
                let slot = Arc::clone(&slot);
                let text = text.to_string();
                Box::pin(async move {
                    // Only the fold's MERGED text carries both markers; that
                    // embed call is the window between snapshot and commit.
                    if text.contains(hook_markers.0) && text.contains(hook_markers.1) {
                        if let Some(service) = slot.lock().await.clone() {
                            service
                                .update_content(rewrite_target, rewrite_content)
                                .await
                                .expect("the concurrent live rewrite must land");
                        }
                    }
                    Ok(vec![1.0_f32, 0.0, 0.0])
                })
            })
        };
        let service = Arc::new(
            MemoryService::new(MemoryServiceConfig {
                dimension: 3,
                ..Default::default()
            })
            .with_embed_fn(embed),
        );
        (service, slot)
    }

    /// A live rewrite of the SURVIVOR while its fold is mid-commit must win:
    /// the fold's compare-and-swap declines the stale merge, nothing is
    /// removed, and both memories stay for the next cycle. Before the guard,
    /// the stale merge overwrote the live write — content loss with no trace.
    #[tokio::test]
    async fn a_live_rewrite_during_a_fold_is_not_clobbered() {
        const LIVE: &str = "the port is 5149, and the daemon binds it at boot";
        let (service, slot) =
            hooked_dedup_service(("stated once", "stated again"), "survivor", LIVE);
        *slot.lock().await = Some(Arc::clone(&service));

        let mut survivor =
            dedup_entry("survivor", "the port is 5149 — stated once", vec![1.0, 0.0, 0.0], None);
        // Strongest first: make this the fold target deterministically.
        survivor.fsrs.importance = 1.5;
        let candidate =
            dedup_entry("candidate", "the port is 5149 — stated again", vec![1.0, 0.0, 0.0], None);
        service.add_entry(survivor.clone()).await.expect("seed");
        service.add_entry(candidate.clone()).await.expect("seed");

        let mut result = ConsolidationResult::default();
        let survivors = service
            .fold_near_duplicates(vec![survivor, candidate], 10, &mut result)
            .await;

        assert_eq!(result.memories_deduped, 0, "the stale fold must not commit");
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(survivors.len(), 2, "both memories stay for the next cycle");
        assert_eq!(service.count().await, 2, "nothing may be removed");
        let live = service.get("survivor").await.expect("survivor exists");
        assert_eq!(
            live.content, LIVE,
            "the live rewrite must win over the merge of the stale snapshot"
        );
        assert!(
            service.get("candidate").await.is_some(),
            "the source must not be removed when its fold declined"
        );
    }

    /// A live rewrite of the SOURCE after its fold committed must survive the
    /// end-of-pass batch removal: the survivor absorbed the source's OLD text,
    /// so deleting the source would delete the fresh write that exists nowhere
    /// else. The matched removal keeps it as a transient near-duplicate.
    #[tokio::test]
    async fn a_source_rewritten_after_its_fold_survives_the_batch_removal() {
        const LIVE: &str = "the port is 5149 — stated again; ALSO the tls cert rotates monthly";
        let (service, slot) =
            hooked_dedup_service(("stated once", "stated again"), "candidate", LIVE);
        *slot.lock().await = Some(Arc::clone(&service));

        let mut survivor =
            dedup_entry("survivor", "the port is 5149 — stated once", vec![1.0, 0.0, 0.0], None);
        survivor.fsrs.importance = 1.5;
        let candidate =
            dedup_entry("candidate", "the port is 5149 — stated again", vec![1.0, 0.0, 0.0], None);
        service.add_entry(survivor.clone()).await.expect("seed");
        service.add_entry(candidate.clone()).await.expect("seed");

        let mut result = ConsolidationResult::default();
        let survivors = service
            .fold_near_duplicates(vec![survivor, candidate], 10, &mut result)
            .await;

        // The fold itself committed (the survivor was untouched)…
        assert_eq!(result.memories_deduped, 1);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(survivors.len(), 1);
        let merged = service.get("survivor").await.expect("survivor exists");
        assert!(
            merged.content.contains("stated again"),
            "the survivor absorbed the source's old text"
        );

        // …but the rewritten source is KEPT by the matched removal.
        assert_eq!(service.count().await, 2, "the rewritten source must survive");
        let kept = service.get("candidate").await.expect("source survives");
        assert_eq!(
            kept.content, LIVE,
            "the fresh write exists nowhere else and must not be deleted"
        );
    }

    /// THE incident test (2026-08-10): a dream dedup fold ran against the same
    /// scoped store a live harness step was writing through, and the step never
    /// returned. The contract under concurrency is forward progress — both
    /// sides complete inside a generous bound — and losslessness: nothing the
    /// step wrote disappears, however the folds interleave.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn fold_and_scoped_traffic_make_progress_concurrently() {
        let embed: EmbedFn = Arc::new(|_text: &str| {
            Box::pin(async {
                // Yield so the fold and the traffic genuinely interleave.
                tokio::task::yield_now().await;
                Ok(vec![1.0_f32, 0.0, 0.0])
            })
        });
        let service = Arc::new(
            MemoryService::new(MemoryServiceConfig {
                dimension: 3,
                ..Default::default()
            })
            .with_embed_fn(embed),
        );

        // Two families of near-duplicates in the SAME scope the traffic
        // writes to. The [1,0,0] family shares the traffic's direction, so
        // its folds and the step's writes contend for the same entries (the
        // CAS-decline path — under total contention every one of its folds
        // may legitimately go stale). The [0,1,0] family is out of the
        // traffic's reach and folds cleanly, so the cycle provably does real
        // dedup work no matter how the contended family's races land.
        for i in 0..30 {
            let embedding = if i % 2 == 0 {
                vec![1.0, 0.0, 0.0]
            } else {
                vec![0.0, 1.0, 0.0]
            };
            let entry = dedup_entry(
                &format!("seed-{i}"),
                &format!("the build passes — restatement {i}"),
                embedding,
                Some("ws1"),
            );
            service.add_entry(entry).await.expect("seed");
        }

        // Dedup-only cycle: an unreachable cluster size disables the
        // summarizing clusterer, so this is exactly the fold phase the
        // incident ran — and the summarizer must never be paid.
        let config = ConsolidationConfig {
            min_cluster_size: usize::MAX,
            min_remaining_memories: 5,
            ..ConsolidationConfig::default()
        };

        let fold = {
            let service = Arc::clone(&service);
            tokio::spawn(async move {
                service
                    .consolidate(&config, |_p| async { Ok::<String, String>(String::new()) })
                    .await
            })
        };
        let traffic = {
            let service = Arc::clone(&service);
            tokio::spawn(async move {
                for i in 0..25 {
                    service
                        .remember_scoped(
                            &format!("fresh observation {i}"),
                            HashMap::new(),
                            1.5,
                            Some("ws1".to_string()),
                        )
                        .await
                        .expect("a remember during a fold must land");
                    service
                        .recall_scoped(&format!("observation {i}"), Some("ws1"))
                        .await
                        .expect("a recall during a fold must answer");
                }
            })
        };

        // Forward progress: a deadlock between the fold and the step surfaces
        // here as a timeout, which is the regression this test exists to catch.
        let fold_result = tokio::time::timeout(std::time::Duration::from_secs(60), async {
            let fold_result = fold
                .await
                .expect("the fold task must not panic")
                .expect("consolidate must succeed");
            traffic.await.expect("the traffic task must not panic");
            fold_result
        })
        .await
        .expect("fold + live scoped traffic must make forward progress");

        assert!(
            fold_result.memories_deduped > 0,
            "the fold must have actually folded, or this proved nothing"
        );

        // Losslessness under contention: every observation the step wrote is
        // still readable — folded into a survivor or standing as its own row.
        let all = service.list_all().await;
        for i in 0..25 {
            let needle = format!("fresh observation {i}");
            assert!(
                all.iter().any(|m| m.content.contains(&needle)),
                "'{needle}' was written during the fold and must not be lost"
            );
        }
    }
}
