//! Embedded Backend - Runs agent directly in GUI process
//!
//! Fallback mode when daemon is unavailable

use nanna_agent::{Agent, AgentConfig, AgentContext, ExtractedMemory, RunOptions};
use nanna_llm::LlmClient;
use nanna_memory::MemoryService;
use nanna_storage::{Message, Session, Storage};
use nanna_tools::ToolRegistry;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Embedded backend that runs the agent directly
pub struct EmbeddedBackend {
    llm: Arc<LlmClient>,
    tools: Arc<ToolRegistry>,
    memory: Arc<MemoryService>,
    storage: Arc<Storage>,
    /// In-memory session cache (session_id -> context)
    sessions: Arc<RwLock<std::collections::HashMap<String, AgentContext>>>,
    config: AgentConfig,
}

impl EmbeddedBackend {
    /// Create a new embedded backend
    pub async fn new(
        llm: Arc<LlmClient>,
        tools: Arc<ToolRegistry>,
        memory: Arc<MemoryService>,
        storage: Arc<Storage>,
    ) -> Self {
        Self {
            llm,
            tools,
            memory,
            storage,
            sessions: Arc::new(RwLock::new(std::collections::HashMap::new())),
            config: AgentConfig::default(),
        }
    }
    
    /// Send a chat message and get response
    pub async fn chat(
        &self,
        session_id: String,
        message: String,
        system_prompt: Option<String>,
    ) -> Result<Value, String> {
        // Store user message
        self.storage
            .add_message(&session_id, "user", &message)
            .await
            .map_err(|e| format!("Failed to store message: {}", e))?;
        
        // Get or create context for this session
        let context = {
            let mut sessions = self.sessions.write().await;
            sessions.entry(session_id.clone())
                .or_insert_with(|| {
                    let mut ctx = AgentContext::new(session_id.clone());
                    if let Some(ref prompt) = system_prompt {
                        ctx = ctx.with_system_prompt(prompt);
                    }
                    ctx
                })
                .clone()
        };
        
        // Create agent with context
        let agent = Agent::new(
            self.config.clone(),
            self.llm.clone(),
            self.tools.clone(),
        ).with_context(context);

        // Clone memory service for auto-extraction callback
        let memory_for_extraction = self.memory.clone();

        // Create run options with auto memory extraction enabled
        let run_options = RunOptions {
            auto_extract_memories: true,
            on_memory: Some(Box::new(move |memory: ExtractedMemory| {
                let mem_service = memory_for_extraction.clone();
                Box::pin(async move {
                    let mut metadata = HashMap::new();
                    metadata.insert("category".to_string(), memory.category.clone());
                    metadata.insert("source".to_string(), "auto_extract".to_string());
                    // Derive importance from category
                    let importance = match memory.category.as_str() {
                        "preference" | "identity" => 4.0,
                        "fact" | "insight" => 3.5,
                        "context" => 3.0,
                        _ => 3.0,
                    };
                    if let Err(e) = mem_service.remember_with_importance(&memory.content, metadata, importance).await {
                        warn!("Failed to auto-store memory: {}", e);
                    } else {
                        info!("Auto-extracted memory [{}]: {}", memory.category, truncate(&memory.content, 50));
                    }
                })
            })),
            ..Default::default()
        };

        // Run the agent
        let result = agent.run(&message, run_options)
            .await
            .map_err(|e| e.to_string())?;
        
        // Store assistant response
        self.storage
            .add_message(&session_id, "assistant", &result.text)
            .await
            .map_err(|e| format!("Failed to store response: {}", e))?;
        
        Ok(json!({
            "content": result.text,
            "tool_calls": result.tool_calls,
            "input_tokens": result.input_tokens,
            "output_tokens": result.output_tokens,
        }))
    }
    
    /// List sessions
    pub async fn list_sessions(&self) -> Result<Vec<Session>, String> {
        self.storage
            .list_gui_sessions_by_workspace(None, 100)
            .await
            .map_err(|e| format!("Failed to list sessions: {}", e))
    }
    
    /// Create a new session
    pub async fn create_session(&self, name: Option<String>) -> Result<Session, String> {
        let session_name = name.unwrap_or_else(|| {
            format!("Chat {}", chrono::Utc::now().format("%Y-%m-%d %H:%M"))
        });
        
        self.storage
            .create_gui_session_with_workspace(&session_name, None)
            .await
            .map_err(|e| format!("Failed to create session: {}", e))
    }
    
    /// Get session history
    pub async fn get_session_history(&self, session_id: &str, limit: usize) -> Result<Vec<Message>, String> {
        self.storage
            .get_session_messages(session_id, limit as i64)
            .await
            .map_err(|e| format!("Failed to get history: {}", e))
    }
    
    /// Delete a session
    pub async fn delete_session(&self, session_id: &str) -> Result<(), String> {
        self.storage
            .delete_session(session_id)
            .await
            .map_err(|e| format!("Failed to delete session: {}", e))?;
        
        // Remove from cache
        let mut sessions = self.sessions.write().await;
        sessions.remove(session_id);
        
        Ok(())
    }
    
    /// Clear session messages (removes from context cache)
    pub async fn clear_session(&self, session_id: &str) -> Result<(), String> {
        // Note: Storage doesn't have clear_session_messages method yet
        // For now, just clear the in-memory context
        let mut sessions = self.sessions.write().await;
        sessions.remove(session_id);
        
        Ok(())
    }
    
    /// Search memory
    pub async fn search_memory(&self, query: &str, limit: usize) -> Result<Value, String> {
        let results = self.memory
            .recall(query)
            .await
            .map_err(|e| format!("Memory search failed: {}", e))?;
        
        let memories: Vec<_> = results.into_iter()
            .take(limit)
            .map(|r| json!({
                "id": r.id,
                "content": r.content,
                "score": r.score,
                "weight": r.weight,
                "metadata": r.metadata,
            }))
            .collect();
        
        Ok(json!({ "memories": memories }))
    }
    
    /// Get memory stats
    pub async fn memory_stats(&self) -> Result<Value, String> {
        let stats = self.memory.stats().await;
        Ok(json!({
            "total": stats.total,
            "active": stats.active,
            "dormant": stats.dormant,
            "silent": stats.silent,
            "unavailable": stats.unavailable,
        }))
    }
    
    /// List available tools
    pub async fn list_tools(&self) -> Result<Value, String> {
        let definitions = self.tools.definitions().await;
        let tools: Vec<_> = definitions.into_iter()
            .map(|t| json!({
                "name": t.name,
                "description": t.description,
                "parameters": t.parameters,
            }))
            .collect();
        
        Ok(json!({ "tools": tools }))
    }
    
    /// Get system status
    pub async fn system_status(&self) -> Result<Value, String> {
        let tool_count = self.tools.definitions().await.len();
        let memory_stats = self.memory.stats().await;
        let session_count = {
            let sessions = self.sessions.read().await;
            sessions.len()
        };
        
        Ok(json!({
            "mode": "embedded",
            "tool_count": tool_count,
            "session_count": session_count,
            "memory_total": memory_stats.total,
            "version": env!("CARGO_PKG_VERSION"),
        }))
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}
