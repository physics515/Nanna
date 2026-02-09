//! Application state shared across handlers

use nanna_agent::{Agent, AgentConfig, AgentContext, ExtractedMemory, MemoryCallback, RunOptions};
use nanna_channels::{DiscordChannel, TelegramChannel};
use nanna_core::{
    DreamingRuntime, DreamingRuntimeConfig, MemoryFeedback, Nanna, Scheduler, SchedulerConfig,
    consolidation_task,
};
use nanna_llm::{EmbeddingClient, LlmClient};
use nanna_storage::Storage;
use nanna_tools::ToolRegistry;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// A recently stored memory (for feedback attribution)
#[derive(Debug, Clone)]
pub struct RecentMemory {
    pub memory_id: String,
    pub stored_at: Instant,
}

impl RecentMemory {
    /// Check if this memory is still recent (within threshold)
    pub fn is_recent(&self, max_age: Duration) -> bool {
        self.stored_at.elapsed() < max_age
    }
}

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
    /// Dreaming runtime for memory consolidation
    pub dreaming: Option<Arc<DreamingRuntime>>,
    /// Scheduler for periodic tasks (heartbeats, consolidation)
    pub scheduler: Option<Arc<RwLock<Scheduler>>>,
    /// Map message IDs to session IDs for reaction-based feedback
    pub message_session_map: Arc<RwLock<HashMap<String, String>>>,
    /// Track recent memory IDs per session (for feedback attribution)
    pub session_recent_memories: Arc<RwLock<HashMap<String, Vec<RecentMemory>>>>,
}

/// Builder for `AppState`
pub struct AppStateBuilder {
    bot: Option<Nanna>,
    storage: Option<Arc<Storage>>,
    llm: Option<Arc<LlmClient>>,
    embed: Option<Arc<EmbeddingClient>>,
    tools: Option<Arc<ToolRegistry>>,
    webhook_secret: Option<String>,
    discord_public_key: Option<String>,
    default_model: String,
    telegram_token: Option<String>,
    discord_bot_token: Option<String>,
    discord_app_id: Option<String>,
    enable_dreaming: bool,
    enable_scheduler: bool,
    dreaming_config: Option<DreamingRuntimeConfig>,
    scheduler_config: Option<SchedulerConfig>,
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
            embed: None,
            tools: None,
            webhook_secret: None,
            discord_public_key: None,
            default_model: "claude-sonnet-4-20250514".to_string(),
            telegram_token: None,
            discord_bot_token: None,
            discord_app_id: None,
            enable_dreaming: true,
            enable_scheduler: true,
            dreaming_config: None,
            scheduler_config: None,
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

    /// Set the embedding client for memory operations.
    #[must_use]
    pub fn embed(mut self, embed: EmbeddingClient) -> Self {
        self.embed = Some(Arc::new(embed));
        self
    }

    /// Set the embedding client (Arc).
    #[must_use]
    pub fn embed_arc(mut self, embed: Arc<EmbeddingClient>) -> Self {
        self.embed = Some(embed);
        self
    }

    /// Enable or disable the dreaming memory system.
    #[must_use]
    pub fn dreaming(mut self, enable: bool) -> Self {
        self.enable_dreaming = enable;
        self
    }

    /// Set custom dreaming configuration.
    #[must_use]
    pub fn dreaming_config(mut self, config: DreamingRuntimeConfig) -> Self {
        self.dreaming_config = Some(config);
        self
    }

    /// Enable or disable the scheduler.
    #[must_use]
    pub fn scheduler(mut self, enable: bool) -> Self {
        self.enable_scheduler = enable;
        self
    }

