//! Memory tools for remembering and recalling information
//!
//! Uses Turso/SQLite for persistent storage.

use crate::{Tool, ToolDefinition, ToolError, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

/// Storage handle for memory tools
pub type StorageHandle = Arc<dyn MemoryStorage + Send + Sync>;

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
    tags: Vec<String>,
}

#[async_trait]
impl MemoryStorage for InMemoryStorage {
    async fn store(&self, content: &str, tags: &[String]) -> Result<String, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let mut memories = self.memories.write().await;
        memories.push(StoredMemory {
            id: id.clone(),
            content: content.to_string(),
            tags: tags.to_vec(),
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
        format!("{}...", &s[..max])
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
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let id = self
            .storage
            .store(content, &tags)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e))?;

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
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;

        let results = self
            .storage
            .search(query, limit)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e))?;

        if results.is_empty() {
            Ok(ToolResult::success("No memories found matching query."))
        } else {
            let output = results
                .iter()
                .map(|r| {
                    let score_str = r.score.map(|s| format!(" ({:.2})", s)).unwrap_or_default();
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
            .map_err(|e| ToolError::ExecutionFailed(e))?;

        info!("Reflection: {} (id: {})", truncate(observation, 50), &id[..8]);

        Ok(ToolResult::success(format!(
            "Reflection recorded ({}): {}",
            category,
            truncate(observation, 100)
        )))
    }
}
