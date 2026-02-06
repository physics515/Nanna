#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! Agent system for Nanna
//!
//! Implements the agentic loop with tool calling, memory, and context management.

mod chunker;
mod context;
mod loop_runner;
mod multi;
pub mod prompts;
mod registry;
mod summarizer;
mod supervisor;

#[cfg(feature = "mcp")]
pub mod mcp;

// Re-export workspace crate for convenience
pub use nanna_workspace;
pub use nanna_workspace::{Workspace, WorkspaceFiles, WorkspaceManager, WorkspaceTemplate};

pub use context::{AgentContext, ContextIsolation, ContextSummary, ContextSummarizationConfig};
pub use registry::{
    AgentMetadata, AgentRegistry, AgentRole, AgentState as RegistryAgentState, LifecycleEvent, 
    RegisteredAgent,
};
pub use loop_runner::{
    Agent, AgentConfig, AgentResponse, EmotionalContext, ExtractedMemory, MemoryCallback,
    ReasoningBlock, ReasoningContent, RunOptions, StreamCallback, ThinkingCallback,
    ThinkingMode, ToolCallRecord,
};
pub use multi::{
    AgentCoordinator, AgentEntry, AgentMessage, BackgroundTask, CriticalPathMetrics,
    TaskStatus, SwarmConfig, SwarmResult, SwarmTaskResult,
    // Swarm Coordinator
    SwarmCoordinator, DecomposedTask, DomainAgent, Subtask,
};
pub use supervisor::{
    AgentState, AgentStats, HealthCheckConfig, RestartPolicy, SupervisedAgentConfig,
    SupervisionStrategy, Supervisor, SupervisorEvent, SupervisorEventType,
};
pub use summarizer::{
    new_summary_cache, summarize_if_large, SummaryCache, SummaryCacheEntry, Summarizer,
    SummarizerConfig,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AgentError {
    #[error("LLM error: {0}")]
    Llm(#[from] nanna_llm::LlmError),
    #[error("Tool error: {0}")]
    Tool(#[from] nanna_tools::ToolError),
    #[error("Max iterations exceeded")]
    MaxIterations,
    #[error("Context too long")]
    ContextTooLong,
    #[error("Agent stopped")]
    Stopped,
}

/// Message content types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentContent {
    Text { text: String },
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult { tool_use_id: String, content: String, is_error: bool },
}

impl AgentContent {
    pub fn text(content: impl Into<String>) -> Self {
        Self::Text { text: content.into() }
    }
}
