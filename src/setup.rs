//! Shared component wiring: API key checks, scheduler, LLM/tools/storage setup.

use crate::onboarding;
use chrono::Utc;
use nanna_agent::{Agent, AgentConfig, AgentContext, RunOptions};
use nanna_config::Config;
use nanna_core::{LlmClient, Scheduler, SchedulerConfig, ScheduledTask, TaskResult};
use nanna_storage::{Storage, StorageConfig};
use nanna_tools::{
    CancelReminderTool, EchoTool, ExecTool, ExploreTool, ListDirTool, ListRemindersTool,
    ReadFileTool, RecallTool, ReflectTool, ReminderStore, RememberTool, RemindTool, StatusTool,
    ToolRegistry, TursoMemoryStorage, WebFetchTool, WebSearchTool, WonderTool, WriteFileTool,
};
use std::sync::Arc;
use tracing::{error, info};

/// Ensure API key is configured, prompt if not.
pub fn ensure_api_key(mut config: Config) -> anyhow::Result<Config> {
    if !onboarding::has_api_key(&config) {
        onboarding::quick_setup(&mut config)?;
    }
    Ok(config)
}

/// Create the scheduler with a task executor that runs tasks through an agent.
pub fn create_scheduler(
    config: &Config,
    llm: Arc<LlmClient>,
    tools: Arc<ToolRegistry>,
    storage: Arc<Storage>,
) -> Scheduler {
    let scheduler_config = SchedulerConfig {
        heartbeat_interval: std::time::Duration::from_mins(5), // 5 minutes
        heartbeat_enabled: true,
        heartbeat_prompt: "Heartbeat: check in and review state".to_string(),
        max_concurrent: 4,
        check_interval: std::time::Duration::from_secs(30),
        default_timezone: "UTC".to_string(),
    };

    // Clone storage for the scheduler's persistence
    let scheduler_storage = storage.clone();

    // Create a task executor that runs tasks through an agent
    let model = config.llm.model.clone();
    let executor: nanna_core::TaskExecutor = Arc::new(move |task: ScheduledTask| {
        let llm = llm.clone();
        let tools = tools.clone();
        let storage = storage.clone();
        let model = model.clone();

        Box::pin(async move {
            let start = std::time::Instant::now();
            let task_id = task.id.clone();

            info!("Executing scheduled task: {} ({})", task.name, task_id);

            // Create a dedicated agent for the task
            let agent_config = AgentConfig {
                model,
                max_tokens: 4096,
                temperature: 0.7,
                max_iterations: Some(5),
                thinking_mode: nanna_agent::ThinkingMode::Instant,
                summarization_priority: vec![],
                summarization_ollama_url: Some("http://localhost:11434".to_string()),
                ..Default::default()
            };

            let session_id = format!("scheduler:{}", task.name);
            let system_prompt = match task.task_type {
                nanna_core::TaskType::Heartbeat => {
                    "You are Nanna in heartbeat mode. Check in, review your state, \
                     and do any proactive work that needs attention. Be concise."
                }
                _ => {
                    "You are Nanna executing a scheduled task. Complete the task efficiently."
                }
            };

            let context = AgentContext::new(&session_id).with_system_prompt(system_prompt);
            let agent = Agent::new(agent_config, llm, tools).with_context(context);

            let started_at = Utc::now();
            let task_name = task.name.clone();

            // Run the task
            match agent.run(&task.payload, RunOptions::default()).await {
                Ok(response) => {
                    // Store the result
                    let _ = storage
                        .messages()
                        .create(nanna_storage::NewMessage {
                            session_id,
                            role: "assistant".to_string(),
                            content: response.text.clone(),
                            content_type: "text".to_string(),
                            tool_use_id: None,
                            tokens_in: Some(i64::from(response.input_tokens)),
                            tokens_out: Some(i64::from(response.output_tokens)),
                            metadata: Some(serde_json::json!({"task_id": task_id})),
                        })
                        .await;

                    let finished_at = Utc::now();
                    TaskResult {
                        task_id,
                        task_name,
                        success: true,
                        output: Some(response.text),
                        error: None,
                        duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
                        started_at,
                        finished_at,
                    }
                }
                Err(e) => {
                    tracing::warn!("Scheduled task {} failed: {}", task_id, e);
                    let finished_at = Utc::now();
                    TaskResult {
                        task_id,
                        task_name,
                        success: false,
                        output: None,
                        error: Some(e.to_string()),
                        duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
                        started_at,
                        finished_at,
                    }
                }
            }
        })
    });

    Scheduler::new(scheduler_config)
        .with_storage(scheduler_storage)
        .with_executor(executor)
}

