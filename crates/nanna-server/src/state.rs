//! Application state shared across handlers

use nanna_agent::{Agent, AgentConfig, AgentContext, RunOptions};
use nanna_core::Nanna;
use nanna_llm::LlmClient;
use nanna_storage::Storage;
use nanna_tools::ToolRegistry;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub bot: Arc<Nanna>,
    pub storage: Arc<Storage>,
    pub llm: Arc<LlmClient>,
    pub tools: Arc<ToolRegistry>,
    pub agents: Arc<RwLock<HashMap<String, Arc<RwLock<Agent>>>>>,
    pub webhook_secret: Option<String>,
    pub default_model: String,
}

/// Builder for AppState
pub struct AppStateBuilder {
    bot: Option<Nanna>,
    storage: Option<Arc<Storage>>,
    llm: Option<Arc<LlmClient>>,
    tools: Option<Arc<ToolRegistry>>,
    webhook_secret: Option<String>,
    default_model: String,
}

impl AppStateBuilder {
    pub fn new() -> Self {
        Self {
            bot: None,
            storage: None,
            llm: None,
            tools: None,
            webhook_secret: None,
            default_model: "claude-sonnet-4-20250514".to_string(),
        }
    }

    pub fn bot(mut self, bot: Nanna) -> Self {
        self.bot = Some(bot);
        self
    }

    pub fn storage(mut self, storage: Storage) -> Self {
        self.storage = Some(Arc::new(storage));
        self
    }

    pub fn storage_arc(mut self, storage: Arc<Storage>) -> Self {
        self.storage = Some(storage);
        self
    }

    pub fn llm(mut self, llm: LlmClient) -> Self {
        self.llm = Some(Arc::new(llm));
        self
    }

    pub fn llm_arc(mut self, llm: Arc<LlmClient>) -> Self {
        self.llm = Some(llm);
        self
    }

    pub fn tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = Some(Arc::new(tools));
        self
    }

    pub fn tools_arc(mut self, tools: Arc<ToolRegistry>) -> Self {
        self.tools = Some(tools);
        self
    }

    pub fn webhook_secret(mut self, secret: Option<String>) -> Self {
        self.webhook_secret = secret;
        self
    }

    pub fn default_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = model.into();
        self
    }

    pub fn build(self) -> AppState {
        AppState {
            bot: Arc::new(self.bot.expect("bot required")),
            storage: self.storage.expect("storage required"),
            llm: self.llm.expect("llm required"),
            tools: self.tools.expect("tools required"),
            agents: Arc::new(RwLock::new(HashMap::new())),
            webhook_secret: self.webhook_secret,
            default_model: self.default_model,
        }
    }
}

impl AppState {

    /// Get or create an agent for a session
    pub async fn get_or_create_agent(&self, session_id: &str, system_prompt: Option<&str>) -> Arc<RwLock<Agent>> {
        let mut agents = self.agents.write().await;
        
        if let Some(agent) = agents.get(session_id) {
            return agent.clone();
        }

        // Create new agent
        let config = AgentConfig {
            model: self.default_model.clone(),
            max_tokens: 8192,
            temperature: 0.7,
            max_iterations: 10,
        };

        let context = AgentContext::new(session_id)
            .with_system_prompt(system_prompt.unwrap_or(DEFAULT_SYSTEM_PROMPT));

        let agent = Agent::new(config, self.llm.clone(), self.tools.clone())
            .with_context(context);

        let agent = Arc::new(RwLock::new(agent));
        agents.insert(session_id.to_string(), agent.clone());

        // Persist session to storage
        if let Err(e) = self.storage.sessions().create(session_id, "api", None).await {
            tracing::warn!("Failed to persist session {}: {}", session_id, e);
        }

        agent
    }

    /// Process a message through the agent and persist to storage
    pub async fn process_message(
        &self,
        session_id: &str,
        message: &str,
        system_prompt: Option<&str>,
    ) -> Result<String, nanna_agent::AgentError> {
        let agent = self.get_or_create_agent(session_id, system_prompt).await;
        let agent = agent.read().await;

        // Store user message
        let _ = self.storage.messages().create(nanna_storage::NewMessage {
            session_id: session_id.to_string(),
            role: "user".to_string(),
            content: message.to_string(),
            content_type: "text".to_string(),
            tool_use_id: None,
            tokens_in: None,
            tokens_out: None,
            metadata: None,
        }).await;

        // Run agent
        let response = agent.run(message, RunOptions::default()).await?;

        // Store assistant response
        let _ = self.storage.messages().create(nanna_storage::NewMessage {
            session_id: session_id.to_string(),
            role: "assistant".to_string(),
            content: response.text.clone(),
            content_type: "text".to_string(),
            tool_use_id: None,
            tokens_in: Some(response.input_tokens as i64),
            tokens_out: Some(response.output_tokens as i64),
            metadata: None,
        }).await;

        Ok(response.text)
    }
}

const DEFAULT_SYSTEM_PROMPT: &str = r#"You are Nanna — moon god of the digital realm.

You have tools at your disposal. Use them when needed.

Be helpful. Be competent. Don't waste words."#;
