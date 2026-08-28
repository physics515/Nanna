
/// Degradation ledger for tracking agent health issues
#[derive(Debug, Clone)]
pub struct DegradationLedger {
    pub hallucination_count: u32,
    pub tool_error_count: u32,
    pub loop_count: u32,
    pub last_hallucination_at: Option<u64>,
    pub last_tool_error_at: Option<u64>,
    pub last_loop_at: Option<u64>,
}

/// Run options for the agent loop
#[derive(Debug, Clone)]
pub struct RunOptions {
    pub max_iterations: u32,
    pub max_tokens_per_iteration: u32,
    pub temperature: f32,
    pub top_p: f32,
    pub model_tier: u32,
    pub allow_parallel_tools: bool,
    pub allow_memory_ops: bool,
    pub allow_web_search: bool,
}

/// Agent response structure
#[derive(Debug, Clone)]
pub struct AgentResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub reasoning: Option<ReasoningContent>,
    pub emotional_context: Option<EmotionalContext>,
    pub extracted_memories: Vec<ExtractedMemory>,
    pub is_degraded: bool,
}

/// Reasoning content block
#[derive(Debug, Clone)]
pub struct ReasoningContent {
    pub blocks: Vec<ReasoningBlock>,
}

/// A single reasoning block
#[derive(Debug, Clone)]
pub struct ReasoningBlock {
    pub thought: String,
    pub tool_calls: Vec<ToolCall>,
}

/// Memory provenance source
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryProvenance {
    Stated,
    Observed,
}

/// Degradation ledger for tracking system health
#[derive(Debug, Clone)]
pub struct DegradationLedger {
    inner: std::sync::Mutex<DegradationState>,
}

/// Run options for agent execution
#[derive(Default)]
pub struct RunOptions {
    /// Override max iterations for this run
}

/// Step kind enum
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    /// Decomposing or re-planning a task — route to the most capable tier.
}

/// Agent response from loop runner
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    /// Final text response
}

/// Reasoning content block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningContent {
    /// The full reasoning/thinking text
}

/// Reasoning block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningBlock {
    /// The reasoning text for this block
}

/// Emotional context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmotionalContext {
    /// Primary emotion (e.g., "neutral", "frustrated", "excited", "confused")
}

/// Tool call record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub id: String,
}

/// Repeat ledger for detecting loops
#[derive(Debug, Clone)]
pub struct RepeatLedger {
    pub recent_calls: HashMap<String, u64>,
    pub threshold: u32,
    pub window_seconds: u64,
}

/// Nudge level enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NudgeLevel {
    /// First, gentle: keep going if progressing, else pause and answer.
}

/// Agent struct (large, kept separate)
pub type Agent = crate::agent::Agent;

/// Extracted memory type (already defined above, re-exported for completeness)
pub use super::loop_runner_types::ExtractedMemory;
