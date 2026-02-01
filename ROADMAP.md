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

### Daemon Mode ✅ (Core Complete)
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

- [x] **CLI binary** - `nanna-daemon run/start/stop/status/install/uninstall` commands
- [x] **Service installation** - Windows Service / systemd / launchd
- [x] **Headless operation** - No GUI required, config-driven
- [x] **Graceful shutdown** - SIGTERM/Ctrl+C handling, save state
- [x] **IPC server** - WebSocket server on port 5149
- [x] **GUI client library** - `nanna-client` crate for connecting
- [x] **Session persistence** - JSON persistence with auto-save
- [x] **Protocol definition** - Complete IPC protocol for all actions
- [ ] **PID file + lockfile** - Prevent multiple instances
- [ ] **Auto-restart** - Crash recovery with backoff
- [ ] **Log rotation** - File-based logs with rotation
- [ ] **Health endpoint** - HTTP `/health` for monitoring
- [ ] **GUI integration** - Wire GUI to use daemon client
- [ ] **Agent integration** - Connect control plane to actual agent

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
| 63 | Wire GUI to daemon client | 🔜 |
| 64 | Agent integration in daemon | 🔜 |
| 65 | Webhook server (Axum) | 🔜 |
| 66 | Telegram listener | ✅ Done |
| 67 | Unified message router | ✅ Done |
| 68 | Cron system + persistence | 🔜 |
| 69 | Discord Gateway | ✅ Done |
| 70 | Slack Socket Mode | ✅ Done |
| 71 | Heartbeats | 🔜 |
| 72 | Sub-agent sessions | 🔜 |
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

---

## Open TODOs (Next Sprint)

### Daemon Completion (High Priority)
1. **Agent Integration** - Connect `ControlPlane` to `nanna-agent` for actual LLM calls
2. **GUI Wiring** - Update Tauri backend to use `nanna-client` in daemon mode
3. **Memory Integration** - Connect daemon to `nanna-memory` for persistence
4. **Tool Registry** - Wire up `nanna-tools` in daemon
5. **Fallback Mode** - GUI runs embedded agent when daemon unavailable

### Channel Listeners (Medium Priority)
6. **Telegram Listener** - Long-polling `getUpdates` loop
7. **Discord Gateway** - WebSocket connection
8. **Slack Socket Mode** - Real-time Slack events
9. **Unified Router** - All channels → single message queue

### Webhook Server (Medium Priority)
10. **Axum HTTP Server** - Base server with routing
11. **Telegram Webhook** - `/webhook/telegram` endpoint
12. **Discord Interactions** - `/webhook/discord` with signature verification
13. **Slack Events** - `/webhook/slack` with challenge handling

### Cron & Scheduling (Low Priority)
14. **Cron Parser** - Parse standard cron expressions
15. **Job Persistence** - Store jobs in SQLite
16. **Job Execution** - Run prompts/tools on schedule
17. **GUI Cron Editor** - Visual cron builder

### Multi-Device Swarm (Phase 9 - Foundation)
18. **nanna-identity** - Ed25519 keypair + onion address derivation
19. **nanna-tor** - Embedded arti + hidden service publishing
20. **Peer pairing** - QR code exchange + mutual approval flow
