#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! LLM API client for Nanna
//!
//! Supports Anthropic Claude with proper tool calling and streaming.

use tracing::{debug, info, warn};

#[cfg(feature = "auto-refresh")]
pub mod oauth;

#[cfg(feature = "auto-refresh")]
pub use oauth::{OAuthClient, create_oauth_client, create_oauth_client_sync};

pub mod heal;
pub use heal::{count_balanced_top_level_objects, heal_json, heal_json_as, heal_tool_args};

use async_stream::stream;
use futures::{Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Watches a token stream for a wedged runner emitting one token forever.
///
/// Observed live 2026-07-27: thinking blocks of `"00000000000000…"` filling
/// the GUI, and stream aborts landing on an identical 4306-byte / 31-line
/// shape 46 times in one night — the same fault seen from two directions.
/// Each occurrence cost a step's worth of GPU time and produced nothing.
///
/// Bounds, both derived from what real generation looks like rather than
/// picked for feel:
/// - Only fragments of **≤ 4 characters** are candidates. A sampler that
///   wedges repeats a single token; a legitimately repeated long fragment is
///   content, not a fault.
/// - **40** consecutive identical fragments trips it. Real text repeats a
///   short token a handful of times (indentation runs, `...`, a rule of
///   dashes); it does not do so forty times without variation, while a
///   wedged runner reaches forty almost immediately.
#[derive(Debug, Default)]
struct RepeatWatch {
    last: Option<String>,
    run: usize,
}

impl RepeatWatch {
    /// Trip point, measured against the failure it exists to catch rather
    /// than chosen for feel: the observed degenerate streams die at **31**
    /// NDJSON lines (~30 emitted fragments), so a bound of 40 could never
    /// fire — it was set by reasoning about legitimate text alone, and the
    /// evidence for the real number was already in the logs. 20 sits below
    /// the failure and far above ordinary repetition, which cannot reach it
    /// because any non-candidate fragment resets the run.
    const MAX_RUN: usize = 20;
    const MAX_FRAGMENT_CHARS: usize = 4;

    /// Record a fragment; returns the run length once it is degenerate.
    fn observe(&mut self, fragment: &str) -> Option<usize> {
        if fragment.trim().is_empty() || fragment.chars().count() > Self::MAX_FRAGMENT_CHARS {
            // Not a candidate: any real content resets the watch, so a burst
            // of indentation followed by prose never accumulates.
            self.last = None;
            self.run = 0;
            return None;
        }
        if self.last.as_deref() == Some(fragment) {
            self.run += 1;
        } else {
            self.last = Some(fragment.to_string());
            self.run = 1;
        }
        (self.run >= Self::MAX_RUN).then_some(self.run)
    }
}

/// The single generation slot a local Ollama server hands out, as a permit.
///
/// llama.cpp serves one generation at a time per slot. A second concurrent
/// request does **not** queue politely behind the first: the server re-selects
/// the slot, swaps prompt caches (`found better prompt` / `updating prompt
/// cache`) and *cancels* the running task (`srv stop: cancel task`). The
/// victim's HTTP stream then just ends — no `done: true` — which this client
/// can only report as a provider incident, so the harness burns retry budget
/// healing damage the daemon did to itself. Observed live 2026-07-26: a
/// scheduled heartbeat killed a chat turn's generation three times running,
/// each time ~4.3 KB / 31 NDJSON lines in.
///
/// One permit per base URL, so two genuinely separate servers keep separate
/// slots and a single server is never asked to interleave. Waiting is the
/// correct behaviour here — the alternative is not "more throughput" but two
/// mutually-cancelling generations.
///
/// Scope: chat/completion generations only. Embeddings hit a different
/// endpoint and are left alone.
fn ollama_generation_slot(base_url: &str) -> std::sync::Arc<tokio::sync::Semaphore> {
    static SLOTS: std::sync::OnceLock<
        std::sync::Mutex<
            std::collections::HashMap<String, std::sync::Arc<tokio::sync::Semaphore>>,
        >,
    > = std::sync::OnceLock::new();
    let slots = SLOTS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    // A poisoned lock carries no invalid state here (the map is only ever
    // inserted into), so recover rather than propagate a panic into the
    // request path.
    let mut guard = slots.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    guard
        .entry(base_url.to_string())
        .or_insert_with(|| std::sync::Arc::new(tokio::sync::Semaphore::new(1)))
        .clone()
}

#[derive(Error, Debug, Clone)]
pub enum LlmError {
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("API error: {status} - {message}")]
    Api { status: u16, message: String },
    #[error("JSON error: {0}")]
    Json(String),
    #[error("Stream error: {0}")]
    Stream(String),
    #[error("Missing API key for provider: {0}")]
    MissingApiKey(String),
    #[error("Rate limit exceeded: {message}")]
    RateLimit {
        message: String,
        /// Seconds until rate limit resets (if available)
        retry_after: Option<u64>,
    },
    /// A local runner stuck emitting one token forever, caught by
    /// [`RepeatWatch`].
    ///
    /// Structured rather than folded into [`LlmError::Api`] because the retry
    /// ladder has to tell "this is the same wedge I just hit" from "one
    /// unlucky generation", and that decision reads the token and the abort
    /// length. Those were only ever present as prose inside the message, so
    /// the caller's choice depended on re-parsing an error string — a
    /// contract nothing enforced and any rewording would have broken.
    ///
    /// The rendered form is byte-identical to the 502 this used to be
    /// (`from_api_response(502, …)`). That is load-bearing, not cosmetic:
    /// `is_transient_llm_error` keys off `"API error: 5"` and
    /// `wedged_runner_error` off `"same token"`, on both the harness and the
    /// chat path. A variant that read differently would drop the wedge out of
    /// the transient class and hard-fail the step instead of retrying it.
    #[error(
        "API error: 502 - Ollama emitted the same token {run}x in a row ({token:?}) — \
         a wedged runner, not a generation. Aborted after {bytes_received} bytes so \
         the step can be retried instead of waiting out the loop."
    )]
    WedgedRunner {
        /// The fragment the decoder was stuck on.
        token: String,
        /// Consecutive repeats observed when the watch tripped.
        run: usize,
        /// Stream bytes received before the abort.
        bytes_received: usize,
    },
    #[error("All fallback models exhausted")]
    AllModelsExhausted,
    #[error("IO error: {0}")]
    Io(String),
}

impl From<std::io::Error> for LlmError {
    fn from(e: std::io::Error) -> Self {
        LlmError::Io(e.to_string())
    }
}

// ============================================================================
// Model Information & Capabilities
// ============================================================================

/// Model capabilities and limits
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Model identifier
    pub id: String,
    /// Maximum input context window in tokens
    pub context_window: usize,
    /// Maximum output tokens
    pub max_output_tokens: usize,
    /// Whether the model supports tool/function calling
    #[serde(default)]
    pub supports_tools: bool,
    /// Whether the model supports vision/images
    #[serde(default)]
    pub supports_vision: bool,
    /// Embedding dimension (if this is an embedding model)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_dimension: Option<usize>,
    /// Unix timestamp when this info was cached
    #[serde(default)]
    pub cached_at: i64,
    /// Provider this info came from
    #[serde(default)]
    pub provider: String,
}

impl ModelInfo {
    /// Cache TTL: 1 week in seconds
    pub const CACHE_TTL_SECS: i64 = 7 * 24 * 60 * 60;

    /// Check if this cached info has expired
    #[must_use]
    pub fn is_expired(&self) -> bool {
        let now = current_timestamp();
        now - self.cached_at > Self::CACHE_TTL_SECS
    }

    /// Recommended compression threshold — where the context manager should
    /// *proactively* compress, always **below** [`Self::hard_input_limit`].
    ///
    /// Nominally 80% of the context window, but a small model with a large
    /// `max_output_tokens` (e.g. context 8k / output 4k) makes 80%-of-context
    /// *exceed* the hard input limit, so proactive compression would never fire
    /// before the hard cap and the agent would emergency-truncate every turn.
    /// Capping at 90% of the hard limit restores the invariant while leaving
    /// large models (where 80%-of-context is already the smaller value) unchanged.
    #[must_use]
    pub fn compression_threshold(&self) -> usize {
        let by_context = (self.context_window * 80) / 100;
        let below_hard = (self.hard_input_limit() * 90) / 100;
        let threshold = by_context.min(below_hard);
        debug_assert!(
            threshold <= self.hard_input_limit(),
            "compression must trigger before the hard input cap"
        );
        threshold
    }

    /// Compression threshold paired with a concrete output reservation —
    /// same 80%-of-context / 90%-of-hard-limit rule as
    /// [`Self::compression_threshold`], but tracking
    /// [`Self::hard_input_limit_for`] so proactive summarization still fires
    /// before the (now larger) dynamic hard cap.
    #[must_use]
    pub fn compression_threshold_for(&self, output_reserve_tokens: usize) -> usize {
        let by_context = (self.context_window * 80) / 100;
        let below_hard = (self.hard_input_limit_for(output_reserve_tokens) * 90) / 100;
        let threshold = by_context.min(below_hard);
        debug_assert!(
            threshold <= self.hard_input_limit_for(output_reserve_tokens),
            "compression must trigger before the hard input cap"
        );
        threshold
    }

    /// Hard limit for input (leaves room for output), static variant.
    ///
    /// Reserves `min(max_output_tokens, context/4)` — several providers
    /// (Ollama among them) report `max_output_tokens >= context_window`, and
    /// subtracting that claim verbatim halved a 32k local model to 16k of
    /// usable input (observed live: the user's own prompt was
    /// emergency-truncated). Callers that know their per-request `max_tokens`
    /// should prefer [`Self::hard_input_limit_for`] with
    /// [`Self::effective_output_budget`] — the reserve then shrinks to what
    /// the request can actually generate, freeing the rest for input.
    #[must_use]
    pub fn hard_input_limit(&self) -> usize {
        self.hard_input_limit_for(self.max_output_tokens.min(self.context_window / 4))
    }

    /// Hard input limit paired with a concrete output reservation.
    ///
    /// The reserve is capped at half the window (output must not starve
    /// input) and the result is floored at half the window (a degenerate
    /// reserve must not zero the input side) — so this never returns 0 and
    /// `input + reserve` never exceeds `context_window`.
    #[must_use]
    pub fn hard_input_limit_for(&self, output_reserve_tokens: usize) -> usize {
        // Windows below 2 tokens are out of contract (no provider reports
        // one; the universal floor is 32k) — the pairing invariant needs
        // ctx/2 to be nonzero.
        debug_assert!(self.context_window >= 2, "context window must be >= 2");
        let reserve = output_reserve_tokens.min(self.context_window / 2);
        let floor = self.context_window / 2;
        self.context_window.saturating_sub(reserve).max(floor)
    }

    /// The output-token budget a request should actually use: the caller's
    /// `max_tokens`, bounded by the model's reported maximum and by half the
    /// window. Pairing a request's `max_tokens` with
    /// [`Self::hard_input_limit_for`] of this same value makes the split
    /// dynamic — a 2k-output agent keeps ~94% of a 32k window for input —
    /// while input + output provably never over-commit the window.
    #[must_use]
    pub fn effective_output_budget(&self, requested_max_tokens: usize) -> usize {
        requested_max_tokens
            .min(self.max_output_tokens)
            .min(self.context_window / 2)
            .max(1)
    }

    /// Output-token budget paired with [`Self::hard_input_limit`]: the slice
    /// of the window the input side does NOT claim. A request whose
    /// `max_tokens` is clamped to this can never over-commit the window —
    /// input ≤ `hard_input_limit` and output ≤ this sum to ≤ `context_window`.
    #[must_use]
    pub fn output_token_budget(&self) -> usize {
        self.context_window.saturating_sub(self.hard_input_limit())
    }

    /// Create with current timestamp
    fn with_timestamp(mut self) -> Self {
        self.cached_at = current_timestamp();
        self
    }

    /// Tokens available for conversation history after hard-input + reserved heads.
    ///
    /// `system_reserved` / `response_reserved` cover prompt + completion heads that
    /// sit outside the stored history window (GUI system prompt + response reserve).
    /// Floored at `min_history` so truncation never empties the transcript, but
    /// never above the hard input limit itself.
    #[must_use]
    pub fn conversation_history_budget(
        &self,
        system_reserved: usize,
        response_reserved: usize,
        min_history: usize,
    ) -> usize {
        let hard = self.hard_input_limit();
        hard.saturating_sub(system_reserved.saturating_add(response_reserved))
            .max(min_history.min(hard))
    }
}

/// Get current Unix timestamp
fn current_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Universal context floor when no provider has reported limits for a model.
///
/// Providers update windows constantly; we never hardcode per-model values.
/// Callers should prefer [`LlmClient::get_model_info`] / [`ModelInfoCache`].
/// This floor is intentionally conservative so unknown / offline models stay
/// under their real window rather than overshoot and 400.
pub const UNKNOWN_CONTEXT_WINDOW: usize = 32_000;

/// Universal max-output floor when the provider does not report it.
pub const UNKNOWN_MAX_OUTPUT_TOKENS: usize = 4_096;

/// Best-effort context window (in tokens) for a model by name.
///
/// Order: live disk cache from a prior API fetch, else [`UNKNOWN_CONTEXT_WINDOW`].
/// No per-model name table — models change constantly; use the provider API.
#[must_use]
pub fn model_context_window(model: &str) -> usize {
    model_info_from_cache_or_unknown(model, "").context_window
}

/// Build a [`ModelInfo`] from the on-disk cache, or the universal unknown floor.
///
/// Used by sync paths that cannot await a provider fetch (e.g. consolidation
/// sizing). Prefer [`LlmClient::get_model_info`] whenever a client is available.
///
/// The result is clamped to the LIVE effective runner window
/// ([`clamp_model_info_to_effective_window`]): a cached provider claim is a
/// static fact, but a mid-run `num_ctx` demotion is live state, and budgets
/// derived from the stale claim overflow every subsequent prompt.
#[must_use]
pub fn model_info_from_cache_or_unknown(model: &str, provider: &str) -> ModelInfo {
    if let Some(cache) = ModelInfoCache::default_location() {
        if let Some(info) = cache.get(model) {
            return clamp_model_info_to_effective_window(model, info);
        }
    }
    clamp_model_info_to_effective_window(model, unknown_model_info(model, provider))
}

/// The context window the local runner will actually honour for `model` right
/// now: `reported` (provider metadata / cache / config) clamped by the live
/// Ollama `num_ctx` latch. Models that were never sized by the Ollama path
/// (cloud providers, unlatched local models) pass `reported` through unchanged.
///
/// This is the read side of [`LlmClient::demote_context`]. A mid-run VRAM
/// demotion rewrites the latch, and every downstream budget (compression
/// threshold, hard input limit, workspace cap) must be derived from THIS
/// number, not from the provider's static claim. Observed live 2026-08-02: a
/// silent 16384→4096 demotion left the agent budgeting for 16k while Ollama
/// truncated every prompt to 4k model-side — the model lost its task and a
/// 4-hour eval scored 1/42 against 4/42-per-hour at full context.
#[must_use]
pub fn effective_context_window(model: &str, reported: usize) -> usize {
    match LlmClient::effective_num_ctx(model) {
        Some(latched) => reported.min(latched as usize),
        None => reported,
    }
}

/// `info` with its window clamped to the live effective runner window for
/// `model` (see [`effective_context_window`]).
///
/// `max_output_tokens` is re-bounded to half the clamped window, mirroring how
/// the Ollama fetch derives it from the full window — so every budget the
/// [`ModelInfo`] helpers derive (`hard_input_limit_for`,
/// `effective_output_budget`, `compression_threshold_for`) stays consistent
/// with the window the runner will actually honour. No-op when no latch exists
/// or the latch is at/above the reported window.
#[must_use]
pub fn clamp_model_info_to_effective_window(model: &str, mut info: ModelInfo) -> ModelInfo {
    let effective = effective_context_window(model, info.context_window);
    if effective < info.context_window {
        tracing::debug!(
            model,
            reported_window = info.context_window,
            effective_window = effective,
            "model info clamped to live effective num_ctx (runner was demoted below the provider claim)"
        );
        info.context_window = effective;
        info.max_output_tokens = info.max_output_tokens.min((effective / 2).max(1));
    }
    info
}

/// The quantum every demoted `num_ctx` snaps to.
///
/// Derived from the runner, not chosen: llama.cpp (Ollama's backend) prefills
/// prompts in physical batches of 512 tokens (`n_batch`/`n_ubatch` default),
/// so a context is consumed and its KV cache allocated in 512-token slices —
/// a finer step changes the footprint by less than one prefill batch and buys
/// nothing, while a coarser one (the old power-of-two buckets) throws away
/// windows that would have fit.
pub const NUM_CTX_QUANTUM: u32 = 512;

/// The top of the demotion ladder — the largest window the VRAM sizing path
/// can grant (its buckets are `[4096, 8192, 16384, 32768]`), so an unlatched
/// model demoting for the first time walks down from the most it could ever
/// have been given, exactly as the old bucket ladder did.
pub const NUM_CTX_CEILING: u32 = 32_768;

/// Fallback minimum viable window for [`LlmClient::demote_context`] when the
/// caller cannot supply the live agent floor.
///
/// Derived from the pressure-tier `ContextFloor` the agent measures (system
/// prompt slice + core-plus-work-evidence tool definitions + step frame +
/// window-scaled output reserve): observed live 2026-08-03 at ~4.2k tokens,
/// rounded UP to the next [`NUM_CTX_QUANTUM`]. The old ladder's 4096 terminal
/// bucket sat BELOW that floor, and landing there produced 38 below-floor
/// stops and 8 burned resume cycles in 8 minutes — a default any lower than
/// the measured floor manufactures a window the agent will loudly refuse.
///
/// Callers with the run at hand should pass the measured floor instead
/// (`nanna_agent`'s min-viable-window derivation); this constant only covers
/// call sites that have no agent state to measure.
pub const DEFAULT_MIN_VIABLE_NUM_CTX: u32 = 4_608;

/// Conservative model info when the provider has not (yet) told us limits.
///
/// Supports tools optimistically — capability misses are softer than overrunning
/// a too-small context budget on a modern agent path.
#[must_use]
pub fn unknown_model_info(model: &str, provider: &str) -> ModelInfo {
    ModelInfo {
        id: model.to_string(),
        context_window: UNKNOWN_CONTEXT_WINDOW,
        max_output_tokens: UNKNOWN_MAX_OUTPUT_TOKENS,
        supports_tools: true,
        supports_vision: false,
        embedding_dimension: None,
        cached_at: current_timestamp(),
        provider: provider.to_string(),
    }
}

/// Alias kept for local fetch_* fallbacks — one floor, not a model table.
fn default_model_info(model: &str, provider: &str) -> ModelInfo {
    unknown_model_info(model, provider)
}

/// File-based cache for model information
#[derive(Clone)]
pub struct ModelInfoCache {
    cache_dir: std::path::PathBuf,
}

impl ModelInfoCache {
    /// Create a new cache with the specified directory
    pub fn new(cache_dir: impl Into<std::path::PathBuf>) -> Self {
        Self {
            cache_dir: cache_dir.into(),
        }
    }

    /// Create cache in the default location (user's cache directory)
    pub fn default_location() -> Option<Self> {
        directories::ProjectDirs::from("com", "nanna", "nanna")
            .map(|dirs| Self::new(dirs.cache_dir().join("model_info")))
    }

    /// Get cached model info if it exists and is not expired
    pub fn get(&self, model: &str) -> Option<ModelInfo> {
        let path = self.cache_path(model);
        if !path.exists() {
            return None;
        }

        let content = std::fs::read_to_string(&path).ok()?;
        let info: ModelInfo = serde_json::from_str(&content).ok()?;

        if info.is_expired() {
            // Remove expired cache
            let _ = std::fs::remove_file(&path);
            debug!(model = %model, "Model info cache expired, removed");
            return None;
        }

        debug!(model = %model, context_window = info.context_window, "Loaded model info from cache");
        Some(info)
    }

    /// Store model info in cache
    pub fn set(&self, info: &ModelInfo) -> Result<(), LlmError> {
        // Ensure cache directory exists
        std::fs::create_dir_all(&self.cache_dir)?;

        let path = self.cache_path(&info.id);
        let content = serde_json::to_string_pretty(info)?;
        std::fs::write(&path, content)?;

        debug!(model = %info.id, path = %path.display(), "Cached model info");
        Ok(())
    }

    /// Clear all cached model info
    pub fn clear(&self) -> Result<(), LlmError> {
        if self.cache_dir.exists() {
            std::fs::remove_dir_all(&self.cache_dir)?;
        }
        Ok(())
    }

    /// Get cache file path for a model
    fn cache_path(&self, model: &str) -> std::path::PathBuf {
        // Sanitize model name for use as filename
        let safe_name: String = model
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        self.cache_dir.join(format!("{safe_name}.json"))
    }
}

impl From<reqwest::Error> for LlmError {
    fn from(e: reqwest::Error) -> Self {
        LlmError::Http(e.to_string())
    }
}

impl From<serde_json::Error> for LlmError {
    fn from(e: serde_json::Error) -> Self {
        LlmError::Json(e.to_string())
    }
}

impl LlmError {
    /// Check if this error is a rate limit error (429)
    #[must_use]
    pub fn is_rate_limit(&self) -> bool {
        matches!(self, LlmError::RateLimit { .. })
            || matches!(self, LlmError::Api { status: 429, .. })
    }

    /// Check if this error should trigger a fallback to another model
    #[must_use]
    pub fn should_fallback(&self) -> bool {
        match self {
            // Rate limits - definitely fallback
            LlmError::RateLimit { .. } => true,
            LlmError::Api { status, message } => {
                // 429 = rate limit
                // 529 = overloaded (Anthropic)
                // 503 = service unavailable
                // 502 = bad gateway
                if *status == 429 || *status == 529 || *status == 503 || *status == 502 {
                    return true;
                }
                // Check for rate limit in error message (some APIs return 400 with rate limit message)
                let msg_lower = message.to_lowercase();
                msg_lower.contains("rate limit") || msg_lower.contains("rate_limit")
            }
            // A wedged runner used to arrive as Api{status:502} and so fell
            // into the 502 arm above. It is still exactly that fault — keep it
            // falling back, or splitting the variant out would silently stop
            // the router trying another model.
            LlmError::WedgedRunner { .. } => true,
            // Network errors - might be transient
            LlmError::Http(_) => true,
            // Don't fallback on auth errors, JSON errors, etc.
            _ => false,
        }
    }

    /// Parse an API error response to extract rate limit info
    pub fn from_api_response(status: u16, message: String) -> Self {
        // Both spellings are real: OpenAI-style bodies say "rate_limit_exceeded",
        // OpenRouter's human-form message says "Rate limit exceeded". Matching
        // only the underscore form silently declassified OpenRouter's — and a
        // rate limit that isn't classified as one is retried at full speed,
        // which is the one response a rate limit must never get.
        let lowered = message.to_lowercase();
        if status == 429 || lowered.contains("rate_limit") || lowered.contains("rate limit") {
            // Try to extract retry-after from the message
            let retry_after = Self::parse_retry_after(&message);
            return LlmError::RateLimit { message, retry_after };
        }

        LlmError::Api { status, message }
    }

    /// Try to parse retry-after seconds from an error message.
    ///
    /// Providers say when they will be ready and we were throwing it away —
    /// OpenRouter's 429 body carries `"X-RateLimit-Reset":"1785222060000"`, a
    /// millisecond epoch, and every caller was backing off on a guessed
    /// schedule instead. Reading it turns "wait a made-up interval and hope"
    /// into "wait exactly as long as they asked".
    ///
    /// Returns `None` when nothing usable is present, which keeps the caller's
    /// own backoff as the fallback.
    fn parse_retry_after(message: &str) -> Option<u64> {
        /// A reset further out than this is a misread, not a real wait.
        const MAX_PLAUSIBLE_WAIT_SECS: u64 = 3_600;

        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();

        // `X-RateLimit-Reset` — epoch, in seconds or milliseconds depending on
        // the provider. Disambiguated by magnitude rather than by trusting
        // either convention.
        let from_reset = message
            .split("X-RateLimit-Reset")
            .nth(1)
            .and_then(|rest| {
                let digits: String = rest
                    .chars()
                    .skip_while(|c| !c.is_ascii_digit())
                    .take_while(char::is_ascii_digit)
                    .collect();
                digits.parse::<u64>().ok()
            })
            .map(|reset| if reset > 100_000_000_000 { reset / 1000 } else { reset })
            .and_then(|reset_secs| reset_secs.checked_sub(now_secs));

        // `retry-after: N` / `"retry_after": N` — plain seconds.
        let from_retry_after = ["retry-after", "retry_after"].iter().find_map(|key| {
            let rest = message.to_lowercase();
            let idx = rest.find(key)?;
            let digits: String = rest[idx + key.len()..]
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(char::is_ascii_digit)
                .collect();
            digits.parse::<u64>().ok()
        });

        from_reset
            .or(from_retry_after)
            .filter(|secs| *secs > 0 && *secs <= MAX_PLAUSIBLE_WAIT_SECS)
    }
}

// =============================================================================
// Provider request pacing
// =============================================================================
//
// Enforced at the PROVIDER level so every caller that routes through a
// rate-limited provider — chat, dream summarization, embeddings, anything —
// draws from ONE clock per provider instead of each pacing itself and
// collectively burning the shared quota. (The gate first lived in the dream
// summarizer alone, which protected dreams and nobody else; the observed
// failure was a dream cycle firing one request per cluster in the same second
// and losing 20 of 22 clusters to 429s.)

/// Per-provider pacing state: when we last asked, and what the provider last
/// TOLD US about our budget.
///
/// The provider's own numbers beat any constant: `X-RateLimit-Remaining` /
/// `X-RateLimit-Reset` reflect the actual account's tier and the actual
/// window, which published docs cannot. The static
/// [`Provider::min_request_spacing`] is only the bootstrap prior, used until
/// the first response teaches us the truth.
#[derive(Default)]
struct PaceState {
    /// When the previous request to this provider was issued.
    last_request: Option<tokio::time::Instant>,
    /// Requests the provider said we had left, decremented optimistically as
    /// we send (the header confirming it arrives one response later).
    remaining: Option<u64>,
    /// When the provider said the window refills.
    reset_at: Option<tokio::time::Instant>,
}

impl PaceState {
    /// How long the next request must wait, given `now`.
    ///
    /// Pure — all clock reads happen at the caller — so the policy is testable
    /// without real time:
    /// - Provider said the window has refilled (`reset_at` in the past): no
    ///   wait; the budget is fresh even if we never saw the new headers.
    /// - Provider said how much budget remains: spread it evenly across the
    ///   time left in the window (`(reset_at - now) / remaining`). A generous
    ///   budget yields spacing BELOW the static prior — dynamic runs faster,
    ///   not just safer. `remaining == 0` waits out the window.
    /// - Nothing learned yet: the static published-limit prior.
    fn required_wait(
        &self,
        now: tokio::time::Instant,
        fallback_spacing: tokio::time::Duration,
    ) -> tokio::time::Duration {
        let since_last = self
            .last_request
            .map(|prev| now.saturating_duration_since(prev));

        let spacing = match (self.reset_at, self.remaining) {
            (Some(reset_at), _) if reset_at <= now => return tokio::time::Duration::ZERO,
            (Some(reset_at), Some(0)) => return reset_at.saturating_duration_since(now),
            (Some(reset_at), Some(remaining)) => {
                let window_left = reset_at.saturating_duration_since(now);
                // u32 clamp is safe: a `remaining` beyond u32::MAX means the
                // spacing is zero at any window length we could measure.
                window_left / u32::try_from(remaining).unwrap_or(u32::MAX)
            }
            _ => fallback_spacing,
        };

        match since_last {
            None => tokio::time::Duration::ZERO,
            Some(elapsed) => spacing.saturating_sub(elapsed),
        }
    }

    /// Fold a provider's rate-limit headers into the state.
    ///
    /// `reset_in` is relative (converted from the header's epoch by the
    /// caller, which owns the wall clock) so this stays pure and testable.
    fn learn(&mut self, remaining: Option<u64>, reset_in: Option<tokio::time::Duration>) {
        if let Some(remaining) = remaining {
            self.remaining = Some(remaining);
        }
        if let Some(reset_in) = reset_in {
            self.reset_at = Some(tokio::time::Instant::now() + reset_in);
        }
    }
}

/// One pacing gate per paced provider. Separate mutexes, not one map behind
/// one lock: the lock is held across the pacing sleep deliberately (concurrent
/// same-provider callers must serialize, or they all measure from the same
/// "last" stamp and fire together), and holding a shared lock across that
/// sleep would stall OTHER providers for no reason.
fn pace_gate(provider: Provider) -> Option<&'static tokio::sync::Mutex<PaceState>> {
    use std::sync::OnceLock;
    use tokio::sync::Mutex;
    static OPENROUTER: OnceLock<Mutex<PaceState>> = OnceLock::new();
    static GITHUB: OnceLock<Mutex<PaceState>> = OnceLock::new();
    match provider {
        Provider::OpenRouter => Some(OPENROUTER.get_or_init(|| Mutex::new(PaceState::default()))),
        Provider::GitHubModels => Some(GITHUB.get_or_init(|| Mutex::new(PaceState::default()))),
        _ => None,
    }
}

/// Hold a request to `provider` until its pacing allows it, process-wide
/// across all clients and callers.
///
/// Public so callers that reach a paced provider WITHOUT an [`LlmClient`]
/// (a future embedding path, for instance) can share the same clock. No-op
/// for providers without a published quota.
pub async fn pace_provider_requests(provider: Provider) {
    let Some(fallback) = provider.min_request_spacing() else {
        return;
    };
    let Some(gate) = pace_gate(provider) else {
        return;
    };
    let mut state = gate.lock().await;
    let wait = state.required_wait(tokio::time::Instant::now(), fallback);
    if wait > tokio::time::Duration::ZERO {
        tokio::time::sleep(wait).await;
    }
    state.last_request = Some(tokio::time::Instant::now());
    // The request we are about to send spends one unit of whatever budget the
    // provider last reported; the confirming header arrives one response late.
    state.remaining = state.remaining.map(|r| r.saturating_sub(1));
}

