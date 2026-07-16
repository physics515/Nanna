//! `server` command and the legacy daemon-mode entry point.

use crate::setup::{create_scheduler, init_components};
use nanna_config::Config;
use nanna_core::{LlmClient, Nanna, NannaConfig};
use nanna_server::{start_server, AppStateBuilder, ServerConfig};
use tracing::{debug, info, warn};

/// Run the HTTP server
pub(crate) async fn run_server(config: &Config, host: String, port: u16) -> anyhow::Result<()> {
    let (llm, tools, storage) = init_components(config).await?;

    // Get API key for bot - default to Anthropic
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
        .ok_or_else(|| anyhow::anyhow!("API key not found"))?;

    // Create Nanna bot instance for backwards compatibility
    let bot_config = NannaConfig {
        name: config.general.name.clone(),
        default_model: config.llm.model.clone(),
        max_context_messages: 20,
        enable_gpu: true,
    };

    let bot_llm = match config.llm.provider.as_str() {
        "openai" => LlmClient::openai(&api_key),
        "openrouter" => LlmClient::openrouter(&api_key),
        _ => LlmClient::anthropic(&api_key), // anthropic or unknown
    };

    let bot = Nanna::new(bot_config, bot_llm).await?;

    if bot.has_gpu() {
        info!("GPU acceleration enabled");
    } else {
        info!("CPU-only mode (SIMD active)");
    }

    // Get Telegram token from config or environment
    let telegram_token = config
        .channels
        .telegram
        .as_ref()
        .map(|t| t.bot_token.clone())
        .or_else(|| std::env::var("TELEGRAM_BOT_TOKEN").ok());

    if telegram_token.is_some() {
        info!("Telegram channel enabled");
    }

    // Get Discord config
    let discord_bot_token = config
        .channels
        .discord
        .as_ref()
        .map(|d| d.bot_token.clone())
        .or_else(|| std::env::var("DISCORD_BOT_TOKEN").ok());

    let discord_app_id = config
        .channels
        .discord
        .as_ref()
        .map(|d| d.application_id.clone())
        .or_else(|| std::env::var("DISCORD_APP_ID").ok());

    let discord_public_key = config
        .channels
        .discord
        .as_ref()
        .map(|d| d.public_key.clone());

    if discord_bot_token.is_some() && discord_app_id.is_some() {
        info!("Discord channel enabled");
    }

    // Build app state - pass Arcs directly
    let state = AppStateBuilder::new()
        .bot(bot)
        .storage_arc(storage.clone())
        .llm_arc(llm.clone())
        .tools_arc(tools.clone())
        .webhook_secret(config.server.webhook_secret.clone())
        .discord_public_key(discord_public_key)
        .default_model(config.llm.model.clone())
        .telegram_token(telegram_token)
        .discord_config(discord_bot_token, discord_app_id)
        .build();

    // Start the scheduler for heartbeats and scheduled tasks
    let mut scheduler = create_scheduler(config, llm.clone(), tools.clone(), storage.clone());

    // Load persisted cron jobs
    match scheduler.load_jobs().await {
        Ok(count) if count > 0 => info!("Loaded {} persisted cron jobs", count),
        Ok(_) => debug!("No persisted cron jobs found"),
        Err(e) => warn!("Failed to load cron jobs: {}", e),
    }

    scheduler.start();
    info!("Scheduler started");

    let server_config = ServerConfig {
        host: host.clone(),
        port,
        webhook_secret: config.server.webhook_secret.clone(),
    };

    info!("Server listening on {}:{}", host, port);
    start_server(server_config, state).await?;

    // Clean shutdown
    scheduler.stop().await;

    Ok(())
}

/// Run the daemon server (background mode)
pub(crate) async fn run_daemon(config: &Config, host: String, port: u16) -> anyhow::Result<()> {
    use nanna_daemon::{DaemonConfig, DaemonServer, IpcServerConfig, WebhookConfig};
    use nanna_daemon::server::{LlmConfig, EmbeddingConfig};
    use nanna_daemon::agent_service::AgentServiceConfig;

    // Configure daemon
    let data_dir = Config::default_data_dir()?;
    
    let daemon_config = DaemonConfig {
        ipc: IpcServerConfig {
            host: host.clone(),
            port,
            ..Default::default()
        },
        data_dir,
        log_level: "info".to_string(),
        auto_save_interval_secs: 60,
        llm: LlmConfig {
            provider: config.llm.provider.clone(),
            anthropic_api_key: config.llm.api_key.clone(),
            anthropic_oauth_token: None,
            anthropic_use_oauth: false,
            openai_api_key: config.llm.openai_api_key.clone(),
            openrouter_api_key: config.llm.openrouter_api_key.clone(),
            github_token: config.llm.github_token.clone(),
            ollama_host: "http://localhost:11434".to_string(),
            ollama_api_key: None,
            api_key: config.llm.api_key.clone(),
        },
        agent: AgentServiceConfig::default(),
        enable_memory: true,
        enable_health_server: true,
        health_port: 5148,
        enable_pid_file: true,
        enable_webhook_server: false,
        webhook_port: 3000,
        webhook: WebhookConfig::default(),
        use_script_tools: config.tools.use_script_tools,
        tools_dir: config.tools.tools_dir.clone(),
        // Legacy single-binary path: channels are not started here (matches the
        // field's Default). The daemon path wires channel config separately.
        channels: None,
        memory_max_compression_ratio: config.memory.max_compression_ratio,
        memory_min_remaining_memories: config.memory.min_remaining_memories,
    };

    info!("Initializing daemon server...");
    let mut server = DaemonServer::new(
        daemon_config,
        EmbeddingConfig::default(),
        None,
        None,
    );

    info!("Daemon listening on {}:{}", host, port);
    info!("WebSocket endpoint: ws://{}:{}/ws", host, port);

    // Run until interrupted
    server.run().await?;

    info!("Daemon shutting down");
    Ok(())
}
