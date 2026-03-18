//! Multi-provider embedding router with automatic fallback and re-embedding
//!
//! Routes embedding requests to the primary provider, falling back to alternates
//! when the primary is unavailable. Tracks provider switches and signals when
//! re-embedding may be needed due to dimension changes.

use nanna_llm::EmbeddingClient;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Identifies an embedding provider
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EmbeddingProviderInfo {
    /// Provider name (e.g., "ollama", "openai")
    pub name: String,
    /// Model name (e.g., "nomic-embed-text", "text-embedding-3-small")
    pub model: String,
}

impl std::fmt::Display for EmbeddingProviderInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.name, self.model)
    }
}

/// An embedding provider entry in the router
struct EmbeddingProviderEntry {
    info: EmbeddingProviderInfo,
    client: Arc<EmbeddingClient>,
}

/// Multi-provider embedding router with automatic fallback.
///
/// Tries the primary provider first, then falls back through alternates
/// in order. Tracks which provider is currently active so consumers can
/// detect provider switches and trigger re-embedding if needed.
pub struct EmbeddingRouter {
    /// Ordered list: index 0 is primary, rest are fallbacks
    providers: Vec<EmbeddingProviderEntry>,
    /// Index of the currently active provider
    active_index: RwLock<usize>,
    /// Generation counter — incremented on every provider switch.
    /// Consumers compare their last-seen generation to detect changes.
    generation: AtomicU64,
}

impl EmbeddingRouter {
    /// Create a new router with a primary provider.
    pub fn new(info: EmbeddingProviderInfo, client: Arc<EmbeddingClient>) -> Self {
        Self {
            providers: vec![EmbeddingProviderEntry { info, client }],
            active_index: RwLock::new(0),
            generation: AtomicU64::new(0),
        }
    }

    /// Add a fallback provider. Order matters — tried in insertion order.
    #[must_use]
    pub fn with_fallback(mut self, info: EmbeddingProviderInfo, client: Arc<EmbeddingClient>) -> Self {
        self.providers.push(EmbeddingProviderEntry { info, client });
        self
    }

    /// Get the current generation counter.
    /// Consumers should store this and compare on subsequent calls.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// Get info about the currently active provider.
    pub async fn active_provider(&self) -> EmbeddingProviderInfo {
        let idx = *self.active_index.read().await;
        self.providers[idx].info.clone()
    }

    /// Embed a single text with automatic fallback.
    ///
    /// Tries the current active provider first. On failure, iterates through
    /// all providers starting from the next one. On successful fallback,
    /// updates the active provider and increments the generation counter.
    ///
    /// Returns `(embedding, provider_changed)` where `provider_changed` is
    /// true if a different provider was used than expected.
    pub async fn embed_one(&self, text: &str) -> Result<(Vec<f32>, bool), String> {
        let current_idx = *self.active_index.read().await;
        let total = self.providers.len();

        // Try current active provider first
        let entry = &self.providers[current_idx];
        match entry.client.embed_one(text).await {
            Ok(embedding) => return Ok((embedding, false)),
            Err(e) => {
                warn!(
                    "Embedding provider {} failed: {}. Trying fallbacks...",
                    entry.info, e
                );
            }
        }

        // Try remaining providers in order
        for offset in 1..total {
            let idx = (current_idx + offset) % total;
            let entry = &self.providers[idx];

            debug!("Trying fallback embedding provider: {}", entry.info);

            match entry.client.embed_one(text).await {
                Ok(embedding) => {
                    // Switch active provider
                    let old_info = self.providers[current_idx].info.clone();
                    {
                        let mut active = self.active_index.write().await;
                        *active = idx;
                    }
                    self.generation.fetch_add(1, Ordering::Relaxed);

                    info!(
                        "Embedding provider switched: {} → {} (generation {})",
                        old_info,
                        entry.info,
                        self.generation.load(Ordering::Relaxed)
                    );

                    return Ok((embedding, true));
                }
                Err(e) => {
                    warn!("Fallback embedding provider {} failed: {}", entry.info, e);
                }
            }
        }

        Err(format!(
            "All {} embedding providers failed",
            total
        ))
    }

    /// Try to restore the primary provider.
    ///
    /// Call this periodically (e.g., on a timer or every N embed calls) to
    /// check if the primary has recovered. Only probes if currently on a fallback.
    ///
    /// Returns `true` if primary was restored.
    pub async fn try_restore_primary(&self) -> bool {
        let current_idx = *self.active_index.read().await;
        if current_idx == 0 {
            return false; // Already on primary
        }

        let primary = &self.providers[0];
        match primary.client.embed_one("probe").await {
            Ok(_) => {
                let old_info = self.providers[current_idx].info.clone();
                {
                    let mut active = self.active_index.write().await;
                    *active = 0;
                }
                self.generation.fetch_add(1, Ordering::Relaxed);

                info!(
                    "Primary embedding provider restored: {} → {} (generation {})",
                    old_info,
                    primary.info,
                    self.generation.load(Ordering::Relaxed)
                );
                true
            }
            Err(_) => {
                debug!("Primary embedding provider {} still unavailable", primary.info);
                false
            }
        }
    }

    /// Number of configured providers (primary + fallbacks)
    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }
}