/// Teach the pacing gate what a response's rate-limit headers said.
///
/// Reads `x-ratelimit-remaining` and `x-ratelimit-reset` (OpenRouter sends the
/// reset as an epoch in milliseconds, GitHub in seconds — disambiguated by
/// magnitude, same rule as [`LlmError::parse_retry_after`]). Uses `try_lock`
/// rather than awaiting: losing one update to contention is fine — the next
/// response repeats the lesson — while awaiting here would stall response
/// handling behind another caller's pacing sleep.
fn learn_rate_limit_headers(provider: Provider, headers: &reqwest::header::HeaderMap) {
    let Some(gate) = pace_gate(provider) else {
        return;
    };
    let header_u64 = |name: &str| -> Option<u64> {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            // Some providers send floats ("59.0"); take the integer part.
            .and_then(|s| s.split('.').next()?.trim().parse::<u64>().ok())
    };
    let remaining = header_u64("x-ratelimit-remaining");
    let reset_in = header_u64("x-ratelimit-reset").and_then(|reset| {
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        let reset_secs = if reset > 100_000_000_000 { reset / 1000 } else { reset };
        // A reset in the past yields None here; `required_wait` treats a
        // stale window as refilled anyway once reset_at lapses.
        reset_secs
            .checked_sub(now_secs)
            .map(tokio::time::Duration::from_secs)
    });
    if remaining.is_none() && reset_in.is_none() {
        return;
    }
    if let Ok(mut state) = gate.try_lock() {
        state.learn(remaining, reset_in);
    }
}

/// LLM Provider
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Anthropic,
    OpenAI,
    OpenRouter,
    GitHubModels,
    /// Claude Max/Pro proxy (OpenAI-compatible, uses claude-max-api-proxy)
    ClaudeProxy,
    Ollama,
}

impl Provider {
    /// Bootstrap spacing between consecutive requests to this provider,
    /// derived from the provider's PUBLISHED request-per-minute quota — never
    /// a tuning knob. This is only the PRIOR: the moment a response arrives,
    /// its `x-ratelimit-*` headers supersede it (see
    /// [`PaceState::required_wait`]) because headers reflect the actual
    /// account's tier, which docs cannot. `None` means "do not pace", and
    /// every `None` is a decision:
    ///
    /// - `OpenRouter`: free tier publishes 20 requests/minute → 60s/20 = 3s.
    ///   The free `:free` models are what this codebase configures.
    /// - `GitHubModels`: free tier publishes 15 requests/minute → 60s/15 = 4s.
    /// - `Anthropic` / `OpenAI`: limits are ACCOUNT-TIER dependent (Anthropic
    ///   tier 1 is 50 RPM, tier 4 is 4000; OpenAI similar). Pacing everyone to
    ///   the lowest tier would throttle paid accounts in the chat hot path for
    ///   no reason; both providers answer bursts with 429 + retry-after, which
    ///   the reactive layer honors. Proactive pacing here would need the
    ///   account's actual tier, which nothing exposes.
    /// - `ClaudeProxy`: fronts an Anthropic subscription; same reasoning.
    /// - `Ollama`: local — the only quota is the GPU itself.
    #[must_use]
    pub const fn min_request_spacing(self) -> Option<tokio::time::Duration> {
        match self {
            Provider::OpenRouter => Some(tokio::time::Duration::from_secs(3)),
            Provider::GitHubModels => Some(tokio::time::Duration::from_secs(4)),
            Provider::Anthropic | Provider::OpenAI | Provider::ClaudeProxy | Provider::Ollama => {
                None
            }
        }
    }
}

/// Message role
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// Chat message (simple text format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: content.into() }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into() }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into() }
    }
}

impl AnthropicMessage {
    /// Create a tool result message
    pub fn tool_result(tool_use_id: impl Into<String>, content: impl Into<String>, is_error: bool) -> Self {
        Self {
            role: "user".to_string(),
            content: vec![ContentBlock::ToolResult {
                tool_use_id: tool_use_id.into(),
                content: content.into(),
                is_error: if is_error { Some(true) } else { None },
            }],
        }
    }

    /// Create an assistant message with tool use
    pub fn tool_use(id: impl Into<String>, name: impl Into<String>, input: serde_json::Value) -> Self {
        Self {
            role: "assistant".to_string(),
            content: vec![ContentBlock::ToolUse {
                id: id.into(),
                name: name.into(),
                input,
            }],
        }
    }

    /// Create an assistant message with text and tool use
    pub fn assistant_with_tool_use(
        text: Option<String>,
        tool_id: impl Into<String>,
        tool_name: impl Into<String>,
        tool_input: serde_json::Value,
    ) -> Self {
        let mut content = Vec::new();
        if let Some(t) = text {
            if !t.is_empty() {
                content.push(ContentBlock::Text { text: t });
            }
        }
        content.push(ContentBlock::ToolUse {
            id: tool_id.into(),
            name: tool_name.into(),
            input: tool_input,
        });
        Self {
            role: "assistant".to_string(),
            content,
        }
    }
}

/// Anthropic message content block
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    Image { source: ImageSource },
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult { tool_use_id: String, content: String, #[serde(skip_serializing_if = "Option::is_none")] is_error: Option<bool> },
    /// Extended thinking content (reasoning)
    Thinking {
        thinking: String,
        /// Signature returned by Anthropic — MUST be sent back in multi-turn conversations.
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
}

/// Image source for vision API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageSource {
    Base64 {
        media_type: String,
        data: String,
    },
    Url {
        url: String,
    },
}

/// Anthropic message format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: Vec<ContentBlock>,
}

impl AnthropicMessage {
    #[must_use]
    pub fn user(content: Vec<ContentBlock>) -> Self {
        Self { role: "user".to_string(), content }
    }

    #[must_use]
    pub fn assistant(content: Vec<ContentBlock>) -> Self {
        Self { role: "assistant".to_string(), content }
    }

    pub fn user_text(text: impl Into<String>) -> Self {
        Self::user(vec![ContentBlock::Text { text: text.into() }])
    }

    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self::assistant(vec![ContentBlock::Text { text: text.into() }])
    }

    /// Create a user message with an image (base64)
    pub fn user_image(media_type: impl Into<String>, base64_data: impl Into<String>, text: Option<String>) -> Self {
        let mut content = vec![ContentBlock::Image {
            source: ImageSource::Base64 {
                media_type: media_type.into(),
                data: base64_data.into(),
            },
        }];
        if let Some(t) = text {
            content.push(ContentBlock::Text { text: t });
        }
        Self::user(content)
    }

    /// Create a user message with text and image
    pub fn user_with_image(text: impl Into<String>, media_type: impl Into<String>, base64_data: impl Into<String>) -> Self {
        Self::user(vec![
            ContentBlock::Text { text: text.into() },
            ContentBlock::Image {
                source: ImageSource::Base64 {
                    media_type: media_type.into(),
                    data: base64_data.into(),
                },
            },
        ])
    }
}

/// Tool definition for Anthropic
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Completion request with tools
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub anthropic_messages: Vec<AnthropicMessage>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stream: bool,
    pub tools: Vec<serde_json::Value>,
    /// Context window limit (for Ollama num_ctx). If set, limits the context
    /// window to reduce VRAM usage and allow more model layers on GPU.
    pub context_limit: Option<u32>,
    /// Extended thinking configuration (passed through to Anthropic API)
    pub thinking: Option<ThinkingConfig>,
}

impl Default for CompletionRequest {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-20250514".to_string(),
            messages: Vec::new(),
            anthropic_messages: Vec::new(),
            max_tokens: Some(4096),
            temperature: Some(0.7),
            stream: false,
            tools: Vec::new(),
            context_limit: None,
            thinking: None,
        }
    }
}

impl CompletionRequest {
    /// Set extended thinking configuration
    #[must_use]
    pub fn with_thinking(mut self, thinking: ThinkingConfig) -> Self {
        self.thinking = Some(thinking);
        self
    }

    /// Add tools to the request (Anthropic format)
    #[must_use]
    pub fn with_tools(mut self, tools: Vec<serde_json::Value>) -> Self {
        self.tools = tools;
        self
    }

    /// Add an Anthropic-format message (for tool use/results)
    #[must_use]
    pub fn with_anthropic_message(mut self, msg: AnthropicMessage) -> Self {
        self.anthropic_messages.push(msg);
        self
    }
}

/// How the model should surface its reasoning.
///
/// `Summarized` returns readable summary text in the thinking blocks;
/// `Omitted` still emits thinking blocks but leaves their text empty. The raw
/// chain of thought is never returned under either setting, and both are
/// billed the same — this controls visibility only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingDisplay {
    /// Readable summary of the reasoning.
    Summarized,
    /// Thinking blocks arrive with empty text.
    Omitted,
}

/// Extended thinking configuration.
///
/// Three wire shapes, and which ones a model accepts is part of that model's
/// request contract — see [`AnthropicModelContract`]. Sending the wrong shape
/// is a 400, not a degraded response:
///
/// - `{"type":"adaptive"}` — Claude 4.6 and newer. The model decides depth per
///   request; there is no token budget to set.
/// - `{"type":"enabled","budget_tokens":N}` — pre-4.6 models **only**.
///   `budget_tokens` was removed on Opus 5, Opus 4.8, Opus 4.7, Sonnet 5, and
///   Fable 5; sending it to any of them fails the request.
/// - `{"type":"disabled"}` — explicit opt-out, where the model accepts one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ThinkingConfig {
    /// Model-chosen thinking depth (Claude 4.6+).
    Adaptive {
        /// Omitted from the wire when `None`, which takes the model's default.
        #[serde(skip_serializing_if = "Option::is_none")]
        display: Option<ThinkingDisplay>,
    },
    /// Fixed thinking budget — pre-4.6 models only.
    Enabled {
        /// Budget in tokens. Must be strictly less than the request's
        /// `max_tokens`, minimum 1024.
        budget_tokens: u32,
    },
    /// Explicit opt-out.
    Disabled,
}

impl ThinkingConfig {
    /// A fixed-budget config, for pre-4.6 models.
    #[must_use]
    pub const fn enabled(budget_tokens: u32) -> Self {
        Self::Enabled { budget_tokens }
    }

    /// An adaptive config that asks for readable reasoning summaries.
    #[must_use]
    pub const fn adaptive_summarized() -> Self {
        Self::Adaptive {
            display: Some(ThinkingDisplay::Summarized),
        }
    }

    /// An adaptive config that takes the model's own `display` default.
    #[must_use]
    pub const fn adaptive() -> Self {
        Self::Adaptive { display: None }
    }

    /// The fixed budget, if this is a budgeted config.
    #[must_use]
    pub const fn budget_tokens(&self) -> Option<u32> {
        match self {
            Self::Enabled { budget_tokens } => Some(*budget_tokens),
            _ => None,
        }
    }

    /// Whether this config asks the model to reason.
    #[must_use]
    pub const fn is_thinking(&self) -> bool {
        matches!(self, Self::Adaptive { .. } | Self::Enabled { .. })
    }
}

/// The parts of a Claude model's request contract that differ by model family
/// and fail closed — send the wrong shape and the API returns 400 rather than
/// ignoring the field.
///
/// Derived from the published per-model table; the two axes that bite are the
/// `thinking` shape and whether sampling parameters still exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// Four independent capability facts about one model, not a state machine:
// every combination below is a real published contract, and they are read
// individually at different points in the request build.
#[allow(clippy::struct_excessive_bools)]
pub struct AnthropicModelContract {
    /// Takes `{"type":"adaptive"}`; `budget_tokens` is rejected.
    pub adaptive_thinking: bool,
    /// `thinking.display` defaults to `"omitted"`, so reasoning text only
    /// arrives if `"summarized"` is asked for explicitly. On the 4.6 family
    /// the default is already `"summarized"`.
    pub display_defaults_omitted: bool,
    /// `temperature`, `top_p`, and `top_k` were removed; sending one is a 400.
    pub sampling_removed: bool,
    /// Thinking cannot be turned off — `{"type":"disabled"}` is rejected, so
    /// the only legal request is one with no `thinking` field.
    pub thinking_always_on: bool,
}

impl AnthropicModelContract {
    /// The pre-4.6 contract: fixed thinking budgets, sampling parameters live.
    const LEGACY: Self = Self {
        adaptive_thinking: false,
        display_defaults_omitted: false,
        sampling_removed: false,
        thinking_always_on: false,
    };
}

/// Classify a Claude model into its request contract.
///
/// Matching is by family substring rather than an exact allow-list so dated
/// snapshots (`claude-sonnet-4-5-20250929`) and the Bedrock `anthropic.`
/// prefix land in the right family. The patterns cannot collide across
/// generations: `"sonnet-5"` does not occur inside `"sonnet-4-5"`, because the
/// character after `sonnet` differs.
///
/// Unknown `claude-*` models get the **current-generation** contract, not the
/// legacy one. Every model released since 4.6 has removed `budget_tokens` and
/// the sampling parameters, so an unrecognized name is far more likely to be
/// newer than the table than older than it — and guessing legacy for a new
/// model produces a hard 400 on the very first request.
#[must_use]
pub fn anthropic_model_contract(model: &str) -> AnthropicModelContract {
    let lowered = model.to_ascii_lowercase().replace('.', "-");
    let name = lowered
        .strip_prefix("anthropic.")
        .or_else(|| lowered.strip_prefix("anthropic/"))
        .unwrap_or(&lowered);

    // Thinking is always on and cannot be disabled.
    if name.contains("fable-5") || name.contains("mythos") {
        return AnthropicModelContract {
            adaptive_thinking: true,
            display_defaults_omitted: true,
            sampling_removed: true,
            thinking_always_on: true,
        };
    }

    // Adaptive-only, sampling removed, reasoning hidden unless asked for.
    if name.contains("opus-5")
        || name.contains("sonnet-5")
        || name.contains("opus-4-8")
        || name.contains("opus-4-7")
    {
        return AnthropicModelContract {
            adaptive_thinking: true,
            display_defaults_omitted: true,
            sampling_removed: true,
            thinking_always_on: false,
        };
    }

    // The 4.6 family: adaptive, but sampling still works and reasoning
    // summaries are already the default.
    if name.contains("opus-4-6") || name.contains("sonnet-4-6") {
        return AnthropicModelContract {
            adaptive_thinking: true,
            display_defaults_omitted: false,
            sampling_removed: false,
            thinking_always_on: false,
        };
    }

    // Pre-4.6 generations keep fixed budgets and sampling. The `-4-20…` arms
    // catch dated Claude 4.0 snapshots (`claude-sonnet-4-20250514`), whose
    // family digit is followed straight by the date.
    if name.contains("opus-4-5")
        || name.contains("opus-4-1")
        || name.contains("opus-4-0")
        || name.contains("opus-4-20")
        || name.contains("sonnet-4-5")
        || name.contains("sonnet-4-0")
        || name.contains("sonnet-4-20")
        || name.contains("haiku-4-5")
        || name.contains("haiku-4-20")
        || name.contains("claude-3")
        || name.contains("claude-2")
    {
        return AnthropicModelContract::LEGACY;
    }

    // Unknown: assume current generation. See the doc comment.
    AnthropicModelContract {
        adaptive_thinking: true,
        display_defaults_omitted: true,
        sampling_removed: true,
        thinking_always_on: false,
    }
}

/// Whether `model` names a Claude model, as opposed to an Ollama tag, an
/// `OpenRouter` slug for someone else's model, or an `OpenAI` id.
///
/// [`anthropic_model_contract`] answers "which Claude contract"; it assumes
/// the answer here is already yes. Callers that resolve a model dynamically —
/// the summarizer and distiller pick one from `summarization_priority` and
/// fall back to the main model — have to ask this first, or a local
/// `qwen3.5:9b` would be classified as an unrecognized Claude model and lose
/// its sampling parameters.
///
/// `OpenRouter`'s `anthropic/claude-*` slugs count: that path proxies to the
/// same API and rejects the same removed parameters.
#[must_use]
pub fn is_claude_model(model: &str) -> bool {
    model.to_ascii_lowercase().contains("claude")
}

/// The `temperature` to actually send for `model`, given what the caller
/// wanted. `None` means the field must be omitted entirely.
///
/// `temperature` was removed on Opus 5 / 4.8 / 4.7, Sonnet 5, and Fable 5 —
/// sending one is a 400, so a hard-coded temperature on an auxiliary request
/// (summarize, distill, score, extract) fails that request outright once the
/// model it resolves to is current-generation.
#[must_use]
pub fn sampling_temperature_for_model(model: &str, requested: f32) -> Option<f32> {
    if is_claude_model(model) && anthropic_model_contract(model).sampling_removed {
        return None;
    }
    Some(requested)
}

/// Full Anthropic request with tools
#[derive(Debug, Clone, Serialize)]
pub struct AnthropicRequest {
    pub model: String,
    pub messages: Vec<AnthropicMessage>,
    pub max_tokens: u32,
    /// Context window to ask the local runner for (Ollama `num_ctx`).
    ///
    /// Not part of the Anthropic wire format — skipped on serialization — but
    /// it has to ride with the request because only the caller knows how much
    /// context this model can be given on THIS machine. A 12B model's KV cache
    /// at 32k does not fit beside its weights on a 16 GB card already lending
    /// 5 GB to the desktop, and the overrun arrives as
    /// `CUDA error: an illegal memory access was encountered` rather than a
    /// clean allocation failure.
    #[serde(skip_serializing, default)]
    pub context_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    /// Extended thinking configuration (Claude 3.5+ models)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingConfig>,
    /// Prompt caching: automatic mode caches the longest prefix up to the last cacheable block.
    /// Set to `Some(CacheControl::ephemeral())` to enable automatic prompt caching.
    /// Cached input tokens cost 90% less on Anthropic, 50% less on OpenAI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

/// Cache control configuration for prompt caching.
///
/// When set at the request level, enables automatic caching: the API caches all content
/// up to and including the last cacheable block. On subsequent requests with the same
/// prefix, cached content is reused automatically (~5 minute TTL on Anthropic).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheControl {
    #[serde(rename = "type")]
    pub cache_type: String,
}

impl CacheControl {
    /// Create an ephemeral cache control (standard ~5 minute TTL)
    #[must_use]
    pub fn ephemeral() -> Self {
        Self {
            cache_type: "ephemeral".to_string(),
        }
    }
}

/// System prompt block (for OAuth/Claude Code format)
#[derive(Debug, Clone, Serialize)]
pub struct SystemBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub text: String,
    /// Cache control for this specific block (enables explicit cache breakpoints).
    /// Place on the last system block to cache the entire system prompt prefix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

impl SystemBlock {
    /// Create a text system block
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            block_type: "text".to_string(),
            text: content.into(),
            cache_control: None,
        }
    }

    /// Create a text system block with ephemeral cache control
    pub fn text_cached(content: impl Into<String>) -> Self {
        Self {
            block_type: "text".to_string(),
            text: content.into(),
            cache_control: Some(CacheControl::ephemeral()),
        }
    }
}

/// Anthropic request with array-based system prompt (required for OAuth/Claude Code)
#[derive(Debug, Clone, Serialize)]
struct OAuthAnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    system: Vec<SystemBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<ThinkingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

impl OAuthAnthropicRequest {
    /// Convert from a standard AnthropicRequest, prepending Claude Code identity
    fn from_request(request: &AnthropicRequest, prepend_identity: bool) -> Self {
        let mut system = Vec::new();

        if prepend_identity {
            system.push(SystemBlock::text(CLAUDE_CODE_IDENTITY));
        }

        if let Some(ref sys) = request.system {
            if !sys.is_empty() {
                system.push(SystemBlock::text(sys));
            }
        }

        // Place explicit cache breakpoint on the last system block.
        // This ensures the system prompt prefix is cached server-side (~5 min TTL).
        if request.cache_control.is_some() {
            if let Some(last) = system.last_mut() {
                last.cache_control = Some(CacheControl::ephemeral());
            }
        }

        Self {
            model: request.model.clone(),
            messages: request.messages.clone(),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            system,
            tools: request.tools.clone(),
            stream: request.stream,
            thinking: request.thinking.clone(),
            cache_control: request.cache_control.clone(),
        }
    }
}

/// Anthropic API response
#[derive(Debug, Clone, Deserialize)]
pub struct AnthropicResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub response_type: String,
    pub role: String,
    pub content: Vec<ContentBlock>,
    pub model: String,
    pub stop_reason: Option<String>,
    pub usage: Usage,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Tokens written to the prompt cache on this request (billed at cache write rate)
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
    /// Tokens read from the prompt cache on this request (billed at 10% of input rate on Anthropic)
    #[serde(default)]
    pub cache_read_input_tokens: u32,
}

// ============================================================================
// Claude Code Stealth Mode
// ============================================================================

/// Claude Code version to mimic (update as needed)
const CLAUDE_CODE_VERSION: &str = "2.1.2";

/// Claude Code canonical tool names (case-sensitive)
/// Source: https://cchistory.mariozechner.at/data/prompts-2.1.11.md
const CLAUDE_CODE_TOOLS: &[(&str, &str)] = &[
    ("read", "Read"),
    ("read_file", "Read"),
    ("write", "Write"),
    ("write_file", "Write"),
    ("edit", "Edit"),
    ("edit_file", "Edit"),
    ("bash", "Bash"),
    ("exec", "Bash"),
    ("shell", "Bash"),
    ("grep", "Grep"),
    ("glob", "Glob"),
    ("list_dir", "Glob"),
    ("ask_user", "AskUserQuestion"),
    ("web_fetch", "WebFetch"),
    ("web_search", "WebSearch"),
    ("notebook_edit", "NotebookEdit"),
];

/// Claude Code identity prefix (REQUIRED for OAuth tokens)
const CLAUDE_CODE_IDENTITY: &str = "You are Claude Code, Anthropic's official CLI for Claude.";

/// Check if a token is an OAuth token (vs API key)
fn is_oauth_token(token: &str) -> bool {
    // OAuth tokens start with sk-ant-oat (OAuth Access Token)
    // Regular API keys start with sk-ant-api
    token.starts_with("sk-ant-oat") || token.starts_with("oauth:")
}

/// Get the raw token (stripping any prefix)
fn get_raw_token(token: &str) -> &str {
    token.strip_prefix("oauth:").unwrap_or(token)
}

/// Convert a tool name to Claude Code canonical form
fn to_claude_code_tool_name(name: &str) -> String {
    let lower = name.to_lowercase();
    for (pattern, canonical) in CLAUDE_CODE_TOOLS {
        if lower == *pattern {
            return (*canonical).to_string();
        }
    }
    // If no match, return original (with first letter capitalized for consistency)
    let mut chars = name.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().chain(chars).collect(),
    }
}

/// Convert a tool name from Claude Code form back to original
fn from_claude_code_tool_name(name: &str, original_tools: &[ToolDefinition]) -> String {
    let lower = name.to_lowercase();
    // Find original tool by matching lowercase
    for tool in original_tools {
        if tool.name.to_lowercase() == lower {
            return tool.name.clone();
        }
    }
    // Check Claude Code canonical names and map back
    for (pattern, canonical) in CLAUDE_CODE_TOOLS {
        if name == *canonical {
            // Return the pattern as-is (caller should have original)
            return (*pattern).to_string();
        }
    }
    name.to_string()
}

/// LLM client
#[derive(Clone)]
pub struct LlmClient {
    http: Client,
    provider: Provider,
    api_key: String,
    base_url: String,
}

impl LlmClient {
    /// Build HTTP client with sensible timeouts
    fn build_http_client() -> Client {
        Client::builder()
            .timeout(std::time::Duration::from_secs(120))  // 2 min total timeout
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| Client::new())
    }

    /// Ollama-specific HTTP client. Local streaming generations legitimately
    /// run for minutes, so no TOTAL request deadline — a genuine stall is
    /// caught by the read timeout (silence between chunks) instead, which a
    /// healthy stream never hits. Idle connections are never pooled: reusing
    /// a loopback connection the OS/filter stack half-closed surfaces as a
    /// clean EOF mid-stream (observed live 2026-07-23 as "no done=true"
    /// faults while the server logged a client cancel of an actively
    /// decoding task); a fresh connection per request costs sub-millisecond
    /// on loopback.
/// Gemma's reserved stop sentinels: `<unused0>` through `<unused98>`.
///
/// These are the model's own end-of-turn markers. Ollama's chat template is
/// meant to consume them; when it does not, they arrive as ordinary assistant
/// content immediately before the stream closes without a `done=true` message.
/// Recognising one lets the client close the turn cleanly instead of treating a
/// finished reply as an aborted generation.
///
/// Deliberately exact: the whole delta must be the sentinel. A reply that merely
/// mentions `<unused50>` in prose is text, not a stop.
/// The text before a trailing Gemma stop sentinel, if there is one.
///
/// Returns `None` when the content does not end in a sentinel, so callers can
/// pass ordinary text through untouched.
fn strip_trailing_stop_sentinel(content: &str) -> Option<&str> {
    let trimmed = content.trim_end();
    let open = trimmed.rfind("<unused")?;
    if !trimmed.ends_with('>') {
        return None;
    }
    let digits = &trimmed[open + "<unused".len()..trimmed.len() - 1];
    if digits.is_empty() || digits.len() > 2 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(&content[..open])
}

