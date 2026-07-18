//! Model id parsing and LLM client routing.
//!
//! `create_llm_client_for_model` caches `LlmClient`s by model id so we don't
//! rebuild a fresh `reqwest::Client` on every call. Entries are keyed by the
//! model id the caller passed, and invalidated when the credential fingerprint
//! for that entry no longer matches (key rotated, OAuth swapped, ollama host
//! changed, …). Callers that mutate credentials should also call
//! [`invalidate_llm_client_cache`] to drop stale secrets from memory promptly.

#[allow(clippy::wildcard_imports)]
use crate::*;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};

// =============================================================================
// Model Selection & Fallback
// =============================================================================

/// Parse a model ID into `(provider, model_name)`.
///
/// Explicit provider prefixes always win (`openrouter/…`, `github/…`, `ollama/…`,
/// `openai/…`, `anthropic/…`). Bare names are inferred from family prefixes:
/// `gpt-*` / `o1` / `o3` → openai, `claude*` → anthropic, a `:tag` (e.g.
/// `llama3.2:latest`) → ollama. Unknown bare names still default to anthropic,
/// matching the historical behavior for Claude-only installs.
pub(crate) fn parse_model_id(model_id: &str) -> (String, String) {
    assert!(!model_id.is_empty(), "model_id must be non-empty");

    // Explicit multi-segment provider prefixes first.
    if let Some(rest) = model_id.strip_prefix("openrouter/") {
        assert!(!rest.is_empty(), "openrouter model id missing model segment");
        return ("openrouter".into(), rest.to_string());
    }
    if let Some(rest) = model_id.strip_prefix("github/") {
        assert!(!rest.is_empty(), "github model id missing model segment");
        return ("github".into(), rest.to_string());
    }
    if let Some(rest) = model_id.strip_prefix("ollama/") {
        assert!(!rest.is_empty(), "ollama model id missing model segment");
        return ("ollama".into(), rest.to_string());
    }
    if let Some(rest) = model_id.strip_prefix("openai/") {
        assert!(!rest.is_empty(), "openai model id missing model segment");
        return ("openai".into(), rest.to_string());
    }
    if let Some(rest) = model_id.strip_prefix("anthropic/") {
        assert!(!rest.is_empty(), "anthropic model id missing model segment");
        return ("anthropic".into(), rest.to_string());
    }

    // Generic provider/model form for remaining named prefixes (`provider/model`).
    if let Some((provider, model)) = model_id.split_once('/') {
        if !provider.is_empty() && !model.is_empty() {
            return (provider.to_string(), model.to_string());
        }
    }

    // Bare model name — infer provider from the family prefix.
    let lower = model_id.to_ascii_lowercase();
    if lower.starts_with("gpt-") || lower.starts_with("o1") || lower.starts_with("o3") {
        return ("openai".into(), model_id.to_string());
    }
    if lower.starts_with("claude") {
        return ("anthropic".into(), model_id.to_string());
    }
    // Ollama tag notation (e.g. "deepseek-r1:14b", "llama3.2:latest").
    if lower.contains(':') {
        return ("ollama".into(), model_id.to_string());
    }

    // Historical default: bare unknowns go to Anthropic.
    ("anthropic".into(), model_id.to_string())
}

// =============================================================================
// LlmClient cache (by model ID, invalidated on credential change)
// =============================================================================

struct CacheEntry {
    /// Hash of the credentials / host used to build this client.
    fingerprint: u64,
    client: LlmClient,
    model_name: String,
}

fn client_cache() -> &'static Mutex<HashMap<String, CacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Drop every cached [`LlmClient`]. Call when credentials or ollama host change
/// so stale secrets are not retained and the next lookup rebuilds.
pub(crate) fn invalidate_llm_client_cache() {
    if let Ok(mut cache) = client_cache().lock() {
        cache.clear();
    }
}

