//! Agent context management

use nanna_llm::AnthropicMessage;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Context for an agent session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContext {
    /// Session identifier
    pub session_id: String,
    /// System prompt
    pub system_prompt: String,
    /// Conversation history (Anthropic format)
    pub messages: Vec<AnthropicMessage>,
    /// Session metadata
    pub metadata: HashMap<String, String>,
    /// Maximum number of messages to keep
    pub max_messages: usize,
}

impl AgentContext {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            system_prompt: String::new(),
            messages: Vec::new(),
            metadata: HashMap::new(),
            max_messages: 100,
        }
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = prompt.into();
        self
    }

    /// Add a user text message
    pub fn add_user_message(&mut self, content: impl Into<String>) {
        self.messages.push(AnthropicMessage::user_text(content));
        self.trim_if_needed();
    }

    /// Add an assistant text message
    pub fn add_assistant_message(&mut self, content: impl Into<String>) {
        self.messages.push(AnthropicMessage::assistant_text(content));
        self.trim_if_needed();
    }

    /// Estimate token count (rough heuristic: ~4 chars per token)
    pub fn estimate_tokens(&self) -> usize {
        let system_tokens = self.system_prompt.len() / 4;
        let message_tokens: usize = self
            .messages
            .iter()
            .map(|m| {
                m.content
                    .iter()
                    .map(|c| match c {
                        nanna_llm::ContentBlock::Text { text } => text.len() / 4,
                        nanna_llm::ContentBlock::ToolUse { input, .. } => {
                            input.to_string().len() / 4 + 50
                        }
                        nanna_llm::ContentBlock::ToolResult { content, .. } => content.len() / 4 + 20,
                    })
                    .sum::<usize>()
            })
            .sum();

        system_tokens + message_tokens
    }

    fn trim_if_needed(&mut self) {
        while self.messages.len() > self.max_messages {
            self.messages.remove(0);
        }
    }
}

impl Default for AgentContext {
    fn default() -> Self {
        Self::new(Uuid::new_v4().to_string())
    }
}
