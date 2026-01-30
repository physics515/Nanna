//! Dreaming Service - Periodic Memory Consolidation
//!
//! Implements the biological "dreaming" process where memories are:
//! - Compressed based on fading importance
//! - Expanded when highly valuable
//! - Clustered with similar memories
//! - Automatically promoted/demoted based on feedback

use crate::{
    ConsolidationConfig, ConsolidationResult, MemoryService, MemoryServiceConfig,
    MemoryError, EmbedFn,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Configuration for the dreaming service
#[derive(Debug, Clone)]
pub struct DreamingConfig {
    /// Memory service configuration
    pub memory: MemoryServiceConfig,
    /// Consolidation configuration
    pub consolidation: ConsolidationConfig,
    /// Auto-promote memories that are accessed frequently (threshold)
    pub auto_promote_access_threshold: u32,
    /// Auto-demote memories that are never accessed after N days
    pub auto_demote_days: f32,
    /// Minimum memories before consolidation runs
    pub min_memories_for_consolidation: usize,
}

impl Default for DreamingConfig {
    fn default() -> Self {
        Self {
            memory: MemoryServiceConfig::default(),
            consolidation: ConsolidationConfig::default(),
            auto_promote_access_threshold: 5,
            auto_demote_days: 30.0,
            min_memories_for_consolidation: 10,
        }
    }
}

/// Feedback type for automatic promotion/demotion
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryFeedback {
    /// User indicated this was helpful (👍, "thanks", positive reaction)
    Helpful,
    /// User indicated this was wrong/unhelpful (👎, "that's wrong", correction)
    Unhelpful,
    /// Memory was successfully used in a task
    UsedSuccessfully,
    /// Memory led to an error or bad outcome
    CausedError,
}

/// Statistics from a dreaming run
#[derive(Debug, Clone, Default)]
pub struct DreamingStats {
    /// Consolidation results
    pub consolidation: ConsolidationResult,
    /// Memories auto-promoted
    pub auto_promoted: usize,
    /// Memories auto-demoted
    pub auto_demoted: usize,
    /// Total memories after dreaming
    pub total_memories: usize,
}

/// The dreaming service - manages memory consolidation and auto-feedback
pub struct DreamingService {
    config: DreamingConfig,
    memory: MemoryService,
    /// Pending feedback to apply (memory_id -> accumulated feedback)
    pending_feedback: RwLock<HashMap<String, Vec<MemoryFeedback>>>,
}

impl DreamingService {
    /// Create a new dreaming service
    #[must_use]
    pub fn new(config: DreamingConfig) -> Self {
        let memory = MemoryService::new(config.memory.clone());
        Self {
            config,
            memory,
            pending_feedback: RwLock::new(HashMap::new()),
        }
    }

    /// Set the embedding function
    #[must_use]
    pub fn with_embed_fn(mut self, f: EmbedFn) -> Self {
        self.memory = self.memory.with_embed_fn(f);
        self
    }

    /// Get reference to the underlying memory service
    #[must_use]
    pub fn memory(&self) -> &MemoryService {
        &self.memory
    }

    /// Record feedback for a memory (will be applied during dreaming)
    pub async fn record_feedback(&self, memory_id: &str, feedback: MemoryFeedback) {
        let mut pending = self.pending_feedback.write().await;
        pending
            .entry(memory_id.to_string())
            .or_default()
            .push(feedback);
        debug!("Recorded {:?} feedback for memory {}", feedback, memory_id);
    }

    /// Apply pending feedback immediately (doesn't wait for dreaming)
    pub async fn apply_feedback(&self, memory_id: &str, feedback: MemoryFeedback) -> Result<(), MemoryError> {
        let boost = match feedback {
            MemoryFeedback::Helpful => 0.3,
            MemoryFeedback::UsedSuccessfully => 0.5,
            MemoryFeedback::Unhelpful => -0.3,
            MemoryFeedback::CausedError => -0.5,
        };

        if boost > 0.0 {
            self.memory.promote(memory_id, boost).await?;
        } else {
            self.memory.demote(memory_id, boost.abs()).await?;
        }

        info!("Applied {:?} feedback to memory {} (boost: {})", feedback, memory_id, boost);
        Ok(())
    }

    /// Run the dreaming process (memory consolidation)
    ///
    /// This should be called periodically (e.g., hourly or during idle times).
    ///
    /// # Arguments
    /// * `summarize_fn` - Async function that takes a prompt and returns summarized text (LLM call)
    ///
    /// # Errors
    ///
    /// Returns `MemoryError` if consolidation fails.
    pub async fn dream<F, Fut>(&self, summarize_fn: F) -> Result<DreamingStats, MemoryError>
    where
        F: Fn(String) -> Fut,
        Fut: std::future::Future<Output = Result<String, String>>,
    {
        let mut stats = DreamingStats::default();

        // 1. Apply pending feedback
        let pending = {
            let mut pending = self.pending_feedback.write().await;
            std::mem::take(&mut *pending)
        };

        for (memory_id, feedbacks) in pending {
            // Aggregate feedback
            let mut total_boost = 0.0_f32;
            for feedback in feedbacks {
                let boost = match feedback {
                    MemoryFeedback::Helpful => 0.3,
                    MemoryFeedback::UsedSuccessfully => 0.5,
                    MemoryFeedback::Unhelpful => -0.3,
                    MemoryFeedback::CausedError => -0.5,
                };
                total_boost += boost;
            }

            // Apply aggregated feedback
            if total_boost > 0.0 {
                if let Err(e) = self.memory.promote(&memory_id, total_boost.min(1.0)).await {
                    warn!("Failed to apply feedback to {}: {}", memory_id, e);
                } else {
                    stats.auto_promoted += 1;
                }
            } else if total_boost < 0.0 {
                if let Err(e) = self.memory.demote(&memory_id, total_boost.abs().min(1.0)).await {
                    warn!("Failed to apply feedback to {}: {}", memory_id, e);
                } else {
                    stats.auto_demoted += 1;
                }
            }
        }

        // 2. Apply FSRS updates (testing effect from recalls)
        self.memory.apply_pending_updates().await;

        // 3. Check if we have enough memories for consolidation
        let count = self.memory.count().await;
        if count < self.config.min_memories_for_consolidation {
            info!("Skipping consolidation: only {} memories (need {})", 
                count, self.config.min_memories_for_consolidation);
            stats.total_memories = count;
            return Ok(stats);
        }

        // 4. Run consolidation (the actual "dreaming")
        info!("Starting memory consolidation ({} memories)...", count);
        stats.consolidation = self.memory.consolidate(&self.config.consolidation, summarize_fn).await?;
        
        stats.total_memories = self.memory.count().await;
        
        info!(
            "Dreaming complete: {} processed, {} merged, {} expanded, {} promoted, {} demoted",
            stats.consolidation.memories_processed,
            stats.consolidation.memories_merged,
            stats.consolidation.memories_expanded,
            stats.auto_promoted,
            stats.auto_demoted,
        );

        Ok(stats)
    }

    /// Remember something (delegates to memory service)
    pub async fn remember(
        &self,
        content: &str,
        metadata: HashMap<String, String>,
    ) -> Result<String, MemoryError> {
        self.memory.remember(content, metadata).await
    }

    /// Recall memories (delegates to memory service)
    pub async fn recall(&self, query: &str) -> Result<Vec<crate::RecallResult>, MemoryError> {
        self.memory.recall(query).await
    }

    /// Forget a memory (delegates to memory service)
    pub async fn forget(&self, id: &str) -> Result<(), MemoryError> {
        self.memory.forget(id).await
    }

    /// Get memory statistics
    pub async fn stats(&self) -> crate::MemoryStats {
        self.memory.stats().await
    }

    /// Save memories to file
    pub async fn save(&self, path: &std::path::Path) -> Result<(), MemoryError> {
        self.memory.save(path).await
    }

    /// Load memories from file
    pub async fn load(&self, path: &std::path::Path) -> Result<(), MemoryError> {
        self.memory.load(path).await
    }
}

/// Create a summarization function from an LLM client
/// 
/// This is a helper to create the `summarize_fn` argument for `dream()`.
pub fn make_summarize_fn<C>(
    llm: Arc<C>,
    model: String,
) -> impl Fn(String) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
where
    C: LlmSummarizer + Send + Sync + 'static,
{
    move |prompt: String| {
        let llm = llm.clone();
        let model = model.clone();
        Box::pin(async move {
            llm.summarize(&model, &prompt).await
        })
    }
}

/// Trait for LLM summarization (implemented by LlmClient)
/// 
/// This is a simple trait that any LLM client can implement for memory consolidation.
pub trait LlmSummarizer: Send + Sync {
    /// Summarize/consolidate the given prompt into a condensed memory.
    fn summarize(
        &self,
        model: &str,
        prompt: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + '_>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dreaming_service_creation() {
        let service = DreamingService::new(DreamingConfig::default());
        let stats = service.stats().await;
        assert_eq!(stats.total, 0);
    }

    #[tokio::test]
    async fn test_feedback_recording() {
        let service = DreamingService::new(DreamingConfig::default());
        
        service.record_feedback("mem-1", MemoryFeedback::Helpful).await;
        service.record_feedback("mem-1", MemoryFeedback::UsedSuccessfully).await;
        service.record_feedback("mem-2", MemoryFeedback::Unhelpful).await;

        let pending = service.pending_feedback.read().await;
        assert_eq!(pending.get("mem-1").map(|v| v.len()), Some(2));
        assert_eq!(pending.get("mem-2").map(|v| v.len()), Some(1));
    }
}
