# Nanna — Roadmap

> The single master roadmap **and status source of truth** for Nanna — there is no separate
> `STATUS.md`, `planning/`, or `docs/`. The daily dev routine reads this file, picks the **single
> next unimplemented item**, builds it with tests, ticks the box, and appends a dated note.
> Shipped capability is *described* in [`README.md`](README.md); here it is only tracked.
> Edit surgically; never rewrite wholesale.

**Last updated:** 2026-07-06 (local-first direction pivot) · code snapshot through 2026-03-17
**Repo:** local Cargo workspace, branch `master` — one Rust workspace + a Tauri 2 / Nuxt 4 GUI.
**Stack:** Rust 2024 (rustc 1.85+) · Tokio · **Burn** (wgpu + ndarray) for on-device inference · wgpu 24 · Tauri 2 · Nuxt 4 / Vue 3 / Tailwind 4 · **Turso** (embedded, SQLite-compatible) · Boa + Deno scripting.

> **Direction (2026-07-06 pivot) — local-first by default.** A small open model running on a single
> consumer GPU *is* the agent and does the whole job — full agentic reasoning, tools, and memory —
> entirely on-device (private, offline-capable). Cloud APIs stay reachable as **optional** augmentation
> the local model can choose to call, never a dependency. The always-on multi-channel presence is
> unchanged. The heavy new investment: a best-in-class **Burn** model runner (local inference,
> single-GPU) and the **memory + dreaming** system (Turso-only, DSP-backed time-series) that is
> Nanna's moat. See **P12** (Local Model Runner) and **P13** (Memory & Dreaming) below.

---

## North Star

**Nanna is an always-on, fully-local personal AI *presence* — not a chatbot, and not a cloud
client.** A headless Rust daemon that runs continuously on your own machine, thinks with a **small
open model on a single consumer GPU** (the local model *is* the agent — it runs the whole loop:
reasoning, tools, and memory), remembers across time with a cognitive (FSRS-6) memory, and is
reachable from any channel — GUI, CLI, Telegram, Discord, Slack, Signal, WhatsApp — where the GUI
is *just the richest channel*, never a privileged controller.

**Local is the North Star experience, not a degraded mode.** Everything works offline, private, on
one GPU. Nanna *can* reach out to cloud APIs (Anthropic/OpenAI/OpenRouter) when it chooses to — for a
harder problem, a bigger context, a capability the local model lacks — but that is optional
augmentation the agent invokes, never a dependency. Think "open-source clawdbot / Hermes-class agent
you actually own," not "a frontend for someone else's model."

Two things make it more than a local Ollama wrapper: (1) a **best-in-class in-Rust model runner**
(Burn) that squeezes advanced agentic behavior out of small single-GPU models; (2) a **memory system
whose *dreaming* is the moat** — cognitive consolidation augmented by DSP time-series compression, so
the agent's understanding compounds over time instead of resetting each session.

The long arc still reaches a **personal device mesh** (peer daemons over Tor; your phone's camera a
tool your desktop can call). The bar: a calm, competent assistant that is *there* when you look up —
persistent, multi-channel, autonomous, private, and yours.

Every run should move one phase toward that end state — depth over breadth.

---

## Core Model

Bottom-up crate dependency tiers (workspace crates + the Tauri app). `*` = planned crate for the
local-first direction (does not exist yet):

```
Tier 0  nanna-simd        SIMD vector ops (AVX-512/AVX2/NEON) — the default fast path
        nanna-gpu         GPU compute (wgpu) — vector search >~50k + DSP/inference kernels
          |
Tier 1  nanna-infer*      Burn model runner: local LLM inference (wgpu + ndarray, single-GPU)
        nanna-memory      Vector store, FSRS-6 cognitive memory, dreaming (the moat)
        nanna-timeline*   DSP-backed event/episode timeline + compression-as-dreaming
        nanna-storage     Turso persistence (embedded, SQLite-compatible) — the ONLY DB
        nanna-llm         Inference routing: local (nanna-infer) first · cloud APIs optional
          |
Tier 2  nanna-tools       Tool system (all tools are filesystem JS/TS skills)
        nanna-scripting   Boa (pure-Rust JS) + Deno (V8/TS) engines; embedded Python
        nanna-workspace   Workspace detection, .nanna/ context files (SOUL/USER/AGENTS/…)
        nanna-channels    Channel listeners + unified message router
        nanna-browser     Browser control (CDP / Playwright)
          |
Tier 3  nanna-agent       Agent loop, multi-agent swarm, supervisors, context management
        nanna-mcp         Model Context Protocol client (+ server mode, see P3 caveat)
          |
Tier 4  nanna-daemon      Headless background service, WebSocket IPC
        nanna-client      Daemon client library
        nanna-server      HTTP server, webhooks
        nanna-config      TOML config, credentials
          |
Tier 5  nanna-core        Orchestration, scheduler/cron, workspace registry, dreaming runtime
          |
        gui/src-tauri     Tauri 2 backend + Nuxt 4 frontend (embeds core OR attaches to daemon)
```

**Governing architecture — "channels as control-plane clients":** the daemon owns *all* state
(sessions, memory, config, tools, scheduler/cron, workspace registry, keyring, channel manager).
Every channel — GUI included — reaches that control plane through the WebSocket IPC protocol.
Channel *capabilities* (markdown/tables/embeds/buttons/modals/streaming) determine **how** a
response renders, never **what** a channel can access. Multiple clients (phone + desktop) can attach
to the same daemon and see consistent state.

**Inference model — local-first, cloud-optional (the pivot):** `nanna-llm` is a routing layer, not a
cloud client. The default and intended backend is the **local Burn runner** (`nanna-infer`) executing
a small open model on the user's single GPU (with a CPU fallback) — it runs the *entire* agent loop,
tool use, embeddings, and dreaming-time summarization on-device. Cloud providers
(Anthropic/OpenAI/OpenRouter; Ollama for other local servers) stay selectable and the agent can
*escalate* to them, but a fully-local, offline-capable run is the default, not a fallback. The
existing cross-provider complexity router (P10) is extended so **"local" is simply the
top-priority, zero-cost tier** and cloud is an opt-in escalation.

**Ports:** health HTTP `5148` (`/health`, `/healthz`, `/readyz`, `/status`) · WebSocket IPC `5149` · daemon sidecar (GUI-spawned) `9833`.

---

## Current State (what's real today)

Phases **1–5** and **7** are complete; **10** is mostly complete; **6** and **8** are partial;
**9** is greenfield. The new local-first phases (**P12**, **P13**) are greenfield. Concretely, today Nanna:

- Runs as a **headless daemon** (Windows service / systemd / launchd) with WebSocket IPC, PID
  lockfile, health endpoints, and session persistence to **Turso**; the **GUI attaches** as a client
  with auto-reconnect and falls back to an **embedded** in-process backend when no daemon is running.
