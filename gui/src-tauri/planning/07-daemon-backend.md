# 07 — Daemon/Backend Architecture

## Feature Description

Nanna operates in two modes: **daemon mode** (connecting to a separate `nanna-daemon` process via WebSocket) and **embedded mode** (running the agent directly in the GUI process). The `Backend` abstraction layer makes this transparent to the rest of the application — commands route through the backend, which delegates to whichever mode is active.

### Architecture
```
┌─────────────────┐
│   Tauri Frontend │
│   (Nuxt/Vue)     │
└───────┬─────────┘
        │ IPC Commands
┌───────▼─────────┐
│   lib.rs         │
│   (Tauri cmds)   │
└───────┬─────────┘
        │
┌───────▼─────────┐
│   Backend        │──── mode? ────┐
└───────┬─────────┘                │
        │                          │
   ┌────▼────┐              ┌──────▼──────┐
   │ Embedded │              │ DaemonClient │
   │ Backend  │              │ (WebSocket)  │
   └──────────┘              └──────┬──────┘
                                    │
                             ┌──────▼──────┐
                             │ nanna-daemon │
                             │ (sidecar)    │
                             └─────────────┘
```

### Mode Selection
1. On startup, `Backend::init()` tries to connect to daemon
2. If daemon is running → daemon mode
3. If not → start daemon sidecar via Tauri shell plugin
4. If sidecar fails → embedded mode (fallback)
5. Mode can change at runtime if daemon disconnects/reconnects

## Current Implementation

### Backend (backend.rs)
- `BackendMode` enum: `Daemon`, `Embedded`, `Initializing`
- Holds `Arc<RwLock<Option<DaemonClient>>>` and `Arc<RwLock<Option<EmbeddedBackend>>>`
- `init()`: Attempts daemon connection, falls back to embedded
- ~50 proxy methods that delegate to daemon client or return "not in daemon mode" error
- Event forwarding: subscribes to daemon events, re-emits as Tauri events

### DaemonClient (daemon_client.rs)
- WebSocket connection to `ws://localhost:{port}`
- Request/response protocol with JSON-RPC-like structure (id, action, result)
- `PendingRequest` map: correlates request IDs to oneshot response channels
- Reconnection loop: exponential backoff (1s → 2s → 4s → 8s → 16s → 30s max)
- Event subscription: `broadcast::channel` for daemon events
- Connection states: `Connected`, `Connecting`, `Disconnected`, `Reconnecting`

### DaemonManager (daemon_manager.rs)
- Manages the daemon sidecar process lifecycle
- `start()`: Spawns `nanna-daemon` binary via Tauri shell plugin
- `wait_for_ready()`: Polls health endpoint until daemon responds (10 attempts, 500ms interval)
- `stop()`: Kills the sidecar process
- `restart()`: Stop + start
- Health monitor: periodic health checks, auto-restart on failure

### EmbeddedBackend (embedded.rs)
- Direct agent execution without daemon
- Creates `Agent` from `nanna-agent` with config, tools, memory
- `chat()`: Runs agent with `RunOptions`, streams events
- Limited feature set compared to daemon (no channels, no multi-agent, no distributed state)
- Session/memory CRUD delegates to local storage/memory service

### Event Forwarding (backend.rs ~208)
- Subscribes to `DaemonEvent` broadcast channel
- Maps daemon events to Tauri events:
  - `StreamChunk` → `stream-chunk`
  - `ToolCall` → `tool-call`
  - `SessionUpdate` → `session-update`
  - `MemoryUpdate` → `memory-update`
  - `AgentEvent` → `agent-event`
  - `SchedulerEvent` → `scheduler-event`

## Issues & Bugs

### Critical
1. **Embedded mode is severely limited**: Many features only work in daemon mode — channels, multi-agent, scheduler execution, workspace management via daemon. The embedded backend implements a subset: chat, sessions, memory, tools, system status. But the Tauri commands still expose all features, returning "not in daemon mode" errors for unsupported operations. Users get a degraded experience with no clear indication of what's available.

2. **No graceful mode transition**: When the daemon disconnects, the backend doesn't automatically fall back to embedded mode for in-flight operations. Pending requests fail with connection errors. There's no queuing or retry-on-reconnect.

3. **WebSocket message ordering**: The daemon client processes messages sequentially in the handler loop. If a response arrives before the pending request is registered (race between `send` and `insert`), the response is dropped. The current code inserts the pending request before sending, which is correct, but there's no timeout cleanup for pending requests that never get a response.

### Moderate
4. **Reconnection loop runs forever**: `start_reconnect_loop` has no maximum retry count. If the daemon is permanently dead, the client retries indefinitely with 30s max backoff. This wastes resources and never triggers a fallback to embedded mode.

5. **Health monitor restart is aggressive**: If a health check fails, the monitor immediately tries to restart the daemon. No grace period for transient failures. A single missed health check could trigger unnecessary restarts.

6. **Proxy method duplication**: `Backend` has ~50 methods that are nearly identical: check if daemon client exists, call the corresponding method, return error if not. This is massive boilerplate. A macro or dynamic dispatch would reduce this significantly.

7. **No request timeout**: `DaemonClient::request()` waits indefinitely for a response via the oneshot channel. If the daemon hangs or the response is lost, the caller blocks forever. Should use `tokio::time::timeout`.

8. **Sidecar port is hardcoded**: `DaemonManagerConfig::default()` uses port 9833. No port conflict detection. If another process uses that port, the daemon fails to start with an unhelpful error.

### Minor
9. **Event forwarding drops events on slow consumers**: The broadcast channel has a fixed capacity. If the Tauri event system is slow, events are dropped. No backpressure mechanism.

10. **No daemon version check**: The GUI doesn't verify that the daemon binary version matches. Version mismatches could cause protocol incompatibilities.

11. **`ConnectionMode::Embedded` is set but embedded backend may not be initialized**: The mode enum has `Embedded` but the `EmbeddedBackend` is set separately. There's a window where mode is `Embedded` but the backend isn't ready.

## Improvement Suggestions

### High Priority
- **Request timeouts**: Add configurable timeout (default 30s) to `DaemonClient::request()`. Clean up pending requests on timeout.
- **Reconnection limit with fallback**: After N reconnection attempts (e.g., 10), switch to embedded mode. Periodically try to reconnect in background.
- **Feature availability API**: Expose a `get_available_features` command that returns which features work in the current mode. Frontend can adapt UI accordingly.
- **Dynamic port selection**: Try configured port, if busy try port+1, port+2, etc. Or use port 0 and let OS assign, then communicate via stdout.

### Medium Priority
- **Reduce proxy boilerplate**: Use a macro like `daemon_proxy!(method_name, action_type, params...)` to generate the ~50 proxy methods.
- **Graceful mode transition**: On daemon disconnect, queue pending operations. When reconnected, replay. If timeout expires, fall back to embedded for queued operations.
- **Health check grace period**: Require 3 consecutive failed health checks before triggering restart. Configurable.
- **Daemon version handshake**: On connection, exchange version info. Warn or refuse if incompatible.

### Future
- **Multi-daemon support**: Connect to multiple daemon instances for load balancing or specialized workloads.
- **Remote daemon**: Support connecting to a daemon on a different machine (currently localhost only). Would need authentication/TLS.
- **Daemon auto-update**: When GUI updates, check if daemon binary needs updating. Bundle and replace.
- **Protocol versioning**: Add version field to request/response protocol. Support backward compatibility.
- **Embedded mode parity**: Gradually bring embedded mode to feature parity with daemon. This reduces the criticality of daemon availability.