# Phase 5: Agent Swarm & Parallel Execution

**Status:** ✅ Complete

## Overview

Inspired by Kimi K2.5's agent swarm architecture, this phase implements parallel task decomposition, multi-agent coordination, and sophisticated context management. The core insight: complex tasks can be broken into independent subtasks, executed in parallel by domain-specific agents, then synthesized.

## Architecture

```
crates/nanna-agent/
├── src/
│   ├── multi.rs        # AgentCoordinator, SwarmCoordinator, BackgroundTask (1017 lines)
│   ├── context.rs      # AgentContext, compression, dedup, summarization (1228 lines)
│   ├── loop_runner.rs  # Agent, RunOptions, ThinkingMode (1297 lines)
│   ├── chunker.rs      # CDC deduplication with Gear rolling hash (265 lines)
│   ├── summarizer.rs   # LLM-based summarization with caching (460 lines)
│   ├── supervisor.rs   # Erlang-style supervision (736 lines)
│   └── registry.rs     # Tool registry integration (967 lines)
```

## Current Implementation

### Swarm Coordinator (`multi.rs`)

**`AgentCoordinator`** — Central orchestrator for multi-agent execution:
- `register_agent()` — Register agents with configs and system prompts
- `spawn_task()` — Spawn background tasks with status tracking (Pending/Running/Completed/Failed)
- `spawn_swarm()` — Execute a swarm of parallel subtasks with configurable concurrency
- `parallel_research()` — Convenience method for parallel research queries
- `send_message()` / `check_mailbox()` — Inter-agent message passing

**`SwarmCoordinator`** — Higher-level task decomposition:
- `decompose_task()` — Uses LLM to break task into subtasks with dependencies
- `execute_task()` — Builds execution levels from dependency graph, runs parallel batches
- `build_execution_levels()` — Topological sort of subtask DAG into parallel levels
- `ensure_agent_registered()` — Auto-registers domain agents on demand

**`SwarmConfig`:**
- `max_parallel` — Concurrency limit (default 5)
- `timeout_per_task` — Per-task timeout (default 120s)
- `max_retries` — Retry count (default 1)
- `thinking_mode` — ThinkingMode for sub-agents

**`CriticalPathMetrics`:**
- `calculate()` — Computes wall time, total CPU time, parallelism ratio, critical path, speedup factor
- Tracks per-task timing for optimization

### Context Management (`context.rs`)

**`AgentContext`** — The heart of context window management:
- `messages_for_request()` — Prepends consolidated summary, deduplicates, returns messages
- `deduplicate_messages()` — Uses CDC chunk hashing to detect and remove duplicate content
- `estimate_tokens()` — Rough token estimation (~4 chars/token)
- `needs_compression()` / `exceeds_hard_limit()` — Threshold checks
- `truncate_to_limit()` — Hard truncation of individual messages (50KB limit)
- `enforce_limits()` — Standard compression at threshold
- `enforce_limits_with_summarization()` — Tiered compression with LLM summarization
- `drop_oldest()` — No-LLM fallback: drops old messages, preserves key fragments in summary
- `compress()` — Full LLM-based context compression
- `allocate_budget()` — Distributes token budget across parallel agents

**Tiered Compression:**
1. **Tier 1 (40% threshold)** — `drop_oldest()` every 5 iterations (proactive)
2. **Tier 2 (compression_threshold)** — Full summarization if models configured, else drop_oldest
3. **Tier 3 (hard_limit)** — Aggressive summarization or truncation

**`ContextIsolation`:**
- `Full` — Shares complete context
- `SystemOnly` — Only system prompt
- `Summary` — System prompt + compressed summary
- `Isolated` — Clean slate

**`ContextSummarizationConfig`:**
- Model priority list for summarization
- Ollama URL for local models
- Summarizer context window size

### CDC Deduplication (`chunker.rs`)

**FastCDC Algorithm:**
- Gear rolling hash with random lookup table
- Content-defined chunk boundaries at ~2KB-32KB intervals
- `chunk_and_hash()` — Returns set of chunk hashes for content
- `dedup_coverage()` — Calculates overlap ratio between two hash sets
- 70% overlap threshold triggers deduplication

This handles:
- Same file content split across different message boundaries
- Minor edits to previously seen content
- Reordered content blocks

### Thinking Mode (`loop_runner.rs`)