/// Stable fingerprint of the material that goes into a provider's `LlmClient`.
///
/// Hashed so a debug dump of the cache can't leak raw keys. `None` when the
/// provider is missing credentials and no client can be built.
fn credential_fingerprint(
    provider: &str,
    config: &Config,
    ollama_host: &str,
) -> Option<u64> {
    // Discriminant tags finish the hash so empty-key collisions across modes
    // (e.g. oauth-with-empty vs key-with-empty) cannot share a fingerprint.
    match provider {
        "anthropic" => {
            // Must mirror build_llm_client_for_provider: OAuth when token is present,
            // otherwise fall back to API key even if use_oauth is set.
            if config.llm.anthropic_use_oauth {
                if let Some(token) = config.llm.anthropic_oauth_token.as_deref() {
                    return Some(hash_parts(&["anthropic", "oauth", token]));
                }
            }
            let api_key = config
                .llm
                .api_key
                .as_deref()
                .map(str::to_owned)
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())?;
            Some(hash_parts(&["anthropic", "key", &api_key]))
        }
        "openai" => {
            let api_key = config
                .llm
                .openai_api_key
                .as_deref()
                .map(str::to_owned)
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())?;
            Some(hash_parts(&["openai", &api_key]))
        }
        "openrouter" => {
            let api_key = config
                .llm
                .openrouter_api_key
                .as_deref()
                .map(str::to_owned)
                .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())?;
            Some(hash_parts(&["openrouter", &api_key]))
        }
        "github" => {
            let api_key = config
                .llm
                .github_token
                .as_deref()
                .map(str::to_owned)
                .or_else(|| std::env::var("GITHUB_TOKEN").ok())?;
            Some(hash_parts(&["github", &api_key]))
        }
        "claude-proxy" => {
            let proxy_url = std::env::var("CLAUDE_PROXY_URL")
                .unwrap_or_else(|_| "http://localhost:3456".to_string());
            Some(hash_parts(&["claude-proxy", &proxy_url]))
        }
        "ollama" => Some(hash_parts(&["ollama", ollama_host])),
        _ => None,
    }
}

fn hash_parts(parts: &[&str]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for part in parts {
        part.hash(&mut hasher);
    }
    hasher.finish()
}

/// Build a fresh client for `(provider, model_name)`. No caching.
fn build_llm_client_for_provider(
    provider: &str,
    model_name: String,
    config: &Config,
    ollama_host: &str,
) -> Option<(LlmClient, String)> {
    #[cfg(test)]
    test_support::record_build();

    match provider {
        "anthropic" => {
            // Check if OAuth is enabled and has a token
            if config.llm.anthropic_use_oauth {
                if let Some(ref oauth_token) = config.llm.anthropic_oauth_token {
                    return Some((LlmClient::anthropic_oauth(oauth_token), model_name));
                }
            }
            // Fall back to API key
            let api_key = config
                .llm
                .api_key
                .clone()
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())?;
            Some((LlmClient::anthropic(&api_key), model_name))
        }
        "openai" => {
            let api_key = config
                .llm
                .openai_api_key
                .clone()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())?;
            Some((LlmClient::openai(&api_key), model_name))
        }
        "openrouter" => {
            let api_key = config
                .llm
                .openrouter_api_key
                .clone()
                .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())?;
            Some((LlmClient::openrouter(&api_key), model_name))
        }
        "github" => {
            let api_key = config
                .llm
                .github_token
                .clone()
                .or_else(|| std::env::var("GITHUB_TOKEN").ok())?;
            Some((LlmClient::github_models(&api_key), model_name))
        }
        "claude-proxy" => {
            let proxy_url = std::env::var("CLAUDE_PROXY_URL")
                .unwrap_or_else(|_| "http://localhost:3456".to_string());
            Some((LlmClient::claude_proxy(&proxy_url), model_name))
        }
        "ollama" => Some((LlmClient::ollama(ollama_host), model_name)),
        _ => None,
    }
}

/// Create an LLM client for a specific model.
///
/// Clients are cached by `model_id`. A cache hit is returned only when the
/// current credential fingerprint matches the one used to build the entry —
/// rotating a key, toggling OAuth, or changing the ollama host rebuilds.
/// Call [`invalidate_llm_client_cache`] on credential mutation to drop stale
/// entries sooner than the lazy fingerprint miss.
pub(crate) fn create_llm_client_for_model(
    model_id: &str,
    config: &Config,
    ollama_host: &str,
) -> Option<(LlmClient, String)> {
    let (provider, model_name) = parse_model_id(model_id);
    let fingerprint = credential_fingerprint(&provider, config, ollama_host)?;

    // Fast path: reuse a live client whose credentials still match.
    if let Ok(cache) = client_cache().lock() {
        if let Some(entry) = cache.get(model_id) {
            if entry.fingerprint == fingerprint {
                return Some((entry.client.clone(), entry.model_name.clone()));
            }
        }
    }

    let (client, model_name) =
        build_llm_client_for_provider(&provider, model_name, config, ollama_host)?;

    if let Ok(mut cache) = client_cache().lock() {
        cache.insert(
            model_id.to_string(),
            CacheEntry {
                fingerprint,
                client: client.clone(),
                model_name: model_name.clone(),
            },
        );
    }

    Some((client, model_name))
}

