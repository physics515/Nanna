//! Adapter to bridge nanna-tools memory traits with nanna-memory MemoryService

use async_trait::async_trait;
use nanna_memory::MemoryService;
use nanna_tools::{MemoryResult, MemoryStorage};
use std::collections::HashMap;
use std::sync::Arc;

/// Adapter that implements MemoryStorage using the full MemoryService
pub struct MemoryServiceAdapter {
    service: Arc<MemoryService>,
}

impl MemoryServiceAdapter {
    pub fn new(service: Arc<MemoryService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl MemoryStorage for MemoryServiceAdapter {
    async fn store(&self, content: &str, tags: &[String]) -> Result<String, String> {
        let mut metadata = HashMap::new();
        if !tags.is_empty() {
            metadata.insert("tags".to_string(), tags.join(","));
        }
        
        // Use moderate importance (3.0) for explicit remember calls
        self.service
            .remember_with_importance(content, metadata, 3.0)
            .await
            .map(|(id, _)| id)
            .map_err(|e| e.to_string())
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryResult>, String> {
        self.service
            .recall(query)
            .await
            .map(|results| {
                results
                    .into_iter()
                    .take(limit)
                    .map(|r| MemoryResult {
                        id: r.id,
                        content: r.content,
                        score: Some(r.score),
                    })
                    .collect()
            })
            .map_err(|e| e.to_string())
    }

    async fn delete(&self, id: &str) -> Result<bool, String> {
        self.service
            .forget(id)
            .await
            .map(|_| true)
            .map_err(|e| e.to_string())
    }

    async fn list(&self, limit: usize) -> Result<Vec<MemoryResult>, String> {
        let all = self.service.list_all().await;
        Ok(all
            .into_iter()
            .take(limit)
            .map(|m| MemoryResult {
                id: m.id,
                content: m.content,
                score: Some(m.retrievability),
            })
            .collect())
    }
}
