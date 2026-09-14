//! `server` command and the legacy daemon-mode entry point.

use crate::setup::{create_scheduler, init_components};
use nanna_config::Config;
use nanna_core::{LlmClient, Nanna, NannaConfig};
use nanna_server::{AppStateBuilder, ServerConfig, start_server};
use tracing::{debug, info, warn};

/// The port `nanna server` listens on: the `--port` flag when given, otherwise
/// `[server].port` from the config — which the `PORT` env override and the
/// onboarding "Server port" answer both write, and which defaults to 3000.
///
/// Until 2026-09-11 the flag defaulted to the literal 3000 and the config key
/// was read by nothing, so the port a user chose during onboarding, the README's
/// documented `[server] port = …`, and `PORT` were all silently ignored. Pure,
/// so the precedence is pinned by a test rather than argued.
#[must_use]
pub fn server_port(flag: Option<u16>, config: &Config) -> u16 {
    let port = flag.unwrap_or(config.server.port);
    debug_assert!(
        flag.is_none_or(|f| f == port),
        "an explicit flag always wins"
    );
    debug_assert!(
        flag.is_some() || port == config.server.port,
        "…else the config decides"
    );
    port
}

/// Run the HTTP server
pub async fn run_server(config: &Config, host: String, port: u16) -> anyhow::Result<()> {
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
        // Slack's signing secret was parsed and then never handed to the
        // server, so the `nanna serve` Slack webhook verified nothing. Now the
        // handler fails closed without it, so the wiring is load-bearing.
        .slack_signing_secret(
            config
                .channels
                .slack
                .as_ref()
                .map(|s| s.signing_secret.clone()),
        )
        .telegram_webhook_secret(
            config
                .channels
                .telegram
                .as_ref()
                .and_then(|t| t.webhook_secret.clone()),
        )
        .signal_webhook_secret(
            config
                .channels
                .signal
                .as_ref()
                .and_then(|s| s.webhook_secret.clone()),
        )
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
pub async fn run_daemon(config: &Config, host: String, port: u16) -> anyhow::Result<()> {
    use nanna_daemon::agent_service::AgentServiceConfig;
    use nanna_daemon::server::{EmbeddingConfig, LlmConfig};
    use nanna_daemon::{DaemonConfig, DaemonServer, IpcServerConfig, WebhookConfig};

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
            // `config` was hydrated from the SecureStore at load; keep OAuth
            // state in sync with the daemon path (DaemonBuilder::from_nanna_config).
            anthropic_oauth_token: config.llm.anthropic_oauth_token.clone(),
            anthropic_use_oauth: config.llm.anthropic_use_oauth,
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
        tool_allowlist: Some(config.tools.enabled.clone()),
        tool_denylist: config.tools.disabled.clone(),
        tool_audit_log: config.tools.audit_log,
        tool_audit_log_values: config.tools.audit_log_values,
        // Legacy single-binary path: channels are not started here (matches the
        // field's Default). The daemon path wires channel config separately.
        channels: None,
        memory_max_compression_ratio: config.memory.max_compression_ratio,
        memory_min_remaining_memories: config.memory.min_remaining_memories,
        dream_idle_threshold_secs: config.memory.dream_idle_threshold_secs,
        dream_memory_pressure_count: config.memory.dream_memory_pressure_count,
        scheduler_enabled: config.scheduler.enabled,
        heartbeat_enabled: config.scheduler.heartbeat_enabled,
        heartbeat_interval_secs: config.scheduler.heartbeat_interval_secs,
    };

    info!("Initializing daemon server...");
    // From the user's config, not `EmbeddingConfig::default()` — the default
    // ignores the configured provider, model, AND priority list, so this
    // entry point ran a different embedder than the daemon path did from the
    // same config file.
    let embedding = EmbeddingConfig {
        provider: config.memory.embedding_provider.clone(),
        model: config.memory.embedding_model.clone(),
        ollama_host: config.memory.ollama_host.clone(),
        priority: config.memory.embedding_priority.clone(),
    };
    let mut server = DaemonServer::new(daemon_config, embedding, None, None);

    info!("Daemon listening on {}:{}", host, port);
    info!("WebSocket endpoint: ws://{}:{}/ws", host, port);

    // Run until interrupted
    server.run().await?;

    info!("Daemon shutting down");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The port a user chose during onboarding — written to `[server].port`,
    /// as is `PORT` — is the one `nanna server` listens on when no `--port`
    /// is given. Before 2026-09-11 it was silently ignored.
    #[test]
    fn server_port_falls_back_to_the_configured_port() {
        let mut config = Config::default();
        config.server.port = 4100;
        assert_eq!(server_port(None, &config), 4100);
    }

    /// An explicit `--port` still wins over the config.
    #[test]
    fn an_explicit_port_flag_overrides_the_config() {
        let mut config = Config::default();
        config.server.port = 4100;
        assert_eq!(server_port(Some(8080), &config), 8080);
    }

    /// With nothing configured the listening port is what it always was.
    #[test]
    fn the_default_port_is_unchanged() {
        assert_eq!(server_port(None, &Config::default()), 3000);
    }
}
