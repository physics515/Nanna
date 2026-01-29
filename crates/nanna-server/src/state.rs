//! Application state shared across handlers

use nanna_agent::{Agent, AgentConfig, AgentContext, RunOptions};
use nanna_channels::{DiscordChannel, TelegramChannel};
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
    pub discord_public_key: Option<String>,
    pub default_model: String,
    /// Telegram channel for proactive sends
    pub telegram: Option<Arc<TelegramChannel>>,
    /// Discord channel for proactive sends
    pub discord: Option<Arc<DiscordChannel>>,
}

/// Builder for `AppState`
pub struct AppStateBuilder {
    bot: Option<Nanna>,
    storage: Option<Arc<Storage>>,
    llm: Option<Arc<LlmClient>>,
    tools: Option<Arc<ToolRegistry>>,
    webhook_secret: Option<String>,
    discord_public_key: Option<String>,
    default_model: String,
    telegram_token: Option<String>,
    discord_bot_token: Option<String>,
    discord_app_id: Option<String>,
}

impl Default for AppStateBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AppStateBuilder {
    #[must_use] 
    pub fn new() -> Self {
        Self {
            bot: None,
            storage: None,
            llm: None,
            tools: None,
            webhook_secret: None,
            discord_public_key: None,
            default_model: "claude-sonnet-4-20250514".to_string(),
            telegram_token: None,
            discord_bot_token: None,
            discord_app_id: None,
        }
    }

    #[must_use] 
    pub fn bot(mut self, bot: Nanna) -> Self {
        self.bot = Some(bot);
        self
    }

    #[must_use] 
    pub fn storage(mut self, storage: Storage) -> Self {
        self.storage = Some(Arc::new(storage));
        self
    }

    #[must_use] 
    pub fn storage_arc(mut self, storage: Arc<Storage>) -> Self {
        self.storage = Some(storage);
        self
    }

    #[must_use] 
    pub fn llm(mut self, llm: LlmClient) -> Self {
        self.llm = Some(Arc::new(llm));
        self
    }

    #[must_use] 
    pub fn llm_arc(mut self, llm: Arc<LlmClient>) -> Self {
        self.llm = Some(llm);
        self
    }

    pub fn tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = Some(Arc::new(tools));
        self
    }

    /// Set the tools (Arc).
    #[must_use]
    pub fn tools_arc(mut self, tools: Arc<ToolRegistry>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Set the webhook secret.
    #[must_use]
    pub fn webhook_secret(mut self, secret: Option<String>) -> Self {
        self.webhook_secret = secret;
        self
    }

    /// Set the Discord public key for signature verification.
    #[must_use]
    pub fn discord_public_key(mut self, key: Option<String>) -> Self {
        self.discord_public_key = key;
        self
    }

    /// Set the default model.
    #[must_use]
    pub fn default_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = model.into();
        self
    }

    /// Set the Telegram bot token.
    #[must_use]
    pub fn telegram_token(mut self, token: Option<String>) -> Self {
        self.telegram_token = token;
        self
    }

    /// Set the Discord bot token and application ID.
    #[must_use]
    pub fn discord_config(mut self, bot_token: Option<String>, app_id: Option<String>) -> Self {
        self.discord_bot_token = bot_token;
        self.discord_app_id = app_id;
        self
    }

    /// Build the `AppState`.
    ///
    /// # Panics
    ///
    /// Panics if bot, storage, llm, or tools are not set.
    #[must_use]
    pub fn build(self) -> AppState {
        let telegram = self.telegram_token.map(|token| {
            Arc::new(TelegramChannel::new(token))
        });

        let discord = match (self.discord_bot_token, self.discord_app_id) {
            (Some(token), Some(app_id)) => Some(Arc::new(DiscordChannel::new(token, app_id))),
            _ => None,
        };

        AppState {
            bot: Arc::new(self.bot.expect("bot required")),
            storage: self.storage.expect("storage required"),
            llm: self.llm.expect("llm required"),
            tools: self.tools.expect("tools required"),
            agents: Arc::new(RwLock::new(HashMap::new())),
            webhook_secret: self.webhook_secret,
            discord_public_key: self.discord_public_key,
            default_model: self.default_model,
            telegram,
            discord,
        }
    }
}

impl AppState {

    /// Get or create an agent for a session.
    pub async fn get_or_create_agent(
        &self,
        session_id: &str,
        system_prompt: Option<&str>,
    ) -> Arc<RwLock<Agent>> {
        // Check if agent exists (read lock)
        {
            let agents = self.agents.read().await;
            if let Some(agent) = agents.get(session_id) {
                return agent.clone();
            }
        }

        // Create new agent (outside lock)
        let config = AgentConfig {
            model: self.default_model.clone(),
            max_tokens: 8192,
            temperature: 0.7,
            max_iterations: 10,
        };

        let context = AgentContext::new(session_id)
            .with_system_prompt(system_prompt.unwrap_or(DEFAULT_SYSTEM_PROMPT));

        let agent = Agent::new(config, self.llm.clone(), self.tools.clone()).with_context(context);

        let agent = Arc::new(RwLock::new(agent));

        // Insert with write lock
        {
            let mut agents = self.agents.write().await;
            // Double-check in case another task created it
            if let Some(existing) = agents.get(session_id) {
                return existing.clone();
            }
            agents.insert(session_id.to_string(), agent.clone());
        }

        // Persist session to storage (outside lock)
        if let Err(e) = self
            .storage
            .sessions()
            .create(session_id, "api", None)
            .await
        {
            tracing::warn!("Failed to persist session {session_id}: {e}");
        }

        agent
    }

    /// Process a message through the agent and persist to storage.
    ///
    /// # Errors
    ///
    /// Returns `AgentError` if the agent fails to process the message.
    pub async fn process_message(
        &self,
        session_id: &str,
        message: &str,
        system_prompt: Option<&str>,
    ) -> Result<String, nanna_agent::AgentError> {
        // Store user message first
        let _ = self
            .storage
            .messages()
            .create(nanna_storage::NewMessage {
                session_id: session_id.to_string(),
                role: "user".to_string(),
                content: message.to_string(),
                content_type: "text".to_string(),
                tool_use_id: None,
                tokens_in: None,
                tokens_out: None,
                metadata: None,
            })
            .await;

        // Run agent (scoped lock)
        let response = {
            let agent_lock = self.get_or_create_agent(session_id, system_prompt).await;
            let agent = agent_lock.read().await;
            agent.run(message, RunOptions::default()).await?
        };

        // Store assistant response
        let _ = self
            .storage
            .messages()
            .create(nanna_storage::NewMessage {
                session_id: session_id.to_string(),
                role: "assistant".to_string(),
                content: response.text.clone(),
                content_type: "text".to_string(),
                tool_use_id: None,
                tokens_in: Some(i64::from(response.input_tokens)),
                tokens_out: Some(i64::from(response.output_tokens)),
                metadata: None,
            })
            .await;

        Ok(response.text)
    }
}

const DEFAULT_SYSTEM_PROMPT: &str = r"You are Nanna — moon god of the digital realm.

You have tools at your disposal. Use them when needed.

Be helpful. Be competent. Don't waste words.";
