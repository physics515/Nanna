//! Memory service - ties together embeddings, storage, and search

use crate::{MemoryEntry, MemoryError, VectorStore, VectorStoreConfig};
use std::collections::HashMap;
use std::sync::Arc;
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
}

impl Default for MemoryServiceConfig {
    fn default() -> Self {
        Self {
            dimension: 1536,
            min_score: 0.7,
            max_results: 10,
        }
    }
}

/// Callback for generating embeddings (injected dependency)
pub type EmbedFn = Arc<dyn Fn(&str) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<f32>, String>> + Send>> + Send + Sync>;

/// Memory service for semantic search and recall
pub struct MemoryService {
    config: MemoryServiceConfig,
    store: VectorStore,
    embed_fn: Option<EmbedFn>,
}

impl MemoryService {
    /// Create new memory service
    pub fn new(config: MemoryServiceConfig) -> Self {
        let store_config = VectorStoreConfig {
            dimension: config.dimension,
            use_f16: true,
        };
        Self {
            config,
            store: VectorStore::new(store_config),
            embed_fn: None,
        }
    }

    /// Set the embedding function
    pub fn with_embed_fn(mut self, f: EmbedFn) -> Self {
        self.embed_fn = Some(f);
        self
    }

    /// Remember something - store with embedding
    pub async fn remember(
        &self,
        content: &str,
        metadata: HashMap<String, String>,
    ) -> Result<String, MemoryError> {
        let embed_fn = self.embed_fn.as_ref().ok_or_else(|| {
            MemoryError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No embedding function configured",
            ))
        })?;

        // Generate embedding
        let embedding = (embed_fn)(content)
            .await
            .map_err(|e| MemoryError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        let id = uuid::Uuid::new_v4().to_string();
        let entry = MemoryEntry {
            id: id.clone(),
            content: content.to_string(),
            embedding,
            metadata,
            timestamp: chrono_timestamp(),
        };

        self.store.add(entry).await?;
        info!("Remembered: {} (id: {})", truncate(content, 50), id);
        Ok(id)
    }

    /// Recall memories similar to a query
    pub async fn recall(&self, query: &str) -> Result<Vec<RecallResult>, MemoryError> {
        let embed_fn = self.embed_fn.as_ref().ok_or_else(|| {
            MemoryError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No embedding function configured",
            ))
        })?;

        // Generate query embedding
        let query_embedding = (embed_fn)(query)
            .await
            .map_err(|e| MemoryError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        // Search
        let results = self.store.search(&query_embedding, self.config.max_results).await;

        // Filter by min score and convert
        let filtered: Vec<RecallResult> = results
            .into_iter()
            .filter(|(_, score)| *score >= self.config.min_score)
            .map(|(entry, score)| RecallResult {
                id: entry.id,
                content: entry.content,
                score,
                metadata: entry.metadata,
            })
            .collect();

        debug!("Recall '{}' found {} results", truncate(query, 30), filtered.len());
        Ok(filtered)
    }

    /// Forget a memory by ID
    pub async fn forget(&self, id: &str) -> Result<(), MemoryError> {
        self.store.remove(id).await?;
        info!("Forgot memory: {}", id);
        Ok(())
    }

    /// Get memory count
    pub async fn count(&self) -> usize {
        self.store.len().await
    }

    /// Clear all memories
    pub async fn clear(&self) {
        self.store.clear().await;
        info!("Cleared all memories");
    }

    /// Save memories to file
    pub async fn save(&self, path: &std::path::Path) -> Result<(), MemoryError> {
        self.store.save(path).await
    }

    /// Load memories from file
    pub async fn load(&self, path: &std::path::Path) -> Result<(), MemoryError> {
        self.store.load(path).await
    }
}

/// Result from memory recall
#[derive(Debug, Clone)]
pub struct RecallResult {
    pub id: String,
    pub content: String,
    pub score: f32,
    pub metadata: HashMap<String, String>,
}

fn chrono_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
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
}
