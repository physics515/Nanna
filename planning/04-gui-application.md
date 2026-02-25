# Phase 4: GUI Application

**Status:** ✅ Complete
**Stack:** Tauri v2 + Nuxt v4 + Tailwind v4

## Overview

The GUI is a desktop application built with Tauri v2 (Rust backend) and Nuxt v4 (Vue frontend). It serves as the richest rendering channel for Nanna — but architecturally, it's "just another channel" connecting to the control plane.

## Architecture

```
gui/
├── app/                    # Nuxt v4 frontend
│   ├── components/         # 35+ Vue components
│   ├── composables/        # useBackend, useSessionState, useNotifications, etc.
│   ├── layouts/default.vue # Main layout with sidebar (646 lines)
│   ├── pages/              # 8 pages: index, settings, memory, channels, agents, scheduler, tools, workspaces
│   └── plugins/            # Monaco editor plugin
├── src-tauri/
│   ├── src/
│   │   ├── lib.rs          # Main Tauri app (8163 lines — THE monolith)
│   │   ├── backend.rs      # Backend abstraction (daemon vs embedded)
│   │   ├── daemon_client.rs # WebSocket client to daemon
│   │   ├── daemon_manager.rs # Auto-start/connect daemon
│   │   ├── embedded.rs     # Embedded fallback (no daemon)
│   │   ├── agents.rs       # Agent management commands
│   │   └── tool_authoring.rs # User tool CRUD
│   └── tauri.conf.json
```

## Current Implementation

### Frontend (Nuxt v4)

**Pages:**
- `index.vue` — Chat interface with streaming, tool call cards, markdown rendering, thinking/reasoning display
- `settings.vue` (1483 lines) — Tabbed settings: Models, Agent, Memory, Tools, Scheduler, Data
- `memory.vue` (657 lines) — Memory browser with search, stats, CRUD
- `channels.vue` (282 lines) — Channel dashboard with setup wizards
- `agents.vue` (481 lines) — Agent registry and management
- `scheduler.vue` (493 lines) — Cron job editor with visual builder
- `tools.vue` (692 lines) — Tool browser and user tool authoring
- `workspaces.vue` (801 lines) — Workspace management with file editing

**Components:**
- `ChatInput.vue` (340 lines) — Message input with attachment support
- `ToolCallCard.vue` (362 lines) — Rich tool call visualization with icons, progress, collapsible I/O
- `MarkdownContent.vue` (115 lines) — Markdown rendering for responses
- `MonacoBlock.vue` (236 lines) — Monaco editor for code blocks
- `SessionItem.vue` (215 lines) — Session sidebar item
- `ConnectionStatus.vue` (182 lines) — Backend connection indicator
- `BackendStatus.vue` (162 lines) — Daemon/embedded mode status
- Channel setup wizards: `TelegramSetup.vue`, `DiscordSetup.vue`, `SlackSetup.vue`, `SignalSetup.vue`, `WhatsAppSetup.vue`
- UI primitives: button, card, input, select, switch, badge, modal, sheet, spinner, tabs, tooltip, textarea, avatar

**Composables:**
- `useBackend.ts` — Backend mode detection and initialization
- `useSessionState.ts` — Session state management (streaming, thinking, tool calls, message queue)
- `useCloseHandler.ts` — Window close behavior (minimize to tray vs quit)
- `useNotifications.ts` — Native notification handling
- `useConfirm.ts` — Confirmation dialogs

### Backend (Tauri/Rust)

**`lib.rs` (8163 lines)** — This is the central issue. It contains:
- All Tauri command handlers (~100+ `#[tauri::command]` functions)
- LLM client creation and model routing
- Token estimation and context truncation logic
- Tool result summarization
- Memory service adapters
- Agent loop execution (both embedded and daemon modes)
- Session management
- Channel configuration
- Workspace management
- System tray setup
- App state initialization

**`backend.rs` (753 lines)** — Abstracts daemon vs embedded mode:
- `BackendMode::Daemon` — Routes through WebSocket to nanna-daemon
- `BackendMode::Embedded` — Runs services in-process
- Auto-detection and fallback

**`daemon_client.rs` (1209 lines)** — WebSocket client:
- Connects to daemon on port 5149
- Request/response correlation
- Event subscription (streaming, tool calls)
- Auto-reconnection

## Recent Completions (2026-02-09)

### Chat Persistence & Reliability
- **Tool calls persisted in session history** — `SessionMessage` stores `tool_calls: Vec<ToolCallRecord>` and `reasoning: Option<String>` via `add_full_message()`. Survive reload.
- **Thinking/reasoning persisted** — `reasoning` field on `SessionMessage`, displayed as collapsible `<details>` block in chat UI
- **ThinkingDelta event pipeline** — Agent `on_thinking` callback → `Event::ThinkingDelta` → WebSocket → `thinking-chunk` Tauri event → Vue `streamingThinking` composable → live indicator during streaming
- **ToolStart event pipeline** — Agent `on_tool_start` callback → `Event::ToolStart` → WebSocket → `tool-started` Tauri event → active tool tracking in run state
- **Empty content block prevention** — Tool results never empty (`[No output]` fallback), `sanitize_messages()` removes empty text blocks, `max_block_chars` floored to 100
- **GUI history parsing fixed** — Reads `tool_calls` and `reasoning` from top-level session message fields (was incorrectly reading from `metadata.tool_calls`)

