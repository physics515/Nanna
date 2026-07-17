//! Memory tools for remembering and recalling information
//!
//! Uses MemoryService for persistent storage with embeddings and FSRS.

use crate::{Tool, ToolDefinition, ToolError, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

/// Adapter that wraps MemoryService to implement MemoryStorage trait.
/// This bridges the gap between the tool abstraction and the actual memory service.
pub struct MemoryServiceStorage {
    service: Arc<dyn MemoryServiceAdapter + Send + Sync>,
}

/// Trait to abstract over MemoryService (allows using it without direct dependency)
#[async_trait]
pub trait MemoryServiceAdapter: Send + Sync {
    async fn remember(&self, content: &str, metadata: HashMap<String, String>, importance: f32) -> Result<String, String>;
    async fn recall(&self, query: &str, limit: usize) -> Result<Vec<MemoryResult>, String>;
    async fn forget(&self, id: &str) -> Result<(), String>;
    async fn list(&self, limit: usize) -> Result<Vec<MemoryResult>, String>;
}

impl MemoryServiceStorage {
    pub fn new(service: Arc<dyn MemoryServiceAdapter + Send + Sync>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl MemoryStorage for MemoryServiceStorage {
    async fn store(&self, content: &str, tags: &[String]) -> Result<String, String> {
        let mut metadata = HashMap::new();
        if !tags.is_empty() {
            metadata.insert("tags".to_string(), tags.join(","));
        }
        // Default importance of 3.0 (medium)
        self.service.remember(content, metadata, 3.0).await
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryResult>, String> {
        self.service.recall(query, limit).await
    }

    async fn delete(&self, id: &str) -> Result<bool, String> {
        self.service.forget(id).await.map(|()| true)
    }

    async fn list(&self, limit: usize) -> Result<Vec<MemoryResult>, String> {
        self.service.list(limit).await
    }
}

/// Storage handle for memory tools
pub type StorageHandle = Arc<dyn MemoryStorage + Send + Sync>;

/// Leading text of `MemoryError::NoEmbeddingProvider`'s message (nanna-memory).
///
/// Errors cross the `MemoryStorage` trait boundary as plain strings, so the
/// tools cannot match on the variant — they match on this prefix instead. The
/// full message is pinned by tests in nanna-memory to never regress, so the
/// prefix is stable.
pub const NO_EMBEDDING_PROVIDER_MARKER: &str = "no embedding provider configured";

/// True when a memory-storage error string means "no embedding provider is
/// wired up" — a legible, user-fixable condition, not a tool failure.
#[must_use]
pub fn is_no_embedding_provider(err: &str) -> bool {
    err.starts_with(NO_EMBEDDING_PROVIDER_MARKER)
}

/// Trait for memory storage operations (allows mocking/different backends)
#[async_trait]
pub trait MemoryStorage: Send + Sync {
    async fn store(&self, content: &str, tags: &[String]) -> Result<String, String>;
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryResult>, String>;
    async fn delete(&self, id: &str) -> Result<bool, String>;
    async fn list(&self, limit: usize) -> Result<Vec<MemoryResult>, String>;
}

/// Memory search result
#[derive(Debug, Clone)]
pub struct MemoryResult {
    pub id: String,
    pub content: String,
    pub score: Option<f32>,
}

/// Simple in-memory storage (fallback when no Turso)
#[derive(Default)]
pub struct InMemoryStorage {
    memories: tokio::sync::RwLock<Vec<StoredMemory>>,
}

#[derive(Clone)]
struct StoredMemory {
    id: String,
    content: String,
    _tags: Vec<String>,
}

#[async_trait]
impl MemoryStorage for InMemoryStorage {
    async fn store(&self, content: &str, tags: &[String]) -> Result<String, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let mut memories = self.memories.write().await;
        memories.push(StoredMemory {
            id: id.clone(),
            content: content.to_string(),
            _tags: tags.to_vec(),
        });
        Ok(id)
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryResult>, String> {
        let query_lower = query.to_lowercase();
        let memories = self.memories.read().await;
        Ok(memories
            .iter()
            .filter(|m| m.content.to_lowercase().contains(&query_lower))
            .take(limit)
            .map(|m| MemoryResult {
                id: m.id.clone(),
                content: m.content.clone(),
                score: None,
            })
            .collect())
    }

    async fn delete(&self, id: &str) -> Result<bool, String> {
        let mut memories = self.memories.write().await;
        if let Some(pos) = memories.iter().position(|m| m.id == id) {
            memories.remove(pos);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn list(&self, limit: usize) -> Result<Vec<MemoryResult>, String> {
        let memories = self.memories.read().await;
        Ok(memories
            .iter()
            .rev()
            .take(limit)
            .map(|m| MemoryResult {
                id: m.id.clone(),
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
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

/// Tool to remember information
pub struct RememberTool {
    storage: StorageHandle,
}

impl RememberTool {
    pub fn new(storage: StorageHandle) -> Self {
        Self { storage }
    }

    #[must_use] 
    pub fn with_default_storage() -> Self {
        Self {
            storage: Arc::new(InMemoryStorage::default()),
        }
    }
}

#[async_trait]
impl Tool for RememberTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "remember",
            "Store information in memory for later recall. Use for important facts, preferences, or context you want to remember.",
        )
        .string_param("content", "The information to remember", true)
        .array_param("tags", "Optional tags for categorization", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("content is required".to_string()))?;

        let tags: Vec<String> = params
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(std::string::ToString::to_string))
                    .collect()
            })
            .unwrap_or_default();

        let id = self
            .storage
            .store(content, &tags)
            .await
            .map_err(ToolError::ExecutionFailed)?;

        info!("Remembered: {} (id: {})", truncate(content, 50), &id[..8]);

        Ok(ToolResult::success(format!(
            "Remembered (id: {}): {}",
            &id[..8],
            truncate(content, 100)
        )))
    }
}

/// Tool to recall information
pub struct RecallTool {
    storage: StorageHandle,
}

impl RecallTool {
    pub fn new(storage: StorageHandle) -> Self {
        Self { storage }
    }

    #[must_use] 
    pub fn with_default_storage() -> Self {
        Self {
            storage: Arc::new(InMemoryStorage::default()),
        }
    }
}

#[async_trait]
impl Tool for RecallTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "recall",
            "Search memory for previously stored information. Use to retrieve facts, preferences, or context.",
        )
        .string_param("query", "Search query to find relevant memories", true)
        .integer_param("limit", "Maximum number of results (default: 5)", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("query is required".to_string()))?;

        let limit = params
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(5) as usize;

        let results = match self.storage.search(query, limit).await {
            Ok(results) => results,
            Err(e) if is_no_embedding_provider(&e) => {
                // Memory is unconfigured, not failed: return a normal result so
                // the model reads it as "memory search unavailable" and moves on
                // instead of seeing a failed tool call it cannot act on.
                return Ok(ToolResult::success(format!(
                    "Memory search is unavailable: {e}"
                )));
            }
            Err(e) => return Err(ToolError::ExecutionFailed(e)),
        };

        if results.is_empty() {
            Ok(ToolResult::success("No memories found matching query."))
        } else {
            let output = results
                .iter()
                .map(|r| {
                    let score_str = r.score.map(|s| format!(" ({s:.2})")).unwrap_or_default();
                    format!("[{}{}] {}", &r.id[..8], score_str, r.content)
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(ToolResult::success(output))
        }
    }
}

/// Tool to reflect and write self-observations
pub struct ReflectTool {
    storage: StorageHandle,
}

impl ReflectTool {
    pub fn new(storage: StorageHandle) -> Self {
        Self { storage }
    }

    #[must_use] 
    pub fn with_default_storage() -> Self {
        Self {
            storage: Arc::new(InMemoryStorage::default()),
        }
    }
}

#[async_trait]
impl Tool for ReflectTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "reflect",
            "Write a self-reflection or observation. Use for meta-learning: noting patterns, mistakes, insights, or things to remember about yourself or your work.",
        )
        .string_param("observation", "Your reflection or observation", true)
        .enum_param("category", "Category of reflection", false, &["mistake", "insight", "pattern", "preference", "lesson", "other"])
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let observation = params
            .get("observation")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("observation is required".to_string()))?;

        let category = params
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("other");

        let content = format!("[REFLECTION:{}] {}", category.to_uppercase(), observation);
        let tags = vec!["reflection".to_string(), category.to_string()];

        let id = self
            .storage
            .store(&content, &tags)
            .await
            .map_err(ToolError::ExecutionFailed)?;

        info!("Reflection: {} (id: {})", truncate(observation, 50), &id[..8]);

        Ok(ToolResult::success(format!(
            "Reflection recorded ({}): {}",
            category,
            truncate(observation, 100)
        )))
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Storage that always fails the way nanna-memory does with no embedder wired:
    /// `MemoryError::NoEmbeddingProvider`'s display text, verbatim.
    struct UnconfiguredStorage;

    #[async_trait]
    impl MemoryStorage for UnconfiguredStorage {
        async fn store(&self, _content: &str, _tags: &[String]) -> Result<String, String> {
            Err(format!(
                "{NO_EMBEDDING_PROVIDER_MARKER} — set one in Settings, or run a local Ollama with an embedding model pulled, then memory search will work"
            ))
        }

        async fn search(&self, _query: &str, _limit: usize) -> Result<Vec<MemoryResult>, String> {
            Err(format!(
                "{NO_EMBEDDING_PROVIDER_MARKER} — set one in Settings, or run a local Ollama with an embedding model pulled, then memory search will work"
            ))
        }

        async fn delete(&self, _id: &str) -> Result<bool, String> {
            Err(NO_EMBEDDING_PROVIDER_MARKER.to_string())
        }

        async fn list(&self, _limit: usize) -> Result<Vec<MemoryResult>, String> {
            Ok(Vec::new())
        }
    }

    /// Storage whose search fails with a real, unrelated error.
    struct BrokenStorage;

    #[async_trait]
    impl MemoryStorage for BrokenStorage {
        async fn store(&self, _content: &str, _tags: &[String]) -> Result<String, String> {
            Err("disk on fire".to_string())
        }

        async fn search(&self, _query: &str, _limit: usize) -> Result<Vec<MemoryResult>, String> {
            Err("vector index corrupted".to_string())
        }

        async fn delete(&self, _id: &str) -> Result<bool, String> {
            Err("disk on fire".to_string())
        }

        async fn list(&self, _limit: usize) -> Result<Vec<MemoryResult>, String> {
            Ok(Vec::new())
        }
    }

    fn params(query: &str) -> HashMap<String, Value> {
        let mut m = HashMap::new();
        m.insert("query".to_string(), Value::String(query.to_string()));
        m
    }

    #[tokio::test]
    async fn recall_soft_degrades_when_no_embedding_provider() {
        let tool = RecallTool::new(Arc::new(UnconfiguredStorage));
        let result = tool
            .execute(params("anything"))
            .await
            .expect("unconfigured memory must not be a tool error");

        assert!(
            result.success,
            "soft-degraded recall must report success, got: {result:?}"
        );
        assert!(
            result.content.contains(NO_EMBEDDING_PROVIDER_MARKER),
            "soft-degraded recall must carry the actionable text, got: {}",
            result.content
        );
        assert!(
            result.content.contains("unavailable"),
            "the model should read this as memory being unavailable, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn recall_real_errors_still_fail() {
        let tool = RecallTool::new(Arc::new(BrokenStorage));
        let err = tool
            .execute(params("anything"))
            .await
            .expect_err("genuine storage failures must remain tool errors");

        assert!(
            matches!(err, ToolError::ExecutionFailed(_)),
            "expected ExecutionFailed, got: {err:?}"
        );
    }

    #[test]
    fn marker_matches_nanna_memory_message_prefix() {
        // nanna-memory pins the full NoEmbeddingProvider message with its own tests;
        // here we pin the prefix the tools actually match on.
        assert_eq!(NO_EMBEDDING_PROVIDER_MARKER, "no embedding provider configured");
        assert!(is_no_embedding_provider(
            "no embedding provider configured — set one in Settings, or run a local Ollama with an embedding model pulled, then memory search will work"
        ));
        assert!(!is_no_embedding_provider("IO error: No embedding function configured"));
        assert!(!is_no_embedding_provider("vector index corrupted"));
    }
}