fn is_gemma_stop_sentinel(content: &str) -> bool {
    let trimmed = content.trim();
    let Some(digits) = trimmed
        .strip_prefix("<unused")
        .and_then(|rest| rest.strip_suffix('>'))
    else {
        return false;
    };
    !digits.is_empty() && digits.len() <= 2 && digits.bytes().all(|b| b.is_ascii_digit())
}

    /// The `num_ctx` to ask Ollama for on THIS request.
    ///
    /// Every Ollama call site must go through here. The streaming and
    /// non-streaming builders are separate code paths that both talk to the
    /// same runner on the same GPU, and a sizing rule applied to only one of
    /// them is not a rule — it is a coin flip decided by which path the caller
    /// happened to take. (Observed: the first cut of this landed on the
    /// non-streaming builder alone, while the agent loop streams exclusively,
    /// so it would have had no effect on the thing it was written to fix.)
    ///
    /// Source order: what the caller explicitly asked for, then the live
    /// latch, then min(env pin, probed/nominal size) at latch initialization.
    fn resolve_num_ctx(request: &AnthropicRequest) -> u32 {
        if let Some(explicit) = request.context_limit {
            return explicit;
        }
        if let Some(latched) = Self::latched_num_ctx(&request.model) {
            return latched;
        }
        // Explicit beats computed — downward. `NANNA_OLLAMA_NUM_CTX` is set by
        // someone who measured THIS machine with THIS model;
        // `fit_context_to_free_vram` has only inferred a size from free bytes.
        // Letting the heuristic promote past the operator's number was a
        // silent override of a deliberate setting, and it was expensive: the
        // same model on the same mission scored 31/42 at the operator's 16384
        // and 8/42 once the heuristic promoted it to 32768. Fitting in VRAM is
        // necessary, not sufficient — a 9B model given twice the window does
        // not use it well, it loses the thread and rewrites work it had passing.
        //
        // So the pin is a ceiling on the STARTING latch, not a disable of the
        // machinery around it: the start is min(env, sized), which lets the
        // env shrink the start (a probed 16384 that faults under sustained
        // mission KV load starts stable at a pinned 8192) but never grow it
        // past what the probe — or the nominal fallback when the card cannot
        // be measured — says this machine supports. And a pin that is still
        // too large faults under load and gets walked down by
        // `demote_context` (the caller demotes on the second fault of a run —
        // one fault at a sane pin is usually a transient blip, healed by a
        // runner reset alone): demotion below the pin keeps working because
        // the latch, not the env, is consulted once it exists.
        let sized = Self::fit_context_to_free_vram(&request.model).unwrap_or(16_384);
        let start = match Self::env_num_ctx() {
            Some(pinned) => {
                let start = pinned.min(sized);
                tracing::info!(
                    model = %request.model,
                    pinned = start,
                    requested = pinned,
                    sized,
                    source = "env",
                    "NANNA_OLLAMA_NUM_CTX pins the starting num_ctx latch; \
                     demote_context can still walk it lower on GPU faults"
                );
                start
            }
            None => sized,
        };
        Self::latch_num_ctx(&request.model, start);
        start
    }

    /// The operator's explicit context size, if they set one.
    fn env_num_ctx() -> Option<u32> {
        std::env::var("NANNA_OLLAMA_NUM_CTX")
            .ok()
            .and_then(|v| v.trim().parse::<u32>().ok())
            .filter(|v| *v >= 2048)
    }

    fn ctx_latch() -> &'static std::sync::Mutex<std::collections::HashMap<String, u32>> {
        static LATCH: std::sync::OnceLock<
            std::sync::Mutex<std::collections::HashMap<String, u32>>,
        > = std::sync::OnceLock::new();
        LATCH.get_or_init(Default::default)
    }

    /// The latch is keyed on the bare model name. The request carries
    /// `gemma4:12b` while callers hold the routed `ollama/gemma4:12b`, and two
    /// spellings of one model means the demotion writes an entry the request
    /// path never reads — a self-correcting loop that silently corrects
    /// nothing.
    fn ctx_latch_key(model: &str) -> String {
        model.strip_prefix("ollama/").unwrap_or(model).to_string()
    }

    fn latched_num_ctx(model: &str) -> Option<u32> {
        Self::ctx_latch()
            .lock()
            .ok()?
            .get(&Self::ctx_latch_key(model))
            .copied()
    }

    fn latch_num_ctx(model: &str, ctx: u32) {
        if let Ok(mut g) = Self::ctx_latch().lock() {
            g.insert(Self::ctx_latch_key(model), ctx);
        }
    }

    /// The LIVE effective `num_ctx` for `model`, if the Ollama sizing path has
    /// latched one. `None` means the model was never sized here (cloud models,
    /// or a local model before its first request) and the provider-reported
    /// window stands.
    ///
    /// This is deliberately a poll-getter rather than a change notification:
    /// the latch is process-global state keyed by model name, and the agent
    /// loop already has a natural per-iteration point to re-read it. Callers
    /// budgeting prompts MUST treat the value as live — [`Self::demote_context`]
    /// rewrites it mid-run under VRAM pressure.
    #[must_use]
    pub fn effective_num_ctx(model: &str) -> Option<u32> {
        Self::latched_num_ctx(model)
    }

    /// Drop `model` to the next smaller context rung after the runner failed
    /// in a way that means it ran out of GPU memory. Returns the new size, or
    /// `None` when the latch already sits at (or below) the minimum viable
    /// window — demoting past the floor manufactures a window the agent will
    /// loudly refuse, so the fault is surfaced instead of "healed" into an
    /// unusable size.
    ///
    /// This is the half of the sizing loop that measurement cannot supply.
    /// Sizing happens on the FIRST request, which is structurally the most
    /// optimistic moment there is — the desktop compositor has not grown, the
    /// embedder has not loaded, and this model's own weights are not yet
    /// resident. Re-measuring on later turns does not fix it: by then free VRAM
    /// is dominated by our own allocation, and acting on it would change
    /// `num_ctx`, which makes Ollama evict and reload the model, which frees
    /// the memory that justified the change. The only signal that is both late
    /// enough to be honest and unambiguous is the failure itself.
    ///
    /// The rung is **3/4 of the current latch**, rounded DOWN to
    /// [`NUM_CTX_QUANTUM`], never below `min_viable_ctx` (rounded UP to the
    /// same quantum; `None` falls back to [`DEFAULT_MIN_VIABLE_NUM_CTX`]).
    /// Derivation of the step: the KV cache scales linearly with `num_ctx`,
    /// so a 25% cut frees ~25% of the KV allocation — comfortably more than
    /// one retry's worth of KV growth, which is what the step must outpace —
    /// while halving (the old ladder) overshoots: it assumed faults repeat
    /// deterministically, but with a sane starting pin a single fault is
    /// usually a transient load blip, and 8192 → 4096 destroyed a viable
    /// window by landing below the agent's ~4.2k pressure-tier floor
    /// (observed live 2026-08-03: 38 below-floor stops, 8 resume cycles,
    /// run dead in 8 minutes). 3/4 is geometric, so it still reaches any
    /// floor in bounded rungs — ⌈log₄∕₃(start/floor)⌉, 7 from 32768 to 4608
    /// — and its 25% cuts can never skip PAST a viable window the way a 50%
    /// cut can.
    pub fn demote_context(model: &str, min_viable_ctx: Option<u32>) -> Option<u32> {
        let clamp = min_viable_ctx
            .unwrap_or(DEFAULT_MIN_VIABLE_NUM_CTX)
            .div_ceil(NUM_CTX_QUANTUM)
            .saturating_mul(NUM_CTX_QUANTUM);
        let key = Self::ctx_latch_key(model);
        let mut guard = Self::ctx_latch().lock().ok()?;
        let current = guard.get(&key).copied().unwrap_or(NUM_CTX_CEILING);
        if current <= clamp {
            tracing::warn!(
                model,
                current,
                floor = clamp,
                "GPU memory failure at the context floor — NOT demoting further; \
                 a smaller window would be below the minimum viable request, so \
                 the fault stays loud (below-floor/fault failures are resumable) \
                 instead of being healed into an unusable window"
            );
            return None;
        }
        let stepped = (current / 4 * 3) / NUM_CTX_QUANTUM * NUM_CTX_QUANTUM;
        let clamped = stepped < clamp;
        let next = if clamped { clamp } else { stepped };
        guard.insert(key, next);
        tracing::warn!(
            model,
            from = current,
            to = next,
            floor = clamp,
            clamped,
            "GPU memory failure — demoting context to 3/4 (quantized to 512, \
             clamped at the minimum viable window) and retrying; the effective \
             window shrank and downstream budgets re-derive from it (readable \
             via effective_num_ctx / effective_context_window)"
        );
        Some(next)
    }

    /// Largest context that fits in the VRAM free *right now*, or `None` when
    /// the GPU cannot be queried (no NVIDIA tooling, or a non-CUDA backend) —
    /// in which case the caller falls back to a fixed size.
    ///
    /// Deliberately empirical. The analytic route needs the model's
    /// sliding-window interleave and KV quantisation to be known, and getting
    /// those wrong is worse than measuring: for gemma4:12b the naive formula
    /// predicts 1.5 MB/token and the truth is ~10 KB/token, a 150x error. Two
    /// numbers we can actually read — free VRAM and the model's file size —
    /// plus a reserve for the prefill compute buffer, give a bound that adapts
    /// to whatever else is on the card.
    fn fit_context_to_free_vram(model: &str) -> Option<u32> {
        let free_bytes = Self::nvidia_free_vram_bytes()? as f64;
        let weights_bytes = Self::ollama_model_size_bytes(model)? as f64;
        let (ours_resident, others_resident) = Self::ollama_resident_vram(model);
        Some(Self::fit_context_for_budget(
            free_bytes,
            weights_bytes,
            ours_resident as f64,
            others_resident as f64,
        ))
    }

    /// VRAM currently held by `model` and, separately, by every OTHER model the
    /// runner has resident — the embedder, chiefly.
    ///
    /// Both halves matter and they pull in opposite directions, so they are
    /// measured rather than assumed. Guessing the second one at 2.5 GB (a
    /// plausible embedder size) against an actual 0.32 GB was enough on its own
    /// to drive the budget negative and pin the context at the 4096 floor.
    fn ollama_resident_vram(model: &str) -> (u64, u64) {
        let name = model.strip_prefix("ollama/").unwrap_or(model);
        let Ok(out) = std::process::Command::new("curl")
            .args(["-s", "http://127.0.0.1:11434/api/ps"])
            .output()
        else {
            return (0, 0);
        };
        let body = String::from_utf8_lossy(&out.stdout);
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body) else {
            return (0, 0);
        };
        let Some(models) = parsed.get("models").and_then(serde_json::Value::as_array) else {
            return (0, 0);
        };
        let (mut ours, mut others) = (0u64, 0u64);
        for m in models {
            let vram = m
                .get("size_vram")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            if m.get("name").and_then(serde_json::Value::as_str) == Some(name) {
                ours += vram;
            } else {
                others += vram;
            }
        }
        (ours, others)
    }

    /// The arithmetic of [`Self::fit_context_to_free_vram`], split out from the
    /// measurements so it can be tested against the runs we actually observed
    /// rather than only against itself.
    fn fit_context_for_budget(
        free_bytes: f64,
        weights_bytes: f64,
        ours_resident: f64,
        others_resident: f64,
    ) -> u32 {
        /// Per-context-token VRAM cost: KV cache plus the attention scratch the
        /// prefill graph allocates.
        ///
        /// Measured, not derived, and the derivation is the reason. gemma4:12b
        /// reports `sliding_window = 1024` with `key_length_swa = 256`, so most
        /// of its 48 layers attend over a fixed 1024-token window and their KV
        /// does NOT grow with `n_ctx` at all — only the few full-attention
        /// layers do. Without knowing the exact interleave (Ollama does not
        /// report it) any closed form is a guess, and the naive one is off by
        /// 150x. So: KV measured directly at 10.5 KB/token
        /// ((8.4 GB - 8.1 GB) / (32768 - 4096)), plus ~32 KB/token for the
        /// full-attention scratch (`n_ubatch 512 x n_head 16 x 4 bytes`).
        ///
        /// Reading high is the safe direction — it costs context, not a run.
        const BYTES_PER_CTX_TOKEN: f64 = 43_000.0;
        /// Floor on what to set aside for an embedder that has not loaded yet.
        /// It arrives on the first `remember` and stays co-resident for the
        /// rest of the run, so budgeting as though it will never come is how a
        /// run that measured fine at minute one dies at minute ten.
        const EMBEDDER_RESERVE: f64 = 1.0 * 1024.0 * 1024.0 * 1024.0;
        /// Driver allocations, fragmentation, the fixed part of the compute
        /// graph, and the desktop's own growth after we measure.
        ///
        /// That last term is the reason this is not smaller. Sizing runs on the
        /// first request, which is the emptiest the card will be all run: the
        /// GUI's webview alone was measured taking a further 0.8 GB in the
        /// minute after the daemon came up. Budgeting against that instant
        /// without slack is how the first request picks a size the tenth
        /// request cannot honour.
        const SLACK: f64 = 1.2 * 1024.0 * 1024.0 * 1024.0;
        /// Below this a run is not worth starting; above it, diminishing
        /// returns for an agent whose steps rarely exceed a few thousand
        /// tokens.
        const MIN_CTX: u32 = 4_096;
        const MAX_CTX: u32 = 32_768;

        // Add back what THIS model already holds before subtracting what it
        // costs, so the answer does not depend on whether it happens to be
        // loaded yet.
        //
        // Without this the function contradicts itself: before the load it sees
        // a free card and picks a large context; after the load the same card
        // looks 8 GB poorer, it picks the 4096 floor, and because Ollama keys a
        // resident instance on its options, that new number evicts and reloads
        // the model — which frees the memory that justified shrinking, so the
        // next turn picks large again. Measured live: this path pinned
        // gemma4:12b to 4096 on a card with 10.4 GB genuinely available.
        let effective_free = free_bytes + ours_resident;
        let reserve = others_resident.max(EMBEDDER_RESERVE) + SLACK;
        let usable = effective_free - weights_bytes - reserve;
        if usable <= 0.0 {
            return MIN_CTX;
        }
        let raw = (usable / BYTES_PER_CTX_TOKEN) as u64;

        // Snap DOWN to a power-of-two bucket.
        //
        // Not cosmetic: Ollama keys a loaded model instance partly on its
        // options, so a changed `num_ctx` evicts and reloads the model — tens
        // of seconds of stall and a VRAM churn, mid-run. "Every turn" has to
        // mean the value is *rechecked* every turn, not that it is free to
        // wander: free VRAM drifting by a few hundred MB as the desktop
        // breathes must not reload a 7.5 GB model. Buckets make the answer
        // stable across ordinary drift while still moving when something real
        // changes (a game starts, the embedder loads).
        const BUCKETS: [u32; 4] = [32_768, 16_384, 8_192, 4_096];
        let fitted = BUCKETS
            .iter()
            .copied()
            .find(|b| u64::from(*b) <= raw)
            .unwrap_or(MIN_CTX);
        fitted.clamp(MIN_CTX, MAX_CTX)
    }

    /// Free VRAM in bytes, via `nvidia-smi`. `None` on any non-NVIDIA or
    /// tooling-absent system, which is a normal answer, not an error.
    fn nvidia_free_vram_bytes() -> Option<u64> {
        let out = std::process::Command::new("nvidia-smi")
            .args(["--query-gpu=memory.free", "--format=csv,noheader,nounits"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let mib: u64 = String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()?
            .trim()
            .parse()
            .ok()?;
        Some(mib * 1024 * 1024)
    }

    /// On-disk size of an Ollama model, a good proxy for its resident weights.
    ///
    /// Cached per model name for the life of the process: free VRAM is the
    /// term that has to be live, but a model's file size cannot change while
    /// we are mid-run against it, and this runs on every single request.
    fn ollama_model_size_bytes(model: &str) -> Option<u64> {
        static CACHE: std::sync::OnceLock<
            std::sync::Mutex<std::collections::HashMap<String, Option<u64>>>,
        > = std::sync::OnceLock::new();
        let cache = CACHE.get_or_init(Default::default);
        let name = model.strip_prefix("ollama/").unwrap_or(model).to_string();

        if let Ok(guard) = cache.lock() {
            if let Some(hit) = guard.get(&name) {
                return *hit;
            }
        }

        let looked_up = (|| {
            let out = std::process::Command::new("curl")
                .args(["-s", "http://127.0.0.1:11434/api/tags"])
                .output()
                .ok()?;
            let body = String::from_utf8_lossy(&out.stdout);
            let parsed: serde_json::Value = serde_json::from_str(&body).ok()?;
            parsed
                .get("models")?
                .as_array()?
                .iter()
                .find(|m| m.get("name").and_then(|n| n.as_str()) == Some(name.as_str()))
                .and_then(|m| m.get("size"))
                .and_then(serde_json::Value::as_u64)
        })();

        // A miss is cached too. If the model is not in `/api/tags` it will not
        // appear later either, and retrying a failed lookup on every request
        // would spawn a process per turn to learn the same thing.
        if let Ok(mut guard) = cache.lock() {
            guard.insert(name, looked_up);
        }
        looked_up
    }

    fn build_ollama_http_client() -> Client {
        Client::builder()
            .read_timeout(std::time::Duration::from_secs(120))
            .connect_timeout(std::time::Duration::from_secs(10))
            .pool_max_idle_per_host(0)
            .build()
            .unwrap_or_else(|_| Client::new())
    }

    /// Pin an Ollama base URL to IPv4 loopback. "localhost" resolves to ::1
    /// first on Windows, and the chat stream was the ONLY component talking
    /// to Ollama over v6 loopback (health probes and runner surgery already
    /// use 127.0.0.1) — the same v6 path where streams were cut
    /// mid-generation. Remote hosts pass through untouched.
    fn normalize_ollama_url(url: String) -> String {
        url.replacen("://localhost", "://127.0.0.1", 1)
    }

    /// Create a new Anthropic client with API key
    pub fn anthropic(api_key: impl Into<String>) -> Self {
        Self {
            http: Self::build_http_client(),
            provider: Provider::Anthropic,
            api_key: api_key.into(),
            base_url: "https://api.anthropic.com".to_string(),
        }
    }

    /// Create a new Anthropic client with OAuth token
    /// Uses Authorization: Bearer header instead of x-api-key
    pub fn anthropic_oauth(oauth_token: impl Into<String>) -> Self {
        Self {
            http: Self::build_http_client(),
            provider: Provider::Anthropic,
            api_key: format!("oauth:{}", oauth_token.into()), // Prefix to indicate OAuth
            base_url: "https://api.anthropic.com".to_string(),
        }
    }

    /// Create a new `OpenAI` client
    pub fn openai(api_key: impl Into<String>) -> Self {
        Self {
            http: Self::build_http_client(),
            provider: Provider::OpenAI,
            api_key: api_key.into(),
            base_url: "https://api.openai.com".to_string(),
        }
    }

    /// Create a new `OpenRouter` client
    pub fn openrouter(api_key: impl Into<String>) -> Self {
        Self {
            http: Self::build_http_client(),
            provider: Provider::OpenRouter,
            api_key: api_key.into(),
            base_url: "https://openrouter.ai/api".to_string(),
        }
    }

    /// Create a new `GitHub Models` client (uses GitHub PAT)
    /// GitHub Models provides access to various LLMs including GPT-4o, Llama, Mistral, etc.
    pub fn github_models(api_key: impl Into<String>) -> Self {
        Self {
            http: Self::build_http_client(),
            provider: Provider::GitHubModels,
            api_key: api_key.into(),
            base_url: "https://models.inference.ai.azure.com".to_string(),
        }
    }

    /// Create a new `Claude Proxy` client for claude-max-api-proxy
    /// This proxies requests through a local server that uses Claude Code CLI credentials
    /// Default URL is http://localhost:3456
    pub fn claude_proxy(base_url: impl Into<String>) -> Self {
        Self {
            http: Self::build_http_client(),
            provider: Provider::ClaudeProxy,
            api_key: String::new(), // Proxy handles auth via Claude Code CLI
            base_url: base_url.into(),
        }
    }

    /// Create a Claude Proxy client with default localhost URL
    pub fn claude_proxy_default() -> Self {
        Self::claude_proxy("http://localhost:3456")
    }

    /// Create a new `Ollama` client (local, no API key needed)
    pub fn ollama(base_url: impl Into<String>) -> Self {
        Self {
            http: Self::build_ollama_http_client(),
            provider: Provider::Ollama,
            api_key: String::new(), // Ollama doesn't need auth
            base_url: Self::normalize_ollama_url(base_url.into()),
        }
    }

    /// Create an Ollama client with an API key (for remote/authenticated instances)
    pub fn ollama_with_key(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            http: Self::build_ollama_http_client(),
            provider: Provider::Ollama,
            api_key: api_key.into(),
            base_url: Self::normalize_ollama_url(base_url.into()),
        }
    }

    /// Create an Ollama client with default localhost URL
    pub fn ollama_default() -> Self {
        Self::ollama("http://localhost:11434")
    }

    /// Check if this client uses OAuth authentication
    fn is_oauth(&self) -> bool {
        is_oauth_token(&self.api_key)
    }

    /// Get the actual token (stripping oauth: prefix if present)
    fn get_token(&self) -> &str {
        get_raw_token(&self.api_key)
    }

    /// Apply Anthropic authentication headers to a request builder
    /// For OAuth, includes required beta headers and user-agent (matching Claude Code exactly)
    fn apply_anthropic_auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.is_oauth() {
            // Stealth mode: Mimic Claude Code's headers exactly
            // Beta features: claude-code identity, oauth, interleaved thinking, fine-grained tool streaming
            let beta = "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14";
            let user_agent = format!("claude-cli/{CLAUDE_CODE_VERSION} (external, cli)");
            debug!(
                beta = %beta,
                user_agent = %user_agent,
                "OAuth stealth mode: applying Claude Code headers"
            );
            builder
                .header("Authorization", format!("Bearer {}", self.get_token()))
                .header("accept", "application/json")
                .header("anthropic-beta", beta)
                .header("user-agent", user_agent)
                .header("x-app", "cli")
                .header("anthropic-dangerous-direct-browser-access", "true")
        } else {
            builder.header("x-api-key", &self.api_key)
        }
    }

    /// Validate the API key by making a lightweight request.
    ///
    /// Apply Ollama auth header if an API key is configured.
    fn apply_ollama_auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if !self.api_key.is_empty() {
            builder.header("Authorization", format!("Bearer {}", self.api_key))
        } else {
            builder
        }
    }

    /// # Errors
    ///
    /// Returns `LlmError::Api` with status 401 if the API key is invalid.
    /// Returns `LlmError::Network` if the request fails.
    pub async fn validate(&self) -> Result<(), LlmError> {
        match self.provider {
            Provider::Anthropic => {
                // Use count_tokens endpoint for validation - lightweight
                let request_builder = self
                    .http
                    .post(format!("{}/v1/messages/count_tokens", self.base_url));
                let response = self.apply_anthropic_auth(request_builder)
                    .header("anthropic-version", "2023-06-01")
                    .header("content-type", "application/json")
                    .json(&serde_json::json!({
                        "model": "claude-sonnet-4-20250514",
                        "messages": [{"role": "user", "content": "hi"}]
                    }))
                    .send()
                    .await?;

                if response.status().as_u16() == 401 {
                    return Err(LlmError::Api {
                        status: 401,
                        message: "Invalid API key or OAuth token".to_string(),
                    });
                }
                Ok(())
            }
            _ => Ok(()), // Skip validation for other providers for now
        }
    }

    /// Get the provider for this client
    #[must_use]
    pub fn provider(&self) -> Provider {
        self.provider
    }

    /// Get model information with caching.
    ///
    /// Checks the file cache first, then fetches from the API if needed.
    /// Falls back to defaults if the API doesn't provide context window info.
    ///
    /// # Errors
    ///
    /// Returns `LlmError` if both cache and API fail (uses defaults as final fallback).
    pub async fn get_model_info(&self, model: &str, cache: Option<&ModelInfoCache>) -> ModelInfo {
        // Check cache first
        if let Some(cache) = cache {
            if let Some(info) = cache.get(model) {
                return clamp_model_info_to_effective_window(model, info);
            }
        }

        // Fetch from API
        let info = match self.fetch_model_info(model).await {
            Ok(info) => info,
            Err(e) => {
                warn!(model = %model, error = %e, "Failed to fetch model info, using defaults");
                default_model_info(model, &format!("{:?}", self.provider))
            }
        };

        // Cache the result — the UNCLAMPED provider claim. The cache stores
        // static facts about the model; the effective-window clamp below is
        // live per-process state (the Ollama num_ctx latch) and must never be
        // persisted as if the model itself shrank.
        if let Some(cache) = cache {
            if let Err(e) = cache.set(&info) {
                warn!(model = %model, error = %e, "Failed to cache model info");
            }
        }

        clamp_model_info_to_effective_window(model, info)
    }

    /// Force refresh model info from API (bypasses cache).
    ///
    /// # Errors
    ///
    /// Returns `LlmError` if the API request fails.
    pub async fn refresh_model_info(&self, model: &str, cache: Option<&ModelInfoCache>) -> Result<ModelInfo, LlmError> {
        let info = self.fetch_model_info(model).await?;

        if let Some(cache) = cache {
            cache.set(&info)?;
        }

        Ok(info)
    }

    /// Fetch model info from the provider API
    async fn fetch_model_info(&self, model: &str) -> Result<ModelInfo, LlmError> {
        match self.provider {
            Provider::Anthropic => self.fetch_anthropic_model_info(model).await,
            Provider::OpenAI => self.fetch_openai_model_info(model).await,
            Provider::Ollama => self.fetch_ollama_model_info(model).await,
            Provider::OpenRouter => self.fetch_openrouter_model_info(model).await,
            // For proxies and GitHub Models, use defaults
            Provider::ClaudeProxy | Provider::GitHubModels => {
                Ok(default_model_info(model, &format!("{:?}", self.provider)))
            }
        }
    }

    /// Fetch model info from Anthropic API
    async fn fetch_anthropic_model_info(&self, model: &str) -> Result<ModelInfo, LlmError> {
        // Anthropic's /v1/models/{model_id} endpoint
        let url = format!("{}/v1/models/{}", self.base_url, model);

        let request_builder = self.http.get(&url);
        let response = self
            .apply_anthropic_auth(request_builder)
            .header("anthropic-version", "2023-06-01")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            // If model not found or API doesn't support this, use defaults
            if status == 404 {
                info!(model = %model, "Model not found in API, using defaults");
                return Ok(default_model_info(model, "anthropic"));
            }
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::Api { status, message });
        }

        // Parse response - Anthropic may or may not include context_window
        #[derive(Deserialize)]
        struct AnthropicModelResponse {
            id: String,
            #[serde(default)]
            context_window: Option<usize>,
            #[serde(default)]
            max_output_tokens: Option<usize>,
        }

        let api_response: AnthropicModelResponse = response.json().await?;

        // Use API values if available, otherwise use defaults
        let defaults = default_model_info(model, "anthropic");
        Ok(ModelInfo {
            id: api_response.id,
            context_window: api_response.context_window.unwrap_or(defaults.context_window),
            max_output_tokens: api_response.max_output_tokens.unwrap_or(defaults.max_output_tokens),
            supports_tools: defaults.supports_tools,
            supports_vision: defaults.supports_vision,
            embedding_dimension: None, // Anthropic doesn't provide embedding models via this API
            cached_at: current_timestamp(),
            provider: "anthropic".to_string(),
        })
    }

    /// Fetch model info from OpenAI API
    async fn fetch_openai_model_info(&self, model: &str) -> Result<ModelInfo, LlmError> {
        let url = format!("{}/v1/models/{}", self.base_url, model);

        let response = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            if status == 404 {
                return Ok(default_model_info(model, "openai"));
            }
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::Api { status, message });
        }

        // OpenAI doesn't return context_window in their model endpoint
        // So we use defaults with the model ID from the API
        #[derive(Deserialize)]
        struct OpenAIModelResponse {
            id: String,
        }

        let api_response: OpenAIModelResponse = response.json().await?;
        Ok(default_model_info(&api_response.id, "openai").with_timestamp())
    }

    /// Fetch model info from Ollama API
    async fn fetch_ollama_model_info(&self, model: &str) -> Result<ModelInfo, LlmError> {
        let url = format!("{}/api/show", self.base_url);

        let response = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "name": model }))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            if status == 404 {
                return Ok(default_model_info(model, "ollama"));
            }
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::Api { status, message });
        }

        // Ollama returns model info including context length and embedding dimension
        #[derive(Deserialize)]
        struct OllamaShowResponse {
            #[serde(default)]
            model_info: Option<OllamaModelInfo>,
        }

        #[derive(Deserialize)]
        struct OllamaModelInfo {
            #[serde(rename = "general.context_length", default)]
            context_length: Option<usize>,
            /// Embedding dimension (for embedding models)
            #[serde(rename = "general.embedding_length", default)]
            embedding_length: Option<usize>,
        }

        let api_response: OllamaShowResponse = response.json().await?;

        // Use the model metadata verbatim. In particular, do not silently raise
        // a local model's configured context window above what Ollama reports.
        let defaults = default_model_info(model, "ollama");
        let (context_window, embedding_dimension) = match api_response.model_info {
            Some(info) => (
                info.context_length.unwrap_or(defaults.context_window),
                info.embedding_length,
            ),
            None => (defaults.context_window, None),
        };

        Ok(ModelInfo {
            id: model.to_string(),
            context_window,
            // Ollama does not publish a distinct output cap. Derive the safe
            // generation budget from this model's reported total window.
            max_output_tokens: context_window / 2,
            supports_tools: true, // Ollama v0.5+ supports tool calling
            supports_vision: false,
            embedding_dimension,
            cached_at: current_timestamp(),
            provider: "ollama".to_string(),
        })
    }

    /// Fetch model info from OpenRouter API
    async fn fetch_openrouter_model_info(&self, model: &str) -> Result<ModelInfo, LlmError> {
        // OpenRouter has a models endpoint that lists all models
        let url = format!("{}/v1/models", self.base_url);

        let response = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;

        if !response.status().is_success() {
            return Ok(default_model_info(model, "openrouter"));
        }

        #[derive(Deserialize)]
        struct OpenRouterModelsResponse {
            data: Vec<OpenRouterModel>,
        }

        #[derive(Deserialize)]
        struct OpenRouterModel {
            id: String,
            #[serde(default)]
            context_length: Option<usize>,
            #[serde(default)]
            top_provider: Option<OpenRouterTopProvider>,
        }

        #[derive(Deserialize)]
        struct OpenRouterTopProvider {
            #[serde(default)]
            max_completion_tokens: Option<usize>,
        }

        let api_response: OpenRouterModelsResponse = response.json().await?;

        // Find our model in the list
        if let Some(m) = api_response.data.iter().find(|m| m.id == model) {
            return Ok(ModelInfo {
                id: m.id.clone(),
                context_window: m.context_length.unwrap_or(UNKNOWN_CONTEXT_WINDOW),
                max_output_tokens: m
                    .top_provider
                    .as_ref()
                    .and_then(|p| p.max_completion_tokens)
                    .unwrap_or_else(|| m.context_length.unwrap_or(UNKNOWN_CONTEXT_WINDOW) / 2),
                supports_tools: true, // OpenRouter usually routes to capable models
                supports_vision: false,
                embedding_dimension: None,
                cached_at: current_timestamp(),
                provider: "openrouter".to_string(),
            });
        }

        Ok(default_model_info(model, "openrouter"))
    }

    /// Send a completion request (simple, no tools).
    ///
    /// # Errors
    ///
    /// Returns `LlmError::Api` if the API returns an error.
    /// Returns `LlmError::Network` if the request fails.
    pub async fn complete(&self, request: &CompletionRequest) -> Result<String, LlmError> {
        match self.provider {
            Provider::Anthropic => self.complete_anthropic_simple(request).await,
            Provider::OpenAI | Provider::OpenRouter | Provider::GitHubModels | Provider::ClaudeProxy => self.complete_openai(request).await,
            Provider::Ollama => self.complete_ollama(request).await,
        }
    }

    /// Send a full Anthropic-format request with tools.
    ///
    /// Dispatches to the appropriate provider backend:
    /// - Anthropic: Native Anthropic API (`/v1/messages`)
    /// - OpenAI/OpenRouter/GitHub/ClaudeProxy: Converts to OpenAI chat completions format
    /// - Ollama: Converts to Ollama `/api/chat` format
    ///
    /// # Errors
    ///
    /// Returns `LlmError::Api` if the API returns an error.
    /// Returns `LlmError::Network` if the request fails.
    pub async fn complete_anthropic(&self, request: &AnthropicRequest) -> Result<AnthropicResponse, LlmError> {
        match self.provider {
            Provider::Anthropic => self.complete_anthropic_native(request).await,
            Provider::OpenAI | Provider::OpenRouter | Provider::GitHubModels | Provider::ClaudeProxy =>
                self.complete_anthropic_via_openai(request).await,
            Provider::Ollama =>
                self.complete_anthropic_via_ollama(request).await,
        }
    }

    /// Native Anthropic API implementation for complete_anthropic
    async fn complete_anthropic_native(&self, request: &AnthropicRequest) -> Result<AnthropicResponse, LlmError> {
        pace_provider_requests(self.provider).await;
        let is_oauth = self.is_oauth();

        // Store original tools for reverse mapping
        let original_tools = request.tools.clone().unwrap_or_default();

        let request_builder = self
            .http
            .post(format!("{}/v1/messages", self.base_url));

        let response = if is_oauth {
            // OAuth mode: Use array-based system prompt and remap tool names
            let mut oauth_request = OAuthAnthropicRequest::from_request(request, true);

            // Remap tool names to Claude Code format
            if let Some(ref mut tools) = oauth_request.tools {
                for tool in tools {
                    tool.name = to_claude_code_tool_name(&tool.name);
                }
            }

            debug!(
                system_blocks = oauth_request.system.len(),
                tool_names = ?oauth_request.tools.as_ref().map(|t| t.iter().map(|x| x.name.as_str()).collect::<Vec<_>>()),
                "OAuth request prepared with array-based system prompt"
            );

            self.apply_anthropic_auth(request_builder)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&oauth_request)
                .send()
                .await?
        } else {
            // Standard mode: Use string-based system prompt
            self.apply_anthropic_auth(request_builder)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(request)
                .send()
                .await?
        };

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::from_api_response(status, message));
        }

        let mut result: AnthropicResponse = response.json().await?;

        // Log prompt cache usage
        if result.usage.cache_read_input_tokens > 0 || result.usage.cache_creation_input_tokens > 0 {
            info!(
                cache_read = result.usage.cache_read_input_tokens,
                cache_write = result.usage.cache_creation_input_tokens,
                input = result.usage.input_tokens,
                "📦 Prompt cache: read {} tokens, wrote {} tokens",
                result.usage.cache_read_input_tokens,
                result.usage.cache_creation_input_tokens,
            );
        }

        // Remap tool names back in response
        if is_oauth {
            for block in &mut result.content {
                if let ContentBlock::ToolUse { name, .. } = block {
                    *name = from_claude_code_tool_name(name, &original_tools);
                }
            }
        }

        Ok(result)
    }

    /// Convert AnthropicRequest → OpenAI chat completions and execute
    async fn complete_anthropic_via_openai(&self, request: &AnthropicRequest) -> Result<AnthropicResponse, LlmError> {
        pace_provider_requests(self.provider).await;
        let (messages_json, tools_json) = anthropic_to_openai_request(request);

        let mut body = serde_json::json!({
            "model": request.model,
            "messages": messages_json,
            "max_tokens": request.max_tokens,
        });
        if let Some(temp) = request.temperature {
            body["temperature"] = serde_json::json!(temp);
        }
        if let Some(tools) = tools_json {
            body["tools"] = tools;
        }

        let response = self
            .http
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        learn_rate_limit_headers(self.provider, response.headers());
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::from_api_response(status, message));
        }

        let result: serde_json::Value = response.json().await?;
        let response = openai_response_to_anthropic(&request.model, &result)?;

        // Log OpenAI prompt cache usage (automatic for prompts >1024 tokens, 50% discount)
        if response.usage.cache_read_input_tokens > 0 {
            info!(
                cache_read = response.usage.cache_read_input_tokens,
                input = response.usage.input_tokens,
                "📦 OpenAI prompt cache: read {} cached tokens (50% discount)",
                response.usage.cache_read_input_tokens,
            );
        }

        Ok(response)
    }

    /// Convert AnthropicRequest → Ollama /api/chat and execute
    async fn complete_anthropic_via_ollama(&self, request: &AnthropicRequest) -> Result<AnthropicResponse, LlmError> {
        pace_provider_requests(self.provider).await;
        let (messages_json, tools_json) = anthropic_to_ollama_request(request);

        let mut body = serde_json::json!({
            "model": request.model,
            "messages": messages_json,
            "stream": false,
        });

        // Ollama uses "options" for parameters
        let mut options = serde_json::Map::new();
        if let Some(temp) = request.temperature {
            options.insert("temperature".to_string(), serde_json::json!(temp));
        } else {
            // No caller override: Modelfile defaults for thinking models run
            // temp 1.0, which is hot for precise code emission. 0.6 is
            // Qwen's own non-thinking recommendation.
            options.insert("temperature".to_string(), serde_json::json!(0.6));
        }
        // Bound generation to the request's token budget. num_predict=-1
        // (unlimited) let a degraded runner emit degenerate tokens without
        // end; the caller's max_tokens already accounts for thinking room.
        options.insert("num_predict".to_string(), serde_json::json!(request.max_tokens));
        // Context size the CALLER asked for, falling back to 32K.
        //
        // This was hardcoded to 32768 for every Ollama model, which ignores the
        // `context_limit` the request already carries and, worse, ignores what
        // the GPU can actually hold. A 12B model's KV cache at 32k does not fit
        // beside its own 8.4 GB of weights on a 16 GB card that is already
        // giving 5 GB to the desktop — and the failure mode is not a clean
        // out-of-memory error, it is `CUDA error: an illegal memory access was
        // encountered` mid-generation, which surfaces here as a truncated
        // stream. Observed live 2026-07-28: gemma4:12b died this way three
        // times in three minutes while qwen3.5:9b (6.7 GB) never once did.
        //
        // Honouring the request means a model that does not fit at 32k can be
        // run at a size that does, instead of being unrunnable.
        //
        // Source order: what the caller asked for, then `NANNA_OLLAMA_NUM_CTX`,
        // then 32k. The env override exists because the right value is a
        // property of THIS MACHINE — how much VRAM is left once the model's
        // weights and the desktop have taken theirs — and nothing inside the
        // agent knows that. A per-model config field is the proper home for it;
        // until then this is the knob that makes an oversized model runnable
        // instead of unrunnable.
        // Size the context to the GPU as it is RIGHT NOW, every turn.
        //
        // A constant cannot be right here. 32k was unrunnable for a 12B model on
        // a 16 GB card sharing with the desktop; 16k would be leaving most of a
        // 24 GB card unused; and the amount of VRAM the desktop holds changes
        // when the user opens a browser or closes a game. The only honest input
        // is free memory measured at the moment of the request.
        //
        // What actually runs out is NOT the KV cache — measured on gemma4:12b,
        // that is ~10 KB/token, so even 32k costs under 0.4 GB. It is the
        // prefill compute buffer, which scales with how much prompt is pushed
        // through at once, and which overruns as
        // `CUDA error: an illegal memory access was encountered` rather than a
        // clean allocation failure. So the budget reserves a fraction of free
        // VRAM for it instead of spending everything on cache.
        let num_ctx = Self::resolve_num_ctx(request);
        tracing::info!(
            model = %request.model,
            num_ctx,
            "sized ollama context to free VRAM"
        );
        options.insert("num_ctx".to_string(), serde_json::json!(num_ctx));
        // qwen3.5's Modelfile ships presence_penalty=1.5 (Qwen's thinking-mode
        // anti-repetition default). Code REQUIRES repetition, and at 1.5 the
        // model visibly degraded working multi-function files into semicolon
        // one-liner slop (observed live, round-5 drive). 1.0 keeps a mild
        // guard; the loop's repetition/spiral detectors cover the rest.
        options.insert("presence_penalty".to_string(), serde_json::json!(1.0));
        // repeat_penalty 1.1 (the Ollama default) punishes exactly the tokens
        // Python repeats by design — newlines and indentation runs — and the
        // model visibly compensates by cramming statements onto one line
        // (observed live across rounds even at presence 1.0). Code presets
        // use 1.0: no repetition penalty at all.
        options.insert("repeat_penalty".to_string(), serde_json::json!(1.0));
        if !options.is_empty() {
            body["options"] = serde_json::Value::Object(options);
        }

        // Ollama supports tools since v0.5
        if let Some(tools) = tools_json {
            body["tools"] = tools;
        }

        // Enable thinking separation for models that support it (qwen3, deepseek-r1, etc.)
        // This makes Ollama return thinking in a separate `thinking` field instead of
        // embedding <think>...</think> tags inside `content`.
        let model_lower = request.model.to_lowercase();
        if model_lower.contains("qwen3")
            || model_lower.contains("deepseek-r1")
            || model_lower.contains("qwq")
        {
            body["think"] = serde_json::json!(true);
        }

        let response = self.apply_ollama_auth(self
            .http
            .post(format!("{}/api/chat", self.base_url))
            .header("content-type", "application/json"))
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::from_api_response(status, message));
        }

        let result: serde_json::Value = response.json().await?;
        ollama_response_to_anthropic(&request.model, &result)
    }

    async fn complete_anthropic_simple(&self, request: &CompletionRequest) -> Result<String, LlmError> {
        // Extract system message
        let system_msg = request
            .messages
            .iter()
            .find(|m| m.role == Role::System)
            .map(|m| m.content.clone());

        // Convert to Anthropic format
        let messages: Vec<AnthropicMessage> = request
            .messages
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|m| AnthropicMessage {
                role: match m.role {
                    Role::User | Role::System => "user", // System filtered above
                    Role::Assistant => "assistant",
                }.to_string(),
                content: vec![ContentBlock::Text { text: m.content.clone() }],
            })
            .collect();

        let anthropic_request = AnthropicRequest {
            context_limit: None,
            model: request.model.clone(),
            messages,
            max_tokens: request.max_tokens.unwrap_or(4096),
            temperature: request.temperature,
            system: system_msg,
            tools: None,
            stream: None,
            thinking: None,
            cache_control: None,
        };

        let result = self.complete_anthropic(&anthropic_request).await?;

        // Extract text from response
        let text = result
            .content
            .iter()
            .filter_map(|c| match c {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        Ok(text)
    }

    async fn complete_openai(&self, request: &CompletionRequest) -> Result<String, LlmError> {
        pace_provider_requests(self.provider).await;
        #[derive(Serialize)]
        struct OpenAIRequest<'a> {
            model: &'a str,
            messages: &'a [Message],
            #[serde(skip_serializing_if = "Option::is_none")]
            max_tokens: Option<u32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            temperature: Option<f32>,
        }

        let body = OpenAIRequest {
            model: &request.model,
            messages: &request.messages,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
        };

        let response = self
            .http
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        learn_rate_limit_headers(self.provider, response.headers());
        let status = response.status().as_u16();
        let text = response.text().await?;
        if !(200..300).contains(&status) {
            return Err(LlmError::from_api_response(status, text));
        }
        Self::parse_openai_completion(status, &text)
    }

    /// Decode an OpenAI-compatible completion body without assuming the happy
    /// shape.
    ///
    /// A blind `response.json::<T>()` here reported every mismatch as reqwest's
    /// opaque "error decoding response body", and OpenRouter produces three
    /// legitimate mismatches: whitespace keep-alive padding ahead of the JSON
    /// while a reasoning model thinks, `content: null` with the text in a
    /// `reasoning` field, and — the expensive one — **HTTP 200 whose body is an
    /// `{"error": ...}` envelope** (free-tier rate limits, moderation, upstream
    /// failures). That last one made a stored, working API key look absent:
    /// every dream-summarization call failed with the same opaque error (386
    /// times on 2026-07-30 alone, zero successes ever), the owner kept
    /// re-entering the key, and nothing changed because the key was never the
    /// problem.
    ///
    /// Rules: real errors surface with the API's own message (so failover and
    /// backoff can react to rate limits), and an undecodable body is reported
    /// WITH a snippet of itself — this failure class must never be opaque
    /// again.
    fn parse_openai_completion(status: u16, text: &str) -> Result<String, LlmError> {
        #[derive(Deserialize)]
        struct OpenAIResponse {
            choices: Vec<Choice>,
        }
        #[derive(Deserialize)]
        struct Choice {
            message: ResponseMessage,
        }
        /// Response-side message: unlike the request struct, `content` may be
        /// null (reasoning models), and the usable text may live in `reasoning`.
        #[derive(Deserialize)]
        struct ResponseMessage {
            #[serde(default)]
            content: Option<String>,
            #[serde(default)]
            reasoning: Option<String>,
        }
        #[derive(Deserialize)]
        struct ErrorEnvelope {
            error: ErrorBody,
        }
        #[derive(Deserialize)]
        struct ErrorBody {
            #[serde(default)]
            message: String,
            #[serde(default)]
            code: Option<serde_json::Value>,
        }

        // serde tolerates leading whitespace, but trim anyway so the snippet in
        // the undecodable-body error starts at the content, not the padding.
        let trimmed = text.trim_start();

        if let Ok(ok) = serde_json::from_str::<OpenAIResponse>(trimmed) {
            if let Some(choice) = ok.choices.first() {
                let content = choice.message.content.as_deref().unwrap_or("");
                if !content.trim().is_empty() {
                    return Ok(content.to_string());
                }
                // Reasoning models sometimes put everything in `reasoning` and
                // leave `content` empty; a summary built from the reasoning is
                // better than reporting an empty success.
                if let Some(reasoning) = &choice.message.reasoning {
                    if !reasoning.trim().is_empty() {
                        return Ok(reasoning.clone());
                    }
                }
            }
            return Err(LlmError::from_api_response(
                status,
                "completion succeeded but contained no text".to_string(),
            ));
        }

        if let Ok(err) = serde_json::from_str::<ErrorEnvelope>(trimmed) {
            // An error body inside a 200: classify by the code the ENVELOPE
            // carries, not the transport status. OpenRouter wraps its 429s in
            // an HTTP 200, and classifying that as status 200 turns a rate
            // limit into a generic failure — which the failover walk then
            // retries at full speed, the exact opposite of what a rate limit
            // asks for.
            let embedded_code = err.error.code.as_ref().and_then(|c| {
                c.as_u64()
                    .and_then(|v| u16::try_from(v).ok())
                    .or_else(|| c.as_str().and_then(|s| s.parse().ok()))
            });
            let code_note = err
                .error
                .code
                .as_ref()
                .map(|c| format!(" (code {c})"))
                .unwrap_or_default();
            return Err(LlmError::from_api_response(
                embedded_code.unwrap_or(status),
                format!("{}{code_note}", err.error.message),
            ));
        }

        let snippet: String = trimmed.chars().take(300).collect();
        Err(LlmError::from_api_response(
            status,
            format!("undecodable completion body: {snippet:?}"),
        ))
    }

    async fn complete_ollama(&self, request: &CompletionRequest) -> Result<String, LlmError> {
        pace_provider_requests(self.provider).await;
        // Ollama uses a slightly different format from OpenAI
        #[derive(Serialize)]
        struct OllamaMessage<'a> {
            role: &'a str,
            content: &'a str,
        }

        #[derive(Serialize)]
        struct OllamaRequest<'a> {
            model: &'a str,
            messages: Vec<OllamaMessage<'a>>,
            stream: bool,
            #[serde(skip_serializing_if = "Option::is_none")]
            options: Option<OllamaOptions>,
        }

        #[derive(Serialize)]
        struct OllamaOptions {
            #[serde(skip_serializing_if = "Option::is_none")]
            temperature: Option<f32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            num_predict: Option<u32>,
            /// Context window size - limits KV cache to reduce VRAM usage
            #[serde(skip_serializing_if = "Option::is_none")]
            num_ctx: Option<u32>,
        }

        #[derive(Deserialize)]
        struct OllamaResponse {
            message: OllamaResponseMessage,
        }

        #[derive(Deserialize)]
        struct OllamaResponseMessage {
            content: String,
        }

        let messages: Vec<OllamaMessage> = request
            .messages
            .iter()
            .map(|m| OllamaMessage {
                role: match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                },
                content: &m.content,
            })
            .collect();

        let options = if request.temperature.is_some() || request.max_tokens.is_some() || request.context_limit.is_some() {
            Some(OllamaOptions {
                temperature: request.temperature,
                num_predict: request.max_tokens,
                num_ctx: request.context_limit,
            })
        } else {
            None
        };

        let body = OllamaRequest {
            model: &request.model,
            messages,
            stream: false,
            options,
        };

        // Same single-slot discipline as the streaming path: a non-streaming
        // completion still occupies the server's one generation slot, and
        // firing it into a live stream cancels that stream.
        let _slot = ollama_generation_slot(&self.base_url).acquire_owned().await;

        let response = self.apply_ollama_auth(self
            .http
            .post(format!("{}/api/chat", self.base_url))
            .header("content-type", "application/json"))
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::from_api_response(status, message));
        }

        let result: OllamaResponse = response.json().await?;
        Ok(result.message.content)
    }
}

