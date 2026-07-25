#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! Configuration management for Nanna

pub mod credentials;
pub mod bind;

/// Canonical application identity for [`directories::ProjectDirs`].
///
/// Every Nanna surface (config, credentials, daemon data, model cache, GUI skills)
/// MUST use these three components so a single uninstall removes the whole tree and
/// so secrets never end up orphaned under a different vendor slug.
pub const APP_QUALIFIER: &str = "com";
pub const APP_ORGANIZATION: &str = "nanna";
pub const APP_NAME: &str = "nanna";

/// Build the canonical [`ProjectDirs`] for Nanna.
///
/// # Errors
///
/// Returns `None` only when the host has no home directory.
#[must_use]
pub fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from(APP_QUALIFIER, APP_ORGANIZATION, APP_NAME)
}

/// Legacy pre-unification identity (`bot/clawd/Nanna`). Kept solely so the first
/// boot after upgrade can migrate existing config and credential files into the
/// canonical tree instead of stranding a user's data under the old vendor slug.
#[must_use]
pub fn legacy_clawd_project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("bot", "clawd", "Nanna")
}


pub use bind::{LOOPBACK_HOST, is_loopback_host};
pub use credentials::{
    ClaudeCredentialManager, CredentialError, CredentialSource, LoadedCredential, OAuthCredential,
};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::info;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),
    #[error("TOML serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
    #[error("Config directory not found")]
    NoDirFound,
    #[error("Missing required field: {0}")]
    MissingField(String),
}

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct Config {
    /// General settings
    pub general: GeneralConfig,
    /// LLM provider settings
    pub llm: LlmConfig,
    /// Agent personality settings
    pub agent: AgentConfig,
    /// Server settings
    pub server: ServerConfig,
    /// Channel configurations
    pub channels: ChannelsConfig,
    /// Tool settings
    pub tools: ToolsConfig,
    /// Memory settings  
    pub memory: MemoryConfig,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub name: String,
    pub log_level: String,
    pub workspace: Option<PathBuf>,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            name: "Nanna".to_string(),
            log_level: "info".to_string(),
            workspace: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    pub provider: String,
    pub model: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub max_tokens: u32,
    pub temperature: f32,
    /// OpenAI API key for embeddings (semantic memory)
    pub openai_api_key: Option<String>,
    /// OpenRouter API key for multi-provider access
    pub openrouter_api_key: Option<String>,
    /// GitHub token for GitHub Models
    pub github_token: Option<String>,
    /// Model priority list for fallback (first working model is used)
    /// Format: ["claude-opus-4-20250514", "claude-sonnet-4-20250514", "ollama/llama3.2"]
    pub model_priority: Vec<String>,
    /// Anthropic OAuth access token (alternative to API key)
    pub anthropic_oauth_token: Option<String>,
    /// Whether to use OAuth token instead of API key for Anthropic
    pub anthropic_use_oauth: bool,
    /// Model priority list for summarization (first working model is used)
    /// Format: ["ollama/llama3.2", "ollama/mistral", "claude-haiku"]
    /// If empty, truncates instead of summarizing
    pub summarization_priority: Vec<String>,
    /// Ollama server URL for summarization (if using ollama model)
    pub ollama_url: Option<String>,
    /// Ollama API key (optional — for remote/authenticated Ollama instances)
    pub ollama_api_key: Option<String>,
    /// Model routing priority for cost optimization.
    /// Format: ["model:tier", ...] where tier is simple|medium|complex.
    /// Cheapest models first. Empty = disabled (always use primary model).
    /// Example: ["claude-haiku-3-5-20241022:simple", "claude-sonnet-4-20250514:complex"]
    pub model_routing: Vec<String>,
    /// Whether to always use the primary model for the first iteration. Default: true.
    pub routing_first_turn_primary: bool,
    /// Model to use for sub-agent tasks (optional).
    /// When set, sub-agents spawned via the `task` tool use this cheaper model
    /// instead of the primary model. Format: "provider/model" e.g. "ollama/qwen3:4b"
    pub sub_agent_model: Option<String>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            api_key: None,
            base_url: None,
            max_tokens: 8192,
            temperature: 0.7,
            openai_api_key: None,
            openrouter_api_key: None,
            github_token: None,
            model_priority: vec![], // Empty - dynamically populated from available providers
            anthropic_oauth_token: None,
            anthropic_use_oauth: false,
            summarization_priority: vec![], // Empty = truncate instead of summarize
            ollama_url: Some("http://localhost:11434".to_string()),
            ollama_api_key: None,
            model_routing: vec![], // Empty = disabled (always use primary model)
            routing_first_turn_primary: true,
            sub_agent_model: None, // None = use primary model for sub-agents
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    /// Agent name (displayed in responses)
    pub name: String,
    /// Custom system prompt (None = use default)
    pub system_prompt: Option<String>,
    /// Personality mode: balanced, professional, casual, minimal
    pub personality_mode: String,
    /// Who the agent is (formerly per-workspace SOUL.md / IDENTITY.md).
    /// Injected into every session independent of workspace.
    #[serde(default)]
    pub persona: Option<String>,
    /// Who the user is (formerly per-workspace USER.md).
    /// Injected into every session independent of workspace.
    #[serde(default)]
    pub user_profile: Option<String>,
    /// Enable thinking/reasoning mode
    pub thinking_enabled: bool,
    /// Enable streaming responses
    pub streaming_enabled: bool,
    /// Absolute cap on agent-loop iterations (tool-call rounds). `None` = unlimited
    /// (the default) — the agent is a long-horizon worker; only Stop/cancel ends it.
    /// A value here is a pure runaway backstop for unattended runs.
    #[serde(default)]
    pub max_iterations: Option<usize>,
    /// Iteration at which the first escalating "wrap-up" soft nudge is injected.
    /// The loop is NOT stopped — the nudge only steers a possibly-stuck model.
    /// Default: 500.
    #[serde(default = "default_nudge_after")]
    pub nudge_after_iterations: usize,
    /// After the first nudge, inject a further (more urgent) nudge every N
    /// iterations. Default: 100.
    #[serde(default = "default_nudge_interval")]
    pub nudge_interval_iterations: usize,
}

