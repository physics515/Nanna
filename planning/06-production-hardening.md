# Phase 6: Production Hardening

**Status:** 🔶 Partially Complete

## Overview

Production hardening covers reliability, observability, and operational maturity. The goal is to make Nanna robust enough for always-on deployment: graceful error handling, rate limiting, metrics, cost tracking, and runtime configuration.

## Architecture

Rate limiting, queuing, and error recovery are implemented in `nanna-channels`. Metrics, tracing, and cost tracking are not yet implemented.

```
crates/nanna-channels/
├── src/
│   ├── queue.rs     # MessageQueue, RateLimiter, priority queue (596 lines)
│   ├── status.rs    # StatusManager, HealthChecker, ConnectionState (581 lines)
│   └── listeners/   # Channel-specific listeners with retry logic
```

## Current Implementation

### Rate Limiting (Outbound) ✅

**Location:** `crates/nanna-channels/src/queue.rs`

**`RateLimiter`** — Token bucket algorithm:
- `max_tokens` — Bucket capacity
- `refill_rate` — Tokens per second
- `for_provider()` — Provider-specific defaults:
  - Telegram: 30 tokens, 1/sec refill
  - Discord: 5 tokens, 5/sec refill
  - Slack: 1 token, 1/sec refill
  - Default: 10 tokens, 2/sec refill
- `can_send()` / `try_acquire()` — Check and consume tokens
- `time_until_available()` — Backpressure calculation
- `set_cooldown()` — External 429 response handling

**Suggestions:**
- Add per-channel rate limits (not just per-provider) — e.g., different limits for different Discord servers
- Implement adaptive rate limiting that learns from 429 responses
- Add rate limit metrics (requests/sec, throttle count, average wait time)

### Error Recovery / Retry Logic ✅

**Location:** `crates/nanna-channels/src/queue.rs`

**Exponential Backoff:**
- `calculate_backoff()` — Base delay × 2^attempts with jitter
- Configurable `max_retries` (default 3) and `base_retry_delay` (default 1s)
- Max backoff capped at 60 seconds
- Jitter: random 0-500ms added to prevent thundering herd