    /// Set custom scheduler configuration.
    #[must_use]
    pub fn scheduler_config(mut self, config: SchedulerConfig) -> Self {
        self.scheduler_config = Some(config);
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

        let llm = self.llm.expect("llm required");

        // Create dreaming runtime if enabled and embeddings are available
        let dreaming = if self.enable_dreaming {
            let config = self.dreaming_config.unwrap_or_default();
            if let Some(embed) = &self.embed {
                Some(Arc::new(DreamingRuntime::new(config, llm.clone(), embed.clone())))
            } else {
                tracing::warn!("Dreaming enabled but no embedding client provided - using without embeddings");
                Some(Arc::new(DreamingRuntime::new_without_embeddings(config, llm.clone())))
            }
        } else {
            None
        };

        // Create scheduler if enabled
        let scheduler = if self.enable_scheduler {
            let config = self.scheduler_config.unwrap_or_default();
            let storage = self.storage.clone();
            let mut sched = Scheduler::new(config);
            
            // Add storage for persistence if available
            if let Some(ref store) = storage {
                sched = sched.with_storage(store.clone());
            }
            
            Some(Arc::new(RwLock::new(sched)))
        } else {
            None
        };

        AppState {
            bot: Arc::new(self.bot.expect("bot required")),
            storage: self.storage.expect("storage required"),
            llm,
            tools: self.tools.expect("tools required"),
            agents: Arc::new(RwLock::new(HashMap::new())),
            webhook_secret: self.webhook_secret,
            discord_public_key: self.discord_public_key,
            default_model: self.default_model,
            telegram,
            discord,
            dreaming,
            scheduler,
            message_session_map: Arc::new(RwLock::new(HashMap::new())),
            session_recent_memories: Arc::new(RwLock::new(HashMap::new())),
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
            max_iterations: Some(10),
            thinking_mode: nanna_agent::ThinkingMode::Instant,
            summarization_priority: vec![],
            summarization_ollama_url: Some("http://localhost:11434".to_string()),
            ..Default::default()
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

        // Build run options with memory extraction if dreaming is enabled
        let run_options = if let Some(dreaming) = &self.dreaming {
            let dreaming = dreaming.clone();
            let session = session_id.to_string();
            let recent_memories = self.session_recent_memories.clone();
            
            // Create memory callback that stores extracted memories and tracks them
            let on_memory: MemoryCallback = Box::new(move |memory: ExtractedMemory| {
                let dreaming = dreaming.clone();
                let session = session.clone();
                let recent_memories = recent_memories.clone();
                Box::pin(async move {
                    let mut metadata = std::collections::HashMap::new();
                    metadata.insert("category".to_string(), memory.category.clone());
                    metadata.insert("session_id".to_string(), session.clone());
                    metadata.insert("source".to_string(), "extraction".to_string());
                    
                    match dreaming.remember(&memory.content, metadata).await {
                        Ok(id) => {
                            tracing::info!(
                                "Stored memory from extraction: {} (category: {}, id: {})",
                                memory.content.chars().take(50).collect::<String>(),
                                memory.category,
                                id
                            );
                            
                            // Track this memory for feedback attribution
                            let mut sessions = recent_memories.write().await;
                            let memories = sessions.entry(session).or_default();
                            memories.push(RecentMemory {
                                memory_id: id,
                                stored_at: Instant::now(),
                            });
                        }
                        Err(e) => {
                            tracing::warn!("Failed to store extracted memory: {}", e);
                        }
                    }
                })
            });

            RunOptions {
                auto_extract_memories: true,
                on_memory: Some(on_memory),
                ..Default::default()
            }
        } else {
            RunOptions::default()
        };

        // Run agent (scoped lock)
        let response = {
            let agent_lock = self.get_or_create_agent(session_id, system_prompt).await;
            let agent = agent_lock.read().await;
            agent.run(message, run_options).await?
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

impl AppState {
    /// Start the scheduler with dreaming executor.
    ///
    /// Call this once after building the state to start periodic tasks.
    pub async fn start_scheduler(&self) {
        let Some(scheduler) = &self.scheduler else {
            tracing::debug!("Scheduler not enabled");
            return;
        };

        let mut sched = scheduler.write().await;

        // Load persisted jobs
        if let Err(e) = sched.load_jobs().await {
            tracing::warn!("Failed to load persisted jobs: {}", e);
        }

        // Set up the task executor
        if let Some(dreaming) = &self.dreaming {
            let dreaming = dreaming.clone();
            
            // Wrap to handle all task types
            let full_executor: nanna_core::TaskExecutor = Arc::new(move |task: nanna_core::ScheduledTask| {
                let dreaming = dreaming.clone();
                Box::pin(async move {
                    // Check if it's a dreaming task
                    if nanna_core::is_dreaming_task(&task) {
                        let start = std::time::Instant::now();
                        let started_at = chrono::Utc::now();
                        tracing::info!("Starting memory consolidation (dreaming)...");

                        match dreaming.dream().await {
                            Ok(stats) => {
                                let output = format!(
                                    "Dreaming complete: {} processed, {} merged, {} expanded",
                                    stats.consolidation.memories_processed,
                                    stats.consolidation.memories_merged,
                                    stats.consolidation.memories_expanded,
                                );
                                tracing::info!("{}", output);

                                let finished_at = chrono::Utc::now();
                                nanna_core::TaskResult {
                                    task_id: task.id.clone(),
                                    task_name: task.name.clone(),
                                    success: true,
                                    output: Some(output),
                                    error: None,
                                    duration_ms: start.elapsed().as_millis() as u64,
                                    started_at,
                                    finished_at,
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Dreaming failed: {}", e);
                                let finished_at = chrono::Utc::now();
                                nanna_core::TaskResult {
                                    task_id: task.id.clone(),
                                    task_name: task.name.clone(),
                                    success: false,
                                    output: None,
                                    error: Some(e.to_string()),
                                    duration_ms: start.elapsed().as_millis() as u64,
                                    started_at,
                                    finished_at,
                                }
                            }
                        }
                    } else {
                        // Default executor for other tasks (heartbeat, etc.)
                        let start = std::time::Instant::now();
                        let started_at = chrono::Utc::now();
                        tracing::info!("Executing task: {} ({})", task.name, task.id);
                        let finished_at = chrono::Utc::now();
                        nanna_core::TaskResult {
                            task_id: task.id.clone(),
                            task_name: task.name.clone(),
                            success: true,
                            output: Some(task.payload),
                            error: None,
                            duration_ms: start.elapsed().as_millis() as u64,
                            started_at,
                            finished_at,
                        }
                    }
                })
            });

            sched.set_executor(full_executor);

            // Add the consolidation task if not already present
            let tasks = sched.list_tasks().await;
            let has_consolidation = tasks.iter().any(|t| t.name == "memory_consolidation");
            if !has_consolidation {
                sched.add_task(consolidation_task(None)).await;
                tracing::info!("Added memory consolidation task (every 1 hour)");
            }
        }

        sched.start();
        tracing::info!("Scheduler started");
    }

    /// Stop the scheduler gracefully.
    pub async fn stop_scheduler(&self) {
        if let Some(scheduler) = &self.scheduler {
            scheduler.write().await.stop().await;
            tracing::info!("Scheduler stopped");
        }
    }

    /// Record feedback for a message (maps to memory feedback).
    ///
    /// Call this when a user reacts to a message with 👍/👎.
    /// Applies feedback to all recent memories from the session.
    pub async fn record_message_feedback(&self, message_id: &str, positive: bool) {
        // Look up the session ID for this message
        let session_id = {
            let map = self.message_session_map.read().await;
            map.get(message_id).cloned()
        };

        let Some(session_id) = session_id else {
            tracing::debug!("No session mapped for message {}", message_id);
            return;
        };

        let Some(dreaming) = &self.dreaming else {
            return;
        };

        // Get recent memories for this session
        let recent_memories = {
            let mut sessions = self.session_recent_memories.write().await;
            let memories = sessions.entry(session_id.clone()).or_default();
            
            // Filter to only recent memories (within 10 minutes)
            let max_age = Duration::from_secs(600);
            memories.retain(|m| m.is_recent(max_age));
            
            memories.clone()
        };

        if recent_memories.is_empty() {
            tracing::debug!("No recent memories for session {}", session_id);
            return;
        }

        let feedback = if positive {
            MemoryFeedback::Helpful
        } else {
            MemoryFeedback::Unhelpful
        };

        // Apply feedback to all recent memories
        for memory in &recent_memories {
            dreaming.record_feedback(&memory.memory_id, feedback).await;
        }

        tracing::info!(
            "Recorded {:?} feedback for {} memories in session {} (message {})",
            feedback,
            recent_memories.len(),
            session_id,
            message_id
        );
    }

    /// Associate a message ID with a session for feedback tracking.
    pub async fn link_message_to_session(&self, message_id: &str, session_id: &str) {
        let mut map = self.message_session_map.write().await;
        map.insert(message_id.to_string(), session_id.to_string());
        
        // Prune old entries if map gets too large
        const MAX_ENTRIES: usize = 10000;
        if map.len() > MAX_ENTRIES {
            let to_remove: Vec<_> = map.keys().take(MAX_ENTRIES / 2).cloned().collect();
            for key in to_remove {
                map.remove(&key);
            }
        }
    }

    /// Track a memory ID for a session (for feedback attribution).
    pub async fn track_memory_for_session(&self, session_id: &str, memory_id: &str) {
        let mut sessions = self.session_recent_memories.write().await;
        let memories = sessions.entry(session_id.to_string()).or_default();
        
        memories.push(RecentMemory {
            memory_id: memory_id.to_string(),
            stored_at: Instant::now(),
        });

        // Keep only the last N memories per session
        const MAX_PER_SESSION: usize = 50;
        if memories.len() > MAX_PER_SESSION {
            memories.drain(0..memories.len() - MAX_PER_SESSION);
        }
    }
}

const DEFAULT_SYSTEM_PROMPT: &str = r"You are Nanna — moon god of the digital realm.

You have tools at your disposal. Use them when needed.

Be helpful. Be competent. Don't waste words.";