- Holds real **chat** with streaming, tool calling, interleaved thinking, and tiered context
  compression; routes across **Anthropic / OpenAI / OpenRouter / Ollama** with complexity-based model
  cascade and native prompt caching (50–80% input savings). *(All inference is still remote-API or
  Ollama today — the native local Burn runner is P12.)*
- Has a **cognitive memory** system (FSRS-6 spaced repetition, semantic recall with testing-effect
  reinforcement, consolidation/"dreaming", duplicate detection) persisted to **Turso**.
- Ships **all tools as filesystem JS/TS skills** (39 default skills) executed by the Boa engine, plus
  **MCP client** integration and an **embedded/tiered OCR** pipeline (pure-Rust `ocrs` → vision-model fallback).
- Connects **five channels** (Telegram, Discord, Slack, Signal, WhatsApp) with a webhook server and a
  unified router that delivers agent responses back to the originating channel.
- Presents a **Tauri 2 + Nuxt 4** desktop GUI: streaming chat, Tiptap+Monaco rich editor, session
  management, tabbed settings with full config migration, memory browser, channel onboarding wizards,
  tool-stats/model-stats dashboards, system tray, and native notifications.

**Storage note:** **Turso** (the `turso` crate — a pure-Rust, SQLite-compatible embedded DB) is
*already the only database*. "Remove SQLite" is a naming/branding cleanup (comments, log strings, the
`SqliteMemoryPersistence` struct name, docs), **not** an engine swap — the SQL dialect, `.db` files,
and `datetime('now')`/`AUTOINCREMENT`/`json_*` usage are all Turso-supported and load-bearing (P13).

**Not yet verified / closed:** no **native local model runner** yet (P12); **dreaming** exists but is
a fixed hourly cron over an O(N²) clusterer with no timeline/DSP layer, and the richer feedback-driven
`DreamingService`/`DreamingRuntime` is dead code (P13); the daemon + embedded-fallback + reconnection
path has **no end-to-end test**; **MCP server mode** is claimed complete but `nanna-server/src/mcp.rs`
does not exist (unverified — see P3); channel webhook **signature verification is a placeholder**
(Discord/Slack); several daemon control actions return `not_implemented`; and there is real
**security/correctness debt** (user-tool path traversal, workspace file traversal, non-atomic memory
writes) tracked below.

---

## Performance & Benchmarking (governing concern)

Nanna targets **one consumer GPU and a small model** — every feature competes for scarce VRAM,
compute, tokens, and watts. Performance is therefore not a phase, it's a **gate**: borrowing DSP's
discipline, *no change ships unless a reproducible benchmark shows it holds or improves the budget*,
and **every performance claim in [`README.md`](README.md) must link to a benchmark artifact.** This
section defines the objective measurements the daily dev routine uses to judge progress.

### Governing metric — "capability at budget"

One number every sub-benchmark ladders up to:

> **Task success @ budget** — the fraction of the **agent-eval suite** the *default local model*
> completes correctly while staying inside the reference GPU's VRAM ceiling and a **p95 wall-clock
> target per task**. Secondary: **capability density** = task-success per GB of VRAM (rewards getting
> more agent out of a smaller model), and **cost of the escape hatch** = fraction of tasks that had to
> escalate to a cloud API (lower = more self-sufficient locally).

A faster model that fails more tasks is not an improvement; a smaller model that holds task-success is.

### Reference hardware (pin the denominators)

| Tier | Hardware | Purpose |
|------|----------|---------|
| **Reference GPU** | RTX 4070 Ti SUPER 16 GB (Vulkan/wgpu) + AMD Zen 4 (AVX-512) | primary target; the number we report |
| **Low-VRAM GPU** | 8 GB card | budget guardrail — forces f16 + smaller tier to still pass |
| **CPU-only** | Zen 4 / Apple Silicon (NEON) | offline, no-GPU fallback path |

All reported numbers name the tier, the model + quantization, and the commit.

### Harness

- A **`nanna-bench` crate** (criterion 0.5 + `html_reports`, the pattern already used in `nanna-gpu`),
  plus per-crate `benches/`. Reproducible: fixed seeds, warmup, pinned model weights, release profile
  for reported numbers (`--profile dev` only for iteration).
- Results land under `target/criterion/`; a committed **`bench/BASELINE.md`** (or JSON) holds the
  reference-hardware numbers the daily routine diffs against.
- Runtime telemetry already exists and feeds the same metrics live: `model_request_log`,
  `tool_call_log`, `tool_stats_hourly/daily` (P95, throughput), per-model stats tracker.

### Benchmark suites (each metric gets a target once a baseline exists)

**1. Inference — `nanna-infer` / Burn (the new hot path)**
- Time-to-first-token (TTFT); **prefill tok/s**; **decode tok/s**; tokens/sec vs context length.
- Peak **VRAM**; model **load time** (cold vs warm cache); GPU (wgpu) vs CPU (ndarray); **f16 vs f32**.
- Sweep model sizes (0.5B / 1.5B / 3B) × context (1k/8k/32k).
- **Correctness gate:** byte-parity of logits + a short greedy sequence vs a reference (Candle/Ollama), à la laurelane — a fast model that diverges is a failed benchmark, not a win.

**2. Memory & vector search**
- Recall p50/p95 latency; local-embedding throughput (tok/s on the MiniLM path); vector-search latency
  at N = 1k/10k/50k/100k (**reuse `gpu_vs_simd`** — the SIMD→GPU crossover is already measured at ~50k);
  `bulk_load` startup time; RAM per 100k memories.