**Suggestions:**
- Differentiate transient vs permanent errors (don't retry 400 Bad Request)
- Add circuit breaker pattern: after N consecutive failures, stop trying for a cooldown period
- Log retry attempts with structured tracing

### Message Queuing ✅

**Location:** `crates/nanna-channels/src/queue.rs`

**`MessageQueue`** — Priority queue with burst handling:
- `BinaryHeap<QueuedMessage>` — Priority-ordered message queue
- `MessagePriority` — Critical > High > Normal > Low > Bulk
- `enqueue()` / `enqueue_with_priority()` — Add messages
- `process()` — Dequeue and send with rate limiting and retry
- `QueueStats` — Track enqueued, sent, failed, retried, rate_limited counts
- `QueueEvent` — Events for sent, failed, retried, rate_limited, queue_empty

**Per-channel queues** — Each provider gets its own `ChannelQueue` with independent rate limiter and stats.

**Suggestions:**
- Add queue persistence (messages survive daemon restart)
- Implement dead letter queue for permanently failed messages
- Add queue size limits with overflow handling (drop oldest Bulk messages)
- Add queue drain on shutdown (flush remaining messages before exit)

### Graceful Rate Limit Handling ✅

**Location:** `crates/nanna-channels/src/queue.rs` + `status.rs`

When a 429 is detected:
1. `set_cooldown()` on the rate limiter
2. `record_rate_limit()` on the status manager
3. Message re-queued with incremented retry count
4. Backoff delay applied before next attempt

**`StatusManager`** tracks:
- `ConnectionState` — Disconnected, Connecting, Connected, Degraded, RateLimited, Error, Maintenance
- `HealthMetrics` — Response times, failure counts, uptime
- `ChannelStatus` — Per-channel health with queue stats
- Broadcast events for real-time GUI updates

**Suggestions:**
- Add predictive rate limiting: slow down before hitting 429 based on response header hints
- Track rate limit patterns (time of day, message volume) for proactive throttling

### NOT YET IMPLEMENTED

#### Prometheus Metrics ❌

**What's needed:**
- Request latency histograms (LLM calls, tool execution, channel sends)
- Token usage counters (input, output, by model, by session)
- Queue depth gauges
- Error rate counters
- Memory usage (vector count, embedding dimensions)
- Active session count
- Swarm execution metrics

**Suggested Implementation:**
```rust
// crates/nanna-metrics/
use prometheus::{
    HistogramVec, IntCounterVec, IntGauge, Registry,
    histogram_opts, opts,
};

pub struct NannaMetrics {
    pub llm_request_duration: HistogramVec,    // labels: provider, model
    pub llm_tokens_total: IntCounterVec,       // labels: provider, model, direction (in/out)
    pub tool_execution_duration: HistogramVec,  // labels: tool_name
    pub channel_messages_total: IntCounterVec,  // labels: channel, direction (in/out)
    pub channel_errors_total: IntCounterVec,    // labels: channel, error_type
    pub queue_depth: IntGauge,                  // labels: channel
    pub active_sessions: IntGauge,
    pub memory_entries: IntGauge,
}
```

**Expose via:**
- HTTP `/metrics` endpoint on the health server (already has Axum)
- Tauri event for GUI dashboard

#### Tracing Spans for Tool Calls ❌

**What's needed:**
- Structured tracing with `tracing` crate (already a dependency)
- Span hierarchy: Session → Agent Loop → LLM Call / Tool Call
- Tool call spans with: name, duration, input size, output size, success/failure
- LLM call spans with: model, tokens, latency, streaming duration

**Current state:** Basic `tracing::info!` / `tracing::warn!` logging exists throughout the codebase. The daemon has emoji-prefixed tool logging. But there are no structured spans for performance analysis.

**Suggested Implementation:**
```rust
#[tracing::instrument(skip(self, tool_call), fields(
    tool = %tool_call.name,
    session = %self.session_id,
))]
async fn execute_tool(&self, tool_call: &ToolCall) -> Result<String, ToolError> {
    let span = tracing::info_span!("tool_execution", 
        tool.name = %tool_call.name,
        tool.input_bytes = tool_call.input.len(),
    );
    // ...
}
```

#### Cost Tracking Per Session ❌

**What's needed:**
- Track token usage per session (input + output + cached)
- Map tokens to cost using model pricing tables
- Aggregate cost by: session, time period, model, tool
- Display in GUI (per-session cost, daily/monthly totals)

**Suggested Implementation:**
```rust
pub struct CostTracker {
    pricing: HashMap<String, ModelPricing>,  // model -> pricing
    usage: Vec<UsageRecord>,
}

pub struct ModelPricing {
    input_per_million: f64,
    output_per_million: f64,
    cached_input_per_million: f64,
}

pub struct UsageRecord {
    session_id: String,
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    cached_tokens: u64,
    timestamp: i64,
    cost_usd: f64,
}
```

**Pricing data source:** Hardcoded table with periodic updates, or fetch from provider APIs.

#### Runtime Config Reload ❌

**What's needed:**
- Watch `config.toml` for changes
- Apply changes without restart
- Notify connected clients of config changes

**Current state:** Config is loaded at startup. Changes via GUI are saved to file and applied to in-memory state, but the daemon doesn't watch for external file changes.

**Suggested Implementation:**
- Use `notify` crate to watch config file
- Debounce changes (500ms)
- Validate new config before applying
- Emit config change events to GUI

#### Per-Channel Config ❌

**What's needed:**
- Different system prompts per channel
- Different model selection per channel
- Different tool allowlists per channel
- Different personality per channel

**Current state:** All channels share the same agent configuration.

**Suggested Implementation:**
```toml
[channels.telegram.agent]
system_prompt = "You are a concise assistant for Telegram."
model = "claude-sonnet-4-20250514"
max_tokens = 1000
tools = ["web_search", "recall", "remember"]

[channels.discord.agent]
system_prompt = "You are a helpful bot in Discord."
model = "claude-sonnet-4-20250514"
tools = ["*"]
```

#### Tool Allowlists/Blocklists ❌

**What's needed:**
- Global tool allowlist (only these tools are available)
- Global tool blocklist (these tools are never available)
- Per-channel tool restrictions
- Per-user tool restrictions (for multi-user channels)

**Current state:** All registered tools are available to all sessions.

**Suggested Implementation:**
```rust
pub struct ToolPolicy {
    global_allowlist: Option<HashSet<String>>,  // None = all allowed
    global_blocklist: HashSet<String>,
    channel_policies: HashMap<String, ChannelToolPolicy>,
}
```

## Priority Order

1. **Cost tracking** — Users need to know what they're spending
2. **Tracing spans** — Essential for debugging production issues
3. **Prometheus metrics** — Operational visibility
4. **Tool allowlists** — Security for multi-channel deployment
5. **Per-channel config** — Different channels need different behaviors
6. **Runtime config reload** — Convenience for always-on deployment
