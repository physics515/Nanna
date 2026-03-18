# Tool Stats Page — Planning Doc

## Problem
- Tool calls take minutes in production
- No visibility into which tools are slow, what's failing, or where time is spent
- Model stats page exists but only covers LLM requests, not tool execution

## Proposed: Tool & Session Stats Page

### 1. Tool Performance Stats (per tool)

Track and display for each tool:

| Metric | Description |
|--------|-------------|
| **Call count** | Total invocations |
| **Success rate** | % of calls that returned success |
| **Avg duration** | Mean execution time (ms) |
| **P50 / P95 / P99 latency** | Latency percentiles |
| **Last called** | Timestamp of most recent use |
| **Avg output size** | Mean response size (chars) |
| **Error breakdown** | Common error messages/types |

**UI:** Table with sortable columns + sparkline charts for latency trends.
Click a tool to see its recent invocations (last 50) with individual timings.

### 2. Session Stats (per conversation)

| Metric | Description |
|--------|-------------|
| **Total iterations** | Tool call rounds in the agent loop |
| **Total tool calls** | Number of tool invocations |
| **Tool time vs LLM time** | Pie chart: time spent in tools vs waiting for LLM |
| **Token usage** | Input/output/cache tokens |
| **Estimated cost** | Based on model pricing |
| **Context compression events** | How many times summarization triggered |
| **Narration loop detections** | Count of narration loop catches |
| **Wrap-up nudges** | Count of iteration budget nudges |

### 3. Live Run View (when agent is processing)

Real-time display during an active agent run:

- Current iteration number
- Active tool calls (with live timer)
- Accumulated text preview
- Token burn rate (tokens/sec)
- Tool call timeline (Gantt-chart style: which tools ran when, how long)

### 4. Global Dashboard

Top-level stats visible at a glance:

- **Slowest tools** (top 5 by P95 latency)
- **Most used tools** (top 10 by call count)  
- **Most failed tools** (top 5 by error rate)
- **Total tokens today / this week / this month**
- **Estimated cost today / this week / this month**
- **Average iterations per conversation**
- **Tool vs LLM time ratio** (are we spending more time in tools or waiting for the model?)

## Implementation Plan

### Phase 1: Data Collection (Rust)

**New crate: nanna-stats (or add to nanna-agent)**

```rust
pub struct ToolStats {
    pub name: String,
    pub call_count: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub latencies_ms: Vec<u64>,    // ring buffer, last 200
    pub output_sizes: Vec<usize>,  // ring buffer, last 200
    pub last_called: Option<i64>,  // epoch ms
    pub errors: Vec<(String, u64)>, // (error_msg, count)
}

pub struct SessionStats {
    pub session_id: String,
    pub iterations: usize,
    pub tool_calls: usize,
    pub tool_time_ms: u64,
    pub llm_time_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub compressions: usize,
    pub narration_loops: usize,
    pub wrapup_nudges: usize,
}
```

**Where to collect:**
- `loop_runner.rs` already tracks `duration_ms` per tool call — pipe to stats tracker
- `loop_runner.rs` already tracks `model_stats` — extend with tool stats
- Agent response already returns `tool_records` — aggregate into ToolStats

**Persistence:**
- Write to `{data_dir}/tool-stats.json` periodically (every 5 min or on shutdown)
- Keep last 30 days of aggregated daily stats
- Keep last 200 individual timings per tool (ring buffer)

### Phase 2: IPC Endpoints (Daemon)

```
get_tool_stats      → Vec<ToolStats>         // all tools
get_session_stats   → SessionStats           // specific session  
get_global_stats    → GlobalStats            // dashboard summary
get_tool_history    → Vec<ToolInvocation>     // last N calls for a tool
get_live_run_state  → LiveRunState           // current agent run (already exists partially)
```

### Phase 3: GUI Page

**Route:** `/stats` or extend `/model-stats` → `/performance`

**Layout:**
```
┌─────────────────────────────────────────┐
│ Performance Dashboard                    │
├─────────┬───────────────────────────────┤
│         │ [Global Stats Cards]           │
│         │ Tokens Today | Cost | Avg Iter │
│  Nav    ├───────────────────────────────┤
│         │ [Tool Performance Table]       │
│ Overview│ Name | Calls | P50 | P95 | ER │
│ Tools   │ read_file    12   45ms  120ms │
│ Sessions│ exec          8   2.1s   15s  │
│ Live    │ web_search    3   800ms  1.2s │
│         ├───────────────────────────────┤
│         │ [Tool vs LLM Time Chart]      │
│         │ ████████░░░░  65% tools       │
└─────────┴───────────────────────────────┘
```

### Phase 4: Diagnostics

Once we have the data, add diagnostic insights:
- "read_file P95 is 15s — this may indicate cross-filesystem I/O (WSL↔Windows)"
- "exec accounts for 80% of tool time — consider batching shell commands"
- "discover_tools called 3x per session — tool activation is not being cached"
- "Context compressed 4 times in this session — consider increasing context window"

## Priority

1. **Phase 1** — Start collecting stats now (small code change, big data win)
2. **Phase 2** — IPC endpoints (needed for GUI)
3. **Phase 3** — GUI page (user-visible value)
4. **Phase 4** — Diagnostics (nice to have, can be iterative)

## Quick Win: Tool Timing Logs

Even before the GUI, add `info!` logs for slow tools (>5s):
```rust
if duration_ms > 5000 {
    warn!(tool = name, duration_ms, "⚠️ Slow tool execution (>5s)");
}
```
This gives immediate visibility in the daemon logs.
