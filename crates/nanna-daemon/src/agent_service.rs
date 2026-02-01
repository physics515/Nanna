//! Agent Service - Wraps nanna-agent for the daemon
//!
//! Handles LLM calls, streaming, and tool execution.

use crate::protocol::Event;
use crate::session::{MessageRole, SessionId, ToolCallRecord};
use nanna_agent::{Agent, AgentConfig, AgentResponse, RunOptions, ThinkingMode};
use nanna_llm::LlmClient;
use nanna_memory::MemoryService;
use nanna_tools::ToolRegistry;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, error, info, warn};

/// Configuration for the agent service
#[derive(Debug, Clone)]
pub struct AgentServiceConfig {
    /// Default model to use
    pub model: String,
    /// Maximum tokens per response
    pub max_tokens: u32,
    /// Temperature for sampling
    pub temperature: f32,
    /// Maximum tool iterations per request
    pub max_iterations: usize,
    /// Default thinking mode
    pub thinking_mode: ThinkingMode,
}

impl Default for AgentServiceConfig {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-20250514".to_string(),
            max_tokens: 8192,
            temperature: 0.7,
            max_iterations: 10,
            thinking_mode: ThinkingMode::Instant,
        }
    }
}

/// Active chat request state
struct ActiveChat {
    session_id: SessionId,
    cancelled: bool,
}

/// The agent service that handles LLM interactions
pub struct AgentService {
    config: AgentServiceConfig,
    llm: Arc<LlmClient>,
    tools: Arc<ToolRegistry>,
    memory: Option<Arc<MemoryService>>,
    /// Event broadcaster for streaming to clients
    event_tx: broadcast::Sender<Event>,
    /// Currently active chats (session_id -> state)
    active_chats: Arc<RwLock<HashMap<SessionId, ActiveChat>>>,
}

impl AgentService {
    /// Create a new agent service
    pub fn new(
        config: AgentServiceConfig,
        llm: Arc<LlmClient>,
        tools: Arc<ToolRegistry>,
        memory: Option<Arc<MemoryService>>,
        event_tx: broadcast::Sender<Event>,
    ) -> Self {
        Self {
            config,
            llm,
            tools,
            memory,
            event_tx,
            active_chats: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    /// Create agent config from service config
    fn agent_config(&self) -> AgentConfig {
        AgentConfig {
            model: self.config.model.clone(),
            max_tokens: self.config.max_tokens,
            temperature: self.config.temperature,
            max_iterations: self.config.max_iterations,
            thinking_mode: self.config.thinking_mode,
        }
    }
    
    /// Run a chat completion
    pub async fn chat(
        &self,
        session_id: &str,
        message: &str,
        system_prompt: Option<String>,
    ) -> Result<ChatResult, String> {
        // Mark chat as active
        {
            let mut active = self.active_chats.write().await;
            active.insert(session_id.to_string(), ActiveChat {
                session_id: session_id.to_string(),
                cancelled: false,
            });
        }
        
        // Create agent with custom context if system prompt provided
        let agent = if let Some(ref prompt) = system_prompt {
            let context = nanna_agent::AgentContext::new(session_id.to_string())
                .with_system_prompt(prompt);
            Agent::new(
                self.agent_config(),
                self.llm.clone(),
                self.tools.clone(),
            ).with_context(context)
        } else {
            Agent::new(
                self.agent_config(),
                self.llm.clone(),
                self.tools.clone(),
            )
        };
        
        // Create run options with streaming callbacks
        let session_id_for_stream = session_id.to_string();
        let event_tx = self.event_tx.clone();
        
        let options = RunOptions {
            on_text: Some(Box::new(move |chunk: &str| {
                let _ = event_tx.send(Event::MessageDelta {
                    session_id: session_id_for_stream.clone(),
                    message_id: String::new(),
                    delta: chunk.to_string(),
                });
            })),
            ..Default::default()
        };
        
        // Run the agent
        let message_id = uuid::Uuid::new_v4().to_string();
        
        // Emit start event
        let _ = self.event_tx.send(Event::MessageStart {
            session_id: session_id.to_string(),
            message_id: message_id.clone(),
        });
        
        let result = agent.run(message, options).await;
        
        // Remove from active chats
        {
            let mut active = self.active_chats.write().await;
            active.remove(session_id);
        }
        
        match result {
            Ok(response) => {
                // Emit tool events
                for tc in &response.tool_calls {
                    let _ = self.event_tx.send(Event::ToolEnd {
                        session_id: session_id.to_string(),
                        call_id: tc.id.clone(),
                        output: tc.output.clone(),
                        success: tc.success,
                        duration_ms: tc.duration_ms,
                    });
                }
                
                // Emit completion event
                let _ = self.event_tx.send(Event::MessageEnd {
                    session_id: session_id.to_string(),
                    message_id: message_id.clone(),
                    content: response.text.clone(),
                });
                
                Ok(ChatResult {
                    message_id,
                    content: response.text,
                    tool_calls: response.tool_calls.into_iter().map(|tc| ToolCallRecord {
                        id: tc.id,
                        name: tc.name,
                        input: tc.input,
                        output: Some(tc.output),
                        success: Some(tc.success),
                        duration_ms: Some(tc.duration_ms),
                    }).collect(),
                    input_tokens: response.input_tokens,
                    output_tokens: response.output_tokens,
                })
            }
            Err(e) => {
                let _ = self.event_tx.send(Event::Error {
                    code: "agent_error".to_string(),
                    message: e.to_string(),
                });
                Err(e.to_string())
            }
        }
    }
    
    /// Cancel an active chat
    pub async fn cancel(&self, session_id: &str) -> bool {
        let mut active = self.active_chats.write().await;
        if let Some(chat) = active.get_mut(session_id) {
            chat.cancelled = true;
            true
        } else {
            false
        }
    }
    
    /// Check if a chat is active
    pub async fn is_active(&self, session_id: &str) -> bool {
        let active = self.active_chats.read().await;
        active.contains_key(session_id)
    }
    
    /// Get memory context for a query
    pub async fn recall_memories(&self, query: &str, limit: usize) -> Vec<MemoryContext> {
        if let Some(ref memory) = self.memory {
            match memory.recall(query).await {
                Ok(results) => {
                    results.into_iter()
                        .take(limit)
                        .map(|r| MemoryContext {
                            id: r.id,
                            content: r.content,
                            score: r.score,
                        })
                        .collect()
                }
                Err(e) => {
                    warn!("Memory recall failed: {}", e);
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        }
    }
    
    /// Store a memory
    pub async fn remember(&self, content: &str, metadata: HashMap<String, String>) -> Result<String, String> {
        if let Some(ref memory) = self.memory {
            memory.remember_with_importance(content, metadata, 3.0)
                .await
                .map(|(id, _)| id)
                .map_err(|e| e.to_string())
        } else {
            Err("Memory service not configured".to_string())
        }
    }
}

/// Result of a chat completion
#[derive(Debug, Clone)]
pub struct ChatResult {
    pub message_id: String,
    pub content: String,
    pub tool_calls: Vec<ToolCallRecord>,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Memory context for injection
#[derive(Debug, Clone)]
pub struct MemoryContext {
    pub id: String,
    pub content: String,
    pub score: f32,
}
