# GUI Wiring Analysis: Daemon vs Embedded Mode

**Last Updated:** 2026-02-03

## Recent Fixes (2026-02-03)

### Daemon ControlPlane Implementations Added:
- ✅ `MemoryAction::Get` - Now retrieves memory by ID with full metadata
- ✅ `MemoryAction::Update` - Now updates memory content
- ✅ `MemoryAction::Consolidate` - Now triggers consolidation with LLM summarization

### Infrastructure Changes:
- Added `with_all_services()` constructor to ControlPlane that accepts LLM client
- Modified `init_services()` to return the LLM client for consolidation
- Added `RequestBuilder` trait import for proper CompletionRequest building

---

## Overview

The GUI supports two modes:
- **Daemon Mode**: Routes requests through WebSocket to `nanna-daemon`
- **Embedded Mode**: Runs the agent directly in the GUI process (fallback)

This document analyzes what's properly wired for both modes and what's missing.

---

## ✅ Properly Wired (Both Modes Work)

### Chat
| Command | Daemon Route | Embedded Fallback |
|---------|--------------|-------------------|
| `send_message` | ✅ `backend.chat_send()` | ✅ Full agent loop |

### Sessions
| Command | Daemon Route | Embedded Fallback |
|---------|--------------|-------------------|
| `create_session` | ✅ `backend.session_create()` | ✅ SQLite |
| `list_sessions` | ✅ `backend.sessions_list()` + SQLite merge | ✅ SQLite |
| `get_session_history` | ✅ `backend.session_history()` | ✅ SQLite fallback |
| `delete_session` | ✅ `backend.session_delete()` | ✅ SQLite fallback |
| `rename_session` | ✅ `backend.session_rename()` | ✅ SQLite fallback |

### Memory (FSRS Cognitive Memory)
| Command | Daemon Route | Embedded Fallback |
|---------|--------------|-------------------|
| `get_cognitive_memory_stats` | ✅ `backend.memory_stats()` | ✅ Local MemoryService |
| `trigger_consolidation` | ✅ `backend.memory_consolidate()` | ✅ Local MemoryService |
| `list_memories` | ✅ `backend.memory_search()` | ✅ Local MemoryService |
| `get_memory` | ✅ `backend.memory_get()` | ✅ Local MemoryService |
| `delete_memory` | ✅ `backend.memory_delete()` | ✅ Local MemoryService |
| `update_memory` | ✅ `backend.memory_update()` | ✅ Local MemoryService |

---

## ⚠️ Partial / Inconsistent Wiring

### Backend Has Methods But GUI Doesn't Use Them

The `Backend` struct has these methods implemented but GUI commands don't call them:

**Scheduler Operations** (Backend has them, GUI ignores them):
- `backend.scheduler_list()`
- `backend.scheduler_get()`
- `backend.scheduler_add()`
- `backend.scheduler_update()`
- `backend.scheduler_remove()`
- `backend.scheduler_run_now()`
- `backend.scheduler_history()`

**Tool Operations** (Backend has them, GUI ignores them):
- `backend.tool_list()` - Not used, GUI reads tools from local ToolRegistry
- `backend.tool_execute()` - Not used

**Session Operations**:
- `backend.session_clear()` - Backend has it, GUI doesn't use it

### Daemon Protocol Status (Updated 2026-02-03)

| Action | Status |
|--------|--------|
| `Chat::*` | ✅ Working |
| `Session::*` | ✅ Working |
| `Memory::*` | ✅ All working (Search, Get, Create, Update, Delete, Stats, Consolidate) |
| `Config::*` | ✅ **FIXED** - Get, Set, Reset, Reload, Export, Import |
| `Scheduler::*` | ✅ **FIXED** - List, Get, Add, Update, Remove, RunNow, History |
| `Channel::*` | 🔄 Partial - List, Status work. Enable/Disable/Test/Send need ChannelManager |
| `Workspace::*` | ✅ **NEW** - List, Get, Open, Close, SetActive, ClearActive, Reload, GetContext, UpdateContext |
| `Tool::List/Get/Execute` | ✅ Working |
| `Tool::Create/Update/Delete` | ✅ **FIXED** - UserToolManager moved to daemon |
| `Tool::Test/ListUser` | ✅ **NEW** - Test tools and list user tools |