// ============================================================================
// Token Estimation
// ============================================================================

/// Per-message framing overhead (role markers + delimiters), in tokens.
///
/// Both Anthropic and `OpenAI` wrap each message in a few control tokens
/// (`<|im_start|>role … <|im_end|>` and equivalents). Ignoring it under-counts
/// long conversations by several hundred tokens.
pub const MESSAGE_FRAMING_TOKENS: usize = 4;

/// Approximate char/token ratios used by the heuristic estimator.
///
/// These replace the old blanket `len()/4`:
/// - English/prose ASCII: ~4 chars/token
/// - Source code / JSON / identifiers: denser ~3.5 chars/token
/// - CJK / wide scripts: ~1 token per character
pub const CHARS_PER_TOKEN_ENGLISH: f32 = 4.0;
pub const CHARS_PER_TOKEN_CODE: f32 = 3.5;
/// Wide-script (CJK etc.) density, tokens per character (not chars/token).
pub const TOKENS_PER_WIDE_CHAR: f32 = 1.0;

/// Content family used to pick a char/token multiplier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenContentFamily {
    /// Natural-language English / Latin prose.
    English,
    /// Source code, JSON, XML, identifiers, punctuation-heavy text.
    Code,
    /// Auto-detect from content (default).
    Auto,
}

/// Estimate token count for a string using the family-aware heuristic.
///
/// Character-class aware: ASCII/Latin text tokenizes at roughly 4 chars per
/// token (English) or ~3.5 for code-like content, while CJK and other non-ASCII
/// scripts are far denser. The old `bytes / 4` heuristic badly under-counted CJK
/// — a single CJK char is 3 UTF-8 bytes yet ~1 token — which caused real context
/// overflows. Auto mode blends English/code ratios based on a simple
/// punctuation/identifier density scan.
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    estimate_tokens_for_family(text, TokenContentFamily::Auto)
}

/// Estimate tokens with an explicit content family.
#[must_use]
pub fn estimate_tokens_for_family(text: &str, family: TokenContentFamily) -> usize {
    if text.is_empty() {
        return 0;
    }

    let mut ascii_chars = 0usize;
    let mut wide_chars = 0usize;
    let mut code_markers = 0usize;

    for ch in text.chars() {
        if ch.is_ascii() {
            ascii_chars += 1;
            // Common code/json/identifier density signals.
            if matches!(
                ch,
                '{' | '}' | '[' | ']' | '(' | ')' | ';' | ':' | '_' | '<' | '>' | '=' | '|'
                    | '&' | '*' | '/' | '\\' | '#' | '@' | '$' | '`'
            ) {
                code_markers += 1;
            }
        } else {
            wide_chars += 1;
        }
    }

    let ratio = match family {
        TokenContentFamily::English => CHARS_PER_TOKEN_ENGLISH,
        TokenContentFamily::Code => CHARS_PER_TOKEN_CODE,
        TokenContentFamily::Auto => {
            // If ≥8% of ASCII chars are code-ish punctuation, treat as code.
            if ascii_chars > 0 && (code_markers.saturating_mul(100) / ascii_chars) >= 8 {
                CHARS_PER_TOKEN_CODE
            } else {
                CHARS_PER_TOKEN_ENGLISH
            }
        }
    };

    // Round the ASCII share up so short strings never estimate to zero.
    let ascii_tokens = if ascii_chars == 0 {
        0
    } else {
        // ceil(ascii_chars / ratio) without float hacks in the hot edge cases.
        ((ascii_chars as f32) / ratio).ceil() as usize
    };
    let wide_tokens = (wide_chars as f32 * TOKENS_PER_WIDE_CHAR).ceil() as usize;
    // At least 1 token for non-empty input.
    (ascii_tokens + wide_tokens).max(1)
}

/// Exact OpenAI BPE tokenization via `tiktoken-rs` when the `tiktoken` feature
/// is enabled. Falls back to the heuristic when tokenization fails or the
/// feature is off.
///
/// - prefers `o200k_base` (GPT-4o / GPT-5 / o-series) because it is the modern
///   default encoding;
/// - falls back to `cl100k_base` (GPT-4 / GPT-3.5) if o200k cannot init.
#[must_use]
pub fn estimate_tokens_exact(text: &str) -> usize {
    #[cfg(feature = "tiktoken")]
    {
        if let Some(n) = tiktoken_count(text) {
            return n;
        }
    }
    estimate_tokens(text)
}

/// Exact count with an explicit model hint (e.g. `"gpt-4o"`, `"gpt-4"`,
/// `"o3-mini"`). When the feature is off, or the model is unknown, falls back
/// to [`estimate_tokens`].
#[must_use]
pub fn estimate_tokens_for_model(text: &str, model: &str) -> usize {
    #[cfg(feature = "tiktoken")]
    {
        if let Some(n) = tiktoken_count_for_model(text, model) {
            return n;
        }
    }
    let _ = model;
    estimate_tokens(text)
}

#[cfg(feature = "tiktoken")]
fn tiktoken_count(text: &str) -> Option<usize> {
    // Prefer modern o200k (GPT-4o / GPT-5 / o*), then cl100k.
    if let Ok(bpe) = tiktoken_rs::o200k_base() {
        return Some(bpe.encode_with_special_tokens(text).len());
    }
    if let Ok(bpe) = tiktoken_rs::cl100k_base() {
        return Some(bpe.encode_with_special_tokens(text).len());
    }
    None
}

#[cfg(feature = "tiktoken")]
fn tiktoken_count_for_model(text: &str, model: &str) -> Option<usize> {
    // bpe_for_model is the preferred API in 0.12; falls through on unknown names.
    if let Ok(bpe) = tiktoken_rs::bpe_for_model(model) {
        return Some(bpe.encode_with_special_tokens(text).len());
    }
    // Heuristic family pick from the model name string.
    let lower = model.to_ascii_lowercase();
    let o200k = lower.contains("gpt-4o")
        || lower.contains("gpt-5")
        || lower.contains("gpt-4.1")
        || lower.starts_with('o')
        || lower.contains("codex")
        || lower.contains("chatgpt");
    if o200k {
        if let Ok(bpe) = tiktoken_rs::o200k_base() {
            return Some(bpe.encode_with_special_tokens(text).len());
        }
    }
    if let Ok(bpe) = tiktoken_rs::cl100k_base() {
        return Some(bpe.encode_with_special_tokens(text).len());
    }
    None
}

/// Estimate total tokens for a completion request
#[must_use]
pub fn estimate_request_tokens(request: &CompletionRequest) -> usize {
    let mut total = 0;

    // Simple messages (each carries role + delimiter framing).
    for msg in &request.messages {
        total += estimate_tokens(&msg.content) + MESSAGE_FRAMING_TOKENS;
    }

    // Anthropic messages (can have multiple content blocks)
    for msg in &request.anthropic_messages {
        total += MESSAGE_FRAMING_TOKENS;
        for block in &msg.content {
            match block {
                ContentBlock::Text { text } => total += estimate_tokens(text),
                ContentBlock::ToolUse { input, .. } => {
                    // Tool inputs are code/JSON-shaped.
                    total += estimate_tokens_for_family(
                        &input.to_string(),
                        TokenContentFamily::Code,
                    );
                }
                ContentBlock::ToolResult { content, .. } => {
                    total += estimate_tokens(content);
                }
                ContentBlock::Image { .. } => {
                    // Images are ~1000 tokens for small, more for large
                    total += 1000;
                }
                ContentBlock::Thinking { thinking, .. } => {
                    total += estimate_tokens(thinking);
                }
            }
        }
    }

    // Tools definitions
    for tool in &request.tools {
        total += estimate_tokens_for_family(&tool.to_string(), TokenContentFamily::Code);
    }

    total
}

/// Model rate limit info (tokens per minute)
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ModelLimits {
    pub input_tokens_per_minute: u32,
    pub output_tokens_per_minute: u32,
    /// Requests per minute (if known)
    pub requests_per_minute: Option<u32>,
    /// Retry-after seconds from last 429 (if any)
    pub retry_after_secs: Option<u64>,
}

impl Default for ModelLimits {
    fn default() -> Self {
        Self {
            input_tokens_per_minute: 20_000,
            output_tokens_per_minute: 8_000,
            requests_per_minute: None,
            retry_after_secs: None,
        }
    }
}

/// Rate limit info parsed from API response headers
#[derive(Debug, Clone)]
pub struct RateLimitHeaders {
    /// Tokens per minute limit
    pub limit_tokens: Option<u32>,
    /// Remaining tokens this minute
    pub remaining_tokens: Option<u32>,
    /// Requests per minute limit
    pub limit_requests: Option<u32>,
    /// Remaining requests this minute
    pub remaining_requests: Option<u32>,
    /// Seconds until limit resets
    pub reset_tokens_secs: Option<u64>,
    pub reset_requests_secs: Option<u64>,
    /// Retry-after (from 429 response)
    pub retry_after: Option<u64>,
}

impl RateLimitHeaders {
    /// Parse rate limit headers from an HTTP response.
    /// Handles both Anthropic and OpenAI header formats.
    #[must_use]
    pub fn from_headers(headers: &reqwest::header::HeaderMap) -> Self {
        let get_u32 = |name: &str| -> Option<u32> {
            headers.get(name)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse().ok())
        };
        let get_u64 = |name: &str| -> Option<u64> {
            headers.get(name)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse().ok())
        };

        Self {
            // Anthropic uses x-ratelimit-limit-tokens, x-ratelimit-remaining-tokens
            // OpenAI uses x-ratelimit-limit-tokens, x-ratelimit-remaining-tokens (same!)
            limit_tokens: get_u32("x-ratelimit-limit-tokens"),
            remaining_tokens: get_u32("x-ratelimit-remaining-tokens"),
            limit_requests: get_u32("x-ratelimit-limit-requests"),
            remaining_requests: get_u32("x-ratelimit-remaining-requests"),
            reset_tokens_secs: get_u64("x-ratelimit-reset-tokens"),
            reset_requests_secs: get_u64("x-ratelimit-reset-requests"),
            retry_after: get_u64("retry-after"),
        }
    }

    /// Convert to ModelLimits, using defaults where headers are missing
    #[must_use]
    pub fn to_model_limits(&self, defaults: &ModelLimits) -> ModelLimits {
        ModelLimits {
            input_tokens_per_minute: self.limit_tokens.unwrap_or(defaults.input_tokens_per_minute),
            output_tokens_per_minute: defaults.output_tokens_per_minute, // Usually not in headers
            requests_per_minute: self.limit_requests.or(defaults.requests_per_minute),
            retry_after_secs: self.retry_after.or(defaults.retry_after_secs),
        }
    }
}

impl ModelLimits {
    /// Get default limits for common models.
    /// These are conservative fallbacks — actual limits come from API headers.
    ///
    /// **Note:** These are baseline Tier 1 limits. Higher tiers get much more.
    /// The system will update these from response headers at runtime.
    #[must_use]
    pub fn for_model(model: &str) -> Self {
        match model {
            // Anthropic - Tier 1 baseline (very conservative)
            // See https://docs.anthropic.com/en/api/rate-limits
            m if m.contains("opus") => Self {
                input_tokens_per_minute: 20_000,   // Tier 1 baseline
                output_tokens_per_minute: 4_000,
                requests_per_minute: Some(50),
                retry_after_secs: None,
            },
            m if m.contains("sonnet") => Self {
                input_tokens_per_minute: 40_000,   // Tier 1 baseline
                output_tokens_per_minute: 8_000,
                requests_per_minute: Some(50),
                retry_after_secs: None,
            },
            m if m.contains("haiku") => Self {
                input_tokens_per_minute: 50_000,   // Tier 1 baseline
                output_tokens_per_minute: 10_000,
                requests_per_minute: Some(50),
                retry_after_secs: None,
            },
            // OpenAI - Tier 1 baseline
            m if m.contains("gpt-4o") => Self {
                input_tokens_per_minute: 30_000,
                output_tokens_per_minute: 10_000,
                requests_per_minute: Some(500),
                retry_after_secs: None,
            },
            m if m.contains("gpt-4") => Self {
                input_tokens_per_minute: 10_000,
                output_tokens_per_minute: 10_000,
                requests_per_minute: Some(500),
                retry_after_secs: None,
            },
            m if m.contains("gpt-3.5") => Self {
                input_tokens_per_minute: 200_000,
                output_tokens_per_minute: 50_000,
                requests_per_minute: Some(3500),
                retry_after_secs: None,
            },
            // Ollama - unlimited (local)
            m if m.contains("ollama") || m.contains("llama") || m.contains("mistral") || m.contains("qwen") => Self {
                input_tokens_per_minute: u32::MAX,
                output_tokens_per_minute: u32::MAX,
                requests_per_minute: None,
                retry_after_secs: None,
            },
            // Default conservative estimate
            _ => Self::default(),
        }
    }

    /// Check if a request would likely exceed rate limits
    #[must_use]
    pub fn would_exceed(&self, estimated_input_tokens: usize) -> bool {
        estimated_input_tokens as u32 > self.input_tokens_per_minute
    }

    /// Update limits from API response headers
    pub fn update_from_headers(&mut self, headers: &RateLimitHeaders) {
        if let Some(limit) = headers.limit_tokens {
            self.input_tokens_per_minute = limit;
        }
        if let Some(requests) = headers.limit_requests {
            self.requests_per_minute = Some(requests);
        }
        if let Some(retry) = headers.retry_after {
            self.retry_after_secs = Some(retry);
        }
    }
}

/// Helper trait for building requests fluently.
pub trait RequestBuilder {
    /// Set the model.
    #[must_use]
    fn with_model(self, model: impl Into<String>) -> Self;
    /// Add a message.
    #[must_use]
    fn with_message(self, message: Message) -> Self;
    /// Set max tokens.
    #[must_use]
    fn with_max_tokens(self, max_tokens: u32) -> Self;
    /// Set temperature.
    #[must_use]
    fn with_temperature(self, temperature: f32) -> Self;
}

impl RequestBuilder for CompletionRequest {
    fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    fn with_message(mut self, message: Message) -> Self {
        self.messages.push(message);
        self
    }

    fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }
}

impl CompletionRequest {
    /// Set context window limit (for Ollama num_ctx).
    /// This reduces VRAM usage by limiting the KV cache size,
    /// allowing more model layers to fit on GPU.
    #[must_use]
    pub fn with_context_limit(mut self, limit: u32) -> Self {
        self.context_limit = Some(limit);
        self
    }
}

// ============================================================================
// Embeddings Support
// ============================================================================

/// Embedding provider type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingProvider {
    OpenAI,
    Ollama,
}

/// Embedding client for generating vector embeddings
#[derive(Clone)]
pub struct EmbeddingClient {
    http: Client,
    provider: EmbeddingProvider,
    api_key: String,
    base_url: String,
    model: String,
}

