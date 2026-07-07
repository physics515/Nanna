//! Multi-provider LLM router
//!
//! Routes model requests to the appropriate LLM client based on model name/prefix.
//! Supports fallback across multiple providers.
//! Includes health-aware model selection (stats-informed routing).

use nanna_agent::ModelStatsTracker;
use nanna_llm::{LlmClient, ModelInfo, ModelInfoCache, CompletionRequest, LlmError};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, debug, warn};

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
    pub fn from_model(model: &str) -> Self {
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
        } else if lower.contains(':') {
            // Tag notation (e.g., "deepseek-r1:14b", "llama3.2:latest") = local Ollama model
            ProviderId::Ollama
        } else {
            // Default to Anthropic for unknown models
            ProviderId::Anthropic
        }
    }

    /// Strip provider prefix from model name (e.g., "ollama/deepseek-r1:14b" -> "deepseek-r1:14b")
    pub fn strip_prefix(model: &str) -> &str {
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

/// Model health status for routing decisions
#[derive(Debug, Clone, Serialize)]
pub enum ModelHealth {
    /// Model is working normally
    Healthy,
    /// Model has elevated errors or latency but is still usable
    Degraded(String),
    /// Model is failing consistently and should be skipped
    Unhealthy(String),
    /// Model was unhealthy but is in a cooldown/recovery period
    Cooldown {
        reason: String,
        /// Timestamp (epoch ms) after which to retry
        retry_after_ms: u64,
    },
}

impl ModelHealth {
    /// Whether this model should be used for new requests
    pub fn is_usable(&self) -> bool {
        match self {
            ModelHealth::Healthy | ModelHealth::Degraded(_) => true,
            ModelHealth::Unhealthy(_) => false,
            ModelHealth::Cooldown { retry_after_ms, .. } => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                now >= *retry_after_ms
            }
        }
    }
}

/// Multi-provider LLM router
pub struct LlmRouter {
    /// Available providers and their clients
    providers: HashMap<ProviderId, Arc<LlmClient>>,
    /// Model info cache
    model_cache: Option<ModelInfoCache>,
    /// Shared model stats tracker for health-aware routing (set post-init)
    stats: Arc<tokio::sync::RwLock<Option<ModelStatsTracker>>>,
}

impl Clone for LlmRouter {
    fn clone(&self) -> Self {
        Self {
            providers: self.providers.clone(),
            model_cache: self.model_cache.clone(),
            stats: self.stats.clone(),
        }
    }
}

