//! Multi-provider LLM router
//!
//! Routes model requests to the appropriate LLM client based on model name/prefix.
//! Supports fallback across multiple providers.

use nanna_llm::{LlmClient, ModelInfo, ModelInfoCache, CompletionRequest, LlmError};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, debug};

/// Provider identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderId {
    Anthropic,
    OpenAI,
    OpenRouter,
    GitHubModels,
    Ollama,
}

impl ProviderId {
    /// Parse provider from model string prefix
    fn from_model(model: &str) -> Self {
        let lower = model.to_lowercase();

        if lower.starts_with("openrouter/") {
            ProviderId::OpenRouter
        } else if lower.starts_with("github/") {
            ProviderId::GitHubModels
        } else if lower.starts_with("ollama/") {
            ProviderId::Ollama
        } else if lower.starts_with("gpt-") || lower.starts_with("o1") || lower.starts_with("o3") {
            ProviderId::OpenAI
        } else if lower.starts_with("claude") {
            ProviderId::Anthropic
        } else {
            // Default to Anthropic for unknown models
            ProviderId::Anthropic
        }
    }

    /// Strip provider prefix from model name
    fn strip_prefix(model: &str) -> &str {
        if let Some(rest) = model.strip_prefix("openrouter/") {
            rest
        } else if let Some(rest) = model.strip_prefix("github/") {
            rest
        } else if let Some(rest) = model.strip_prefix("ollama/") {
            rest
        } else {
            model
        }
    }
}

/// Multi-provider LLM router
pub struct LlmRouter {
    /// Available providers and their clients
    providers: HashMap<ProviderId, Arc<LlmClient>>,
    /// Model info cache
    model_cache: Option<ModelInfoCache>,
}

impl LlmRouter {
    /// Create a new router with no providers
    pub fn new() -> Self {
        let model_cache = ModelInfoCache::default_location();
        Self {
            providers: HashMap::new(),
            model_cache,
        }
    }

    /// Add an Anthropic provider
    pub fn with_anthropic(mut self, api_key: &str) -> Self {
        info!("Adding Anthropic provider to router");
        self.providers.insert(ProviderId::Anthropic, Arc::new(LlmClient::anthropic(api_key)));
        self
    }

    /// Add an Anthropic provider with OAuth
    pub fn with_anthropic_oauth(mut self, oauth_token: &str) -> Self {
        info!("Adding Anthropic OAuth provider to router");
        self.providers.insert(ProviderId::Anthropic, Arc::new(LlmClient::anthropic_oauth(oauth_token)));
        self
    }

    /// Add an OpenAI provider
    pub fn with_openai(mut self, api_key: &str) -> Self {
        info!("Adding OpenAI provider to router");
        self.providers.insert(ProviderId::OpenAI, Arc::new(LlmClient::openai(api_key)));
        self
    }

    /// Add an OpenRouter provider
    pub fn with_openrouter(mut self, api_key: &str) -> Self {
        info!("Adding OpenRouter provider to router");
        self.providers.insert(ProviderId::OpenRouter, Arc::new(LlmClient::openrouter(api_key)));
        self
    }

    /// Add a GitHub Models provider
    pub fn with_github_models(mut self, token: &str) -> Self {
        info!("Adding GitHub Models provider to router");
        self.providers.insert(ProviderId::GitHubModels, Arc::new(LlmClient::github_models(token)));
        self
    }

    /// Add an Ollama provider
    pub fn with_ollama(mut self, host: &str) -> Self {
        info!("Adding Ollama provider to router");
        self.providers.insert(ProviderId::Ollama, Arc::new(LlmClient::ollama(host)));
        self
    }

    /// Check if a provider is available
    pub fn has_provider(&self, provider: ProviderId) -> bool {
        self.providers.contains_key(&provider)
    }

    /// Get all available providers
    pub fn available_providers(&self) -> Vec<ProviderId> {
        self.providers.keys().copied().collect()
    }

    /// Check if we can handle a given model
    pub fn can_handle(&self, model: &str) -> bool {
        let provider = ProviderId::from_model(model);
        self.providers.contains_key(&provider)
    }

    /// Get the client for a model
    pub fn client_for_model(&self, model: &str) -> Option<Arc<LlmClient>> {
        let provider = ProviderId::from_model(model);
        self.providers.get(&provider).cloned()
    }

    /// Get the primary LLM client (first available, preferring Anthropic).
    /// Used for sub-agent spawning where we need a client but don't know the model yet.
    pub fn primary_client(&self) -> Option<Arc<LlmClient>> {
        // Priority order: Anthropic > OpenAI > OpenRouter > GitHub > Ollama
        for provider in &[
            ProviderId::Anthropic,
            ProviderId::OpenAI,
            ProviderId::OpenRouter,
            ProviderId::GitHubModels,
            ProviderId::Ollama,
        ] {
            if let Some(client) = self.providers.get(provider) {
                return Some(client.clone());
            }
        }
        None
    }

    /// Get model info for a model (routing to correct provider)
    pub async fn get_model_info(&self, model: &str) -> ModelInfo {
        let provider = ProviderId::from_model(model);
        let actual_model = ProviderId::strip_prefix(model);

        debug!("Getting model info for {} via {:?}", actual_model, provider);

        if let Some(client) = self.providers.get(&provider) {
            client.get_model_info(actual_model, self.model_cache.as_ref()).await
        } else {
            // Return defaults if provider not available
            ModelInfo {
                id: model.to_string(),
                context_window: 128_000,
                max_output_tokens: 8192,
                supports_tools: true,
                supports_vision: false,
                embedding_dimension: None,
                provider: format!("{:?}", provider),
                cached_at: chrono::Utc::now().timestamp(),
            }
        }
    }

    /// Complete a request (routing to correct provider)
    pub async fn complete(&self, model: &str, request: CompletionRequest) -> Result<String, LlmError> {
        let provider = ProviderId::from_model(model);
        let actual_model = ProviderId::strip_prefix(model);

        debug!("Routing completion for {} to {:?}", actual_model, provider);

        let client = self.providers.get(&provider)
            .ok_or_else(|| LlmError::MissingApiKey(format!("{:?}", provider)))?;

        // Update model in request
        let mut request = request;
        request.model = actual_model.to_string();

        client.complete(&request).await
    }
}

impl Default for LlmRouter {
    fn default() -> Self {
        Self::new()
    }
}