### Token Optimization
- **Smart memory recall gating** — Only recalls memories for messages with >5 words, contains `?`, or >80 chars. Saves ~500-2000 tokens per trivial exchange.
- **Memory dedup** — Skips injecting recalled memories whose content already appears in the last 4 messages of conversation history.

### Model Fallback
- **LLM Router** — Multi-provider routing with automatic model→provider detection (Anthropic, OpenAI, OpenRouter, GitHub Models, Ollama)
- **Ollama tag notation** — Models with `:` (e.g. `deepseek-r1:14b`) auto-detected as Ollama
- **Stateless fallback** — Each `chat()` call rebuilds model list from `model_priority` config. No persistent "last successful model" state — heartbeats always start from top of priority list.
- **Diagnostic logging** — Logs detected provider and available providers when a model can't be routed

## Issues & Suggestions

### Critical: `lib.rs` is 8163 lines

This is the most pressing architectural issue. A single 8163-line file containing all Tauri commands, business logic, LLM routing, and state management.

**Suggestion:** Split into modules:
```
src/
├── lib.rs              # App setup, state, plugin registration
├── commands/
│   ├── chat.rs         # send_message, run_agent_loop
│   ├── sessions.rs     # create/list/delete/rename sessions
│   ├── memory.rs       # search/list/delete/update memories
│   ├── settings.rs     # get/set config, API keys, models
│   ├── channels.rs     # channel config, status, testing
│   ├── workspaces.rs   # workspace CRUD, file editing
│   ├── scheduler.rs    # cron jobs
│   ├── tools.rs        # tool listing, user tools
│   └── system.rs       # notifications, tray, window management
├── llm/
│   ├── routing.rs      # Model selection, client creation
│   ├── truncation.rs   # Context truncation, token estimation
│   └── summarization.rs # Tool result summarization
├── state.rs            # AppState, EmbeddedRunState
├── backend.rs          # (existing)
├── daemon_client.rs    # (existing)
├── daemon_manager.rs   # (existing)
└── embedded.rs         # (existing)
```

### Duplicated Logic Between Embedded and Daemon Modes

The `send_message` function in `lib.rs` has two completely separate code paths — one for daemon mode (~60 lines) and one for embedded mode (~280 lines). The embedded path duplicates significant logic that exists in the daemon's `AgentService`.

**Suggestion:** Extract the agent loop logic into a shared crate or ensure the embedded path delegates to the same `AgentService` the daemon uses. The `run_agent_loop` and `run_agent_loop_with_fallback` functions (combined ~500 lines) should be unified.

### Context Truncation Logic Duplicated

Token estimation, tool budget allocation, and context truncation exist in both `lib.rs` (for embedded mode) and `nanna-agent/src/context.rs` (for daemon mode). The embedded mode has its own `truncate_context`, `allocate_tool_budgets`, `smart_truncate_tool_result` functions that partially overlap with `AgentContext` methods.

**Suggestion:** Remove the truncation logic from `lib.rs` entirely. The `AgentContext` in nanna-agent should be the single source of truth for context management. The embedded mode should use `AgentContext` directly.

### Settings Page is 1483 Lines

The settings page handles Models, Agent, Memory, Tools, Scheduler, and Data tabs all in one component.

**Suggestion:** Split into sub-components per tab: `SettingsModels.vue`, `SettingsAgent.vue`, `SettingsMemory.vue`, etc.

### No Error Boundary Components

Frontend has no global error boundary. If a component throws, the whole page may break.

**Suggestion:** Add Vue error boundary components and a global error handler that shows a recovery UI.

### Monaco Plugin Loaded Globally

The Monaco editor plugin (`monaco.client.ts`, 107 lines) is loaded as a client plugin for all pages, but it's only needed on the chat page and tools page.

**Suggestion:** Lazy-load Monaco only when needed via dynamic imports.

### Missing Accessibility

No ARIA labels on interactive elements, no keyboard navigation support, no screen reader considerations.

**Suggestion:** Add ARIA labels to all UI components, implement keyboard shortcuts, test with screen readers.

### Theme Consistency

The Palenight theme is defined in `tailwind.config.ts` and `main.css` but some components use hardcoded color values instead of theme tokens.

**Suggestion:** Audit all components for hardcoded colors and replace with Tailwind theme classes.

### Mobile Responsiveness

The layout has mobile-responsive classes but hasn't been tested on actual mobile devices via Tauri mobile builds.

**Suggestion:** Test on Android/iOS simulators. The sidebar likely needs a hamburger menu on mobile.

## Potential Enhancements

1. **Keyboard shortcuts** — Cmd/Ctrl+K for quick actions, Cmd/Ctrl+N for new chat, etc.
2. **Drag-and-drop file upload** — Drop files into chat to attach
3. **Split view** — View two sessions side by side
4. **Search across sessions** — Full-text search through all conversation history
5. **Export conversations** — Export as Markdown, PDF, or JSON
6. **Theme customization** — Let users adjust accent colors
7. **Font size controls** — Accessibility feature
8. **Command palette** — Slash commands or Cmd+K palette for quick actions