---

## ❌ Embedded Only (No Daemon Route)

These commands only work in embedded mode:

### Memory (Session-based Search)
| Command | Status |
|---------|--------|
| `search_memory` | Embedded only - searches SQLite messages |
| `get_memory_stats` | Embedded only - counts SQLite messages |
| `apply_memory_updates` | Embedded only - FSRS updates |
| `save_memories` | Embedded only - saves to JSON |
| `clear_all_memories` | Embedded only - no daemon `Memory::Clear` action |

### Workspaces
All workspace commands are embedded-only (daemon has no workspace support):
- `list_workspaces`
- `open_workspace`
- `close_workspace`
- `set_active_workspace`
- `discover_workspaces`
- `reload_workspace_context`

### Settings/Config
All settings are embedded-only (intentional - config is GUI-local):
- `get_config`
- `set_model`
- `set_api_key`
- `set_provider`
- `get_extended_settings`
- All `set_*` settings commands

### User Tool Authoring
Embedded only:
- `list_user_tools`
- `create_user_tool`
- `update_user_tool`
- `delete_user_tool`

---

## Implementation Priorities

### High Priority: Wire GUI to Use Backend Methods

1. **Clear Session** - Wire `clear_session` GUI command to use `backend.session_clear()` when in daemon mode

2. **Tool List** - Consider routing `get_config` tools through daemon when available

### Medium Priority: Complete Daemon ControlPlane

These need implementation in `nanna-daemon/src/control.rs`:

1. **Memory::Consolidate** - Trigger consolidation
2. **Memory::Get** - Get memory by ID
3. **Memory::Update** - Update memory content
4. **Scheduler::Add/Update/Remove** - Full scheduler support

### High Priority: Move To Daemon (Currently Misplaced in GUI)

**Workspaces** - MUST be daemon concept:
- Memory scoping depends on workspaces (daemon needs to know context)
- Multi-device swarm requires shared workspace awareness across nodes
- Sessions are already workspace-scoped, but workspace registry is GUI-only
- **Action:** Add `WorkspaceAction` to daemon protocol, move `WorkspaceRegistry` to daemon

**Settings/Config** - SHOULD be daemon-managed:
- Daemon needs same config as GUI to function independently
- Multiple GUIs/CLIs should see consistent configuration
- **Action:** Implement `ConfigAction` handlers in daemon, GUI reads/writes through IPC

**API Keys / Credentials** - SHOULD be shared keyring:
- Daemon needs API keys to run headless (heartbeats, channel listeners)
- Multiple clients need same credentials without duplication
- **Action:** Create shared credential store (OS keyring) accessible by daemon + GUI

### High Priority: Move To Daemon (Currently Misplaced in GUI) - Continued

**User Tool Authoring** - SHOULD be daemon concept:
- Tools created should be available to all clients (GUI, CLI, API, channels)
- Daemon needs tool registry for headless operation
- Multi-device swarm needs shared tools
- **Action:** Add tool creation to `ToolAction` (Create/Update/Delete), move `UserToolManager` to daemon

### Low Priority: Truly GUI-Only Features

These can remain embedded-only:
- **UI-specific settings**: Theme, window position, close behavior
- **Notifications**: Platform-specific (Tauri-only)

---

## Target Architecture

### Daemon as Single Source of Truth