fn default_nudge_after() -> usize {
    500
}

fn default_nudge_interval() -> usize {
    100
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: "Nanna".to_string(),
            system_prompt: None,
            personality_mode: "balanced".to_string(),
            persona: None,
            user_profile: None,
            thinking_enabled: false,
            streaming_enabled: true,
            max_iterations: None,
            nudge_after_iterations: default_nudge_after(),
            nudge_interval_iterations: default_nudge_interval(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub webhook_secret: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            host: "0.0.0.0".to_string(),
            port: 3000,
            webhook_secret: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ChannelsConfig {
    pub telegram: Option<TelegramConfig>,
    pub discord: Option<DiscordConfig>,
    pub slack: Option<SlackConfig>,
    pub signal: Option<SignalConfig>,
    pub whatsapp: Option<WhatsAppConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub bot_token: String,
    pub webhook_url: Option<String>,
    pub allowed_users: Option<Vec<i64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordConfig {
    pub bot_token: String,
    pub application_id: String,
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackConfig {
    pub bot_token: String,
    pub app_token: Option<String>,
    pub signing_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalConfig {
    /// Phone number registered with Signal (e.g., "+1234567890")
    pub phone_number: String,
    /// URL of signal-cli-rest-api instance
    pub api_url: Option<String>,
    /// Allowed phone numbers (None = allow all)
    pub allowed_numbers: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsAppConfig {
    /// Connection method: "cloud-api" or "web"
    pub connection_method: String,
    /// Phone Number ID (for Cloud API)
    pub phone_number_id: Option<String>,
    /// Access token (for Cloud API)
    pub access_token: Option<String>,
    /// Webhook verify token (for Cloud API)
    pub verify_token: Option<String>,
    /// Session name (for Web bridge)
    pub session_name: Option<String>,
    /// Allowed phone numbers (None = allow all)
    pub allowed_contacts: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsConfig {
    pub enabled: Vec<String>,
    pub disabled: Vec<String>,
    pub exec_allowlist: Option<Vec<String>>,
    pub file_sandbox: Option<PathBuf>,
    /// Brave Search API key for web_search tool
    pub brave_api_key: Option<String>,
    /// Use TypeScript skill implementations instead of Rust builtins
    pub use_script_tools: bool,
    /// Directory containing tool scripts (default: {data_dir}/tools/)
    /// Can be overridden with NANNA_TOOLS_DIR environment variable
    pub tools_dir: Option<PathBuf>,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            enabled: vec!["*".to_string()], // All tools enabled by default
            disabled: Vec::new(),
            exec_allowlist: None,
            file_sandbox: None,
            brave_api_key: None,
            use_script_tools: true,
            tools_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    pub enabled: bool,
    /// Embedding provider: "openai", "ollama", or "disabled"
    pub embedding_provider: String,
    /// Embedding model name
    pub embedding_model: String,
    pub vector_dimension: usize,
    pub storage_path: Option<PathBuf>,
    /// Ollama server URL (used for both chat and embeddings)
    pub ollama_host: String,
    /// Model to use for memory extraction (empty = use chat model)
    pub extraction_model: String,
    /// Embedding model priority list for fallback
    /// Format: ["openai/text-embedding-3-small", "ollama/nomic-embed-text"]
    pub embedding_priority: Vec<String>,
    /// Maximum fraction of memories that can be removed in a single consolidation run (0.0-1.0).
    /// Default: 0.50 (50%)
    #[serde(default = "default_max_compression_ratio")]
    pub max_compression_ratio: f32,
    /// Minimum number of memories to retain after consolidation (hard floor).
    /// Default: 20
    #[serde(default = "default_min_remaining_memories")]
    pub min_remaining_memories: usize,

    /// Seconds the daemon must be idle (no chat activity) before the scheduled
    /// dream cycle is allowed to run. Dreaming competes with the live agent for
    /// the summarizer model and rewrites the store mid-conversation, so it waits
    /// for a genuine lull. Default: 300 (5 min).
    /// When true, every user/assistant turn (≥3 words) is written into long-term
    /// memory automatically. **Default false** — memory accumulation is opt-in so
    /// a first-run install does not silently hoover conversation content. The
    /// agent can still `remember` deliberately via the tool, and extraction of
    /// explicit memories during a run is controlled separately.
    #[serde(default)]
    pub auto_remember_messages: bool,

    #[serde(default = "default_dream_idle_threshold_secs")]
    pub dream_idle_threshold_secs: u64,

    /// Live memory count at/above which the scheduled dream cycle runs
    /// **regardless** of idle time (memory-pressure relief, so a continuously
    /// busy daemon still consolidates before the store grows unbounded). `0`
    /// disables the pressure override. Default: 5000.
    #[serde(default = "default_dream_memory_pressure_count")]
    pub dream_memory_pressure_count: usize,

    // -----------------------------------------------------------------------
    // OCR settings
    // -----------------------------------------------------------------------

    /// OCR model priority list for document text extraction.
    ///
    /// Format: `["ollama/llava", "anthropic/claude-opus-4-6"]`
    ///
    /// Models are tried in order after embedded OCR (if enabled).
    /// Only vision-capable models should be listed here.
    #[serde(default)]
    pub ocr_model_priority: Vec<String>,

    /// Whether to use the embedded `ocrs` pure-Rust OCR engine before
    /// falling through to the model priority list.
    ///
    /// Default: `true`.  The embedded engine handles Latin-script images
    /// offline with no API cost; disable it to force model-based OCR.
    #[serde(default = "default_use_embedded_ocr")]
    pub use_embedded_ocr: bool,
}

fn default_max_compression_ratio() -> f32 { 0.50 }
fn default_min_remaining_memories() -> usize { 20 }
fn default_dream_idle_threshold_secs() -> u64 { 300 }
fn default_dream_memory_pressure_count() -> usize { 5000 }
fn default_use_embedded_ocr() -> bool { true }

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            embedding_provider: "openai".to_string(),
            embedding_model: "text-embedding-3-small".to_string(),
            vector_dimension: 1536,
            storage_path: None,
            ollama_host: "http://localhost:11434".to_string(),
            extraction_model: String::new(), // Empty = use chat model
            embedding_priority: vec![
                "openai/text-embedding-3-small".to_string(),
            ],
            max_compression_ratio: default_max_compression_ratio(),
            min_remaining_memories: default_min_remaining_memories(),
            auto_remember_messages: false,
            dream_idle_threshold_secs: default_dream_idle_threshold_secs(),
            dream_memory_pressure_count: default_dream_memory_pressure_count(),
            ocr_model_priority: vec![],
            use_embedded_ocr: default_use_embedded_ocr(),
        }
    }
}

impl Config {
    /// Load config from default location.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError` if the config path cannot be determined or the file cannot be read.
    pub fn load() -> Result<Self, ConfigError> {
        let path = Self::default_config_path()?;
        if path.exists() {
            Self::load_from(&path)
        } else {
            let mut cfg = Self::default();
            cfg.load_secrets_from_store();
            Ok(cfg)
        }
    }

    /// Load config from a specific path.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::Io` if the file cannot be read.
    /// Returns `ConfigError::Parse` if the TOML is invalid.
    pub fn load_from(path: &PathBuf) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let mut config: Self = toml::from_str(&content)?;
        info!("Loaded config from {path:?}");
        config.load_secrets_from_store();
        Ok(config)
    }

    /// Save config to default location.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError` if the config path cannot be determined or the file cannot be written.
    pub fn save(&self) -> Result<(), ConfigError> {
        let path = Self::default_config_path()?;
        self.save_to(&path)
    }

    /// Save config to a specific path.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::Io` if the directory cannot be created or the file cannot be written.
    /// Returns `ConfigError::Parse` if the config cannot be serialized.
    pub fn save_to(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Never write secrets into config.toml. The secure store is the only
        // durable home for API keys; the in-memory Config may still hold them
        // for the running process (loaded from keyring/env at boot).
        let mut disk = self.clone();
        disk.strip_secrets_for_disk();
        let contents = toml::to_string_pretty(&disk)?;
        fs::write(path, contents)?;
        Ok(())
    }

    /// Blank every secret field so a serialized config never contains
    /// credentials. Called by [`Self::save_to`].
    pub fn strip_secrets_for_disk(&mut self) {
        self.llm.api_key = None;
        self.llm.openai_api_key = None;
        self.llm.openrouter_api_key = None;
        self.llm.github_token = None;
        self.llm.anthropic_oauth_token = None;
        self.llm.ollama_api_key = None;
        self.tools.brave_api_key = None;
    }

    /// Persist any secret fields currently held in-memory into the OS keyring
    /// (or encrypted file fallback), then blank them on this Config. Call after
    /// onboarding / GUI key entry so `save()` never writes secrets to disk.
    pub fn migrate_secrets_to_keyring(&mut self) -> Result<(), crate::credentials::CredentialError> {
        use crate::credentials::{keys, SecureStore};
        let store = SecureStore::new();
        let put = |key: &str, val: &mut Option<String>| -> Result<(), crate::credentials::CredentialError> {
            if let Some(v) = val.take() {
                let trimmed = v.trim();
                if !trimmed.is_empty() {
                    store.set(key, trimmed)?;
                }
            }
            Ok(())
        };
        put(keys::ANTHROPIC_API_KEY, &mut self.llm.api_key)?;
        put(keys::OPENAI_API_KEY, &mut self.llm.openai_api_key)?;
        put(keys::OPENROUTER_API_KEY, &mut self.llm.openrouter_api_key)?;
        put(keys::GITHUB_TOKEN, &mut self.llm.github_token)?;
        put(keys::BRAVE_API_KEY, &mut self.tools.brave_api_key)?;
        // OAuth token + ollama key share the store under theirs-named keys too.
        if let Some(v) = self.llm.anthropic_oauth_token.take() {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                store.set("anthropic_oauth_token", trimmed)?;
            }
        }
        if let Some(v) = self.llm.ollama_api_key.take() {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                store.set("ollama_api_key", trimmed)?;
            }
        }
        Ok(())
    }

    /// Hydrate secret fields from SecureStore + environment if they are unset.
    /// Safe to call repeatedly; never overwrites a value already present.
    pub fn load_secrets_from_store(&mut self) {
        use crate::credentials::{keys, SecureStore};
        let store = SecureStore::new();
        let fill = |slot: &mut Option<String>, key: &str, env: &str| {
            if slot.as_ref().map(|s| !s.trim().is_empty()).unwrap_or(false) {
                return;
            }
            if let Ok(v) = std::env::var(env) {
                if !v.trim().is_empty() {
                    *slot = Some(v);
                    return;
                }
            }
            if let Ok(v) = store.get(key) {
                if !v.trim().is_empty() {
                    *slot = Some(v);
                }
            }
        };
        fill(&mut self.llm.api_key, keys::ANTHROPIC_API_KEY, "ANTHROPIC_API_KEY");
        fill(&mut self.llm.openai_api_key, keys::OPENAI_API_KEY, "OPENAI_API_KEY");
        fill(&mut self.llm.openrouter_api_key, keys::OPENROUTER_API_KEY, "OPENROUTER_API_KEY");
        fill(&mut self.llm.github_token, keys::GITHUB_TOKEN, "GITHUB_TOKEN");
        fill(&mut self.tools.brave_api_key, keys::BRAVE_API_KEY, "BRAVE_API_KEY");
        fill(&mut self.llm.ollama_api_key, "ollama_api_key", "OLLAMA_API_KEY");
        fill(&mut self.llm.anthropic_oauth_token, "anthropic_oauth_token", "ANTHROPIC_OAUTH_TOKEN");
    }

    /// Get default config path.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::NoDirFound` if the system config directory cannot be determined.
    pub fn default_config_path() -> Result<PathBuf, ConfigError> {
        Self::migrate_legacy_config_if_needed();
        let dirs = project_dirs().ok_or(ConfigError::NoDirFound)?;
        Ok(dirs.config_dir().join("config.toml"))
    }

    /// Get default data directory.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::NoDirFound` if the system data directory cannot be determined.
    pub fn default_data_dir() -> Result<PathBuf, ConfigError> {
        Self::migrate_legacy_config_if_needed();
        let dirs = project_dirs().ok_or(ConfigError::NoDirFound)?;
        Ok(dirs.data_dir().to_path_buf())
    }

    /// Copy config.toml from the legacy `bot/clawd/Nanna` tree into the
    /// canonical `com/nanna/nanna` tree when the latter does not yet exist.
    /// Best-effort and silent on failure — a failed migrate leaves the user on
    /// defaults rather than refusing to start.
    fn migrate_legacy_config_if_needed() {
        let Some(new_dirs) = project_dirs() else { return };
        let new_cfg = new_dirs.config_dir().join("config.toml");
        if new_cfg.exists() {
            return;
        }
        let Some(old_dirs) = legacy_clawd_project_dirs() else { return };
        let old_cfg = old_dirs.config_dir().join("config.toml");
        if !old_cfg.exists() {
            return;
        }
        if let Some(parent) = new_cfg.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match fs::copy(&old_cfg, &new_cfg) {
            Ok(_) => tracing::info!(
                from = %old_cfg.display(),
                to = %new_cfg.display(),
                "Migrated config.toml from legacy clawd path to com.nanna.nanna"
            ),
            Err(e) => tracing::warn!(
                error = %e,
                "Failed to migrate legacy config.toml; continuing with defaults"
            ),
        }
        // Best-effort: also migrate the data dir contents the first time.
        let old_data = old_dirs.data_dir();
        let new_data = new_dirs.data_dir();
        if old_data.exists() && !new_data.exists() {
            if let Err(e) = copy_dir_recursive(old_data, new_data) {
                tracing::warn!(error = %e, "Failed to migrate legacy data dir");
            } else {
                tracing::info!(
                    from = %old_data.display(),
                    to = %new_data.display(),
                    "Migrated data directory from legacy clawd path"
                );
            }
        }
    }

    /// Override config with environment variables
    #[must_use] 
    pub fn with_env_overrides(mut self) -> Self {
        // LLM API keys
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            self.llm.api_key = Some(key);
        }
        if let Ok(key) = std::env::var("OPENAI_API_KEY")
            && self.llm.provider == "openai" {
                self.llm.api_key = Some(key);
            }

        // Server config
        if let Ok(port) = std::env::var("PORT")
            && let Ok(p) = port.parse() {
                self.server.port = p;
            }

        // Telegram
        if let Ok(token) = std::env::var("TELEGRAM_BOT_TOKEN") {
            self.channels.telegram = Some(TelegramConfig {
                bot_token: token,
                webhook_url: std::env::var("TELEGRAM_WEBHOOK_URL").ok(),
                allowed_users: None,
            });
        }

        // Discord
        if let Ok(token) = std::env::var("DISCORD_BOT_TOKEN")
            && let (Ok(app_id), Ok(pub_key)) = (
                std::env::var("DISCORD_APPLICATION_ID"),
                std::env::var("DISCORD_PUBLIC_KEY"),
            ) {
                self.channels.discord = Some(DiscordConfig {
                    bot_token: token,
                    application_id: app_id,
                    public_key: pub_key,
                });
            }

        self
    }
}

