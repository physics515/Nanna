#![warn(clippy::pedantic, clippy::nursery, clippy::all)]

//! Nanna GUI - Tauri backend
//!
//! IPC bridge between the frontend and nanna-core with agentic tool loop.
//! Includes FSRS-6 cognitive memory and dreaming/consolidation.
//!
//! Supports two modes:
//! - **Daemon mode**: Connects to nanna-daemon via WebSocket
//! - **Embedded mode**: Runs agent directly (fallback when daemon unavailable)

pub mod agents;
pub mod backend;
pub mod daemon_client;
pub mod daemon_manager;
pub mod embedded;
pub mod tool_authoring;
pub mod commands;
pub mod llm;
pub mod state;

use backend::{Backend, BackendMode};

use nanna_config::Config;
use nanna_core::{
    Scheduler, SchedulerConfig, consolidation_task,
    MemoryService, MemoryServiceConfig, ConsolidationConfig,
    // Workspaces
    Workspace, WorkspaceRegistry,
    find_workspace_root, discover_workspaces,
};
use nanna_llm::{CompletionRequest, LlmClient, Message as LlmMessage, ModelInfoCache, RequestBuilder, Role};
use nanna_storage::{Storage, StorageConfig};
use nanna_tools::ToolRegistry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    menu::{MenuBuilder, MenuItemBuilder},
    AppHandle, Emitter, Manager, State,
};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

// =============================================================================
// Channel Routing for Scheduled Tasks
// =============================================================================

use nanna_channels::{Channel, ChannelId, MessageContent, OutgoingMessage};

// Re-export moved items at the crate root so sibling modules and the
// remaining code here keep working with their existing paths.
pub(crate) use commands::chat::*;
pub(crate) use commands::settings::*;
pub(crate) use llm::routing::*;
pub(crate) use state::*;

// =============================================================================
// App Setup
// =============================================================================

