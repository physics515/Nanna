# 04 — Multi-Provider LLM Support

## Feature Description

Nanna supports six LLM providers with automatic failover, rate limit management, and credential handling. Users configure a priority list of models; the system tries each in order, falling back on errors or rate limits. Each provider has its own authentication mechanism and model discovery API.

### Supported Providers
| Provider | Auth | Models | Context |
|----------|------|--------|---------|
| **Anthropic** | API key or OAuth (Claude Code CLI) | Claude 4 Opus/Sonnet, 3.5 Sonnet/Haiku | 200k |
| **OpenAI** | API key | GPT-4o, GPT-4-turbo, o1, o3 | 128k-200k |
| **OpenRouter** | API key | All models (DeepSeek, Gemini, Llama, Mistral, Qwen) | Varies |
| **GitHub Models** | GitHub token | Claude models via GitHub API | Varies |
| **Claude Proxy** | Local proxy (uses Claude Code creds) | Claude models | 200k |
| **Ollama** | None (local) | Any pulled model | 32k default |

### Model ID Format
`provider/model-name` — e.g., `anthropic/claude-sonnet-4-20250514`, `ollama/llama3.2`

## Current Implementation

### Model ID Parsing (lib.rs ~1023)
`parse_model_id(model_id)`: Splits on first `/`. Returns `(provider, model_name)`. Falls back to `("anthropic", model_id)` if no slash — assumes Anthropic for bare model names.

### Client Creation (lib.rs ~1048)
`create_llm_client_for_model(model_id, config, ollama_host)`:
- Matches on provider string to create appropriate `LlmClient`
- Anthropic: checks OAuth token first, falls back to API key, then env var
- OpenAI/OpenRouter/GitHub: reads API key from config or env var
- Claude Proxy: constructs URL from config
- Ollama: uses configured host URL
- Returns `Option<(LlmClient, String)>` — None if no credentials available

### Model Selection (lib.rs ~1092)
`select_best_model(priority, rate_limited, config, ollama_host)`:
- Iterates priority list
- Skips rate-limited models (checks cooldown timestamp)
- Attempts to create client for each
- Returns first successful `(LlmClient, model_id)`

### Rate Limit Detection (lib.rs ~1129)
`is_rate_limit_error(error_msg)`: String matching on error messages:
- "429", "rate limit", "too many requests"
- "529", "overloaded"
- "RECOVERABLE:" prefix (from nanna-llm)
- Case-insensitive matching

`parse_retry_after_from_error(error_msg)`: Regex-like parsing for retry-after seconds from error strings.

### Model Discovery APIs

**`get_anthropic_models`** (lib.rs ~3781):
- Fetches from `https://api.anthropic.com/v1/models`
- Uses API key or OAuth token
- Filters and formats model names
- OAuth users see subscription-tier models

**`get_openai_models`** (lib.rs ~3848):
- Fetches from `https://api.openai.com/v1/models`
- Filters to known model prefixes (gpt-4, o1, o3)
- Sorts alphabetically

**`get_openrouter_models`** (lib.rs ~3906):
- Fetches from `https://openrouter.ai/api/v1/models`
- Returns all models with pricing info
- Large response — can be hundreds of models

**`get_github_models`** (lib.rs ~3983):
- Fetches from GitHub Models API
- Filters to Claude models
- Uses GitHub personal access token

**`get_claude_proxy_models`** (lib.rs ~4052):
- Queries local proxy for available models
- Falls back to hardcoded Claude model list if proxy doesn't support model listing

**`get_ollama_models`** (lib.rs ~3689):
- Fetches from `{ollama_host}/api/tags`
- Returns locally pulled models with sizes

### OAuth / Credential Management (lib.rs ~3279-3547)
- `run_claude_setup_token`: Executes `claude setup-token` CLI command for PKCE OAuth flow
- `import_claude_code_credentials`: Reads existing Claude Code CLI credentials
- `save_anthropic_oauth_token`: Stores OAuth token in config
- `refresh_oauth_token`: Refreshes expired OAuth tokens
- `get_credential_status`: Reports which credential source is active (OAuth, API key, env var)
- `logout_anthropic_oauth`: Clears stored OAuth credentials