```
┌─────────────────────────────────────────────────────────────┐
│                        DAEMON (nanna-daemon)                 │
│  ┌──────────┬──────────┬──────────┬──────────┬──────────┐  │
│  │ Sessions │ Memory   │ Config   │ Tools    │ Scheduler│  │
│  │ Manager  │ Service  │ Manager  │ Registry │ /Cron    │  │
│  └──────────┴──────────┴──────────┴──────────┴──────────┘  │
│  ┌──────────┬──────────┬──────────┐                        │
│  │Workspaces│ Keyring  │ Channels │                        │
│  │ Registry │ Access   │ Manager  │                        │
│  └──────────┴──────────┴──────────┘                        │
│                            ▲                                │
│                            │ IPC (WebSocket)                │
│  ┌─────────────────────────┴───────────────────────────┐   │
│  │                  Channel Router                      │   │
│  └──┬────────┬────────┬────────┬────────┬────────┬─────┘   │
└─────┼────────┼────────┼────────┼────────┼────────┼─────────┘
      ▼        ▼        ▼        ▼        ▼        ▼
  Telegram  Discord   GUI    CLI     API    Slack
```

### Key Principles

1. **Daemon owns all state**: Sessions, memories, config, workspaces
2. **GUI is just another channel**: Full control plane access via IPC
3. **Shared credentials**: OS keyring accessible by daemon + all clients
4. **Workspace-scoped memory**: Daemon tracks which memories belong to which workspace
5. **Multiple GUIs supported**: Phone + desktop can attach to same daemon

### Protocol Extensions Needed

```rust
// New: WorkspaceAction
pub enum WorkspaceAction {
    List,
    Open { path: String },
    Close { id: String },
    SetActive { id: String },
    GetContext { id: String },
    Reload { id: String },
}

// New: KeyringAction (or integrate into Config)
pub enum KeyringAction {
    Get { provider: String },
    Set { provider: String, credential: String },
    Delete { provider: String },
    List,
}
```

---

## Recommended Changes

### 1. Backend Abstraction Improvements

Add missing convenience methods to `Backend`:
- `session_clear()` - Already exists, verify GUI uses it
- `tool_list()` - Already exists, verify GUI uses it

### 2. GUI Command Updates

Update these commands to check daemon mode first:

```rust
// Example pattern for commands that should use daemon when available
#[tauri::command]
async fn some_command(state: State<'_, Arc<RwLock<AppState>>>) -> Result<T, String> {
    let state_guard = state.read().await;
    
    // Try daemon first
    if state_guard.backend.is_daemon_mode().await {
        if let Ok(result) = state_guard.backend.daemon_method().await {
            return Ok(parse_result(result));
        }
        // Fall through to embedded on failure
    }
    
    // Embedded mode / fallback
    Ok(embedded_implementation())
}
```

### 3. Daemon ControlPlane Implementation

Complete these handlers in `control.rs`:
- `MemoryAction::Get` - Retrieve by ID
- `MemoryAction::Update` - Update content
- `MemoryAction::Consolidate` - Trigger consolidation
- `SchedulerAction::*` - Full scheduler support (integrate nanna-core Scheduler)

---

## Testing Checklist

After wiring changes, verify these scenarios:

**Daemon Mode:**
- [ ] Create session → daemon creates, GUI shows it
- [ ] Send message → streams through daemon
- [ ] Session history → fetched from daemon
- [ ] Memory recall → daemon searches, returns results
- [ ] Memory management → create/delete/update via daemon
- [ ] Memory consolidation → daemon runs LLM summarization
- [ ] Workspaces → daemon manages registry, GUI displays
- [ ] Config changes → daemon persists, all clients see updates
- [ ] Scheduler → create/update/delete jobs via daemon
- [ ] User tools → create/test/update/delete tools via daemon
- [ ] Channel list → daemon reports configured channels

**Embedded Mode (daemon unavailable):**
- [ ] Create session → SQLite creates, GUI shows it
- [ ] Send message → embedded agent loop runs
- [ ] Session history → fetched from SQLite
- [ ] Memory recall → local MemoryService searches
- [ ] All features work without daemon

**Transition:**
- [ ] Start GUI with daemon → daemon mode active
- [ ] Stop daemon → GUI falls back to embedded
- [ ] Restart daemon → GUI reconnects automatically
- [ ] Workspaces open in GUI → daemon picks them up on reconnect

**Multi-Client:**
- [ ] Two GUIs connect to same daemon → both see same sessions
- [ ] Config change in GUI A → GUI B sees update
- [ ] Memory created in CLI → GUI shows it
- [ ] User tool created in CLI → GUI can use it
