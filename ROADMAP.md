# Nanna Roadmap

## Phase 1: Core Infrastructure ✅
- [x] SIMD vector operations (AVX/AVX2)
- [x] GPU compute (wgpu)
- [x] SQLite persistence (Turso)
- [x] Vector store + conversation memory
- [x] LLM clients (Anthropic, OpenAI, OpenRouter)
- [x] Streaming + tool calling
- [x] Agent loop with context management
- [x] Scheduler (heartbeats, cron)

## Phase 2: Tools & Channels ✅
### Tools
- [x] File operations (read, write, list)
- [x] Shell execution
- [x] Web fetch/search
- [x] Memory (remember, recall, reflect)
- [x] Scheduling (remind, list, cancel)
- [x] Browser tools (screenshot, extract, action, evaluate)
- [x] Vision tools (analyze_image)
- [x] OCR tools (extract text from images, describe image contents)
- [x] Audio tools (TTS, transcription)
- [x] PDF tools (read text, extract images, analyze embedded images)
- [x] Authoring tools (runtime tool creation) — `nanna-scripting` crate
  - [x] Boa engine (pure Rust, lightweight)
  - [x] Deno engine (V8 fallback for full TS support)
    - [x] TypeScript transpilation via deno_ast
    - [x] V8 execution via deno_core 0.375

### Channels
- [x] Telegram (full API: send, react, edit, delete, pin, poll)
- [x] Discord (REST API: send, react, edit, delete, pin, threads)
- [x] Slack (Web API: send, react, edit, delete, pin, threads, files)
- [x] Signal (signald: send, react, groups)
- [x] WhatsApp (Cloud API: send, react, media, templates)

## Phase 3: Multi-Agent & MCP ✅
- [x] MCP client (external tool servers) — `nanna-mcp` crate
  - [x] JSON-RPC 2.0 protocol types
  - [x] Stdio transport (spawn processes)
  - [x] HTTP/SSE transport
  - [x] Tool discovery and execution
  - [x] Resource and prompt support
  - [x] Adapter for nanna-tools integration
- [x] MCP server mode (expose Nanna tools) — `server.rs`
  - [x] Stdio transport
  - [x] Tool/resource/prompt registration
  - [x] nanna-tools bridge
- [x] Background task spawning — `AgentCoordinator::spawn_task()`
- [x] Agent-to-agent communication — `send_message()` / `check_mailbox()`
- [x] Supervisor patterns — `nanna-agent/src/supervisor.rs`
  - [x] RestartPolicy (Never, Always, OnFailure, ExponentialBackoff)
  - [x] HealthCheckConfig (interval, timeout, thresholds, probe prompt)
  - [x] SupervisionStrategy (OneForOne, OneForAll, RestForOne)
  - [x] Supervisor with lifecycle management

## Phase 4: GUI Application ✅
**Stack:** Tauri v2 + Nuxt v4 + Tailwind v4

### Design
**Theme:** 80s hacker retro — Palenight-inspired using closest Tailwind defaults

| Element | Tailwind Color |
|---------|----------------|
| Background | `slate-900` / `slate-950` |
| Surface | `slate-800` |
| Primary | `violet-500` |
| Secondary | `indigo-400` |
| Accent | `cyan-400` |
| Text | `slate-200` |
| Muted | `slate-400` |
| Success | `emerald-400` |
| Warning | `amber-400` |
| Error | `rose-400` |

CRT glow effects, scanlines optional, monospace fonts (JetBrains Mono / Fira Code)

### Platforms (priority order)
1. **Desktop** (Windows, macOS, Linux) — primary target
2. **Android** — via Tauri mobile
3. **iOS** — via Tauri mobile

### Features
- [x] Chat interface with streaming
- [x] Markdown rendering for responses
- [x] Session management (create, switch, rename, delete)
- [x] Settings page (API key, model selection)
- [x] Tool call visualization (type icons, progress bars, collapsible I/O)
- [x] Memory browser (search across sessions, stats dashboard)
- [x] System tray / menu bar presence (show/new chat/quit menu)
- [x] Streaming UX polish (ConnectionStatus, MessageSkeleton, retry logic)
- [x] Channel status dashboard
- [x] Native notifications (wired up with permission handling)

### Technical
- [x] Tauri v2 setup with Rust backend
- [x] Nuxt v4 frontend (SSG mode for Tauri)
- [x] Tailwind v4 styling
- [x] IPC bridge to nanna-core
- [x] Streaming via Tauri events
- [x] Session persistence (SQLite)
- [x] System tray with tauri tray-icon feature
- [x] Notification plugin installed
- [x] UI component library (Button, Card, Input, Select, Switch, Badge, etc.)
- [x] Mobile-responsive layouts

### Memory Recall & Extraction ✅
- [x] **Separate embedding provider/model** - configure embeddings independently from chat (e.g., Opus for chat + Ollama for embeddings)
- [x] **Configurable extraction model** - GUI setting to select extraction model (empty = use chat model)
- [x] **Skip extraction without embeddings** - don't extract facts if recall won't work anyway
- [x] **FSRS feedback loop** - when memory is recalled, apply testing effect (update retrievability)
- [x] **Importance scoring** - LLM rates extracted fact importance (1-5) for FSRS weight
- [x] **Duplicate detection** - `smart_ingest()` checks similarity before storing (>0.92 = reinforce, >0.75 = update, <0.75 = create)
- [x] **Memory persistence** - load on startup, save on exit, auto-save after extraction
- [x] **Fact source tagging** - distinguish STATED (user said) vs OBSERVED (model inferred) facts
- [x] **Anti-confabulation prompting** - explicit instructions to not fabricate memories
- [x] **Similarity threshold tuning** - lowered min_score from 0.70 to 0.40 for semantic matching
- [x] **Memory management UI** - view, edit, delete individual memories in GUI ✅
- [x] **Configurable similarity threshold** - expose min_score in GUI settings

### Settings Overhaul ✅
- [x] **Tabbed settings UI** - organize settings into logical tabs (Models, Agent, Memory, Tools, Scheduler, Data)
- [x] **Full config migration** - settings saved to config.toml via GUI
- [x] **Persistent settings** - save to config file on change, load on startup
- [x] **Tool configuration** - tool API keys (Brave), tool list display
- [x] **System prompt editor** - edit default system prompt in GUI with templates
- [x] **Import/Export config** - backup and restore settings (download TOML / upload TOML)

### Channels Page Overhaul ✅
- [x] **Channel onboarding wizards** - step-by-step setup for each channel
  - [x] Telegram: BotFather walkthrough, token input, webhook setup
  - [x] Discord: App creation guide, bot token, permissions setup
  - [x] Slack: App manifest, OAuth flow, socket mode setup (4-step wizard)
  - [x] Signal: signald setup, phone verification, REST API options
  - [x] WhatsApp: Cloud API + Web bridge modes, full setup flow
- [x] **Credential management** - secure input/storage for tokens and keys
- [x] **Connection testing** - verify credentials work before saving
- [x] **Channel status live updates** - real-time connection health via Tauri events

### Build Artifacts (Windows)
- MSI: `D:\Development\Cargo Target\release\bundle\msi\Nanna_0.1.0_x64_en-US.msi`
- NSIS: `D:\Development\Cargo Target\release\bundle\nsis\Nanna_0.1.0_x64-setup.exe`

