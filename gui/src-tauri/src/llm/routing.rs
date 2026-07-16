//! Model id parsing and LLM client routing.

#[allow(clippy::wildcard_imports)]
use crate::*;

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

/// Create an LLM client for a specific model
pub(crate) fn create_llm_client_for_model(model_id: &str, config: &Config, ollama_host: &str) -> Option<(LlmClient, String)> {
    let (provider, model_name) = parse_model_id(model_id);

    match provider.as_str() {
        "anthropic" => {
            // Check if OAuth is enabled and has a token
            if config.llm.anthropic_use_oauth {
                if let Some(ref oauth_token) = config.llm.anthropic_oauth_token {
                    return Some((LlmClient::anthropic_oauth(oauth_token), model_name));
                }
            }
            // Fall back to API key
            let api_key = config.llm.api_key.clone()
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())?;
            Some((LlmClient::anthropic(&api_key), model_name))
        }
        "openai" => {
            let api_key = config.llm.openai_api_key.clone()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())?;
            Some((LlmClient::openai(&api_key), model_name))
        }
        "openrouter" => {
            let api_key = config.llm.openrouter_api_key.clone()
                .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())?;
            Some((LlmClient::openrouter(&api_key), model_name))
        }
        "github" => {
            let api_key = config.llm.github_token.clone()
                .or_else(|| std::env::var("GITHUB_TOKEN").ok())?;
            Some((LlmClient::github_models(&api_key), model_name))
        }
        "claude-proxy" => {
            let proxy_url = std::env::var("CLAUDE_PROXY_URL")
                .unwrap_or_else(|_| "http://localhost:3456".to_string());
            Some((LlmClient::claude_proxy(&proxy_url), model_name))
        }
        "ollama" => {
            Some((LlmClient::ollama(ollama_host), model_name))
        }
        _ => None,
    }
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
