#![warn(clippy::pedantic, clippy::nursery, clippy::all)]

//! Nanna GUI - Tauri backend
//!
//! The GUI is a **pure daemon client**. It starts and connects to the headless
//! `nanna-daemon` sidecar over WebSocket IPC and forwards every request to it.
//! The daemon owns storage, memory, the tool registry, the agent loop, and the
//! scheduler — there is no in-process ("embedded") backend. If the daemon cannot
//! be reached the app reports a disconnected status rather than running its own
//! copy of the stack.

pub mod agents;
pub mod backend;
pub mod daemon_client;
pub mod daemon_manager;
pub mod commands;
pub mod state;

use backend::{Backend, BackendMode};

use nanna_config::Config;
use nanna_core::{
    Workspace, WorkspaceRegistry, find_workspace_root, discover_workspaces,
};
use nanna_core::log_buffer::{LogBuffer, LogBufferLayer, LogEntry, LogSource};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    menu::{MenuBuilder, MenuItemBuilder},
    AppHandle, Emitter, Manager, State,
};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

// Re-export moved items at the crate root so sibling modules that `use crate::*`
// keep resolving their existing paths.
pub(crate) use commands::settings::*;
pub(crate) use state::*;

// =============================================================================
// App Setup
// =============================================================================

/// Hydrate the workspace-registry cache from the daemon.
///
/// The daemon owns workspace persistence; this cache backs local reads and the
/// workspace-file editing commands. Best-effort: an empty registry if the
/// daemon is unreachable at startup (the reconnect loop will attach later).
async fn load_workspaces_from_daemon(backend: &Backend) -> WorkspaceRegistry {
    let mut registry = WorkspaceRegistry::new();

    let result = match backend.workspace_list().await {
        Ok(result) => result,
        Err(e) => {
            warn!("Failed to load workspaces from daemon: {e}");
            return registry;
        }
    };

    let records = result
        .get("workspaces")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let active_id = result
        .get("active_id")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
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
                warn!("Failed to load workspace context for {path:?}: {e}");
            }
            registry.register(ws);
        } else {
            warn!("Daemon workspace path no longer exists: {path:?}");
        }
    }
    if let Some(id) = active_id {
        registry.set_active(&id);
    }
    if count > 0 {
        info!("Restored {count} workspaces from daemon");
    }

    registry
}

/// Build the thin-client [`AppState`]. All heavy subsystems live in the daemon.
async fn setup_state(
    backend: Arc<Backend>,
    log_buffer: LogBuffer,
) -> Result<AppState, Box<dyn std::error::Error + Send + Sync>> {
    let config = Config::load().unwrap_or_default().with_env_overrides();

    let workspaces = Arc::new(RwLock::new(load_workspaces_from_daemon(&backend).await));

    // Initial active model for the badge, from the priority list or default.
    let initial_active_model = config
        .llm
        .model_priority
        .first()
        .cloned()
        .unwrap_or_else(|| config.llm.model.clone());

    info!("Nanna GUI (daemon client) initialized with model: {}", config.llm.model);

    Ok(AppState {
        config,
        close_mode: Arc::new(RwLock::new(CloseMode::default())),
        active_model: Arc::new(RwLock::new(initial_active_model)),
        rate_limited_models: Arc::new(RwLock::new(HashMap::new())),
        workspaces,
        backend,
        log_buffer,
    })
}

/// Recent in-process log lines, kept so the Logs page can show the GUI's own
/// output merged with the daemon's. ~5k entries x ~200 B is ~1 MB resident.
const GUI_LOG_BUFFER_ENTRIES: usize = 5000;

pub fn run() {
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    // Log to stdout AND into an in-memory buffer, so the GUI's own lines are
    // visible on the Logs page (merged with the daemon's).
    let log_buffer = LogBuffer::new(GUI_LOG_BUFFER_ENTRIES, LogSource::Embedded);
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("nanna=info".parse().unwrap_or_else(|_| LevelFilter::INFO.into())),
        )
        .with(tracing_subscriber::fmt::layer())
        .with(LogBufferLayer::new(log_buffer.clone()))
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle().clone();

            setup_system_tray(app)?;

            tauri::async_runtime::spawn(async move {
                // Start and connect to the daemon sidecar. A failed connect is a
                // hard, user-visible state (the frontend shows a "start the
                // daemon" affordance); there is no embedded fallback.
                let backend = Arc::new(Backend::new());
                let mode = backend.init(&handle).await;
                match mode {
                    BackendMode::Daemon => info!("Backend connected to daemon"),
                    BackendMode::Disconnected => {
                        error!("Backend could not reach the daemon — the app will show a disconnected state until it connects");
                    }
                }

                match setup_state(backend, log_buffer).await {
                    Ok(state) => {
                        handle.manage(Arc::new(RwLock::new(state)));
                        info!("App state initialized successfully");
                    }
                    Err(e) => {
                        error!("Failed to initialize app state: {e}");
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
                // Stop the daemon sidecar on exit. Memory/session persistence is
                // the daemon's job (Turso write-through), so there is nothing to
                // flush from the client here.
                if let Some(state) = app.try_state::<Arc<RwLock<AppState>>>() {
                    let state = state.inner().clone();
                    tauri::async_runtime::block_on(async {
                        let state_guard = state.read().await;
                        info!("Shutting down backend...");
                        state_guard.backend.shutdown().await;
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