**`ThinkingMode`:**
- `Instant` — No extended thinking
- `Low` — 1,024 token budget
- `Medium` — 4,096 token budget
- `High` — 16,384 token budget
- `Maximum` — 32,768 token budget

**`ReasoningContent`** / **`ReasoningBlock`:**
- Captures thinking content before tool calls
- `AgentResponse.reasoning` stores the full reasoning chain
- Interleaved reasoning: thinking blocks appear between tool calls

### Token Budget Tracking

**`RunOptions`:**
- `token_budget` — Maximum total tokens for the run
- `budget_awareness` — Inject budget note into context
- Cumulative tracking: `cumulative_input_tokens`, `cumulative_output_tokens`
- Warnings at 80%, hard stop at 100%

### Task Delegation Tool

**`AgentSpawner` trait** (in nanna-tools):
- `spawn()` — Spawn sub-agent with isolated context
- Implemented in nanna-daemon's `server.rs`
- Sub-agent gets fresh context (system prompt + workspace only)
- 5-minute timeout, max 25 iterations
- Returns text + usage metadata

### Code Analysis Tools

Token-efficient codebase understanding:
- `code_outline` — Function signatures, struct/enum/trait defs (~5-20% of file size)
- `code_search` — Regex search with context lines across files
- `project_structure` — Directory tree with file sizes and line counts

## Issues & Suggestions

### Swarm Decomposition Quality

The `decompose_task()` method relies on LLM to produce a JSON decomposition. If the LLM returns malformed JSON or poor subtask boundaries, the swarm fails or produces suboptimal results.

**Suggestion:**
- Add structured output validation with retry on malformed JSON
- Implement decomposition templates for common task types (research, code review, data analysis)
- Add a feedback loop: if swarm results are poor, re-decompose with different strategy

### No Swarm Visualization in GUI

The swarm coordinator tracks `CriticalPathMetrics` but there's no way to visualize swarm execution in the GUI — parallel lanes, task dependencies, timing.

**Suggestion:**
- Add a swarm execution view showing parallel lanes with Gantt-chart style visualization
- Show real-time progress of each subtask
- Display critical path highlighting

### CDC Dedup Threshold is Fixed

The 70% overlap threshold (`DEDUP_THRESHOLD: f32 = 0.7`) is hardcoded. Different content types may benefit from different thresholds.

**Suggestion:**
- Make the threshold configurable per content type
- Lower threshold for code (where small changes matter)
- Higher threshold for natural language (where paraphrasing is common)

### Summarization Cache is In-Memory Only

The `SummaryCache` (LRU, 100 entries) is lost on restart. Long sessions that restart lose all cached summaries.

**Suggestion:**
- Persist summary cache to disk alongside session data
- Include cache hit/miss metrics for optimization

### Context Budget Allocation is Linear

`allocate_budget()` distributes tokens evenly across agents with a slight bonus for earlier agents. This doesn't account for task complexity.

**Suggestion:**
- Weight budget allocation by estimated task complexity
- Allow sub-agents to request more budget if they're running low
- Implement budget stealing: idle agents donate remaining budget to active ones

### Agent Message Queue is In-Memory

Inter-agent messages (`send_message()` / `check_mailbox()`) use in-memory `Vec<AgentMessage>`. Messages are lost on crash.

**Suggestion:**
- Persist messages to SQLite for crash recovery
- Add message acknowledgment
- Implement request/response correlation IDs

### Proactive Compression May Drop Important Context

`drop_oldest()` preserves "key fragments" in the consolidated summary, but the heuristic for what's "key" is basic (first few words of each dropped message).

**Suggestion:**
- Use LLM to score message importance before dropping
- Preserve messages with high information density (tool results, decisions)
- Allow users to "pin" messages that should never be compressed

## Potential Enhancements

1. **Adaptive concurrency** — Auto-tune `max_parallel` based on API rate limits and response times
2. **Swarm templates** — Pre-built swarm configurations for common workflows
3. **Cross-session swarms** — Swarm agents that persist across conversations
4. **Streaming swarm results** — Show partial results as subtasks complete
5. **Swarm cost estimation** — Predict token usage before executing swarm
6. **Context compression metrics** — Track compression ratio, information loss estimates
7. **Hierarchical summarization** — Summarize summaries for very long sessions