/// Initialize common components
pub async fn init_components(
    config: &Config,
) -> anyhow::Result<(Arc<LlmClient>, Arc<ToolRegistry>, Arc<Storage>)> {
    // Get API key - default to Anthropic
    let env_var = match config.llm.provider.as_str() {
        "openai" => "OPENAI_API_KEY",
        "openrouter" => "OPENROUTER_API_KEY",
        _ => "ANTHROPIC_API_KEY", // anthropic or unknown
    };
    
    let api_key = config
        .llm
        .api_key
        .clone()
        .or_else(|| std::env::var(env_var).ok())
        .ok_or_else(|| anyhow::anyhow!(
            "API key required. Run 'nanna init' or set {env_var} environment variable"
        ))?;

    // Create LLM client - default to Anthropic
    let llm = Arc::new(match config.llm.provider.as_str() {
        "openai" => LlmClient::openai(&api_key),
        "openrouter" => LlmClient::openrouter(&api_key),
        provider => {
            if provider != "anthropic" {
                error!("Unknown LLM provider: {provider}, defaulting to anthropic");
            }
            LlmClient::anthropic(&api_key)
        }
    });

    // Validate API key early
    info!("Validating API key...");
    if let Err(e) = llm.validate().await {
        return Err(anyhow::anyhow!("API key validation failed: {e}. Check your config or {env_var} environment variable."));
    }
    info!("API key valid");

    // Create tool registry
    let tools = Arc::new(ToolRegistry::new());

    let workspace = config
        .general
        .workspace
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    // Initialize storage first (needed for memory tools)
    let storage_path = config
        .memory
        .storage_path
        .clone()
        .unwrap_or_else(|| {
            Config::default_data_dir()
                .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default())
                .join("nanna.db")
        });

    let storage_config = StorageConfig {
        path: storage_path.to_string_lossy().to_string(),
    };
    let storage = Arc::new(Storage::new(&storage_config).await?);
    info!("Storage initialized at {}", storage_path.display());

    // Register tools
    tools.register(EchoTool).await;
    tools
        .register(ExecTool::new().with_workdir(workspace.display().to_string()))
        .await;
    tools.register(ReadFileTool::new()).await;
    tools.register(WriteFileTool::new()).await;
    tools.register(ListDirTool::new()).await;
    tools.register(WebFetchTool::new()).await;

    // Web search with Brave API (if configured)
    if let Ok(brave_key) = std::env::var("BRAVE_API_KEY") {
        tools.register(WebSearchTool::new().with_api_key(brave_key)).await;
        info!("Web search enabled (Brave API)");
    }

    // Memory tools backed by Turso with optional embeddings
    let memory_storage: Arc<dyn nanna_tools::MemoryStorage + Send + Sync> = {
        let base = TursoMemoryStorage::new(storage.clone());

        // Try to enable semantic search with OpenAI embeddings
        if let Ok(openai_key) = std::env::var("OPENAI_API_KEY") {
            use nanna_llm::EmbeddingClient;
            let embed_client = Arc::new(EmbeddingClient::openai(&openai_key));
            let embed_fn: nanna_tools::EmbedFn = Arc::new(move |text: String| {
                let client = embed_client.clone();
                Box::pin(async move {
                    client.embed_one(&text).await.map_err(|e| e.to_string())
                })
            });
            info!("Semantic search enabled (OpenAI embeddings)");
            Arc::new(base.with_embeddings(embed_fn, "text-embedding-3-small"))
        } else {
            info!("Semantic search disabled (no OPENAI_API_KEY)");
            Arc::new(base)
        }
    };
    tools.register(RememberTool::new(memory_storage.clone())).await;
    tools.register(RecallTool::new(memory_storage.clone())).await;
    tools.register(ReflectTool::new(memory_storage.clone())).await;

    // Scheduling tools
    let scheduler_state = Arc::new(tokio::sync::RwLock::new(ReminderStore::default()));
    tools.register(RemindTool::new(scheduler_state.clone())).await;
    tools.register(ListRemindersTool::new(scheduler_state.clone())).await;
    tools.register(CancelReminderTool::new(scheduler_state.clone())).await;

    // Curiosity/autonomy tools
    tools.register(ExploreTool).await;
    tools.register(WonderTool).await;
    tools.register(StatusTool).await;

    register_discover_tools(&tools, config).await;

    info!("{} tools ready", tools.definitions().await.len());

    Ok((llm, tools, storage))
}

/// Register the `discover_tools` JS/TS skill with the tool registry.
async fn register_discover_tools(tools: &Arc<ToolRegistry>, config: &Config) {
    let tools_dir = nanna_tools::skills::defaults::resolve_tools_dir(
        config.tools.tools_dir.as_deref()
    );
    if let Some(ref dir) = tools_dir
        && let Some(source) = nanna_tools::skills::defaults::load_discover_tools_source(dir) {
            let wrapper = nanna_tools::skills::ScriptedToolWrapper::from_source("discover_tools", &source)
                .expect("discover_tools skill must parse")
                .with_registry(Arc::downgrade(tools));
            tools.register(wrapper).await;
        }
}