### Model Priority System
- `get/set_chat_model_priority`: Ordered list for chat completion
- `get/set_embedding_model_priority`: Ordered list for embeddings
- `get/set_summarization_model_priority`: Ordered list for tool result summarization
- Each is a `Vec<String>` of model IDs

### Rate Limit Tracking
- `rate_limited_models: HashMap<String, chrono::DateTime<Utc>>` in AppState
- Models added on rate limit detection with cooldown expiry
- `clear_rate_limit`: Manual override to clear a model's rate limit
- `get_model_status`: Returns current model and rate limit state

## Issues & Bugs

### Critical
1. **Rate limit detection is fragile string matching**: `is_rate_limit_error` does substring matching on error messages. If nanna-llm changes its error format, or a provider returns a different message, rate limits won't be detected. Should use structured error types from nanna-llm.

2. **OAuth token refresh has no retry**: `refresh_oauth_token` attempts one refresh. If it fails (network issue, expired refresh token), the user loses API access until they manually re-authenticate. No automatic retry or graceful degradation to API key.

3. **`create_llm_client_for_model` creates a new client every call**: No caching. Each model switch in the fallback chain creates a fresh HTTP client. For rapid fallback scenarios, this adds latency.

### Moderate
4. **Bare model names default to Anthropic**: `parse_model_id("gpt-4o")` returns `("anthropic", "gpt-4o")`. This will fail silently — the Anthropic client will try to use "gpt-4o" and get an error. Should either require explicit provider prefix or use smarter inference.

5. **Model discovery results aren't cached**: Each call to `get_anthropic_models` etc. hits the API. If the user opens the model selector frequently, this generates unnecessary API traffic. Should cache with TTL.

6. **Claude Proxy health check is simplistic**: `check_claude_proxy_health` just checks if the URL responds. Doesn't verify the proxy can actually make API calls.

7. **No validation of model priority lists**: Users can add invalid model IDs to priority lists. These will fail at runtime but waste a fallback attempt.

8. **Cooldown duration is fixed**: Rate limit cooldowns use a fixed duration (60s minimum). Some providers return `retry-after` headers with specific durations, but the parsing is best-effort regex.

### Minor
9. **`format_claude_model_name` is hardcoded**: Maps model IDs to human-readable names with a static match. New models require code changes.

10. **No model capability detection**: The system doesn't know which models support tool use, streaming, or vision. It tries and fails.

11. **OpenRouter model list is unfiltered**: Returns hundreds of models. No way to filter by capability, pricing, or quality.

## Improvement Suggestions

### High Priority
- **Structured error types**: Have nanna-llm return typed errors (`RateLimitError { retry_after: Option<Duration> }`, `AuthError`, `ModelNotFoundError`) instead of strings. Match on these in the fallback logic.
- **Cache LlmClient instances**: Store created clients in a `HashMap<String, LlmClient>` keyed by model ID. Reuse across calls. Invalidate on credential changes.
- **Smart model ID inference**: If no provider prefix, check known model name patterns (gpt-* → openai, claude-* → anthropic, llama* → ollama).

### Medium Priority
- **Cache model discovery results**: Store with 5-minute TTL. Refresh in background on expiry.
- **OAuth retry with backoff**: On refresh failure, retry 3 times with exponential backoff. If all fail, emit a `credential-expired` event to the frontend.
- **Validate priority lists on save**: Check that each model ID has a valid provider prefix and that credentials exist for that provider.
- **Respect retry-after headers**: Parse the actual retry-after value and use it as the cooldown duration instead of the fixed minimum.

### Future
- **Model capability matrix**: Maintain a database of model capabilities (tool use, streaming, vision, max tokens). Use this to skip incompatible models in the fallback chain.
- **Cost tracking**: Track API costs per model. Show cumulative spend in the UI. Allow budget limits.
- **Automatic model recommendation**: Based on task complexity (estimated tokens, tool use needed), recommend the optimal model from the priority list.
- **Provider health dashboard**: Show latency, error rates, and availability for each configured provider.
- **Custom provider support**: Allow users to add arbitrary OpenAI-compatible API endpoints as custom providers.
