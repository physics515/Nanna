# Nanna — Roadmap

> The single master roadmap **and status source of truth** for Nanna — there is no separate
> `STATUS.md`, `planning/`, or `docs/`. The daily dev routine reads this file, picks the **single
> next unimplemented item**, builds it with tests, ticks the box, and appends a dated note.
> Shipped capability is *described* in [`README.md`](README.md); here it is only tracked.
> Edit surgically; never rewrite wholesale.

**Last updated:** 2026-07-06 (doc consolidation) · code snapshot through 2026-03-17
**Repo:** local Cargo workspace, branch `master` — one Rust workspace + a Tauri 2 / Nuxt 4 GUI.
**Stack:** Rust 2024 (rustc 1.85+) · Tokio · wgpu 24 · Tauri 2 · Nuxt 4 / Vue 3 / Tailwind 4 · SQLite (Turso) · Boa + Deno scripting.

---

## North Star

**Nanna is an always-on, local-first personal AI *presence* — not a chatbot.** A headless Rust
daemon that runs continuously, remembers across time with a cognitive (FSRS-6) memory, and is
reachable from any channel — GUI, CLI, Telegram, Discord, Slack, Signal, WhatsApp — where the GUI
is *just the richest channel*, never a privileged controller. It is extensible with JS/TS tools and
MCP servers, routes across LLM providers for cost/quality, and its long arc is a **personal device
mesh**: your phone's camera is a tool your desktop can call, your desktop's GPU a resource your
phone can use, connected peer-to-peer over Tor with no network configuration.

The bar: a calm, competent assistant that is *there* when you look up — persistent, multi-channel,
autonomous, and yours.

Every run should move one phase toward that end state — depth over breadth.

---

## Core Model

Bottom-up crate dependency tiers (17 workspace crates + the Tauri app):

```
Tier 0  nanna-simd        SIMD vector ops (AVX-512/AVX2/NEON) — the default fast path
        nanna-gpu         GPU compute (wgpu/Vulkan/DX12/Metal) — only above ~50k vectors
          |
Tier 1  nanna-memory      Vector store, FSRS-6 cognitive memory, consolidation ("dreaming")
        nanna-storage     SQLite/Turso persistence
        nanna-llm         LLM clients: Anthropic, OpenAI, OpenRouter, Ollama (+OAuth stealth)
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
        nanna-config      TOML config, Claude OAuth credentials
          |
Tier 5  nanna-core        Orchestration, scheduler/cron, workspace registry
          |
        gui/src-tauri     Tauri 2 backend + Nuxt 4 frontend (embeds core OR attaches to daemon)
```

**Governing architecture — "channels as control-plane clients":** the daemon owns *all* state
(sessions, memory, config, tools, scheduler/cron, workspace registry, keyring, channel manager).
Every channel — GUI included — reaches that control plane through the WebSocket IPC protocol.
Channel *capabilities* (markdown/tables/embeds/buttons/modals/streaming) determine **how** a
response renders, never **what** a channel can access. Multiple clients (phone + desktop) can attach
to the same daemon and see consistent state.

**Ports:** health HTTP `5148` (`/health`, `/healthz`, `/readyz`, `/status`) · WebSocket IPC `5149` · daemon sidecar (GUI-spawned) `9833`.

---

## Current State (what's real today)

Phases **1–5** and **7** are complete; **10** is mostly complete; **6** and **8** are partial;
**9** is greenfield. Concretely, today Nanna:

- Runs as a **headless daemon** (Windows service / systemd / launchd) with WebSocket IPC, PID
  lockfile, health endpoints, and session persistence to SQLite; the **GUI attaches** as a client
  with auto-reconnect and falls back to an **embedded** in-process backend when no daemon is running.
- Holds real **chat** with streaming, tool calling, interleaved thinking, and tiered context
  compression; routes across **Anthropic / OpenAI / OpenRouter / Ollama** with complexity-based model
  cascade and native prompt caching (50–80% input savings).
- Has a **cognitive memory** system (FSRS-6 spaced repetition, semantic recall with testing-effect
  reinforcement, consolidation/"dreaming", duplicate detection) persisted to SQLite.
- Ships **all tools as filesystem JS/TS skills** (39 default skills) executed by the Boa engine, plus
  **MCP client** integration and an **embedded/tiered OCR** pipeline (pure-Rust `ocrs` → vision-model fallback).
- Connects **five channels** (Telegram, Discord, Slack, Signal, WhatsApp) with a webhook server and a
  unified router that delivers agent responses back to the originating channel.
- Presents a **Tauri 2 + Nuxt 4** desktop GUI: streaming chat, Tiptap+Monaco rich editor, session
  management, tabbed settings with full config migration, memory browser, channel onboarding wizards,
  tool-stats/model-stats dashboards, system tray, and native notifications.

**Not yet verified / closed:** the daemon + embedded-fallback + reconnection path has **no
end-to-end test**; **MCP server mode** is claimed complete but `nanna-server/src/mcp.rs` does not
exist (unverified — see P3); channel webhook **signature verification is a placeholder** (Discord/Slack);
several daemon control actions return `not_implemented`; and there is real **security/correctness
debt** (user-tool path traversal, workspace file traversal, non-atomic memory writes) tracked below.

---

## Phases

### P1 — Core Infrastructure ✅
SIMD vector ops (AVX/AVX2), GPU compute (wgpu), SQLite/Turso persistence, vector store + conversation
memory, LLM clients (Anthropic/OpenAI/OpenRouter/Ollama) with streaming + tool calling, agent loop
with context management, scheduler (heartbeats, cron). **Shipped.**

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
  circuit breaker + dead-letter queue + queue persistence; adaptive/per-channel rate limits; persist inter-agent messages to SQLite.
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
- **Storage:** DB migrations system; WAL mode; backup/restore; evaluate Turso-only (drop raw SQLite).
- **SIMD/GPU:** verify AVX-512 + add ARM NEON (Apple Silicon/mobile, critical for mobile); benchmark
  vs `simsimd`; GPU optimizations to lower the SIMD→GPU crossover from ~50k toward ~5k vectors
  (persistent GPU buffers, batched multi-query, async transfer/compute overlap, raw-Vulkan hot path);
  dynamic/hardware-aware GPU threshold + multi-vendor testing (NVIDIA/AMD/Intel Arc); `[gpu]` config section.
- **Observability/testing:** cross-agent distributed tracing; agent health metrics; integration tests
  for multi-agent scenarios; chaos testing; message-passing benchmarks.

---

## Immediate next actions (top of queue)

1. **End-to-end daemon test** (P8) — the whole daemon/embedded/reconnect story is unverified; write it first so everything else has a safety net.
2. **Fix the two path-traversal holes** (P11 security) — user-tool names + workspace file writes.
3. **Verify MCP server mode** (P3) — confirm it ships or mark it unbuilt; the roadmap currently overstates it.
4. **Add webhook signature verification** (P11) — Discord Ed25519 + Slack HMAC.
5. **Atomic memory persistence** (P11) — `tempfile` + `fs::rename` to stop crash-corruption.
6. **Model-ID → provider inference** (P11) — stop silently misrouting `gpt-*` to Anthropic.
7. **Per-channel sessions** (P8) — isolate context per chat/DM.
8. **Close the 3 deferred clippy warnings** (P6) — get to a clean `cargo clippy --all-targets`.

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
