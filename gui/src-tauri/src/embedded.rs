//! Embedded-mode agent backend.
//!
//! When the daemon is unavailable, the GUI runs the daemon's [`AgentService`]
//! in-process instead of hand-rolling its own agent loop. Both modes therefore
//! share ONE agent loop (nanna-agent driven by `AgentService`, with
//! `AgentContext` as the single source of truth) and ONE event pipeline:
//! `protocol::Event` → [`DaemonEvent`] → the single Tauri forwarding task in
//! [`crate::backend::Backend::start_event_forwarding`].

use crate::daemon_client::DaemonEvent;
use nanna_agent::ThinkingMode;
use nanna_config::Config;
use nanna_daemon::agent_service::{AgentService, AgentServiceConfig};
use nanna_daemon::llm_router::LlmRouter;
use nanna_daemon::protocol::Event;
use nanna_memory::MemoryService;
use nanna_storage::Storage;
use nanna_tools::ToolRegistry;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, warn};

/// Build the multi-provider LLM router from the GUI config.
///
/// Mirrors the daemon's `init_services` provider wiring, using the same
/// credential sources the GUI historically used (config values with env-var
/// fallbacks). Ollama is always registered so at least one provider exists.
pub(crate) fn build_llm_router(config: &Config, ollama_host: &str) -> LlmRouter {
    let mut router = LlmRouter::new();

    // Anthropic: OAuth preferred when enabled, else API key (config or env).
    let oauth_token = if config.llm.anthropic_use_oauth {
        config.llm.anthropic_oauth_token.clone()
    } else {
        None
    };
    if let Some(token) = oauth_token {
        router = router.with_anthropic_oauth(&token);
    } else if let Some(key) = config
        .llm
        .api_key
        .clone()
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
    {
        router = router.with_anthropic(&key);
    }

    if let Some(key) = config
        .llm
        .openai_api_key
        .clone()
        .or_else(|| std::env::var("OPENAI_API_KEY").ok())
    {
        router = router.with_openai(&key);
    }

    if let Some(key) = config
        .llm
        .openrouter_api_key
        .clone()
        .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())
    {
        router = router.with_openrouter(&key);
    }

    if let Some(token) = config
        .llm
        .github_token
        .clone()
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
    {
        router = router.with_github_models(&token);
    }

    // Ollama is always available (local models need no credentials).
    match config.llm.ollama_api_key.as_deref() {
        Some(key) if !key.is_empty() => {
            router = router.with_ollama_authenticated(ollama_host, key);
        }
        _ => {
            router = router.with_ollama(ollama_host);
        }
    }

    router
}

/// Map the GUI's `nanna_config::Config` onto the daemon's [`AgentServiceConfig`].
///
/// Mirrors `DaemonServer::from_config`, including the iteration policy
/// (`max_iterations = None` means unbounded; escalating wrap-up nudges are
/// driven by `nudge_after_iterations` / `nudge_interval_iterations`).
pub(crate) fn agent_service_config_from(config: &Config, ollama_host: &str) -> AgentServiceConfig {
    AgentServiceConfig {
        model: config
            .llm
            .model_priority
            .first()
            .cloned()
            .unwrap_or_else(|| config.llm.model.clone()),
        model_priority: config.llm.model_priority.clone(),
        max_tokens: config.llm.max_tokens,
        temperature: config.llm.temperature,
        max_iterations: config.agent.max_iterations,
        nudge_after_iterations: config.agent.nudge_after_iterations,
        nudge_interval_iterations: config.agent.nudge_interval_iterations,
        thinking_mode: if config.agent.thinking_enabled {
            ThinkingMode::Medium
        } else {
            ThinkingMode::Instant
        },
        summarization_priority: config.llm.summarization_priority.clone(),
        summarization_ollama_url: config
            .llm
            .ollama_url
            .clone()
            .or_else(|| Some(ollama_host.to_string())),
        model_routing: config.llm.model_routing.clone(),
        routing_first_turn_primary: config.llm.routing_first_turn_primary,
        sub_agent_model: config.llm.sub_agent_model.clone(),
        openrouter_api_key: config.llm.openrouter_api_key.clone(),
        openai_api_key: config.llm.openai_api_key.clone(),
    }
}

/// Construct the long-lived in-process [`AgentService`] for embedded mode.
///
/// Its `protocol::Event` stream is bridged into `daemon_event_tx` — the same
/// channel `DaemonClient` publishes WebSocket events on — so the identical
/// forwarding code emits `stream-chunk` / `tool-call` / `model-status` Tauri
/// events for both modes.
pub(crate) async fn build_embedded_agent_service(
    config: &Config,
    ollama_host: &str,
    tools: Arc<ToolRegistry>,
    memory: Arc<MemoryService>,
    storage: Arc<Storage>,
    daemon_event_tx: broadcast::Sender<DaemonEvent>,
) -> Option<Arc<AgentService>> {
    let router = build_llm_router(config, ollama_host);
    let providers = router.available_providers();
    if providers.is_empty() {
        warn!("Embedded agent service not constructed: no LLM providers configured");
        return None;
    }
    info!(
        "Embedded agent service: {} LLM providers available: {:?}",
        providers.len(),
        providers
    );

    // Shared model-stats tracker: agent runs record into it and the router
    // reads it for health-aware model ordering (same wiring as the daemon).
    let stats = nanna_agent::ModelStatsTracker::new();
    router.set_stats(stats.clone()).await;
    let router = Arc::new(router);

    // Bridge protocol events into the shared DaemonEvent bus.
    let (event_tx, event_rx) = broadcast::channel::<Event>(256);
    spawn_event_bridge(event_rx, daemon_event_tx);

    let data_dir = Config::default_data_dir().ok();
    let service = AgentService::with_data_dir(
        agent_service_config_from(config, ollama_host),
        router,
        tools,
        Some(memory),
        event_tx,
        data_dir,
    )
    .with_stats(stats)
    .with_storage(storage);

    Some(Arc::new(service))
}

/// Forward `protocol::Event`s from the in-process `AgentService` onto the
/// shared [`DaemonEvent`] bus. Both enums use the same serde representation
/// (`tag = "event", rename_all = "snake_case"`), so conversion is a serde
/// round-trip; variants `DaemonEvent` doesn't model are dropped, exactly as
/// they would be if received over the daemon's WebSocket.
fn spawn_event_bridge(mut rx: broadcast::Receiver<Event>, tx: broadcast::Sender<DaemonEvent>) {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let converted = serde_json::to_value(&event)
                        .ok()
                        .and_then(|v| serde_json::from_value::<DaemonEvent>(v).ok());
                    if let Some(daemon_event) = converted {
                        // Send fails only when nobody is subscribed yet; that's
                        // fine — events are fire-and-forget, like the daemon's.
                        let _ = tx.send(daemon_event);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("Embedded event bridge lagged, dropped {} events", n);
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}