/// Generate a default config file content
#[must_use] 
pub fn generate_default_config() -> String {
    let config = Config::default();
    toml::to_string_pretty(&config).unwrap_or_default()
}

/// Recursively copy a directory tree. Used only for the one-shot legacy-path migrate.
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else if ty.is_file() {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_to_strips_secrets_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut cfg = Config::default();
        cfg.llm.api_key = Some("sk-secret-anthropic".into());
        cfg.llm.openai_api_key = Some("sk-secret-openai".into());
        cfg.llm.openrouter_api_key = Some("sk-secret-or".into());
        cfg.llm.github_token = Some("ghp_secret".into());
        cfg.llm.ollama_api_key = Some("ollama-secret".into());
        cfg.llm.anthropic_oauth_token = Some("oauth-secret".into());
        cfg.tools.brave_api_key = Some("brave-secret".into());
        cfg.save_to(&path).unwrap();

        let on_disk = std::fs::read_to_string(&path).unwrap();
        for needle in [
            "sk-secret-anthropic",
            "sk-secret-openai",
            "sk-secret-or",
            "ghp_secret",
            "ollama-secret",
            "oauth-secret",
            "brave-secret",
        ] {
            assert!(
                !on_disk.contains(needle),
                "secret {needle:?} leaked into config.toml: {on_disk}"
            );
        }
        // In-memory config is untouched so the running process still has the keys.
        assert_eq!(cfg.llm.api_key.as_deref(), Some("sk-secret-anthropic"));
    }

    #[test]
    fn auto_remember_messages_defaults_off() {
        assert!(!MemoryConfig::default().auto_remember_messages);
        // serde default for missing field is also false
        let parsed: MemoryConfig = toml::from_str("").unwrap();
        assert!(!parsed.auto_remember_messages);
    }

    #[test]
    fn project_dirs_uses_canonical_identity() {
        let dirs = project_dirs().expect("home dir");
        let cfg = dirs.config_dir().to_string_lossy().to_lowercase();
        // Windows: .../nanna/nanna ; Unix: .../nanna
        assert!(
            cfg.contains("nanna"),
            "config dir should contain nanna, got {cfg}"
        );
        assert!(
            !cfg.contains("clawd"),
            "canonical path must not use legacy clawd slug: {cfg}"
        );
    }
}
