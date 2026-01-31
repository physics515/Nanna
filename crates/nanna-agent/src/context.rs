//! Agent context management

use nanna_llm::{AnthropicMessage, ContentBlock, LlmClient, RequestBuilder};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Compressed context summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSummary {
    /// The compressed summary text
    pub summary: String,
    /// Number of messages that were compressed
    pub messages_compressed: usize,
    /// Approximate tokens saved
    pub tokens_saved: usize,
    /// When the summary was created
    pub created_at: i64,
}

/// Context isolation mode for sub-agents
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContextIsolation {
    /// Full context inherited from parent
    #[default]
    Full,
    /// Only system prompt inherited
    SystemOnly,
    /// Summary of parent context provided
    Summary,
    /// Completely isolated (fresh context)
    Isolated,
}

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
    /// Compressed summaries of older context
    #[serde(default)]
    pub summaries: Vec<ContextSummary>,
    /// Maximum tokens before compression triggers
    #[serde(default = "default_compression_threshold")]
    pub compression_threshold: usize,
    /// Parent context ID (if this is a sub-agent)
    #[serde(default)]
    pub parent_context_id: Option<String>,
    /// How much context was inherited from parent
    #[serde(default)]
    pub isolation_mode: Option<String>,
    /// Context budget in tokens for sub-agents (limits how much context can be used)
    #[serde(default)]
    pub context_budget: Option<usize>,
}

fn default_compression_threshold() -> usize {
    50_000 // ~50k tokens before compression
}

