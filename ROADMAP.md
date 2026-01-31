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

## Phase 4: GUI Application 🔜
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
- [x] **Tabbed settings UI** - organize settings into logical tabs (Models, Memory, Tools, Scheduler, Data)
- [x] **Full config migration** - settings saved to config.toml via GUI
- [x] **Persistent settings** - save to config file on change, load on startup
- [x] **Tool configuration** - tool API keys (Brave), tool list display
- [ ] **System prompt editor** - edit default system prompt in GUI
- [ ] **Import/Export config** - backup and restore settings

### Channels Page Overhaul 🔶
- [x] **Channel onboarding wizards** - step-by-step setup for each channel
  - [x] Telegram: BotFather walkthrough, token input, webhook setup
  - [x] Discord: App creation guide, bot token, permissions setup
  - [ ] Slack: App manifest, OAuth flow, socket mode setup
  - [ ] Signal: signald setup, phone verification
  - [ ] WhatsApp: Business API setup, access token
- [x] **Credential management** - secure input/storage for tokens and keys
- [x] **Connection testing** - verify credentials work before saving
- [ ] **Channel status live updates** - real-time connection health

### Build Artifacts (Windows)
- MSI: `D:\Development\Cargo Target\release\bundle\msi\Nanna_0.1.0_x64_en-US.msi`
- NSIS: `D:\Development\Cargo Target\release\bundle\nsis\Nanna_0.1.0_x64-setup.exe`

## Phase 5: Agent Swarm & Parallel Execution 🔜
*Inspired by Kimi K2.5's agent swarm architecture*

### Parallel Agent Orchestration
- [x] **Swarm Coordinator** - orchestrator that decomposes tasks into parallel subtasks ✅
- [x] **Dynamic sub-agent spawning** - instantiate domain-specific agents on-the-fly ✅
- [x] **Parallel tool execution** - execute independent tool calls concurrently ✅
- [ ] **Critical Path metrics** - measure efficiency by longest parallel branch, not total steps
- [x] **Sub-agent communication** - message passing between parallel agents ✅
- [x] **Result aggregation** - collect and synthesize outputs from parallel branches ✅

### Context Management
- [x] **Sliding window truncation** - retain only latest N messages when context grows ✅
- [x] **Message truncation** - truncate individual long messages (50KB limit) ✅
- [x] **Intelligent tool output truncation** - proportional budget allocation across tool results based on total context budget, with minimum floor per tool and recency bias ✅
- [ ] **Selective context compression** - summarize old context instead of dropping
- [ ] **Per-agent context isolation** - sub-agents don't inherit full parent context
- [ ] **Context budget allocation** - distribute context tokens across parallel agents

### Thinking Mode Enhancements
- [ ] **Explicit thinking toggle** - instant vs thinking mode per request
- [ ] **Interleaved reasoning** - thinking blocks between tool calls (like Claude)
- [ ] **Reasoning content field** - expose chain-of-thought separately from response
- [ ] **Thinking budget** - configurable max tokens for reasoning phase

### Visual Agent Capabilities (Future)
- [ ] **Screenshot-to-code** - generate UI code from images
- [ ] **Visual debugging loop** - agent inspects its own output visually
- [ ] **Video understanding** - process video inputs for multi-step workflows

## Phase 6: Production Hardening
- [ ] Prometheus metrics
- [ ] Tracing spans for tool calls
- [ ] Cost tracking per session
- [ ] Runtime config reload
- [ ] Per-channel config
- [ ] Tool allowlists/blocklists
- [ ] Rate limiting (outbound)
- [ ] Error recovery / retry logic
- [ ] **Message queuing** - queue outbound messages to handle bursts and offline periods
- [ ] **Graceful rate limit handling** - detect 429s, backoff, queue, and retry with exponential delay

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
| 34 | Critical Path metrics | Later |
| 35 | Thinking mode toggle | Later |
| 36 | ~~UI component library~~ | ✅ Done |
| 37 | Tauri mobile (Android/iOS) | Later |
| 38 | Visual debugging loop | Future |
| 39 | Production hardening | Later |
| 40 | ~~PDF tools (read + extract images)~~ | ✅ Done |
| 41 | ~~OCR + image description~~ | ✅ Done |
| 42 | ~~Settings overhaul (tabs + full config)~~ | ✅ Done |
| 43 | ~~Channels page (Telegram/Discord wizards)~~ | ✅ Done |
| 44 | Channels: Slack/Signal/WhatsApp wizards | 🔜 |
| 45 | System prompt editor | 🔜 |
| 46 | Config import/export | Later |
| 47 | Message queuing | 🔜 |
| 48 | Graceful rate limit handling | 🔜 |
| 49 | ~~Intelligent tool output truncation~~ | ✅ Done |
