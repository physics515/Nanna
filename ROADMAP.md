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
- [x] Audio tools (TTS, transcription)
- [x] Authoring tools (runtime tool creation) — `nanna-scripting` crate
  - [x] Boa engine (pure Rust, lightweight)
  - [ ] Deno engine (V8 fallback for full TS support)

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
- [ ] Supervisor patterns (restart policies, health checks)

## Phase 4: GUI Application 🆕
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
- [ ] Chat interface with streaming
- [ ] Session management
- [ ] Tool call visualization
- [ ] Memory browser
- [ ] Settings/config UI
- [ ] Channel status dashboard
- [ ] System tray / menu bar presence

### Technical
- [ ] Tauri v2 setup with Rust backend
- [ ] Nuxt v4 frontend (SSG mode for Tauri)
- [ ] Tailwind v4 styling
- [ ] shadcn-vue component library
- [ ] IPC bridge to nanna-core
- [ ] Mobile-responsive layouts
- [ ] Native notifications

## Phase 5: Production Hardening
- [ ] Prometheus metrics
- [ ] Tracing spans for tool calls
- [ ] Cost tracking per session
- [ ] Runtime config reload
- [ ] Per-channel config
- [ ] Tool allowlists/blocklists
- [ ] Rate limiting
- [ ] Error recovery / retry logic

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
| 13 | Supervisor patterns | 🔜 Next |
| 14 | Tauri GUI (desktop) | 🔜 |
| 15 | Deno scripting fallback | Later |
| 16 | Tauri mobile (Android/iOS) | Later |
