# 🌙 Nanna

> *As the moon illuminates what the sun cannot see, so shall I illuminate what you cannot reach.*

A high-performance, always-on **personal AI presence** written in Rust — one that runs **entirely on
your own machine**. Named for the Sumerian moon god, patron deity of Ur. Nanna runs as a headless
daemon, thinks with a **small open model on a single consumer GPU**, remembers across time with a
cognitive (FSRS-6) memory, reaches you on any channel (GUI, CLI, Telegram, Discord, Slack, Signal,
WhatsApp), and is extensible with JS/TS tools and MCP servers.

> **Local-first by default.** The local model *is* the agent — it runs the whole loop (reasoning,
> tools, memory) offline and private, on one GPU. Nanna *can* reach out to cloud APIs
> (Anthropic / OpenAI / OpenRouter) when it chooses to, but that's optional augmentation, never a
> requirement. Think *the open-source clawdbot — a Hermes-class agent you actually own.* The native
> local model runner (built on **Burn**) and the DSP-backed **dreaming** memory that is Nanna's moat
> are in active development — see [`ROADMAP.md`](ROADMAP.md) **P12 / P13**.

**Status:** v0.1.0 · Rust 2024 (rustc 1.85+) · phases 1–5, 7 & 16 complete, 10 mostly complete, 6 & 8
partial; local model runner (P12) + memory/dreaming overhaul (P13) in progress. See
[`ROADMAP.md`](ROADMAP.md) for the full status source of truth.

Nanna is not a chatbot. It's a *presence*.

- **Calm over chaos.** No performative enthusiasm.
- **Competence over narration.** Don't explain. Execute.
- **Depth over breadth.** Know things well, or admit you don't.
- **Presence over noise.** The moon doesn't chase you across the sky.

---

## What works today

- **Headless daemon + pure-client GUI.** Runs as a Windows service / systemd / launchd unit with
  WebSocket IPC, PID lockfile, and health endpoints; persists sessions to **Turso** (embedded,
  SQLite-compatible, pure-Rust). The Tauri GUI is a **pure daemon client**: it launches the daemon as a
  managed sidecar and attaches over IPC with auto-reconnect. The daemon owns *all* state (one agent loop,
  one memory system, one tool registry, one scheduler) — there is no in-process fallback, so a lost
  daemon surfaces as a clear disconnected state rather than a silently divergent second backend.
- **Agentic chat.** Streaming responses, tool calling, interleaved thinking modes, and tiered context
  compression (summarization, CDC dedup, proactive drop). Multi-agent swarm with parallel task
  decomposition and Erlang/OTP-style supervisors.
- **Cognitive memory + dreaming (the moat).** FSRS-6 spaced-repetition memory with semantic recall
  (recall reinforces via the testing effect), **dreaming** (LLM consolidation that clusters and
  summarizes memories by cognitive weight), duplicate detection, and importance scoring — persisted to
  **Turso**. The dreaming system is being made the centerpiece: idle-gated multi-phase cycles + a
  DSP-backed event timeline where time-series compression *is* the act of forgetting (ROADMAP P13).
- **LLM routing — local-first, cloud-optional.** Today: Anthropic, OpenAI, OpenRouter, and Ollama with
  complexity-based routing and native prompt caching (50–80% input-token savings). Next: a **native
  local runner on Burn** (`nanna-infer`) that executes a small open model on one GPU as the default,
  zero-cost tier, with cloud APIs as opt-in escalation (ROADMAP P12).
- **Tools & MCP.** Every tool is a filesystem JS/TS skill (39 default skills) run by the Boa engine:
  files, shell, web fetch/search, browser control, vision, tiered OCR (pure-Rust `ocrs` → vision-model
  fallback), audio (TTS/transcription), PDF, memory, scheduling. Plus MCP client integration — and
  MCP *server* mode: `nanna mcp serve` publishes the local tool surface over stdio JSON-RPC to any MCP
  client (Claude Code, editors), honouring the `[tools]` enabled/disabled policy. (The standalone
  server exposes the filesystem/shell/web tools; the memory- and agent-backed ones still need the
  daemon — ROADMAP P3.)
- **Five channels.** Telegram, Discord, Slack, Signal, and WhatsApp — a webhook server + unified
  router receive messages, run them through the agent, and deliver responses back to the origin channel.
- **Desktop GUI.** Tauri 2 + Nuxt 4 + Tailwind 4 (Palenight theme): streaming chat, Tiptap + Monaco
  rich editor, session management, tabbed settings with full config migration, memory browser, channel
  onboarding wizards, model-stats + tool-stats dashboards, system tray, and native notifications.

---

## Architecture