**3. Dreaming & DSP compression (the moat — measure it, don't hand-wave)**
- Dream-cycle wall-clock; memories/sec consolidated; clustering time (O(N²) baseline → HNSW target).
- **Compression ratio** and **reconstruction error** of the DSP timeline; **information retention** —
  recall quality (hit-rate / answer accuracy) on a fixed probe set *before vs after* a dream cycle. The
  headline claim to prove: *dreaming shrinks the memory footprint while holding (or improving) recall.*

**4. Agent loop — end-to-end (where the governing metric lives)**
- Task-success rate on the **agent-eval suite**; tokens/task; **tool-call validity rate** (malformed
  vs valid calls — critical for small models); iterations/task; wall-clock/task; tool-execution overhead.

**5. Resource-budget guardrails (hard ceilings — fail CI if exceeded)**
- Binary size; idle daemon RAM; VRAM ceiling per tier; cold-start time; tokens/turn. These are the
  "small resource budget" contract — a change that blows a ceiling is rejected regardless of speed.

**6. Efficiency (P10 tie-in)**
- Prompt-cache hit rate; tokens saved by routing/compression/dedup; local-vs-cloud task split; $/task when cloud is used.

### Regression gating & reporting

- [ ] **`nanna-bench` crate** + move/extend the `nanna-gpu` benches into a unified suite.
- [ ] Define the **agent-eval suite** (a fixed set of scored tasks) — the denominator for task-success.
- [ ] Set **per-tier budgets** (VRAM ceilings, min decode tok/s, max TTFT, max dream-cycle time) in `bench/BASELINE.md`.
- [ ] **CI gate**: run a fast benchmark subset on every PR; fail if a budget regresses > threshold (e.g. >10% slower, or over a VRAM ceiling).
- [ ] **Memory-retention harness**: fixed probe set + before/after-dream recall scoring.
- [ ] **Inference parity harness**: logit/sequence parity vs reference for every Burn model port.
- [ ] **Perf dashboard**: extend the existing model-stats/tool-stats GUI pages with live TTFT / tok-s / VRAM / cache-hit, and a per-release trend view.
- [ ] Wire the daily dev routine to **update `bench/BASELINE.md`** after each perf-affecting change and cite artifacts in commit messages.

**Current benchmarking state (honest):** only `nanna-gpu` has benches (`gpu_vs_simd`,
`gpu_vs_simd_extended`, `gpu_vs_simd_quick`, `threshold_benchmark`; criterion, html reports) — this is
where the GPU-vs-SIMD reversal data comes from. Runtime `model_request_log` / `tool_call_log` /
`tool_stats_*` capture live latency/throughput/errors. There is **no** inference, memory, dreaming, or
end-to-end benchmark, **no** CI gating, **no** eval suite, and **no** defined budgets yet — building
that harness is the first performance work, and a prerequisite for honestly claiming P12/P13 progress.

---

## Phases

### P1 — Core Infrastructure ✅
SIMD vector ops (AVX/AVX2), GPU compute (wgpu), Turso persistence (embedded, SQLite-compatible),
vector store + conversation memory, LLM clients (Anthropic/OpenAI/OpenRouter/Ollama) with streaming +
tool calling, agent loop with context management, scheduler (heartbeats, cron). **Shipped.**

### P2 — Tools & Channels ✅
File/shell/web tools, memory tools (remember/recall/reflect), scheduling, browser tools, vision
(analyze_image), tiered OCR, audio (TTS/transcription), PDF (text + image extraction). All tools
migrated to filesystem JS/TS skills (Boa + Deno). All five channels (Telegram/Discord/Slack/Signal/WhatsApp)
with send/react/edit/delete/pin/threads/media where supported. **Shipped.**

### P3 — Multi-Agent & MCP ✅ (one caveat)
MCP client (stdio + HTTP/SSE transports, tool discovery, adapter into nanna-tools), background task
spawning, agent-to-agent messaging (mailbox), Erlang/OTP-style supervisors (RestartPolicy,
SupervisionStrategy OneForOne/OneForAll/RestForOne, HealthCheckConfig). **Shipped**, except:
- [ ] **Verify or build MCP *server* mode** — doc claims `crates/nanna-server/src/mcp.rs`; that file
      does not exist and no MCP refs found under `nanna-server/src`. Confirm shipped location or implement
      (stdio server, tool/resource/prompt registration, HTTP mode, tool filtering, auth, streaming).
- [ ] Supervisor health check runs a placeholder, not a real agent loop (`supervisor.rs:496`).
- [ ] Supervisor recovery tracking recovers on first success instead of counting consecutive successes (`supervisor.rs:577`).

### P4 — GUI Application ✅
Tauri 2 + Nuxt 4 + Tailwind 4, 80s-hacker Palenight theme. Streaming chat with markdown, session
management, tabbed settings + full config migration + import/export, tool-call visualization,
memory browser, channel onboarding wizards (all five), model-stats + tool-stats dashboards, system
tray, native notifications, UI component library, mobile-responsive layouts. **Shipped.**
Open polish: mobile testing on real devices (Tauri Android/iOS), per-tool drill-down finish, latency sparklines.

### P5 — Agent Swarm & Context Management ✅
Swarm coordinator (parallel decomposition, dynamic sub-agent spawning, result aggregation, critical-path
metrics), context management (sliding window, per-tool proportional truncation, incremental
summarization + cache, CDC deduplication, tiered compression at 40%/threshold/hard-cap), thinking
modes (Instant/Low/Medium/High/Maximum), task-delegation `task` tool, token-budget tracking, code
analysis tools (outline/search/structure). **Shipped.**
Open: swarm execution view in GUI (CriticalPathMetrics tracked but not visualized); stream partial swarm results.

### P6 — Production Hardening 🚧 (partial)
Done: outbound rate limiting (per-provider token buckets), error recovery / exponential backoff with
jitter, priority message queue, graceful 429 handling, health endpoint, PID file. Open:
- [ ] **Prometheus metrics** — new `nanna-metrics` crate (`NannaMetrics`: llm_request_duration,
      llm_tokens_total, tool_execution_duration, channel_messages/errors_total, queue_depth,
      active_sessions, memory_entries); expose via `/metrics` on the Axum health server + a GUI event.
- [ ] **Structured tracing spans** — hierarchy Session → Agent Loop → LLM/Tool Call, capturing
      name/duration/IO-size/success via `#[tracing::instrument]` + `info_span!`.
- [ ] **Cost tracking** — `CostTracker` (pricing table per model, `UsageRecord` per call), aggregate by
      session/day/month/model/tool, surface in GUI.
- [ ] **Runtime config reload** — watch `config.toml` with `notify` (debounce 500ms), validate before
      apply, apply without restart, emit `config-change` events.
- [ ] **Per-channel config** — `[channels.<name>.agent]` sections (system_prompt/model/max_tokens/tools allowlist).
- [ ] **Tool allowlists/blocklists** — `ToolPolicy` (global allow/block + per-channel + per-user for multi-user channels).
- [ ] **Log rotation** — `tracing-appender` daily rotation, max ~7 files (logs currently accumulate unbounded).
- [ ] Reach **0 clippy warnings** — 3 deferred items remain: refactor `handle_daemon_command`
      (main.rs ~1442-1636, `too_many_lines`), move mid-function `use nanna_client::…` to top (main.rs ~1576,
      `items_after_statements`), drop unused `async` on `is_daemon_running` (main.rs ~1694, `unused_async`).

### P7 — Rich Input & Editor ✅
Tiptap editor with Monaco code blocks replacing the chat textarea: formatting, headings, lists,
blockquotes, links, images, horizontal rules, markdown shortcuts, language selector, copy button,
Palenight theme sync, floating BubbleMenu, slash commands, drag-drop blocks, mobile toolbar,
undo/redo, streaming-while-editing. **Shipped.** Open (optional): tables, toggleable line numbers,
CRT glow on focus, localStorage draft persistence, Vim keybindings, reuse editor for memory/system-prompt/workspace-file editing.

### P8 — Clawdbot Parity 🚧 (partial)
Done: daemon binary + service install, IPC protocol, session persistence, `nanna-client`, GUI↔daemon
wiring, agent integration, OAuth in daemon, tool-name aliases, webhook server (all endpoints),
channel listeners (Telegram/Discord/Slack), unified router + response routing, cron system, sub-agent
scaffolding, shared OS keyring, daemon-side workspaces/config/scheduler/tool-authoring. Open:
- [ ] **End-to-end daemon testing** (High) — start daemon, connect client, run a conversation, verify
      persistence + embedded fallback + reconnection (currently untested).
- [ ] **Per-channel sessions** (High) — map `channel_id:chat_id → session_id` so each chat/DM gets
      isolated context (all messages currently share one context).
- [ ] **Response formatting per channel** — a `ResponseFormatter` driven by `ChannelFeatures` bitflags
      (strip markdown for Signal, tables→text for Telegram, embeds for Discord, Block Kit for Slack).
      Bitflags exist but every channel currently receives identical raw text.
- [ ] **Client API completeness** — add `SchedulerApi`/`WorkspaceApi`/`ChannelApi` + typed event subscription to `nanna-client`.
- [ ] **HEARTBEAT.md execution** — parse/run a workspace file of periodic tasks (inbox, calendar,
      monitoring), `quiet_hours` config, proactive outreach, history (currently only a scheduler task type).
- [ ] **Sub-agent named sessions** — `spawn_child_session()`, labels, inter-session messaging, timeouts, result callbacks, GUI monitor.
- [ ] **TTS multi-provider** — add ElevenLabs + local Piper (only OpenAI wired); per-channel TTS config; voice-note sending; audio cache.
- [ ] **Browser relay Chrome extension** (Low/High) — MV3 extension ↔ daemon relay (proposed port 5150),
      feed the LLM the accessibility tree (not raw DOM); tools `browser_relay_{snapshot,action,screenshot}`.
- [ ] **Paired devices / nodes** — defer to P9 (Tor P2P) rather than a standalone mDNS/WebSocket scheme.
- [ ] Gateway control: `/restart` + `/status` as channel commands, full backup/restore archive, self-update via GitHub releases.

### P9 — Multi-Device Swarm (Tor P2P) 🌱 (not started)
Personal device mesh over Tor hidden services — zero-config, encrypted, no port forwarding. Every
daemon gets a persistent Ed25519 identity + `.onion` address; peers invoke each other's tools
(`remote:phone:camera_snap`). ~3,600 LOC / ~2–3 weeks estimated. New crates:
- [ ] **`nanna-identity`** (~500 LOC) — Ed25519 keypair, v3 onion derivation, fingerprint
      (`XXXX-XXXX-XXXX-XXXX`), encrypted-at-rest `~/.nanna/identity.json` (Argon2 KDF + AES-256-GCM, zeroized).
- [ ] **`nanna-tor`** (~800 LOC) — embedded `arti` client (~30s bootstrap) with system-Tor fallback,
      hidden-service publishing, outbound `.onion` HTTP, bootstrap progress. Gate behind a feature flag (+10–20MB).
- [ ] **`nanna-mesh`** (~1,500 LOC) — QR/`nanna://pair` discovery (peers in `~/.nanna/peers.toml`),
      signed JSON tool_request/response protocol over Tor, default-deny trust model (`ToolPolicy`,
      require_approval, per-peer rate limit), audit log, relay wiring remote tools into the local registry.
- [ ] **GUI** — peer management page, identity management (view/rotate/export), Tor status widget.
- [ ] **Claude Code / external-agent bridge** — HTTP/SSE transport on the MCP server + peer-tool registration + auth.
- [ ] Key rotation announcement, identity backup (BIP-39?), Tor-state caching, mobile (arti on Android) investigation.

### P10 — Token Efficiency & Cost Optimization ✅ (mostly)
Done: Anthropic + OpenAI native prompt caching + hit tracking, cross-provider model routing with
complexity classifier + tool-call-only routing + first-message override, aggressive tool-output
summarization, progressive distillation (rolling summary every N turns), tool-result eviction,
CDC message-level dedup, per-model stats tracker + persistence + stats-informed routing. Open:
- [ ] **LLMLingua-style prompt compression** (needs local GPU model, e.g. Phi-3/Qwen2 via Ollama; perplexity token scoring, selective).
- [ ] **Structured tool output schemas** — audit tool verbosity, optional `output_schema` on `ToolDefinition`, JSON output mode.
- [ ] **Better token estimation** — replace `len()/4` with tiktoken-rs (OpenAI) or family-aware
      multipliers (3.5 code / 4 English / 2 CJK); account for per-message framing (~100 tok) and
      truncation-marker text. Current heuristic causes ~20–30% overflow/underutilization.
- [ ] Streaming cache tracking (`loop_runner.rs:834`) — parse usage from `message_start` for accurate cache stats.

### P11 — Correctness, Security & Architecture Debt 🚧 (new — cross-cutting)
Concrete, actionable items with `file:line` anchors. **This is the near-term backlog the daily
routine should drain first.**

**Security (do first):**
- [ ] **User-tool path traversal** — `UserToolManager::save_tool` joins `{name}.json` unsanitized; a
      name like `../../etc/cron.d/evil` escapes the tools dir. Enforce `^[a-z][a-z0-9_]{0,63}$` in
      `create_tool` + `CreateToolTool` (same validation skills already use).
- [ ] **Workspace file traversal** — `save_workspace_file` joins the `file` param unvalidated
      (`../../etc/passwd` escapes). Canonicalize and assert containment before writing.
- [ ] **Discord webhook signature** (`webhook.rs:306`) trusts any non-empty header — add Ed25519 (`ed25519-dalek`).
- [ ] **Slack webhook signature** (`webhook.rs:438`) is a placeholder — add HMAC (`ring`/`hmac`).
- [ ] Harden `delete_skill`'s `remove_dir_all` (symlink check / soft-delete); stronger user-script sandboxing.
- [ ] Harden memory extraction against prompt injection (raw conversation is embedded in the extraction prompt).

**Correctness bugs:**
- [ ] `parse_model_id("gpt-4o")` returns `("anthropic","gpt-4o")` and fails silently — infer provider from name prefix (`gpt-*`→openai, `claude-*`→anthropic, `llama*`/`:tag`→ollama).
- [ ] **Atomic memory persistence** — `save_memories` writes in place; a crash mid-write corrupts the store. Use `tempfile` → write → `fs::rename`.
- [ ] **Memory merge** (`memory/service.rs:207`) — `Update` creates a new memory instead of merging.
- [ ] **Tool-memory workspace scope** — `MemoryServiceAdapter::store()` always creates global memories; the `remember` tool ignores workspace scope. Thread workspace context through.
- [ ] **Context budget for small models** — `truncate_context` uses hardcoded `MAX_CONVERSATION_TOKENS` (132k) while `calculate_dynamic_tool_budget` is model-aware, so a 32k Ollama model gets wrong math. Thread model limits everywhere.
- [ ] Orphaned-message on failure — embedded mode stores the user message before the loop; a mid-loop failure leaves no assistant reply. Store a partial error message instead.
- [ ] `not_implemented` daemon control actions: Regenerate message (`control.rs:416`), Tool enable/disable (`control.rs:1155`), Channel status (`control.rs:1558`, needs ChannelManager), Uptime (`control.rs:1636`, needs start timestamp), non-destructive `peek_mailbox` (`control.rs:578`).
- [ ] Windows service `install/uninstall/start/stop` return errors (`service.rs:136`) though runtime works via `windows_service.rs`.
- [ ] Server stats not wired to shared daemon state (`server.rs:882`).
- [ ] MCP server notifications logged but not handled (`transport.rs:148`).
- [ ] JS tools don't parse parameter schemas from manifests (`scripting/tool.rs:188`).
- [ ] Tool-manager consistency: `update_tool` mutates memory before save (diverges on write failure → clone/mutate/save/swap); `create_user_tool` swallows registration errors in `if let Ok`; no duplicate-name check; `enabled:false` tools still execute; no `ToolRegistry::unregister` (deleted tools stay callable until restart); non-string enums dropped in `parse_params_from_schema`.
- [ ] Leaked `embedded_run_states` entries on failed/panicked runs (only removed on success).
- [ ] `create_llm_client_for_model` builds a fresh HTTP client every call — cache `LlmClient` by model ID, invalidate on credential change.

**Architecture debt:**
- [ ] **Decompose `gui/src-tauri/src/lib.rs`** (8,163-line monolith) into `commands/{chat,sessions,memory,settings,channels,workspaces,scheduler,tools,system}.rs`, `llm/{routing,truncation,summarization}.rs`, `state.rs`.
- [ ] **Unify embedded vs daemon agent loop** — embedded path (~280 lines) duplicates the daemon's `AgentService`; make `nanna-agent::AgentContext` the single source of truth and have embedded delegate to `AgentService`.
- [ ] Split `control.rs` (1,523 lines) into `control/{scheduler,workspace,config,system}.rs`; reduce Backend's ~50 near-identical proxy methods with a macro.
- [ ] Split `settings.vue` (1,483 lines) into per-tab components.
- [ ] Refactor over-long `main.rs` command handlers (~1099, ~1221).

### P12 — Local Model Runner (Burn) 🌱 flagship (the pivot)
**Goal:** a new `nanna-infer` crate that runs small open models **natively in Rust on a single
consumer GPU** as the default, first-class inference backend — no Ollama, no cloud required. The
local model runs the whole agent loop. Blueprint proven in `physics515/laurelane` (Burn 0.21, from-scratch
Qwen2.5/LFM2/MiniLM, validated on an RTX 4070 Ti SUPER 16GB).

- [ ] **Crate `nanna-infer` on Burn** — `burn = { version = "0.21", default-features = false, features = ["std","ndarray","wgpu","fusion","autotune","store"] }`. Model code generic over `B: Backend`.
- [ ] **One binary, dual backend, runtime probe** — compile BOTH `Wgpu` (Vulkan/DX12/Metal, no CUDA toolchain) and `NdArray` CPU; a cheap `wgpu::Instance::enumerate_adapters` probe (cached in `OnceCell`) picks GPU if present, else CPU. No feature-split builds. (laurelane `use_gpu()` pattern.)
- [ ] **First model: a Hermes-class function-calling small model** — a from-scratch Burn decoder (start from laurelane's Qwen2.5 / LFM2 modules: RmsNorm + GQA + RoPE + SwiGLU, tied lm_head) sized for one GPU (1.5–3B). Prove tool-calling quality is good enough to run the loop.
- [ ] **Weight loading** — HF safetensors via `burn-store` `SafetensorsStore` + `PyTorchToBurnAdapter` + a `CastFloatAdapter` (bf16→f32/f16); checked load (fail on missing/unused keys). Stream weights from HF to a per-user model cache (resume `.part`, resources-dir first).
- [ ] **Tokenization + chat format** — HF `tokenizers` crate; ChatML (or the chosen model's) template built explicitly; correct special/EOS tokens.
- [ ] **Fast decode** — per-layer KV cache (+ conv-state cache for hybrid models like LFM2); on-device `argmax` so only the winning index syncs to CPU; sampling (temp/top-p) beyond greedy; **streaming tokens** to Tauri events + channels; cooperative interrupt check between tokens (cancellation).
- [ ] **Single-GPU VRAM budgeting** — a size-tier picker (larger model on GPU, smaller on CPU) and an opt-in **f16** path (`Wgpu<half::f16, i32>`) to ~halve VRAM; account for KV cache + display headroom (3B f32 ~12GB is tight on 16GB).
- [ ] **Local embeddings** — a from-scratch MiniLM-class sentence-embedder in Burn (ndarray/CPU) to serve the memory `embed_fn` fully offline (replaces the API `EmbeddingClient` on the local path). Fixes the "no local embeddings" gap.
- [ ] **Wire in as `Provider::Local`** — add the variant to `nanna-llm::Provider`, dispatch `complete`/stream/tool-calling to `nanna-infer`; make it the **top-priority tier** in the P10 complexity router so cloud is opt-in escalation. Parse tool-calls from local model output into the existing `ContentBlock::ToolUse` shape.
- [ ] **Correctness gate** — parity-test each Burn port against a reference (Candle or a local Ollama run of the same model): single-forward top-k logits + a short greedy sequence must match. This is how laurelane trusts its reimplementations.
- [ ] **Model management UX** — GUI: browse/download/select model, tier + f16 toggles, VRAM estimate, download progress; config `[infer]` section (model repo, cache dir, device override, f16).
- [ ] Later: training/fine-tune loop (Burn supports it); LoRA adapters; quantization (int8/int4) for bigger models on the same GPU; vision/OCR models on the same runner (retire the Candle OCR path).

### P13 — Memory & Dreaming: the moat (Turso-only + DSP time-series) 🌱 flagship (the pivot)
**Goal:** make **dreaming** (cognitive consolidation) the differentiator — a multi-phase, idle-gated,
feedback-driven process, extended with a **DSP-backed event timeline** where time-series compression
*is* the act of forgetting/consolidating. All on Turso, all local.

**Turso-only cleanup (do first — pure hygiene, no engine change):**
- [ ] Rename `SqliteMemoryPersistence` → `TursoMemoryPersistence` (`nanna-daemon/src/memory_persistence.rs`; refs in `server.rs`); align with the already-correct `TursoMemoryStorage`.
- [ ] Purge the word "SQLite" from code comments, log/`warn!` strings, and doc-comments (storage lib.rs/Cargo.toml; daemon persistence/session/control/server; memory service/lib; GUI `sqlite_*` var names) → "Turso"/"the database". **Do not** change SQL, `.db` files, or `datetime('now')`/`AUTOINCREMENT`/`json_*`.
- [ ] Delete stale `crates/nanna-daemon/src/server.rs.bak`. Pin `turso` precisely (0.x is pre-1.0). Add a CI guard that fails if `rusqlite`/`libsql`/`sqlx` ever enters the dep tree. (Note: a transitive `libsqlite3-sys` comes from RustPython in `nanna-scripting`, separate concern.)

**Best-in-class dreaming:**
- [ ] **Unify the two stacks** — the running app calls low-level `MemoryService::consolidate()` while the richer `DreamingService`/`nanna-core::DreamingRuntime` (feedback, gates, promote/demote) is dead code. Make `DreamingService` the single orchestrator via `create_dreaming_executor`; delete the GUI branch (`lib.rs:8462`) + daemon `MemoryAction::Consolidate` duplication.
- [ ] **Idle-gated, multi-phase dream cycle** (like sleep, not a fixed hourly cron): track last-activity; after N min idle (or memory-pressure) run phases — (a) purge-expired + testing-effect flush, (b) **true merge/dedup**, (c) cluster-consolidate by FSRS weight band, (d) expand high-weight, (e) DSP timeline compression (below). Emit progress events.
- [ ] **Implement the missing true merge** — `IngestAction::Update` currently falls back to create/reinforce (`service.rs:300`); add content-level merge so dreaming deduplicates instead of accreting near-duplicates.
- [ ] **Indexed clustering** — replace the O(N²) greedy single-pass `cluster_memories()` with HNSW/IVF candidate neighbors + connected-components/HDBSCAN over `composite_cluster_score`; scales past the ~50k in-RAM ceiling.
- [ ] **Feedback-driven FSRS** — wire real signals (thumbs, corrections, tool-success/failure) into `DreamingService::record_feedback` so importance is learned, not static.
- [ ] **Local dreaming** — run `summarize_fn` on the local Burn model (P12) so consolidation is fully offline; persist the `SummaryCache` (currently in-memory, lost on restart).

**DSP-backed time-series / event-timeline memory (compression-as-dreaming):**
- [ ] **`nanna-timeline` crate + append-only event log** — `MemoryEvent { id, ts, kind, workspace_id, content, embedding, salience, source_ids }` in a new Turso migration; the raw episodic stream (messages, tool calls, recalls, outcomes) on a wall-clock axis. `MemoryEntry` stays the semantic/fact layer; episodes consolidate *into* facts during dreaming.
- [ ] **Resample the timeline into per-signal series** — salience(t), access-rate(t), emotional valence(t), per-cluster topic-activation(t).
- [ ] **DSP compression = dreaming over time** — keep the recent window at full sample rate; for older windows decimate/wavelet-drop low-energy detail with the **keep-rate driven by FSRS `power_law_retrievability`** — sharp near-term detail, blurred long-term gist. Lift DSP's pure `simplify_with_aggressiveness` + slope-change simplifier + `splimes::auto_interpolate` (see design notes); store decimated windows / coeff blobs as Turso `f32` BLOBs.
- [ ] **Peak detection seeds consolidation** — DSP peak/energy detection marks salient moments → promote those episodes to facts + boost importance; long flat stretches → compress to Essence/drop. Ties the timeline back into the existing FSRS weight bands.
- [ ] **Single-GPU DSP kernels** — implement FFT/wavelet/convolution as wgpu compute shaders in `nanna-gpu` (alongside `CosineSimilaritySearch`), with a CPU fallback in `nanna-simd`. No external DSP service.
- [ ] **Decision — Turso-only vs DSP `.dspseg`:** DSP normally keeps measurements in `.dspseg` files *outside* libSQL. To stay Turso-only, lift DSP's *pure algorithms* (`simplify_with_aggressiveness`, `splimes`) and store reduced points in Turso BLOBs, rather than depending on DSP's `SegmentStore`/`Database`. (Revisit if the timeline outgrows Turso.)
- [ ] **Make it demoable** — GUI dream-log + a salience **spectrogram/waterfall** over time (consolidation lineage `consolidated_from`/`generation` already exists). This is the "unique sauce" screen.
- [ ] Also from backlog: HNSW persistent vector index (avoid full `bulk_load` into RAM); emotional valence; memory-graph edges; dedup-before-store; extraction filtering (<50 chars).

---

## Feature backlog (grouped — lower priority, pull as capacity allows)

These are aspirational per-subsystem enhancements distilled from the old planning docs. Grouped to
keep the phases readable; promote individual items into a phase when they become active work.

- **Memory:** HNSW/IVF indexing for large stores; persistent vector index (Turso, avoid full reload);
  f16 embedding compression + GC via "dreaming"; memory graphs (relationships); emotional valence;
  importance decay; active forgetting; narratives; per-query similarity threshold; export/import to
  Markdown; embedding-dimension migration + re-embed on provider change; extraction filtering (<50 chars);
  dedup-before-storage; background consolidation with progress events; memory categories/tags.
- **LLM providers:** add Google Gemini, Mistral, Grok (xAI); custom OpenAI-compatible endpoints; model
  capability matrix (skip incompatible models in fallback); model-discovery cache (5-min TTL); typed
  errors instead of string matching; respect `retry-after` headers; OAuth refresh retry; provider
  health dashboard; investigate GitHub Copilot API masking.
- **Channels:** per-channel feature builders (Discord components/embeds/voice, Slack Block Kit/Connect/app-home,
  Telegram inline mode/media groups/keyboards/channel posting, WhatsApp templates/catalog/status,
  Signal groups/attachments/disappearing); message-ID dedup (webhook+listener); auto transport-mode select;
  circuit breaker + dead-letter queue + queue persistence; adaptive/per-channel rate limits; persist inter-agent messages to Turso.
- **Scheduler/cron:** natural-language scheduling (`chrono-english`); per-job timezone (`chrono-tz`);
  job dependencies/chaining; job templates; missed-job handling on startup; retry policy; per-job
  timeout + running-lock; isolated sessions for scheduled tasks; history retention; safer delete-by-name; GUI cron builder.
- **Workspaces:** persist the registry (lost on restart); `.nanna/.lock` concurrent-access guard;
  enforce ~8k-token context budget (truncate/summarize on overflow); daily-memory rotation/archival;
  auto-discovery on startup (depth-limited); inheritance (monorepo parent/child); rename; git diff of
  `.nanna/`; per-workspace model prefs; sharing/export archive; customizable templates; enforce validity.
- **Tools:** agent-callable `UpdateTool`/`DeleteTool`; non-blocking fs I/O (`tokio::fs`/`spawn_blocking`);
  tool call caching (idempotent); versioning/rollback; duplicate-name detection; dangerous-tool
  confirmation; circuit breaker; analytics; tool marketplace/sharing; WASM tool support; **Python tool support** (currently JS/TS only).
- **GUI:** command palette (Cmd/Ctrl+K); full-text search across sessions; export conversations
  (MD/PDF/JSON); context-budget visualization; live run view (iteration, active tools, token burn-rate,
  Gantt timeline); drag-drop file upload; split view; font-size + accent-color controls; ARIA/keyboard
  accessibility; Vue error boundary; lazy-load Monaco; theme-token audit.
- **Storage:** DB migrations system; WAL mode; backup/restore. *(Turso-only is decided — the "SQLite" naming cleanup lives in P13, not an engine swap.)*
- **SIMD/GPU:** verify AVX-512 + add ARM NEON (Apple Silicon/mobile, critical for mobile); benchmark
  vs `simsimd`; GPU optimizations to lower the SIMD→GPU crossover from ~50k toward ~5k vectors
  (persistent GPU buffers, batched multi-query, async transfer/compute overlap, raw-Vulkan hot path);
  dynamic/hardware-aware GPU threshold + multi-vendor testing (NVIDIA/AMD/Intel Arc); `[gpu]` config section.
- **Observability/testing:** cross-agent distributed tracing; agent health metrics; integration tests
  for multi-agent scenarios; chaos testing; message-passing benchmarks.

---

## Immediate next actions (top of queue)

Reordered around the local-first pivot (P12/P13 lead), with the highest-value safety items kept in view.

1. **Turso-only cleanup** (P13) — fast, pure hygiene that sets the direction: rename `SqliteMemoryPersistence`, purge "SQLite" strings, delete `server.rs.bak`, add the CI dep-guard.
2. **`nanna-infer` Burn skeleton** (P12) — one binary, dual `wgpu`+`ndarray` backend, runtime GPU probe, load one small model, greedy decode: prove local inference end-to-end on the dev GPU.
3. **Local embeddings in Burn** (P12) — MiniLM-class CPU embedder wired into the memory `embed_fn` → fully-local memory (no API embeddings).
4. **`Provider::Local` in the router** (P12) — dispatch completion/stream/tool-calls to `nanna-infer` and make local the top-priority (zero-cost) tier; cloud becomes opt-in escalation.
5. **Unify + upgrade dreaming** (P13) — one `DreamingService` orchestrator, idle-gated multi-phase cycle, true merge, local `summarize_fn`.
6. **`nanna-timeline` + compression-as-dreaming** (P13) — append-only event log in Turso + lift DSP's `simplify_with_aggressiveness`/`splimes` as the timeline compressor keyed by FSRS retrievability.
7. **Fix the two path-traversal holes** (P11 security) — user-tool names + workspace file writes.
8. **End-to-end daemon test** (P8) — the daemon/embedded/reconnect story is still unverified.

---

## Design decisions & reference notes

Preserved from the consolidated planning docs so the rationale survives. These are *facts/decisions*, not tasks.

### GPU vs SIMD — the benchmark reversal (load-bearing)
Empirical benchmark (2026-02-07, AMD Zen 4 AVX-512 + RTX 4070 Ti SUPER, wgpu/Vulkan, 768 & 1536-dim):
**GPU never beats SIMD up to 10,000 vectors.** 768-dim results — 100 vec: SIMD ~15µs vs GPU ~780µs
(**52× slower**); 1,000: ~148µs vs ~3.4ms (**23×**); 5,000: ~740µs vs ~4.1ms (5.5×); 10,000: ~1.48ms
vs ~5.22ms (3.5×). The GPU has a **~750µs fixed per-dispatch overhead** (buffer upload ~200µs +
dispatch ~50µs + synchronous readback ~500µs), so it can't win small workloads; AVX-512 does one
768-dim cosine sim in ~0.1µs and scales linearly. **Decision: `GPU_THRESHOLD` raised 1,000 → 50,000**
(realistic upper bound for a personal memory store); below that, SIMD is strictly superior. The old
"GPU wins at 512–768 dims / 10× throughput" predictions were **wrong** — do not restate them as fact.
Bench: `cargo bench --bench gpu_vs_simd -p nanna-gpu` (use `--profile dev`; release LTO takes minutes).

### Memory / FSRS lifecycle
Five stages: **Extraction** (LLM extracts facts, importance 1–5, source STATED vs OBSERVED) →
**Storage** (embeddings + FSRS params + workspace scope + tags) → **Recall** (semantic search that
also records an FSRS "review" — the testing effect *strengthens* recalled memories) →
**Consolidation/"dreaming"** (periodic clustering merges duplicates) → **Decay** (retrievability
falls over time). FSRS state bands: Active / Dormant / Silent / Unavailable. Dedup cutoff >0.9 similarity
(update FSRS schedule rather than create). Importance is static and feeds initial difficulty (distinct
from decaying retrievability). Recall gating: only recall when message is non-trivial (>5 words OR contains `?` OR >80 chars);
skip injecting a memory already present in the last 4 messages. Recall scope: workspace sessions see
global + that workspace's memories; global sessions see all.

### Context management
Token budget (200k Claude): 10k system reserved + 8k response reserved + ~132k conversation.
Per-tool truncation ratios: command output 20% head / 80% tail (recent output matters); web 80/20
(intro holds content); code 40% head / 40% tail with an omitted-lines marker. Summarization: 10k-char
threshold, hierarchical for large content, ~25% compression per level, truncation fallback if models
fail. Budget allocation: proportional by size → +20% recency boost → 2,000-char min floor → redistribute excess.
Per-message hard truncation 50KB; `max_block_chars` floored to 100 (Anthropic rejects empty blocks; messages sanitized before every call).

### Agent loop & routing
Iteration cap hardcoded 10 (suggest configurable, ceiling 50); preferred-model retry 3× with 15/30/45s
backoff, fallback models 1 attempt each; suggested wall-clock timeout 5 min. `AppState` RwLock read
lock is held for the *entire* loop, so config changes/model switches block for minutes and parallel
sessions serialize (fix: clone-out then release). Model fallback resets to the top of `model_priority`
every call (incl. heartbeats) — deliberate, stateless. Ollama auto-detected by a `:tag` in the model id.

### Multi-agent / swarm
Kimi-K2.5-inspired. `SwarmConfig` defaults: max_parallel 5, timeout_per_task 120s, max_retries 1.
Sub-agent (`AgentSpawner`) limits: 5-min timeout, 25 iterations, fresh context (system + workspace only).
ThinkingMode budgets: Instant 0 / Low 1024 / Medium 4096 / High 16384 / Maximum 32768.
Supervisors are Erlang/OTP-style (backoff initial 1s / max 60s / ×2; health interval 30s / timeout 10s /
healthy 2 / unhealthy 3). CDC: Gear rolling hash, ~2KB–32KB boundaries, DEDUP_THRESHOLD 0.7. SummaryCache LRU 100, in-memory.

### Production defaults
Rate-limit token buckets: Telegram 30@1/s, Discord 5@5/s, Slack 1@1/s, default 10@2/s. Backoff:
base×2^attempts, max_retries 3, base delay 1s, cap 60s, jitter 0–500ms. Message priority Critical >
High > Normal > Low > Bulk (BinaryHeap). Shell exec default timeout 30s. Scheduler defaults: heartbeat
30 min, consolidation hourly, suggested job timeout 10 min, history retention 30 days.

### Daemon / GUI
Reconnect: current code fixed 10s interval, max 30 attempts (~5 min) before embedded fallback (design
intent was 1→2→4→8→16→30s backoff). Config defaults: connect_timeout 5s, request_timeout 300s (sized
for large summarization), sidecar port 9833, health-ping timeout 15s. Pending request is inserted into
the map *before* send (avoids a response-before-registration race). Health monitor restarts on a single
failed check (aggressive — a grace period of 3 is proposed).

### LLM providers
Context windows: Anthropic 200k, OpenAI 128k–200k, OpenRouter varies, Ollama 32k default. Anthropic
credential order: OAuth token → API key → env var. OAuth uses PKCE via `claude setup-token`; can import
from an existing Claude Code CLI install. "OAuth stealth mode" remaps tool names to Claude Code canonical
names (`write_file`→`Write`, etc.).

### Local model runner (Burn) — patterns proven in `physics515/laurelane` (P12 reference)
laurelane runs small open models locally on **Burn 0.21** (`burn`, `burn-wgpu`, `burn-ndarray`,
`burn-store`, `burn-fusion`, `autotune`), validated on an RTX 4070 Ti SUPER 16GB. Reusable patterns:
- **One binary, two backends, runtime pick.** Compile both `Wgpu` (Vulkan/DX12/Metal — *no CUDA
  toolchain*) and `NdArray<f32>` CPU. All model code is generic over `B: Backend`. A cheap
  `wgpu::Instance::enumerate_adapters(PRIMARY)` probe (via `pollster::block_on`, cached in `OnceCell`)
  chooses GPU-if-present else CPU. `fusion` makes `Wgpu` → `Fusion<Wgpu>` transparently.
- **f16 path** is opt-in: `type Gpu = Wgpu<half::f16, i32>` roughly halves VRAM (~15GB→7–8GB).
- **From-scratch decoders in Burn** (not Candle): Qwen2.5 (RmsNorm + GQA + rotate-half RoPE + SwiGLU,
  tied lm_head, config-driven from HF `config.json`) and LFM2.5 (hybrid: GQA-attention blocks + gated
  short-conv "LIV" blocks via depthwise causal `Conv1d`, per-head q/k RMSNorm). Plus an all-MiniLM-L6-v2
  sentence-embedder (6-layer BERT, masked-mean pool + L2 norm) on ndarray/CPU for embeddings.
- **Weights**: HF **safetensors** via `burn-store` `SafetensorsStore::from_file(...).with_from_adapter(
  PyTorchToBurnAdapter.chain(CastFloatAdapter{target})).allow_partial(true).with_key_remapping(...)`.
  HF weights are bf16 → a custom `CastFloatAdapter` (`impl ModuleAdapter`) casts to backend float.
  `PyTorchToBurnAdapter` auto-transposes Linear + renames norm gamma/beta but **not RmsNorm** (remap by
  hand). Load is checked: fail on `report.missing`/`errors`, warn on `unused` (signals non-tied lm_head).
- **Decode**: HF `tokenizers` crate; ChatML template built explicitly (`<|im_start|>…`); per-layer KV
  cache `Option<(Tensor<B,4>,Tensor<B,4>)>` (+ conv-state cache for LFM2); **on-device `logits.argmax`**
  so only the winning index syncs to CPU; sync Burn wrapped in `spawn_blocking`; interrupt check between
  tokens. Model+tokenizer cached for process life in `OnceCell<Mutex<Loaded>>` (Burn `Param` is Send not Sync).
- **Weight provisioning**: stream from `huggingface.co/{repo}/resolve/main/{file}` to a `.part` then
  rename, into a per-user cache dir; check a bundled resources dir first. (hf-hub was dropped — bad URLs.)
- **Trust via parity**: every Burn port was gated byte-identical vs a Candle/Ollama reference
  (single-forward top-5 logits + a short greedy sequence). Do the same for `nanna-infer`.

### DSP integration for time-series / dreaming (P13 reference)
From `physics515/DSP` (Rust, nightly, pins `turso` 0.6 + `wgpu` 27). Real workspace crates:
`splimes` (spline interpolation), `database` (Turso control-plane + `.dspseg` columnar store +
`compression` + pattern/event pipeline), `dsp-physical-type` (`.dspseg` format + codecs), `dsp-arrow*`,
`dsp-server`, `dsp-tui`, `dsp-bench`. (`dsp-connector` does **not** exist yet.)
- **Compression is lossy-analytical, not a codec.** `database::compression::algorithm::simplify_with_aggressiveness(&[Measurement], aggressiveness 0..1, base_resolution, original_resolution, Spline)`:
  interpolate to a coarser resolution (`splimes::auto_interpolate`) then `simplify_by_slope_change` keeps
  only points where slope **sign** changes (Douglas-Peucker-like, extrema-preserving). Time-based tiers:
  a recent `pure_duration` stays full-fidelity; older data compressed harder (Linear/Exponential
  aggressiveness scaling, capped ~0.95). **This is the "dreaming over a timeline" primitive.**
- **Both `simplify_with_aggressiveness` and `simplify_by_slope_change` are self-contained pure
  functions** over `Vec<Point>` (`Point{timestamp, value: BigDecimal}`) — liftable without adopting DSP's
  storage. `splimes::auto_interpolate` auto-selects GPU (wgpu)/SIMD/CPU by size with graceful fallback
  (call `prewarm_gpu()` once). This is the smallest-surface, Turso-only path.
- **Storage tension:** DSP's `SegmentStore`/`Database` deliberately keep measurements in `.dspseg` files
  *outside* libSQL (control plane holds only metadata). To stay **Turso-only**, use the pure algorithms +
  store reduced points/coeffs as Turso `f32` BLOBs; don't adopt `SegmentStore`. Event-timeline types
  (`Event{Manifestation{start,end}}`, `detect_peaks/valleys/threshold/drawdown`) exist if we later depend on `database`.
- **FSRS as the sampling rate.** `FsrsState::weight()` (= retrievability × importance) and
  `power_law_retrievability()` are the natural keep-mask: high weight → keep detail, low → decimate.

### P9 dependency pins (when P9 starts)
`ed25519-dalek` 2.1, `arti-client`/`arti-hyper`/`tor-rtcompat`/`tor-hsservice` 0.26, `aes-gcm` 0.10,
`argon2` 0.5, `zeroize` 1, `sha2` 0.10, `base64` 0.22, `qrcode` 0.14, `image` 0.25, `axum` 0.8, `hyper` 1,
`thiserror` 2. Tor latency ~500ms–2s/request; embedded arti bootstrap ~15–60s. Replay defense: reject
requests older than 5 min. One identity per device (not shared) — compromise of one doesn't compromise all.

### Historical (superseded designs — do not resurrect as fact)
- **Stop button:** early design used a global `CancellationToken` + `cancel_message` Tauri command;
  the shipped design cancels **per-session** via the daemon `Cancel { session_id }` protocol (better for concurrency). `cancel_message` does not exist.
- **GPU CUDA plan:** an early doc described CUDA `BatchProcessor`/`MemoryPool`/`auto_tune_batch_size`
  + `GpuContext::process_vector_store` builder API. Shipped code uses **wgpu** and exports
  `BatchedSearch`/`GpuMemoryStats`/`GpuVectorStore` — treat the CUDA API as design intent only.
- **Tool system:** an early doc listed Rust built-in tools registered at startup; all tools were later
  migrated to filesystem JS/TS skills — the `builtin/*.rs` paths are historical.