impl LlmRouter {
    /// Create a new router with no providers
    pub fn new() -> Self {
        let model_cache = ModelInfoCache::default_location();
        Self {
            providers: HashMap::new(),
            model_cache,
            stats: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// Set the stats tracker (can be called after construction, even behind Arc)
    pub async fn set_stats(&self, stats: ModelStatsTracker) {
        *self.stats.write().await = Some(stats);
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

    /// Add an Ollama provider with API key authentication
    pub fn with_ollama_authenticated(mut self, host: &str, api_key: &str) -> Self {
        info!("Adding Ollama provider to router (authenticated)");
        self.providers.insert(ProviderId::Ollama, Arc::new(LlmClient::ollama_with_key(host, api_key)));
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

    /// Strip provider prefix from a model name.
    /// Public convenience method for use by agent_service and other consumers.
    /// e.g., "ollama/deepseek-r1:14b" -> "deepseek-r1:14b"
    pub fn strip_model_prefix(model: &str) -> String {
        ProviderId::strip_prefix(model).to_string()
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

    /// Check the health of a model based on recent stats.
    ///
    /// Thresholds:
    /// - Unhealthy: success_rate < 50% with 5+ requests, or 5+ consecutive failures
    /// - Degraded: success_rate < 80% or avg latency > 30s
    /// - Cooldown: was unhealthy, apply exponential backoff before retry
    pub async fn model_health(&self, model: &str) -> ModelHealth {
        let stats_guard = self.stats.read().await;
        let Some(ref stats) = *stats_guard else {
            return ModelHealth::Healthy; // No stats tracker, assume healthy
        };

        let summaries = stats.summaries().await;
        let summary = match summaries.iter().find(|s| s.model == model) {
            Some(s) => s,
            None => return ModelHealth::Healthy, // No data yet
        };

        // Not enough data to judge
        if summary.total_requests < 3 {
            return ModelHealth::Healthy;
        }

        let error_rate = 1.0 - summary.success_rate;
        let total_errors = (summary.total_requests as f64 * error_rate).round() as u64;

        // Check for consecutive failures (unhealthy → cooldown)
        if summary.consecutive_failures >= 5 {
            // Exponential cooldown: 30s * 2^(consecutive-5), capped at 10 min
            let exponent = (summary.consecutive_failures.saturating_sub(5)).min(8);
            let backoff_secs = (30u64).saturating_mul(1u64 << exponent).min(600);
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            // Estimate: cooldown started ~now (conservative; we don't have exact last-error time)
            let retry_after_ms = now_ms + (backoff_secs * 1000);

            return ModelHealth::Cooldown {
                reason: format!("{} consecutive failures", summary.consecutive_failures),
                retry_after_ms,
            };
        }

        // High error rate → unhealthy
        if error_rate > 0.5 && summary.total_requests >= 5 {
            return ModelHealth::Unhealthy(format!(
                "Success rate {:.0}% ({} errors in {} requests)",
                summary.success_rate * 100.0,
                total_errors,
                summary.total_requests
            ));
        }

        // Moderate error rate or high latency → degraded
        if error_rate > 0.2 {
            return ModelHealth::Degraded(format!(
                "Success rate {:.0}% ({} errors in {} requests)",
                summary.success_rate * 100.0,
                total_errors,
                summary.total_requests
            ));
        }

        if summary.avg_latency_ms > 30_000 {
            return ModelHealth::Degraded(format!(
                "High latency: {:.1}s avg",
                summary.avg_latency_ms as f64 / 1000.0
            ));
        }

        ModelHealth::Healthy
    }

    /// Filter a model priority list to prefer healthy models.
    ///
    /// Returns the reordered list: healthy models first (in original order),
    /// then degraded models, then cooldown-eligible models. Unhealthy models
    /// and models still in cooldown are excluded.
    pub async fn health_sorted_models(&self, models: &[String]) -> Vec<String> {
        let mut healthy = Vec::new();
        let mut degraded = Vec::new();
        let mut cooldown_ready = Vec::new();

        for model in models {
            let health = self.model_health(model).await;
            match health {
                ModelHealth::Healthy => healthy.push(model.clone()),
                ModelHealth::Degraded(_) => degraded.push(model.clone()),
                ModelHealth::Cooldown { .. } if health.is_usable() => {
                    cooldown_ready.push(model.clone());
                }
                ModelHealth::Cooldown { reason, .. } => {
                    debug!("Skipping model {} (cooldown: {})", model, reason);
                }
                ModelHealth::Unhealthy(reason) => {
                    warn!("Skipping unhealthy model {}: {}", model, reason);
                }
            }
        }

        // If all models are unhealthy, fall back to the original list
        // (better to try something than give up entirely)
        if healthy.is_empty() && degraded.is_empty() && cooldown_ready.is_empty() {
            warn!("All models unhealthy, falling back to original priority list");
            return models.to_vec();
        }

        let mut result = healthy;
        result.extend(degraded);
        result.extend(cooldown_ready);
        result
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

#[cfg(test)]
mod tests {
    use super::ProviderId;

    #[test]
    fn from_model_infers_provider_by_prefix() {
        // Bare model names route by family prefix (this is exactly what the GUI's
        // parse_model_id historically got wrong for OpenAI models).
        assert_eq!(ProviderId::from_model("gpt-4o"), ProviderId::OpenAI);
        assert_eq!(ProviderId::from_model("gpt-5"), ProviderId::OpenAI);
        assert_eq!(ProviderId::from_model("o1-preview"), ProviderId::OpenAI);
        assert_eq!(ProviderId::from_model("o3-mini"), ProviderId::OpenAI);
        assert_eq!(
            ProviderId::from_model("claude-opus-4"),
            ProviderId::Anthropic
        );
        // Case-insensitive.
        assert_eq!(
            ProviderId::from_model("Claude-Sonnet-4"),
            ProviderId::Anthropic
        );
    }

    #[test]
    fn from_model_recognizes_explicit_and_tagged_prefixes() {
        assert_eq!(
            ProviderId::from_model("openrouter/meta-llama/llama-3"),
            ProviderId::OpenRouter
        );
        assert_eq!(
            ProviderId::from_model("github/gpt-4o"),
            ProviderId::GitHubModels
        );
        assert_eq!(ProviderId::from_model("ollama/qwen3"), ProviderId::Ollama);
        // A `:tag` (with no known prefix) is a local Ollama model.
        assert_eq!(
            ProviderId::from_model("deepseek-r1:14b"),
            ProviderId::Ollama
        );
        assert_eq!(
            ProviderId::from_model("llama3.2:latest"),
            ProviderId::Ollama
        );
        // Unknown, prefix-less models fall back to Anthropic.
        assert_eq!(
            ProviderId::from_model("some-unknown-model"),
            ProviderId::Anthropic
        );
    }

    #[test]
    fn strip_prefix_removes_only_routing_prefixes() {
        assert_eq!(
            ProviderId::strip_prefix("ollama/deepseek-r1:14b"),
            "deepseek-r1:14b"
        );
        assert_eq!(ProviderId::strip_prefix("openrouter/x/y"), "x/y");
        assert_eq!(ProviderId::strip_prefix("github/gpt-4o"), "gpt-4o");
        // Family-named models keep their name (the family IS the model id).
        assert_eq!(ProviderId::strip_prefix("gpt-4o"), "gpt-4o");
        assert_eq!(ProviderId::strip_prefix("claude-opus-4"), "claude-opus-4");
    }
}
