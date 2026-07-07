//! Memory service - ties together embeddings, storage, and search
//!
//! Integrates FSRS-6 cognitive memory model with vector storage.

use crate::{
    MemoryEntry, MemoryError, VectorStore, VectorStoreConfig,
    FsrsParameters, FsrsState, MemoryState, Rating, IngestAction,
    ConsolidationConfig, ConsolidationResult, CompressionLevel,
    MemoryCluster, cluster_memories, create_consolidated_entry,
};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
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

impl MemoryServiceConfig {
    /// Create config with dimension inferred from embedding model name
    #[must_use]
    pub fn for_model(model: &str) -> Self {
        Self {
            dimension: Self::dimension_for_model(model),
            ..Default::default()
        }
    }

    /// Get the embedding dimension for a given model name
    /// Covers common embedding models from OpenAI, Ollama, and other providers
    #[must_use]
    pub fn dimension_for_model(model: &str) -> usize {
        let model_lower = model.to_lowercase();

        // OpenAI models
        if model_lower.contains("text-embedding-3-large") {
            return 3072;
        }
        if model_lower.contains("text-embedding-3-small") || model_lower.contains("ada-002") {
            return 1536;
        }

        // BGE models (BAAI)
        if model_lower.contains("bge-large") {
            return 1024;
        }
        if model_lower.contains("bge-m3") {
            return 1024;
        }
        if model_lower.contains("bge-small") {
            return 384;
        }
        if model_lower.contains("bge-base") {
            return 768;
        }

        // MxBai models
        if model_lower.contains("mxbai") {
            return 1024;
        }

        // MiniLM models
        if model_lower.contains("minilm") {
            return 384;
        }

        // Nomic models
        if model_lower.contains("nomic-embed") {
            return 768;
        }

        // Jina models
        if model_lower.contains("jina") {
            return 768;
        }

        // Default to 768 (common for many models)
        768
    }
}

/// Callback for generating embeddings (injected dependency)
pub type EmbedFn = Arc<dyn Fn(&str) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<f32>, String>> + Send>> + Send + Sync>;

/// Memory service for semantic search and recall with FSRS-6 cognitive model
pub struct MemoryService {
    config: MemoryServiceConfig,
    store: VectorStore,
    embed_fn: Option<EmbedFn>,
    /// Track memory IDs that need FSRS updates (for async batch updates)
    pending_updates: RwLock<Vec<(String, Rating)>>,
    /// Runtime-adjustable minimum score (overrides config)
    min_score_override: RwLock<Option<f32>>,
    /// Runtime dimension — updated atomically when probe detects a change.
    /// This allows `probe_and_align_dimension` to work on `&self` (behind Arc).
    runtime_dimension: AtomicUsize,
}

impl MemoryService {
    /// Create new memory service
    #[must_use] 
    pub fn new(config: MemoryServiceConfig) -> Self {
        let store_config = VectorStoreConfig {
            dimension: std::sync::atomic::AtomicUsize::new(config.dimension),
            use_f16: true,
        };
        let dim = config.dimension;
        Self {
            config,
            store: VectorStore::new(store_config),
            embed_fn: None,
            pending_updates: RwLock::new(Vec::new()),
            min_score_override: RwLock::new(None),
            runtime_dimension: AtomicUsize::new(dim),
        }
    }

    /// Set the embedding function.
    #[must_use]
    pub fn with_embed_fn(mut self, f: EmbedFn) -> Self {
        self.embed_fn = Some(f);
        self
    }