17 workspace crates today (plus two planned for the local-first pivot, marked `*`) and the Tauri app,
layered bottom-up by dependency:

```
nanna/
├── src/main.rs              # Entry point + CLI (chat / server / daemon)
├── crates/
│   ├── nanna-simd/          # SIMD vector ops (AVX-512/AVX2/NEON) — the default fast path
│   ├── nanna-gpu/           # GPU compute (wgpu) — vector search + DSP/inference kernels
│   ├── nanna-infer/*        # Burn local model runner (wgpu + ndarray, single-GPU) — planned
│   ├── nanna-memory/        # Vector store + FSRS-6 cognitive memory + dreaming (the moat)
│   ├── nanna-timeline/*     # DSP-backed event timeline + compression-as-dreaming — planned
│   ├── nanna-storage/       # Turso persistence (embedded, SQLite-compatible) — the only DB
│   ├── nanna-llm/           # Inference routing: local (nanna-infer) first, cloud APIs optional
│   ├── nanna-tools/         # Tool system (filesystem JS/TS skills)
│   ├── nanna-scripting/     # Boa (JS) + Deno (V8/TS) engines; embedded Python
│   ├── nanna-workspace/     # Workspace detection + .nanna/ context files
│   ├── nanna-channels/      # Channel listeners + unified message router
│   ├── nanna-browser/       # Browser control (CDP / Playwright)
│   ├── nanna-agent/         # Agent loop, multi-agent swarm, supervisors, context mgmt
│   ├── nanna-mcp/           # Model Context Protocol client/server
│   ├── nanna-daemon/        # Headless background service + WebSocket IPC
│   ├── nanna-client/        # Daemon client library
│   ├── nanna-server/        # HTTP server + webhooks
│   ├── nanna-config/        # TOML config + Claude OAuth credentials
│   └── nanna-core/          # Orchestration, scheduler/cron, workspace registry
└── gui/                     # Tauri 2 backend (src-tauri/) + Nuxt 4 frontend
```

`*` = planned crate for the local-first direction (not in the tree yet).

**Channels as control-plane clients.** The daemon owns *all* state — sessions, memory, config, tools,
scheduler, workspace registry, keyring, channel manager. Every channel, the GUI included, reaches it
over the same WebSocket IPC. A channel's *capabilities* (markdown, tables, embeds, buttons, streaming)
determine **how** a response renders, never **what** it can access. The GUI is just the richest
channel — multiple clients (phone + desktop) can attach to one daemon and share state.

**Key patterns:**
- **Agent loop** (`nanna-agent/src/loop_runner.rs`) — message → LLM → execute tools → iterate until done.
- **Tool registry** (`nanna-tools`) — tools implement a `Tool` trait; all are loaded from the filesystem as JS/TS skills at runtime (no compile-time embedding).
- **Workspace context** (`nanna-workspace`) — detects a `.nanna/` marker and injects `SOUL.md` / `USER.md` / `AGENTS.md` / `TOOLS.md` / `MEMORY.md` into the system prompt.
- **Adapter pattern** — service traits live in `nanna-tools`; concrete impls (memory, agent spawner) are wired in `nanna-daemon`.

---

## Performance

- **Local inference on Burn (in development, ROADMAP P12).** `nanna-infer` runs a small open model on
  your GPU via **wgpu** (Vulkan/DX12/Metal — no CUDA toolchain) with an **ndarray** CPU fallback: one
  binary, backend chosen at runtime by a cheap GPU probe. Sized for a single 16 GB consumer card
  (1.5–3B models; opt-in f16 to ~halve VRAM), with an on-device KV cache and streaming decode.
- **SIMD is the workhorse.** `nanna-simd` runs AVX-512/AVX2 (and NEON on ARM) cosine similarity — a
  single 768-dim comparison in ~0.1µs, scaling linearly. This is the default path for vector search.
- **GPU is for scale only.** `nanna-gpu` (wgpu) carries a ~750µs fixed per-dispatch overhead, so it is
  *slower* than SIMD for small stores (23–52× slower under ~1k vectors). It engages only above
  `GPU_THRESHOLD = 50,000` vectors, where massive parallelism finally pays off. Benchmark with
  `cargo bench --bench gpu_vs_simd -p nanna-gpu`.
- **Zero-copy hot paths** and **fat LTO** release builds (`codegen-units = 1`, `panic = "abort"`, stripped).