impl AgentContext {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            system_prompt: String::new(),
            messages: Vec::new(),
            metadata: HashMap::new(),
            max_messages: 100,
            summaries: Vec::new(),
            compression_threshold: default_compression_threshold(),
            parent_context_id: None,
            isolation_mode: None,
            context_budget: None,
        }
    }

    /// Set the system prompt.
    #[must_use]
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = prompt.into();
        self
    }

    /// Set the compression threshold
    #[must_use]
    pub fn with_compression_threshold(mut self, threshold: usize) -> Self {
        self.compression_threshold = threshold;
        self
    }

    /// Set the context budget in tokens
    #[must_use]
    pub fn with_context_budget(mut self, budget: usize) -> Self {
        self.context_budget = Some(budget);
        self
    }

    /// Allocate a portion of context budget to a sub-agent.
    /// 
    /// Divides the available budget among multiple sub-agents, with the option
    /// to give priority to earlier agents (lower index gets slightly more).
    /// 
    /// # Arguments
    /// * `num_agents` - Total number of sub-agents to allocate for
    /// * `agent_index` - Index of this agent (0-based)
    /// 
    /// # Returns
    /// The allocated budget in tokens for this sub-agent.
    /// Returns a default of 10,000 tokens if no budget is set.
    #[must_use]
    pub fn allocate_budget(&self, num_agents: usize, agent_index: usize) -> usize {
        let total_budget = self.context_budget.unwrap_or(100_000);
        
        if num_agents == 0 {
            return total_budget;
        }
        
        // Reserve 20% for coordination/aggregation overhead
        let distributable = (total_budget * 80) / 100;
        
        // Base allocation per agent
        let base_per_agent = distributable / num_agents;
        
        // Give slightly more to earlier agents (they often do foundational work)
        // This creates a gentle gradient: first agent gets ~10% more than last
        let priority_bonus = if num_agents > 1 {
            let remaining_priority = (distributable * 10) / 100; // 10% for priority distribution
            let position_factor = (num_agents - 1 - agent_index) as f64 / (num_agents - 1) as f64;
            ((remaining_priority as f64 * position_factor) / num_agents as f64) as usize
        } else {
            0
        };
        
        base_per_agent + priority_bonus
    }

    /// Create an isolated sub-context based on isolation mode
    #[must_use]
    pub fn create_isolated(&self, mode: ContextIsolation) -> Self {
        let mut ctx = Self::new(Uuid::new_v4().to_string());
        ctx.parent_context_id = Some(self.session_id.clone());
        ctx.isolation_mode = Some(format!("{mode:?}"));

        match mode {
            ContextIsolation::Full => {
                ctx.system_prompt = self.system_prompt.clone();
                ctx.messages = self.messages.clone();
                ctx.summaries = self.summaries.clone();
            }
            ContextIsolation::SystemOnly => {
                ctx.system_prompt = self.system_prompt.clone();
            }
            ContextIsolation::Summary => {
                ctx.system_prompt = self.system_prompt.clone();
                // Add summaries as context in system prompt
                if !self.summaries.is_empty() {
                    let summary_text: String = self.summaries
                        .iter()
                        .map(|s| s.summary.as_str())
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    ctx.system_prompt = format!(
                        "{}\n\n## Previous Context Summary\n{}",
                        ctx.system_prompt, summary_text
                    );
                }
            }
            ContextIsolation::Isolated => {
                // Completely fresh - only set parent_context_id for reference
            }
        }

        ctx
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
    #[must_use] 
    pub fn estimate_tokens(&self) -> usize {
        let system_tokens = self.system_prompt.len() / 4;
        let summary_tokens: usize = self.summaries.iter().map(|s| s.summary.len() / 4).sum();
        let message_tokens: usize = self
            .messages
            .iter()
            .map(|m| {
                m.content
                    .iter()
                    .map(|c| match c {
                        ContentBlock::Text { text } => text.len() / 4,
                        ContentBlock::ToolUse { input, .. } => {
                            input.to_string().len() / 4 + 50
                        }
                        ContentBlock::ToolResult { content, .. } => content.len() / 4 + 20,
                        ContentBlock::Image { .. } => 1000, // Images are ~1k tokens
                        ContentBlock::Thinking { thinking } => thinking.len() / 4,
                    })
                    .sum::<usize>()
            })
            .sum();

        system_tokens + summary_tokens + message_tokens
    }

    /// Check if compression is needed based on token count
    #[must_use]
    pub fn needs_compression(&self) -> bool {
        self.estimate_tokens() > self.compression_threshold
    }

    /// Compress old messages into a summary using LLM.
    /// 
    /// Keeps the most recent `keep_recent` messages and compresses the rest.
    /// 
    /// # Errors
    /// Returns error if LLM call fails
    pub async fn compress(
        &mut self,
        llm: &LlmClient,
        model: &str,
        keep_recent: usize,
    ) -> Result<ContextSummary, nanna_llm::LlmError> {
        if self.messages.len() <= keep_recent {
            // Nothing to compress
            return Ok(ContextSummary {
                summary: String::new(),
                messages_compressed: 0,
                tokens_saved: 0,
                created_at: chrono_timestamp(),
            });
        }

        // Split messages into old (to compress) and recent (to keep)
        let split_point = self.messages.len() - keep_recent;
        let old_messages = &self.messages[..split_point];
        
        // Build a text representation of old messages
        let mut conversation_text = String::new();
        for msg in old_messages {
            let role = &msg.role;
            for block in &msg.content {
                match block {
                    ContentBlock::Text { text } => {
                        conversation_text.push_str(&format!("[{role}]: {text}\n"));
                    }
                    ContentBlock::ToolUse { name, .. } => {
                        conversation_text.push_str(&format!("[{role}]: [Called tool: {name}]\n"));
                    }
                    ContentBlock::ToolResult { content, .. } => {
                        // Truncate long tool results in summary
                        let truncated = if content.len() > 200 {
                            format!("{}...", &content[..200])
                        } else {
                            content.clone()
                        };
                        conversation_text.push_str(&format!("[tool result]: {truncated}\n"));
                    }
                    ContentBlock::Thinking { thinking } => {
                        // Include reasoning in summary, truncated
                        let truncated = if thinking.len() > 200 {
                            format!("{}...", &thinking[..200])
                        } else {
                            thinking.clone()
                        };
                        conversation_text.push_str(&format!("[thinking]: {truncated}\n"));
                    }
                    ContentBlock::Image { .. } => {
                        conversation_text.push_str("[image]\n");
                    }
                }
            }
        }

        // Create summarization prompt
        let prompt = format!(
            r#"Summarize this conversation concisely, preserving key facts, decisions, and context that would be important for continuing the conversation. Focus on:
- Important user preferences or information shared
- Key decisions or conclusions reached
- Relevant context about ongoing tasks or projects
- Any commitments or follow-ups mentioned

Conversation to summarize:
{}

Provide a concise summary (2-4 paragraphs max):"#,
            conversation_text
        );

        // Call LLM for summarization
        let request = nanna_llm::CompletionRequest::default()
            .with_model(model)
            .with_message(nanna_llm::Message::user(&prompt))
            .with_max_tokens(1024)
            .with_temperature(0.3);

        let summary_text = llm.complete(&request).await?;

        // Calculate tokens saved
        let old_tokens: usize = old_messages.iter()
            .map(|m| m.content.iter()
                .map(|c| match c {
                    ContentBlock::Text { text } => text.len() / 4,
                    ContentBlock::ToolUse { input, .. } => input.to_string().len() / 4,
                    ContentBlock::ToolResult { content, .. } => content.len() / 4,
                    ContentBlock::Thinking { thinking } => thinking.len() / 4,
                    ContentBlock::Image { .. } => 1000,
                })
                .sum::<usize>()
            )
            .sum();
        let new_tokens = summary_text.len() / 4;
        let tokens_saved = old_tokens.saturating_sub(new_tokens);

        let summary = ContextSummary {
            summary: summary_text,
            messages_compressed: old_messages.len(),
            tokens_saved,
            created_at: chrono_timestamp(),
        };

        // Store summary and remove old messages
        self.summaries.push(summary.clone());
        self.messages = self.messages.split_off(split_point);

        Ok(summary)
    }

    /// Get combined context including summaries for building prompts
    #[must_use]
    pub fn get_full_context(&self) -> String {
        let mut context = String::new();

        // Add summaries first (older context)
        if !self.summaries.is_empty() {
            context.push_str("## Previous Conversation Summary\n");
            for summary in &self.summaries {
                context.push_str(&summary.summary);
                context.push_str("\n\n");
            }
            context.push_str("---\n\n## Current Conversation\n");
        }

        context
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

fn chrono_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