    /// Attach a SQLite (or other) persistence backend to the underlying store.
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
        let embed_fn = self.embed_fn.as_ref().ok_or_else(|| {
            MemoryError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No embedding function configured",
            ))
        })?;

        // Generate a test embedding with a short probe string
        let test_embedding = (embed_fn)("dimension probe")
            .await
            .map_err(|e| MemoryError::Io(std::io::Error::other(e)))?;

        let actual_dim = test_embedding.len();
        let expected_dim = self.runtime_dimension.load(Ordering::Relaxed);
        if actual_dim == expected_dim {
            info!("Embedding dimension confirmed: {}", actual_dim);
            return Ok(actual_dim);
        }

        warn!(
            "Embedding model returns {} dims, expected {}. Reconfiguring and re-embedding.",
            actual_dim, expected_dim
        );
        self.runtime_dimension.store(actual_dim, Ordering::Relaxed);
        // Also update the VectorStore's dimension so add() accepts the new size
        self.store.set_dimension(actual_dim);

        // Re-embed all entries that have the wrong dimension.
        // Clone the embed_fn Arc so we can pass it into the closure.
        let embed_fn_clone = Arc::clone(embed_fn);
        let re_embedded = self.store.re_embed_mismatched(actual_dim, |text| {
            let ef = Arc::clone(&embed_fn_clone);
            async move { (ef)(&text).await }
        }).await;

        if re_embedded > 0 {
            info!("Re-embedded {} entries to {} dimensions", re_embedded, actual_dim);
        }

        Ok(actual_dim)
    }

    /// Get the current embedding dimension (may differ from initial config
    /// if the embedding model changed at runtime).
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.runtime_dimension.load(Ordering::Relaxed)
    }

    /// Get FSRS parameters
    #[must_use]
    pub fn fsrs_params(&self) -> &FsrsParameters {
        &self.config.fsrs
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
        let embed_fn = self.embed_fn.as_ref().ok_or_else(|| {
            MemoryError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No embedding function configured",
            ))
        })?;

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
                    if folded {
                        info!(
                            "Merged into: {} (sim: {:.3})",
                            truncate(&existing.content, 30),
                            similarity
                        );
                    } else {
                        info!(
                            "Update no-op (subset): {} (sim: {:.3})",
                            truncate(&existing.content, 30),
                            similarity
                        );
                    }
                    // A related mention is a successful recall — reinforce FSRS.
                    self.pending_updates
                        .write()
                        .await
                        .push((existing.id.clone(), Rating::Good));
                    return Ok((existing.id.clone(), IngestAction::Update));
                }
                IngestAction::Create => {
                    // Novel content, create new
                }
            }
        }

        // Create new memory
        let id = uuid::Uuid::new_v4().to_string();
        let entry = MemoryEntry {
            id: id.clone(),
            content: content.to_string(),
            embedding,
            metadata,
            timestamp: chrono_timestamp(),
            fsrs: FsrsState::new(),
            workspace_id: None, // smart_ingest creates global memories
            expires_at: None,
        };

        self.store.add(entry).await?;
        info!("Remembered: {} (id: {})", truncate(content, 50), id);
        Ok((id, IngestAction::Create))
    }

    /// Fold `incoming` content into an existing memory (`existing_id`), re-embedding
    /// the merged text. Shared by every ingest path that lands in the `Update` band.
    ///
    /// Returns `true` if a merge was written, `false` if `incoming` added nothing new
    /// (a subset of the existing content). The caller is responsible for FSRS
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
    ) -> Result<bool, MemoryError> {
        debug_assert!(
            !existing_id.is_empty(),
            "existing memory id must not be empty"
        );
        let merged = merge_memory_content(existing_content, incoming);
        if merged == existing_content {
            return Ok(false);
        }
        let merged_embedding = (embed_fn)(&merged)
            .await
            .map_err(|e| MemoryError::Io(std::io::Error::other(e)))?;
        self.store
            .update_content_and_embedding(existing_id, &merged, merged_embedding)
            .await?;
        Ok(true)
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
        let embed_fn = self.embed_fn.as_ref().ok_or_else(|| {
            MemoryError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No embedding function configured",
            ))
        })?;

        // Truncate oversized content to avoid embedding model context overflow
        // Most embedding models have 8192 token context; ~4 chars/token ≈ 30k chars safe limit
        let content = if content.len() > 30_000 {
            warn!("Truncating oversized memory content ({} chars) to 30000", content.len());
            &content[..30_000]
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
            
            // Don't reinforce expired or error memories — let them die
            let skip_reinforce = existing.is_expired()
                || existing.content.contains("Error:")
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
                    if folded {
                        info!(
                            "Merged (importance): {} (sim: {:.3})",
                            truncate(&existing.content, 30),
                            similarity
                        );
                    } else {
                        info!(
                            "Related memory exists: {} (sim: {:.3})",
                            truncate(&existing.content, 30),
                            similarity
                        );
                    }
                    return Ok((existing.id.clone(), action));
                }
                _ => {
                    // Novel content or skipped reinforcement — fall through to create
                    if skip_reinforce {
                        info!("Skipping reinforcement of expired/error memory: {} (sim: {:.3})", truncate(&existing.content, 30), similarity);
                    }
                }
            }
        }

        // Create new memory with importance
        let id = uuid::Uuid::new_v4().to_string();
        let mut fsrs = FsrsState::new();
        // Normalize importance from 1-5 scale to 0.5-1.5 multiplier
        fsrs.importance = (importance / 5.0).clamp(0.5, 1.5);
        
        let entry = MemoryEntry {
            id: id.clone(),
            content: content.to_string(),
            embedding,
            metadata: metadata.clone(),
            timestamp: chrono_timestamp(),
            fsrs,
            workspace_id: None, // Global memory
            expires_at: metadata.get("expires_at").and_then(|s| s.parse::<i64>().ok()),
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
        let embed_fn = self.embed_fn.as_ref().ok_or_else(|| {
            MemoryError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No embedding function configured",
            ))
        })?;

        // Generate embedding
        let embedding = (embed_fn)(content)
            .await
            .map_err(|e| MemoryError::Io(std::io::Error::other(e)))?;

        // Check for similar existing memories (within same scope)
        let results = self.store.search_scoped(&embedding, 1, workspace_id.as_deref()).await;
        
        if let Some((existing, similarity)) = results.first() {
            let action = IngestAction::from_similarity(*similarity);
            
            match action {
                IngestAction::Reinforce | IngestAction::Update => {
                    // Don't reinforce expired or error memories
                    if existing.is_expired() {
                        info!("Skipping reinforcement of expired memory: {} (sim: {:.3})", truncate(&existing.content, 30), similarity);
                    } else if existing.content.contains("Error:") || existing.content.contains("Command failed") {
                        info!("Skipping reinforcement of error memory: {} (sim: {:.3})", truncate(&existing.content, 30), similarity);
                    } else {
                        // On the Update band, fold new information into the existing
                        // (scoped) memory before reinforcing — dedup, not accrete.
                        let folded = if action == IngestAction::Update {
                            self.fold_into_memory(embed_fn, &existing.id, &existing.content, content)
                                .await?
                        } else {
                            false
                        };
                        self.pending_updates.write().await.push((existing.id.clone(), Rating::Good));
                        if folded {
                            info!(
                                "Merged (scoped): {} (sim: {:.3})",
                                truncate(&existing.content, 30),
                                similarity
                            );
                        } else {
                            info!(
                                "Reinforced (scoped): {} (sim: {:.3})",
                                truncate(&existing.content, 30),
                                similarity
                            );
                        }
                        return Ok((existing.id.clone(), action));
                    }
                }
                IngestAction::Create => {}
            }
        }

        // Create new memory with workspace scope
        let id = uuid::Uuid::new_v4().to_string();
        let mut fsrs = FsrsState::new();
        fsrs.importance = (importance / 5.0).clamp(0.5, 1.5);
        
        // Extract optional expiration from metadata
        let expires_at = metadata.get("expires_at")
            .and_then(|s| s.parse::<i64>().ok());

        let entry = MemoryEntry {
            id: id.clone(),
            content: content.to_string(),
            embedding,
            metadata,
            timestamp: chrono_timestamp(),
            fsrs,
            workspace_id,
            expires_at,
        };

        self.store.add(entry).await?;
        let ttl_note = expires_at.map_or(String::new(), |t| {
            let secs = t - chrono_timestamp();
            format!(" (expires in {}m)", secs / 60)
        });
        info!("Remembered (scoped, importance {}{}): {} (id: {})", importance, ttl_note, truncate(content, 50), id);
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
        let embed_fn = self.embed_fn.as_ref().ok_or_else(|| {
            MemoryError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No embedding function configured",
            ))
        })?;

        let store_count = self.store.len().await;
        info!("Recall: generating embedding for query (store has {} entries)", store_count);

        // Generate query embedding
        let query_embedding = (embed_fn)(query)
            .await
            .map_err(|e| {
                warn!("Recall: embedding generation failed: {}", e);
                MemoryError::Io(std::io::Error::other(e))
            })?;
        
        info!("Recall: embedding generated ({} dims), searching...", query_embedding.len());

        // Search
        let results = self.store.search(&query_embedding, self.config.max_results * 2).await;
        let min_score = self.get_min_score();
        info!("Recall: raw search returned {} results (min_score: {:.2}, min_weight: {:.2})", 
               results.len(), min_score, self.config.min_weight);
        
        // Log top results before filtering
        for (i, (entry, score)) in results.iter().take(3).enumerate() {
            info!("  [{}] score={:.3}: {}", i, score, truncate(&entry.content, 50));
        }

        // Filter by min score and weight, apply testing effect
        let mut filtered = Vec::new();
        let mut updates = Vec::new();
        
        for (entry, score) in results {
            if score < min_score {
                continue;
            }
            
            let weight = entry.fsrs.weight(&self.config.fsrs);
            let state = entry.fsrs.state(&self.config.fsrs);
            
            // Skip effectively forgotten memories
            if weight < self.config.min_weight {
                debug!("Skipping forgotten memory: {} (weight: {:.3})", entry.id, weight);
                continue;
            }
            
            // Queue testing effect update
            updates.push((entry.id.clone(), Rating::Good));
            
            filtered.push(RecallResult {
                id: entry.id,
                content: entry.content,
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

        // Queue FSRS updates (testing effect)
        if !updates.is_empty() {
            self.pending_updates.write().await.extend(updates);
        }

        debug!("Recall '{}' found {} results", truncate(query, 30), filtered.len());
        Ok(filtered)
    }

    /// Recall memories with workspace scope filtering.
    ///
    /// Scope rules:
    /// - `workspace_id = Some(id)`: returns global + that workspace's memories
    /// - `workspace_id = None` (global): returns all memories
    ///
    /// # Errors
    ///
    /// Returns `MemoryError` if no embedding function is configured.
    pub async fn recall_scoped(
        &self,
        query: &str,
        workspace_id: Option<&str>,
    ) -> Result<Vec<RecallResult>, MemoryError> {
        let embed_fn = self.embed_fn.as_ref().ok_or_else(|| {
            MemoryError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No embedding function configured",
            ))
        })?;

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
        
        // Filter by min score and weight, apply testing effect
        let mut filtered = Vec::new();
        let mut updates = Vec::new();
        
        for (entry, score) in results {
            if score < min_score {
                continue;
            }
            
            let weight = entry.fsrs.weight(&self.config.fsrs);
            let state = entry.fsrs.state(&self.config.fsrs);
            
            if weight < self.config.min_weight {
                continue;
            }
            
            updates.push((entry.id.clone(), Rating::Good));
            
            filtered.push(RecallResult {
                id: entry.id,
                content: entry.content,
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

    /// Purge expired memories (call during dream time)
    pub async fn purge_expired(&self) -> usize {
        self.store.purge_expired().await
    }

    /// Get memory count
    pub async fn count(&self) -> usize {
        self.store.len().await
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

    /// Flush all in-memory entries to the persistence backend (for JSON → SQLite migration).
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
        F: Fn(String) -> Fut,
        Fut: Future<Output = Result<String, String>>,
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

            // Skip detailed level (no compression needed)
            if compression_level == CompressionLevel::Detailed {
                result.memories_processed += memories.len();
                continue;
            }

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

        info!(
            "Consolidation complete: {} processed, {} clusters, {} merged, {} expanded, {} errors",
            result.memories_processed,
            result.clusters_formed,
            result.memories_merged,
            result.memories_expanded,
            result.errors.len()
        );

        Ok(result)
    }

    /// Consolidate a single cluster of memories
    async fn consolidate_cluster<F, Fut>(
        &self,
        cluster: &MemoryCluster,
        summarize_fn: &F,
    ) -> Result<(), MemoryError>
    where
        F: Fn(String) -> Fut,
        Fut: Future<Output = Result<String, String>>,
    {
        // Build prompt and get summary
        let prompt = cluster.build_consolidation_prompt();
        let summary = summarize_fn(prompt)
            .await
            .map_err(|e| MemoryError::Io(std::io::Error::other(e)))?;

        // Generate embedding for the summary
        let embed_fn = self.embed_fn.as_ref().ok_or_else(|| {
            MemoryError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No embedding function configured",
            ))
        })?;

        let embedding = (embed_fn)(&summary)
            .await
            .map_err(|e| MemoryError::Io(std::io::Error::other(e)))?;

        // Create consolidated entry
        let consolidated = create_consolidated_entry(cluster, summary, embedding);

        // Remove old memories
        for memory in &cluster.memories {
            if let Err(e) = self.store.remove(&memory.id).await {
                warn!("Failed to remove old memory {}: {}", memory.id, e);
            }
        }

        // Add consolidated memory
        self.store.add(consolidated).await?;

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
        F: Fn(String) -> Fut,
        Fut: Future<Output = Result<String, String>>,
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
            self.store.update_content(&memory.id, &expanded).await?;
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
    pub content: String,
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

/// Maximum byte length of merged memory content — bounds accretion across repeated merges.
const MEMORY_MERGE_MAX_BYTES: usize = 4096;

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

    // Bounded append.
    let merged_len_bytes = existing.len() + SEPARATOR.len() + incoming.len();
    if merged_len_bytes > MEMORY_MERGE_MAX_BYTES {
        return existing.to_string();
    }
    format!("{existing}{SEPARATOR}{incoming}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_memory_service_no_embed() {
        let service = MemoryService::new(MemoryServiceConfig::default());

        // Should fail without embed function
        let result = service.remember("test", HashMap::new()).await;
        assert!(result.is_err());
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

    #[test]
    fn test_merge_memory_content_bounded() {
        // Appending past the byte cap keeps the existing content (no unbounded growth).
        let existing = "a".repeat(MEMORY_MERGE_MAX_BYTES - 1);
        let incoming = "b".repeat(MEMORY_MERGE_MAX_BYTES);
        let merged = merge_memory_content(&existing, &incoming);
        assert_eq!(merged, existing);
        assert!(merged.len() <= MEMORY_MERGE_MAX_BYTES);
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
}