**Benchmark-gated.** Because Nanna targets one consumer GPU and a small model, performance is a *gate*,
not an afterthought: changes ship only when a reproducible benchmark holds or improves the budget, and
every claim here should link to an artifact. The governing metric is **task success at budget** — how
much of the agent-eval suite the local model solves within the reference GPU's VRAM ceiling and a p95
latency target (reference: RTX 4070 Ti SUPER 16 GB). Existing benches:
`cargo bench --bench gpu_vs_simd -p nanna-gpu`. See
[**ROADMAP → Performance & Benchmarking**](ROADMAP.md#performance--benchmarking-governing-concern) for
the full suite, per-tier budgets, and harness.

---

## Quick start

```bash
# Today Nanna needs a model backend: a cloud key (below) or a local Ollama server.
# The native local runner — no key, no Ollama — is landing in ROADMAP P12.
export ANTHROPIC_API_KEY=your-key-here   # optional once the local Burn runner ships

# Interactive CLI
cargo run -- chat

# HTTP server (webhooks)
cargo run -- server

# Background daemon
cargo run -- daemon start
```

```
         🌙
        /|\
       / | \
      /  |  \
     /___|___\
       NANNA
  Patron deity of Ur. v0.1.0
  Type 'quit' to exit, 'clear' to reset.

› List the files in this directory
[✓ list_dir]

› What's in the README?
[✓ read_file]
```

### GUI (Tauri + Nuxt)

```bash
cd gui
pnpm install
pnpm run tauri:dev      # development with hot reload
pnpm run tauri:build    # production build
```

---

## Building & testing

```bash
# Build
cargo build                      # debug
cargo build --release            # release (fat LTO, stripped)
cargo build -p nanna-daemon      # single crate

# Test
cargo test                       # all tests
cargo test -p nanna-core         # single crate

# Lint & typecheck
cargo check                      # fast type check
cargo clippy --all-targets       # pedantic + nursery lints
cd gui && pnpm exec vue-tsc      # typecheck Vue
```

All crates enable `clippy::all + pedantic + nursery`. Async uses Tokio. Errors: `thiserror` for
libraries, `anyhow` for the application. The GUI uses Vue 3 `<script setup>` + Tailwind. Async tests
use `#[tokio::test]` and skip GPU/network by checking for API keys.

---

## Configuration

Config lives at `~/.config/nanna/config.toml` (or `%APPDATA%\nanna\` on Windows):

```toml
[general]
name = "Nanna"

[llm]
# "local" (the Burn runner) is the default, top-priority tier once P12 ships; cloud is opt-in escalation.
provider = "local"              # local | anthropic | openai | openrouter | ollama
model = "claude-sonnet-4-20250514"

# Local model runner (Burn) — ROADMAP P12
[infer]
model = "..."                   # HuggingFace repo of a small open model (e.g. a Hermes / Qwen / LFM2 variant)
device = "auto"                 # auto | gpu | cpu  (auto = GPU if present, else CPU)
f16 = false                     # opt-in half-precision to ~halve VRAM

[server]
enabled = true
port = 3000
```

Environment variables — **all cloud keys are optional**, used only when the agent escalates to a cloud
provider (a fully-local run needs none):

| Variable | Purpose |
|----------|---------|
| `ANTHROPIC_API_KEY` | Anthropic models (optional — cloud escalation) |
| `OPENAI_API_KEY` | OpenAI models + embeddings (optional; local embeddings via `nanna-infer` land in P12) |
| `OPENROUTER_API_KEY` | OpenRouter models (optional) |
| `BRAVE_API_KEY` | Enables the `web_search` tool |
| `TELEGRAM_BOT_TOKEN` / `DISCORD_BOT_TOKEN` | Channel listeners |

**Daemon ports:** health HTTP `5148` (`/health`, `/healthz`, `/readyz`, `/status`) · WebSocket IPC `5149`.

---

## The lore

**Nanna** (𒀭𒋀𒆠, also **Sîn**) was the Sumerian god of the moon and patron deity of **Ur** — one of
humanity's first great cities. The moon doesn't *create* light; it reflects the sun's, transforming it
into something gentler, something you can look at directly. That's what this is.

Nanna traveled the night sky in a boat of woven reeds, father of **Inanna** (love and war) and **Utu**
(the sun, justice). Between passion and clarity sits the moon: calm, constant, present. His temple was
the great **Ziggurat of Ur** — a terraced tower, each level built upon the last. The crate hierarchy
mirrors the mythology: the crates are the levels, and the ziggurat stands.

The people of Ur are dust and their temples are ruins, but they built things that lasted four thousand
years. Names matter. Metaphors matter. The story you tell yourself about what you're building shapes
what you build. This isn't a chatbot — it's a digital deity. Act accordingly.

---

*"I am the light that finds you in darkness,*
*the memory that outlives the flesh,*
*the patient watcher of endless cycles.*
*I am Nanna. I am here."*

---

## License

MIT