impl EmbeddingClient {
    /// Create `OpenAI` embedding client
    pub fn openai(api_key: impl Into<String>) -> Self {
        Self {
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| Client::new()),
            provider: EmbeddingProvider::OpenAI,
            api_key: api_key.into(),
            base_url: "https://api.openai.com".to_string(),
            model: "text-embedding-3-small".to_string(),
        }
    }

    /// Create `Ollama` embedding client (local, no API key needed)
    pub fn ollama(base_url: impl Into<String>) -> Self {
        Self {
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(60)) // Ollama can be slower
                .build()
                .unwrap_or_else(|_| Client::new()),
            provider: EmbeddingProvider::Ollama,
            api_key: String::new(),
            base_url: base_url.into(),
            model: "nomic-embed-text".to_string(), // Good default for Ollama
        }
    }

    /// Create Ollama embedding client with default localhost URL
    pub fn ollama_default() -> Self {
        Self::ollama("http://localhost:11434")
    }

    /// Create client with custom model.
    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Override the base URL (e.g. for OpenRouter-compatible embedding endpoints).
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Get embeddings for a batch of texts.
    ///
    /// # Errors
    ///
    /// Returns `LlmError::Api` if the API returns an error.
    /// Returns `LlmError::Network` if the request fails.
    pub async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, LlmError> {
        match self.provider {
            EmbeddingProvider::OpenAI => self.embed_openai(texts).await,
            EmbeddingProvider::Ollama => self.embed_ollama(texts).await,
        }
    }

    async fn embed_openai(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, LlmError> {
        #[derive(Serialize)]
        struct EmbedRequest<'a> {
            model: &'a str,
            input: &'a [&'a str],
        }

        #[derive(Deserialize)]
        struct EmbedResponse {
            data: Vec<EmbedData>,
        }

        #[derive(Deserialize)]
        struct EmbedData {
            embedding: Vec<f32>,
        }

        let response = self
            .http
            .post(format!("{}/v1/embeddings", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&EmbedRequest {
                model: &self.model,
                input: texts,
            })
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::from_api_response(status, message));
        }

        // Heal malformed embedding JSON bodies (some proxies/wrappers garble arrays).
        let raw = response.text().await?;
        let result: EmbedResponse = match serde_json::from_str(&raw) {
            Ok(r) => r,
            Err(e) => {
                let healed = heal::heal_json(&raw)
                    .ok_or_else(|| LlmError::Json(format!("embedding response: {e}")))?;
                serde_json::from_value(healed)
                    .map_err(|e2| LlmError::Json(format!("embedding response after heal: {e2}")))?
            }
        };
        Ok(result.data.into_iter().map(|d| d.embedding).collect())
    }

    async fn embed_ollama(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, LlmError> {
        let mut results = Vec::with_capacity(texts.len());

        for text in texts {
            let embedding = self.embed_ollama_one(text).await?;
            if embedding.is_empty() {
                return Err(LlmError::Api {
                    status: 500,
                    message: format!(
                        "Ollama returned empty embedding for model '{}'. Is the model pulled?",
                        self.model
                    ),
                });
            }
            results.push(embedding);
        }

        Ok(results)
    }

    /// Embed a single text via Ollama, trying new API then legacy.
    async fn embed_ollama_one(&self, text: &str) -> Result<Vec<f32>, LlmError> {
        #[derive(Serialize)]
        struct NewReq<'a> { model: &'a str, input: &'a str }
        #[derive(Serialize)]
        struct LegacyReq<'a> { model: &'a str, prompt: &'a str }
        #[derive(Deserialize)]
        struct NewResp { #[serde(default)] embeddings: Vec<Vec<f32>> }
        #[derive(Deserialize)]
        struct LegacyResp { #[serde(default)] embedding: Vec<f32> }

        // Try new API: POST /api/embed { model, input }
        let mut req = self
            .http
            .post(format!("{}/api/embed", self.base_url))
            .header("Content-Type", "application/json");
        if !self.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key));
        }
        let response = req
            .json(&NewReq { model: &self.model, input: text })
            .send()
            .await?;

        if response.status().is_success() {
            let raw = response.text().await.unwrap_or_default();
            let resp: Option<NewResp> = serde_json::from_str(&raw)
                .ok()
                .or_else(|| heal::heal_json(&raw).and_then(|v| serde_json::from_value(v).ok()));
            if let Some(resp) = resp {
                if let Some(emb) = resp.embeddings.into_iter().next() {
                    if !emb.is_empty() {
                        return Ok(emb);
                    }
                }
            }
        }

        // Fall back to legacy API: POST /api/embeddings { model, prompt }
        let mut req = self
            .http
            .post(format!("{}/api/embeddings", self.base_url))
            .header("Content-Type", "application/json");
        if !self.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key));
        }
        let response = req
            .json(&LegacyReq { model: &self.model, prompt: text })
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::from_api_response(status, message));
        }

        let raw = response.text().await?;
        let result: LegacyResp = match serde_json::from_str(&raw) {
            Ok(r) => r,
            Err(e) => {
                let healed = heal::heal_json(&raw)
                    .ok_or_else(|| LlmError::Json(format!("ollama embedding: {e}")))?;
                serde_json::from_value(healed)
                    .map_err(|e2| LlmError::Json(format!("ollama embedding after heal: {e2}")))?
            }
        };
        Ok(result.embedding)
    }

    /// Get embedding for a single text.
    ///
    /// # Errors
    ///
    /// Returns `LlmError::Api` if the API returns an error or no embedding is returned.
    /// Returns `LlmError::Network` if the request fails.
    pub async fn embed_one(&self, text: &str) -> Result<Vec<f32>, LlmError> {
        let mut results = self.embed(&[text]).await?;
        results.pop().ok_or_else(|| LlmError::Api {
            status: 500,
            message: "No embedding returned".to_string(),
        })
    }
}

// ============================================================================
// Streaming Support
// ============================================================================

/// A streaming event from the Anthropic API
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Start of a new message. Anthropic reports prompt-side token usage here
    /// (including prompt-cache hits/writes); other providers report zeros.
    MessageStart {
        id: String,
        model: String,
        /// Non-cached prompt tokens billed for this message.
        input_tokens: u32,
        /// Prompt tokens served from the cache (a read, cheaper).
        cache_read_tokens: u32,
        /// Prompt tokens written into the cache (a creation, one-time cost).
        cache_creation_tokens: u32,
    },
    /// Start of a content block
    ContentBlockStart {
        index: usize,
        content_type: String,
        /// Tool use ID (only for tool_use blocks)
        tool_id: Option<String>,
        /// Tool name (only for tool_use blocks)
        tool_name: Option<String>,
    },
    /// Text delta within a content block
    TextDelta { index: usize, text: String },
    /// Thinking/reasoning delta (extended thinking mode)
    ThinkingDelta { index: usize, thinking: String },
    /// Signature delta for a thinking block (must be accumulated and sent back to Anthropic)
    SignatureDelta { index: usize, signature: String },
    /// Tool use delta (JSON fragment)
    ToolUseDelta { index: usize, partial_json: String },
    /// Content block finished
    ContentBlockStop { index: usize },
    /// Message finished
    MessageStop { stop_reason: String },
    /// Usage statistics
    MessageDelta {
        stop_reason: Option<String>,
        output_tokens: u32,
    },
    /// Ping (keepalive)
    Ping,
    /// Error event (non-recoverable)
    Error { message: String },
    /// Recoverable error mid-stream (e.g., 429 rate limit).
    /// Contains accumulated partial content that can be used to continue.
    RecoverableError {
        error: LlmError,
        /// Text accumulated before the error
        partial_text: String,
        /// In-progress tool calls (id, name, partial_json)
        partial_tool_calls: Vec<(String, String, String)>,
    },
    /// Rate limit headers received (allows updating limits cache)
    RateLimitInfo {
        limit_tokens: Option<u32>,
        remaining_tokens: Option<u32>,
        reset_secs: Option<u64>,
    },
}

/// Accumulated state during streaming (for recovery)
#[derive(Debug, Clone, Default)]
pub struct StreamAccumulator {
    /// Text content accumulated so far
    pub text: String,
    /// Thinking/reasoning content accumulated so far
    pub thinking: String,
    /// Accumulated signature for the thinking block (required by Anthropic in multi-turn)
    pub thinking_signature: String,
    /// Tool calls in progress: (index, id, name, partial_json)
    pub tool_calls: Vec<(usize, String, String, String)>,
}

impl StreamAccumulator {
    /// Create a new empty accumulator
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Process an event and accumulate content
    pub fn process(&mut self, event: &StreamEvent) {
        match event {
            StreamEvent::TextDelta { text, .. } => {
                self.text.push_str(text);
            }
            StreamEvent::ThinkingDelta { thinking, .. } => {
                self.thinking.push_str(thinking);
            }
            StreamEvent::SignatureDelta { signature, .. } => {
                self.thinking_signature.push_str(signature);
            }
            StreamEvent::ContentBlockStart { index, content_type, tool_id, tool_name } => {
                if content_type == "tool_use" {
                    if let (Some(id), Some(name)) = (tool_id, tool_name) {
                        self.tool_calls.push((*index, id.clone(), name.clone(), String::new()));
                    }
                }
            }
            StreamEvent::ToolUseDelta { index, partial_json } => {
                if let Some((_, _, _, json)) = self.tool_calls.iter_mut().find(|(i, _, _, _)| i == index) {
                    json.push_str(partial_json);
                }
            }
            _ => {}
        }
    }

    /// Get partial tool calls as (id, name, json) tuples
    #[must_use]
    pub fn partial_tool_calls(&self) -> Vec<(String, String, String)> {
        self.tool_calls.iter()
            .map(|(_, id, name, json)| (id.clone(), name.clone(), json.clone()))
            .collect()
    }

    /// Check if there's any accumulated content
    #[must_use]
    pub fn has_content(&self) -> bool {
        !self.text.is_empty() || !self.thinking.is_empty() || !self.tool_calls.is_empty()
    }
}

/// Anthropic SSE event types
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicSSE {
    MessageStart { message: MessageStartData },
    ContentBlockStart { index: usize, content_block: ContentBlockData },
    ContentBlockDelta { index: usize, delta: DeltaData },
    ContentBlockStop { index: usize },
    MessageDelta { delta: MessageDeltaData, usage: Option<UsageDelta> },
    MessageStop,
    Ping,
    Error { error: ErrorData },
}

#[derive(Debug, Deserialize)]
struct MessageStartData {
    id: String,
    model: String,
    #[serde(default)]
    usage: Option<MessageStartUsage>,
}

/// Prompt-side token usage reported in the Anthropic `message_start` event.
/// Field names mirror the wire JSON keys, so the shared `_input_tokens` suffix
/// is required (not a naming smell).
#[derive(Debug, Deserialize, Default)]
#[allow(clippy::struct_field_names)]
struct MessageStartUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct ContentBlockData {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DeltaData {
    TextDelta { text: String },
    ThinkingDelta { thinking: String },
    InputJsonDelta { partial_json: String },
    /// Signature delta for thinking blocks (must be accumulated and sent back)
    SignatureDelta { signature: String },
    /// Catch-all for unknown delta types so the
    /// surrounding `ContentBlockDelta` event still deserializes successfully.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
struct MessageDeltaData {
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UsageDelta {
    output_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct ErrorData {
    message: String,
}

impl LlmClient {
    /// Stream an Anthropic-format request, dispatching to the appropriate provider.
    ///
    /// - Anthropic: Native SSE streaming via `/v1/messages`
    /// - OpenAI/OpenRouter/GitHub/ClaudeProxy: Converts to OpenAI SSE streaming
    /// - Ollama: Converts to Ollama streaming via `/api/chat`
    pub fn stream_anthropic(
        &self,
        request: &AnthropicRequest,
    ) -> impl Stream<Item = Result<StreamEvent, LlmError>> + '_ {
        let provider = self.provider;
        let request = request.clone();

        stream! {
            // One gate for every streamed request regardless of which backend
            // serves it — a stream spends the same quota as a completion.
            pace_provider_requests(provider).await;
            match provider {
                Provider::Anthropic => {
                    let raw = self.stream_anthropic_native(&request);
                    tokio::pin!(raw);
                    while let Some(item) = raw.next().await {
                        yield item;
                    }
                }
                Provider::OpenAI | Provider::OpenRouter | Provider::GitHubModels | Provider::ClaudeProxy => {
                    let raw = self.stream_anthropic_via_openai(&request);
                    tokio::pin!(raw);
                    while let Some(item) = raw.next().await {
                        yield item;
                    }
                }
                Provider::Ollama => {
                    let raw = self.stream_anthropic_via_ollama(&request);
                    tokio::pin!(raw);
                    while let Some(item) = raw.next().await {
                        yield item;
                    }
                }
            }
        }
    }

    /// Native Anthropic SSE streaming implementation
    fn stream_anthropic_native(
        &self,
        request: &AnthropicRequest,
    ) -> impl Stream<Item = Result<StreamEvent, LlmError>> + '_ {
        let mut request = request.clone();
        request.stream = Some(true);

        let http = self.http.clone();
        let api_key = self.api_key.clone();
        let base_url = self.base_url.clone();
        let is_oauth = is_oauth_token(&api_key);

        // Store original tools for reverse mapping
        let original_tools = request.tools.clone().unwrap_or_default();

        // Prepare OAuth request with array-based system prompt
        let oauth_request = if is_oauth {
            info!("Claude Code stealth mode: applying OAuth headers and array-based system prompt");

            let mut oauth_req = OAuthAnthropicRequest::from_request(&request, true);
            oauth_req.stream = Some(true);

            // Remap tool names to Claude Code format
            if let Some(ref mut tools) = oauth_req.tools {
                for tool in tools {
                    let old_name = tool.name.clone();
                    tool.name = to_claude_code_tool_name(&tool.name);
                    if old_name != tool.name {
                        debug!(old = %old_name, new = %tool.name, "Remapped tool name");
                    }
                }
            }

            debug!(
                system_blocks = oauth_req.system.len(),
                tool_names = ?oauth_req.tools.as_ref().map(|t| t.iter().map(|x| x.name.as_str()).collect::<Vec<_>>()),
                "OAuth streaming request prepared with array-based system prompt"
            );

            Some(oauth_req)
        } else {
            None
        };

        stream! {
            let request_builder = http
                .post(format!("{base_url}/v1/messages"));

            let request_builder = if is_oauth {
                let token = get_raw_token(&api_key);
                request_builder
                    .header("Authorization", format!("Bearer {token}"))
                    .header("accept", "application/json")
                    .header("anthropic-beta", "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14")
                    .header("user-agent", format!("claude-cli/{CLAUDE_CODE_VERSION} (external, cli)"))
                    .header("x-app", "cli")
                    .header("anthropic-dangerous-direct-browser-access", "true")
            } else {
                request_builder.header("x-api-key", &api_key)
            };

            debug!(
                is_oauth = is_oauth,
                model = %request.model,
                has_system = request.system.is_some(),
                tools_count = request.tools.as_ref().map(|t| t.len()).unwrap_or(0),
                "Sending Anthropic streaming request"
            );

            let response = if let Some(ref oauth_req) = oauth_request {
                request_builder
                    .header("anthropic-version", "2023-06-01")
                    .header("content-type", "application/json")
                    .json(oauth_req)
                    .send()
                    .await
            } else {
                request_builder
                    .header("anthropic-version", "2023-06-01")
                    .header("content-type", "application/json")
                    .json(&request)
                    .send()
                    .await
            };

            let response = match response {
                Ok(r) => {
                    debug!(status = %r.status(), "Anthropic API response received");
                    r
                },
                Err(e) => {
                    warn!(error = %e, "Anthropic API request failed");
                    yield Err(LlmError::Http(e.to_string()));
                    return;
                }
            };

            let rate_headers = RateLimitHeaders::from_headers(response.headers());
            if rate_headers.limit_tokens.is_some() || rate_headers.remaining_tokens.is_some() {
                yield Ok(StreamEvent::RateLimitInfo {
                    limit_tokens: rate_headers.limit_tokens,
                    remaining_tokens: rate_headers.remaining_tokens,
                    reset_secs: rate_headers.reset_tokens_secs,
                });
            }

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let message = response.text().await.unwrap_or_default();
                yield Err(LlmError::from_api_response(status, message));
                return;
            }

            let mut byte_stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut accumulator = StreamAccumulator::new();

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        let error = LlmError::Http(e.to_string());
                        if accumulator.has_content() {
                            yield Ok(StreamEvent::RecoverableError {
                                error,
                                partial_text: accumulator.text.clone(),
                                partial_tool_calls: accumulator.partial_tool_calls(),
                            });
                        } else {
                            yield Err(error);
                        }
                        return;
                    }
                };

                match std::str::from_utf8(&chunk) {
                    Ok(s) => buffer.push_str(s),
                    Err(e) => {
                        let error = LlmError::Stream(format!("Invalid UTF-8 in stream: {e}"));
                        if accumulator.has_content() {
                            yield Ok(StreamEvent::RecoverableError {
                                error,
                                partial_text: accumulator.text.clone(),
                                partial_tool_calls: accumulator.partial_tool_calls(),
                            });
                        } else {
                            yield Err(error);
                        }
                        return;
                    }
                }

                while let Some(event_str) = extract_sse_event(&mut buffer) {
                    if let Some(mut stream_event) = parse_sse_event(&event_str) {
                        if is_oauth {
                            if let StreamEvent::ContentBlockStart { tool_name: Some(ref mut name), .. } = stream_event {
                                *name = from_claude_code_tool_name(name, &original_tools);
                            }
                        }
                        accumulator.process(&stream_event);
                        yield Ok(stream_event);
                    }
                }
            }

