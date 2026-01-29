//! Turso storage adapter for memory tools
//!
//! Uses SIMD-accelerated vector operations for semantic search.

use super::memory::{MemoryResult, MemoryStorage};
use async_trait::async_trait;
use nanna_simd::cosine_similarity_f32;
use nanna_storage::{NewMemory, Storage};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Function type for generating embeddings
pub type EmbedFn = Arc<
    dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<Vec<f32>, String>> + Send>>
        + Send
        + Sync,
>;

/// Turso-backed memory storage with optional embeddings
pub struct TursoMemoryStorage {
    storage: Arc<Storage>,
    embed_fn: Option<EmbedFn>,
    embedding_model: String,
}

impl TursoMemoryStorage {
    #[must_use] 
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            storage,
            embed_fn: None,
            embedding_model: "none".to_string(),
        }
    }

    /// Enable semantic search with an embedding function
    pub fn with_embeddings(mut self, embed_fn: EmbedFn, model: &str) -> Self {
        self.embed_fn = Some(embed_fn);
        self.embedding_model = model.to_string();
        self
    }
}

#[async_trait]
impl MemoryStorage for TursoMemoryStorage {
    async fn store(&self, content: &str, tags: &[String]) -> Result<String, String> {
        let memory_id = uuid::Uuid::new_v4().to_string();

        // Generate embedding if available
        let embedding = if let Some(ref embed_fn) = self.embed_fn {
            match embed_fn(content.to_string()).await {
                Ok(emb) => Some(emb),
                Err(e) => {
                    tracing::warn!("Failed to generate embedding: {}", e);
                    None
                }
            }
        } else {
            None
        };

        let new_memory = NewMemory {
            memory_id: memory_id.clone(),
            content: content.to_string(),
            embedding,
            embedding_model: Some(self.embedding_model.clone()),
            session_id: None,
            metadata: None,
            tags: tags.to_vec(),
        };

        self.storage
            .memories()
            .create(new_memory)
            .await
            .map_err(|e| e.to_string())?;

        Ok(memory_id)
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryResult>, String> {
        // Get all memories
        let memories = self
            .storage
            .memories()
            .list_all(500) // Get more for vector search
            .await
            .map_err(|e| e.to_string())?;

        // If we have embeddings, do vector similarity search
        if let Some(ref embed_fn) = self.embed_fn {
            let query_embedding = embed_fn(query.to_string())
                .await
                .map_err(|e| format!("Failed to embed query: {e}"))?;

            // Score each memory by SIMD-accelerated cosine similarity
            let mut scored: Vec<(MemoryResult, f32)> = memories
                .iter()
                .filter_map(|m| {
                    m.embedding.as_ref().and_then(|emb| {
                        // Skip if dimensions don't match
                        if emb.len() != query_embedding.len() {
                            return None;
                        }
                        let score = cosine_similarity_f32(&query_embedding, emb);
                        Some((
                            MemoryResult {
                                id: m.memory_id.clone(),
                                content: m.content.clone(),
                                score: Some(score),
                            },
                            score,
                        ))
                    })
                })
                .collect();

            // Sort by score descending
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

            // Filter by minimum score and take top results
            let results: Vec<MemoryResult> = scored
                .into_iter()
                .filter(|(_, score)| *score > 0.5) // Minimum similarity threshold
                .take(limit)
                .map(|(r, _)| r)
                .collect();

            if !results.is_empty() {
                return Ok(results);
            }
            // Fall through to text search if no vector results
        }

        // Fallback: text search
        let query_lower = query.to_lowercase();
        let results: Vec<MemoryResult> = memories
            .into_iter()
            .filter(|m| m.content.to_lowercase().contains(&query_lower))
            .take(limit)
            .map(|m| MemoryResult {
                id: m.memory_id,
                content: m.content,
                score: None,
            })
            .collect();

        Ok(results)
    }

    async fn delete(&self, id: &str) -> Result<bool, String> {
        self.storage
            .memories()
            .delete(id)
            .await
            .map_err(|e| e.to_string())
    }

    async fn list(&self, limit: usize) -> Result<Vec<MemoryResult>, String> {
        let memories = self
            .storage
            .memories()
            .list_all(limit as i64)
            .await
            .map_err(|e| e.to_string())?;

        Ok(memories
            .into_iter()
            .map(|m| MemoryResult {
                id: m.memory_id,
                content: truncate(&m.content, 100),
                score: None,
            })
            .collect())
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}
