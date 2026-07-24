//! Settings, credentials, and model configuration commands.
//!
//! The daemon owns the live LLM clients, tool registry, and memory service.
//! These commands mutate the local `config` cache and persist it to
//! `config.toml` (which the daemon also reads); model/tool listings that need
//! live state are fetched from the daemon.

#[allow(clippy::wildcard_imports)]
use crate::*;

/// Tool names as seen by the daemon's registry (empty if the daemon is down).
async fn daemon_tool_names(state: &AppState) -> Vec<String> {
    state
        .backend
        .tool_list()
        .await
        .ok()
        .and_then(|r| {
            r.get("tools").and_then(|v| v.as_array()).map(|arr| {
                arr.iter()
                    .filter_map(|t| t.get("name").and_then(|v| v.as_str()).map(String::from))
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// Format Claude model IDs into friendly names.
fn format_claude_model_name(id: &str) -> String {
    match id {
        "claude-opus-4-5-20251101" => "Claude Opus 4.5".to_string(),
        "claude-opus-4-20250514" => "Claude Opus 4".to_string(),
        "claude-sonnet-4-20250514" => "Claude Sonnet 4".to_string(),
        "claude-3-5-sonnet-20241022" => "Claude Sonnet 3.5".to_string(),
        "claude-3-5-haiku-20241022" => "Claude Haiku 3.5".to_string(),
        _ => id.to_string(),
    }
}

/// Get application config
#[tauri::command]
pub async fn get_config(state: State<'_, Arc<RwLock<AppState>>>) -> Result<AppConfig, String> {
    let state_guard = state.read().await;

    let tool_names: Vec<String> = daemon_tool_names(&state_guard).await;

    Ok(AppConfig {
        theme: "dark".to_string(),
        model: state_guard.config.llm.model.clone(),
        api_key_set: state_guard.config.llm.api_key.is_some()
            || std::env::var("ANTHROPIC_API_KEY").is_ok(),
        available_models: vec![
            // Anthropic
            "claude-opus-4-20250514".to_string(),
            "claude-sonnet-4-20250514".to_string(),
            "claude-3-5-sonnet-20241022".to_string(),
            "claude-3-5-haiku-20241022".to_string(),
            // OpenAI
            "gpt-4o".to_string(),
            "gpt-4o-mini".to_string(),
            "gpt-4-turbo".to_string(),
            "o1".to_string(),
            "o1-mini".to_string(),
            // OpenRouter
            "deepseek/deepseek-chat".to_string(),
            "google/gemini-2.5-flash-preview-05-20".to_string(),
            "google/gemini-2.5-pro-preview-05-06".to_string(),
            // Ollama (local)
            "llama3.2".to_string(),
            "llama3.1".to_string(),
            "mistral".to_string(),
            "mixtral".to_string(),
            "codellama".to_string(),
            "qwen2.5".to_string(),
            "deepseek-coder-v2".to_string(),
        ],
        available_tools: tool_names,
    })
}

/// Update model setting
#[tauri::command]
pub async fn set_model(state: State<'_, Arc<RwLock<AppState>>>, model: String) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.llm.model = model;
    Ok(())
}

/// Set API key
#[tauri::command]
pub async fn set_api_key(
    state: State<'_, Arc<RwLock<AppState>>>,
    api_key: String,
) -> Result<(), String> {
    let mut state_guard = state.write().await;

    // The daemon owns the live LLM client; persist the key to config (which the
    // daemon reads) and ask it to reload.
    state_guard.config.llm.api_key = Some(api_key.clone());

    // SAFETY: single-threaded application context
    unsafe {
        std::env::set_var("ANTHROPIC_API_KEY", &api_key);
    }

    if let Err(e) = state_guard.config.save() {
        error!("Failed to save config: {e}");
    }
    let _ = state_guard.backend.config_reload().await;

    info!("API key updated");
    Ok(())
}

/// Extended settings for the settings page
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtendedSettings {
    // API Keys (masked for display)
    pub anthropic_key_set: bool,
    pub openai_key_set: bool,
    pub openrouter_key_set: bool,
    pub github_key_set: bool,
    pub claude_proxy_enabled: bool,
    pub claude_proxy_url: String,
    pub brave_key_set: bool,

    // Anthropic OAuth status
    pub anthropic_oauth_logged_in: bool,
    pub anthropic_use_oauth: bool,

    // Chat Provider
    pub provider: String,
    pub available_providers: Vec<String>,

    // Chat Model
    pub model: String,
    pub available_models: Vec<String>,

    // Embedding Provider (separate from chat)
    pub embedding_provider: String,
    pub embedding_model: String,
    pub available_embedding_providers: Vec<String>,
    pub available_embedding_models: Vec<String>,
    pub embedding_enabled: bool,

    // Memory extraction model (empty = use chat model)
    pub extraction_model: String,
    pub available_extraction_models: Vec<String>,

    // Ollama configuration
    pub ollama_host: String,
    pub ollama_api_key: String,

    // Generation params
    pub temperature: f32,
    pub top_p: f32,
    pub max_tokens: u32,

    // Tools
    pub tools: Vec<ToolInfo>,

    // Memory & Scheduling
    pub dreaming_enabled: bool,
    pub max_compression_ratio: f32,
    pub min_remaining_memories: usize,
    pub scheduler_enabled: bool,
    pub heartbeat_enabled: bool,
    pub heartbeat_interval_seconds: u64,

    // Agent loop (long-horizon worker). `agent_max_iterations` None = unlimited.
    pub agent_max_iterations: Option<usize>,
    pub agent_nudge_after_iterations: usize,
    pub agent_nudge_interval_iterations: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub enabled: bool,
}

/// Get extended settings
#[tauri::command]
pub async fn get_extended_settings(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<ExtendedSettings, String> {
    let state_guard = state.read().await;

    // Tools come from the daemon's live registry.
    let tools: Vec<ToolInfo> = state_guard
        .backend
        .tool_list()
        .await
        .ok()
        .and_then(|r| {
            r.get("tools").and_then(|v| v.as_array()).map(|arr| {
                arr.iter()
                    .filter_map(|t| {
                        Some(ToolInfo {
                            name: t.get("name")?.as_str()?.to_string(),
                            description: t.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            enabled: t.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
                        })
                    })
                    .collect()
            })
        })
        .unwrap_or_default();

    // Runtime memory/scheduler toggles have no daemon control action yet; the
    // daemon-only setters are no-ops, so report the enabled defaults.
    let dreaming_enabled = true;
    let scheduler_enabled = true;
    let heartbeat_enabled = true;
    let heartbeat_interval_seconds = 1800;

    // Embedding settings come from the config cache (the daemon reads the same file).
    let embedding_provider = state_guard.config.memory.embedding_provider.clone();
    let embedding_model = state_guard.config.memory.embedding_model.clone();
    let embedding_enabled = state_guard.config.memory.enabled;
    let ollama_host = state_guard.config.memory.ollama_host.clone();

    Ok(ExtendedSettings {
        anthropic_key_set: state_guard.config.llm.api_key.is_some()
            || std::env::var("ANTHROPIC_API_KEY").is_ok(),
        openai_key_set: state_guard.config.llm.openai_api_key.is_some()
            || std::env::var("OPENAI_API_KEY").is_ok(),
        openrouter_key_set: state_guard.config.llm.openrouter_api_key.is_some()
            || std::env::var("OPENROUTER_API_KEY").is_ok(),
        github_key_set: state_guard.config.llm.github_token.is_some()
            || std::env::var("GITHUB_TOKEN").is_ok(),
        claude_proxy_enabled: std::env::var("CLAUDE_PROXY_ENABLED").is_ok(),
        claude_proxy_url: std::env::var("CLAUDE_PROXY_URL")
            .unwrap_or_else(|_| "http://localhost:3456".to_string()),
        brave_key_set: std::env::var("BRAVE_API_KEY").is_ok(),

        // Anthropic OAuth status
        anthropic_oauth_logged_in: state_guard.config.llm.anthropic_oauth_token.is_some(),
        anthropic_use_oauth: state_guard.config.llm.anthropic_use_oauth,

        provider: state_guard.config.llm.provider.clone(),
        available_providers: vec![
            "anthropic".to_string(),
            "openai".to_string(),
            "openrouter".to_string(),
            "github".to_string(),
            "claude-proxy".to_string(),
            "ollama".to_string(),
        ],

        model: state_guard.config.llm.model.clone(),
        available_models: vec![
            // Anthropic
            "claude-opus-4-20250514".to_string(),
            "claude-sonnet-4-20250514".to_string(),
            "claude-3-5-sonnet-20241022".to_string(),
            "claude-3-5-haiku-20241022".to_string(),
            // OpenAI
            "gpt-4o".to_string(),
            "gpt-4o-mini".to_string(),
            "gpt-4-turbo".to_string(),
            "o1".to_string(),
            "o1-mini".to_string(),
            // OpenRouter
            "deepseek/deepseek-chat".to_string(),
            "google/gemini-2.5-flash-preview-05-20".to_string(),
            "google/gemini-2.5-pro-preview-05-06".to_string(),
            // Ollama (local)
            "llama3.2".to_string(),
            "llama3.1".to_string(),
            "mistral".to_string(),
            "mixtral".to_string(),
            "codellama".to_string(),
            "qwen2.5".to_string(),
            "deepseek-coder-v2".to_string(),
        ],

        // Embedding settings (separate from chat)
        embedding_provider,
        embedding_model,
        embedding_enabled,
        available_embedding_providers: vec![
            "openai".to_string(),
            "ollama".to_string(),
            "disabled".to_string(),
        ],
        available_embedding_models: vec![
            // OpenAI
            "text-embedding-3-small".to_string(),  // 1536 dims
            "text-embedding-3-large".to_string(),  // 3072 dims
            // Ollama (dynamic list fetched separately)
            "nomic-embed-text".to_string(),        // 768 dims
            "mxbai-embed-large".to_string(),       // 1024 dims
            "all-minilm".to_string(),              // 384 dims
        ],

        ollama_host,
        ollama_api_key: state_guard.config.llm.ollama_api_key.clone().unwrap_or_default(),

        // Memory extraction model
        extraction_model: state_guard.config.memory.extraction_model.clone(),
        available_extraction_models: vec![
            String::new(), // Empty = use chat model
            "claude-3-5-haiku-20241022".to_string(),
            "claude-3-5-sonnet-20241022".to_string(),
            "gpt-4o-mini".to_string(),
            "gpt-4o".to_string(),
        ],

        temperature: 1.0,
        top_p: 0.95,
        max_tokens: 8192,

        tools,

        // Memory & Scheduling settings
        dreaming_enabled,
        max_compression_ratio: state_guard.config.memory.max_compression_ratio,
        min_remaining_memories: state_guard.config.memory.min_remaining_memories,
        scheduler_enabled,
        heartbeat_enabled,
        heartbeat_interval_seconds,

        // Agent-loop iteration policy
        agent_max_iterations: state_guard.config.agent.max_iterations,
        agent_nudge_after_iterations: state_guard.config.agent.nudge_after_iterations,
        agent_nudge_interval_iterations: state_guard.config.agent.nudge_interval_iterations,
    })
}

/// Set memory extraction model (empty string = use chat model)
#[tauri::command]
pub async fn set_extraction_model(
    state: State<'_, Arc<RwLock<AppState>>>,
    model: String,
) -> Result<(), String> {
    let mut state_guard = state.write().await;

    // Persist to config (the daemon reads the same file).
    state_guard.config.memory.extraction_model = model.clone();
    if let Err(e) = state_guard.config.save() {
        warn!("Failed to save extraction model to config: {}", e);
    }
    let _ = state_guard.backend.config_reload().await;

    if model.is_empty() {
        info!("Extraction model set to: (use chat model)");
    } else {
        info!("Extraction model set to: {}", model);
    }
    Ok(())
}

/// Set a specific API key
#[tauri::command]
pub async fn set_provider_api_key(
    state: State<'_, Arc<RwLock<AppState>>>,
    provider: String,
    api_key: String,
) -> Result<(), String> {
    let mut state_guard = state.write().await;

    // The daemon owns the live LLM clients and tool registry; persist the key to
    // config (+ env for this process) and let the daemon reload.
    match provider.as_str() {
        "anthropic" => {
            state_guard.config.llm.api_key = Some(api_key.clone());
            unsafe { std::env::set_var("ANTHROPIC_API_KEY", &api_key); }
        }
        "openai" => {
            state_guard.config.llm.openai_api_key = Some(api_key.clone());
            unsafe { std::env::set_var("OPENAI_API_KEY", &api_key); }
        }
        "brave" => {
            state_guard.config.tools.brave_api_key = Some(api_key.clone());
            unsafe { std::env::set_var("BRAVE_API_KEY", &api_key); }
        }
        "openrouter" => {
            state_guard.config.llm.openrouter_api_key = Some(api_key.clone());
            unsafe { std::env::set_var("OPENROUTER_API_KEY", &api_key); }
        }
        "github" => {
            state_guard.config.llm.github_token = Some(api_key.clone());
            unsafe { std::env::set_var("GITHUB_TOKEN", &api_key); }
        }
        "claude-proxy" => {
            // For claude-proxy, the "api_key" is actually the proxy URL
            unsafe {
                std::env::set_var("CLAUDE_PROXY_URL", &api_key);
                std::env::set_var("CLAUDE_PROXY_ENABLED", "1");
            }
        }
        _ => return Err(format!("Unknown provider: {}", provider)),
    }

    // Durable storage is the OS keyring; config.toml never receives secrets.
    // (claude-proxy is a URL, not a secret — strip_secrets leaves it alone.)
    if provider != "claude-proxy" {
        if let Err(e) = state_guard.config.migrate_secrets_to_keyring() {
            error!("Failed to store API key in keyring: {e}");
            return Err(format!("failed to store API key securely: {e}"));
        }
    }
    if let Err(e) = state_guard.config.save() {
        error!("Failed to save config: {}", e);
        // Non-fatal - key is hydrated in-process for this session
    }
    let _ = state_guard.backend.config_reload().await;

    info!("API key set for provider: {} (secure store)", provider);
    Ok(())
}

// =============================================================================
// Anthropic OAuth Login (via `claude setup-token`)
// =============================================================================

/// Run `claude setup-token` to authenticate via Claude Code CLI
/// This opens a browser for OAuth, then imports the resulting credentials
#[tauri::command]
pub async fn run_claude_setup_token(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<String, String> {
    use nanna_config::ClaudeCredentialManager;

    // Check if Claude CLI is available
    if !ClaudeCredentialManager::is_claude_cli_available() {
        return Err(
            "Claude Code CLI not found. Please install it first:\n\
             npm install -g @anthropic-ai/claude-code\n\n\
             Or paste your token from `claude setup-token` directly.".to_string()
        );
    }

    info!("Running claude setup-token...");

    // Run claude setup-token (this will open browser and wait for auth)
    ClaudeCredentialManager::run_setup_token()
        .map_err(|e| format!("Failed to run claude setup-token: {}", e))?;

    info!("claude setup-token completed");

    // Now import the credentials that were saved
    let manager = ClaudeCredentialManager::new();
    let loaded = manager.load()
        .map_err(|e| format!("Failed to load credentials after setup: {}", e))?;

    // Save the token
    let mut state_guard = state.write().await;
    state_guard.config.llm.anthropic_oauth_token = Some(loaded.credential.access_token.clone());
    state_guard.config.llm.anthropic_use_oauth = true;

    if let Err(e) = state_guard.config.save() {
        error!("Failed to save OAuth token: {}", e);
    }
    let _ = state_guard.backend.config_reload().await;

    let subscription = loaded.credential.subscription_type.unwrap_or_else(|| "unknown".to_string());
    info!("Successfully authenticated via claude setup-token (subscription: {})", subscription);

    Ok(format!("Successfully authenticated! Subscription: {}", subscription))
}

/// Import credentials from Claude Code CLI (~/.claude/.credentials.json)
/// This uses the token that Claude Code CLI obtained, which is whitelisted
#[tauri::command]
pub async fn import_claude_code_credentials(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    use nanna_config::ClaudeCredentialManager;

    let manager = ClaudeCredentialManager::new();

    // Load credentials (checks file and keychain)
    let loaded = manager.load()
        .map_err(|e| format!("No credentials found: {}. Please run `claude login` first.", e))?;

    // Check if token is expired
    if loaded.credential.is_expired() {
        if loaded.credential.can_refresh() {
            info!("Token expired, attempting auto-refresh...");
            let refreshed = manager.refresh_token(&loaded.credential).await
                .map_err(|e| format!("Token expired and refresh failed: {}. Please run `claude login`.", e))?;

            // Save refreshed token back to source
            if let Err(e) = manager.save(&refreshed, loaded.source) {
                warn!("Failed to save refreshed token: {}", e);
            }

            // Update state with refreshed token
            let mut state_guard = state.write().await;
            state_guard.config.llm.anthropic_oauth_token = Some(refreshed.access_token.clone());
            state_guard.config.llm.anthropic_use_oauth = true;

            if let Err(e) = state_guard.config.save() {
                error!("Failed to save config: {}", e);
            }
            let _ = state_guard.backend.config_reload().await;

            info!("Token refreshed and imported (subscription: {:?})", refreshed.subscription_type);
            return Ok(());
        } else {
            return Err("Token expired and cannot auto-refresh. Please run `claude login`.".to_string());
        }
    }

    info!(
        "Imported Claude Code credentials (subscription: {:?})",
        loaded.credential.subscription_type
    );

    // Save the token and enable OAuth mode
    let mut state_guard = state.write().await;
    state_guard.config.llm.anthropic_oauth_token = Some(loaded.credential.access_token.clone());
    state_guard.config.llm.anthropic_use_oauth = true;

    // Persist to config (the daemon rebuilds its LLM client on reload)
    if let Err(e) = state_guard.config.save() {
        error!("Failed to save OAuth token: {}", e);
    }
    let _ = state_guard.backend.config_reload().await;

    info!("Successfully imported Claude Code credentials");
    Ok(())
}

/// Save an Anthropic OAuth token directly (from `claude setup-token`)
#[tauri::command]
pub async fn save_anthropic_oauth_token(
    state: State<'_, Arc<RwLock<AppState>>>,
    token: String,
) -> Result<(), String> {
    let mut state_guard = state.write().await;

    let token = token.trim().to_string();
    if token.is_empty() {
        return Err("Token cannot be empty".to_string());
    }

    // Save the token and enable OAuth mode
    state_guard.config.llm.anthropic_oauth_token = Some(token.clone());
    state_guard.config.llm.anthropic_use_oauth = true;

    // Persist to config (the daemon rebuilds its LLM client on reload)
    if let Err(e) = state_guard.config.save() {
        error!("Failed to save OAuth token: {}", e);
    }
    let _ = state_guard.backend.config_reload().await;

    info!("Anthropic OAuth token saved");
    Ok(())
}

/// Log out of Anthropic OAuth (clear token and switch to API key mode)
#[tauri::command]
pub async fn logout_anthropic_oauth(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    let mut state_guard = state.write().await;

    state_guard.config.llm.anthropic_oauth_token = None;
    state_guard.config.llm.anthropic_use_oauth = false;

    // Persist to config (the daemon rebuilds its LLM client on reload)
    if let Err(e) = state_guard.config.save() {
        error!("Failed to save config after logout: {}", e);
    }
    let _ = state_guard.backend.config_reload().await;

    info!("Anthropic OAuth logout successful");
    Ok(())
}

/// Get Claude CLI credential status
#[derive(serde::Serialize)]
pub struct CredentialStatus {
    cli_available: bool,
    credentials_found: bool,
    source: Option<String>,
    is_expired: bool,
    can_refresh: bool,
    seconds_until_expiry: Option<i64>,
    subscription_type: Option<String>,
}

#[tauri::command]
pub async fn get_credential_status() -> Result<CredentialStatus, String> {
    use nanna_config::{ClaudeCredentialManager, CredentialSource};

    let cli_available = ClaudeCredentialManager::is_claude_cli_available();
    let manager = ClaudeCredentialManager::new();

    match manager.load() {
        Ok(loaded) => {
            let source = match loaded.source {
                CredentialSource::File => "file",
                CredentialSource::MacOsKeychain => "macos_keychain",
                CredentialSource::WindowsCredentialManager => "windows_credential_manager",
                CredentialSource::LinuxSecretService => "linux_secret_service",
            };
            Ok(CredentialStatus {
                cli_available,
                credentials_found: true,
                source: Some(source.to_string()),
                is_expired: loaded.credential.is_expired(),
                can_refresh: loaded.credential.can_refresh(),
                seconds_until_expiry: loaded.credential.seconds_until_expiry(),
                subscription_type: loaded.credential.subscription_type,
            })
        }
        Err(_) => {
            Ok(CredentialStatus {
                cli_available,
                credentials_found: false,
                source: None,
                is_expired: false,
                can_refresh: false,
                seconds_until_expiry: None,
                subscription_type: None,
            })
        }
    }
}

/// Refresh the OAuth token if expired or expiring soon
#[tauri::command]
pub async fn refresh_oauth_token(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<String, String> {
    use nanna_config::ClaudeCredentialManager;

    let manager = ClaudeCredentialManager::new();
    let loaded = manager.load()
        .map_err(|e| format!("No credentials found: {}", e))?;

    if !loaded.credential.can_refresh() {
        return Err("Cannot refresh: no refresh token available".to_string());
    }

    let refreshed = manager.refresh_token(&loaded.credential).await
        .map_err(|e| format!("Token refresh failed: {}", e))?;

    // Save back to source
    if let Err(e) = manager.save(&refreshed, loaded.source) {
        warn!("Failed to save refreshed token to source: {}", e);
    }

    // Update the config cache and let the daemon rebuild its client on reload.
    let mut state_guard = state.write().await;
    state_guard.config.llm.anthropic_oauth_token = Some(refreshed.access_token.clone());

    if let Err(e) = state_guard.config.save() {
        error!("Failed to save config: {}", e);
    }
    let _ = state_guard.backend.config_reload().await;

    let hours = refreshed.seconds_until_expiry().map(|s| s / 3600).unwrap_or(0);
    info!("OAuth token refreshed, expires in {}h", hours);

    Ok(format!("Token refreshed! Expires in {}h", hours))
}

/// Set the active LLM provider
#[tauri::command]
pub async fn set_provider(
    state: State<'_, Arc<RwLock<AppState>>>,
    provider: String,
) -> Result<(), String> {
    let mut state_guard = state.write().await;

    // Validate that the selected provider has usable credentials, so we fail
    // here with a clear message rather than at chat time in the daemon. The
    // daemon owns the actual LLM client; we just persist the choice.
    match provider.as_str() {
        "anthropic" => {
            if state_guard.config.llm.anthropic_oauth_token.is_none() {
                return Err("No OAuth token available. Run `claude setup-token` or paste your token.".to_string());
            }
        }
        "openai" => {
            if state_guard.config.llm.openai_api_key.is_none() && std::env::var("OPENAI_API_KEY").is_err() {
                return Err("No API key set for openai".to_string());
            }
        }
        "openrouter" => {
            if state_guard.config.llm.openrouter_api_key.is_none() && std::env::var("OPENROUTER_API_KEY").is_err() {
                return Err("No API key set for openrouter".to_string());
            }
        }
        "github" => {
            if state_guard.config.llm.github_token.is_none() && std::env::var("GITHUB_TOKEN").is_err() {
                return Err("No API key set for github".to_string());
            }
        }
        "claude-proxy" | "ollama" => {}
        _ => return Err(format!("Unknown provider: {}", provider)),
    }

    state_guard.config.llm.provider = provider.clone();
    if let Err(e) = state_guard.config.save() {
        error!("Failed to save config: {}", e);
    }
    let _ = state_guard.backend.config_reload().await;

    info!("Provider changed to: {}", provider);
    Ok(())
}

/// Set the embedding provider and model (requires restart to take effect)
#[tauri::command]
pub async fn set_embedding_config(
    state: State<'_, Arc<RwLock<AppState>>>,
    provider: String,
    model: String,
) -> Result<String, String> {
    let mut state_guard = state.write().await;

    // Validate provider
    if !["openai", "ollama", "disabled"].contains(&provider.as_str()) {
        return Err(format!("Unknown embedding provider: {}", provider));
    }

    let model = if provider == "disabled" { "none".to_string() } else { model };

    // Validate OpenAI models (Ollama accepts any installed model)
    if provider == "openai" {
        let valid_openai = ["text-embedding-3-small", "text-embedding-3-large"];
        if !valid_openai.contains(&model.as_str()) {
            return Err(format!("Unknown OpenAI embedding model: {}", model));
        }
    }

    // Save to config file (the daemon reads the same file at startup)
    state_guard.config.memory.embedding_provider = provider.clone();
    state_guard.config.memory.embedding_model = model.clone();
    state_guard.config.memory.enabled = provider != "disabled";
    if let Err(e) = state_guard.config.save() {
        error!("Failed to save embedding config: {}", e);
    }

    info!("Embedding config changed to: {} / {}", provider, model);

    // Return warning about restart
    Ok("Embedding settings updated. Restart required for changes to take effect. Note: Changing embedding dimensions will make existing memories incompatible.".to_string())
}

/// Get env var status (for checking if keys are set)
#[tauri::command]
pub async fn check_env_var(name: String) -> Result<bool, String> {
    Ok(std::env::var(&name).is_ok())
}

/// Set Ollama host URL
#[tauri::command]
pub async fn set_ollama_host(
    state: State<'_, Arc<RwLock<AppState>>>,
    host: String,
) -> Result<String, String> {
    let mut state_guard = state.write().await;

    // Validate URL format
    if !host.starts_with("http://") && !host.starts_with("https://") {
        return Err("Ollama host must start with http:// or https://".to_string());
    }

    // Remove trailing slash
    let host = host.trim_end_matches('/').to_string();

    // Save to config file (the daemon reads the same file)
    state_guard.config.memory.ollama_host = host.clone();
    match state_guard.config.save() {
        Ok(()) => {
            info!("Ollama host saved to config: {}", host);
        }
        Err(e) => {
            let err_msg = format!("Failed to save config: {}", e);
            error!("{}", err_msg);
            return Err(err_msg);
        }
    }
    let _ = state_guard.backend.config_reload().await;

    // Also set env var for current session
    unsafe { std::env::set_var("OLLAMA_HOST", &host); }

    Ok(format!("Ollama host saved: {}", host))
}

/// Set Ollama API key (for remote/authenticated instances)
#[tauri::command]
pub async fn set_ollama_api_key(
    state: State<'_, Arc<RwLock<AppState>>>,
    key: String,
) -> Result<String, String> {
    let mut state_guard = state.write().await;
    state_guard.config.llm.ollama_api_key = if key.is_empty() { None } else { Some(key.clone()) };
    if let Err(e) = state_guard.config.migrate_secrets_to_keyring() {
        let err_msg = format!("Failed to store Ollama API key securely: {e}");
        error!("{err_msg}");
        return Err(err_msg);
    }
    match state_guard.config.save() {
        Ok(()) => {
            info!("Ollama API key saved to OS keychain");
        }
        Err(e) => {
            let err_msg = format!("Failed to save config: {}", e);
            error!("{}", err_msg);
            return Err(err_msg);
        }
    }
    let _ = state_guard.backend.config_reload().await;
    Ok("Ollama API key saved".to_string())
}

/// Fetch available models from Ollama
#[tauri::command]
pub async fn get_ollama_models(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<OllamaModelInfo>, String> {
    let state_guard = state.read().await;
    let ollama_host = state_guard.config.memory.ollama_host.clone();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .get(format!("{}/api/tags", ollama_host))
        .send()
        .await
        .map_err(|e| format!("Failed to connect to Ollama at {}: {}", ollama_host, e))?;

    if !response.status().is_success() {
        return Err(format!("Ollama returned error: {}", response.status()));
    }

    #[derive(Deserialize)]
    struct OllamaTagsResponse {
        models: Vec<OllamaModel>,
    }

    #[derive(Deserialize)]
    struct OllamaModel {
        name: String,
        size: u64,
    }

    let tags: OllamaTagsResponse = response.json().await
        .map_err(|e| format!("Failed to parse Ollama response: {}", e))?;

    // Convert to our info struct, marking known embedding models
    // Comprehensive list of known embedding model name patterns
    let embedding_patterns = [
        // BGE family
        "bge-m3", "bge-large", "bge-small", "bge-base",
        // Nomic
        "nomic-embed",
        // MixedBread
        "mxbai-embed",
        // Sentence transformers / all-minilm
        "all-minilm", "minilm",
        // Snowflake
        "snowflake-arctic-embed",
        // E5 family
        "e5-small", "e5-base", "e5-large", "e5-mistral",
        // GTE family
        "gte-small", "gte-base", "gte-large", "gte-qwen",
        // Jina
        "jina-embed",
        // Voyage
        "voyage",
        // Cohere
        "embed-english", "embed-multilingual",
        // Generic patterns (catch-all)
        "-embed-", "-embed:",
    ];

    let models: Vec<OllamaModelInfo> = tags.models
        .into_iter()
        .map(|m| {
            let name_lower = m.name.to_lowercase();
            let base_name = m.name.split(':').next().unwrap_or(&m.name).to_lowercase();

            // Check if model name contains "embed" or matches known embedding patterns
            let is_embedding = name_lower.contains("embed")
                || embedding_patterns.iter().any(|p| base_name.contains(p));

            OllamaModelInfo {
                name: m.name,
                size_mb: m.size / 1_000_000,
                is_embedding_model: is_embedding,
            }
        })
        .collect();

    Ok(models)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaModelInfo {
    pub name: String,
    pub size_mb: u64,
    pub is_embedding_model: bool,
}

/// Fetch available models from Anthropic
#[tauri::command]
pub async fn get_anthropic_models(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<ModelInfo>, String> {
    let state_guard = state.read().await;

    // Check if OAuth is configured, otherwise use API key
    let (auth_header, auth_value) = if state_guard.config.llm.anthropic_use_oauth {
        let token = state_guard.config.llm.anthropic_oauth_token.clone()
            .ok_or("OAuth enabled but no token available")?;
        ("Authorization".to_string(), format!("Bearer {}", token))
    } else {
        let api_key = state_guard.config.llm.api_key.clone()
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
            .ok_or("No Anthropic API key configured")?;
        ("x-api-key".to_string(), api_key)
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let mut request = client
        .get("https://api.anthropic.com/v1/models")
        .header(&auth_header, &auth_value)
        .header("anthropic-version", "2023-06-01");

    // Add OAuth-specific headers if using OAuth
    if state_guard.config.llm.anthropic_use_oauth {
        request = request
            .header("anthropic-beta", "claude-code-20250219,oauth-2025-04-20")
            .header("user-agent", "claude-code/2.1.2");
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("Failed to fetch Anthropic models: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Anthropic API error {}: {}", status, body));
    }

    #[derive(Deserialize)]
    struct AnthropicModelsResponse {
        data: Vec<AnthropicModel>,
    }

    #[derive(Deserialize)]
    struct AnthropicModel {
        id: String,
        display_name: Option<String>,
    }

    let models: AnthropicModelsResponse = response.json().await
        .map_err(|e| format!("Failed to parse Anthropic response: {}", e))?;

    Ok(models.data.into_iter().map(|m| ModelInfo {
        id: m.id.clone(),
        name: m.display_name.unwrap_or(m.id),
    }).collect())
}

/// Fetch available models from OpenAI
#[tauri::command]
pub async fn get_openai_models() -> Result<Vec<ModelInfo>, String> {
    let api_key = std::env::var("OPENAI_API_KEY")
        .map_err(|_| "No OpenAI API key configured")?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .get("https://api.openai.com/v1/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch OpenAI models: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("OpenAI API error {}: {}", status, body));
    }

    #[derive(Deserialize)]
    struct OpenAIModelsResponse {
        data: Vec<OpenAIModel>,
    }

    #[derive(Deserialize)]
    struct OpenAIModel {
        id: String,
    }

    let models: OpenAIModelsResponse = response.json().await
        .map_err(|e| format!("Failed to parse OpenAI response: {}", e))?;

    // Filter to chat models (gpt-*, o1-*, chatgpt-*)
    let chat_prefixes = ["gpt-4", "gpt-3.5", "o1", "o3", "chatgpt"];
    let embedding_prefixes = ["text-embedding"];

    let mut result: Vec<ModelInfo> = models.data.into_iter()
        .filter(|m| {
            chat_prefixes.iter().any(|p| m.id.starts_with(p)) ||
            embedding_prefixes.iter().any(|p| m.id.starts_with(p))
        })
        .map(|m| ModelInfo {
            id: m.id.clone(),
            name: m.id,
        })
        .collect();

    // Sort by name
    result.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(result)
}

/// Fetch available models from OpenRouter
#[tauri::command]
pub async fn get_openrouter_models(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<ModelInfo>, String> {
    let state_guard = state.read().await;

    let api_key = state_guard.config.llm.openrouter_api_key.clone()
        .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())
        .ok_or("No OpenRouter API key configured")?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .get("https://openrouter.ai/api/v1/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch OpenRouter models: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("OpenRouter API error {}: {}", status, body));
    }

    #[derive(Deserialize)]
    struct OpenRouterModelsResponse {
        data: Vec<OpenRouterModel>,
    }

    #[derive(Deserialize)]
    struct OpenRouterModel {
        id: String,
        name: Option<String>,
    }

    let models: OpenRouterModelsResponse = response.json().await
        .map_err(|e| format!("Failed to parse OpenRouter response: {}", e))?;

    // Priority prefixes for sorting (these appear first)
    let priority_prefixes = [
        "anthropic/claude",
        "openai/gpt",
        "openai/o1",
        "openai/o3",
        "openai/chatgpt",
        "google/gemini",
        "deepseek/",
        "meta-llama/",
        "mistralai/",
        "qwen/",
        "cohere/",
        "perplexity/",
    ];

    // Include ALL models (no filtering)
    let mut result: Vec<ModelInfo> = models.data.into_iter()
        .map(|m| ModelInfo {
            name: m.name.unwrap_or_else(|| m.id.clone()),
            id: m.id,
        })
        .collect();

    // Sort: priority models first, then alphabetically
    result.sort_by(|a, b| {
        let a_priority = priority_prefixes.iter().position(|p| a.id.starts_with(p)).unwrap_or(999);
        let b_priority = priority_prefixes.iter().position(|p| b.id.starts_with(p)).unwrap_or(999);
        a_priority.cmp(&b_priority).then_with(|| a.id.cmp(&b.id))
    });

    Ok(result)
}

/// Fetch available embedding models from OpenRouter's dedicated embeddings endpoint
#[tauri::command]
pub async fn get_openrouter_embedding_models(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<ModelInfo>, String> {
    let state_guard = state.read().await;

    let api_key = state_guard.config.llm.openrouter_api_key.clone()
        .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())
        .ok_or("No OpenRouter API key configured")?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .get("https://openrouter.ai/api/v1/embeddings/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch OpenRouter embedding models: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("OpenRouter embeddings API error {}: {}", status, body));
    }

    #[derive(Deserialize)]
    struct OpenRouterModelsResponse {
        data: Vec<OpenRouterEmbeddingModel>,
    }

    #[derive(Deserialize)]
    struct OpenRouterEmbeddingModel {
        id: String,
        name: Option<String>,
    }

    let models: OpenRouterModelsResponse = response.json().await
        .map_err(|e| format!("Failed to parse OpenRouter embeddings response: {}", e))?;

    let result: Vec<ModelInfo> = models.data.into_iter()
        .map(|m| ModelInfo {
            name: m.name.unwrap_or_else(|| m.id.clone()),
            id: m.id,
        })
        .collect();

    Ok(result)
}

/// Fetch available models from GitHub Models API
#[tauri::command]
pub async fn get_github_models(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<ModelInfo>, String> {
    let state_guard = state.read().await;
    let api_key = state_guard.config.llm.github_token.clone()
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
        .ok_or("No GitHub token configured")?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    // GitHub Models catalog endpoint
    let response = client
        .get("https://models.inference.ai.azure.com/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch GitHub models: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("GitHub Models API error {}: {}", status, body));
    }

    #[derive(Deserialize)]
    struct GitHubModelsResponse {
        data: Option<Vec<GitHubModel>>,
        #[serde(default)]
        models: Vec<GitHubModel>,
    }

    #[derive(Deserialize)]
    struct GitHubModel {
        id: Option<String>,
        name: Option<String>,
        #[serde(default)]
        model_name: Option<String>,
    }

    let text = response.text().await
        .map_err(|e| format!("Failed to read GitHub response: {}", e))?;

    // Try to parse as JSON array or object with data/models field
    let models: Vec<GitHubModel> = if let Ok(arr) = serde_json::from_str::<Vec<GitHubModel>>(&text) {
        arr
    } else if let Ok(resp) = serde_json::from_str::<GitHubModelsResponse>(&text) {
        resp.data.unwrap_or(resp.models)
    } else {
        return Err(format!("Failed to parse GitHub response: {}", text));
    };

    // Filter and map models
    let result: Vec<ModelInfo> = models.into_iter()
        .filter_map(|m| {
            let id = m.id.or(m.model_name)?;
            let name = m.name.unwrap_or_else(|| id.clone());
            Some(ModelInfo { id, name })
        })
        .collect();

    Ok(result)
}

/// Fetch available models from Anthropic API for use with Claude Proxy
/// This queries Anthropic directly to get models available on your subscription
#[tauri::command]
pub async fn get_claude_proxy_models(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<ModelInfo>, String> {
    let state_guard = state.read().await;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    // Try OAuth first (for Pro/Max subscription), then API key
    let response = if state_guard.config.llm.anthropic_oauth_token.is_some() {
        let token = state_guard.config.llm.anthropic_oauth_token.clone().unwrap();
        client
            .get("https://api.anthropic.com/v1/models")
            .header("Authorization", format!("Bearer {}", token))
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "claude-code-20250219,oauth-2025-04-20")
            .header("user-agent", "claude-code/2.1.2")
            .send()
            .await
            .map_err(|e| format!("Failed to fetch models: {}", e))?
    } else if let Some(ref api_key) = state_guard.config.llm.api_key {
        client
            .get("https://api.anthropic.com/v1/models")
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .send()
            .await
            .map_err(|e| format!("Failed to fetch models: {}", e))?
    } else if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
        client
            .get("https://api.anthropic.com/v1/models")
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01")
            .send()
            .await
            .map_err(|e| format!("Failed to fetch models: {}", e))?
    } else {
        // No Anthropic credentials - return default Claude models that the proxy supports
        return Ok(vec![
            ModelInfo { id: "claude-sonnet-4-20250514".to_string(), name: "Claude Sonnet 4".to_string() },
            ModelInfo { id: "claude-opus-4-20250514".to_string(), name: "Claude Opus 4".to_string() },
            ModelInfo { id: "claude-3-5-sonnet-20241022".to_string(), name: "Claude Sonnet 3.5".to_string() },
            ModelInfo { id: "claude-3-5-haiku-20241022".to_string(), name: "Claude Haiku 3.5".to_string() },
        ]);
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Anthropic API error {}: {}", status, body));
    }

    #[derive(Deserialize)]
    struct AnthropicModelsResponse {
        data: Vec<AnthropicModel>,
    }

    #[derive(Deserialize)]
    struct AnthropicModel {
        id: String,
        display_name: Option<String>,
    }

    let models: AnthropicModelsResponse = response.json().await
        .map_err(|e| format!("Failed to parse Anthropic response: {}", e))?;

    // Filter to chat models only (exclude embedding models, etc.)
    let result: Vec<ModelInfo> = models.data.into_iter()
        .filter(|m| m.id.starts_with("claude-"))
        .map(|m| {
            let name = m.display_name.unwrap_or_else(|| format_claude_model_name(&m.id));
            ModelInfo { id: m.id, name }
        })
        .collect();

    Ok(result)
}

/// Enable or disable Claude Proxy
#[tauri::command]
pub async fn set_claude_proxy(enabled: bool, url: Option<String>) -> Result<(), String> {
    unsafe {
        if enabled {
            std::env::set_var("CLAUDE_PROXY_ENABLED", "1");
            if let Some(u) = url {
                std::env::set_var("CLAUDE_PROXY_URL", u);
            }
        } else {
            std::env::remove_var("CLAUDE_PROXY_ENABLED");
        }
    }
    Ok(())
}

/// Check if Claude Proxy is running and reachable
#[tauri::command]
pub async fn check_claude_proxy_health() -> Result<bool, String> {
    let proxy_url = std::env::var("CLAUDE_PROXY_URL")
        .unwrap_or_else(|_| "http://localhost:3456".to_string());

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|e| e.to_string())?;

    match client.get(format!("{}/health", proxy_url)).send().await {
        Ok(resp) => Ok(resp.status().is_success()),
        Err(_) => Ok(false),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
}

// =============================================================================
// System Prompt & Agent Settings
// =============================================================================

/// Get the custom system prompt (returns None if using default)
#[tauri::command]
pub async fn get_system_prompt(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Option<String>, String> {
    let state_guard = state.read().await;
    Ok(state_guard.config.agent.system_prompt.clone())
}

/// Set a custom system prompt (pass null to reset to default)
#[tauri::command]
pub async fn set_system_prompt(
    state: State<'_, Arc<RwLock<AppState>>>,
    prompt: Option<String>,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.agent.system_prompt = prompt.clone();

    // Save to disk
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    info!("System prompt {}", if prompt.is_some() { "updated" } else { "reset to default" });
    Ok(())
}

/// Set agent name
#[tauri::command]
pub async fn set_agent_name(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.agent.name = name.clone();
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;
    info!("Agent name set to: {}", name);
    Ok(())
}

/// Set personality mode
#[tauri::command]
pub async fn set_personality_mode(
    state: State<'_, Arc<RwLock<AppState>>>,
    mode: String,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.agent.personality_mode = mode.clone();
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;
    info!("Personality mode set to: {}", mode);
    Ok(())
}

/// Set thinking mode enabled
#[tauri::command]
pub async fn set_thinking_enabled(
    state: State<'_, Arc<RwLock<AppState>>>,
    enabled: bool,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.agent.thinking_enabled = enabled;
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;
    info!("Thinking mode: {}", if enabled { "enabled" } else { "disabled" });
    Ok(())
}

/// Set streaming enabled
#[tauri::command]
pub async fn set_streaming_enabled(
    state: State<'_, Arc<RwLock<AppState>>>,
    enabled: bool,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.agent.streaming_enabled = enabled;
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;
    info!("Streaming: {}", if enabled { "enabled" } else { "disabled" });
    Ok(())
}

/// Set max tokens for responses
#[tauri::command]
pub async fn set_max_tokens(
    state: State<'_, Arc<RwLock<AppState>>>,
    tokens: u32,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.llm.max_tokens = tokens;
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;
    info!("Max tokens set to: {}", tokens);
    Ok(())
}

/// Set the agent-loop iteration policy.
///
/// The loop is a long-horizon worker: `max_iterations` is an optional absolute
/// backstop (`None`/0 = unlimited — only Stop/cancel or the model finishing ends
/// it). Escalating soft nudges begin at `nudge_after` and repeat every
/// `nudge_interval` iterations; they steer a possibly-stuck model but never stop it.
#[tauri::command]
pub async fn set_agent_iteration_policy(
    state: State<'_, Arc<RwLock<AppState>>>,
    max_iterations: Option<usize>,
    nudge_after: usize,
    nudge_interval: usize,
) -> Result<(), String> {
    // Treat 0 (or absent) max as "unlimited". Floor the nudge knobs at 1 so the
    // schedule is always well-defined.
    let max_iterations = max_iterations.filter(|&m| m > 0);
    let nudge_after = nudge_after.max(1);
    let nudge_interval = nudge_interval.max(1);

    let mut state_guard = state.write().await;
    state_guard.config.agent.max_iterations = max_iterations;
    state_guard.config.agent.nudge_after_iterations = nudge_after;
    state_guard.config.agent.nudge_interval_iterations = nudge_interval;
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;
    info!(
        "Agent iteration policy set: max={:?}, nudge_after={}, nudge_interval={}",
        max_iterations, nudge_after, nudge_interval
    );
    Ok(())
}

/// Export config as TOML string
#[tauri::command]
pub async fn export_config(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<String, String> {
    let state_guard = state.read().await;
    toml::to_string_pretty(&state_guard.config)
        .map_err(|e| format!("Failed to serialize config: {}", e))
}

/// Import config from TOML string
#[tauri::command]
pub async fn import_config(
    state: State<'_, Arc<RwLock<AppState>>>,
    config: String,
) -> Result<(), String> {
    let new_config: nanna_config::Config = toml::from_str(&config)
        .map_err(|e| format!("Failed to parse config: {}", e))?;

    let mut state_guard = state.write().await;
    state_guard.config = new_config;
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    info!("Config imported from TOML");
    Ok(())
}

// =============================================================================
// Model Priority (Fallback Chains)
// =============================================================================

/// Get chat model priority list
#[tauri::command]
pub async fn get_chat_model_priority(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<String>, String> {
    let state_guard = state.read().await;
    Ok(state_guard.config.llm.model_priority.clone())
}

/// Set chat model priority list
#[tauri::command]
pub async fn set_chat_model_priority(
    app: AppHandle,
    state: State<'_, Arc<RwLock<AppState>>>,
    priority: Vec<String>,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.llm.model_priority = priority.clone();

    // Also set the primary model to the first in the list for backwards compatibility
    let new_active = priority.first().cloned().unwrap_or_default();
    if !new_active.is_empty() {
        state_guard.config.llm.model = new_active.clone();
    }

    // Update active_model so the badge reflects the change immediately
    {
        let mut active = state_guard.active_model.write().await;
        *active = new_active.clone();
    }

    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    // Propagate to the daemon so changes take effect without restart
    let _ = state_guard.backend.config_set(
        "llm.model_priority",
        serde_json::to_value(&priority).unwrap_or_default(),
    ).await;

    // Emit model-status event so the GUI badge updates
    let _ = app.emit("model-status", ModelStatusEvent {
        active_model: new_active,
        fallback_reason: None,
        rate_limited_models: vec![],
    });

    info!("Chat model priority set: {:?}", priority);
    Ok(())
}

/// Get embedding model priority list
#[tauri::command]
pub async fn get_embedding_model_priority(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<String>, String> {
    let state_guard = state.read().await;
    Ok(state_guard.config.memory.embedding_priority.clone())
}

/// Set embedding model priority list
#[tauri::command]
pub async fn set_embedding_model_priority(
    state: State<'_, Arc<RwLock<AppState>>>,
    priority: Vec<String>,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.memory.embedding_priority = priority.clone();

    // Update the primary embedding config for backwards compatibility
    if let Some(first) = priority.first() {
        if let Some((provider, model)) = first.split_once('/') {
            state_guard.config.memory.embedding_provider = provider.to_string();
            state_guard.config.memory.embedding_model = model.to_string();
        }
    } else {
        state_guard.config.memory.embedding_provider = "disabled".to_string();
    }

    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    info!("Embedding model priority set: {:?}", priority);
    Ok(())
}

/// Get summarization model priority list
#[tauri::command]
pub async fn get_summarization_model_priority(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<String>, String> {
    let state_guard = state.read().await;
    Ok(state_guard.config.llm.summarization_priority.clone())
}

/// Set summarization model priority list
#[tauri::command]
pub async fn set_summarization_model_priority(
    state: State<'_, Arc<RwLock<AppState>>>,
    priority: Vec<String>,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.llm.summarization_priority = priority.clone();

    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    info!("Summarization model priority set: {:?}", priority);
    Ok(())
}

// =============================================================================
// OCR Configuration Commands
// =============================================================================

/// Get OCR model priority list (vision-capable models used for text extraction)
#[tauri::command]
pub async fn get_ocr_model_priority(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<String>, String> {
    let state_guard = state.read().await;
    Ok(state_guard.config.memory.ocr_model_priority.clone())
}

/// Set OCR model priority list
#[tauri::command]
pub async fn set_ocr_model_priority(
    state: State<'_, Arc<RwLock<AppState>>>,
    priority: Vec<String>,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.memory.ocr_model_priority = priority.clone();

    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    info!("OCR model priority set: {:?}", priority);
    Ok(())
}

/// Get whether embedded OCR (ocrs) is enabled
#[tauri::command]
pub async fn get_use_embedded_ocr(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<bool, String> {
    let state_guard = state.read().await;
    Ok(state_guard.config.memory.use_embedded_ocr)
}

/// Set whether embedded OCR (ocrs) is enabled
#[tauri::command]
pub async fn set_use_embedded_ocr(
    state: State<'_, Arc<RwLock<AppState>>>,
    enabled: bool,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.memory.use_embedded_ocr = enabled;

    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    info!("Embedded OCR (ocrs) set to: {}", enabled);
    Ok(())
}

// =============================================================================
// Model Routing Commands
// =============================================================================

/// Get model routing configuration
#[tauri::command]
pub async fn get_model_routing(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<String>, String> {
    let state_guard = state.read().await;
    Ok(state_guard.config.llm.model_routing.clone())
}

/// Set model routing configuration
/// Each entry is "model:tier" where tier is simple|medium|complex
#[tauri::command]
pub async fn set_model_routing(
    state: State<'_, Arc<RwLock<AppState>>>,
    routes: Vec<String>,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.llm.model_routing = routes.clone();

    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    // Propagate to the daemon
    let _ = state_guard.backend.config_set(
        "llm.model_routing",
        serde_json::to_value(&routes).unwrap_or_default(),
    ).await;

    info!("Model routing set: {:?}", routes);
    Ok(())
}

/// Get routing_first_turn_primary setting
#[tauri::command]
pub async fn get_routing_first_turn_primary(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<bool, String> {
    let state_guard = state.read().await;
    Ok(state_guard.config.llm.routing_first_turn_primary)
}

/// Set routing_first_turn_primary setting
#[tauri::command]
pub async fn set_routing_first_turn_primary(
    state: State<'_, Arc<RwLock<AppState>>>,
    enabled: bool,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.llm.routing_first_turn_primary = enabled;

    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    // Propagate to the daemon
    let _ = state_guard.backend.config_set(
        "llm.routing_first_turn_primary",
        serde_json::Value::Bool(enabled),
    ).await;

    info!("Routing first turn primary set: {}", enabled);
    Ok(())
}

/// Get sub-agent model
#[tauri::command]
pub async fn get_sub_agent_model(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Option<String>, String> {
    let state_guard = state.read().await;
    Ok(state_guard.config.llm.sub_agent_model.clone())
}

/// Set sub-agent model (None = use primary model)
#[tauri::command]
pub async fn set_sub_agent_model(
    state: State<'_, Arc<RwLock<AppState>>>,
    model: Option<String>,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    // Treat empty string as None
    let model = model.filter(|m| !m.is_empty());
    state_guard.config.llm.sub_agent_model = model.clone();

    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    // Propagate to the daemon
    let _ = state_guard.backend.config_set(
        "llm.sub_agent_model",
        model.map(serde_json::Value::String).unwrap_or(serde_json::Value::Null),
    ).await;

    info!("Sub-agent model set: {:?}", state_guard.config.llm.sub_agent_model);
    Ok(())
}

// =============================================================================
// Config Persistence Commands
// =============================================================================

/// Save config to disk
#[tauri::command]
pub async fn save_config(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    let state_guard = state.read().await;
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    info!("Config saved to disk");
    Ok(())
}