/// Format Claude model IDs into friendly names
pub(crate) fn format_claude_model_name(id: &str) -> String {
    match id {
        "claude-opus-4-5-20251101" => "Claude Opus 4.5".to_string(),
        "claude-opus-4-20250514" => "Claude Opus 4".to_string(),
        "claude-sonnet-4-20250514" => "Claude Sonnet 4".to_string(),
        "claude-3-5-sonnet-20241022" => "Claude Sonnet 3.5".to_string(),
        "claude-3-5-haiku-20241022" => "Claude Haiku 3.5".to_string(),
        _ => id.to_string(),
    }
}

#[cfg(test)]
mod test_support {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Mutex, MutexGuard};

    static BUILD_COUNT: AtomicUsize = AtomicUsize::new(0);
    // Process-wide caches are shared across cargo's parallel tests; serialize
    // the cache tests so counters and flush order stay deterministic.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    pub(super) fn lock() -> MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn record_build() {
        BUILD_COUNT.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn build_count() -> usize {
        BUILD_COUNT.load(Ordering::Relaxed)
    }

    pub(super) fn reset_build_count() {
        BUILD_COUNT.store(0, Ordering::Relaxed);
    }

    pub(super) fn cache_len() -> usize {
        super::client_cache()
            .lock()
            .map(|c| c.len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod parse_model_id_tests {
    use super::parse_model_id;

    #[test]
    fn parse_model_id_infers_provider_by_family_prefix() {
        assert_eq!(
            parse_model_id("gpt-4o"),
            ("openai".into(), "gpt-4o".into())
        );
        assert_eq!(
            parse_model_id("o1-preview"),
            ("openai".into(), "o1-preview".into())
        );
        assert_eq!(
            parse_model_id("o3-mini"),
            ("openai".into(), "o3-mini".into())
        );
        assert_eq!(
            parse_model_id("claude-opus-4"),
            ("anthropic".into(), "claude-opus-4".into())
        );
        // Case-insensitive on the family prefix.
        assert_eq!(
            parse_model_id("Claude-Sonnet-4"),
            ("anthropic".into(), "Claude-Sonnet-4".into())
        );
        assert_eq!(
            parse_model_id("llama3.2:latest"),
            ("ollama".into(), "llama3.2:latest".into())
        );
        assert_eq!(
            parse_model_id("deepseek-r1:14b"),
            ("ollama".into(), "deepseek-r1:14b".into())
        );
        // Unknown bare names still default to Anthropic.
        assert_eq!(
            parse_model_id("some-unknown-model"),
            ("anthropic".into(), "some-unknown-model".into())
        );
    }

    #[test]
    fn parse_model_id_respects_explicit_provider_prefixes() {
        assert_eq!(
            parse_model_id("openrouter/meta-llama/llama-3"),
            ("openrouter".into(), "meta-llama/llama-3".into())
        );
        assert_eq!(
            parse_model_id("github/gpt-4o"),
            ("github".into(), "gpt-4o".into())
        );
        assert_eq!(
            parse_model_id("ollama/qwen3"),
            ("ollama".into(), "qwen3".into())
        );
        assert_eq!(
            parse_model_id("openai/gpt-4o"),
            ("openai".into(), "gpt-4o".into())
        );
        assert_eq!(
            parse_model_id("anthropic/claude-opus-4"),
            ("anthropic".into(), "claude-opus-4".into())
        );
    }
}

#[cfg(test)]
mod client_cache_tests {
    use super::{
        create_llm_client_for_model, invalidate_llm_client_cache, test_support,
    };
    use nanna_config::Config;

    fn anthropic_config(api_key: &str) -> Config {
        let mut config = Config::default();
        config.llm.api_key = Some(api_key.to_string());
        config.llm.anthropic_use_oauth = false;
        config.llm.anthropic_oauth_token = None;
        config
    }

    #[test]
    fn create_llm_client_reuses_cached_client_for_same_model_and_creds() {
        let _guard = test_support::lock();

        invalidate_llm_client_cache();
        test_support::reset_build_count();

        let config = anthropic_config("sk-test-reuse");
        let host = "http://localhost:11434";

        let first = create_llm_client_for_model("claude-opus-4", &config, host);
        assert!(first.is_some());
        assert_eq!(test_support::build_count(), 1);
        assert_eq!(test_support::cache_len(), 1);

        let second = create_llm_client_for_model("claude-opus-4", &config, host);
        assert!(second.is_some());
        // Cache hit — no fresh HTTP client.
        assert_eq!(test_support::build_count(), 1);
        assert_eq!(test_support::cache_len(), 1);
    }

    #[test]
    fn create_llm_client_rebuilds_when_credential_changes() {
        let _guard = test_support::lock();

        invalidate_llm_client_cache();
        test_support::reset_build_count();

        let host = "http://localhost:11434";
        let config_a = anthropic_config("sk-old");
        assert!(create_llm_client_for_model("claude-sonnet-4", &config_a, host).is_some());
        assert_eq!(test_support::build_count(), 1);

        let config_b = anthropic_config("sk-new");
        assert!(create_llm_client_for_model("claude-sonnet-4", &config_b, host).is_some());
        // Fingerprint miss forces a rebuild; cache stays size 1 (replaced entry).
        assert_eq!(test_support::build_count(), 2);
        assert_eq!(test_support::cache_len(), 1);
    }

    #[test]
    fn create_llm_client_caches_distinct_model_ids_separately() {
        let _guard = test_support::lock();

        invalidate_llm_client_cache();
        test_support::reset_build_count();

        let config = anthropic_config("sk-multi");
        let host = "http://localhost:11434";

        assert!(create_llm_client_for_model("claude-opus-4", &config, host).is_some());
        assert!(create_llm_client_for_model("claude-sonnet-4", &config, host).is_some());
        assert_eq!(test_support::build_count(), 2);
        assert_eq!(test_support::cache_len(), 2);

        // Hits for both.
        assert!(create_llm_client_for_model("claude-opus-4", &config, host).is_some());
        assert!(create_llm_client_for_model("claude-sonnet-4", &config, host).is_some());
        assert_eq!(test_support::build_count(), 2);
    }

    #[test]
    fn invalidate_llm_client_cache_forces_rebuild() {
        let _guard = test_support::lock();

        invalidate_llm_client_cache();
        test_support::reset_build_count();

        let config = anthropic_config("sk-invalidate");
        let host = "http://localhost:11434";

        assert!(create_llm_client_for_model("claude-opus-4", &config, host).is_some());
        assert_eq!(test_support::build_count(), 1);

        invalidate_llm_client_cache();
        assert_eq!(test_support::cache_len(), 0);

        assert!(create_llm_client_for_model("claude-opus-4", &config, host).is_some());
        assert_eq!(test_support::build_count(), 2);
        assert_eq!(test_support::cache_len(), 1);
    }

    #[test]
    fn create_llm_client_rebuilds_when_ollama_host_changes() {
        let _guard = test_support::lock();

        invalidate_llm_client_cache();
        test_support::reset_build_count();

        let config = Config::default();
        assert!(
            create_llm_client_for_model("ollama/llama3.2", &config, "http://localhost:11434")
                .is_some()
        );
        assert_eq!(test_support::build_count(), 1);

        assert!(
            create_llm_client_for_model("ollama/llama3.2", &config, "http://localhost:11435")
                .is_some()
        );
        assert_eq!(test_support::build_count(), 2);
    }

    #[test]
    fn create_llm_client_returns_none_without_credentials() {
        let _guard = test_support::lock();

        invalidate_llm_client_cache();
        // Ensure env has no fallback key for this provider either.
        // SAFETY: tests are single-threaded within this module and only touch
        // a key we own for the duration of the assertion.
        let prev = std::env::var("ANTHROPIC_API_KEY").ok();
        unsafe { std::env::remove_var("ANTHROPIC_API_KEY") };

        let mut config = Config::default();
        config.llm.api_key = None;
        config.llm.anthropic_use_oauth = false;
        config.llm.anthropic_oauth_token = None;

        let result = create_llm_client_for_model("claude-opus-4", &config, "http://localhost:11434");
        assert!(result.is_none());

        if let Some(v) = prev {
            unsafe { std::env::set_var("ANTHROPIC_API_KEY", v) };
        }
    }
}
