# Nanna — Roadmap

> The single master roadmap **and status source of truth** for Nanna — there is no separate
> `STATUS.md`, `planning/`, or `docs/`. The **daily dev routine** (`.claude/skills/daily-dev`, run under
> `/loop`) reads this file, picks the **single next unimplemented item**, builds it **Tiger-Style**
> with tests + benchmarks, ticks the box, and appends a dated note. The engineering doctrine, benchmark
> methodology, dependency policy, and system reference notes live in that skill — this file stays a
> clean checklist. Shipped capability is *described* in [`README.md`](README.md); here it is only
> tracked. Edit surgically; never rewrite wholesale.

**Last updated:** 2026-07-07 (webhook signature verification + deps/Cargo.lock + de-flaked credential test + true memory merge + Turso rename) · code snapshot through 2026-03-17
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
does not exist (unverified — see P3); several daemon control actions return `not_implemented`; and
there is remaining **security/correctness debt** tracked below. *(Fixed 2026-07: Discord/Slack webhook
signature verification is now real Ed25519/HMAC, not a placeholder; user-tool + workspace path traversal
closed; the Update-band ingest now truly merges instead of accreting near-duplicates.)*

---

## Performance & Benchmarking

Performance is a **gate**, not a phase (small single-GPU budget): a change ships only when a benchmark
holds or improves the budget, and README perf claims link to an artifact. Governing metric: **task
success @ budget** — the fraction of the agent-eval suite the local model solves within the reference
GPU's VRAM ceiling and a p95 latency target (reference: RTX 4070 Ti SUPER 16 GB). *Methodology, the six
benchmark suites, and per-tier budgets live in the `daily-dev` skill.* Build-out:

