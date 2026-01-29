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

## Phase 2: Tools & Channels 🚧
### Tools
- [x] File operations (read, write, list)
- [x] Shell execution
- [x] Web fetch/search
- [x] Memory (remember, recall, reflect)
- [x] Scheduling (remind, list, cancel)
- [x] Browser tools (screenshot, extract, action, evaluate)
- [x] Vision tools (analyze_image)
- [x] Audio tools (TTS, transcription)
- [ ] Authoring tools (runtime tool creation)

### Channels
- [x] Telegram (full API: send, react, edit, delete, pin, poll)
- [x] Discord (REST API: send, react, edit, delete, pin, threads)
- [ ] Slack (Bolt SDK)
- [ ] Signal (signald integration)
- [ ] WhatsApp (Baileys or Cloud API)

## Phase 3: Multi-Agent & MCP
- [ ] Background task spawning
- [ ] Agent-to-agent communication
- [ ] Supervisor patterns
- [ ] MCP client (external tool servers)
- [ ] MCP server mode (expose Nanna tools)

## Phase 4: GUI Application 🆕
**Stack:** Tauri v2 + Nuxt v4 + Tailwind v4

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
| 6 | MCP client | 🔜 Next |
| 7 | Tauri GUI (desktop) | 🔜 |
| 8 | Tauri mobile (Android/iOS) | Later |
