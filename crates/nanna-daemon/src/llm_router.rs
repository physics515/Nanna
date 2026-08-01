//! Multi-provider LLM router
//!
//! Routes model requests to the appropriate LLM client based on model name/prefix.
//! Supports fallback across multiple providers.
//! Includes health-aware model selection (stats-informed routing).

use nanna_agent::ModelStatsTracker;
use nanna_config::credentials::{SecureStore, keys};
use nanna_llm::{LlmClient, ModelInfo, ModelInfoCache, CompletionRequest, LlmError};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, debug, warn};

/// Provider identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ProviderId {
    Anthropic,
    OpenAI,
    OpenRouter,
    GitHubModels,
    Ollama,
}

impl ProviderId {
    /// Stable lowercase name, matching the provider ids the GUI uses
    /// (`anthropic`, `openai`, `openrouter`, `github`, `ollama`).
    pub const fn name(self) -> &'static str {
        match self {
            ProviderId::Anthropic => "anthropic",
            ProviderId::OpenAI => "openai",
            ProviderId::OpenRouter => "openrouter",
            ProviderId::GitHubModels => "github",
            ProviderId::Ollama => "ollama",
        }
    }

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
    /// Available providers and their clients.
    ///
    /// Behind a lock so the provider set can be rebuilt at runtime (config
    /// reload after the user authenticates a new provider) while the router is
    /// shared as `Arc<LlmRouter>` across the daemon. Guards are held only for
    /// map access — never across an await — so a std lock suffices.
    providers: RwLock<HashMap<ProviderId, Arc<LlmClient>>>,
    /// Model info cache
    model_cache: Option<ModelInfoCache>,
    /// Shared model stats tracker for health-aware routing (set post-init)
    stats: Arc<tokio::sync::RwLock<Option<ModelStatsTracker>>>,
}

