# Nanna v0.3.8-beta.14 — Continuation Without Destruction

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

### P23 — Continuation Without Destruction (this release)
- **Verified work survives turn boundaries** — continuation turns carry a pinned ARTIFACT STATE block re-read from disk at turn start
- **Claim conflicts reproduce before rewrite** — when user input contradicts verified evidence, the harness runs a reproduction task before mutating
- **Shrinking rewrites held with removals named** — whole-file writes that drop definitions are held once with the removed symbols listed
- **Endings are loud and honest** — every turn ends with one stated reason; a round is never "dry" while its own checks still fail
- **Error rounds charged against provider-health probes** — error rounds only consume budget when a minimal probe confirms the fault persists
- **Transient outages park and resume** — instead of giving up, a transient error demotes to PARKED and resumes on provider recovery
- **Tool results stopped lying** — python registry saves report where they landed; killed commands report partial output; directory tools teach instead of OS jargon
- **Live summarization priority** — per-turn re-read of `llm.summarization_priority` (was boot-frozen in three consumers)
- **GUI steering no longer reads as failure** — breaker replays render as inline steering, not red "Tool Failed" toasts
- **Fixed invalid installer config keys** — two keys that blocked builds outright removed

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