async fn setup_state(
    backend: Arc<Backend>,
    mode: BackendMode,
) -> Result<AppState, Box<dyn std::error::Error + Send + Sync>> {
    // Load config
    let config = Config::load().unwrap_or_default().with_env_overrides();

    // Determine database path
    let db_path = Config::default_data_dir()
        .map(|d| d.join("nanna.db").to_string_lossy().to_string())
        .unwrap_or_else(|_| "nanna.db".to_string());

    // Initialize storage (Arc-wrapped for sharing with scheduler). Turso holds
    // an exclusive file lock, so in daemon mode the daemon owns nanna.db and
    // the GUI must not open it.
    let storage = match mode {
        BackendMode::Embedded => {
            let storage_config = StorageConfig { path: db_path };
            Some(Arc::new(Storage::new(&storage_config).await?))
        }
        BackendMode::Daemon => {
            info!("Daemon mode: local storage not opened (the daemon owns nanna.db)");
            None
        }
    };

    // Initialize LLM client (check for OAuth first)
    let llm = match config.llm.provider.as_str() {
        "anthropic" => {
            // Check if OAuth is enabled and has a token
            if config.llm.anthropic_use_oauth {
                if let Some(ref oauth_token) = config.llm.anthropic_oauth_token {
                    info!("Using Anthropic OAuth authentication");
                    LlmClient::anthropic_oauth(oauth_token)
                } else {
                    // OAuth enabled but no token - fall back to API key
                    let api_key = config.llm.api_key.clone()
                        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                        .unwrap_or_else(|| "missing-key".to_string());
                    LlmClient::anthropic(&api_key)
                }
            } else {
                let api_key = config.llm.api_key.clone()
                    .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                    .unwrap_or_else(|| "missing-key".to_string());
                LlmClient::anthropic(&api_key)
            }
        }
        "openai" => {
            let api_key = config.llm.openai_api_key.clone()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .unwrap_or_else(|| "missing-key".to_string());
            LlmClient::openai(&api_key)
        }
        "openrouter" => {
            let api_key = config.llm.openrouter_api_key.clone()
                .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())
                .unwrap_or_else(|| "missing-key".to_string());
            LlmClient::openrouter(&api_key)
        }
        "github" => {
            let api_key = config.llm.github_token.clone()
                .or_else(|| std::env::var("GITHUB_TOKEN").ok())
                .unwrap_or_else(|| "missing-key".to_string());
            LlmClient::github_models(&api_key)
        }
        _ => {
            let api_key = config.llm.api_key.clone()
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                .unwrap_or_else(|| "missing-key".to_string());
            LlmClient::anthropic(&api_key)
        }
    };
    let llm = Arc::new(llm);

    // Set env vars from config (so they're available for model fetching and API calls)
    if let Some(ref key) = config.llm.openai_api_key {
        unsafe { std::env::set_var("OPENAI_API_KEY", key); }
    }
    if let Some(ref key) = config.llm.openrouter_api_key {
        unsafe { std::env::set_var("OPENROUTER_API_KEY", key); }
    }
    if let Some(ref key) = config.llm.github_token {
        unsafe { std::env::set_var("GITHUB_TOKEN", key); }
    }

    // Initialize tools
    let tools = ToolRegistry::new();

    // Register built-in tools
    tools.register(nanna_tools::ReadFileTool::new()).await;
    tools.register(nanna_tools::WriteFileTool::new()).await;
    tools.register(nanna_tools::ListDirTool::new()).await;
    tools.register(nanna_tools::ExecTool::new()).await;
    tools.register(nanna_tools::WebFetchTool::new()).await;

    // WebSearchTool requires BRAVE_API_KEY (env var or config)
    let brave_key = std::env::var("BRAVE_API_KEY").ok()
        .or_else(|| config.tools.brave_api_key.clone());
    let web_search = if let Some(key) = brave_key {
        // Set env var so it's available for later checks
        unsafe { std::env::set_var("BRAVE_API_KEY", &key); }
        nanna_tools::WebSearchTool::new().with_api_key(key)
    } else {
        info!("BRAVE_API_KEY not set - web_search will be unavailable, use web_fetch instead");
        nanna_tools::WebSearchTool::new()
    };
    tools.register(web_search).await;
    tools.register(nanna_tools::EchoTool).await;

    // Register common aliases for Claude Code compatibility
    // Claude Code uses: read, Write, bash, glob, etc.
    tools.register_alias("read", "read_file").await;
    tools.register_alias("Read", "read_file").await;
    tools.register_alias("write", "write_file").await;
    tools.register_alias("Write", "write_file").await;
    tools.register_alias("bash", "exec").await;
    tools.register_alias("Bash", "exec").await;
    tools.register_alias("glob", "list_dir").await;
    tools.register_alias("Glob", "list_dir").await;
    tools.register_alias("ls", "list_dir").await;
    info!("Registered Claude Code tool aliases (read, write, bash, glob)");

    // Initialize FSRS-6 cognitive memory service
    // Load embedding config from saved config file
    let saved_embedding_provider = config.memory.embedding_provider.clone();
    let saved_embedding_model = config.memory.embedding_model.clone();
    let saved_ollama_host = config.memory.ollama_host.clone();

    // Get API keys
    let openai_key = std::env::var("OPENAI_API_KEY").ok()
        .or_else(|| config.llm.openai_api_key.clone());

    info!("Loaded embedding config: provider={}, model={}, ollama_host={}",
          saved_embedding_provider, saved_embedding_model, saved_ollama_host);

    // Initialize based on configured provider
    let (embedding_provider, embedding_model, embedding_enabled, memory) =
        match saved_embedding_provider.as_str() {
            "openai" => {
                if let Some(openai_key) = openai_key {
                    unsafe { std::env::set_var("OPENAI_API_KEY", &openai_key); }
                    info!("Using OpenAI embeddings with model: {}", saved_embedding_model);

                    // Get dimension from model info cache/API
                    let embed_llm = LlmClient::openai(&openai_key);
                    let cache = ModelInfoCache::default_location();
                    let model_info = embed_llm.get_model_info(&saved_embedding_model, cache.as_ref()).await;
                    let dimension = match model_info.embedding_dimension {
                        Some(dimension) => dimension,
                        None => nanna_llm::EmbeddingClient::openai(&openai_key)
                            .with_model(&saved_embedding_model)
                            .embed_one("dimension probe").await
                            .map_err(|e| format!("Failed to discover embedding dimension: {e}"))?.len(),
                    };
                    info!("Embedding dimension: {} for model {} (from cache/API)", dimension, saved_embedding_model);

                    let memory_config = MemoryServiceConfig {
                        dimension,
                        ..Default::default()
                    };

                    let embed_client = reqwest::Client::new();
                    let embed_key = openai_key.clone();
                    let model_name = saved_embedding_model.clone();

                    let embed_fn: nanna_memory::EmbedFn = Arc::new(move |text: &str| {
                        let client = embed_client.clone();
                        let key = embed_key.clone();
                        let model = model_name.clone();
                        let text = text.to_string();

                        Box::pin(async move {
                            let response = client
                                .post("https://api.openai.com/v1/embeddings")
                                .header("Authorization", format!("Bearer {}", key))
                                .json(&serde_json::json!({
                                    "model": model,
                                    "input": text
                                }))
                                .send()
                                .await
                                .map_err(|e| e.to_string())?;

                            let json: serde_json::Value = response
                                .json()
                                .await
                                .map_err(|e| e.to_string())?;

                            let embedding = json["data"][0]["embedding"]
                                .as_array()
                                .ok_or("No embedding in response")?
                                .iter()
                                .filter_map(|v| v.as_f64().map(|f| f as f32))
                                .collect::<Vec<f32>>();

                            if embedding.is_empty() {
                                return Err("Empty embedding returned".to_string());
                            }

                            Ok(embedding)
                        })
                    });

                    (
                        "openai".to_string(),
                        saved_embedding_model.clone(),
                        true,
                        MemoryService::new(memory_config).with_embed_fn(embed_fn),
                    )
                } else {
                    info!("OpenAI embeddings configured but no API key - disabling");
                    (
                        "disabled".to_string(),
                        "none".to_string(),
                        false,
                        MemoryService::new(MemoryServiceConfig::default()),
                    )
                }
            }
            "ollama" => {
                let ollama_url = saved_ollama_host.clone();
                info!("Using Ollama embeddings at {} with model: {}", ollama_url, saved_embedding_model);

                // Get dimension from model info cache/API (Ollama /api/show endpoint)
                let embed_llm = LlmClient::ollama(&ollama_url);
                let cache = ModelInfoCache::default_location();
                let model_info = embed_llm.get_model_info(&saved_embedding_model, cache.as_ref()).await;
                let dimension = match model_info.embedding_dimension {
                    Some(dimension) => dimension,
                    None => nanna_llm::EmbeddingClient::ollama(&ollama_url)
                        .with_model(&saved_embedding_model)
                        .embed_one("dimension probe").await
                        .map_err(|e| format!("Failed to discover embedding dimension: {e}"))?.len(),
                };
                info!("Embedding dimension: {} for model {} (from cache/API)", dimension, saved_embedding_model);

                let memory_config = MemoryServiceConfig {
                    dimension,
                    ..Default::default()
                };

                let embed_client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(60))
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new());

                let model_name = saved_embedding_model.clone();
                let embed_fn: nanna_memory::EmbedFn = Arc::new(move |text: &str| {
                    let client = embed_client.clone();
                    let url = ollama_url.clone();
                    let model = model_name.clone();
                    let text = text.to_string();

                    Box::pin(async move {
                        let response = client
                            .post(format!("{}/api/embeddings", url))
                            .header("Content-Type", "application/json")
                            .json(&serde_json::json!({
                                "model": model,
                                "prompt": text
                            }))
                            .send()
                            .await
                            .map_err(|e| e.to_string())?;

                        if !response.status().is_success() {
                            let status = response.status();
                            let body = response.text().await.unwrap_or_default();
                            return Err(format!("Ollama error {}: {}", status, body));
                        }

                        let json: serde_json::Value = response
                            .json()
                            .await
                            .map_err(|e| e.to_string())?;

                        let embedding = json["embedding"]
                            .as_array()
                            .ok_or("No embedding in Ollama response")?
                            .iter()
                            .filter_map(|v| v.as_f64().map(|f| f as f32))
                            .collect::<Vec<f32>>();

                        if embedding.is_empty() {
                            return Err("Empty embedding returned from Ollama".to_string());
                        }

                        Ok(embedding)
                    })
                });

                (
                    "ollama".to_string(),
                    saved_embedding_model.clone(),
                    true,
                    MemoryService::new(memory_config).with_embed_fn(embed_fn),
                )
            }
            _ => {
                info!("Embedding provider disabled");
                (
                    "disabled".to_string(),
                    "none".to_string(),
                    false,
                    MemoryService::new(MemoryServiceConfig::default()),
                )
            }
        };
    let memory = Arc::new(memory);

    // Load persisted memories if they exist
    let memory_path = Config::default_data_dir()
        .map(|d| d.join("memories.json"))
        .unwrap_or_else(|_| std::path::PathBuf::from("memories.json"));

    if memory_path.exists() {
        match memory.load(&memory_path).await {
            Ok(()) => info!("Loaded {} memories from {:?}", memory.count().await, memory_path),
            Err(e) => warn!("Failed to load memories (starting fresh): {}", e),
        }
    } else {
        info!("No saved memories found at {:?} (starting fresh)", memory_path);
    }

    // Shared workspace registry — constructed early so memory tools can
    // thread a live handle into the adapter and scope remembers/recalls to
    // the *current* active workspace. Later AppState construction reuses this
    // exact Arc so set/clear_active_workspace keep the adapter in step.
    let workspaces: Arc<RwLock<WorkspaceRegistry>> = {
        let mut registry = WorkspaceRegistry::new();
        if let Some(ref storage) = storage {
            match storage.workspaces().list().await {
                Ok(records) if !records.is_empty() => {
                    let mut active_id = None;
                    for record in &records {
                        let path = std::path::PathBuf::from(&record.path);
                        if path.exists() {
                            let mut ws = Workspace::new(&path);
                            ws.id = record.id.clone();
                            if let Err(e) = ws.load_context().await {
                                warn!(
                                    "Failed to load workspace context for {}: {}",
                                    record.path, e
                                );
                            }
                            registry.register(ws);
                            if record.active {
                                active_id = Some(record.id.clone());
                            }
                        } else {
                            warn!(
                                "Persisted workspace path no longer exists: {}",
                                record.path
                            );
                        }
                    }
                    if let Some(id) = active_id {
                        registry.set_active(&id);
                    }
                    info!("Restored {} workspaces from database", records.len());
                }
                Ok(_) => {}
                Err(e) => {
                    warn!("Failed to load workspaces from database: {}", e);
                }
            }
        } else {
            match backend.workspace_list().await {
                Ok(result) => {
                    let records = result
                        .get("workspaces")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    let active_id = result
                        .get("active_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let count = records.len();
                    for record in &records {
                        let (Some(id), Some(path)) = (
                            record.get("id").and_then(|v| v.as_str()),
                            record.get("path").and_then(|v| v.as_str()),
                        ) else {
                            continue;
                        };
                        let path = std::path::PathBuf::from(path);
                        if path.exists() {
                            let mut ws = Workspace::new(&path);
                            ws.id = id.to_string();
                            if let Err(e) = ws.load_context().await {
                                warn!(
                                    "Failed to load workspace context for {:?}: {}",
                                    path, e
                                );
                            }
                            registry.register(ws);
                        } else {
                            warn!("Daemon workspace path no longer exists: {:?}", path);
                        }
                    }
                    if let Some(id) = active_id {
                        registry.set_active(&id);
                    }
                    if count > 0 {
                        info!("Restored {} workspaces from daemon", count);
                    }
                }
                Err(e) => {
                    warn!("Failed to load workspaces from daemon: {}", e);
                }
            }
        }
        Arc::new(RwLock::new(registry))
    };

    // Register memory tools (remember, recall, reflect) with the FSRS memory service
    let memory_storage: nanna_tools::StorageHandle = Arc::new(MemoryServiceAdapter::new(memory.clone(), workspaces.clone()));
    tools.register(nanna_tools::RememberTool::new(memory_storage.clone())).await;
    tools.register(nanna_tools::RecallTool::new(memory_storage.clone())).await;
    tools.register(nanna_tools::ReflectTool::new(memory_storage)).await;
    info!("Registered memory tools (remember, recall, reflect)");

    // Initialize user tool authoring system
    let user_tools_dir = Config::default_data_dir()
        .map(|d| d.join("user_tools"))
        .unwrap_or_else(|_| std::path::PathBuf::from("user_tools"));
    let user_tool_manager = Arc::new(tool_authoring::UserToolManager::new(user_tools_dir));

    // Load existing user tools
    match user_tool_manager.load_all().await {
        Ok(count) => info!("Loaded {} user-created tools", count),
        Err(e) => warn!("Failed to load user tools: {}", e),
    }

    // Register user tools with the registry
    let tools = Arc::new(tools);
    let registered = user_tool_manager.register_with_registry(&tools).await;
    info!("Registered {} user tools with the tool registry", registered);

    // Register create_tool and list_user_tools tools (so Nanna can create tools at runtime)
    tools.register(tool_authoring::CreateToolTool::new(user_tool_manager.clone(), tools.clone())).await;
    tools.register(tool_authoring::ListUserToolsTool::new(user_tool_manager.clone())).await;
    info!("Registered tool authoring tools (create_tool, list_user_tools)");

    // Register discover_tools (JS/TS skill with registry access)
    {
        let tools_dir = nanna_tools::skills::defaults::resolve_tools_dir(
            config.tools.tools_dir.as_deref()
        );
        if let Some(ref dir) = tools_dir {
            if let Some(source) = nanna_tools::skills::defaults::load_discover_tools_source(dir) {
                let wrapper = nanna_tools::skills::ScriptedToolWrapper::from_source("discover_tools", &source)
                    .expect("discover_tools skill must parse")
                    .with_registry(std::sync::Arc::downgrade(&tools));
                tools.register(wrapper).await;
                info!("Registered discover_tools skill from {:?}", dir);
            } else {
                warn!("discover_tools not found in tools directory");
            }
        }
    }

    // Initialize scheduler with consolidation task
    let scheduler_config = SchedulerConfig {
        heartbeat_interval: Duration::from_secs(1800), // 30 minutes
        heartbeat_enabled: true, // Enable heartbeats for autonomous operation
        heartbeat_prompt: "Read HEARTBEAT.md if it exists (workspace context). Follow it strictly. Do not infer or repeat old tasks from prior chats. If nothing needs attention, reply HEARTBEAT_OK.".to_string(),
        max_concurrent: 4,
        check_interval: Duration::from_secs(30),
        default_timezone: "UTC".to_string(),
    };
    let mut scheduler = Scheduler::new(scheduler_config);

    // Cron persistence needs the local DB; in daemon mode the scheduler still
    // runs (the daemon has no cron runner yet) but jobs live in memory only.
    if let Some(ref storage) = storage {
        scheduler = scheduler.with_storage(Arc::clone(storage));

        // Load persisted cron jobs from storage
        match scheduler.load_jobs().await {
            Ok(count) => info!("Loaded {} cron jobs from database", count),
            Err(e) => warn!("Failed to load cron jobs: {}", e),
        }
    } else {
        info!("Daemon mode: scheduler running without cron persistence");
    }

    // Deduplicate consolidation tasks (fix for historical duplicates)
    let deduped = scheduler.deduplicate_by_name("memory_consolidation").await;
    if deduped > 0 {
        info!("Removed {} duplicate consolidation tasks", deduped);
    }

    // Only add consolidation task if one doesn't already exist
    if !scheduler.has_task_named("memory_consolidation").await {
        let consolidation = consolidation_task(Some(Duration::from_secs(3600)));
        scheduler.add_task(consolidation).await;
        info!("Scheduled memory consolidation task (every 1 hour)");
    } else {
        info!("Memory consolidation task already scheduled");
    }

    // Create executor for scheduled tasks
    let memory_for_executor = memory.clone();
    let tools_for_executor = tools.clone();
    let channels_for_executor = config.channels.clone();
    let config_for_executor = config.clone();
    let ollama_host_for_executor = saved_ollama_host.clone();

    let executor: nanna_core::TaskExecutor = Arc::new(move |task| {
        let memory = memory_for_executor.clone();
        let tools = tools_for_executor.clone();
        let channels = channels_for_executor.clone();
        let config = config_for_executor.clone();
        let ollama_host = ollama_host_for_executor.clone();

        Box::pin(async move {
            let start = std::time::Instant::now();
            let started_at = chrono::Utc::now();

            match task.name.as_str() {
                "heartbeat" => {
                    info!("Running heartbeat...");

                    // Build the heartbeat prompt with context
                    let prompt = task.payload.clone();

                    // Get the first available model from priority list
                    let priority = &config.llm.model_priority;
                    let (llm, model) = if let Some(model_id) = priority.first() {
                        if let Some((client, actual_model)) = create_llm_client_for_model(model_id, &config, &ollama_host) {
                            (Arc::new(client), actual_model)
                        } else {
                            // Fallback to default model
                            let api_key = config.llm.api_key.clone()
                                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                                .unwrap_or_default();
                            (Arc::new(LlmClient::anthropic(&api_key)), config.llm.model.clone())
                        }
                    } else {
                        // No priority list, use default
                        let api_key = config.llm.api_key.clone()
                            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                            .unwrap_or_default();
                        (Arc::new(LlmClient::anthropic(&api_key)), config.llm.model.clone())
                    };

                    // Create agent config
                    let agent_config = nanna_agent::AgentConfig {
                        model: model.clone(),
                        max_iterations: Some(5), // Limit turns for heartbeat
                        ..Default::default()
                    };

                    // Create agent and run
                    let agent = nanna_agent::Agent::new(
                        agent_config,
                        llm.clone(),
                        tools.clone(),
                    );

                    // Run the agent with the heartbeat prompt
                    let run_options = nanna_agent::RunOptions::default();
                    match agent.run(&prompt, run_options).await {
                        Ok(response) => {
                            let finished_at = chrono::Utc::now();
                            let output = response.text.clone();

                            // Check if agent responded with HEARTBEAT_OK
                            let is_heartbeat_ok = output.trim().starts_with("HEARTBEAT_OK")
                                || output.trim().ends_with("HEARTBEAT_OK")
                                || output.trim() == "HEARTBEAT_OK";

                            if is_heartbeat_ok {
                                debug!("Heartbeat: OK (nothing to do)");
                            } else {
                                info!("Heartbeat response: {}", output.chars().take(200).collect::<String>());

                                // Route to channel if specified
                                if let Some(ref channel_id) = task.target_channel {
                                    if let Err(e) = route_to_channel(&channels, channel_id, &output).await {
                                        warn!("Failed to route heartbeat to channel {}: {}", channel_id, e);
                                    }
                                }
                            }

                            nanna_core::TaskResult {
                                task_id: task.id.clone(),
                                task_name: task.name.clone(),
                                success: true,
                                output: Some(if is_heartbeat_ok {
                                    "HEARTBEAT_OK".to_string()
                                } else {
                                    output
                                }),
                                error: None,
                                duration_ms: start.elapsed().as_millis() as u64,
                                started_at,
                                finished_at,
                            }
                        }
                        Err(e) => {
                            let finished_at = chrono::Utc::now();
                            error!("Heartbeat failed: {}", e);
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
                }
                "memory_consolidation" => {
                    info!("Running scheduled memory consolidation...");

                    // Get the first available model from priority list for summarization
                    let priority = &config.llm.model_priority;
                    let llm_for_consolidation = if let Some(model_id) = priority.first() {
                        if let Some((client, _)) = create_llm_client_for_model(model_id, &config, &ollama_host) {
                            Arc::new(client)
                        } else {
                            let api_key = config.llm.api_key.clone()
                                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                                .unwrap_or_default();
                            Arc::new(LlmClient::anthropic(&api_key))
                        }
                    } else {
                        let api_key = config.llm.api_key.clone()
                            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                            .unwrap_or_default();
                        Arc::new(LlmClient::anthropic(&api_key))
                    };

                    let consolidation_config = ConsolidationConfig::default();
                    let summarize = |prompt: String| {
                        let llm = llm_for_consolidation.clone();
                        async move {
                            let request = nanna_llm::CompletionRequest::default()
                                .with_model("claude-3-5-haiku-20241022")
                                .with_message(nanna_llm::Message::user(&prompt));
                            llm.complete(&request).await.map_err(|e| e.to_string())
                        }
                    };

                    match memory.consolidate(&consolidation_config, summarize).await {
                        Ok(result) => {
                            let finished_at = chrono::Utc::now();
                            info!(
                                "Scheduled consolidation: {} processed, {} merged",
                                result.memories_processed, result.memories_merged
                            );
                            nanna_core::TaskResult {
                                task_id: task.id.clone(),
                                task_name: task.name.clone(),
                                success: true,
                                output: Some(format!("Processed {} memories", result.memories_processed)),
                                error: None,
                                duration_ms: start.elapsed().as_millis() as u64,
                                started_at,
                                finished_at,
                            }
                        }
                        Err(e) => {
                            let finished_at = chrono::Utc::now();
                            error!("Scheduled consolidation failed: {}", e);
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
                }
                _ => {
                    // Generic cron job - run as agent prompt
                    if !task.payload.is_empty() {
                        info!("Running cron job '{}': {}", task.name, task.payload.chars().take(50).collect::<String>());

                        // Get the first available model from priority list
                        let priority = &config.llm.model_priority;
                        let (llm, model) = if let Some(model_id) = priority.first() {
                            if let Some((client, actual_model)) = create_llm_client_for_model(model_id, &config, &ollama_host) {
                                (Arc::new(client), actual_model)
                            } else {
                                let api_key = config.llm.api_key.clone()
                                    .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                                    .unwrap_or_default();
                                (Arc::new(LlmClient::anthropic(&api_key)), config.llm.model.clone())
                            }
                        } else {
                            let api_key = config.llm.api_key.clone()
                                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                                .unwrap_or_default();
                            (Arc::new(LlmClient::anthropic(&api_key)), config.llm.model.clone())
                        };

                        let agent_config = nanna_agent::AgentConfig {
                            model: model.clone(),
                            max_iterations: Some(10),
                            ..Default::default()
                        };

                        let agent = nanna_agent::Agent::new(
                            agent_config,
                            llm.clone(),
                            tools.clone(),
                        );

                        let run_options = nanna_agent::RunOptions::default();
                        match agent.run(&task.payload, run_options).await {
                            Ok(response) => {
                                let finished_at = chrono::Utc::now();
                                let output = response.text;
                                info!("Cron job '{}' completed", task.name);

                                // Route to channel if specified
                                if let Some(ref channel_id) = task.target_channel {
                                    if let Err(e) = route_to_channel(&channels, channel_id, &output).await {
                                        warn!("Failed to route cron job to channel {}: {}", channel_id, e);
                                    }
                                }

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
                                let finished_at = chrono::Utc::now();
                                error!("Cron job '{}' failed: {}", task.name, e);
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
                        let finished_at = chrono::Utc::now();
                        debug!("Skipping task with empty payload: {}", task.name);
                        nanna_core::TaskResult {
                            task_id: task.id.clone(),
                            task_name: task.name.clone(),
                            success: true,
                            output: Some("Skipped (empty payload)".to_string()),
                            error: None,
                            duration_ms: start.elapsed().as_millis() as u64,
                            started_at,
                            finished_at,
                        }
                    }
                }
            }
        })
    });

    scheduler = scheduler.with_executor(executor);
    match mode {
        BackendMode::Embedded => {
            scheduler.start();
            info!("Scheduler started with consolidation executor");
        }
        BackendMode::Daemon => {
            // The daemon runs heartbeat + cron (it owns nanna.db, so the
            // persisted jobs live there); starting a second scheduler here
            // would double-fire every task. Cron commands route over IPC.
            info!("Daemon mode: local scheduler not started (the daemon runs heartbeat + cron)");
        }
    }

    let scheduler = Arc::new(RwLock::new(scheduler));
    let last_consolidation = Arc::new(RwLock::new(None));

    info!("Nanna GUI initialized with model: {}", config.llm.model);
    info!("Registered {} tools", tools.definitions().await.len());
    info!("FSRS-6 cognitive memory enabled");

    // Get extraction model from config (empty = use chat model)
    let saved_extraction_model = config.memory.extraction_model.clone();

    // Get initial active model from priority list or default
    let initial_active_model = config.llm.model_priority.first()
        .cloned()
        .unwrap_or_else(|| config.llm.model.clone());

    // Embedded mode: construct the long-lived in-process AgentService (the
    // daemon's agent loop running inside the GUI). Its events are bridged onto
    // the same DaemonEvent bus the backend already forwards to Tauri, so both
    // modes share one loop and one event pipeline. Only possible when the GUI
    // owns local storage (i.e. embedded mode).
    let agent_service = if let Some(ref storage) = storage {
        let service = embedded::build_embedded_agent_service(
            &config,
            &saved_ollama_host,
            tools.clone(),
            memory.clone(),
            storage.clone(),
            backend.daemon_event_sender(),
        )
        .await;
        if service.is_some() {
            info!("Embedded agent service ready (GUI owns local storage)");
        }
        service
    } else {
        info!("Daemon mode: in-process agent service not constructed");
        None
    };

    Ok(AppState {
        storage: storage.clone(),
        llm,
        tools,
        config,
        memory,
        memory_path,
        scheduler,
        last_consolidation,
        // Runtime settings - all enabled by default
        dreaming_enabled: Arc::new(RwLock::new(true)),
        scheduler_enabled: Arc::new(RwLock::new(true)),
        heartbeat_enabled: Arc::new(RwLock::new(true)),
        heartbeat_interval_seconds: Arc::new(RwLock::new(300)), // 5 minutes
        // Embedding settings (loaded from config)
        embedding_provider: Arc::new(RwLock::new(embedding_provider)),
        embedding_model: Arc::new(RwLock::new(embedding_model)),
        embedding_enabled: Arc::new(RwLock::new(embedding_enabled)),
        // Ollama host (from config)
        ollama_host: Arc::new(RwLock::new(saved_ollama_host)),
        // Extraction model (from config, empty = use chat model)
        extraction_model: Arc::new(RwLock::new(saved_extraction_model)),
        // Active model tracking (start with first in priority or default model)
        active_model: Arc::new(RwLock::new(initial_active_model)),
        // Rate limited models (empty at startup)
        rate_limited_models: Arc::new(RwLock::new(HashMap::new())),
        // Workspace registry — shared Arc constructed earlier so memory tools
        // observe the same active workspace as the GUI commands.
        workspaces,
        // User tool authoring manager
        user_tools: user_tool_manager,
        // Backend (initialized above with embedded mode)
        backend,
        // Close behavior (default: ask user)
        close_mode: Arc::new(RwLock::new(CloseMode::default())),
        // In-process agent service (embedded mode only)
        agent_service,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("nanna=info".parse().unwrap()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle().clone();

            // Set up system tray
            setup_system_tray(app)?;

            // Initialize state asynchronously
            tauri::async_runtime::spawn(async move {
                // DAEMON-FIRST: decide the backend mode BEFORE opening any local
                // storage. Turso holds an exclusive file lock on nanna.db, so only
                // one process may own it — if the GUI opens it first, the daemon
                // sidecar boots storage-less (no sessions, no memory persistence).
                // The daemon is the preferred owner; the GUI only opens storage
                // itself when falling back to embedded mode.
                let backend = Arc::new(Backend::new());
                let mode = backend.init(&handle).await;
                info!("Backend initialized in {:?} mode", mode);

                match setup_state(backend, mode).await {
                    Ok(state) => {
                        // Create agent registry with shared LLM and tools
                        let agent_registry = agents::AgentRegistryState::new(
                            state.llm.clone(),
                            state.tools.clone(),
                        );

                        // Manage both states
                        handle.manage(Arc::new(RwLock::new(state)));
                        handle.manage(Arc::new(RwLock::new(agent_registry)));
                        info!("App state initialized successfully");
                    }
                    Err(e) => {
                        error!("Failed to initialize app state: {}", e);
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::chat::send_message,
            commands::sessions::create_session,
            commands::sessions::list_sessions,
            commands::sessions::get_session_history,
            commands::sessions::delete_session,
            commands::sessions::clear_all_sessions,
            commands::sessions::archive_and_delete_session,
            commands::sessions::rename_session,
            commands::sessions::set_session_workspace,
            commands::settings::get_config,
            commands::settings::set_model,
            commands::settings::set_api_key,
            commands::memory::search_memory,
            commands::memory::get_memory_stats,
            commands::system::show_window,
            commands::system::hide_to_tray,
            commands::settings::get_extended_settings,
            commands::settings::set_provider_api_key,
            commands::settings::set_provider,
            // Anthropic OAuth (via claude setup-token)
            commands::settings::run_claude_setup_token,
            commands::settings::import_claude_code_credentials,
            commands::settings::save_anthropic_oauth_token,
            commands::settings::logout_anthropic_oauth,
            commands::settings::get_credential_status,
            commands::settings::refresh_oauth_token,
            commands::settings::check_env_var,
            // Cognitive memory (FSRS-6 + dreaming)
            commands::memory::get_cognitive_memory_stats,
            commands::memory::trigger_consolidation,
            commands::memory::apply_memory_updates,
            // Memory & scheduling settings
            commands::memory::set_dreaming_enabled,
            commands::memory::set_max_compression_ratio,
            commands::memory::set_min_remaining_memories,
            commands::scheduler::set_scheduler_enabled,
            commands::scheduler::set_heartbeat_enabled,
            commands::scheduler::set_heartbeat_interval,
            commands::settings::set_extraction_model,
            // Embedding configuration
            commands::settings::set_embedding_config,
            commands::settings::get_ollama_models,
            commands::settings::set_ollama_host,
            commands::settings::set_ollama_api_key,
            // Dynamic model fetching
            commands::settings::get_anthropic_models,
            commands::settings::get_openai_models,
            commands::settings::get_openrouter_models,
            commands::settings::get_openrouter_embedding_models,
            commands::settings::get_github_models,
            commands::settings::get_claude_proxy_models,
            commands::settings::set_claude_proxy,
            commands::settings::check_claude_proxy_health,
            // Memory persistence
            commands::memory::save_memories,
            // Memory management
            commands::memory::list_memories,
            commands::memory::get_memory,
            commands::memory::delete_memory,
            commands::memory::update_memory,
            commands::memory::clear_all_memories,
            // Channel status
            commands::channels::get_channel_status,
            commands::channels::get_enhanced_channel_status,
            commands::channels::test_all_channels,
            commands::channels::subscribe_channel_status,
            commands::channels::unsubscribe_channel_status,
            // Config persistence
            commands::settings::save_config,
            commands::channels::save_channel_config,
            commands::channels::test_channel_connection,
            // Notifications
            commands::system::send_notification,
            commands::system::request_notification_permission,
            commands::system::check_notification_permission,
            // Similarity threshold
            commands::memory::get_similarity_threshold,
            commands::memory::set_similarity_threshold,
            // System prompt & agent settings
            commands::settings::get_system_prompt,
            commands::settings::set_system_prompt,
            commands::settings::set_agent_name,
            commands::settings::set_personality_mode,
            commands::settings::set_thinking_enabled,
            commands::settings::set_streaming_enabled,
            commands::settings::set_max_tokens,
            commands::settings::set_agent_iteration_policy,
            // Config import/export
            commands::settings::export_config,
            commands::settings::import_config,
            // Model priority (fallback chains)
            commands::settings::get_chat_model_priority,
            commands::settings::set_chat_model_priority,
            commands::settings::get_embedding_model_priority,
            commands::settings::set_embedding_model_priority,
            commands::settings::get_summarization_model_priority,
            commands::settings::set_summarization_model_priority,
            // OCR settings
            commands::settings::get_ocr_model_priority,
            commands::settings::set_ocr_model_priority,
            commands::settings::get_use_embedded_ocr,
            commands::settings::set_use_embedded_ocr,
            // Model routing
            commands::settings::get_model_routing,
            commands::settings::set_model_routing,
            commands::settings::get_routing_first_turn_primary,
            commands::settings::set_routing_first_turn_primary,
            commands::settings::get_sub_agent_model,
            commands::settings::set_sub_agent_model,
            // Model status
            commands::system::get_model_status,
            commands::system::get_model_stats,
            commands::system::get_tool_stats,
            commands::system::get_global_stats,
            commands::system::get_tool_stats_hourly,
            commands::system::get_tool_stats_daily,
            commands::system::get_tool_call_log,
            commands::sessions::spawn_sub_session,
            commands::sessions::list_sub_sessions,
            commands::sessions::kill_sub_session,
            commands::sessions::get_sub_session_status,
            commands::sessions::send_to_sub_session,
            commands::system::clear_rate_limit,
            // Workspaces
            commands::workspaces::list_workspaces,
            commands::workspaces::open_workspace,
            commands::workspaces::set_active_workspace,
            commands::workspaces::clear_active_workspace,
            commands::workspaces::get_active_workspace,
            commands::workspaces::get_workspace_context,
            commands::workspaces::reload_workspace,
            commands::workspaces::close_workspace,
            commands::workspaces::discover_workspaces_in_path,
            commands::workspaces::find_workspace_root_from_path,
            commands::workspaces::save_workspace_file,
            commands::workspaces::append_workspace_memory,
            commands::workspaces::get_workspace_recent_memory,
            commands::workspaces::list_workspace_memory_files,
            commands::workspaces::init_workspace,
            commands::workspaces::read_workspace_file,
            commands::workspaces::check_workspace_validity,
            // Agent visualization
            agents::get_agent_clusters,
            agents::get_all_agents,
            agents::get_agent,
            agents::get_agent_children,
            agents::get_agent_stats,
            agents::cancel_agent,
            agents::subscribe_agent_events,
            agents::cleanup_completed_agents,
            agents::get_workspace_agents,
            // User tool authoring
            commands::tools::list_user_tools_cmd,
            commands::tools::get_user_tool,
            commands::tools::get_tool_source,
            commands::tools::create_user_tool,
            commands::tools::update_user_tool,
            commands::tools::delete_user_tool,
            commands::tools::test_user_tool,
            // All registered tools
            commands::tools::list_tools,
            commands::tools::get_tool,
            // Skill directory tools
            commands::tools::list_skills,
            commands::tools::create_skill,
            commands::tools::update_skill,
            commands::tools::delete_skill,
            commands::tools::test_skill,
            // Backend mode
            commands::system::get_backend_status,
            commands::system::init_backend,
            commands::sessions::get_session_run_state,
            // Cancellation & Logs
            commands::sessions::cancel_session,
            commands::system::get_daemon_logs,
            // Window close behavior
            commands::system::get_close_mode,
            commands::system::set_close_mode,
            commands::system::handle_window_close,
            commands::system::perform_quit,
            // Scheduler / Cron jobs
            commands::scheduler::list_cron_jobs,
            commands::scheduler::create_cron_job,
            commands::scheduler::update_cron_job,
            commands::scheduler::set_cron_job_enabled,
            commands::scheduler::delete_cron_job,
            commands::scheduler::delete_cron_jobs_by_name,
            commands::scheduler::run_cron_job_now,
            commands::scheduler::get_cron_job_history,
            commands::scheduler::validate_cron_expression,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                // Shutdown backend (stop daemon sidecar) and save memories
                if let Some(state) = app.try_state::<Arc<RwLock<AppState>>>() {
                    let state = state.inner().clone();
                    tauri::async_runtime::block_on(async {
                        let state_guard = state.read().await;

                        // Stop the daemon sidecar
                        info!("Shutting down backend...");
                        state_guard.backend.shutdown().await;

                        let count = state_guard.memory.count().await;

                        // Only save if we have memories (prevents wiping on failed load)
                        if count > 0 {
                            // Create backup before saving
                            let backup_path = state_guard.memory_path.with_extension("json.bak");
                            if state_guard.memory_path.exists() {
                                if let Err(e) = std::fs::copy(&state_guard.memory_path, &backup_path) {
                                    warn!("Failed to create memory backup: {}", e);
                                }
                            }

                            if let Err(e) = state_guard.memory.save(&state_guard.memory_path).await {
                                error!("Failed to save memories on exit: {}", e);
                            } else {
                                info!("Saved {} memories to {:?}", count, state_guard.memory_path);
                            }
                        } else {
                            info!("No memories to save (count=0), skipping to preserve existing file");
                        }
                    });
                }
            }
        });
}

/// Set up the system tray icon and menu
fn setup_system_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show_item = MenuItemBuilder::with_id("show", "Show Nanna").build(app)?;
    let new_chat_item = MenuItemBuilder::with_id("new_chat", "New Chat").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&show_item)
        .item(&new_chat_item)
        .separator()
        .item(&quit_item)
        .build()?;

    let _tray = TrayIconBuilder::with_id("main")
        .tooltip("Nanna AI Assistant")
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| {
            match event.id().as_ref() {
                "show" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                "new_chat" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                        // Emit event to create new chat
                        let _ = app.emit("tray-new-chat", ());
                    }
                }
                "quit" => {
                    app.exit(0);
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;

    info!("System tray initialized");
    Ok(())
}