## Phase 5: Agent Swarm & Parallel Execution ✅
*Inspired by Kimi K2.5's agent swarm architecture*

### Parallel Agent Orchestration
- [x] **Swarm Coordinator** - orchestrator that decomposes tasks into parallel subtasks ✅
- [x] **Dynamic sub-agent spawning** - instantiate domain-specific agents on-the-fly ✅
- [x] **Parallel tool execution** - execute independent tool calls concurrently ✅
- [x] **Critical Path metrics** - `CriticalPathMetrics` struct with `calculate()` method ✅
- [x] **Sub-agent communication** - message passing between parallel agents ✅
- [x] **Result aggregation** - collect and synthesize outputs from parallel branches ✅

### Context Management
- [x] **Sliding window truncation** - retain only latest N messages when context grows ✅
- [x] **Message truncation** - truncate individual long messages (50KB limit) ✅
- [x] **Intelligent tool output truncation** - proportional budget allocation across tool results based on total context budget, with minimum floor per tool and recency bias ✅
- [x] **Selective context compression** - `compress()` method summarizes old context via LLM ✅
- [x] **Per-agent context isolation** - `ContextIsolation` enum (Full/SystemOnly/Summary/Isolated) ✅
- [x] **Context budget allocation** - `allocate_budget()` distributes tokens across parallel agents ✅
- [x] **Incremental summarization** - summarize once, reuse on subsequent requests ✅ (2026-02-04)
  - `consolidated_summary` field stores running summary
  - `messages_for_request()` prepends summary to messages
  - No re-summarization unless new content exceeds limits
- [x] **CDC deduplication** - content-defined chunking for duplicate detection ✅ (2026-02-04)
  - Gear rolling hash (FastCDC algorithm) in `nanna-agent/src/chunker.rs`
  - Deterministic chunk boundaries at ~2KB-32KB intervals
  - Same content produces same chunk hashes regardless of split position
  - 70% chunk overlap threshold triggers deduplication
  - Handles: same file split differently, minor edits, reordered content
- [x] **Summarization caching** - in-memory cache for summarization results ✅ (2026-02-04)
  - `SummaryCache` type alias with LRU eviction (100 entries)
  - Cache shared across summarization iterations within session
  - Avoids re-summarizing identical content blocks

### Context Optimization & Token Efficiency ✅ (2026-02-05)
- [x] **Proactive compression** - Every 5 iterations, if tokens > 40% of compression_threshold, drop oldest messages ✅
- [x] **`drop_oldest()` fallback** - No-LLM-required compression: drops messages, preserves key fragments in consolidated summary ✅
- [x] **Strip write content** - `write_file`/`write` tool_use blocks have `content` replaced with `[N bytes written]` in stored context ✅
- [x] **Task delegation tool** - `task` tool spawns sub-agents with isolated context via `AgentSpawner` trait abstraction ✅
  - Same adapter pattern as memory tools (trait in nanna-tools, impl in nanna-daemon)
  - Sub-agent gets fresh context (system prompt + workspace only)
  - 5-minute timeout, max 25 iterations, returns text + usage metadata
- [x] **Tiered compression** - Three tiers: proactive (40%), standard (compression_threshold), hard cap (hard_limit) ✅
  - Tier 1: drop_oldest every 5 iterations
  - Tier 2: full summarization if models configured, else drop_oldest
  - Tier 3: aggressive summarization or truncation at hard limit
- [x] **Token budget tracking** - `RunOptions.token_budget` + `budget_awareness` for pacing ✅
  - Cumulative tracking across iterations
  - Warnings at 80%, hard stop at 100%
  - Budget note injected into context when awareness enabled
  - `AgentResponse.cumulative_input_tokens` / `cumulative_output_tokens`
- [x] **Code analysis tools** - Token-efficient codebase understanding ✅
  - `code_outline`: function signatures, struct/enum/trait defs (~5-20% of file size)
  - `code_search`: regex search with context lines across files
  - `project_structure`: directory tree with file sizes and line counts

### Thinking Mode Enhancements
- [x] **Explicit thinking toggle** - `ThinkingMode` enum (Instant/Low/Medium/High/Maximum) per request ✅
- [x] **Interleaved reasoning** - `ReasoningBlock` captures thinking before tool calls ✅
- [x] **Reasoning content field** - `AgentResponse.reasoning: Option<ReasoningContent>` ✅
- [x] **Thinking budget** - `ThinkingMode::budget_tokens()` returns configurable max tokens ✅

### Visual Agent Capabilities (Future)
- [ ] **Screenshot-to-code** - generate UI code from images
- [ ] **Visual debugging loop** - agent inspects its own output visually
- [ ] **Video understanding** - process video inputs for multi-step workflows

## Phase 7: Rich Input & Editor Experience

### Tiptap Markdown Editor
**Goal:** Replace plain textarea with a rich Tiptap-based editor for markdown input with Monaco code blocks.

