#![warn(clippy::all, clippy::restriction)]
#![deny(clippy::pedantic, clippy::nursery)]

//! Core Nanna runtime
//!
//! Ties together all subsystems: LLM, channels, memory, SIMD, and GPU.

mod scheduler;

pub use scheduler::{
    Scheduler, SchedulerConfig, ScheduledTask, TaskType, TaskResult, TaskExecutor,
    heartbeat_task, recurring_task, delayed_task,
};

pub use nanna_channels::{
    Channel, ChannelCapabilities, ChannelError, ChannelId, IncomingMessage, MessageContent,
    MessageRouter, OutgoingMessage, Sender,
};
pub use nanna_gpu::{GpuContext, GpuError};
pub use nanna_llm::{CompletionRequest, LlmClient, LlmError, Message, Provider, RequestBuilder, Role};
pub use nanna_memory::{ConversationMemory, MemoryEntry, MemoryError, VectorStore, VectorStoreConfig};
pub use nanna_simd::{cosine_similarity_f32, dot_product_f32, normalize_f32};

use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{info, warn};

#[derive(Error, Debug)]
pub enum NannaError {
    #[error("LLM error: {0}")]
    Llm(#[from] LlmError),
    #[error("Channel error: {0}")]
    Channel(#[from] ChannelError),
    #[error("Memory error: {0}")]
    Memory(#[from] MemoryError),
    #[error("GPU error: {0}")]
    Gpu(#[from] GpuError),
    #[error("Configuration error: {0}")]
    Config(String),
}

/// Nanna configuration
#[derive(Debug, Clone)]
pub struct NannaConfig {
    pub name: String,
    pub default_model: String,
    pub max_context_messages: usize,
    pub enable_gpu: bool,
}

impl Default for NannaConfig {
    fn default() -> Self {
        Self {
            name: "Nanna".to_string(),
            default_model: "claude-sonnet-4-20250514".to_string(),
            max_context_messages: 20,
            enable_gpu: true,
        }
    }
}

/// Main Nanna instance
pub struct Nanna {
    config: NannaConfig,
    llm: Arc<LlmClient>,
    router: Arc<RwLock<MessageRouter>>,
    memory: Arc<VectorStore>,
    conversations: Arc<RwLock<std::collections::HashMap<String, ConversationMemory>>>,
    gpu: Option<Arc<GpuContext>>,
}

impl Nanna {
    /// Create a new Nanna instance
    pub async fn new(config: NannaConfig, llm: LlmClient) -> Result<Self, NannaError> {
        let gpu = if config.enable_gpu {
            match GpuContext::new().await {
                Ok(ctx) => {
                    info!("GPU initialized: {}", ctx.adapter_info.name);
                    Some(Arc::new(ctx))
                }
                Err(e) => {
                    warn!("GPU initialization failed, falling back to CPU: {}", e);
                    None
                }
            }
        } else {
            None
        };

        let memory_config = VectorStoreConfig::default();
        
        Ok(Self {
            config,
            llm: Arc::new(llm),
            router: Arc::new(RwLock::new(MessageRouter::new())),
            memory: Arc::new(VectorStore::new(memory_config)),
            conversations: Arc::new(RwLock::new(std::collections::HashMap::new())),
            gpu,
        })
    }

    /// Get or create a conversation memory for a session
    pub async fn get_conversation(&self, session_id: &str) -> ConversationMemory {
        let mut conversations = self.conversations.write().await;
        conversations
            .entry(session_id.to_string())
            .or_insert_with(|| {
                ConversationMemory::new(session_id, self.config.max_context_messages)
            })
            .clone()
    }

    /// Update conversation memory
    pub async fn update_conversation(&self, session_id: &str, memory: ConversationMemory) {
        let mut conversations = self.conversations.write().await;
        conversations.insert(session_id.to_string(), memory);
    }

    /// Process an incoming message and generate a response
    pub async fn process_message(
        &self,
        session_id: &str,
        message: &str,
    ) -> Result<String, NannaError> {
        // Get conversation context
        let mut conversation = self.get_conversation(session_id).await;
        conversation.add("user", message);

        // Build request
        let mut request = CompletionRequest::default()
            .with_model(&self.config.default_model);

        for msg in &conversation.messages {
            let role_msg = match msg.role.as_str() {
                "user" => Message::user(&msg.content),
                "assistant" => Message::assistant(&msg.content),
                "system" => Message::system(&msg.content),
                _ => continue,
            };
            request = request.with_message(role_msg);
        }

        // Call LLM
        let response = self.llm.complete(&request).await?;

        // Update conversation
        conversation.add("assistant", &response);
        self.update_conversation(session_id, conversation).await;

        Ok(response)
    }

    /// Register a channel
    pub async fn register_channel(&self, name: impl Into<String>, channel: Box<dyn Channel>) {
        let mut router = self.router.write().await;
        router.register(name, channel);
    }

    /// Send a message through the router
    pub async fn send_message(&self, message: OutgoingMessage) -> Result<String, NannaError> {
        let router = self.router.read().await;
        Ok(router.send(message).await?)
    }

    /// Add a memory entry
    pub async fn remember(&self, entry: MemoryEntry) -> Result<(), NannaError> {
        Ok(self.memory.add(entry).await?)
    }

    /// Search memories
    pub async fn recall(&self, query_embedding: &[f32], top_k: usize) -> Vec<(MemoryEntry, f32)> {
        self.memory.search(query_embedding, top_k).await
    }

    /// Check if GPU is available
    pub fn has_gpu(&self) -> bool {
        self.gpu.is_some()
    }

    /// Get GPU context if available
    pub fn gpu(&self) -> Option<&Arc<GpuContext>> {
        self.gpu.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_nanna_creation() {
        let config = NannaConfig {
            enable_gpu: false,  // Skip GPU for tests
            ..Default::default()
        };
        let llm = LlmClient::anthropic("test-key");
        let bot = Nanna::new(config, llm).await.unwrap();
        
        assert!(!bot.has_gpu());
    }

    #[tokio::test]
    async fn test_conversation_memory() {
        let config = NannaConfig {
            enable_gpu: false,
            ..Default::default()
        };
        let llm = LlmClient::anthropic("test-key");
        let bot = Nanna::new(config, llm).await.unwrap();

        let conv = bot.get_conversation("test-session").await;
        assert!(conv.is_empty());
    }
}
