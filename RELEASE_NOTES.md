# Nanna v0.3.9-beta.16 — Fail Closed, and Say So

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

### Security (this release)
- **Inbound webhooks fail closed.** Every route verifies its provider signature or shared secret
  before the payload reaches the agent, and a channel with no credential configured now refuses to
  serve (503) rather than accepting anonymous requests. Discord and Slack captures expire on a
  5-minute replay window. `nanna init` mints the Telegram secret and prints the `setWebhook` call.
- **Webhook bodies are authenticated before they are deserialized**, so a hostile payload cannot
  reach the parser at all.
- **Chat markdown is sanitized.** Assistant output, user input, and anything the assistant quotes
  from a tool went into `v-html` unsanitized — `marked` has not sanitized since v5, so an
  `<img onerror>` in model output was live markup with only the Tauri CSP standing between it and
  script execution. Author HTML is now escaped rather than emitted, and link/image URLs are
  scheme-checked against an allowlist.
- **A per-call tool audit trail.** One JSON line per tool call — including refused and not-found
  ones — recorded at the registry chokepoint, so calls made outside the agent loop are covered too.
  Argument *key names* are recorded; values stay out unless you opt in with
  `[tools] audit_log_values`.

### Memory & dreaming (this release)
- **User-stated memories are pinned verbatim** against summarization drift, and the `remember` tool
  can now produce a pinnable memory.
- **A session is compressed once, never re-compressed from a summary** — the drift mitigation that
  stops meaning eroding across passes.
- **A summary can no longer impersonate one of its sources**, and enrichment adds rather than
  rewrites (the Expand prompt was fighting its own guard).
- **One episodic write policy** — chat could not previously remember a failed tool call.

### Agent & context (this release)
- **Repo-aware context** — when the workspace is a git repository, each turn sees a bounded snapshot
  of the branch, uncommitted paths, and recent commits, so the agent knows what work is already in
  flight before it edits. Explicitly stamped "not live" so a stale tree is never mistaken for
  current.
- **Regression attribution** — when a file stops parsing, the agent is told what landed since it last
  parsed.
- **A health probe answering "I am broken" no longer passes.**

### Fixed
- **Every generic webhook shared one conversation** (both the daemon and `nanna serve` copies).
- **`nanna-gui` did not compile, and no CI job would have said so** — the release profile is now
  gated on push and PR, and the `vue-tsc` gate type-checks for real.
- **Memory editing was dead** in the GUI, `@tiptap/core` was never declared, a chart drew nothing,
  and a dropdown had no options.
- **Destructive dialogs said "Confirm"** rather than naming the destructive act, and one guard never
  ran.
- **Two flaky test suites** that asserted wall-clock while measuring something else — they went red
  on loaded CI and green alone.

### Performance
- **SIMD vector ops** (AVX-512/AVX2/NEON) — 768-dim cosine similarity in ~0.1µs
- **GPU compute** (wgpu) for scale above 50k vectors
- **Local inference on Burn** — moved to the shared `Mummu` runner (ROADMAP P12 is now integration-only)

### Architecture
- **17 workspace crates** layered bottom-up by dependency
- **Channel abstraction** — all clients share state via daemon
- **Workspace context** — auto-detects project files for system prompt injection

## Release Checklist

- [x] Create RELEASE_NOTES.md or MILESTONE that freezes scope
- [~] Set up GitHub Actions to build Tauri + daemon sidecar and attach artifacts to Releases
      — *corrected this release:* `release.yml` exists but has **never completed a successful
      dispatch**. Its one recorded run (2026-08-15) failed in 23s on all three platforms at
      toolchain install: it requests `dtolnay/rust-toolchain@stable` while the repo pins a nightly
      in `rust-toolchain.toml`, and the macOS job asks for a `universal-apple-darwin` std that is
      not published. Releases have in practice been built and uploaded by hand.
- [~] Publish signed Windows .msi/.exe installer with bundled daemon sidecar (code signing pending;
      the updater signature is applied at upload time from the local minisign key)
- [ ] Publish signed and notarized macOS .dmg
- [ ] Publish Linux AppImage and/or .deb/.rpm — *corrected: no release has shipped Linux artifacts*
- [x] App launches without terminal; daemon starts automatically
- [x] Add Start Menu / tray / launch-at-login support
- [x] WebView2 handling on Windows
- [x] Document uninstall process
- [x] Add "check for updates" or auto-update mechanism

## Known Issues

- Code signing not yet implemented (SmartScreen warnings expected)
- `release.yml` has never succeeded; Windows artifacts are built and signed locally, and macOS/Linux
  are not published at all
- Burn local runner still in development (in the `Mummu` repo)

## Installation

1. Install [Ollama](https://ollama.com) and pull `qwen3.5:9b`
2. Download installer from [Releases](https://github.com/physics515/Nanna/releases)
3. Run installer (expect SmartScreen warning → "Run anyway")
