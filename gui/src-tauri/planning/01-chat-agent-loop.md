# 01 — Chat & Agent Loop

## Feature Description

The chat system is Nanna's core interaction loop. A user sends a message, the system builds context (history, memories, workspace), streams the LLM response, executes tool calls in parallel, and iterates until the model stops or hits the 10-iteration cap. Results are stored in SQLite and memories are extracted in the background.

### Flow
1. `send_message` → routes to daemon or embedded mode
2. User message stored in SQLite
3. Conversation history retrieved and truncated to token budget
4. Memory recall (FSRS-6 semantic search, workspace-scoped)
5. Workspace context injected into system prompt
6. `run_agent_loop_with_fallback` → tries models in priority order
7. `run_agent_loop` → streams tokens, executes tools, loops up to 10 iterations
8. Assistant response stored with tool call metadata
9. Background: `extract_and_store_memories` runs asynchronously

### Key Structures
- `StreamChunk` — emitted to frontend via `stream-chunk` event (session_id, chunk, done, tool_call)
- `ToolCallEvent` — emitted via `tool-call` event (session_id, tool_name, status, result)
- `PendingToolCall` — tracks in-flight tool executions (name, input, timestamp)
- `EmbeddedRunState` — recovery state for streaming (accumulated_text, active/completed tool calls)

## Current Implementation

### `send_message` (lib.rs ~1233)
- Dual-mode routing: daemon mode delegates to `send_message_daemon`, embedded runs locally
- In embedded mode: acquires read lock on AppState, builds full request, runs agent loop
- Stores user message before processing, assistant message after completion
- Memory recall scoped: workspace sessions get global + workspace memories; global sessions get all

### `run_agent_loop_with_fallback` (lib.rs ~1667)
- Iterates through `model_priority` list from config
- Skips rate-limited models (checks cooldown timestamps)
- Pre-flight: estimates tokens vs model limits via `ModelLimits`
- Preferred model (first in list): up to 3 retries with progressive backoff (15s, 30s, 45s)
- Fallback models: 1 attempt each
- Emits `model-status` events to frontend on switches

### `run_agent_loop` (lib.rs ~1832)
- Builds `CompletionRequest` with system prompt, messages, tools, streaming enabled
- Streams via `futures::StreamExt`
- Handles events: TextDelta, ToolUse (start/delta/end), MessageStop, Error
- Parallel tool execution: spawns all tool calls concurrently, collects results
- Tool results fed back as new messages for next iteration
- Max 10 iterations (hardcoded)
- Tracks run state in `embedded_run_states` for frontend recovery

### `send_message_daemon` (lib.rs ~1169)
- Delegates to `backend.chat_send(session_id, content)`
- Daemon handles all context building, tool execution, streaming
- Events forwarded from daemon WebSocket to frontend via event forwarding

## Issues & Bugs

### Critical
1. **Race condition on AppState**: `send_message` acquires a read lock for the entire duration of the agent loop. While Tokio's RwLock allows concurrent reads, any write operation (config changes, model switches) will block until the agent loop completes. Long-running tool executions can hold this lock for minutes.

2. **No cancellation mechanism**: Once `send_message` starts in embedded mode, there's no way to cancel it. The frontend can't abort a running agent loop. The `EmbeddedRunState` tracks state but provides no abort handle.

3. **10-iteration cap is hardcoded**: No configuration option. Complex multi-step tasks may need more iterations; simple queries waste resources on unnecessary iterations.

### Moderate
4. **Memory extraction runs on every message**: Even trivial messages ("ok", "thanks") trigger the full extraction pipeline with an LLM call. No filtering or minimum-length threshold.

5. **Tool execution has no timeout at the loop level**: Individual tools may have timeouts, but the overall agent loop has no wall-clock timeout. A tool that hangs indefinitely blocks the session.

6. **Duplicate message storage**: In embedded mode, the user message is stored before the agent loop, and the assistant message after. If the agent loop fails partway through, the user message is stored but the assistant message is not, leaving an orphaned user message.

7. **Session run state cleanup**: `embedded_run_states` entries are inserted on start but only cleaned up on successful completion. Failed/panicked runs leave stale entries.

### Minor
8. **Streaming events are fire-and-forget**: `app.emit()` doesn't confirm delivery. If the frontend disconnects mid-stream, events are silently dropped.

9. **Tool call metadata in stored messages**: Tool calls are serialized into the message metadata as JSON. No schema versioning — format changes could break history display.

## Improvement Suggestions

### High Priority
- **Add cancellation support**: Store a `CancellationToken` (from `tokio_util`) in `EmbeddedRunState`. Check it between iterations and before tool execution. Expose a `cancel_message` Tauri command.
- **Make iteration cap configurable**: Add `max_agent_iterations` to config with a sensible default (10) and a hard ceiling (50).
- **Reduce lock scope**: Clone the needed data out of AppState at the start of `send_message`, then release the lock before entering the agent loop.

### Medium Priority
- **Filter trivial messages from extraction**: Skip memory extraction for messages under ~50 characters or that match common acknowledgment patterns.
- **Add agent loop timeout**: Configurable wall-clock timeout (default 5 minutes). Use `tokio::time::timeout` around the entire loop.
- **Implement proper error recovery**: On agent loop failure, store a partial assistant message with error metadata rather than leaving orphaned user messages.
- **Clean up stale run states**: Add a periodic cleanup task or clean on session load.

### Future
- **Multi-turn tool confirmation**: For dangerous tools (exec, write_file), optionally pause and ask the user for confirmation before executing.
- **Streaming tool results**: Instead of waiting for all tools to complete, stream individual tool results as they finish.
- **Parallel session support**: Allow multiple concurrent agent loops across different sessions (currently the RwLock serializes them).