- [ ] `nanna-bench` crate (criterion) — unify the existing `nanna-gpu` benches
- [ ] Define the **agent-eval suite** (the task-success denominator)
- [ ] Per-tier budgets in `bench/BASELINE.md` (VRAM ceilings, min decode tok/s, max TTFT, max dream-cycle time)
- [ ] CI gate — fail a PR that regresses a budget past threshold
- [ ] Inference **parity** harness (logit/sequence vs reference); memory **retention** harness (recall before/after a dream cycle)
- [ ] Perf dashboard — live TTFT / tok-s / VRAM / cache-hit in the GUI

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
- [x] Supervisor recovery tracking recovers on first success instead of counting consecutive successes (`supervisor.rs:577`).
      *(2026-07-07) Added `AgentStats.consecutive_health_passes` (incremented on pass, reset on fail); an
      `Unhealthy` agent now recovers only after `success_threshold` consecutive passes. Extracted pure
      `should_recover_to_running` / `should_mark_unhealthy` helpers (push the decision down, leaf-pure) with
      unit tests covering threshold boundaries and state-gating.*

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
(`remote:phone:camera_snap`). **Tor communication is built on [`onyums`](https://github.com/basic-automation/onyums)**
(arti-backed axum-over-Tor, MIT — same ecosystem as the `arti-axum` repo): it bundles the Tor client,
serves an axum `Router` as a **v3 hidden service**, derives a stable `.onion` from the identity key,
and ships TLS, QR address output, abuse defense, and client authorization out of the box — so we do
**not** hand-roll arti / `tor-hsservice`. New crates:
- [ ] **`nanna-identity`** — Ed25519 keypair custody + fingerprint (`XXXX-XXXX-XXXX-XXXX`),
      encrypted-at-rest `~/.nanna/identity.json` (Argon2 KDF + AES-256-GCM, zeroized). The stable `.onion`
      is derived from this key by onyums (`tor_hscrypto`).
- [ ] **`nanna-tor`** (thin, onyums-backed) — expose the daemon's axum surface as a Tor v3 hidden
      service via `OnionService::builder().router(app).nickname(..).serve()`; report bootstrap/reachability
      from onyums `status_events()`; TLS `Upgrade`/`Strict`; outbound `.onion` requests via onyums'
      re-exported `arti_client`. Feature-flagged (arti adds ~10–20MB). Far smaller than hand-rolling arti.
- [ ] **`nanna-mesh`** — QR / `nanna://pair` discovery (peers in `~/.nanna/peers.toml`) via onyums'
      `OnionAddress::qr_terminal()` / `qr_svg()`; signed JSON tool_request/response protocol; default-deny
      trust model (`ToolPolicy`, require_approval, per-peer rate limit) that leans on onyums' built-in
      **abuse defense** (proof-of-work / rate-limit / WAF "Skin") and **v3 client authorization**
      (restricted discovery) for the transport-level allowlist; audit log; relay wiring remote tools into the local registry.
- [ ] **GUI** — peer management page, identity management (view/rotate/export), Tor status widget
      (onyums `status()` / `status_events()`), QR pairing.
- [ ] **Claude Code / external-agent bridge** — HTTP/SSE transport on the MCP server + peer-tool registration + auth.
- [ ] Key rotation announcement, identity backup (BIP-39?), Tor-state caching, mobile (arti on Android) investigation.

### P10 — Token Efficiency & Cost Optimization ✅ (mostly)
Done: Anthropic + OpenAI native prompt caching + hit tracking, cross-provider model routing with
complexity classifier + tool-call-only routing + first-message override, aggressive tool-output
summarization, progressive distillation (rolling summary every N turns), tool-result eviction,
CDC message-level dedup, per-model stats tracker + persistence + stats-informed routing. Open:
- [ ] **LLMLingua-style prompt compression** (needs local GPU model, e.g. Phi-3/Qwen2 via Ollama; perplexity token scoring, selective).
- [ ] **Structured tool output schemas** — audit tool verbosity, optional `output_schema` on `ToolDefinition`, JSON output mode.
- [~] **Better token estimation** — replace `len()/4` with tiktoken-rs (OpenAI) or family-aware
      multipliers (3.5 code / 4 English / 2 CJK); account for per-message framing (~100 tok) and
      truncation-marker text. Current heuristic causes ~20–30% overflow/underutilization.
      *(2026-07-07) First pass in `nanna-llm`: `estimate_tokens` is now character-class aware
      (ASCII ~4 chars/token via `div_ceil`; wide/CJK ~1 token/char — fixes the byte-`len()/4`
      CJK under-count that was the main overflow source), and `estimate_request_tokens` adds
      `MESSAGE_FRAMING_TOKENS` (4) per message. Tests cover ASCII/CJK/mixed + framing. Still TODO:
      a real tiktoken path and code-vs-English (3.5) differentiation.*
- [ ] Streaming cache tracking (`loop_runner.rs:834`) — parse usage from `message_start` for accurate cache stats.

### P11 — Correctness, Security & Architecture Debt 🚧 (new — cross-cutting)
Concrete, actionable items with `file:line` anchors. **This is the near-term backlog the daily
routine should drain first.**

**Security (do first):**
- [x] **User-tool path traversal** — `UserToolManager::save_tool` joins `{name}.json` unsanitized; a
      name like `../../etc/cron.d/evil` escapes the tools dir. Enforce `^[a-z][a-z0-9_]{0,63}$` in
      `create_tool` + `CreateToolTool` (same validation skills already use).
      *(2026-07-06) `validate_tool_name` added at the `create_tool` chokepoint in both the daemon
      (`user_tools.rs`) and GUI (`tool_authoring.rs`) copies — covers `CreateToolTool` (agent path) too.
      Unit tests reject `../`, separators, non-lowercase-leading, and >64-char names.*
- [x] **Workspace file traversal** — `save_workspace_file` joins the `file` param unvalidated
      (`../../etc/passwd` escapes). Canonicalize and assert containment before writing.
      *(2026-07-06) `validate_context_filename` guards `Workspace::save_context_file` (the chokepoint the
      unguarded GUI-embedded path used; the daemon path already allowlisted). Accepts only a single
      normal component (no `/`, `\`, `.`/`..`, root/drive), bounded 128 bytes; postcondition
      `debug_assert!`s the path stays inside `.nanna`. Tests cover traversal + legit writes.*
- [x] **Discord webhook signature** (`webhook.rs:306`) trusts any non-empty header — add Ed25519 (`ed25519-dalek`).
      *(2026-07-07) `verify_discord_signature` now decodes the hex pubkey/signature and verifies `timestamp‖body`
      with `VerifyingKey::verify_strict` (constant-time, non-malleable). Any decode/length failure rejects.
      Tests cover valid, tampered-body, wrong-timestamp, and malformed-input cases.*
- [x] **Slack webhook signature** (`webhook.rs:438`) is a placeholder — add HMAC (`ring`/`hmac`).
      *(2026-07-07) `verify_slack_signature` recomputes `HMAC-SHA256("v0:{ts}:{body}")` and compares with
      `Mac::verify_slice` (constant-time); keeps the ±5-min replay guard, requires the `v0=` prefix. Tests
      cover valid, wrong-secret, tampered-body, stale-timestamp (replay), and empty-input cases. Deps
      `ed25519-dalek`/`hmac`/`sha2`/`hex` added to `nanna-daemon` matching `nanna-server`'s pinned reqs.*
- [ ] Harden `delete_skill`'s `remove_dir_all` (symlink check / soft-delete); stronger user-script sandboxing.
- [ ] Harden memory extraction against prompt injection (raw conversation is embedded in the extraction prompt).

**Correctness bugs:**
- [ ] `parse_model_id("gpt-4o")` returns `("anthropic","gpt-4o")` and fails silently — infer provider from name prefix (`gpt-*`→openai, `claude-*`→anthropic, `llama*`/`:tag`→ollama).
- [ ] **Atomic memory persistence** — `save_memories` writes in place; a crash mid-write corrupts the store. Use `tempfile` → write → `fs::rename`.
- [x] **Memory merge** (`memory/service.rs:207`) — `Update` creates a new memory instead of merging.
      *(2026-07-07) `smart_ingest`'s `Update` band (0.75–0.92 sim) now folds the incoming content into the
      existing memory (pure `merge_memory_content`: superset-dedup, else bounded append ≤4096 B) and
      reinforces FSRS, instead of creating a near-duplicate. New `VectorStore::update_content_and_embedding`
      re-embeds + upserts the whole entry (content and embedding stay consistent). See also P13 true-merge.*
- [ ] **Tool-memory workspace scope** — `MemoryServiceAdapter::store()` always creates global memories; the `remember` tool ignores workspace scope. Thread workspace context through.
- [ ] **Context budget for small models** — `truncate_context` uses hardcoded `MAX_CONVERSATION_TOKENS` (132k) while `calculate_dynamic_tool_budget` is model-aware, so a 32k Ollama model gets wrong math. Thread model limits everywhere.
- [ ] Orphaned-message on failure — embedded mode stores the user message before the loop; a mid-loop failure leaves no assistant reply. Store a partial error message instead.
- [ ] `not_implemented` daemon control actions: Regenerate message (`control.rs:416`), Tool enable/disable (`control.rs:1155`), Channel status (`control.rs:1558`, needs ChannelManager), ~~Uptime~~ **(done 2026-07-07)**, non-destructive `peek_mailbox` (`control.rs:578`).
      *(2026-07-07 Uptime) `ControlPlane` now holds a monotonic `started_at: Instant` (set in all three
      constructors); `uptime_secs()` reports real elapsed seconds in the status payload (was hardcoded 0).
      Unit-tested (fresh uptime ~0s, monotonic).*
- [ ] Windows service `install/uninstall/start/stop` return errors (`service.rs:136`) though runtime works via `windows_service.rs`.
- [ ] Server stats not wired to shared daemon state (`server.rs:882`).
- [ ] MCP server notifications logged but not handled (`transport.rs:148`).
- [ ] JS tools don't parse parameter schemas from manifests (`scripting/tool.rs:188`).
- [ ] Tool-manager consistency: `update_tool` mutates memory before save (diverges on write failure → clone/mutate/save/swap); `create_user_tool` swallows registration errors in `if let Ok`; no duplicate-name check; `enabled:false` tools still execute; no `ToolRegistry::unregister` (deleted tools stay callable until restart); non-string enums dropped in `parse_params_from_schema`.
- [ ] Leaked `embedded_run_states` entries on failed/panicked runs (only removed on success).
- [ ] `create_llm_client_for_model` builds a fresh HTTP client every call — cache `LlmClient` by model ID, invalidate on credential change.
- [x] **Env-flaky test** `credentials::tests::test_secure_store_file_fallback` (`nanna-config`) — `set` succeeds but `get` fails under a headless OS keyring, so `cargo test` is red in unattended runs. Make the file-fallback path deterministic for tests (temp store dir / feature flag) so it doesn't depend on an interactive keyring. *(discovered 2026-07-06)*
      *(2026-07-07) Added `SecureStore::file_only_at(dir)` — a keyring-bypassing, file-store-only mode
      rooted at an explicit dir. `get`/`set`/`delete` short-circuit to the file helpers; the three file
      helpers became `&self` methods honoring `file_dir`. Tests rewritten deterministically (`file_only_at`
      + `TempDir`): round-trip, overwrite, delete/not-found, and cross-dir isolation. Also usable for
      headless/service deployments where the OS keyring is inaccessible.*
- [ ] **Latent test/compile drift** — as of 2026-07-06 the full-workspace `cargo test` didn't even compile: `nanna-workspace`/`nanna-daemon` used `tempfile` without a dev-dep; `nanna-channels::queue` test lacked a `ChannelId` import; `nanna-memory` `VectorStoreConfig`/`MemoryEntry` test initializers were stale (`AtomicUsize`, `expires_at`); `src/main.rs` `run_daemon()` omitted the new `DaemonConfig.channels` field (a **production** build break). All repaired this run. Add a lightweight `cargo test --no-run` smoke check so test-code drift can't rot silently.

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
      - [ ] *(research 2026-07-07)* Burn 0.21 ships **`burn-dispatch`** (runtime backend selection via `DispatchDevice::Wgpu(WgpuDevice::DiscreteGpu(0))`, static-enum dispatch, no perf regression) and **`burn-flex`** (a lightweight *eager* CPU backend — no fusion/autotune — that replaces `burn-ndarray` for WASM/embedded/small-model inference). Evaluate `burn-dispatch` for the "one binary, dual backend, runtime probe" item (may replace the hand-rolled `wgpu::Instance::enumerate_adapters` probe) and `burn-flex` vs `ndarray` for the CPU-fallback tier and the local MiniLM embedder. Also: up to 8× lower framework overhead — meaningful for the small-model decode budget. Sources: [Burn 0.21.0 release](https://burn.dev/blog/release-0.21.0/), [cross-platform GPU backend](https://burn.dev/blog/cross-platform-gpu-backend/).
- [ ] **One binary, dual backend, runtime probe** — compile BOTH `Wgpu` (Vulkan/DX12/Metal, no CUDA toolchain) and `NdArray` CPU; a cheap `wgpu::Instance::enumerate_adapters` probe (cached in `OnceCell`) picks GPU if present, else CPU. No feature-split builds. (laurelane `use_gpu()` pattern.)
- [ ] **First model: a Hermes-class function-calling small model** — a from-scratch Burn decoder (start from laurelane's Qwen2.5 / LFM2 modules: RmsNorm + GQA + RoPE + SwiGLU, tied lm_head) sized for one GPU (1.5–3B). Prove tool-calling quality is good enough to run the loop.
      - [ ] *(research 2026-07-06)* Evaluate **Qwen 3.5-9B** as the default single-GPU function-calling model — 2026 consensus "sweet spot" (fits ~8GB VRAM, strong tool-call reliability, GGUF Q4 doesn't degrade tool calls). Sources: [insiderllm](https://insiderllm.com/guides/function-calling-local-llms/), [unsloth tool-calling guide](https://unsloth.ai/docs/basics/tool-calling-guide-for-local-llms).
      - [ ] *(research 2026-07-07)* Per-tier default: **8GB → Qwen 3.5-9B**, **16GB → Qwen 3.6-35B-A3B with `--cpu-moe`** (MoE expert offload — ties to the VRAM-budgeting item), **24GB → Qwen 3.6-27B dense or 35B-A3B**. Local ~7–9B models **lose coherence after 2–3 tool-chain steps** → bias toward short loops + sub-agent decomposition for the local tier (revisit the iteration cap / swarm hand-off for local models). Sources: [sitepoint 2026](https://www.sitepoint.com/best-local-llm-models-2026/), [insiderllm function-calling](https://insiderllm.com/guides/function-calling-local-llms/).
      - [ ] *(research 2026-07-07)* Tool-budget evidence **validates the two-tier tool discovery design**: each tool definition costs ~50–150 tokens; keep the always-sent set **under 5–10 tools** for 7–9B models (Nanna's core-tools-vs-`discover_tools` split already does this). Add a benchmark asserting the local model's active-tool count stays within this budget, and prefer `discover_tools` activation over sending the full registry on the local path.
      - [ ] *(research 2026-07-06)* Investigate **MoE + expert CPU-offload** (`--cpu-moe`-style) so a larger agentic model (e.g. Qwen 3.6-A3B) fits a 16GB card — relevant to the single-GPU VRAM budgeting item. Also note the model-specific tool-call parser pattern (Qwen ships `qwen3_coder`) for reliable parsing into `ContentBlock::ToolUse`.
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
- [x] Rename `SqliteMemoryPersistence` → `TursoMemoryPersistence` (`nanna-daemon/src/memory_persistence.rs`; refs in `server.rs`); align with the already-correct `TursoMemoryStorage`.
      *(2026-07-07) Struct renamed (all 5 refs, both files); module doc + the "sqlite datetime format"
      comment de-SQLite'd (no SQL/`.db`/`datetime('now')` changed). Builds green.*
- [ ] Purge the word "SQLite" from code comments, log/`warn!` strings, and doc-comments (storage lib.rs/Cargo.toml; daemon persistence/session/control/server; memory service/lib; GUI `sqlite_*` var names) → "Turso"/"the database". **Do not** change SQL, `.db` files, or `datetime('now')`/`AUTOINCREMENT`/`json_*`.
- [x] Delete stale `crates/nanna-daemon/src/server.rs.bak`. Pin `turso` precisely (0.x is pre-1.0). Add a CI guard that fails if `rusqlite`/`libsql`/`sqlx` ever enters the dep tree. (Note: a transitive `libsqlite3-sys` comes from RustPython in `nanna-scripting`, separate concern.)
      *(2026-07-06) `server.rs.bak` already absent. `turso` pinned `=0.4.4` in `nanna-storage`. The
      CI guard is a `cargo test` (`nanna-storage/tests/dep_guard.rs`) that scans `Cargo.lock` and fails
      if `rusqlite`/`libsql`/`sqlx` appear (no external CI needed). Also pinned `aegis = "=0.9.7"`
      (transitive via `turso_core`) — 0.9.8+ mandates a clang-cl C build; 0.9.7 keeps the pure-Rust path,
      matching the "prefer pure-Rust, no-C where avoidable" doctrine and keeping stock-MSVC builds green.*

**Best-in-class dreaming:**
- [ ] **Unify the two stacks** — the running app calls low-level `MemoryService::consolidate()` while the richer `DreamingService`/`nanna-core::DreamingRuntime` (feedback, gates, promote/demote) is dead code. Make `DreamingService` the single orchestrator via `create_dreaming_executor`; delete the GUI branch (`lib.rs:8462`) + daemon `MemoryAction::Consolidate` duplication.
- [ ] **Idle-gated, multi-phase dream cycle** (like sleep, not a fixed hourly cron): track last-activity; after N min idle (or memory-pressure) run phases — (a) purge-expired + testing-effect flush, (b) **true merge/dedup**, (c) cluster-consolidate by FSRS weight band, (d) expand high-weight, (e) DSP timeline compression (below). Emit progress events.
- [x] **Implement the missing true merge** — `IngestAction::Update` currently falls back to create/reinforce (`service.rs:300`); add content-level merge so dreaming deduplicates instead of accreting near-duplicates.
      *(2026-07-07) Done for **all three ingest paths** (`smart_ingest`, `remember_with_importance`,
      the scoped variant) via a shared `fold_into_memory` helper: `merge_memory_content` +
      `update_content_and_embedding` fold related-but-distinct content into the existing memory
      (bounded, superset-dedup) and reinforce FSRS. Next: apply the same merge in the batch
      dreaming/consolidation clusterer (`cluster_memories`), which still creates consolidated copies.*
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
  Markdown; embedding-dimension migration + re-embed on provider change; ~~extraction filtering~~ /
  ~~dedup-before-storage~~ **(2026-07-07: `filter_extracted_memories` drops empty/whitespace + exact
  dupes within an extraction batch, order-preserving; deliberately NO length threshold so short facts
  survive — cross-batch dedup stays with `smart_ingest` similarity bands)**; background consolidation with
  progress events; memory categories/tags.
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

1. **Turso-only cleanup** (P13) — fast, pure hygiene that sets the direction: ~~rename `SqliteMemoryPersistence`~~ **(done 2026-07-07)**, ~~delete `server.rs.bak`~~ (gone), ~~add the CI dep-guard~~ **(done 2026-07-06)**. Remaining: purge remaining "SQLite" strings from comments/logs/var names across storage/daemon/memory/GUI (do NOT touch SQL, `.db`, `datetime('now')`).
2. **Bring all deps to latest + commit `Cargo.lock`** (doctrine → *Dependency freshness*) — `Cargo.lock`
   un-gitignored and committed (2026-07-07); compatible deps already at latest (`cargo update` = 0 changes).
   Low-risk majors applied green: `directories 5→6` (unified with the workspace pin), `tower-http 0.6→0.7`
   (daemon+server), `socket2 0.5→0.6` (daemon). **Deferred majors** (each needs a real migration — build
   green + tests + benches before landing; do one per run):
   - [ ] `reqwest 0.12→0.13` (workspace-wide HTTP client — wide ripple)
   - [ ] `tokio-tungstenite 0.26→0.29` (IPC/WebSocket across client/daemon/gui/mcp/channels)
   - [ ] `deno_core 0.375→0.406` + `deno_ast 0.51→0.53` + `swc_core 7→72` (nanna-scripting — large)
   - [ ] `rustpython-{vm,stdlib,pylib} 0.4→0.5` (nanna-scripting)
   - [ ] `playwright-rs 0.8→0.14` + `chromiumoxide 0.8→0.9` (nanna-browser)
   - [ ] `keyring 3→4` (nanna-config)
   - [ ] `ed25519-dalek 2→3`, `hmac 0.12→0.13`, `sha2 0.10→0.11` (nanna-server + nanna-daemon — keep pairs aligned)
   - [ ] `scraper 0.22→0.27`, `lopdf 0.34→0.43` (nanna-tools)
   - [ ] `rand 0.8/0.9→0.10` (channels, gui), `toml 0.8→1.1` (gui), `windows-service 0.7→0.8`, `nix 0.29→0.31` (unix), `criterion 0.5→0.8` (nanna-gpu benches)
   - [ ] GUI `pnpm update --latest` sweep in `gui/`
3. **`nanna-infer` Burn skeleton** (P12) — one binary, dual `wgpu`+`ndarray` backend, runtime GPU probe, load one small model, greedy decode: prove local inference end-to-end on the dev GPU.
4. **Local embeddings in Burn** (P12) — MiniLM-class CPU embedder wired into the memory `embed_fn` → fully-local memory (no API embeddings).
5. **`Provider::Local` in the router** (P12) — dispatch completion/stream/tool-calls to `nanna-infer` and make local the top-priority (zero-cost) tier; cloud becomes opt-in escalation.
6. **Unify + upgrade dreaming** (P13) — one `DreamingService` orchestrator, idle-gated multi-phase cycle, true merge, local `summarize_fn`.
7. **`nanna-timeline` + compression-as-dreaming** (P13) — append-only event log in Turso + lift DSP's `simplify_with_aggressiveness`/`splimes` as the timeline compressor keyed by FSRS retrievability.
8. ~~**Fix the two path-traversal holes** (P11 security) — user-tool names + workspace file writes.~~ **(done 2026-07-06)**
9. **End-to-end daemon test** (P8) — the daemon/embedded/reconnect story is still unverified.