impl LlmRouter {
    /// Create a new router with no providers
    pub fn new() -> Self {
        let model_cache = ModelInfoCache::default_location();
        Self {
            providers: RwLock::new(HashMap::new()),
            model_cache,
            stats: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// Set the stats tracker (can be called after construction, even behind Arc)
    pub async fn set_stats(&self, stats: ModelStatsTracker) {
        *self.stats.write().await = Some(stats);
    }

    fn insert_provider(&self, provider: ProviderId, client: LlmClient) {
        self.providers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(provider, Arc::new(client));
    }

    /// Snapshot the client for one provider (guard dropped before return, so
    /// callers can await on the client freely).
    fn client_for(&self, provider: ProviderId) -> Option<Arc<LlmClient>> {
        self.providers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&provider)
            .cloned()
    }

    /// Rebuild the provider set from resolved credentials, replacing the
    /// current map. Safe to call on a shared `Arc<LlmRouter>`; requests
    /// in flight keep the client `Arc` they already snapshotted.
    ///
    /// Returns `(added, removed)` provider ids (sorted) and logs the diff —
    /// this is the runtime path that config reload drives, so a provider the
    /// user authenticates after boot registers without a daemon restart.
    pub fn rebuild(&self, creds: &ProviderCredentials) -> (Vec<ProviderId>, Vec<ProviderId>) {
        let mut new_map: HashMap<ProviderId, Arc<LlmClient>> = HashMap::new();

        match &creds.anthropic {
            Some(AnthropicCredential::OAuth(token)) => {
                new_map.insert(
                    ProviderId::Anthropic,
                    Arc::new(LlmClient::anthropic_oauth(token)),
                );
            }
            Some(AnthropicCredential::ApiKey(key)) => {
                new_map.insert(ProviderId::Anthropic, Arc::new(LlmClient::anthropic(key)));
            }
            None => {}
        }
        if let Some(ref key) = creds.openai_api_key {
            new_map.insert(ProviderId::OpenAI, Arc::new(LlmClient::openai(key)));
        }
        if let Some(ref key) = creds.openrouter_api_key {
            new_map.insert(ProviderId::OpenRouter, Arc::new(LlmClient::openrouter(key)));
        }
        if let Some(ref token) = creds.github_token {
            new_map.insert(
                ProviderId::GitHubModels,
                Arc::new(LlmClient::github_models(token)),
            );
        }
        // Ollama needs no credential: a local instance is always addressable.
        let ollama = match creds.ollama_api_key {
            Some(ref key) => LlmClient::ollama_with_key(&creds.ollama_host, key),
            None => LlmClient::ollama(&creds.ollama_host),
        };
        new_map.insert(ProviderId::Ollama, Arc::new(ollama));

        let (mut added, mut removed) = {
            let mut guard = self
                .providers
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let added: Vec<ProviderId> = new_map
                .keys()
                .filter(|p| !guard.contains_key(p))
                .copied()
                .collect();
            let removed: Vec<ProviderId> = guard
                .keys()
                .filter(|p| !new_map.contains_key(p))
                .copied()
                .collect();
            *guard = new_map;
            (added, removed)
        };
        added.sort_unstable();
        removed.sort_unstable();

        if added.is_empty() && removed.is_empty() {
            debug!("LLM provider rebuild: no membership change");
        } else {
            info!(
                "LLM provider rebuild: added={:?}, removed={:?}, now={:?}",
                added,
                removed,
                self.available_providers_sorted()
            );
        }
        (added, removed)
    }

    /// Add an Anthropic provider
    pub fn with_anthropic(self, api_key: &str) -> Self {
        info!("Adding Anthropic provider to router");
        self.insert_provider(ProviderId::Anthropic, LlmClient::anthropic(api_key));
        self
    }

    /// Add an Anthropic provider with OAuth
    pub fn with_anthropic_oauth(self, oauth_token: &str) -> Self {
        info!("Adding Anthropic OAuth provider to router");
        self.insert_provider(
            ProviderId::Anthropic,
            LlmClient::anthropic_oauth(oauth_token),
        );
        self
    }

    /// Add an OpenAI provider
    pub fn with_openai(self, api_key: &str) -> Self {
        info!("Adding OpenAI provider to router");
        self.insert_provider(ProviderId::OpenAI, LlmClient::openai(api_key));
        self
    }

    /// Add an OpenRouter provider
    pub fn with_openrouter(self, api_key: &str) -> Self {
        info!("Adding OpenRouter provider to router");
        self.insert_provider(ProviderId::OpenRouter, LlmClient::openrouter(api_key));
        self
    }

    /// Add a GitHub Models provider
    pub fn with_github_models(self, token: &str) -> Self {
        info!("Adding GitHub Models provider to router");
        self.insert_provider(ProviderId::GitHubModels, LlmClient::github_models(token));
        self
    }

    /// Add an Ollama provider
    pub fn with_ollama(self, host: &str) -> Self {
        info!("Adding Ollama provider to router");
        self.insert_provider(ProviderId::Ollama, LlmClient::ollama(host));
        self
    }

    /// Add an Ollama provider with API key authentication
    pub fn with_ollama_authenticated(self, host: &str, api_key: &str) -> Self {
        info!("Adding Ollama provider to router (authenticated)");
        self.insert_provider(
            ProviderId::Ollama,
            LlmClient::ollama_with_key(host, api_key),
        );
        self
    }

    /// Check if a provider is available
    pub fn has_provider(&self, provider: ProviderId) -> bool {
        self.providers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(&provider)
    }

    /// Get all available providers
    pub fn available_providers(&self) -> Vec<ProviderId> {
        self.providers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .keys()
            .copied()
            .collect()
    }

    /// All available providers, sorted — for stable log lines and IPC payloads.
    pub fn available_providers_sorted(&self) -> Vec<ProviderId> {
        let mut providers = self.available_providers();
        providers.sort_unstable();
        providers
    }

    /// Check if we can handle a given model
    pub fn can_handle(&self, model: &str) -> bool {
        self.client_for(ProviderId::from_model(model)).is_some()
    }

    /// Get the client for a model
    pub fn client_for_model(&self, model: &str) -> Option<Arc<LlmClient>> {
        self.client_for(ProviderId::from_model(model))
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
        for provider in [
            ProviderId::Anthropic,
            ProviderId::OpenAI,
            ProviderId::OpenRouter,
            ProviderId::GitHubModels,
            ProviderId::Ollama,
        ] {
            if let Some(client) = self.client_for(provider) {
                return Some(client);
            }
        }
        None
    }

    /// Get model info for a model (routing to correct provider)
    pub async fn get_model_info(&self, model: &str) -> ModelInfo {
        let provider = ProviderId::from_model(model);
        let actual_model = ProviderId::strip_prefix(model);

        debug!("Getting model info for {} via {:?}", actual_model, provider);

        if let Some(client) = self.client_for(provider) {
            client.get_model_info(actual_model, self.model_cache.as_ref()).await
        } else {
            // Provider client missing: cache first, else universal floor (no name table).
            if let Some(cache) = self.model_cache.as_ref() {
                if let Some(info) = cache.get(actual_model) {
                    return info;
                }
            }
            nanna_llm::unknown_model_info(model, &format!("{:?}", provider))
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

        let client = self
            .client_for(provider)
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

/// The one resolved credential the Anthropic provider will use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnthropicCredential {
    /// OAuth access token (Claude Pro/Max via `claude setup-token` / CLI login)
    OAuth(String),
    /// Plain API key
    ApiKey(String),
}

/// Credentials resolved for provider construction.
///
/// Resolution (config → keyring → Claude CLI) is separated from client
/// construction ([`LlmRouter::rebuild`]) so construction is deterministic and
/// unit-testable, and so boot and config-reload share one chain instead of the
/// reload silently skipping registration (the boot-only split-brain this
/// module used to have).
#[derive(Debug, Clone)]
pub struct ProviderCredentials {
    pub anthropic: Option<AnthropicCredential>,
    pub openai_api_key: Option<String>,
    pub openrouter_api_key: Option<String>,
    pub github_token: Option<String>,
    pub ollama_host: String,
    pub ollama_api_key: Option<String>,
}

/// `Some(trimmed)` only for a non-blank value — a `Some("")` credential must
/// not register a provider that then fails every call.
fn non_empty(value: Option<&String>) -> Option<String> {
    value
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(String::from)
}

/// Keyring lookup with the same non-blank rule as config values.
fn store_credential(store: &SecureStore, key: &str) -> Option<String> {
    store.get(key).ok().and_then(|v| non_empty(Some(&v)))
}

/// Resolve the Anthropic credential.
///
/// OAuth mode: env override (`ANTHROPIC_OAUTH_TOKEN`, matching
/// `Config::load_secrets_from_store` precedence) → durable stores via
/// [`nanna_config::resolve_anthropic_oauth`] (envelope-aware: refreshes a
/// stale token and mirrors Claude CLI logins into the nanna store, instead
/// of registering a provider whose every call 401s) → config-held token →
/// fall through to an API key. Non-OAuth mode: config/keyring API key first,
/// then any durable OAuth credential (nanna store or Claude CLI login).
async fn resolve_anthropic(
    llm: &crate::server::LlmConfig,
    store: &SecureStore,
) -> Option<AnthropicCredential> {
    if llm.anthropic_use_oauth {
        if let Ok(v) = std::env::var("ANTHROPIC_OAUTH_TOKEN")
            && let Some(token) = non_empty(Some(&v))
        {
            debug!("Anthropic credential: OAuth token from env");
            return Some(AnthropicCredential::OAuth(token));
        }
        match nanna_config::resolve_anthropic_oauth(store).await {
            Ok(cred) => {
                debug!("Anthropic credential: OAuth from durable store (refreshed if stale)");
                return Some(AnthropicCredential::OAuth(cred.access_token));
            }
            Err(e) => {
                if let Some(token) = non_empty(llm.anthropic_oauth_token.as_ref()) {
                    warn!("OAuth resolution failed ({e}); using config-provided token");
                    return Some(AnthropicCredential::OAuth(token));
                }
                warn!("Anthropic OAuth enabled but no usable credential ({e}); trying API key");
            }
        }
    }
    if let Some(key) = non_empty(llm.anthropic_api_key.as_ref())
        .or_else(|| store_credential(store, keys::ANTHROPIC_API_KEY))
    {
        debug!("Anthropic credential: API key from config/keyring");
        return Some(AnthropicCredential::ApiKey(key));
    }
    if !llm.anthropic_use_oauth
        && let Ok(cred) = nanna_config::resolve_anthropic_oauth(store).await
    {
        debug!("Anthropic credential: durable OAuth fallback (nanna store / Claude CLI)");
        return Some(AnthropicCredential::OAuth(cred.access_token));
    }
    None
}

impl ProviderCredentials {
    /// Resolve provider credentials from the daemon's LLM config, falling back
    /// to the OS keyring and finally the Claude CLI's own credentials.
    ///
    /// Per provider, first source wins:
    /// - Anthropic: see [`resolve_anthropic`] — OAuth env/durable-store/config
    ///   chain (refreshing stale tokens) → config API key → keyring API key →
    ///   durable OAuth fallback. An enabled OAuth flag with a missing token
    ///   falls through — the boot chain used to dead-end there, registering no
    ///   Anthropic provider even though the CLI held valid credentials.
    /// - OpenAI / OpenRouter / GitHub: config key → keyring key.
    /// - Ollama: always present (host needs no credential; blank key = anonymous).
    pub async fn resolve(llm: &crate::server::LlmConfig) -> Self {
        let store = SecureStore::new();

        let anthropic = resolve_anthropic(llm, &store).await;

        Self {
            anthropic,
            openai_api_key: non_empty(llm.openai_api_key.as_ref())
                .or_else(|| store_credential(&store, keys::OPENAI_API_KEY)),
            openrouter_api_key: non_empty(llm.openrouter_api_key.as_ref())
                .or_else(|| store_credential(&store, keys::OPENROUTER_API_KEY)),
            github_token: non_empty(llm.github_token.as_ref())
                .or_else(|| store_credential(&store, keys::GITHUB_TOKEN)),
            ollama_host: llm.ollama_host.clone(),
            ollama_api_key: non_empty(llm.ollama_api_key.as_ref()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AnthropicCredential, LlmRouter, ProviderCredentials, ProviderId};

    /// The live failure this guards: a provider authenticated after boot must
    /// register on rebuild (no daemon restart), and one whose credential goes
    /// away must drop — with the untouched providers staying registered.
    #[test]
    fn rebuild_registers_and_removes_providers_at_runtime() {
        let router = LlmRouter::new();
        assert!(!router.has_provider(ProviderId::Anthropic));
        assert!(!router.can_handle("claude-fable-5"));

        let creds = ProviderCredentials {
            anthropic: Some(AnthropicCredential::OAuth("test-token".into())),
            openai_api_key: None,
            openrouter_api_key: Some("sk-or-test".into()),
            github_token: None,
            ollama_host: "http://localhost:11434".into(),
            ollama_api_key: None,
        };
        let (added, removed) = router.rebuild(&creds);
        assert_eq!(
            added,
            vec![
                ProviderId::Anthropic,
                ProviderId::OpenRouter,
                ProviderId::Ollama
            ]
        );
        assert!(removed.is_empty());
        // The exact live symptom: a bare Claude model name must now route.
        assert!(router.can_handle("claude-fable-5"));

        let creds = ProviderCredentials {
            anthropic: None,
            ..creds
        };
        let (added, removed) = router.rebuild(&creds);
        assert!(added.is_empty());
        assert_eq!(removed, vec![ProviderId::Anthropic]);
        assert!(!router.can_handle("claude-fable-5"));
        assert!(router.has_provider(ProviderId::OpenRouter));
        assert!(router.has_provider(ProviderId::Ollama));
    }

    #[test]
    fn blank_credentials_never_count() {
        // A Some("") key must not register a provider that 401s on every call.
        assert_eq!(super::non_empty(Some(&"   ".to_string())), None);
        assert_eq!(super::non_empty(None), None);
        assert_eq!(
            super::non_empty(Some(&" sk-x ".to_string())),
            Some("sk-x".to_string())
        );
    }

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
