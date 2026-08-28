
/// Configuration for the agent loop.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Model to use (primary / most capable model)
    pub model: String,
    /// Maximum tokens in response
    pub max_tokens: u32,
    /// Temperature for sampling
    pub temperature: f32,
    /// Maximum iterations (tool call rounds). None = unlimited (the default).
    /// This is only an absolute runaway backstop; the loop is meant to run long.
    pub max_iterations: Option<usize>,
    /// Iteration at which the first escalating "wrap-up" soft nudge is injected.
    /// The loop is NOT stopped — the nudge only steers a possibly-stuck model.
    /// Default: 500.
    pub nudge_after_iterations: usize,
    /// After the first nudge, inject a further (more urgent) nudge every N
    /// iterations. Default: 100.
    pub nudge_interval_iterations: usize,
    /// Thinking mode for extended reasoning
    pub thinking_mode: ThinkingMode,
    /// Model priority list for summarization (first working model is used)
    /// Format: "provider/model" e.g. ["ollama/llama3.2", "openai/gpt-4o-mini", "anthropic/claude-haiku"]
    pub summarization_priority: Vec<String>,
    /// Ollama URL for summarization (if using ollama)
    pub summarization_ollama_url: Option<String>,
    /// OpenRouter API key (for summarization/extraction via OpenRouter models)
    pub openrouter_api_key: Option<String>,
    /// OpenAI API key (for summarization/extraction via OpenAI models)
    pub openai_api_key: Option<String>,
    /// Threshold (in chars) above which tool results are replaced with a
    /// memory-reference stub in context. 0 = auto (scales with model context window).
    /// Default: 0 (auto).
    pub context_result_threshold: usize,
    /// Progressive context distillation interval (in iterations).
    /// Every N iterations, the agent produces a rolling structured summary of the conversation.
    /// 0 = disabled. Default: 0 (uses existing threshold-based summarization only).
    pub distillation_interval: usize,
    /// Model routing: prioritized list of models for cost optimization.
    /// Each entry is "provider/model:tier" where tier is simple|medium|complex.
    /// When enabled, the agent classifies each iteration's complexity and routes
    /// to the cheapest model capable of handling it.
    /// Empty = disabled (always use primary model).
    /// Example: ["claude-haiku-3-5-20241022:simple", "claude-sonnet-4-20250514:complex"]
    pub model_routing: Vec<ModelTier>,
    /// Whether to always use the primary model for the first iteration
    /// (user-facing response quality). Default: true.
    pub routing_first_turn_primary: bool,
    /// Model to use for sub-agent tasks (optional).
    /// When set, sub-agents spawned via the `task` tool use this model instead of the primary.
    /// Use a cheaper model here to reduce costs for delegated sub-tasks.
    /// Format: "provider/model" e.g. "ollama/qwen3:4b" or "claude-3-5-haiku-20241022"
    pub sub_agent_model: Option<String>,
}

/// A model with its maximum complexity tier for routing purposes.
#[derive(Debug, Clone)]
pub struct ModelTier {
    /// Full model spec (may include provider prefix e.g. "ollama/deepseek-r1:14b")
    pub model: String,
    /// Maximum complexity this model should handle