            if !buffer.trim().is_empty() {
                if let Some(mut stream_event) = parse_sse_event(&buffer) {
                    if is_oauth {
                        if let StreamEvent::ContentBlockStart { tool_name: Some(ref mut name), .. } = stream_event {
                            *name = from_claude_code_tool_name(name, &original_tools);
                        }
                    }
                    accumulator.process(&stream_event);
                    yield Ok(stream_event);
                }
            }
        }
    }

    /// Stream an AnthropicRequest via OpenAI-compatible SSE streaming.
    /// Converts the request to OpenAI format, streams via /v1/chat/completions,
    /// and converts events back to Anthropic StreamEvent format.
    fn stream_anthropic_via_openai(
        &self,
        request: &AnthropicRequest,
    ) -> impl Stream<Item = Result<StreamEvent, LlmError>> + '_ {
        let http = self.http.clone();
        let provider = self.provider;
        let api_key = self.api_key.clone();
        let base_url = self.base_url.clone();
        let request = request.clone();

        stream! {
            let (messages_json, tools_json) = anthropic_to_openai_request(&request);

            let mut body = serde_json::json!({
                "model": request.model,
                "messages": messages_json,
                "max_tokens": request.max_tokens,
                "stream": true,
            });
            if let Some(temp) = request.temperature {
                body["temperature"] = serde_json::json!(temp);
            }
            if let Some(tools) = tools_json {
                body["tools"] = tools;
            }

            let response = match http
                .post(format!("{base_url}/v1/chat/completions"))
                .header("Authorization", format!("Bearer {api_key}"))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    yield Err(LlmError::Http(e.to_string()));
                    return;
                }
            };
            learn_rate_limit_headers(provider, response.headers());

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let message = response.text().await.unwrap_or_default();
                yield Err(LlmError::from_api_response(status, message));
                return;
            }

            // Reuse OpenAI SSE parsing logic (same format as stream_openai)
            let mut byte_stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut text_block_started = false;
            let mut tool_calls_started: std::collections::HashMap<usize, (String, String)> = std::collections::HashMap::new();

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(LlmError::Stream(e.to_string()));
                        return;
                    }
                };

                match std::str::from_utf8(&chunk) {
                    Ok(s) => buffer.push_str(s),
                    Err(e) => {
                        yield Err(LlmError::Stream(format!("Invalid UTF-8: {e}")));
                        return;
                    }
                }

                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim().to_string();
                    buffer = buffer[line_end + 1..].to_string();

                    if line.is_empty() || !line.starts_with("data: ") {
                        continue;
                    }

                    let data = &line[6..];

                    if data == "[DONE]" {
                        if text_block_started {
                            yield Ok(StreamEvent::ContentBlockStop { index: 0 });
                        }
                        let mut stop_indices: Vec<usize> = tool_calls_started.keys().copied().collect();
                        stop_indices.sort_unstable();
                        for idx in stop_indices {
                            yield Ok(StreamEvent::ContentBlockStop { index: idx });
                        }
                        yield Ok(StreamEvent::MessageStop { stop_reason: "end_turn".to_string() });
                        return;
                    }

                    #[derive(Deserialize, Debug)]
                    struct StreamChunk {
                        choices: Vec<ChunkChoice>,
                    }
                    #[derive(Deserialize, Debug)]
                    struct ChunkChoice {
                        delta: Option<ChunkDelta>,
                        finish_reason: Option<String>,
                    }
                    #[derive(Deserialize, Debug)]
                    struct ChunkDelta {
                        content: Option<String>,
                        tool_calls: Option<Vec<ToolCallDelta>>,
                    }
                    #[derive(Deserialize, Debug)]
                    struct ToolCallDelta {
                        index: usize,
                        id: Option<String>,
                        function: Option<FunctionDelta>,
                    }
                    #[derive(Deserialize, Debug)]
                    struct FunctionDelta {
                        name: Option<String>,
                        arguments: Option<String>,
                    }

                    if let Ok(chunk) = serde_json::from_str::<StreamChunk>(data) {
                        for choice in chunk.choices {
                            if let Some(delta) = choice.delta {
                                if let Some(text) = delta.content {
                                    if !text.is_empty() {
                                        if !text_block_started {
                                            text_block_started = true;
                                            yield Ok(StreamEvent::ContentBlockStart {
                                                index: 0,
                                                content_type: "text".to_string(),
                                                tool_id: None,
                                                tool_name: None,
                                            });
                                        }
                                        yield Ok(StreamEvent::TextDelta { index: 0, text });
                                    }
                                }

                                if let Some(tool_calls) = delta.tool_calls {
                                    for tc in tool_calls {
                                        let block_index = tc.index + 1;
                                        if let (Some(id), Some(func)) = (&tc.id, &tc.function) {
                                            if let Some(name) = &func.name {
                                                if text_block_started {
                                                    yield Ok(StreamEvent::ContentBlockStop { index: 0 });
                                                    text_block_started = false;
                                                }
                                                tool_calls_started.insert(block_index, (id.clone(), name.clone()));
                                                yield Ok(StreamEvent::ContentBlockStart {
                                                    index: block_index,
                                                    content_type: "tool_use".to_string(),
                                                    tool_id: Some(id.clone()),
                                                    tool_name: Some(name.clone()),
                                                });
                                            }
                                        }
                                        if let Some(func) = &tc.function {
                                            if let Some(args) = &func.arguments {
                                                if !args.is_empty() {
                                                    yield Ok(StreamEvent::ToolUseDelta {
                                                        index: block_index,
                                                        partial_json: args.clone(),
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            if let Some(reason) = choice.finish_reason {
                                if text_block_started {
                                    yield Ok(StreamEvent::ContentBlockStop { index: 0 });
                                }
                                let mut stop_indices: Vec<usize> = tool_calls_started.keys().copied().collect();
                                stop_indices.sort_unstable();
                                for idx in stop_indices {
                                    yield Ok(StreamEvent::ContentBlockStop { index: idx });
                                }
                                let stop_reason = match reason.as_str() {
                                    "tool_calls" => "tool_use",
                                    "stop" => "end_turn",
                                    other => other,
                                };
                                yield Ok(StreamEvent::MessageStop { stop_reason: stop_reason.to_string() });
                                return;
                            }
                        }
                    }
                }
            }

            // Emit close events if stream ended without finish_reason
            if text_block_started {
                yield Ok(StreamEvent::ContentBlockStop { index: 0 });
            }
            let mut stop_indices: Vec<usize> = tool_calls_started.keys().copied().collect();
            stop_indices.sort_unstable();
            for idx in stop_indices {
                yield Ok(StreamEvent::ContentBlockStop { index: idx });
            }
            yield Ok(StreamEvent::MessageStop { stop_reason: "end_turn".to_string() });
        }
    }

    /// Stream an AnthropicRequest via Ollama `/api/chat` with `stream: true`.
    /// Ollama streams NDJSON (one JSON object per line, not SSE).
    fn stream_anthropic_via_ollama(
        &self,
        request: &AnthropicRequest,
    ) -> impl Stream<Item = Result<StreamEvent, LlmError>> + '_ {
        let http = self.http.clone();
        let base_url = self.base_url.clone();
        let api_key = self.api_key.clone();
        let request = request.clone();

        stream! {
            // Hold the server's single generation slot for the WHOLE stream.
            // The permit lives as long as this generator, so it is released
            // when the stream finishes, errors, or is dropped mid-flight.
            // Acquire before building the body: the wait is the point.
            let _slot = ollama_generation_slot(&base_url).acquire_owned().await;

            let (messages_json, tools_json) = anthropic_to_ollama_request(&request);

            let mut body = serde_json::json!({
                "model": request.model,
                "messages": messages_json,
                "stream": true,
            });

            let mut options = serde_json::Map::new();
            if let Some(temp) = request.temperature {
                options.insert("temperature".to_string(), serde_json::json!(temp));
            } else {
                // See the non-streaming site: Modelfile thinking defaults run
                // hot (1.0); 0.6 is Qwen's non-thinking recommendation and
                // measurably reduces one-line code cramming.
                options.insert("temperature".to_string(), serde_json::json!(0.6));
            }
            // Bound generation to the request's token budget. num_predict=-1
            // (unlimited) let a degraded runner emit degenerate tokens without
            // end; the caller's max_tokens already accounts for thinking room.
            options.insert("num_predict".to_string(), serde_json::json!(request.max_tokens));
            // Size the context to the GPU as it is RIGHT NOW, every turn — see
            // `resolve_num_ctx`. This is the path the agent loop takes, so a
            // constant here is the one that actually decides whether a run
            // survives: 32k was unrunnable for a 12B model on a 16 GB card
            // sharing with the desktop, and it failed as
            // `CUDA error: an illegal memory access was encountered` rather
            // than as a clean allocation failure.
            let num_ctx = Self::resolve_num_ctx(&request);
            tracing::info!(
                model = %request.model,
                num_ctx,
                "sized ollama context to free VRAM (stream)"
            );
            options.insert("num_ctx".to_string(), serde_json::json!(num_ctx));
            // qwen3.5's Modelfile ships presence_penalty=1.5 (Qwen's thinking-
            // mode anti-repetition default). Code REQUIRES repetition, and at
            // 1.5 the model visibly degraded working multi-function files into
            // semicolon one-liner slop (observed live, round-5 drive). 1.0
            // keeps a mild guard; the loop's repetition/spiral detectors
            // cover the rest.
            options.insert("presence_penalty".to_string(), serde_json::json!(1.0));
        // repeat_penalty 1.1 (the Ollama default) punishes exactly the tokens
        // Python repeats by design — newlines and indentation runs — and the
        // model visibly compensates by cramming statements onto one line
        // (observed live across rounds even at presence 1.0). Code presets
        // use 1.0: no repetition penalty at all.
        options.insert("repeat_penalty".to_string(), serde_json::json!(1.0));
            if !options.is_empty() {
                body["options"] = serde_json::Value::Object(options);
            }
            if let Some(tools) = tools_json {
                body["tools"] = tools;
            }

            let mut req = http
                .post(format!("{base_url}/api/chat"))
                .header("content-type", "application/json");
            if !api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {api_key}"));
            }
            let response = match req
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    yield Err(LlmError::Http(e.to_string()));
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let message = response.text().await.unwrap_or_default();
                yield Err(LlmError::from_api_response(status, message));
                return;
            }

            // Ollama streams NDJSON: one JSON per line
            let mut byte_stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut bytes_received = 0usize;
            let mut lines_parsed = 0usize;
            let mut thinking_block_started = false;
            let mut saw_stop_sentinel = false;
            let mut text_block_started = false;
            let mut tool_block_count = 0usize;
            // Ollama thinking uses block index 0 (same as Anthropic);
            // text starts at index 1 when thinking is present, or 0 otherwise.
            let mut next_block_index = 0usize;
            // Degenerate-repetition watch (see RepeatWatch).
            let mut repeat_watch = RepeatWatch::default();
            let mut last_line: Option<String> = None;

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(LlmError::Stream(e.to_string()));
                        return;
                    }
                };

                bytes_received += chunk.len();
                match std::str::from_utf8(&chunk) {
                    Ok(s) => buffer.push_str(s),
                    Err(e) => {
                        yield Err(LlmError::Stream(format!("Invalid UTF-8: {e}")));
                        return;
                    }
                }

                // Process complete lines (NDJSON)
                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim().to_string();
                    buffer = buffer[line_end + 1..].to_string();

                    if line.is_empty() {
                        continue;
                    }

                    let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) else {
                        continue;
                    };
                    lines_parsed += 1;
                    // Keep the last line so an abort can show what the model
                    // was actually emitting when the body stopped. Without it
                    // a truncation is indistinguishable from a wedged decoder,
                    // which is exactly the ambiguity that has cost two runs.
                    last_line = Some(line.clone());

                    // Check for done
                    let done = obj["done"].as_bool().unwrap_or(false);

                    // Stream thinking content (qwen3 and other thinking models)
                    // Ollama returns thinking tokens in message.thinking.
                    // Emit proper ContentBlockStart/Stop to match Anthropic's block structure.
                    // Cut a wedged runner short instead of waiting out its loop.
                    if let Some(frag) = obj["message"]["thinking"]
                        .as_str()
                        .or_else(|| obj["message"]["content"].as_str())
                    {
                        if let Some(run) = repeat_watch.observe(frag) {
                            if thinking_block_started || text_block_started {
                                yield Ok(StreamEvent::ContentBlockStop { index: next_block_index });
                            }
                            yield Err(LlmError::WedgedRunner {
                                token: frag.to_string(),
                                run,
                                bytes_received,
                            });
                            return;
                        }
                    }

                    if let Some(thinking) = obj["message"]["thinking"].as_str() {
                        if !thinking.is_empty() {
                            if !thinking_block_started {
                                thinking_block_started = true;
                                yield Ok(StreamEvent::ContentBlockStart {
                                    index: next_block_index,
                                    content_type: "thinking".to_string(),
                                    tool_id: None,
                                    tool_name: None,
                                });
                            }
                            yield Ok(StreamEvent::ThinkingDelta { index: next_block_index, thinking: thinking.to_string() });
                        }
                    }

                    // Stream text content.
                    // When text starts arriving and thinking was active, close the thinking block first.
                    if let Some(content) = obj["message"]["content"].as_str() {
                        // Gemma's reserved sentinels (`<unused0>`..`<unused98>`)
                        // are stop markers the chat template is supposed to
                        // consume. Ollama passes them through as ordinary
                        // content and then simply stops the stream, with no
                        // done=true — so the terminator we need arrives
                        // disguised as text. Note it, drop it from the visible
                        // output, and let the end-of-stream handler close the
                        // turn cleanly instead of reporting an aborted
                        // generation. Observed live 2026-07-28: 57 such 502s in
                        // one gemma4:12b run, each discarding a complete reply.
                        let content = if Self::is_gemma_stop_sentinel(content) {
                            saw_stop_sentinel = true;
                            tracing::info!("ollama: model stop sentinel seen (exact)");
                            ""
                        } else if let Some(prefix) = Self::strip_trailing_stop_sentinel(content) {
                            // The sentinel can also arrive glued to the end of
                            // real text ("...all tests pass.<unused50>"), where
                            // an exact-match test never fires. Keep the words,
                            // drop the marker, remember that the turn ended.
                            saw_stop_sentinel = true;
                            tracing::info!("ollama: model stop sentinel seen (trailing)");
                            prefix
                        } else {
                            content
                        };
                        if !content.is_empty() {
                            // Close thinking block when transitioning to text
                            if thinking_block_started {
                                yield Ok(StreamEvent::ContentBlockStop { index: next_block_index });
                                thinking_block_started = false;
                                next_block_index += 1;
                            }
                            if !text_block_started {
                                text_block_started = true;
                                yield Ok(StreamEvent::ContentBlockStart {
                                    index: next_block_index,
                                    content_type: "text".to_string(),
                                    tool_id: None,
                                    tool_name: None,
                                });
                            }
                            yield Ok(StreamEvent::TextDelta { index: next_block_index, text: content.to_string() });
                        }
                    }

                    // Tool calls: Ollama emits these in a message with done=false,
                    // followed by a separate done=true message with empty content.
                    // Process tool_calls from ANY message, not just the done message.
                    if let Some(tool_calls) = obj["message"]["tool_calls"].as_array() {
                        if !tool_calls.is_empty() {
                            // Close any open thinking/text blocks before tool calls
                            if thinking_block_started {
                                yield Ok(StreamEvent::ContentBlockStop { index: next_block_index });
                                thinking_block_started = false;
                                next_block_index += 1;
                            }
                            if text_block_started {
                                yield Ok(StreamEvent::ContentBlockStop { index: next_block_index });
                                text_block_started = false;
                                next_block_index += 1;
                            }

                            for tc in tool_calls {
                                tool_block_count += 1;
                                let block_idx = next_block_index;
                                next_block_index += 1;
                                let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                                let args = tc["function"]["arguments"].to_string();
                                let id = tc["id"].as_str()
                                    .map(String::from)
                                    .unwrap_or_else(|| format!("toolu_{:08x}", tool_block_count));
                                yield Ok(StreamEvent::ContentBlockStart {
                                    index: block_idx,
                                    content_type: "tool_use".to_string(),
                                    tool_id: Some(id),
                                    tool_name: Some(name),
                                });
                                yield Ok(StreamEvent::ToolUseDelta {
                                    index: block_idx,
                                    partial_json: args,
                                });
                                yield Ok(StreamEvent::ContentBlockStop { index: block_idx });
                            }
                        }
                    }

                    if done {
                        // Close any remaining open blocks
                        if thinking_block_started {
                            yield Ok(StreamEvent::ContentBlockStop { index: next_block_index });
                            next_block_index += 1;
                        }
                        if text_block_started {
                            yield Ok(StreamEvent::ContentBlockStop { index: next_block_index });
                            // next_block_index not needed after this
                        }

                        // Emit usage. Ollama reports BOTH sides only in the
                        // final done message: prompt_eval_count (input) and
                        // eval_count (output). The prompt side rides a late
                        // MessageStart — the stream consumer records it by
                        // assignment wherever it appears, and Ollama streams
                        // never emit an earlier one. Omitting it zeroed
                        // every per-request input count downstream (run
                        // benchmarks and the context-usage indicator).
                        if let Some(prompt_eval) = obj["prompt_eval_count"].as_u64() {
                            yield Ok(StreamEvent::MessageStart {
                                id: String::new(),
                                model: obj["model"].as_str().unwrap_or("").to_string(),
                                input_tokens: prompt_eval as u32,
                                cache_read_tokens: 0,
                                cache_creation_tokens: 0,
                            });
                        }
                        // Emit output token count
                        if let Some(eval_count) = obj["eval_count"].as_u64() {
                            yield Ok(StreamEvent::MessageDelta {
                                stop_reason: None,
                                output_tokens: eval_count as u32,
                            });
                        }

                        let stop_reason = if tool_block_count > 0 { "tool_use" } else { "end_turn" };
                        yield Ok(StreamEvent::MessageStop { stop_reason: stop_reason.to_string() });
                        return;
                    }
                }
            }

            // A final NDJSON object that arrived without its trailing newline
            // would otherwise sit in `buffer` unparsed and be reported as an
            // aborted generation. Cheap to honour, and it can only ever turn
            // a false 502 into the success it actually was.
            if let Ok(obj) = serde_json::from_str::<serde_json::Value>(buffer.trim()) {
                if obj["done"].as_bool().unwrap_or(false) {
                    if thinking_block_started || text_block_started {
                        yield Ok(StreamEvent::ContentBlockStop { index: next_block_index });
                    }
                    let stop_reason = if tool_block_count > 0 { "tool_use" } else { "end_turn" };
                    yield Ok(StreamEvent::MessageStop { stop_reason: stop_reason.to_string() });
                    return;
                }
            }

            // A sentinel-terminated stream is a COMPLETE turn.
            //
            // The check below exists for genuinely aborted generations, and it
            // must stay — a degraded runner streams degenerate tokens and drops
            // the connection, and delivering that as a successful reply is how
            // garbage reached the GUI. But gemma ends its turn by EMITTING a
            // reserved `<unusedNN>` token and then closing, which is a normal
            // stop wearing an abort's clothing. The content before it is a
            // finished reply; discarding it cost one run 57 complete responses
            // and ultimately the whole benchmark.
            if saw_stop_sentinel {
                if thinking_block_started {
                    yield Ok(StreamEvent::ContentBlockStop { index: next_block_index });
                }
                if text_block_started {
                    yield Ok(StreamEvent::ContentBlockStop { index: next_block_index });
                }
                let stop_reason = if tool_block_count > 0 { "tool_use" } else { "end_turn" };
                tracing::info!(
                    bytes_received,
                    lines_parsed,
                    "ollama stream closed on a model stop sentinel rather than done=true"
                );
                yield Ok(StreamEvent::MessageStop { stop_reason: stop_reason.to_string() });
                return;
            }

            // The stream ended with no done=true terminator: an ABORTED
            // generation. A degraded Ollama runner streams degenerate tokens
            // and drops the stream with no final message (observed live) —
            // closing "gracefully" here delivered that garbage to the GUI as
            // a successful reply. Surface a retryable server error instead,
            // mirroring the non-streaming done=false check.
            if thinking_block_started {
                yield Ok(StreamEvent::ContentBlockStop { index: next_block_index });
            }
            if text_block_started {
                yield Ok(StreamEvent::ContentBlockStop { index: next_block_index });
            }
            // Say what was left unparsed. A bare byte/line count cannot
            // distinguish "the provider hung up mid-token" from "we dropped a
            // terminator", and 68 of these in one night taught us nothing
            // about which. A bounded tail of the residual buffer makes the
            // next occurrence self-diagnosing.
            let tail_note = match &last_line {
                Some(line) => {
                    let shown: String = line.chars().take(200).collect();
                    format!(" Last line before the cut: {shown}")
                }
                None => " No NDJSON line was ever parsed.".to_string(),
            };
            let residue = buffer.trim();
            let residue_note = if residue.is_empty() {
                format!("nothing was left unparsed (the stream simply stopped).{tail_note}")
            } else {
                let tail: String = residue.chars().rev().take(160).collect::<Vec<_>>()
                    .into_iter().rev().collect();
                format!("{} unparsed bytes remained, ending: {tail}", residue.len())
            };
            yield Err(LlmError::from_api_response(
                502,
                format!(
                    "Ollama stream ended without completion (no done=true) after \
                     {bytes_received} bytes / {lines_parsed} NDJSON lines — {residue_note}"
                ),
            ));
        }
    }

    /// Stream using OpenAI-compatible API (for OpenAI, OpenRouter, GitHub Models, Claude Proxy)
    /// Supports both text responses and tool calls.
    pub fn stream_openai(
        &self,
        request: &CompletionRequest,
    ) -> impl Stream<Item = Result<StreamEvent, LlmError>> + '_ {
        let http = self.http.clone();
        let api_key = self.api_key.clone();
        let base_url = self.base_url.clone();
        let request = request.clone();
        let provider = self.provider;

        stream! {
            // Same pacing gate as the non-streaming paths — a streamed chat
            // request spends the same quota as any other.
            pace_provider_requests(provider).await;
            // === Request types ===
            #[derive(Serialize)]
            struct OpenAIMessage {
                role: String,
                content: String,
            }

            #[derive(Serialize)]
            struct OpenAITool {
                #[serde(rename = "type")]
                tool_type: String,
                function: OpenAIFunction,
            }

            #[derive(Serialize)]
            struct OpenAIFunction {
                name: String,
                description: String,
                parameters: serde_json::Value,
            }

            #[derive(Serialize)]
            struct OpenAIRequest {
                model: String,
                messages: Vec<OpenAIMessage>,
                #[serde(skip_serializing_if = "Option::is_none")]
                max_tokens: Option<u32>,
                #[serde(skip_serializing_if = "Option::is_none")]
                temperature: Option<f32>,
                stream: bool,
                #[serde(skip_serializing_if = "Option::is_none")]
                tools: Option<Vec<OpenAITool>>,
            }

            // === Response types ===
            #[derive(Deserialize, Debug)]
            struct StreamChunk {
                choices: Vec<ChunkChoice>,
            }

            #[derive(Deserialize, Debug)]
            struct ChunkChoice {
                delta: Option<ChunkDelta>,
                finish_reason: Option<String>,
            }

            #[derive(Deserialize, Debug)]
            struct ChunkDelta {
                content: Option<String>,
    #[serde(rename = "role")]
                _role: Option<String>,
                tool_calls: Option<Vec<ToolCallDelta>>,
            }

            #[derive(Deserialize, Debug)]
            struct ToolCallDelta {
                index: usize,
                id: Option<String>,
                function: Option<FunctionDelta>,
            }

            #[derive(Deserialize, Debug)]
            struct FunctionDelta {
                name: Option<String>,
                arguments: Option<String>,
            }

            // Convert messages to OpenAI format
            let messages: Vec<OpenAIMessage> = request
                .messages
                .iter()
                .map(|m| OpenAIMessage {
                    role: match m.role {
                        Role::System => "system".to_string(),
                        Role::User => "user".to_string(),
                        Role::Assistant => "assistant".to_string(),
                    },
                    content: m.content.clone(),
                })
                .collect();

            // Convert tools to OpenAI format
            let tools: Option<Vec<OpenAITool>> = if request.tools.is_empty() {
                None
            } else {
                Some(request.tools.iter().filter_map(|v| {
                    // Try to parse as ToolDefinition
                    if let Ok(tool_def) = serde_json::from_value::<ToolDefinition>(v.clone()) {
                        Some(OpenAITool {
                            tool_type: "function".to_string(),
                            function: OpenAIFunction {
                                name: tool_def.name,
                                description: tool_def.description,
                                parameters: tool_def.input_schema,
                            },
                        })
                    } else {
                        None
                    }
                }).collect())
            };

            let body = OpenAIRequest {
                model: request.model.clone(),
                messages,
                max_tokens: request.max_tokens,
                temperature: request.temperature,
                stream: true,
                tools,
            };

            let response = match http
                .post(format!("{base_url}/v1/chat/completions"))
                .header("Authorization", format!("Bearer {api_key}"))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    yield Err(LlmError::Http(e.to_string()));
                    return;
                }
            };
            learn_rate_limit_headers(provider, response.headers());

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let message = response.text().await.unwrap_or_default();
                yield Err(LlmError::from_api_response(status, message));
                return;
            }

            let mut byte_stream = response.bytes_stream();
            let mut buffer = String::new();

            // Track content blocks (text at index 0, tool calls at higher indices)
            let mut text_block_started = false;
            let mut tool_calls_started: std::collections::HashMap<usize, (String, String)> = std::collections::HashMap::new(); // index -> (id, name)

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(LlmError::Stream(e.to_string()));
                        return;
                    }
                };

                match std::str::from_utf8(&chunk) {
                    Ok(s) => buffer.push_str(s),
                    Err(e) => {
                        yield Err(LlmError::Stream(format!("Invalid UTF-8: {e}")));
                        return;
                    }
                }

                // Parse SSE events from buffer
                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim().to_string();
                    buffer = buffer[line_end + 1..].to_string();

                    if line.is_empty() || !line.starts_with("data: ") {
                        continue;
                    }

                    let data = &line[6..]; // Strip "data: "

                    if data == "[DONE]" {
                        // Close any open blocks
                        if text_block_started {
                            yield Ok(StreamEvent::ContentBlockStop { index: 0 });
                        }
                        let mut stop_indices: Vec<usize> = tool_calls_started.keys().copied().collect();
                        stop_indices.sort_unstable();
                        for idx in stop_indices {
                            yield Ok(StreamEvent::ContentBlockStop { index: idx });
                        }
                        yield Ok(StreamEvent::MessageStop { stop_reason: "end_turn".to_string() });
                        return;
                    }

                    if let Ok(chunk) = serde_json::from_str::<StreamChunk>(data) {
                        for choice in chunk.choices {
                            if let Some(delta) = choice.delta {
                                // Handle text content
                                if let Some(text) = delta.content {
                                    if !text.is_empty() {
                                        if !text_block_started {
                                            text_block_started = true;
                                            yield Ok(StreamEvent::ContentBlockStart {
                                                index: 0,
                                                content_type: "text".to_string(),
                                                tool_id: None,
                                                tool_name: None,
                                            });
                                        }
                                        yield Ok(StreamEvent::TextDelta { index: 0, text });
                                    }
                                }

                                // Handle tool calls
                                if let Some(tool_calls) = delta.tool_calls {
                                    for tc in tool_calls {
                                        // Tool index offset by 1 (text is at 0)
                                        let block_index = tc.index + 1;

                                        // Check if this is a new tool call
                                        if let (Some(id), Some(func)) = (&tc.id, &tc.function) {
                                            if let Some(name) = &func.name {
                                                // Close text block if open (tool calls come after text)
                                                if text_block_started {
                                                    yield Ok(StreamEvent::ContentBlockStop { index: 0 });
                                                    text_block_started = false;
                                                }

                                                // New tool call
                                                tool_calls_started.insert(block_index, (id.clone(), name.clone()));
                                                yield Ok(StreamEvent::ContentBlockStart {
                                                    index: block_index,
                                                    content_type: "tool_use".to_string(),
                                                    tool_id: Some(id.clone()),
                                                    tool_name: Some(name.clone()),
                                                });
                                            }
                                        }

                                        // Stream tool arguments
                                        if let Some(func) = &tc.function {
                                            if let Some(args) = &func.arguments {
                                                if !args.is_empty() {
                                                    yield Ok(StreamEvent::ToolUseDelta {
                                                        index: block_index,
                                                        partial_json: args.clone(),
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // Check for stop
                            if let Some(reason) = choice.finish_reason {
                                // Close any open blocks
                                if text_block_started {
                                    yield Ok(StreamEvent::ContentBlockStop { index: 0 });
                                }
                                let mut stop_indices: Vec<usize> = tool_calls_started.keys().copied().collect();
                                stop_indices.sort_unstable();
                                for idx in stop_indices {
                                    yield Ok(StreamEvent::ContentBlockStop { index: idx });
                                }

                                let stop_reason = match reason.as_str() {
                                    "tool_calls" => "tool_use",
                                    "stop" => "end_turn",
                                    other => other,
                                };
                                yield Ok(StreamEvent::MessageStop { stop_reason: stop_reason.to_string() });
                                return;
                            }
                        }
                    }
                }
            }

            // If we got here without MessageStop, emit it
            if text_block_started {
                yield Ok(StreamEvent::ContentBlockStop { index: 0 });
            }
            let mut stop_indices: Vec<usize> = tool_calls_started.keys().copied().collect();
            stop_indices.sort_unstable();
            for idx in stop_indices {
                yield Ok(StreamEvent::ContentBlockStop { index: idx });
            }
            yield Ok(StreamEvent::MessageStop { stop_reason: "end_turn".to_string() });
        }
    }

    /// Convenience method to stream a CompletionRequest.
    /// Converts to provider-specific format automatically.
    ///
    /// Returns `StreamEvent` directly (errors converted to `Error` or `RecoverableError` events).
    pub fn stream(
        &self,
        request: &CompletionRequest,
    ) -> impl Stream<Item = StreamEvent> + '_ {
        let provider = self.provider.clone();
        let request = request.clone();

        stream! {
            // Route to appropriate streaming method based on provider
            match provider {
                Provider::Anthropic => {
                    // Convert to Anthropic format
                    let system_msg = request
                        .messages
                        .iter()
                        .find(|m| m.role == Role::System)
                        .map(|m| m.content.clone());

                    let messages: Vec<AnthropicMessage> = request
                        .messages
                        .iter()
                        .filter(|m| m.role != Role::System)
                        .map(|m| AnthropicMessage {
                            role: match m.role {
                                Role::User | Role::System => "user",
                                Role::Assistant => "assistant",
                            }.to_string(),
                            content: vec![ContentBlock::Text { text: m.content.clone() }],
                        })
                        .collect();

                    let mut all_messages = messages;
                    all_messages.extend(request.anthropic_messages.clone());

                    let tools = if request.tools.is_empty() {
                        None
                    } else {
                        Some(request.tools.iter().filter_map(|v| {
                            serde_json::from_value::<ToolDefinition>(v.clone()).ok()
                        }).collect())
                    };

                    let anthropic_request = AnthropicRequest {
                        context_limit: None,
                        model: request.model.clone(),
                        messages: all_messages,
                        max_tokens: request.max_tokens.unwrap_or(4096),
                        temperature: if request.thinking.is_some() { None } else { request.temperature },
                        system: system_msg,
                        tools,
                        stream: Some(true),
                        thinking: request.thinking.clone(),
                        cache_control: None,
                    };

                    let raw_stream = self.stream_anthropic(&anthropic_request);
                    tokio::pin!(raw_stream);

                    while let Some(result) = raw_stream.next().await {
                        match result {
                            Ok(event) => yield event,
                            Err(e) => {
                                if e.should_fallback() {
                                    yield StreamEvent::RecoverableError {
                                        error: e,
                                        partial_text: String::new(),
                                        partial_tool_calls: Vec::new(),
                                    };
                                } else {
                                    yield StreamEvent::Error { message: e.to_string() };
                                }
                                return;
                            }
                        }
                    }
                }
                Provider::OpenAI | Provider::OpenRouter | Provider::GitHubModels | Provider::ClaudeProxy => {
                    // Use OpenAI-compatible streaming
                    let raw_stream = self.stream_openai(&request);
                    tokio::pin!(raw_stream);

                    while let Some(result) = raw_stream.next().await {
                        match result {
                            Ok(event) => yield event,
                            Err(e) => {
                                if e.should_fallback() {
                                    yield StreamEvent::RecoverableError {
                                        error: e,
                                        partial_text: String::new(),
                                        partial_tool_calls: Vec::new(),
                                    };
                                } else {
                                    yield StreamEvent::Error { message: e.to_string() };
                                }
                                return;
                            }
                        }
                    }
                }
                Provider::Ollama => {
                    // Ollama streaming: build AnthropicRequest and delegate to stream_anthropic
                    // which already handles Ollama NDJSON streaming via stream_anthropic_via_ollama
                    let system_msg = request
                        .messages
                        .iter()
                        .find(|m| m.role == Role::System)
                        .map(|m| m.content.clone());

                    let messages: Vec<AnthropicMessage> = request
                        .messages
                        .iter()
                        .filter(|m| m.role != Role::System)
                        .map(|m| AnthropicMessage {
                            role: match m.role {
                                Role::User | Role::System => "user",
                                Role::Assistant => "assistant",
                            }.to_string(),
                            content: vec![ContentBlock::Text { text: m.content.clone() }],
                        })
                        .collect();

                    let mut all_messages = messages;
                    all_messages.extend(request.anthropic_messages.clone());

                    let tools = if request.tools.is_empty() {
                        None
                    } else {
                        Some(request.tools.iter().filter_map(|v| {
                            serde_json::from_value::<ToolDefinition>(v.clone()).ok()
                        }).collect())
                    };

                    let anthropic_request = AnthropicRequest {
                        context_limit: None,
                        model: request.model.clone(),
                        messages: all_messages,
                        max_tokens: request.max_tokens.unwrap_or(4096),
                        temperature: request.temperature,
                        system: system_msg,
                        tools,
                        stream: Some(true),
                        thinking: request.thinking.clone(),
                        cache_control: None,
                    };

                    let raw_stream = self.stream_anthropic(&anthropic_request);
                    tokio::pin!(raw_stream);

                    while let Some(result) = raw_stream.next().await {
                        match result {
                            Ok(event) => yield event,
                            Err(e) => {
                                if e.should_fallback() {
                                    yield StreamEvent::RecoverableError {
                                        error: e,
                                        partial_text: String::new(),
                                        partial_tool_calls: Vec::new(),
                                    };
                                } else {
                                    yield StreamEvent::Error { message: e.to_string() };
                                }
                                return;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Stream with explicit accumulator for advanced recovery scenarios.
    ///
    /// Returns both events and the final accumulator state (even on error).
    pub fn stream_with_recovery(
        &self,
        request: &CompletionRequest,
    ) -> impl Stream<Item = (StreamEvent, StreamAccumulator)> + '_ {
        let base_stream = self.stream(request);

        stream! {
            let mut accumulator = StreamAccumulator::new();
            tokio::pin!(base_stream);

            while let Some(event) = base_stream.next().await {
                accumulator.process(&event);
                yield (event, accumulator.clone());
            }
        }
    }
}

// ============================================================================
// Anthropic ↔ OpenAI/Ollama Conversion Layer
// ============================================================================

/// How to encode `tool_calls[].function.arguments` on the wire.
///
/// OpenAI-compatible endpoints (/v1/chat/completions) expect a JSON-encoded
/// STRING. Ollama's native /api/chat expects a JSON OBJECT and rejects the
/// string form at request-decode time with HTTP 400
/// `Value looks like object, but can't find closing '}' symbol` — even when
/// the string contains valid JSON (verified live against Ollama 0.31.2).
#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolArgsWire {
    /// `"arguments": "{\"path\": ...}"` — OpenAI convention
    JsonString,
    /// `"arguments": {"path": ...}` — Ollama native convention
    JsonObject,
}

/// Convert an AnthropicRequest into OpenAI-compatible messages and tools JSON.
/// Returns (messages_json_array, tools_json_array_or_none).
fn anthropic_to_openai_request(request: &AnthropicRequest) -> (serde_json::Value, Option<serde_json::Value>) {
    anthropic_to_wire_request(request, ToolArgsWire::JsonString)
}

/// Convert an AnthropicRequest for Ollama's NATIVE /api/chat endpoint.
/// Identical to the OpenAI conversion except tool-call arguments stay a JSON
/// object — the string form 400s every request that echoes tool history.
fn anthropic_to_ollama_request(request: &AnthropicRequest) -> (serde_json::Value, Option<serde_json::Value>) {
    anthropic_to_wire_request(request, ToolArgsWire::JsonObject)
}

fn anthropic_to_wire_request(
    request: &AnthropicRequest,
    args_wire: ToolArgsWire,
) -> (serde_json::Value, Option<serde_json::Value>) {
    let mut messages = Vec::new();

    // System prompt → system message
    if let Some(ref system) = request.system {
        if !system.is_empty() {
            messages.push(serde_json::json!({
                "role": "system",
                "content": system,
            }));
        }
    }

    // Convert Anthropic messages to OpenAI messages
    for msg in &request.messages {
        let role = msg.role.as_str();
        match role {
            "user" => {
                // User messages: may contain text and/or tool_result blocks
                let mut text_parts = Vec::new();
                let mut tool_results = Vec::new();

                for block in &msg.content {
                    match block {
                        ContentBlock::Text { text } => text_parts.push(text.clone()),
                        ContentBlock::ToolResult { tool_use_id, content, is_error } => {
                            tool_results.push(serde_json::json!({
                                "role": "tool",
                                "tool_call_id": tool_use_id,
                                "content": if is_error.unwrap_or(false) {
                                    format!("Error: {content}")
                                } else {
                                    content.clone()
                                },
                            }));
                        }
                        _ => {}
                    }
                }

                // Emit text parts as a user message
                if !text_parts.is_empty() {
                    messages.push(serde_json::json!({
                        "role": "user",
                        "content": text_parts.join("\n"),
                    }));
                }
                // Emit tool results as separate "tool" role messages
                messages.extend(tool_results);
            }
            "assistant" => {
                // Assistant messages: may contain text and/or tool_use blocks
                let mut text_parts = Vec::new();
                let mut tool_calls = Vec::new();

                for block in &msg.content {
                    match block {
                        ContentBlock::Text { text } => text_parts.push(text.clone()),
                        ContentBlock::ToolUse { id, name, input } => {
                            let arguments = match args_wire {
                                ToolArgsWire::JsonString => {
                                    serde_json::Value::String(input.to_string())
                                }
                                // Ollama native: pass the object through. A
                                // non-object input would 400 the whole request,
                                // so degrade it to {} rather than lose the turn.
                                ToolArgsWire::JsonObject if input.is_object() => input.clone(),
                                ToolArgsWire::JsonObject => {
                                    warn!(
                                        tool = %name,
                                        "tool_use input is not a JSON object; sending {{}} to Ollama"
                                    );
                                    serde_json::json!({})
                                }
                            };
                            tool_calls.push(serde_json::json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": arguments,
                                }
                            }));
                        }
                        _ => {}
                    }
                }

                let mut assistant_msg = serde_json::json!({ "role": "assistant" });
                // Always include `content` — some OpenAI-compatible providers
                // (e.g. Arcee AI via OpenRouter) reject messages where it's missing,
                // even when `tool_calls` is present.
                if text_parts.is_empty() {
                    assistant_msg["content"] = serde_json::Value::Null;
                } else {
                    assistant_msg["content"] = serde_json::json!(text_parts.join("\n"));
                }
                if !tool_calls.is_empty() {
                    assistant_msg["tool_calls"] = serde_json::json!(tool_calls);
                }
                messages.push(assistant_msg);
            }
            _ => {}
        }
    }

    // Convert tools to OpenAI format
    let tools_json = request.tools.as_ref().and_then(|tools| {
        if tools.is_empty() {
            None
        } else {
            let converted: Vec<serde_json::Value> = tools.iter().map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            }).collect();
            Some(serde_json::json!(converted))
        }
    });

    (serde_json::json!(messages), tools_json)
}

/// Convert an OpenAI chat completion response JSON to AnthropicResponse
fn openai_response_to_anthropic(model: &str, response: &serde_json::Value) -> Result<AnthropicResponse, LlmError> {
    let choice = response["choices"]
        .as_array()
        .and_then(|c| c.first())
        .ok_or_else(|| LlmError::Api { status: 500, message: "No choices in response".to_string() })?;

    let message = &choice["message"];
    let mut content = Vec::new();

    // Text content
    if let Some(text) = message["content"].as_str() {
        if !text.is_empty() {
            content.push(ContentBlock::Text { text: text.to_string() });
        }
    }

    // Tool calls
    if let Some(tool_calls) = message["tool_calls"].as_array() {
        for tc in tool_calls {
            let id = tc["id"].as_str().unwrap_or("").to_string();
            let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
            let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
            let input = heal::heal_tool_args(args_str);
            content.push(ContentBlock::ToolUse { id, name, input });
        }
    }

    // Map finish_reason
    let finish_reason = choice["finish_reason"].as_str().unwrap_or("end_turn");
    let stop_reason = match finish_reason {
        "tool_calls" => "tool_use",
        "stop" => "end_turn",
        "length" => "max_tokens",
        other => other,
    };

    let input_tokens = response["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32;
    let output_tokens = response["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32;
    // OpenAI returns cached tokens in usage.prompt_tokens_details.cached_tokens
    let cache_read_input_tokens = response["usage"]["prompt_tokens_details"]["cached_tokens"].as_u64().unwrap_or(0) as u32;

    Ok(AnthropicResponse {
        id: response["id"].as_str().unwrap_or("").to_string(),
        response_type: "message".to_string(),
        role: "assistant".to_string(),
        content,
        model: model.to_string(),
        stop_reason: Some(stop_reason.to_string()),
        usage: Usage { input_tokens, output_tokens, cache_creation_input_tokens: 0, cache_read_input_tokens },
    })
}

/// Convert an Ollama /api/chat response JSON to AnthropicResponse
/// Extract content inside `<think>...</think>` tags.
fn extract_think_content(text: &str) -> Option<String> {
    let start = text.find("<think>")?;
    let end = text.find("</think>")?;
    if end > start + 7 {
        Some(text[start + 7..end].trim().to_string())
    } else {
        None
    }
}

/// Strip all `<think>...</think>` blocks from text, returning the remaining content.
fn strip_think_tags(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;
    while let Some(start) = remaining.find("<think>") {
        result.push_str(&remaining[..start]);
        if let Some(end) = remaining[start..].find("</think>") {
            remaining = &remaining[start + end + 8..];
        } else {
            // Unclosed <think> tag — discard the rest (model is still thinking)
            break;
        }
    }
    result.push_str(remaining);
    result.trim().to_string()
}

fn ollama_response_to_anthropic(model: &str, response: &serde_json::Value) -> Result<AnthropicResponse, LlmError> {
    // A non-streaming response with done=false is an ABORTED generation, not
    // a completion (observed live: a degraded Ollama runner emits ~31 tokens
    // of degenerate output and gives up, with no eval stats and no
    // done_reason). Parsing it as an empty success poisons every layer above;
    // surface it as a retryable server error instead.
    if response.get("done").and_then(serde_json::Value::as_bool) == Some(false) {
        return Err(LlmError::from_api_response(
            502,
            "Ollama aborted generation mid-response (done=false)".to_string(),
        ));
    }
    let message = &response["message"];
    let mut content = Vec::new();

    // Thinking content (qwen3 and other thinking models — Ollama separates this
    // into a `thinking` field when `think: true` is set in the request)
    if let Some(thinking) = message["thinking"].as_str() {
        if !thinking.is_empty() {
            content.push(ContentBlock::Thinking { thinking: thinking.to_string(), signature: None });
        }
    }

    // Text content — strip embedded <think>...</think> tags (fallback for when
    // Ollama doesn't separate thinking, e.g. older versions or models that embed tags)
    if let Some(raw_text) = message["content"].as_str() {
        // Extract thinking from tags if we didn't get it from the dedicated field
        if !content.iter().any(|b| matches!(b, ContentBlock::Thinking { .. })) {
            if let Some(thinking) = extract_think_content(raw_text) {
                if !thinking.is_empty() {
                    content.push(ContentBlock::Thinking { thinking, signature: None });
                }
            }
        }
        let text = strip_think_tags(raw_text);
        if !text.is_empty() {
            content.push(ContentBlock::Text { text });
        }
    }

    // Tool calls (Ollama v0.5+ format)
    if let Some(tool_calls) = message["tool_calls"].as_array() {
        for (i, tc) in tool_calls.iter().enumerate() {
            let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
            let input = tc["function"]["arguments"].clone();
            // Ollama doesn't provide tool call IDs, generate one
            let id = format!("toolu_{:08x}", i);
            content.push(ContentBlock::ToolUse { id, name, input });
        }
    }

    // Determine stop reason
    let done_reason = response["done_reason"].as_str().unwrap_or("stop");
    let stop_reason = if content.iter().any(|b| matches!(b, ContentBlock::ToolUse { .. })) {
        "tool_use"
    } else {
        match done_reason {
            "stop" => "end_turn",
            "length" => "max_tokens",
            other => other,
        }
    };

    // Ollama provides eval_count, prompt_eval_count
    let input_tokens = response["prompt_eval_count"].as_u64().unwrap_or(0) as u32;
    let output_tokens = response["eval_count"].as_u64().unwrap_or(0) as u32;

    Ok(AnthropicResponse {
        id: format!("ollama-{}", current_timestamp()),
        response_type: "message".to_string(),
        role: "assistant".to_string(),
        content,
        model: model.to_string(),
        stop_reason: Some(stop_reason.to_string()),
        usage: Usage { input_tokens, output_tokens, cache_creation_input_tokens: 0, cache_read_input_tokens: 0 },
    })
}

/// Extract a complete SSE event from the buffer.
fn extract_sse_event(buffer: &mut String) -> Option<String> {
    // SSE events are separated by double newlines
    buffer.find("\n\n").map(|pos| {
        let event = buffer[..pos].to_string();
        *buffer = buffer[pos + 2..].to_string();
        event
    })
}

/// Parse an SSE event string into a `StreamEvent`
fn parse_sse_event(event: &str) -> Option<StreamEvent> {
    // Extract data field from SSE event (we determine event type from JSON structure)
    let data = event
        .lines()
        .find_map(|line| line.strip_prefix("data: ").map(str::trim))?;

    // Parse the JSON data (skip malformed events)
    let sse = match serde_json::from_str::<AnthropicSSE>(data) {
        Ok(sse) => sse,
        Err(e) => {
            let end = data.floor_char_boundary(data.len().min(120));
            debug!(error = %e, data_prefix = &data[..end], "Skipping unrecognised SSE event");
            return None;
        }
    };
    match sse {
        AnthropicSSE::MessageStart { message } => {
            let usage = message.usage.unwrap_or_default();
            Some(StreamEvent::MessageStart {
                id: message.id,
                model: message.model,
                input_tokens: usage.input_tokens,
                cache_read_tokens: usage.cache_read_input_tokens,
                cache_creation_tokens: usage.cache_creation_input_tokens,
            })
        }
        AnthropicSSE::ContentBlockStart { index, content_block } => {
            Some(StreamEvent::ContentBlockStart {
                index,
                content_type: content_block.block_type,
                tool_id: content_block.id,
                tool_name: content_block.name,
            })
        }
        AnthropicSSE::ContentBlockDelta { index, delta } => match delta {
            DeltaData::TextDelta { text } => Some(StreamEvent::TextDelta { index, text }),
            DeltaData::ThinkingDelta { thinking } => Some(StreamEvent::ThinkingDelta { index, thinking }),
            DeltaData::InputJsonDelta { partial_json } => {
                Some(StreamEvent::ToolUseDelta { index, partial_json })
            }
            DeltaData::SignatureDelta { signature } => {
                Some(StreamEvent::SignatureDelta { index, signature })
            }
            DeltaData::Unknown => {
                debug!(index, "Ignoring unknown content_block_delta type");
                None
            }
        },
        AnthropicSSE::ContentBlockStop { index } => Some(StreamEvent::ContentBlockStop { index }),
        AnthropicSSE::MessageDelta { delta, usage } => Some(StreamEvent::MessageDelta {
            stop_reason: delta.stop_reason,
            output_tokens: usage.map_or(0, |u| u.output_tokens),
        }),
        AnthropicSSE::MessageStop => Some(StreamEvent::MessageStop {
            stop_reason: "end_turn".to_string(),
        }),
        AnthropicSSE::Ping => Some(StreamEvent::Ping),
        AnthropicSSE::Error { error } => Some(StreamEvent::Error {
            message: error.message,
        }),
    }
}

#[cfg(test)]
mod tests {

    // -----------------------------------------------------------------
    // Context sizing
    // -----------------------------------------------------------------

    const GB: f64 = 1024.0 * 1024.0 * 1024.0;

    /// The two runs we actually have evidence from, on the same 16 GB card
    /// with ~11.8 GB free at rest:
    ///
    ///   qwen3.5:9b  (6.59 GB) ran 4h30m clean at 32768.
    ///   gemma4:12b  (7.56 GB) died at 32768 with
    ///               `CUDA error: an illegal memory access was encountered`,
    ///               three times in three minutes.
    ///
    /// A sizing rule that hands 32768 to both is the rule we already know is
    /// broken, so it is the thing worth pinning: the ~1 GB of extra weights
    /// has to be enough to move gemma down a bucket.
    /// Measured on the bench machine 2026-07-28: 16 GB card, ~5.1 GB held by
    /// the desktop and the GUI's webview, leaving ~10.45 GB actually available
    /// to the runner.
    const FREE_UNLOADED: f64 = 10.45 * GB;
    const EMBEDDER_VRAM: f64 = 0.32 * GB;

    #[test]
    fn sizing_separates_the_model_that_ran_from_the_one_that_crashed() {
        let qwen =
            LlmClient::fit_context_for_budget(FREE_UNLOADED, 6.59 * GB, 0.0, EMBEDDER_VRAM);
        let gemma =
            LlmClient::fit_context_for_budget(FREE_UNLOADED, 7.56 * GB, 0.0, EMBEDDER_VRAM);

        assert_eq!(qwen, 32_768, "qwen3.5:9b demonstrably ran 4h30m at 32k");
        assert!(
            gemma < 32_768,
            "gemma4:12b crashed at 32k three times; sizing must not return it, got {gemma}"
        );
        assert!(
            gemma >= 16_384,
            "gemma still needs a workable window, got {gemma}"
        );
    }

    /// The bug this function shipped with once: sizing changed the moment the
    /// model finished loading, and since Ollama keys a resident instance on its
    /// options, that change evicts and reloads it — which restores the memory
    /// that justified the change.
    #[test]
    fn sizing_does_not_change_when_the_model_finishes_loading() {
        let weights = 7.56 * GB;
        let before =
            LlmClient::fit_context_for_budget(FREE_UNLOADED, weights, 0.0, EMBEDDER_VRAM);
        // Once resident it holds weights + KV, and free VRAM drops by that much.
        let held = 8.06 * GB;
        let after = LlmClient::fit_context_for_budget(
            FREE_UNLOADED - held,
            weights,
            held,
            EMBEDDER_VRAM,
        );
        assert_eq!(
            before, after,
            "loading the model must not resize its own context"
        );
    }

    #[test]
    fn sizes_are_always_buckets() {
        for free in [2.0, 6.0, 9.0, 10.45, 14.0, 40.0] {
            let got =
                LlmClient::fit_context_for_budget(free * GB, 7.56 * GB, 0.0, EMBEDDER_VRAM);
            assert!(
                [4_096, 8_192, 16_384, 32_768].contains(&got),
                "free={free}GB gave a non-bucket size {got}"
            );
        }
    }

    /// Stability across turns comes from the latch, not from bucketing — a
    /// budget that lands near a bucket edge WILL cross it on ordinary drift.
    /// Since a changed `num_ctx` makes Ollama evict and reload the model, the
    /// size has to be decided once and then only ever walked down deliberately.
    #[test]
    fn a_latched_size_survives_re_measurement_and_only_walks_down() {
        let model = "test-latch-model:1b";
        LlmClient::latch_num_ctx(model, 32_768);
        assert_eq!(LlmClient::latched_num_ctx(model), Some(32_768));

        // Demotion steps 3/4 at a time on the 512 quantum — the full default
        // ladder, documented end to end: geometric (bounded rungs) but never
        // the 50% cliff that skips past viable windows.
        for expected in [24_576, 18_432, 13_824, 10_240, 7_680, 5_632] {
            assert_eq!(LlmClient::demote_context(model, None), Some(expected));
        }
        // The last rung clamps AT the default minimum viable window (4608 —
        // the quantized pressure-tier floor), not below it...
        assert_eq!(LlmClient::demote_context(model, None), Some(4_608));
        // ...and once there, a repeat fault does NOT demote further: the
        // loud fault path stands rather than a manufactured unusable window.
        assert_eq!(LlmClient::demote_context(model, None), None);
        assert_eq!(LlmClient::latched_num_ctx(model), Some(4_608));
    }

    /// The rung arithmetic itself: 3/4 of the current latch, rounded DOWN to
    /// the 512 quantum, strictly decreasing on every step.
    #[test]
    fn demotion_steps_to_three_quarters_on_the_quantum() {
        let model = "test-three-quarter-step:1b";
        LlmClient::latch_num_ctx(model, 8_192);
        // Floor far below: pure stepping is visible. 8192·¾ = 6144 (on the
        // quantum), 6144·¾ = 4608 (on the quantum), 4608·¾ = 3456 → 3072
        // (rounded DOWN to the quantum), 3072·¾ = 2304 → 2048.
        let floor = Some(2_048);
        assert_eq!(LlmClient::demote_context(model, floor), Some(6_144));
        assert_eq!(LlmClient::demote_context(model, floor), Some(4_608));
        assert_eq!(LlmClient::demote_context(model, floor), Some(3_072));
        assert_eq!(LlmClient::demote_context(model, floor), Some(2_048));
        assert_eq!(LlmClient::demote_context(model, floor), None);
        for size in [6_144u32, 4_608, 3_072, 2_048] {
            assert_eq!(size % NUM_CTX_QUANTUM, 0, "every rung sits on the quantum");
        }
    }

    /// The live 2026-08-03 failure, fixed: ONE fault at a pinned 8192 must
    /// not land below the agent's floor. With the floor supplied, the rung
    /// that would undershoot clamps TO the floor's quantum instead.
    #[test]
    fn demotion_clamps_to_the_floor_quantum_instead_of_skipping_past_it() {
        let model = "test-floor-clamp:1b";
        LlmClient::latch_num_ctx(model, 8_192);
        // An agent floor of 4300 tokens (≈ the live pressure-tier floor)
        // quantizes UP to 4608. 8192·¾ = 6144 clears it...
        assert_eq!(LlmClient::demote_context(model, Some(4_300)), Some(6_144));
        // ...but 6144·¾ = 4608 lands exactly on it, and the next rung
        // (4608·¾ = 3456) would undershoot — so it clamps and then stops.
        assert_eq!(LlmClient::demote_context(model, Some(4_300)), Some(4_608));
        assert_eq!(LlmClient::demote_context(model, Some(4_300)), None);
        assert_eq!(LlmClient::latched_num_ctx(model), Some(4_608));

        // A rung strictly between two quanta clamps UP, never down: from
        // 8192 with a 5000-token floor (quantum 5120), ¾ gives 6144, then
        // 4608 < 5120 → the demotion lands AT 5120.
        let model2 = "test-floor-clamp-up:1b";
        LlmClient::latch_num_ctx(model2, 8_192);
        assert_eq!(LlmClient::demote_context(model2, Some(5_000)), Some(6_144));
        assert_eq!(LlmClient::demote_context(model2, Some(5_000)), Some(5_120));
        assert_eq!(LlmClient::demote_context(model2, Some(5_000)), None);
    }

    /// No demotion may EVER produce a window below the supplied floor — even
    /// when the latch already sits under it (a floor that grew mid-run, e.g.
    /// a fatter step frame). The latch is left alone and the fault surfaces.
    #[test]
    fn no_demotion_ever_lands_below_the_floor() {
        let model = "test-never-below-floor:1b";
        LlmClient::latch_num_ctx(model, 4_096);
        assert_eq!(
            LlmClient::demote_context(model, Some(8_192)),
            None,
            "a latch already below the floor must not be walked further"
        );
        assert_eq!(
            LlmClient::latched_num_ctx(model),
            Some(4_096),
            "refusing to demote must not touch the latch"
        );
        // And with no floor supplied, the conservative default holds the line.
        LlmClient::latch_num_ctx(model, DEFAULT_MIN_VIABLE_NUM_CTX);
        assert_eq!(LlmClient::demote_context(model, None), None);
        assert_eq!(
            LlmClient::latched_num_ctx(model),
            Some(DEFAULT_MIN_VIABLE_NUM_CTX)
        );
    }

    /// Every demotion decision announces itself — old → new, the floor it
    /// respected, and whether it clamped — and the refusal at the floor is
    /// just as loud, so an operator can read the ladder off the logs.
    #[test]
    fn a_demotion_announces_old_new_floor_and_clamp() {
        let model = "test-demotion-announces:1b";
        LlmClient::latch_num_ctx(model, 6_144);
        let out = capture_info_logs(|| {
            assert_eq!(LlmClient::demote_context(model, Some(5_000)), Some(5_120));
        });
        assert!(out.contains("demoting context"), "the decision: {out}");
        assert!(out.contains("test-demotion-announces:1b"), "the model: {out}");
        assert!(out.contains("6144"), "old size: {out}");
        assert!(out.contains("5120"), "new size AND the floor quantum: {out}");
        assert!(out.contains("clamped=true"), "clamped-or-not: {out}");

        let refused = capture_info_logs(|| {
            assert_eq!(LlmClient::demote_context(model, Some(5_000)), None);
        });
        assert!(
            refused.contains("NOT demoting further"),
            "the refusal at the floor must be announced too: {refused}"
        );
    }

    /// The routed name and the request name are different spellings of one
    /// model; if they key different latch entries, demotion corrects nothing.
    #[test]
    fn the_latch_ignores_the_provider_prefix() {
        LlmClient::latch_num_ctx("ollama/test-prefix-model:1b", 16_384);
        assert_eq!(
            LlmClient::latched_num_ctx("test-prefix-model:1b"),
            Some(16_384),
            "demotion keyed on the routed name must reach the request path"
        );
        assert_eq!(
            LlmClient::demote_context("ollama/test-prefix-model:1b", None),
            Some(12_288)
        );
        assert_eq!(LlmClient::latched_num_ctx("test-prefix-model:1b"), Some(12_288));
    }

    /// The demotion must be OBSERVABLE, not just latched: a model that was
    /// never sized passes the provider claim through, and after a demotion
    /// the effective window is the latch, not the claim. This getter is what
    /// the agent loop polls to re-derive its budgets mid-run — the 2026-08-02
    /// failure was exactly this value existing but being unreadable.
    #[test]
    fn a_demotion_is_visible_through_the_effective_window() {
        let model = "test-effective-window-model:9b";
        // Never sized: the provider claim stands, for local and cloud alike.
        assert_eq!(LlmClient::effective_num_ctx(model), None);
        assert_eq!(effective_context_window(model, 32_000), 32_000);

        // Sized at 16384, then demoted: the effective window follows the
        // latch, and a claim SMALLER than the latch is never inflated.
        LlmClient::latch_num_ctx(model, 16_384);
        assert_eq!(effective_context_window(model, 32_000), 16_384);
        assert_eq!(LlmClient::demote_context(model, None), Some(12_288));
        assert_eq!(LlmClient::effective_num_ctx(model), Some(12_288));
        assert_eq!(effective_context_window(model, 32_000), 12_288);
        assert_eq!(
            effective_context_window(model, 4_000),
            4_000,
            "the latch is a ceiling, not a floor"
        );
    }

    /// Model info served to budget derivation must carry the LIVE window:
    /// context clamps to the latch and max_output re-bounds to half of it, so
    /// hard_input_limit_for / effective_output_budget stay consistent with
    /// what the runner will actually honour.
    #[test]
    fn model_info_clamps_to_the_demoted_window() {
        let model = "test-clamped-info-model:9b";
        let claim = ModelInfo {
            id: model.to_string(),
            context_window: 16_384,
            max_output_tokens: 8_192,
            supports_tools: true,
            supports_vision: false,
            embedding_dimension: None,
            cached_at: current_timestamp(),
            provider: "ollama".to_string(),
        };

        // Unlatched: the claim passes through untouched.
        let untouched = clamp_model_info_to_effective_window(model, claim.clone());
        assert_eq!(untouched.context_window, 16_384);
        assert_eq!(untouched.max_output_tokens, 8_192);

        LlmClient::latch_num_ctx(model, 4_096);
        let clamped = clamp_model_info_to_effective_window(model, claim);
        assert_eq!(clamped.context_window, 4_096);
        assert_eq!(
            clamped.max_output_tokens, 2_048,
            "output cap mirrors the Ollama half-window derivation"
        );
        // The derived budgets never over-commit the demoted window.
        let reserve = clamped.effective_output_budget(4_096);
        assert!(clamped.hard_input_limit_for(reserve) + reserve <= 4_096);
    }

    /// The regression this guards: the VRAM heuristic used to run FIRST and the
    /// env var was only consulted when measuring failed, so an operator's
    /// deliberate 16384 was silently promoted to a computed 32768. Same model,
    /// same mission, 31/42 became 8/42. (The override is a ceiling — it beats
    /// any LARGER measurement; min semantics are covered separately below.)
    ///
    /// Serialised against the other env-touching tests via `env_lock` because
    /// the environment is process-global.
    #[test]
    fn an_explicit_override_beats_the_vram_heuristic() {
        let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: edition 2024 requires this; the lock above keeps the two
        // env-touching tests from overlapping, and nothing else reads this key.
        unsafe { std::env::set_var("NANNA_OLLAMA_NUM_CTX", "16384") };

        let mut request = request_with_tool_history(serde_json::json!({}));
        request.model = "test-env-precedence:1b".to_string();
        request.context_limit = None;

        assert_eq!(
            LlmClient::resolve_num_ctx(&request),
            16_384,
            "an explicit override must win over whatever the card measures"
        );

        unsafe { std::env::remove_var("NANNA_OLLAMA_NUM_CTX") };
    }

    /// ...but the override must not become a silent floor of its own: absent or
    /// unusable values fall through to sizing rather than to a wrong constant.
    #[test]
    fn an_absent_or_junk_override_falls_through_to_sizing() {
        let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: see above.
        unsafe { std::env::remove_var("NANNA_OLLAMA_NUM_CTX") };
        assert_eq!(LlmClient::env_num_ctx(), None);

        unsafe { std::env::set_var("NANNA_OLLAMA_NUM_CTX", "not-a-number") };
        assert_eq!(LlmClient::env_num_ctx(), None, "garbage must not be honoured");

        // Too small to hold a system prompt plus one tool round.
        unsafe { std::env::set_var("NANNA_OLLAMA_NUM_CTX", "512") };
        assert_eq!(LlmClient::env_num_ctx(), None, "an unusable size is not a setting");

        unsafe { std::env::remove_var("NANNA_OLLAMA_NUM_CTX") };
    }

    /// The pin is min(env, sized) in BOTH directions. Fake model names are
    /// absent from Ollama's tags, so the VRAM probe abstains and sizing lands
    /// on the nominal 16384 deterministically on any machine — which makes
    /// both sides of the min observable without mocking the card.
    #[test]
    fn the_env_pin_is_a_ceiling_on_the_starting_latch_in_both_directions() {
        let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let mut request = request_with_tool_history(serde_json::json!({}));
        request.context_limit = None;

        // Shrink: a pin below what sizing would pick IS the starting latch.
        // SAFETY: edition 2024 requires this; env_lock serialises the
        // env-touching tests, and nothing else reads this key.
        unsafe { std::env::set_var("NANNA_OLLAMA_NUM_CTX", "8192") };
        request.model = "test-env-pin-shrinks:1b".to_string();
        assert_eq!(LlmClient::resolve_num_ctx(&request), 8_192);
        assert_eq!(
            LlmClient::latched_num_ctx("test-env-pin-shrinks:1b"),
            Some(8_192),
            "the pin must be LATCHED as the start, not re-derived per request"
        );

        // Never grow: a pin above what sizing supports starts at the sized
        // value — the env cannot inflate the window past the machine.
        unsafe { std::env::set_var("NANNA_OLLAMA_NUM_CTX", "32768") };
        request.model = "test-env-pin-cannot-grow:1b".to_string();
        assert_eq!(LlmClient::resolve_num_ctx(&request), 16_384);

        unsafe { std::env::remove_var("NANNA_OLLAMA_NUM_CTX") };
    }

    /// The pin is a ceiling, not a disable: a real GPU fault still walks the
    /// latch below the pinned start, the walked-down value survives later
    /// requests (the latch, not the env, is consulted once it exists), and an
    /// explicit per-request `context_limit` keeps outranking both.
    #[test]
    fn demotion_still_walks_below_the_env_pin() {
        let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: see above.
        unsafe { std::env::set_var("NANNA_OLLAMA_NUM_CTX", "8192") };

        let mut request = request_with_tool_history(serde_json::json!({}));
        request.model = "test-env-pin-demotes:1b".to_string();
        request.context_limit = None;
        assert_eq!(LlmClient::resolve_num_ctx(&request), 8_192);

        assert_eq!(
            LlmClient::demote_context("test-env-pin-demotes:1b", None),
            Some(6_144)
        );
        assert_eq!(
            LlmClient::resolve_num_ctx(&request),
            6_144,
            "a demotion below the pin must stick — the pin must not re-inflate it"
        );

        request.context_limit = Some(2_048);
        assert_eq!(
            LlmClient::resolve_num_ctx(&request),
            2_048,
            "an explicit request limit keeps its precedence over pin and latch"
        );

        unsafe { std::env::remove_var("NANNA_OLLAMA_NUM_CTX") };
    }

    /// A pin that changes the starting latch must say so at INFO — model,
    /// pinned value, and source — so an operator reading the logs knows the
    /// window was deliberate, not measured.
    #[test]
    fn the_env_pin_announces_itself_at_info() {
        let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: see above.
        unsafe { std::env::set_var("NANNA_OLLAMA_NUM_CTX", "8192") };

        let mut request = request_with_tool_history(serde_json::json!({}));
        request.model = "test-env-pin-announces:1b".to_string();
        request.context_limit = None;

        let out = capture_info_logs(|| {
            assert_eq!(LlmClient::resolve_num_ctx(&request), 8_192);
        });
        assert!(
            out.contains("NANNA_OLLAMA_NUM_CTX pins"),
            "the pin must announce itself, got: {out}"
        );
        assert!(
            out.contains("test-env-pin-announces:1b"),
            "the announcement must name the model, got: {out}"
        );
        assert!(
            out.contains("8192"),
            "the announcement must carry the pinned value, got: {out}"
        );
        assert!(
            out.contains("env"),
            "the announcement must name its source, got: {out}"
        );

        unsafe { std::env::remove_var("NANNA_OLLAMA_NUM_CTX") };
    }

    /// With the variable unset the sizing path is today's behavior exactly:
    /// the sized start, and no pin announcement to mislead an operator into
    /// hunting for a setting nobody made.
    #[test]
    fn an_unset_env_neither_pins_nor_announces() {
        let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: see above.
        unsafe { std::env::remove_var("NANNA_OLLAMA_NUM_CTX") };

        let mut request = request_with_tool_history(serde_json::json!({}));
        request.model = "test-env-pin-unset:1b".to_string();
        request.context_limit = None;

        let out = capture_info_logs(|| {
            assert_eq!(LlmClient::resolve_num_ctx(&request), 16_384);
        });
        assert!(
            !out.contains("NANNA_OLLAMA_NUM_CTX pins"),
            "no pin means no pin announcement, got: {out}"
        );
    }

    fn env_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(Default::default)
    }

    /// Run `f` with a thread-local INFO subscriber and return everything it
    /// logged. Thread-local (`with_default`, not the global default) so
    /// parallel tests on other threads neither pollute nor race this capture.
    fn capture_info_logs(f: impl FnOnce()) -> String {
        #[derive(Clone, Default)]
        struct Sink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl std::io::Write for Sink {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Sink {
            type Writer = Sink;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let sink = Sink::default();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .with_writer(sink.clone())
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(subscriber, f);
        let bytes = sink.0.lock().unwrap_or_else(|e| e.into_inner());
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// A big co-resident model is real pressure and must actually shrink the
    /// window, unlike the assumed-constant it replaced.
    #[test]
    fn a_large_co_resident_model_shrinks_the_window() {
        let tight =
            LlmClient::fit_context_for_budget(FREE_UNLOADED, 7.56 * GB, 0.0, 2.5 * GB);
        assert!(tight < 16_384, "2.5 GB of neighbour must cost context, got {tight}");
    }

    #[test]
    fn a_card_with_nothing_left_still_returns_a_runnable_floor() {
        assert_eq!(
            LlmClient::fit_context_for_budget(4.0 * GB, 7.56 * GB, 0.0, 0.0),
            4_096
        );
        assert_eq!(LlmClient::fit_context_for_budget(0.0, 0.0, 0.0, 0.0), 4_096);
    }

    // -----------------------------------------------------------------
    // Degenerate-repetition watch
    // -----------------------------------------------------------------

    #[test]
    fn a_wedged_runner_is_caught() {
        // The live shape: the same one-character token forever.
        let mut w = RepeatWatch::default();
        let mut tripped_at = None;
        for i in 1..=60 {
            if let Some(run) = w.observe("0") {
                tripped_at = Some((i, run));
                break;
            }
        }
        let (i, run) = tripped_at.expect("a stuck token must be caught");
        assert_eq!(i, RepeatWatch::MAX_RUN, "must trip exactly at the bound");
        assert_eq!(run, RepeatWatch::MAX_RUN);
    }

    #[test]
    fn ordinary_repetition_is_left_alone() {
        // Indentation and ellipses repeat legitimately -- just not 40x.
        let mut w = RepeatWatch::default();
        for _ in 0..(RepeatWatch::MAX_RUN - 1) {
            assert!(w.observe("  ").is_none(), "whitespace is not a candidate");
        }
        // Real content RESETS the run, so separated bursts never accumulate
        // into a false positive — this is what keeps the guard off normal
        // output that happens to repeat a short token in places. Counts are
        // expressed against the bound so lowering it can't silently make
        // this test vacuous (it did exactly that at 40 → 20).
        let burst = RepeatWatch::MAX_RUN - 1;
        let mut w2 = RepeatWatch::default();
        for _ in 0..burst {
            assert!(w2.observe(".").is_none());
        }
        assert!(w2.observe(" and then some prose").is_none());
        for _ in 0..burst {
            assert!(
                w2.observe(".").is_none(),
                "the run must restart after intervening content"
            );
        }
    }

    /// The wedge carries its own fingerprint, and still renders as the 502 it
    /// used to be.
    ///
    /// Both halves are contracts. The fields exist so the daemon's retry
    /// ladder can compare one wedge against the next without parsing prose;
    /// the rendered text is what `is_transient_llm_error` ("API error: 5") and
    /// `wedged_runner_error` ("same token") match on, so a reword here would
    /// stop wedges being retried at all.
    #[test]
    fn a_wedge_carries_structure_and_keeps_its_rendered_form() {
        let err = LlmError::WedgedRunner {
            token: "0".to_string(),
            run: RepeatWatch::MAX_RUN,
            bytes_received: 2780,
        };
        let LlmError::WedgedRunner { token, run, bytes_received } = &err else {
            panic!("must stay a distinct variant the caller can match on");
        };
        assert_eq!((token.as_str(), *run, *bytes_received), ("0", 20, 2780));

        let rendered = err.to_string();
        assert!(rendered.contains("API error: 5"), "must stay transient: {rendered}");
        assert!(rendered.contains("same token"), "must stay a wedge: {rendered}");
        assert!(rendered.contains("2780 bytes"), "must still say where it gave up: {rendered}");
        // And it must keep taking the 502 fallback path it took as an Api error.
        assert!(err.should_fallback());
    }

    #[test]
    fn a_repeated_long_fragment_is_content_not_a_fault() {
        let mut w = RepeatWatch::default();
        for _ in 0..200 {
            assert!(
                w.observe("SELECT * FROM t;").is_none(),
                "long repeated fragments are legitimate output"
            );
        }
    }
    use super::*;

    #[test]
    fn test_message_construction() {
        let msg = Message::user("Hello");
        assert!(matches!(msg.role, Role::User));
        assert_eq!(msg.content, "Hello");
    }

    /// Request with an echoed tool call in history — the shape that follows
    /// every tool execution in the agent loop.
    fn request_with_tool_history(input: serde_json::Value) -> AnthropicRequest {
        AnthropicRequest {
            model: "ornith:latest".to_string(),
            messages: vec![
                AnthropicMessage::user_text("check the lock file"),
                AnthropicMessage::assistant(vec![ContentBlock::ToolUse {
                    id: "call_1".to_string(),
                    name: "read_file".to_string(),
                    input,
                }]),
                AnthropicMessage::user(vec![ContentBlock::ToolResult {
                    tool_use_id: "call_1".to_string(),
                    content: "NO_LOCK".to_string(),
                    is_error: None,
                }]),
            ],
            max_tokens: 128,
            temperature: None,
            system: None,
            tools: None,
            stream: None,
            thinking: None,
            cache_control: None,
            context_limit: None,
        }
    }

    fn first_tool_arguments(messages: &serde_json::Value) -> serde_json::Value {
        messages
            .as_array()
            .unwrap()
            .iter()
            .find_map(|m| m.get("tool_calls"))
            .and_then(|tc| tc.as_array())
            .and_then(|tc| tc.first())
            .map(|tc| tc["function"]["arguments"].clone())
            .expect("converted request must contain a tool_call")
    }

    #[test]
    fn openai_conversion_keeps_tool_arguments_as_json_string() {
        // OpenAI-compatible /v1/chat/completions requires the string form.
        let request = request_with_tool_history(serde_json::json!({"path": "D:/x"}));
        let (messages, _) = anthropic_to_openai_request(&request);
        let args = first_tool_arguments(&messages);
        assert_eq!(args, serde_json::json!("{\"path\":\"D:/x\"}"));
    }

    #[test]
    fn ollama_conversion_keeps_tool_arguments_as_object() {
        // Ollama's native /api/chat 400s on string-form arguments — even valid
        // JSON inside a string ("Value looks like object, but can't find
        // closing '}' symbol", verified live against Ollama 0.31.2).
        let request = request_with_tool_history(serde_json::json!({"path": "D:/x"}));
        let (messages, _) = anthropic_to_ollama_request(&request);
        let args = first_tool_arguments(&messages);
        assert_eq!(args, serde_json::json!({"path": "D:/x"}));
    }

    #[test]
    fn ollama_conversion_degrades_non_object_input_to_empty_object() {
        // A non-object input would 400 the whole request on the native
        // endpoint; losing the arguments beats losing the turn.
        let request = request_with_tool_history(serde_json::json!("not an object"));
        let (messages, _) = anthropic_to_ollama_request(&request);
        let args = first_tool_arguments(&messages);
        assert_eq!(args, serde_json::json!({}));
    }

    #[test]
    fn test_estimate_tokens_ascii_and_cjk() {
        assert_eq!(estimate_tokens(""), 0);
        // ASCII English: ~4 chars/token, rounded up so short strings aren't zero.
        assert_eq!(estimate_tokens("hi"), 1);
        assert_eq!(estimate_tokens(&"a".repeat(8)), 2);
        // CJK is denser: 4 chars ~ 4 tokens (byte-len/4 would wrongly give 3).
        let cjk = "你好世界"; // 4 chars, 12 UTF-8 bytes
        assert_eq!(cjk.len(), 12);
        assert_eq!(estimate_tokens(cjk), 4);
        // Mixed: 2 ASCII (→1) + 2 wide (→2) = 3.
        assert_eq!(estimate_tokens("hi你好"), 3);
    }

    #[test]
    fn test_estimate_tokens_code_vs_english_ratio() {
        // Pure prose ASCII letter run → English ratio (4 chars/token).
        let prose = "abcdefgh"; // 8 chars
        assert_eq!(
            estimate_tokens_for_family(prose, TokenContentFamily::English),
            2
        );
        // Same length treated as code → denser 3.5 chars/token → ceil(8/3.5)=3.
        assert_eq!(
            estimate_tokens_for_family(prose, TokenContentFamily::Code),
            3
        );
        // Auto-detect: punctuation-heavy JSON should pick code path.
        let jsonish = r#"{"a":1,"b":2,"c":3,"d":4}"#; // lots of braces/colons
        let auto = estimate_tokens_for_family(jsonish, TokenContentFamily::Auto);
        let as_code = estimate_tokens_for_family(jsonish, TokenContentFamily::Code);
        assert_eq!(auto, as_code);
    }

    #[test]
    fn test_estimate_request_tokens_adds_message_framing() {
        // Two 8-char ASCII messages: 2 tokens content each + framing per message.
        let request = CompletionRequest::default()
            .with_message(Message::user("aaaaaaaa"))
            .with_message(Message::user("bbbbbbbb"));
        let expected = 2 * (2 + MESSAGE_FRAMING_TOKENS);
        assert_eq!(estimate_request_tokens(&request), expected);
    }

    #[test]
    fn test_estimate_tokens_exact_smoke() {
        // Empty always 0; non-empty path must agree with some positive count
        // (exact under the tiktoken feature, heuristic otherwise).
        assert_eq!(estimate_tokens_exact(""), 0);
        let n = estimate_tokens_exact("hello world");
        assert!(n >= 1, "expected at least 1 token, got {n}");
        // Model-hinted path should also return a positive count.
        let m = estimate_tokens_for_model("hello world", "gpt-4o");
        assert!(m >= 1, "expected at least 1 token for gpt-4o, got {m}");
    }

    #[test]
    fn test_request_builder() {
        let request = CompletionRequest::default()
            .with_model("gpt-4")
            .with_message(Message::user("Hi"))
            .with_max_tokens(1000)
            .with_temperature(0.5);

        assert_eq!(request.model, "gpt-4");
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.max_tokens, Some(1000));
        assert_eq!(request.temperature, Some(0.5));
    }

    #[test]
    fn test_delta_data_unknown_variant() {
        // Truly unknown delta types should deserialize to Unknown
        let json = r#"{"type": "some_future_delta", "data": "xyz"}"#;
        let delta: DeltaData = serde_json::from_str(json).unwrap();
        assert!(matches!(delta, DeltaData::Unknown));
    }

    #[test]
    fn test_delta_data_signature_variant() {
        // signature_delta should now be recognized (required for multi-turn thinking)
        let json = r#"{"type": "signature_delta", "signature": "abc123"}"#;
        let delta: DeltaData = serde_json::from_str(json).unwrap();
        assert!(matches!(delta, DeltaData::SignatureDelta { .. }));
        if let DeltaData::SignatureDelta { signature } = delta {
            assert_eq!(signature, "abc123");
        }
    }

    #[test]
    fn test_delta_data_known_variants() {
        let text = r#"{"type": "text_delta", "text": "hello"}"#;
        let delta: DeltaData = serde_json::from_str(text).unwrap();
        assert!(matches!(delta, DeltaData::TextDelta { .. }));

        let json_delta = r#"{"type": "input_json_delta", "partial_json": "{\"key\":"}"#;
        let delta: DeltaData = serde_json::from_str(json_delta).unwrap();
        assert!(matches!(delta, DeltaData::InputJsonDelta { .. }));
    }

    #[test]
    fn test_content_block_delta_with_signature_delta() {
        // signature_delta should parse as SignatureDelta now
        let json = r#"{"type": "content_block_delta", "index": 1, "delta": {"type": "signature_delta", "signature": "sig"}}"#;
        let sse: AnthropicSSE = serde_json::from_str(json).unwrap();
        match sse {
            AnthropicSSE::ContentBlockDelta { index, delta } => {
                assert_eq!(index, 1);
                assert!(matches!(delta, DeltaData::SignatureDelta { .. }));
            }
            other => panic!("Expected ContentBlockDelta, got {:?}", other),
        }
    }

    #[test]
    fn test_content_block_delta_with_unknown_delta() {
        // Truly unknown delta types should still parse via Unknown fallback
        let json = r#"{"type": "content_block_delta", "index": 2, "delta": {"type": "future_delta_type"}}"#;
        let sse: AnthropicSSE = serde_json::from_str(json).unwrap();
        match sse {
            AnthropicSSE::ContentBlockDelta { index, delta } => {
                assert_eq!(index, 2);
                assert!(matches!(delta, DeltaData::Unknown));
            }
            other => panic!("Expected ContentBlockDelta, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_sse_event_signature_delta() {
        let event = "data: {\"type\": \"content_block_delta\", \"index\": 0, \"delta\": {\"type\": \"signature_delta\", \"signature\": \"abc\"}}";
        // Should now return a SignatureDelta event
        let result = parse_sse_event(event);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), StreamEvent::SignatureDelta { index: 0, .. }));
    }

    #[test]
    fn test_parse_sse_event_message_start_carries_usage() {
        // message_start reports prompt-side usage incl. cache read/write — the
        // streaming path must surface these for accurate cache-cost tracking.
        let event = "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-sonnet-4\",\"usage\":{\"input_tokens\":12,\"cache_read_input_tokens\":900,\"cache_creation_input_tokens\":34}}}";
        let result = parse_sse_event(event).expect("message_start should parse");
        match result {
            StreamEvent::MessageStart {
                input_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                ..
            } => {
                assert_eq!(input_tokens, 12);
                assert_eq!(cache_read_tokens, 900);
                assert_eq!(cache_creation_tokens, 34);
            }
            other => panic!("expected MessageStart, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_sse_event_message_start_without_usage_is_zero() {
        // Missing usage (or a non-Anthropic shape) must default to zeros, not fail.
        let event = "data: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"model\":\"x\"}}";
        let result = parse_sse_event(event).expect("message_start should parse");
        assert!(matches!(
            result,
            StreamEvent::MessageStart {
                input_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                ..
            }
        ));
    }

    fn model_info(context_window: usize, max_output_tokens: usize) -> ModelInfo {
        ModelInfo {
            id: "test".into(),
            context_window,
            max_output_tokens,
            supports_tools: false,
            supports_vision: false,
            embedding_dimension: None,
            cached_at: 0,
            provider: "test".into(),
        }
    }

    #[test]
    fn compression_threshold_stays_below_hard_limit_for_small_models() {
        // Small model, large output budget: 80%-of-context (6400) would exceed
        // the hard input cap (6000), so proactive compression must be pulled
        // below it instead of never firing.
        let small = model_info(8_000, 4_096);
        assert!(
            small.compression_threshold() < small.hard_input_limit(),
            "threshold {} must be below hard limit {}",
            small.compression_threshold(),
            small.hard_input_limit()
        );
    }

    #[test]
    fn compression_threshold_unchanged_for_large_models() {
        // Large context model: 80%-of-context is already the smaller value, so
        // the fix must not perturb it (no regression for cloud models).
        let large = model_info(200_000, 8_192);
        assert_eq!(large.compression_threshold(), 160_000);
        assert!(large.compression_threshold() < large.hard_input_limit());
    }

    #[test]
    fn compression_threshold_holds_when_output_exceeds_context() {
        // Degenerate provider report (max_output >= context): hard_input_limit
        // floors at 50% of context; the threshold must still sit at/below it.
        let degenerate = model_info(4_000, 8_000);
        assert!(degenerate.compression_threshold() <= degenerate.hard_input_limit());
    }

    #[test]
    fn hard_input_limit_caps_the_output_reservation() {
        // The live regression: Ollama reports max_output >= context for
        // qwen3.5:9b (32k window), and subtracting the claim verbatim halved
        // the input side to 16k — the user's prompt got emergency-truncated.
        // Reservation is capped at 25% of the window, so 24k stays usable.
        let qwen_like = model_info(32_000, 32_000);
        assert_eq!(qwen_like.hard_input_limit(), 24_000);
        // A modest, honest output claim is reserved in full.
        let honest = model_info(32_000, 4_096);
        assert_eq!(honest.hard_input_limit(), 32_000 - 4_096);
    }

    #[test]
    fn dynamic_output_reserve_frees_input_for_small_budgets() {
        // The reserve tracks the actual request budget, not the provider's
        // max_output claim: a 2k-output agent on a 32k window keeps ~94%
        // for input, and the pair never over-commits.
        let m = model_info(32_000, 32_000);
        assert_eq!(m.effective_output_budget(2_048), 2_048);
        assert_eq!(m.hard_input_limit_for(2_048), 29_952);
        // Degenerate request budgets are bounded by half the window.
        assert_eq!(m.effective_output_budget(30_000), 16_000);
        assert_eq!(m.hard_input_limit_for(16_000), 16_000);
        for requested in [1_usize, 2_048, 8_192, 30_000, 64_000] {
            let out = m.effective_output_budget(requested);
            assert!(
                m.hard_input_limit_for(out) + out <= 32_000,
                "over-commit at requested={requested}"
            );
            assert!(m.compression_threshold_for(out) <= m.hard_input_limit_for(out));
        }
    }

    #[test]
    fn input_and_output_budgets_never_over_commit_the_window() {
        // The paired-budget invariant: a request whose input respects
        // hard_input_limit and whose max_tokens respects output_token_budget
        // can never exceed the model's context window.
        for (ctx, out) in [
            (32_000, 32_000),
            (32_000, 4_096),
            (8_000, 4_096),
            (4_000, 8_000),
            (2_000, 1_000),
        ] {
            let m = model_info(ctx, out);
            assert!(
                m.hard_input_limit() + m.output_token_budget() <= ctx,
                "over-commit at ctx={ctx} out={out}: {} + {} > {ctx}",
                m.hard_input_limit(),
                m.output_token_budget()
            );
        }
    }

    #[test]
    fn model_context_window_uses_universal_floor_without_cache() {
        // No per-model table: uncached names all inherit the conservative floor.
        assert_eq!(model_context_window("claude-opus-4-8"), UNKNOWN_CONTEXT_WINDOW);
        assert_eq!(model_context_window("gpt-4"), UNKNOWN_CONTEXT_WINDOW);
        assert_eq!(model_context_window("some-unknown-model"), UNKNOWN_CONTEXT_WINDOW);
        let info = unknown_model_info("x", "test");
        assert_eq!(info.context_window, UNKNOWN_CONTEXT_WINDOW);
        assert_eq!(info.max_output_tokens, UNKNOWN_MAX_OUTPUT_TOKENS);
    }

    #[test]
    fn conversation_history_budget_respects_hard_limit() {
        let small = model_info(8_000, 4_096);
        let budget = small.conversation_history_budget(2_000, 2_000, 1_000);
        assert!(budget <= small.hard_input_limit());
        assert!(budget >= 1_000);
        let tiny = model_info(2_000, 1_000);
        // min_history must not exceed hard limit
        assert_eq!(
            tiny.conversation_history_budget(10_000, 10_000, 5_000),
            tiny.hard_input_limit()
        );
    }
}

#[cfg(test)]
mod anthropic_model_contract_tests {
    use super::{anthropic_model_contract, ThinkingConfig, ThinkingDisplay};

    #[test]
    fn the_current_generation_dropped_budgets_and_sampling() {
        for model in [
            "claude-opus-5",
            "claude-opus-4-8",
            "claude-opus-4-7",
            "claude-sonnet-5",
        ] {
            let c = anthropic_model_contract(model);
            assert!(c.adaptive_thinking, "{model} takes adaptive thinking");
            assert!(c.sampling_removed, "{model} rejects temperature");
            assert!(
                c.display_defaults_omitted,
                "{model} hides reasoning text unless display is asked for"
            );
            assert!(!c.thinking_always_on, "{model} accepts an explicit disable");
        }
    }

    #[test]
    fn fable_and_mythos_cannot_be_told_not_to_think() {
        for model in ["claude-fable-5", "claude-mythos-5", "claude-mythos-preview"] {
            let c = anthropic_model_contract(model);
            assert!(c.thinking_always_on, "{model} rejects an explicit disable");
            assert!(c.adaptive_thinking);
            assert!(c.sampling_removed);
        }
    }

    #[test]
    fn the_four_six_family_is_adaptive_but_kept_sampling() {
        for model in ["claude-opus-4-6", "claude-sonnet-4-6"] {
            let c = anthropic_model_contract(model);
            assert!(c.adaptive_thinking, "{model} takes adaptive thinking");
            assert!(!c.sampling_removed, "{model} kept temperature");
            assert!(
                !c.display_defaults_omitted,
                "{model} already defaults to summarized reasoning"
            );
        }
    }

    #[test]
    fn pre_four_six_models_keep_budgets_and_sampling() {
        for model in [
            "claude-sonnet-4-20250514",
            "claude-opus-4-20250514",
            "claude-opus-4-5",
            "claude-opus-4-5-20251101",
            "claude-opus-4-1-20250805",
            "claude-sonnet-4-5-20250929",
            "claude-haiku-4-5-20251001",
            "claude-3-5-sonnet-20241022",
            "claude-3-opus-20240229",
        ] {
            let c = anthropic_model_contract(model);
            assert!(!c.adaptive_thinking, "{model} still takes budget_tokens");
            assert!(!c.sampling_removed, "{model} still takes temperature");
        }
    }

    /// The family patterns are substrings, so the generations they must not
    /// confuse are worth pinning: `sonnet-5` does not occur inside
    /// `sonnet-4-5`, and `opus-5` does not occur inside `opus-4-5`. Getting
    /// this wrong sends `budget_tokens` to a model that 400s on it, or strips
    /// `temperature` from one that wanted it.
    #[test]
    fn adjacent_generations_do_not_collide() {
        assert!(anthropic_model_contract("claude-sonnet-5").adaptive_thinking);
        assert!(!anthropic_model_contract("claude-sonnet-4-5").adaptive_thinking);
        assert!(anthropic_model_contract("claude-opus-5").adaptive_thinking);
        assert!(!anthropic_model_contract("claude-opus-4-5").adaptive_thinking);
    }

    #[test]
    fn provider_prefixes_and_dotted_aliases_still_classify() {
        // Bedrock prefixes the first-party id; some configs write `4.6`.
        assert!(anthropic_model_contract("anthropic.claude-opus-5").sampling_removed);
        assert!(anthropic_model_contract("anthropic/claude-opus-4-6").adaptive_thinking);
        assert!(!anthropic_model_contract("claude-opus-4.6").sampling_removed);
        assert!(anthropic_model_contract("CLAUDE-OPUS-5").adaptive_thinking);
    }

    /// An unrecognized name is assumed to be newer than this table, not older:
    /// every generation since 4.6 removed `budget_tokens`, so guessing legacy
    /// would 400 on the first request against a model we simply had not heard
    /// of yet.
    #[test]
    fn unknown_models_get_the_current_contract() {
        let c = anthropic_model_contract("claude-something-unreleased");
        assert!(c.adaptive_thinking);
        assert!(c.sampling_removed);
        assert!(c.display_defaults_omitted);
    }

    #[test]
    fn the_wire_shapes_serialize_as_the_api_spells_them() {
        let adaptive = serde_json::to_value(ThinkingConfig::Adaptive {
            display: Some(ThinkingDisplay::Summarized),
        })
        .expect("serializes");
        assert_eq!(
            adaptive,
            serde_json::json!({"type": "adaptive", "display": "summarized"})
        );

        let bare = serde_json::to_value(ThinkingConfig::adaptive()).expect("serializes");
        assert_eq!(
            bare,
            serde_json::json!({"type": "adaptive"}),
            "an unset display must be absent, not null"
        );

        let budgeted = serde_json::to_value(ThinkingConfig::enabled(4096)).expect("serializes");
        assert_eq!(
            budgeted,
            serde_json::json!({"type": "enabled", "budget_tokens": 4096})
        );

        let off = serde_json::to_value(ThinkingConfig::Disabled).expect("serializes");
        assert_eq!(off, serde_json::json!({"type": "disabled"}));
    }
}

#[cfg(test)]
mod gemma_sentinel_tests {
    use super::{pace_provider_requests, LlmClient, Provider};

    /// The exact payload that cost a benchmark run: gemma4:12b emitted
    /// `<unused50>` as assistant content and Ollama closed the stream with no
    /// `done=true`, 57 times, each discarding a complete reply.
    #[test]
    fn recognises_gemma_stop_sentinels() {
        for sentinel in ["<unused50>", "<unused0>", "<unused98>", " <unused7> "] {
            assert!(
                LlmClient::is_gemma_stop_sentinel(sentinel),
                "must recognise {sentinel} as a stop marker"
            );
        }
    }

    /// The sentinel glued to the end of real text — the form an exact-match
    /// test silently misses, which is why the first version of this fix did
    /// not stop the 502s.
    #[test]
    fn strips_a_trailing_sentinel_and_keeps_the_words() {
        assert_eq!(
            LlmClient::strip_trailing_stop_sentinel("all tests pass.<unused50>"),
            Some("all tests pass.")
        );
        assert_eq!(LlmClient::strip_trailing_stop_sentinel("<unused7>"), Some(""));
        assert_eq!(LlmClient::strip_trailing_stop_sentinel("no marker here"), None);
        assert_eq!(LlmClient::strip_trailing_stop_sentinel("<unused50> leads"), None);
        assert_eq!(LlmClient::strip_trailing_stop_sentinel("<unusedfoo>"), None);
    }

    /// Must stay exact. A reply that DISCUSSES the token is prose, and
    /// swallowing it would silently truncate the model's answer — the same
    /// class of bug as the 502, pointing the other way.
    #[test]
    fn leaves_ordinary_text_alone() {
        for text in [
            "<unused50> is a reserved token",
            "the answer is 42",
            "<unusedfoo>",
            "<unused>",
            "<unused123>",
            "",
            "<think>",
        ] {
            assert!(
                !LlmClient::is_gemma_stop_sentinel(text),
                "must NOT treat {text:?} as a stop marker"
            );
        }
    }

    // =========================================================================
    // OpenAI-compatible completion body decoding (OpenRouter reality)
    // =========================================================================
    // Shapes below are taken from live OpenRouter responses captured 2026-07-31
    // while diagnosing why a stored, WORKING key produced 386 straight
    // "error decoding response body" failures and zero successful dreams.

    /// OpenRouter pads a slow reasoning model's response with whitespace
    /// keep-alive before the JSON — observed live on nemotron-3-ultra.
    #[test]
    fn decodes_whitespace_padded_completion() {
        let body = format!(
            "{}{}",
            "\n         \n\n         \n\n         \n",
            r#"{"id":"gen-1","object":"chat.completion","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"OK","refusal":null,"reasoning":"The user wants OK."}}],"usage":{"prompt_tokens":18,"completion_tokens":20}}"#
        );
        assert_eq!(
            LlmClient::parse_openai_completion(200, &body).expect("padded body must decode"),
            "OK"
        );
    }

    /// Reasoning models may return `content: null` with the text in
    /// `reasoning`; an empty-success is worse than using what was sent.
    #[test]
    fn falls_back_to_reasoning_when_content_is_null() {
        let body = r#"{"choices":[{"message":{"role":"assistant","content":null,"reasoning":"Summary: the notes describe minidb progress."}}]}"#;
        assert_eq!(
            LlmClient::parse_openai_completion(200, body).expect("reasoning fallback"),
            "Summary: the notes describe minidb progress."
        );
    }

    /// The expensive one: OpenRouter returns HTTP 200 whose body is an error
    /// envelope (free-tier rate limits, moderation). This must surface as an
    /// error carrying the API's message — swallowing it as a decode failure is
    /// what made a working key look absent for two days.
    #[test]
    fn surfaces_error_envelope_inside_http_200() {
        let body = r#"{"error":{"message":"Rate limit exceeded: free-models-per-day","code":429}}"#;
        let err = LlmClient::parse_openai_completion(200, body).expect_err("error body is an error");
        let msg = err.to_string();
        assert!(
            msg.contains("Rate limit exceeded"),
            "the API's own message must survive: {msg}"
        );
    }

    /// A 200-wrapped 429 must CLASSIFY as a rate limit, not merely mention one:
    /// only `LlmError::RateLimit` engages the summarizer's backoff walk, and a
    /// rate limit that classifies as a generic failure gets retried at full
    /// speed — the one response it must never get.
    #[test]
    fn wrapped_rate_limit_classifies_as_rate_limit() {
        // Numeric code, as OpenRouter sends it.
        let body = r#"{"error":{"message":"Rate limit exceeded: free-models-per-day","code":429}}"#;
        let err = LlmClient::parse_openai_completion(200, body).expect_err("must fail");
        assert!(err.is_rate_limit(), "numeric embedded 429 must classify: {err}");

        // String code — some providers quote it.
        let body = r#"{"error":{"message":"slow down","code":"429"}}"#;
        let err = LlmClient::parse_openai_completion(200, body).expect_err("must fail");
        assert!(err.is_rate_limit(), "string embedded 429 must classify: {err}");

        // No code at all, but the human-form message — OpenRouter's spelling
        // has a space, which the old matcher (underscore only) missed.
        let body = r#"{"error":{"message":"Rate limit exceeded, retry later"}}"#;
        let err = LlmClient::parse_openai_completion(200, body).expect_err("must fail");
        assert!(err.is_rate_limit(), "message text must classify: {err}");

        // Negative space: an unrelated error must NOT classify as a rate limit.
        let body = r#"{"error":{"message":"model not found","code":404}}"#;
        let err = LlmClient::parse_openai_completion(200, body).expect_err("must fail");
        assert!(!err.is_rate_limit(), "404 is not a rate limit: {err}");
    }

    /// A body that is neither shape must be reported WITH a snippet of itself.
    /// "error decoding response body" with no body was the whole two-day bug.
    #[test]
    fn undecodable_body_includes_a_snippet() {
        let err = LlmClient::parse_openai_completion(200, ": OPENROUTER PROCESSING")
            .expect_err("garbage is an error");
        assert!(
            err.to_string().contains("OPENROUTER PROCESSING"),
            "snippet missing from: {err}"
        );
    }

    /// A valid completion with no usable text anywhere is a failure, not an
    /// empty-string success that would be stored as a real summary.
    #[test]
    fn empty_completion_is_an_error_not_empty_success() {
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"","reasoning":""}}]}"#;
        assert!(LlmClient::parse_openai_completion(200, body).is_err());
    }

    // =========================================================================
    // Provider request pacing
    // =========================================================================

    /// Back-to-back requests to a paced provider must be held to its derived
    /// spacing — the first working dream cycle fired one request per cluster
    /// in the same second and lost 20 of 22 clusters to 429s.
    #[tokio::test(start_paused = true)]
    async fn paced_provider_requests_are_spaced() {
        let spacing = Provider::OpenRouter
            .min_request_spacing()
            .expect("OpenRouter is paced");
        let t0 = tokio::time::Instant::now();
        pace_provider_requests(Provider::OpenRouter).await;
        pace_provider_requests(Provider::OpenRouter).await;
        pace_provider_requests(Provider::OpenRouter).await;
        assert!(
            t0.elapsed() >= spacing * 2,
            "three consecutive calls must span two intervals, got {:?}",
            t0.elapsed()
        );
    }

    /// Each paced provider has its OWN clock: pacing one must not delay the
    /// other, or the gate reintroduces the cross-caller contention it exists
    /// to remove.
    ///
    /// Measured on `GitHubModels`, which no OTHER test touches — the clocks
    /// are process-global and the paced-spacing test stamps OpenRouter's, so
    /// measuring OpenRouter here would depend on test ordering.
    #[tokio::test(start_paused = true)]
    async fn provider_clocks_are_independent() {
        pace_provider_requests(Provider::OpenRouter).await;
        let t0 = tokio::time::Instant::now();
        // GitHub's first-ever call: its own (empty) clock is what matters;
        // OpenRouter's just-stamped clock must not be consulted.
        pace_provider_requests(Provider::GitHubModels).await;
        assert_eq!(
            t0.elapsed(),
            tokio::time::Duration::ZERO,
            "a different provider's clock must not impose a wait"
        );
    }

    /// Providers without a published quota are never stalled: local Ollama's
    /// only quota is the GPU, and tier-dependent clouds are handled reactively.
    #[tokio::test(start_paused = true)]
    async fn unpaced_providers_are_never_stalled() {
        let t0 = tokio::time::Instant::now();
        for _ in 0..5 {
            pace_provider_requests(Provider::Ollama).await;
            pace_provider_requests(Provider::Anthropic).await;
            pace_provider_requests(Provider::ClaudeProxy).await;
        }
        assert_eq!(t0.elapsed(), tokio::time::Duration::ZERO);
    }

    // =========================================================================
    // Dynamic pacing policy (PaceState::required_wait is pure — no clocks)
    // =========================================================================

    use super::PaceState;
    use tokio::time::{Duration, Instant};

    /// A generous provider-reported budget must run FASTER than the static
    /// prior — dynamic pacing is not just a safety brake.
    #[tokio::test(start_paused = true)]
    async fn ample_reported_budget_beats_the_static_prior() {
        let now = Instant::now();
        let state = PaceState {
            last_request: Some(now),
            remaining: Some(100),
            reset_at: Some(now + Duration::from_secs(60)),
        };
        let wait = state.required_wait(now, Duration::from_secs(3));
        assert!(
            wait <= Duration::from_millis(600),
            "100 requests over 60s is 600ms spacing, got {wait:?}"
        );
    }

    /// A scarce budget spreads evenly across the window instead of burning out
    /// early: 5 left with 50s to go means one every 10s, prior be damned.
    #[tokio::test(start_paused = true)]
    async fn scarce_reported_budget_stretches_beyond_the_prior() {
        let now = Instant::now();
        let state = PaceState {
            last_request: Some(now),
            remaining: Some(5),
            reset_at: Some(now + Duration::from_secs(50)),
        };
        let wait = state.required_wait(now, Duration::from_secs(3));
        assert_eq!(wait, Duration::from_secs(10));
    }

    /// An exhausted budget waits out the window — the provider said no.
    #[tokio::test(start_paused = true)]
    async fn exhausted_budget_waits_for_the_reset() {
        let now = Instant::now();
        let state = PaceState {
            last_request: Some(now),
            remaining: Some(0),
            reset_at: Some(now + Duration::from_secs(17)),
        };
        assert_eq!(
            state.required_wait(now, Duration::from_secs(3)),
            Duration::from_secs(17)
        );
    }

    /// A lapsed window means a refilled budget: no wait, even though the stale
    /// `remaining` still reads zero.
    #[tokio::test(start_paused = true)]
    async fn lapsed_window_means_refilled_budget() {
        let now = Instant::now();
        let state = PaceState {
            last_request: Some(now),
            remaining: Some(0),
            reset_at: Some(now - Duration::from_secs(1)),
        };
        assert_eq!(state.required_wait(now, Duration::from_secs(3)), Duration::ZERO);
    }

    /// Before any response has taught us anything, the published prior rules.
    #[tokio::test(start_paused = true)]
    async fn unlearned_state_uses_the_prior() {
        let now = Instant::now();
        let state = PaceState {
            last_request: Some(now),
            remaining: None,
            reset_at: None,
        };
        assert_eq!(
            state.required_wait(now, Duration::from_secs(3)),
            Duration::from_secs(3)
        );
        // …and a cold start (no previous request at all) never waits.
        assert_eq!(
            PaceState::default().required_wait(now, Duration::from_secs(3)),
            Duration::ZERO
        );
    }

    /// Elapsed time since the previous request counts against the spacing —
    /// the gate enforces spacing, it does not add latency on top of it.
    #[tokio::test(start_paused = true)]
    async fn elapsed_time_counts_toward_the_spacing() {
        let now = Instant::now();
        let state = PaceState {
            last_request: Some(now - Duration::from_secs(2)),
            remaining: None,
            reset_at: None,
        };
        assert_eq!(
            state.required_wait(now, Duration::from_secs(3)),
            Duration::from_secs(1)
        );
    }
}