**Stack:**
- [tiptap-shadcn-vue](https://tiptap-shadcn-vue.pages.dev/tiptap) - Vue/Nuxt Tiptap integration with shadcn-vue
- Monaco Editor (via `@monaco-editor/react` port or `monaco-editor-vue3`)
- Tailwind v4 + Nanna Palenight theme

**Core Features:**
- [ ] **Basic formatting** - Bold, italic, strikethrough, underline
- [ ] **Headings** - H1-H3 with keyboard shortcuts
- [ ] **Lists** - Bullet, numbered, and task lists
- [ ] **Blockquotes** - Styled quote blocks
- [ ] **Links** - Inline link insertion with URL preview
- [ ] **Images** - Paste/drag-drop with inline preview
- [ ] **Tables** - Basic table support
- [ ] **Horizontal rules** - Dividers
- [ ] **Markdown shortcuts** - Type `**bold**` and it auto-converts

**Code Block Integration:**
- [ ] **Monaco-powered code blocks** - Full syntax highlighting + autocomplete
- [ ] **Language selector** - Dropdown for language (auto-detect default)
- [ ] **Inline code** - Backtick styling with monospace font
- [ ] **Copy button** - One-click copy for code blocks
- [ ] **Line numbers** - Toggleable in settings
- [ ] **Theme sync** - Monaco uses Nanna Palenight colors

**Nanna Theming:**
- [ ] **Palenight toolbar** - Violet/cyan accents on slate background
- [ ] **CRT glow effects** - Subtle glow on focused elements
- [ ] **Monospace fonts** - JetBrains Mono / Fira Code for code
- [ ] **Dark mode only** - Matches Nanna aesthetic
- [ ] **Custom selection colors** - Violet-tinted text selection

**UX Enhancements:**
- [ ] **Floating toolbar** - Appears on text selection
- [ ] **Slash commands** - Type `/` for quick formatting menu
- [ ] **Drag-and-drop blocks** - Reorder content blocks
- [ ] **Keyboard shortcuts** - Full vim-style navigation optional
- [ ] **Mobile toolbar** - Responsive bottom toolbar on mobile
- [ ] **Placeholder text** - "Ask Nanna anything..." with tips

**Technical:**
- [ ] **Output format** - Markdown (for LLM context) + HTML (for rendering)
- [ ] **Content persistence** - Draft saved to localStorage
- [ ] **Streaming input** - Editor remains editable while response streams
- [ ] **History/Undo** - Full undo/redo stack
- [ ] **Accessibility** - ARIA labels, keyboard navigation

**Integration Points:**
- [ ] **Chat input replacement** - Replace `<textarea>` in chat
- [ ] **System prompt editor** - Use in settings for prompt editing
- [ ] **Memory editor** - Rich editing for memory content
- [ ] **Workspace files** - Edit SOUL.md, USER.md with rich preview

**Dependencies:**
```
# Vue Tiptap ecosystem
@tiptap/vue-3
@tiptap/starter-kit
@tiptap/extension-code-block-lowlight
@tiptap/extension-image
@tiptap/extension-link
@tiptap/extension-table
@tiptap/extension-placeholder
@tiptap/extension-typography

# Monaco (for code blocks)
monaco-editor
@vueuse/core  # for Monaco resize handling

# Syntax highlighting
lowlight
highlight.js
```

**Implementation Order:**
1. Basic Tiptap editor component with formatting toolbar
2. Theme integration (Palenight colors, fonts)
3. Code block extension with syntax highlighting (lowlight)
4. Monaco integration for full code editing
5. Slash commands and floating toolbar
6. Chat integration (replace textarea)
7. Settings/memory editor integration

## Phase 6: Production Hardening
- [ ] Prometheus metrics
- [ ] Tracing spans for tool calls
- [ ] Cost tracking per session
- [ ] Runtime config reload
- [ ] Per-channel config
- [ ] Tool allowlists/blocklists
- [x] **Rate limiting (outbound)** - token bucket per channel with provider-specific defaults
- [x] **Error recovery / retry logic** - exponential backoff with jitter
- [x] **Message queuing** - priority queue with burst handling and offline resilience
- [x] **Graceful rate limit handling** - detect 429s, backoff, queue, and retry with exponential delay

## Phase 8: Clawdbot Feature Parity
*Goal: Nanna can do everything Clawdbot can — always-on, multi-channel, fully autonomous*

### Core Architecture: Channels as Control Plane Clients

**Key Insight:** The GUI is not a privileged controller — it's just another channel. ALL channels should have full access to the control plane, with rendering adapted to their capabilities.

```
┌─────────────────────────────────────────────────────────────┐
│                        Control Plane                        │
│  ┌──────────┬──────────┬──────────┬──────────┬──────────┐  │
│  │ Sessions │ Memory   │ Config   │ Tools    │ Scheduler│  │
│  │ Manager  │ Browser  │ Manager  │ Registry │ /Cron    │  │
│  └──────────┴──────────┴──────────┴──────────┴──────────┘  │
│                            ▲                                │
│                            │ Full Access (all channels)     │
│  ┌─────────────────────────┴───────────────────────────┐   │
│  │                  Channel Router                      │   │
│  └──┬────────┬────────┬────────┬────────┬────────┬─────┘   │
└─────┼────────┼────────┼────────┼────────┼────────┼─────────┘
      ▼        ▼        ▼        ▼        ▼        ▼
  Telegram  Discord   GUI    CLI     API    Slack
```

**Principles:**
- Every channel can: manage sessions, browse/edit memory, configure settings, control tools, manage scheduler
- Capabilities determine HOW things render, not WHAT you can access
- GUI is "just the channel with richest rendering" — not special
- Multiple channels (including multiple GUIs) can attach to same session
- Daemon owns state; channels are interchangeable views/controllers

**Channel Capabilities (rendering hints, not access control):**
| Channel | Markdown | Tables | Embeds | Buttons | Modals | Streaming |
|---------|----------|--------|--------|---------|--------|-----------|
| GUI     | ✓        | ✓      | ✓      | ✓       | ✓      | ✓         |
| Telegram| ✓        | -      | -      | ✓       | -      | -         |
| Discord | ✓        | -      | ✓      | ✓       | ✓      | -         |
| Slack   | ✓        | -      | ✓      | ✓       | ✓      | -         |
| CLI     | ✓        | ✓      | -      | -       | -      | ✓         |
| API     | -        | ✓      | -      | -       | -      | ✓         |

**Multi-GUI / Multi-Device:**
- Multiple GUIs can subscribe to same session (phone + desktop)
- All see messages in real-time (like multiple Telegram clients)
- Cross-channel sessions possible (Slack + Discord + GUI on same conversation)

### Daemon Mode ✅ (Functional - Pending Testing)
**Run Nanna as a background service, headless, with attachable GUI**

**Architecture:** Daemon runs independently; GUI connects as a channel client.
```
┌─────────────────────┐     ┌─────────────────────┐
│   nanna-daemon      │     │    nanna-gui        │
│  (always running)   │◄───►│  (attach/detach)    │
│                     │ WS  │                     │
│  • Agent core       │     │  • Rich UI channel  │
│  • All channels     │     │  • Can run embedded │
│  • Control plane    │     │    (iOS) or remote  │
└─────────────────────┘     └─────────────────────┘
```

**Core Daemon:**
- [x] **CLI binary** - `nanna-daemon run/start/stop/status/install/uninstall` commands
- [x] **Service installation** - Windows Service / systemd / launchd
- [x] **Headless operation** - No GUI required, config-driven
- [x] **Graceful shutdown** - SIGTERM/Ctrl+C handling, save state
- [x] **IPC server** - WebSocket server on port 5149
- [x] **GUI client library** - `nanna-client` crate for connecting
- [x] **Session persistence** - JSON persistence with auto-save
- [x] **Protocol definition** - Complete IPC protocol for all actions

**Integration (2026-02-02):**
- [x] **Memory service** - Initialized with embedding support (OpenAI/Ollama/Anthropic)
- [x] **Agent service** - LLM client + tool registry + memory integration
- [x] **IPC response routing** - Arc-based server sharing, proper request/response pairing
- [x] **Backend abstraction** - Unified interface for daemon vs embedded modes
- [x] **GUI routing** - Commands check mode and route through daemon client or embedded
- [x] **Embedded fallback** - GUI runs direct services when daemon unavailable
- [x] **Auto-reconnection** - Health monitor detects and reconnects to daemon

**Bug Fixes (2026-02-03):**
- [x] **Anthropic OAuth support** - Daemon now supports OAuth tokens from GUI (not just API keys)
- [x] **Detailed tool logging** - Added emoji-prefixed logging showing tool name, input, output, success/failure, duration
- [x] **Tool name aliases** - Added Claude Code compatibility aliases (read→read_file, bash→exec, glob→list_dir, etc.)

**Pending:**
- [ ] **PID file + lockfile** - Prevent multiple instances
- [ ] **Auto-restart** - Crash recovery with backoff
- [ ] **Log rotation** - File-based logs with rotation
- [ ] **Health endpoint** - HTTP `/health` for monitoring
- [ ] **End-to-end testing** - Verify daemon mode + embedded fallback work

**Platform Support:**
| Platform | Daemon | GUI Mode | IPC |
|----------|--------|----------|-----|
| Windows | Background process / Service | Remote (attach to daemon) | Named pipe / localhost WS |
| macOS | launchd agent | Remote | Unix socket / localhost WS |
| Linux | systemd user service | Remote | Unix socket / localhost WS |
| Android | Foreground Service | Remote (same app) | Binder / localhost WS |
| iOS | ❌ Not allowed | Embedded only | In-process |

**Crate Structure:**
```rust
// nanna-daemon: Headless binary, owns all state
// nanna-client: Library for connecting to daemon  
// nanna-gui: Uses nanna-client OR embeds nanna-core (iOS)
```

### Channel Listeners
**Actually connect to messaging platforms and receive messages**

- [x] **Telegram long-polling** - `getUpdates` loop with offset tracking ✅
- [ ] **Telegram webhooks** - Optional webhook mode for high-volume
- [x] **Discord Gateway** - WebSocket connection to Discord Gateway ✅
- [x] **Slack Socket Mode** - WebSocket via Slack's Socket Mode API ✅
- [ ] **Signal listener** - signald WebSocket or REST polling
- [ ] **WhatsApp listener** - Webhook receiver for Cloud API
- [x] **Unified message router** - All channels → single message queue ✅
- [ ] **Per-channel sessions** - Isolated context per chat/channel

**Architecture:**
```
                    ┌─────────────────┐
                    │   MessageRouter │
                    └────────┬────────┘
                             │
    ┌────────────────────────┼────────────────────────┐
    ▼            ▼           ▼           ▼            ▼
┌────────┐ ┌─────────┐ ┌─────────┐ ┌────────┐ ┌──────────┐
│Telegram│ │ Discord │ │  Slack  │ │ Signal │ │ WhatsApp │
│Listener│ │ Gateway │ │ Socket  │ │Listener│ │ Webhook  │
└────────┘ └─────────┘ └─────────┘ └────────┘ └──────────┘
```

### Webhook Server
**HTTP server to receive inbound webhooks from external services**

- [ ] **Axum-based server** - Lightweight, async HTTP server
- [ ] **Telegram webhook endpoint** - `/webhook/telegram`
- [ ] **Discord interactions endpoint** - `/webhook/discord` (slash commands)
- [ ] **Slack events endpoint** - `/webhook/slack` with verification
- [ ] **WhatsApp webhook** - `/webhook/whatsapp` with verify token
- [ ] **Generic webhook** - `/webhook/:id` for custom integrations
- [ ] **Request signing verification** - Validate webhook signatures
- [ ] **Ngrok/tunnel integration** - Dev mode tunnel for local testing

**Config:**
```toml
[server]
enabled = true
host = "0.0.0.0"
port = 3000
webhook_secret = "..."

[server.webhooks]
telegram = { path = "/webhook/telegram", secret = "..." }
discord = { path = "/webhook/discord", public_key = "..." }
slack = { path = "/webhook/slack", signing_secret = "..." }
```

### Cron & Scheduled Jobs
**Persistent scheduled tasks with GUI management**

- [ ] **Cron expression parser** - Standard cron syntax (minute hour day month weekday)
- [ ] **Job persistence** - Jobs survive restarts (SQLite)
- [ ] **Job types** - Message, prompt, tool call, webhook
- [ ] **Timezone support** - Per-job timezone configuration
- [ ] **GUI cron editor** - Visual cron builder in settings
- [ ] **Job history** - Track runs, success/failure, duration
- [ ] **Missed job handling** - Run on startup if missed
- [ ] **Job dependencies** - Run job B after job A completes

**Examples:**
```toml
[[cron.jobs]]
id = "morning-briefing"
schedule = "0 8 * * *"  # 8 AM daily
timezone = "America/Chicago"
action = { type = "prompt", text = "Give me today's briefing" }
channel = "telegram:123456"

[[cron.jobs]]
id = "backup-memories"
schedule = "0 3 * * 0"  # 3 AM Sundays
action = { type = "tool", name = "exec", input = { command = "backup.sh" } }
```

### Heartbeats
**Periodic self-checks with proactive outreach**

- [ ] **Heartbeat interval** - Configurable (default 30 min)
- [ ] **HEARTBEAT.md execution** - Run tasks from workspace file
- [ ] **Inbox checking** - Email, calendar, notifications
- [ ] **Proactive alerts** - Notify user of important events
- [ ] **Quiet hours** - Respect do-not-disturb schedules
- [ ] **Heartbeat history** - Track what was checked and when
- [ ] **GUI heartbeat config** - Enable/disable, set interval

### Sub-Agent Sessions
**Background task spawning with inter-session communication**

- [ ] **Session spawning** - `spawn_session(task, config)` API
- [ ] **Session labels** - Named sessions for easy reference
- [ ] **Inter-session messaging** - `send_to_session(label, message)`
- [ ] **Session lifecycle** - Auto-cleanup on completion
- [ ] **Session timeouts** - Kill runaway sessions
- [ ] **Result callbacks** - Notify parent when child completes
- [ ] **GUI session monitor** - View active sub-agents
- [ ] **Session history** - Browse completed sub-agent runs

### TTS (Text-to-Speech)
**Voice output for responses**

- [ ] **ElevenLabs integration** - High-quality neural TTS
- [ ] **OpenAI TTS** - Alternative provider
- [ ] **Local TTS** - Piper/Coqui for offline use
- [ ] **Voice selection** - Choose from available voices
- [ ] **Per-channel TTS** - Enable TTS for specific channels
- [ ] **Audio streaming** - Stream audio as it generates
- [ ] **Voice message sending** - Send as voice note on Telegram/WhatsApp
- [ ] **Caching** - Cache common phrases

**Config:**
```toml
[tts]
provider = "elevenlabs"  # elevenlabs | openai | local
voice_id = "..."
model = "eleven_turbo_v2"

[tts.channels]
telegram = true
discord = false
```

### Paired Devices (Nodes)
**Control and query mobile devices and remote machines**

- [ ] **Node discovery** - mDNS/manual registration
- [ ] **Node authentication** - Secure pairing flow
- [ ] **Camera access** - Snap photos from phone cameras
- [ ] **Screen capture** - Screenshot/recording from devices
- [ ] **Location access** - GPS coordinates with privacy controls
- [ ] **Notification sending** - Push notifications to devices
- [ ] **Clipboard sync** - Share clipboard across devices
- [ ] **File transfer** - Send/receive files from nodes
- [ ] **Remote execution** - Run commands on paired machines

**Architecture:**
```
Nanna Daemon ◄──────► Node Agent (Phone)
     │                    ├── Camera
     │                    ├── Location
     │                    ├── Screen
     │                    └── Notifications
     │
     └──────────────► Node Agent (Desktop)
                          ├── Screen
                          ├── Clipboard
                          └── Shell
```

### Browser Relay
**Control browser tabs via Chrome extension**

- [ ] **Chrome extension** - "Nanna Browser Relay" extension
- [ ] **Tab attachment** - User clicks toolbar to attach tab
- [ ] **Tab snapshot** - Get DOM/accessibility tree
- [ ] **Tab actions** - Click, type, scroll, navigate
- [ ] **Screenshot** - Capture visible viewport
- [ ] **Console access** - Read browser console logs
- [ ] **Multi-tab support** - Manage multiple attached tabs
- [ ] **WebSocket relay** - Extension ↔ Daemon communication

**Extension manifest:**
```json
{
  "name": "Nanna Browser Relay",
  "permissions": ["activeTab", "scripting", "tabs"],
  "background": { "service_worker": "background.js" }
}
```

### Gateway Control
**Self-management and configuration**

- [ ] **Live config reload** - Apply config changes without restart
- [ ] **Self-update** - Check for and apply updates
- [ ] **Restart command** - `/restart` from any channel
- [ ] **Status command** - `/status` shows health, uptime, memory
- [ ] **Config API** - Read/write config via IPC or HTTP
- [ ] **Backup/restore** - Full state backup and restore

### Implementation Order

1. **Daemon binary** - Headless runtime, no GUI
2. **Webhook server** - HTTP endpoints for inbound
3. **Telegram listener** - First channel with long-polling
4. **Unified message router** - Channel → Agent → Response
5. **Cron system** - Persistent scheduled jobs
6. **Discord Gateway** - WebSocket listener
7. **Slack Socket Mode** - Real-time Slack
8. **Heartbeats** - Periodic self-checks
9. **Sub-agent sessions** - Background tasks
10. **TTS** - Voice output
11. **Browser relay** - Chrome extension
12. **Paired devices** - Mobile/desktop nodes

---

## Phase 9: Multi-Device Swarm (Tor P2P)
*Peer-to-peer daemon communication over Tor hidden services*

**Vision:** Every Nanna daemon becomes a node with its own `.onion` address. Nodes can invoke tools on each other — phone camera from desktop, GPU compute from phone, sensors from anywhere.

### Architecture
```
┌─────────────────────┐  Tor Hidden Service  ┌─────────────────────┐
│    Phone Daemon     │◄════════════════════►│   Desktop Daemon    │
│     (Android)       │   .onion ↔ .onion    │   (Windows/Linux)   │
│                     │                      │                     │
│  Tools:             │  "Run camera_snap    │  Tools:             │
│  - Camera           │   on your phone"     │  - File system      │
│  - GPS              │◄─────────────────────│  - Browser          │
│  - Notifications    │                      │  - GPU compute      │
│  - Sensors          │                      │  - exec             │
└─────────────────────┘                      └─────────────────────┘
          │                                            │
          └────────────────────┬───────────────────────┘
                               ▼
                 ┌─────────────────────────┐
                 │    Gateway / Registry   │
                 │  (Optional central or   │
                 │     DHT-based)          │
                 └─────────────────────────┘
```

### nanna-identity Crate
**Persistent cryptographic identity for each daemon**

- [ ] **Ed25519 keypair generation** - `ed25519-dalek` for signing
- [ ] **Onion address derivation** - Derive `.onion` from public key via `onyums`
- [ ] **Identity persistence** - Store in `~/.nanna/identity.json`
- [ ] **Identity rotation** - Generate new identity on demand
- [ ] **Export/import** - Backup and restore identity
- [ ] **Fingerprint display** - Human-readable identity verification

**Identity format:**
```json
{
  "version": 1,
  "created_at": "2026-02-01T12:00:00Z",
  "public_key": "base64...",
  "secret_key_encrypted": "base64...",
  "onion_address": "abc123xyz789.onion",
  "fingerprint": "A1B2-C3D4-E5F6-G7H8"
}
```

### nanna-tor Crate
**Tor integration for hidden service publishing and outbound requests**

- [ ] **Embedded Tor** - Bundle `arti` (pure Rust Tor) for zero-config
- [ ] **System Tor fallback** - Connect to existing Tor daemon if available
- [ ] **Hidden service publishing** - Expose daemon IPC over `.onion`
- [ ] **Outbound requests** - `artiqwest` for HTTP requests to other `.onion` addresses
- [ ] **Circuit management** - Connection pooling and circuit reuse
- [ ] **Bootstrap progress** - Report Tor bootstrap status to GUI

**Dependencies:**
```toml
arti-client = "0.x"        # Embedded Tor
artiqwest = "0.x"          # Tor HTTP client
onyums = "0.x"             # Onion address generation for axum
```

### Peer Discovery
**How nodes find each other**

- [ ] **Manual pairing** - Exchange onion addresses via QR code or link
- [ ] **Pairing flow** - Device A shows QR, Device B scans, mutual approval
- [ ] **Gateway registry** - Optional central rendezvous server
- [ ] **DHT discovery** - Fully decentralized (Kademlia-style, future)
- [ ] **Peer persistence** - Remember paired devices across restarts

**Pairing protocol:**
```
Device A                          Device B
   │                                  │
   │──── Display QR (onion + nonce) ──►
   │                                  │
   │◄─── Scan + send pairing request ─│
   │     (signed with B's key)        │
   │                                  │
   │──── Approve + send confirmation ─►
   │     (signed with A's key)        │
   │                                  │
   └──── Mutual trust established ────┘
```

### Remote Tool Protocol
**Cross-node tool invocation**

- [ ] **Tool request message** - Request tool execution on remote node
- [ ] **Tool response message** - Return result or error
- [ ] **Streaming results** - Long-running tools stream output
- [ ] **Request signing** - Ed25519 signatures on all requests
- [ ] **Request encryption** - End-to-end encryption (Tor provides transport)
- [ ] **Timeout handling** - Request timeouts with retry logic

**Protocol messages:**
```json
{
  "type": "tool_request",
  "id": "uuid",
  "from": "abc123.onion",
  "to": "xyz789.onion",
  "tool": "camera_snap",
  "input": { "facing": "front" },
  "signature": "base64..."
}

{
  "type": "tool_response",
  "id": "uuid",
  "from": "xyz789.onion",
  "to": "abc123.onion",
  "result": { "image": "base64...", "timestamp": "..." },
  "signature": "base64..."
}
```

### Trust & Permissions
**Fine-grained control over what peers can do**

- [ ] **Peer allowlist** - Only approved peers can connect
- [ ] **Per-peer tool allowlist** - Limit which tools each peer can invoke
- [ ] **Request approval** - Optional user confirmation for sensitive tools
- [ ] **Audit log** - Track all remote tool invocations
- [ ] **Rate limiting** - Per-peer request limits
- [ ] **Revocation** - Remove peer trust instantly

**Trust config:**
```toml
[[peers]]
name = "My Phone"
onion = "abc123.onion"
fingerprint = "A1B2-C3D4-E5F6-G7H8"
tools = ["camera_snap", "location_get", "notify"]
require_approval = ["location_get"]

[[peers]]
name = "Work Laptop"
onion = "xyz789.onion"
fingerprint = "I9J0-K1L2-M3N4-O5P6"
tools = ["*"]  # Full access
```

### Claude Code / External Agent Attachment
**Connect external AI agents to Nanna**

- [ ] **API key mode** - User provides Anthropic key, Nanna proxies requests
- [ ] **Claude Code bridge** - Connect to running Claude Code session via socket
- [ ] **Tool exposure** - Expose Nanna tools to external agent
- [ ] **Session handoff** - External agent takes over conversation
- [ ] **Capability negotiation** - Advertise available tools to external agent

### Implementation Order

1. **nanna-identity** - Keypair generation, onion derivation, persistence
2. **nanna-tor** - Embedded Tor, hidden service publishing
3. **Peer discovery** - Manual pairing via QR/link
4. **Remote tool protocol** - Request/response over Tor
5. **Trust model** - Peer allowlists, tool permissions
6. **GUI integration** - Pair devices, manage peers, view remote tools
7. **Claude Code bridge** - External agent attachment

---

## Phase 10: Token Efficiency & Cost Optimization
*Reduce token consumption across the board without sacrificing quality*

### 1. Prompt Caching (Provider-Native) — Priority: CRITICAL
**Use Anthropic/OpenAI server-side KV cache to avoid re-processing repeated prefixes**

- [x] **Anthropic automatic caching** - Add `cache_control: {type: "ephemeral"}` to requests ✅ (2026-02-23)
  - System prompt + tool definitions cached on first call (~5 min TTL)
  - Conversation prefix cached and extends each turn
  - 90% discount on cached input tokens (Anthropic pricing)
- [ ] **Explicit cache breakpoints** - Place `cache_control` on system prompt and tool definition blocks for fine-grained control
- [x] **OpenAI prompt caching** - Enable for OpenAI provider (automatic for prompts >1024 tokens, 50% discount)
- [x] **Cache hit tracking** - Log `cache_creation_input_tokens` and `cache_read_input_tokens` from responses ✅ (2026-02-23)
- [ ] **Cache-aware context ordering** - Keep system prompt + tools stable at prefix to maximize cache hits

**Expected savings: 50-80% of input token costs in multi-turn sessions**

### 2. Model Routing (Cross-Provider Priority Cascade) — Priority: HIGH
**Route requests to cheaper/faster models when full capability isn't needed**

Uses the existing `model_priority` list to cascade across providers. The agent classifies each iteration's complexity and picks the cheapest model that can handle it.

- [x] **Task complexity classifier** - Analyze the current iteration to determine complexity tier: ✅ (2026-02-23)
  - **Simple**: Direct tool calls (list_dir, read_file), short Q&A, acknowledgments → cheapest model
  - **Medium**: Multi-step reasoning, code generation, summarization → mid-tier model
  - **Complex**: Novel problem solving, long-form analysis, ambiguous requests → top-tier model
- [x] **Priority list routing** - Route to the highest-priority model capable of the task tier: ✅ (2026-02-23)
  - Each model in `model_priority` gets a `max_tier` (simple/medium/complex)
  - Router picks cheapest model whose `max_tier` >= task complexity
  - Falls back up the list if a model fails or returns low-quality output
- [x] **Tool-call-only routing** - When the LLM's only job is to pick which tool to call (no creative output needed), always use cheapest model ✅ (2026-02-23)
- [ ] **Escalation on failure** - If cheap model produces poor results (malformed tool calls, refusals, low confidence), automatically retry with next model up
- [x] **Per-iteration model tracking** - Log which model handled each iteration for cost analysis ✅ (2026-02-23)
- [ ] **Config: model tiers** - `model_priority` entries gain optional tier annotation:
  ```toml
  model_priority = [
    "claude-haiku-3-5-20241022:simple",
    "claude-sonnet-4-20250514:medium",  
    "claude-sonnet-4-20250514:complex",
  ]
  ```
- [x] **First-message override** - Always use primary model for first iteration (user-facing response quality matters most) ✅ (2026-02-23)
- [ ] **GUI model usage dashboard** - Show per-model token usage breakdown per session

**Expected savings: 60-90% on tool-calling iterations (majority of agent loop)**

### 3. Aggressive Tool Output Summarization — Priority: HIGH
**Compress tool outputs immediately after execution, before storing in context**

Respects `OutputTarget` — tools with `OutputTarget::Context` keep full output; tools with `OutputTarget::Memory` get summarized.

- [ ] **Immediate summarization** - After tool execution, if output > threshold (e.g., 2KB), summarize using cheap model before storing in context
- [ ] **Tool-type-aware compression**:
  - `read_file`: Keep only lines referenced in the conversation or relevant to the task
  - `web_fetch`: Extract answer-relevant paragraphs only
  - `exec`: Store exit code + last N lines (full output only on error)
  - `list_dir`: Compact format (names only, no metadata unless requested)
  - `code_search`: Keep matching lines + minimal context
- [ ] **OutputTarget respect** - Tools declaring `OutputTarget::Context` skip summarization
- [ ] **Configurable threshold** - `[agent] tool_output_summarize_threshold = 2048` (bytes)
- [ ] **Summarization model** - Use cheapest available model from `summarization_priority`
- [ ] **Fallback truncation** - If no summarization model available, smart-truncate (keep first + last lines)

**Expected savings: 30-50% on tool-heavy sessions**

### 4. Progressive Context Distillation — Priority: MEDIUM
**Refine the existing incremental summarization into a rolling distillation system**

- [ ] **Rolling summary every N turns** - Summarize every 3-5 turns instead of only at threshold
  - Configurable: `[agent] distillation_interval = 5` (turns)
  - Produces structured key-value facts, not prose (more token-dense)
- [ ] **Fact extraction format** - Distill into structured facts:
  ```
  [FACTS] user_goal: "implement prompt caching" | files_modified: ["lib.rs", "loop_runner.rs"] | decisions: ["use ephemeral cache type"] | blockers: []
  ```
- [ ] **Tool result eviction** - After a tool result has been referenced by the LLM's response, replace with a 1-line stub: `[tool: read_file("src/main.rs") → 245 lines, discussed above]`
- [ ] **Conversation phase detection** - Detect when topic shifts and aggressively summarize the completed phase
- [ ] **Distillation model** - Use cheapest available model (same as summarization_priority)

**Expected savings: 20-40% on long sessions**

### 5. Semantic Deduplication (Message-Level) — Priority: MEDIUM
**Extend CDC chunking to detect redundant information across messages**

- [ ] **Cross-message duplicate detection** - Before storing a tool result, check if substantially similar content already exists in context
  - Same file re-read → replace previous result with new one (don't accumulate both)
  - Same web page fetched again → keep only latest
  - Overlapping search results → merge
- [ ] **Information overlap scoring** - Use embedding similarity (existing nanna-memory infra) to detect when a new message adds <10% new information vs existing context
- [ ] **Automatic dedup on store** - When `store_tool_results()` runs, check for superseded results and evict them
- [ ] **User message dedup** - Detect when user restates information already in context, note it without storing full duplicate

**Expected savings: 10-20% on iterative development sessions**

### 6. LLMLingua-Style Prompt Compression — Priority: LOWER (Hardware Required)
**Use a small local model to score and drop low-information tokens from context**

Requires a local model (7B+) running on GPU. A 4070 Ti Super (16GB) handles this easily.

- [ ] **Perplexity-based token scoring** - Run small model (e.g., Phi-3, Qwen2-7B via Ollama) to compute per-token perplexity
- [ ] **Token dropping** - Remove tokens below importance threshold (configurable compression ratio)
- [ ] **Selective application** - Only compress tool outputs and old conversation turns; never compress system prompt or recent messages
- [ ] **Compression ratio config** - `[agent] llmlingua_ratio = 4` (4x compression)
- [ ] **Ollama integration** - Use existing Ollama infra; model configurable: `[agent] compression_model = "phi3:mini"`
- [ ] **Quality gate** - After compression, verify key information preserved via embedding similarity check
- [ ] **Bidirectional scoring** - Use encoder model (BERT-style) for better compression than causal-only (per Data Distillation paper, arxiv 2403.12968)

**Expected savings: 4-20x on large tool outputs (applied selectively)**

### 7. Structured Tool Output Schemas — Priority: LOWER
**Ensure built-in tools return structured, token-dense output**

- [ ] **Audit built-in tools** - Review all built-in tool outputs for verbosity; tighten where possible
- [ ] **JSON output mode** - Built-in tools return structured JSON instead of prose where appropriate
- [ ] **Output schema in ToolDefinition** - Add optional `output_schema` field to `ToolDefinition` for documentation
- [ ] **Plugin guidance** - Document best practices for skill authors: "return structured data, not explanations"
- [ ] **Output post-processing** - Optional registry-level post-processor that strips common boilerplate from tool outputs

**Expected savings: 10-30% on tool outputs**

### Implementation Priority Order

| # | Technique | Savings Estimate | Effort | Dependencies |
|---|-----------|-----------------|--------|-------------|
| 1 | Prompt Caching | 50-80% input costs | Trivial | None |
| 2 | Model Routing | 60-90% on simple ops | Medium | model_priority config |
| 3 | Tool Output Summarization | 30-50% tool-heavy | Medium | summarization_priority |
| 4 | Progressive Distillation | 20-40% long sessions | Medium | Refine existing code |
| 5 | Semantic Dedup | 10-20% iterative work | Medium | nanna-memory embeddings |
| 6 | LLMLingua Compression | 4-20x selective | High | Local GPU model |
| 7 | Structured Outputs | 10-30% tool outputs | Low | Audit existing tools |

**Combined expected savings: 70-90% token reduction vs current baseline**

---

## Quick Priorities

| Priority | Item | Status |
|----------|------|--------|
| 1 | ~~Telegram E2E~~ | ✅ |
| 2 | ~~Browser tools~~ | ✅ |
| 3 | ~~Vision tools~~ | ✅ |
| 4 | ~~Discord channel~~ | ✅ |
| 5 | ~~Audio tools (TTS/Whisper)~~ | ✅ |
| 6 | ~~MCP client~~ | ✅ |
| 7 | ~~Wire MCP into agent loop~~ | ✅ |
| 8 | ~~Slack channel~~ | ✅ |
| 9 | ~~Signal channel~~ | ✅ |
| 10 | ~~WhatsApp channel~~ | ✅ |
| 11 | ~~MCP server mode~~ | ✅ |
| 12 | ~~Background tasks~~ | ✅ |
| 13 | ~~Supervisor patterns~~ | ✅ |
| 14 | ~~Deno scripting~~ | ✅ |
| 15 | ~~Tauri GUI scaffold~~ | ✅ |
| 16 | ~~GUI IPC + streaming~~ | ✅ |
| 17 | ~~Settings + session mgmt~~ | ✅ |
| 18 | ~~Markdown rendering~~ | ✅ |
| 19 | ~~Tool call visualization~~ | ✅ |
| 20 | ~~System tray~~ | ✅ |
| 21 | ~~Memory browser~~ | ✅ |
| 22 | ~~Streaming UX polish~~ | ✅ |
| 23 | ~~Channel status dashboard~~ | ✅ Done |
| 24 | ~~Native notifications wiring~~ | ✅ Done |
| 25 | ~~Dreaming trigger + auto feedback~~ | ✅ Done |
| 26 | ~~Ollama integration~~ | ✅ Done |
| 27 | ~~Memory extraction: configurable model~~ | ✅ Done |
| 28 | ~~Memory extraction: FSRS feedback loop~~ | ✅ Done |
| 29 | ~~Memory duplicate detection~~ | ✅ Done |
| 30 | ~~Parallel tool execution~~ | ✅ Done |
| 31 | ~~Swarm Coordinator~~ | ✅ Done |
| 32 | ~~Context management/truncation~~ | ✅ Done |
| 33 | ~~Memory management UI~~ | ✅ Done |
| 34 | ~~Critical Path metrics~~ | ✅ Done |
| 35 | ~~Thinking mode toggle~~ | ✅ Done |
| 36 | ~~UI component library~~ | ✅ Done |
| 37 | Tauri mobile (Android/iOS) | Later |
| 38 | Visual debugging loop | Future |
| 39 | Production hardening | Later |
| 40 | ~~PDF tools (read + extract images)~~ | ✅ Done |
| 41 | ~~OCR + image description~~ | ✅ Done |
| 42 | ~~Settings overhaul (tabs + full config)~~ | ✅ Done |
| 43 | ~~Channels page (Telegram/Discord wizards)~~ | ✅ Done |
| 44 | ~~Channels: Slack/Signal/WhatsApp wizards~~ | ✅ Done |
| 45 | ~~System prompt editor~~ | ✅ Done |
| 46 | ~~Config import/export~~ | ✅ Done |
| 47 | ~~Message queuing~~ | ✅ Done |
| 48 | ~~Graceful rate limit handling~~ | ✅ Done |
| 49 | ~~Intelligent tool output truncation~~ | ✅ Done |
| 50 | ~~Channel status live updates~~ | ✅ Done |
| 51 | Prometheus metrics | Later |
| 52 | Cost tracking per session | Later |
| 53 | Tiptap markdown editor | Later |
| 54 | Monaco code blocks | Later |
| 55 | Nanna-themed editor | Later |
| 56 | Slash commands | Later |
| 57 | Chat input replacement | Later |
| **Phase 8: Clawdbot Parity** | | |
| 58 | ~~Daemon binary (headless)~~ | ✅ Done |
| 59 | ~~IPC Protocol definition~~ | ✅ Done |
| 60 | ~~Windows Service~~ | ✅ Done |
| 61 | ~~Session persistence~~ | ✅ Done |
| 62 | ~~Client library (nanna-client)~~ | ✅ Done |
| 63 | ~~Wire GUI to daemon client~~ | ✅ Done (2026-02-02) |
| 64 | ~~Agent integration in daemon~~ | ✅ Done (2026-02-02) |
| 65 | Webhook server (Axum) | ✅ Done (2026-02-03) |
| 66 | Telegram listener | ✅ Done |
| 67 | Unified message router | ✅ Done |
| 68 | Cron system + persistence | ✅ Done |
| 69 | Discord Gateway | ✅ Done |
| 70 | Slack Socket Mode | ✅ Done |
| 71 | Heartbeats | ✅ Done |
| 72 | Sub-agent sessions | 🔜 |
| 72a | Workspaces in daemon | ✅ Done (2026-02-03) |
| 72b | Config/Settings in daemon | ✅ Done (2026-02-03) |
| 72c | Shared keyring for credentials | ✅ Done (2026-02-03) |
| 72d | Scheduler actions in daemon | ✅ Done (2026-02-03) |
| 72e | User tool authoring in daemon | ✅ Done (2026-02-03) |
| 72f | Daemon OAuth support | ✅ Done (2026-02-03) |
| 72g | Tool name aliases (Claude Code compat) | ✅ Done (2026-02-03) |
| 72h | Detailed tool execution logging | ✅ Done (2026-02-03) |
| 72i | Incremental summarization | ✅ Done (2026-02-04) |
| 72j | CDC deduplication (chunker.rs) | ✅ Done (2026-02-04) |
| 72k | Summarization caching | ✅ Done (2026-02-04) |
| 72l | Proactive context compression (tiered) | ✅ Done (2026-02-05) |
| 72m | Strip write content from stored blocks | ✅ Done (2026-02-05) |
| 72n | Task delegation tool (sub-agent) | ✅ Done (2026-02-05) |
| 72o | Token budget tracking & pacing | ✅ Done (2026-02-05) |
| 72p | Code analysis tools (outline/search/structure) | ✅ Done (2026-02-05) |
| 72q | Per-session message queuing (FIFO mutex) | ✅ Done (2026-02-05) |
| 73 | TTS (ElevenLabs/OpenAI) | Later |
| 74 | Browser relay extension | Later |
| 75 | Paired devices (nodes) | Later |
| **Phase 9: Multi-Device Swarm** | | |
| 76 | nanna-identity (keypair + onion) | 🔜 |
| 77 | nanna-tor (embedded Tor + hidden service) | 🔜 |
| 78 | Peer discovery (QR pairing) | Later |
| 79 | Remote tool protocol | Later |
| 80 | Trust model (per-peer permissions) | Later |
| 81 | GUI peer management | Later |
| 82 | Claude Code bridge | Later |
| **Phase 10: Token Efficiency** | | |
| 83 | ~~Prompt caching (Anthropic native)~~ | ✅ Done (2026-02-23) |
| 84 | Prompt caching (OpenAI native) | ✅ Done (2026-02-25) |
| 85 | ~~Cache hit tracking + logging~~ | ✅ Done (2026-02-23) |
| 86 | ~~Model routing (cross-provider cascade)~~ | ✅ Done (2026-02-23) |
| 87 | ~~Task complexity classifier~~ | ✅ Done (2026-02-23) |
| 88 | ~~Tool-call-only routing (cheapest model)~~ | ✅ Done (2026-02-23) |
| 89 | ~~Escalation on failure~~ | ✅ Already impl (loop_runner.rs) |
| 90 | ~~Aggressive tool output summarization~~ | ✅ Already impl |
| 91 | ~~Tool-type-aware compression~~ | ✅ Already impl |
| 92 | Progressive context distillation | ✅ |
| 93 | Rolling summary every N turns | ✅ Done (distillation_interval=5) |
| 94 | Tool result eviction (post-reference) | ✅ Done (2026-02-25) |
| 95 | Semantic dedup (message-level) | ✅ |
| 96 | Cross-message duplicate detection | ✅ Done (CDC-based in context.rs) |
| 97 | LLMLingua prompt compression | ✅ |
| 98 | Structured tool output schemas | ✅ |
| 99 | ~~Model stats tracker (latency/throughput/errors)~~ | ✅ Done (2026-02-23) |
| 100 | ~~Per-response model stats (UI-ready)~~ | ✅ Done (2026-02-23) |
| 101 | ~~Model stats API endpoint (SystemAction::ModelStats)~~ | ✅ Done (2026-02-23) |
| 102 | ~~Sub-agent routing (shared stats)~~ | ✅ Done (2026-02-23) |
| 103 | GUI model stats dashboard | 🔜 |
| 104 | Model stats persistence (survive restarts) | 🔜 |
| 105 | Stats-informed routing (avoid unhealthy models) | 🔜 |

---

## Open TODOs (Next Sprint)

### Daemon Architecture (2026-02-03) - MOSTLY COMPLETE ✅
1. ✅ **Workspaces in Daemon** - WorkspaceRegistry + WorkspaceAction protocol (List/Get/Open/Close/SetActive/ClearActive/Reload/GetContext/UpdateContext)
2. ✅ **Config in Daemon** - ConfigAction handlers (Get/Set/Reset/Reload/Export/Import)
3. ✅ **Scheduler in Daemon** - SchedulerAction handlers (List/Get/Add/Update/Remove/RunNow/History)
4. ✅ **User Tool Authoring in Daemon** - UserToolManager moved, ToolAction::Create/Update/Delete/Test/ListUser
5. ✅ **Shared Keyring** - OS keyring for API keys accessible by daemon + GUI (2026-02-03)
6. 🔜 **GUI Wiring** - Wire GUI to use daemon for all new endpoints

### Daemon Testing & Polish (High Priority)
6. **End-to-end testing** - Test daemon mode + embedded fallback + reconnection
7. **Error handling** - Improve error messages for connection failures
8. ✅ **Health endpoint** - Add HTTP `/health`, `/healthz`, `/readyz`, `/status` endpoints (2026-02-03)
9. ✅ **PID file** - Prevent multiple daemon instances (2026-02-03)
10. **Log rotation** - File-based logs with size/time rotation
11. **Health-check based request timeout** - Replace hard timeout on daemon client requests with session health polling. Instead of timing out after N seconds, the GUI periodically pings `get_session_run_state` to check if the session is still alive and making progress. Only timeout if the daemon stops responding or reports the session as unhealthy. Fixes: long-running agent loops (30+ tool iterations) being killed by the client before the daemon finishes.

### Channel Listeners (Medium Priority)
6. **Telegram Listener** - Long-polling `getUpdates` loop
7. **Discord Gateway** - WebSocket connection
8. **Slack Socket Mode** - Real-time Slack events
9. **Unified Router** - All channels → single message queue

### Webhook Server ✅ DONE (2026-02-03)
10. ✅ **Axum HTTP Server** - Base server with routing, CORS support
11. ✅ **Telegram Webhook** - `/webhook/telegram` endpoint with secret verification
12. ✅ **Discord Interactions** - `/webhook/discord` with PING handling + signature verification
13. ✅ **Slack Events** - `/webhook/slack` with URL verification challenge
14. ✅ **WhatsApp Webhook** - `/webhook/whatsapp` with verify token + Cloud API parsing
15. ✅ **Generic Webhooks** - `/webhook/:id` with Bearer/secret authentication

### Cron & Scheduling ✅ DONE
14. ~~**Cron Parser**~~ - Parse standard cron expressions ✅
15. ~~**Job Persistence**~~ - Store jobs in SQLite ✅
16. ~~**Job Execution**~~ - Run prompts/tools on schedule ✅
17. ~~**GUI Cron Editor**~~ - Visual cron builder ✅

### Session Tool Profiles (Phase 11 - UX)
1. **Profile definitions** - Named tool/prompt profiles (e.g. "coding", "chat", "research")
2. **Per-session profile** - Sessions can be assigned a profile that controls which tools are available and system prompt tuning
3. **Auto-detection** - Classify user intent on first message and suggest/apply appropriate profile
4. **Custom profiles** - Users can create/edit profiles in settings (tool selection, system prompt overrides, iteration limits)
5. **Profile-specific system prompts** - Coding profile gets concise action-oriented prompt; chat profile gets conversational prompt
6. **Tool budget per profile** - Control how many tool definitions are sent to the LLM (reduce context usage for simple profiles)

### Multi-Device Swarm (Phase 9 - Foundation)
18. **nanna-identity** - Ed25519 keypair + onion address derivation
19. **nanna-tor** - Embedded arti + hidden service publishing
20. **Peer pairing** - QR code exchange + mutual approval flow
