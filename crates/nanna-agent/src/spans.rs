//! Structured tracing spans for the agent loop (P6).
//!
//! The hierarchy is **turn → harness step → agent run → iteration → LLM call
//! / tool call**. The daemon opens the outermost span (`chat_turn`,
//! `sub_agent`, `scheduled_run`) with the session id; everything below is
//! opened here, so every log line a turn produces carries the session, the
//! step and the call it came from. Before this the daemon had no spans at
//! all, and two overlapping turns interleaved their lines with nothing to
//! tell them apart.
//!
//! Spans are opened at INFO so the daemon's default filter keeps them. The
//! cost is bounded by the loop's own shape — a handful of spans per LLM call
//! and one per tool call, never one per streamed token.
//!
//! Outcome fields (`latency_ms`, `output_bytes`, `success`, …) are declared
//! [`Empty`] at open and recorded once the call settles, so a span closes
//! carrying what it measured: the daemon's fmt layer prints them on close.

use crate::loop_runner::StepKind;
use tracing::Span;
use tracing::field::Empty;

/// Span names, shared with the tests that assert the hierarchy.
pub const RUN_SPAN: &str = "agent_run";
/// One loop iteration: one LLM call and the tool round that follows it.
pub const ITERATION_SPAN: &str = "agent_iteration";
/// Every provider request of one iteration, escalation and heals included.
pub const LLM_CALL_SPAN: &str = "llm_call";
/// One dispatched (or breaker-short-circuited) tool call.
pub const TOOL_CALL_SPAN: &str = "tool_call";
/// One harness step (P14): the agent run that works one task item.
pub const HARNESS_STEP_SPAN: &str = "harness_step";

/// Open the span for one agent run.
#[must_use]
pub fn run_span(model: &str) -> Span {
    debug_assert!(!model.is_empty(), "an agent run always names its model");
    tracing::info_span!(
        "agent_run",
        model = %model,
        iterations = Empty,
        input_tokens = Empty,
        output_tokens = Empty,
        outcome = Empty,
    )
}

/// Record how a run ended on its span.
pub fn record_run_outcome(
    span: &Span,
    iterations: usize,
    input_tokens: u64,
    output_tokens: u64,
    outcome: &'static str,
) {
    debug_assert!(!outcome.is_empty(), "a run outcome is always named");
    span.record("iterations", iterations);
    span.record("input_tokens", input_tokens);
    span.record("output_tokens", output_tokens);
    span.record("outcome", tracing::field::display(outcome));
}

/// Open the span for one loop iteration under its run. `iteration` is
/// 1-based, matching the count the loop logs and reports.
///
/// The parent is explicit because the loop body is not itself inside the
/// run span — only the futures it awaits are — so an implicit parent would
/// be whatever the caller had entered, skipping the run level.
#[must_use]
pub fn iteration_span(run: &Span, iteration: usize) -> Span {
    debug_assert!(iteration > 0, "iterations are counted from 1");
    debug_assert!(
        run.is_disabled() || run.metadata().is_some_and(|m| m.name() == RUN_SPAN),
        "an iteration's parent is its agent run"
    );
    tracing::info_span!(parent: run, "agent_iteration", iteration)
}

/// Open the span for one iteration's LLM traffic.
#[must_use]
pub fn llm_call_span(model: &str) -> Span {
    debug_assert!(!model.is_empty(), "an LLM call always names its model");
    tracing::info_span!(
        "llm_call",
        model = %model,
        served_model = Empty,
        latency_ms = Empty,
        escalated = Empty,
        input_tokens = Empty,
        output_tokens = Empty,
        tool_calls = Empty,
        outcome = Empty,
    )
}

/// What one iteration's LLM traffic produced, for [`record_llm_outcome`].
pub struct LlmCallOutcome<'a> {
    /// The model the exchange opened with (the span's `model`).
    pub requested_model: &'a str,
    /// The model that actually answered — escalation may have moved it.
    pub model: &'a str,
    pub latency_ms: u64,
    pub escalated: bool,
    /// `None` when every attempt failed.
    pub tokens: Option<(u32, u32)>,
    pub tool_calls: usize,
}

/// Record how one iteration's LLM traffic settled on its span.
pub fn record_llm_outcome(span: &Span, outcome: &LlmCallOutcome<'_>) {
    debug_assert!(
        !outcome.model.is_empty(),
        "the answering model is always named"
    );
    debug_assert!(
        outcome.tokens.is_some() || outcome.tool_calls == 0,
        "a failed call cannot have produced tool calls"
    );
    // Recorded only when it differs: a formatter appends re-recorded
    // fields rather than replacing them, so re-recording `model` itself
    // would print it twice on every call.
    if outcome.model != outcome.requested_model {
        span.record("served_model", tracing::field::display(outcome.model));
    }
    span.record("latency_ms", outcome.latency_ms);
    span.record("escalated", outcome.escalated);
    span.record("tool_calls", outcome.tool_calls);
    if let Some((input_tokens, output_tokens)) = outcome.tokens {
        span.record("input_tokens", input_tokens);
        span.record("output_tokens", output_tokens);
        span.record("outcome", tracing::field::display("ok"));
    } else {
        span.record("outcome", tracing::field::display("error"));
    }
}

/// Open the span for one tool call.
#[must_use]
pub fn tool_call_span(tool: &str, call_id: &str) -> Span {
    debug_assert!(!tool.is_empty(), "a tool call always names its tool");
    tracing::info_span!(
        "tool_call",
        tool = %tool,
        call_id = %call_id,
        duration_ms = Empty,
        output_bytes = Empty,
        success = Empty,
        short_circuited = Empty,
    )
}

/// Record how one tool call ended on its span. `output_bytes` is the size of
/// what the tool returned — content and error text — before any truncation
/// the loop applies for context.
pub fn record_tool_outcome(
    span: &Span,
    duration_ms: u64,
    output_bytes: usize,
    success: bool,
    short_circuited: bool,
) {
    debug_assert!(
        !short_circuited || duration_ms == 0,
        "a short-circuited call never ran, so it cannot have taken time"
    );
    debug_assert!(
        !short_circuited || !success,
        "a short-circuited call is reported as a refusal, never a success"
    );
    span.record("duration_ms", duration_ms);
    span.record("output_bytes", output_bytes);
    span.record("success", success);
    span.record("short_circuited", short_circuited);
}

/// Open the span for one harness step.
#[must_use]
pub fn harness_step_span(step_index: usize, item_id: i64, step_kind: StepKind) -> Span {
    debug_assert!(item_id >= 0, "task item ids are row ids, never negative");
    tracing::info_span!(
        "harness_step",
        step = step_index,
        item_id,
        kind = ?step_kind,
    )
}
