#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! Configuration management for Nanna

pub mod credentials;

pub use credentials::{
    ClaudeCredentialManager, CredentialError, CredentialSource, LoadedCredential, OAuthCredential,
};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
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
            model_priority: vec![
                "claude-sonnet-4-20250514".to_string(),
                "claude-3-5-sonnet-20241022".to_string(),
            ],
            anthropic_oauth_token: None,
            anthropic_use_oauth: false,
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
    /// Enable thinking/reasoning mode
    pub thinking_enabled: bool,
    /// Enable streaming responses
    pub streaming_enabled: bool,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: "Nanna".to_string(),
            system_prompt: None,
            personality_mode: "balanced".to_string(),
            thinking_enabled: false,
            streaming_enabled: true,
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
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            enabled: vec!["*".to_string()], // All tools enabled by default
            disabled: Vec::new(),
            exec_allowlist: None,
            file_sandbox: None,
            brave_api_key: None,
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
}

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
            Ok(Self::default())
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
        let config: Self = toml::from_str(&content)?;
        info!("Loaded config from {path:?}");
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
    pub fn save_to(&self, path: &PathBuf) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        info!("Saved config to {path:?}");
        Ok(())
    }

    /// Get default config path.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::NoDirFound` if the system config directory cannot be determined.
    pub fn default_config_path() -> Result<PathBuf, ConfigError> {
        let dirs =
            ProjectDirs::from("bot", "clawd", "Nanna").ok_or(ConfigError::NoDirFound)?;
        Ok(dirs.config_dir().join("config.toml"))
    }

    /// Get default data directory.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::NoDirFound` if the system data directory cannot be determined.
    pub fn default_data_dir() -> Result<PathBuf, ConfigError> {
        let dirs =
            ProjectDirs::from("bot", "clawd", "Nanna").ok_or(ConfigError::NoDirFound)?;
        Ok(dirs.data_dir().to_path_buf())
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
