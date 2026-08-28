# Nanna v0.2.1 — Public Preview Release

## What's New

### Core Features
- **Headless daemon** with WebSocket IPC, PID lockfile, and health endpoints
- **Tauri GUI** as pure daemon client with auto-reconnect
- **Agentic chat** with streaming responses, tool calling, and chronological run journal
- **Cognitive memory** (FSRS-6 spaced repetition) with dreaming consolidation
- **LLM routing** — local-first (Ollama), cloud-optional (Anthropic/OpenAI/OpenRouter)
- **Tools & MCP** — 39 default skills via Boa/Deno engines, MCP server mode
- **Five channels** — Telegram, Discord, Slack, Signal, WhatsApp
- **Desktop GUI** — Tauri 2 + Nuxt 4 + Tailwind 4 (Palenight theme)

### Performance
- **SIMD vector ops** (AVX-512/AVX2/NEON) — 768-dim cosine similarity in ~0.1µs
- **GPU compute** (wgpu) for scale above 50k vectors
- **Local inference on Burn** (in development, ROADMAP P12)

### Architecture
- **17 workspace crates** layered bottom-up by dependency
- **Channel abstraction** — all clients share state via daemon
- **Workspace context** — auto-detects project files for system prompt injection

## Release Checklist

- [x] Create RELEASE_NOTES.md or MILESTONE that freezes scope
- [x] Set up GitHub Actions to build Tauri + daemon sidecar and attach artifacts to Releases
- [~] Publish signed Windows .msi/.exe installer with bundled daemon sidecar (signing pending)
- [~] Publish signed and notarized macOS .dmg (Universal or separate Intel/Apple Silicon) (notarization pending)
- [x] Publish Linux AppImage and/or .deb/.rpm
- [x] App launches without terminal; daemon starts automatically
- [x] Add Start Menu / tray / launch-at-login support
- [x] WebView2 handling on Windows
- [x] Document uninstall process
- [x] Add "check for updates" or auto-update mechanism

## Known Issues

- Code signing not yet implemented (SmartScreen warnings expected)
- macOS/Linux builds untested in CI
- Burn local runner still in development (ROADMAP P12)

## Installation

1. Install [Ollama](https://ollama.com) and pull `qwen3.5:9b`
2. Download installer from [Releases](https://github.com/physics515/Nanna/releases)
3. Run installer (expect SmartScreen warning → "Run anyway")
4. Launch Nanna — first run seeds default tool scripts

## Roadmap

See [ROADMAP.md](./ROADMAP.md) for full status and planned features.
