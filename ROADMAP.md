# Nanna — Roadmap

> The single master roadmap **and status source of truth** for Nanna — there is no separate
> `STATUS.md`, `planning/`, or `docs/`. The **daily dev routine** (`.claude/skills/daily-dev`, run under
> `/loop`) reads this file, picks the **single next unimplemented item**, builds it **Tiger-Style**
> with tests + benchmarks, ticks the box, and appends a dated note. The engineering doctrine, benchmark
> methodology, dependency policy, and system reference notes live in that skill — this file stays a
> clean checklist. Shipped capability is *described* in [`README.md`](README.md); here it is only
> tracked. Edit surgically; never rewrite wholesale.

**Last updated:** 2026-07-15 (response healing + stop-context retention; prior: 2026-07-14 deps refresh)
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

### P0 - Public Preview Release
- [ ] Decide: ship as Developer Preview 0.1.0 (compile-from-source, power users only) OR commit to non-technical public beta (requires Phase 1 completion).
- [ ] If Developer Preview: rewrite README top section to explicitly state "experimental alpha, requires Rust + Node, requires cloud API key or Ollama, native local inference not yet included, not recommended for sensitive data."
- [ ] Create RELEASE_NOTES.md or MILESTONE that freezes scope.
- [ ] Set up GitHub Actions to build Tauri + daemon sidecar and attach artifacts to Releases.
- [ ] Publish signed Windows .msi/.exe installer with bundled daemon sidecar.
- [ ] Publish signed and notarized macOS .dmg (Universal or separate Intel/Apple Silicon).
- [ ] Publish Linux AppImage and/or .deb/.rpm.
- [ ] App launches without terminal; daemon starts automatically.
- [ ] Add Start Menu / tray / launch-at-login support.
- [ ] WebView2 handling on Windows.
- [ ] Document uninstall process.
- [ ] Add "check for updates" or auto-update mechanism

#### P0.1 - First Run UX
- [ ] Create public facing website / Github Pages
- [ ] Build GUI onboarding wizard (replaces CLI-centric onboarding).
- [ ] Plain-language intro screen explaining what Nanna is.
- [ ] Data storage location selection.
- [ ] Backend chooser: Anthropic / OpenAI / OpenRouter / Ollama — with clear "native local model coming soon" if not implemented.
- [ ] API key entry with validation; fix has_api_key to check all provider keys, not only Anthropic.
- [ ] Ollama detection (is server running? is a model pulled?).
- [ ] Memory/privacy explanation with opt-in toggle for auto-remembering.
- [ ] Tool permission setup: ask before enabling shell/browser/file-write.
- [ ] Daemon/embedded backend auto-start.
- [ ] Health check screen with helpful, non-technical error messages (API key invalid, Ollama not running, port conflict, etc.).
- [ ] Emergency stop / pause-memory button visible in main UI.

#### P0.2 - Documentation
- [ ] Rewrite README top half for users: pitch, Download buttons, system requirements, 5 screenshots, "first 5 minutes" checklist, uninstall.
- [ ] Move architecture/performance content to the bottom of the readme
- [ ] Add truthful capability matrix: Desktop GUI / CLI chat / Fully local inference / Ollama backend / Cloud providers / Channels — each with Status and Requires columns.
- [ ] Add PRIVACY.md documenting: what's stored locally, what's sent to LLM providers, OpenAI embeddings, Brave Search, channels, websites; how to disable cloud calls; how to delete/export data.
- [ ] Add screenshots of: chat, settings, memory browser, channel setup, daemon/tray state, model/backend selection.
- [ ] Add troubleshooting guide: API key invalid, Ollama not running, daemon not responding, port already in use, macOS app blocked, Windows Defender warning, Linux WebKitGTK missing, GPU not detected.
- [ ] Add per-OS installation docs.
- [ ] Commit LICENSE file (MIT) — appears absent despite README reference.
- [ ] Add CONTRIBUTING.md and CODE_OF_CONDUCT.md.
- [ ] Fix Cargo.toml repository URL from clawdbot/nanna to physics515/Nanna.
- [ ] Add GitHub repo description and topics.
- [ ] Unify port documentation (README says 5149; CLI defaults to 9999) — pick one, update both code and docs.

#### P0.3 - Stronger Public Release (can follow 0.1)
- [ ] Local Ollama setup assistant in GUI.
- [ ] Model/backend status dashboard.
- [ ] Cost tracking for cloud models.
- [ ] Backup/export/delete data UI.
- [ ] Per-channel session isolation (critical if any channel is marketed).
- [ ] Channel-native response formatting.
- [ ] Log rotation + crash diagnostics export.
- [ ] Windows service install/uninstall/start/stop actually working.
- [ ] Code signing / notarization in CI.
- [ ] Accessibility pass (screen reader, keyboard navigation, ARIA, color contrast).
- [ ] Internationalization/localization framework (currently English-only).
- [ ] Burn local runner (P12) → re-market true offline.
- [ ] Dreaming overhaul (P13)
- [ ] Self-update via GitHub Releases.
- [ ] Resource cleanup verification on uninstall (daemon, config, memory DB, credentials fully removed).

#### P0.3 - Code Quality & CI
- [ ] Add GitHub Actions workflow: cargo fmt --check, cargo clippy --all-targets --all-features -- -D warnings, cargo test --workspace --all-features, cargo test --no-run smoke check.
- [ ]  Add cargo audit and cargo deny to CI.
- [ ]  Add frontend CI: pnpm install --frozen-lockfile, pnpm exec vue-tsc, pnpm audit, Tauri build smoke test.
- [ ]  Add Tauri packaging CI producing signed artifacts per OS.
- [ ]  Add end-to-end daemon test: start → connect → conversation → persistence → fallback → reconnect.
- [ ]  Add gitleaks/trufflehog secret-scan step to CI.
- [ ]  Add coverage tracking (codecov/coveralls) if practical.
- [ ]  Add ESLint/Prettier/Vitest/Playwright configs for frontend.
- [ ]  Add Dependabot/Renovate config.
- [ ]  Resolve deferred clippy warnings (too_many_lines, etc.) — enforce -D warnings in CI.
- [ ]  Begin decomposing giant files: loop_runner.rs (~132KB), nanna-llm/src/lib.rs (~159KB), gui/src-tauri/src/lib.rs (8k+ lines) — not all required for 0.1 but plan the split.

### P1 — Core Infrastructure
SIMD vector ops (AVX/AVX2), GPU compute (wgpu), Turso persistence (embedded, SQLite-compatible),
vector store + conversation memory, LLM clients (Anthropic/OpenAI/OpenRouter/Ollama) with streaming +
tool calling, agent loop with context management, scheduler (heartbeats, cron).
- [ ] Onboarding writes API keys to plaintext config.toml (src/onboarding.rs), even though a SecureStore using OS keyring exists. The OS keychain should be the default path; TOML config should store only non-secret settings.
- [ ] SecureStore file fallback is plaintext JSON (mode 0600), not encrypted — the module comment misleadingly says "encrypted file storage." Fix the comment or implement real AES-GCM encryption with an OS-protected key.
- [ ] Inconsistent application directory namespaces — config uses ProjectDirs::from("bot", "clawd", "Nanna") while credentials use ProjectDirs::from("com", "nanna", "nanna"), causing orphaned data and confused uninstall flows.
- [ ] Onboarding has_api_key only checks config.llm.api_key or ANTHROPIC_API_KEY, ignoring OpenAI/OpenRouter keys. quick_setup specifically asks for an Anthropic key despite multi-provider support — broken first-run for non-Anthropic users.
- [ ] Tauri CSP is set to null in gui/src-tauri/tauri.conf.json — not acceptable for a desktop app rendering model output and markdown.
- [ ] Tauri Devtools enabled by default in production features (gui/src-tauri/Cargo.toml) — should be removed from default features.
- [ ] Tauri shell permissions (allow-open/spawn/kill/execute) for the daemon sidecar need least-privilege review.
- [ ] ROADMAP explicitly lists open items: disabled tools still execute, deleted tools remain callable until restart, delete_skill needs hardening against remove_dir_all/symlink races, stronger sandboxing needed.
- [ ] HTTP server defaults to 0.0.0.0:3000 (src/main.rs) — potential footgun if exposed without auth.
- [ ] Port inconsistencies: README says daemon IPC is 5149, but src/main.rs daemon start defaults to 9999, and daemon status checks 5149. Must be unified and documented.
- [ ] Current usage can transmit user data to: cloud LLM providers, OpenAI embeddings (if OPENAI_API_KEY set), Brave Search, channel platforms (Telegram/Discord/Slack/Signal/WhatsApp), and websites fetched by tools/browser. A PRIVACY.md documenting data flows, opt-out options, and data deletion procedures is mandatory.
- [ ] Auto-remembering user messages and assistant replies into long-term memory should be opt-in with clear onboarding language and a pause/delete memory UI.
- [ ] No SECURITY.md or vulnerability disclosure process.
- [ ] No Dependabot / cargo-audit / npm audit automation.
- [ ] No GitHub secret scanning enabled.
- [ ] Store all secrets in OS keychain by default; remove secret fields from config.toml.
- [ ] Encrypt the SecureStore file fallback with AES-GCM (OS-protected key) or remove fallback; correct the misleading "encrypted" comment.
      - [ ] *(research 2026-07-09)* `keyring 4` (now on the workspace) split into a `keyring-core` layer
            exposing a pluggable `CredentialStore`/`CredentialBuilder` trait registrable via
            `keyring::set_default_store(..)`. That's the clean seam for this item: implement an
            encrypted-file `CredentialStore` (AES-GCM) and register it as the default when no OS keyring is
            present, instead of the ad-hoc plaintext-JSON fallback in `credentials.rs`. Source: [keyring-core docs](https://docs.rs/keyring-core).
- [ ] Set a restrictive Tauri CSP (not null).
- [ ] Disable devtools in production default features in gui/src-tauri/Cargo.toml.
- [ ] Per-tool toggles visible in GUI; audit log for every tool call.
- [ ] Fix tool lifecycle bugs: disabled tools must not execute; deleted tools must not remain callable until restart (ROADMAP P6/P11).
- [ ] Harden delete_skill against remove_dir_all/symlink races.
- [ ] Bind local services (health/webhook) to localhost by default; require explicit opt-in for public exposure.
- [ ] Add authentication for any non-local control plane.
- [ ] Verify webhook signature validation across all channels (Telegram secret, WhatsApp verification, Signal bridge trust, replay protection).
- [ ] Unify ProjectDirs namespaces — config and credentials must use the same ("com", "nanna", "nanna") (or equivalent) namespace.
- [ ] Run gitleaks detect --source . and trufflehog git file://. across full git history.
- [ ] Remove or gitignore .claude/settings.local.json (committed with machine paths and broad agent permissions).
- [ ] Add SECURITY.md with vulnerability disclosure process.
- [ ] Enable GitHub secret scanning and Dependabot.
- [ ] Claude UI Testing automations
- [ ] Implement Mummu model runner to replace the built in

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
      *(2026-07-06) Extracted a pure `apply_health_result` state machine (Unhealthy→Running requires `success_threshold` **consecutive** successes; Running→Unhealthy requires `failure_threshold` consecutive failures; thresholds floored at 1; a failure resets the recovery streak). Added the `consecutive_health_successes` stat. Bonus: events now emit after the agents write-lock is released (was held across `.await`). 6 unit tests.*

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
- [~] **Cost tracking** — `CostTracker` (pricing table per model, `UsageRecord` per call), aggregate by
      session/day/month/model/tool, surface in GUI.
      *(2026-07-12)* Core shipped in `nanna-agent::cost`: `ModelPricing` (input/output/cache-read/cache-write
      USD-per-1M) + a reference list-price table (Jan-2026 public prices for Claude/GPT/o-series families,
      matched by family **prefix** so dated ids like `claude-opus-4-8` resolve) + a pure `estimate_cost_usd(..)`
      (per-class arithmetic, `debug_assert` non-negative rates, ≥0 result). Local/Ollama/unknown models return
      `None` → reported `priced:false`, never a silent $0. Wired to the token counts the daemon now records
      (see the model-stats fix this run): `ModelStatsTracker::cost_report() -> Vec<ModelCost>` (snapshots under
      the read lock then prices lock-free, priciest-first) and surfaced on the live `SystemAction::ModelStats`
      IPC response as a new `costs` array (additive, non-breaking). 5 unit tests (exact per-million arithmetic,
      zero-cost, prefix resolution incl. most-specific-wins, local/unknown unpriced, tracker integration
      pricing a Sonnet run at $18 + flagging a local model). Remaining: per-session/day/month aggregation +
      per-tool cost + GUI surfacing (needs a GUI build); pricing table should become config-overridable.
      *(2026-07-12, research-corrected)* Table updated to **2026 actual list prices**: Opus 4.x is **$5/$25**
      per Mtok (was wrongly seeded with the legacy Opus-3 $15/$75), Haiku 4.5 is **$1/$5**; cache-read = 0.1x
      input, cache-write = 1.25x input (5-min TTL). Sonnet unchanged at $3/$15. Source:
      [Claude pricing docs](https://platform.claude.com/docs/en/about-claude/pricing).
      - [ ] Add **Fable 5** (`claude-fable-5`) to the pricing table once its per-Mtok rate is published.
      - [ ] Config-overridable pricing (`[pricing]` TOML or a fetched table) so rates don't rot in-code; add a
            batch-mode (0.5x) + 1-hour-cache (2.0x) multiplier the tracker can apply.
      *(2026-07-12)* Completeness: `ModelStatsSummary` now carries `total_cache_creation_tokens` (`record()`
      already accumulated it but `summary()` dropped it, hiding cache-write volume and understating cost);
      populated in `summary()` + a regression test. Backward-compatible (additive field; serde consumers ignore
      unknown/extra fields). Added `ModelStatsTracker::total_cost_usd()` (grand-total known cloud spend; sums
      only priced models) surfaced as `total_cost_usd` on the `SystemAction::ModelStats` response; test.
- [ ] **Runtime config reload** — watch `config.toml` with `notify` (debounce 500ms), validate before
      apply, apply without restart, emit `config-change` events.
- [ ] **Per-channel config** — `[channels.<name>.agent]` sections (system_prompt/model/max_tokens/tools allowlist).
- [ ] **Tool allowlists/blocklists** — `ToolPolicy` (global allow/block + per-channel + per-user for multi-user channels).
- [x] **Log rotation** — `tracing-appender` daily rotation, max ~7 files (logs currently accumulate unbounded).
      *(2026-07-09)* New `nanna-daemon::log_file` builds a `RollingFileAppender` (DAILY rotation,
      `filename_prefix="nanna-daemon"`, `.log` suffix, `max_log_files(7)`) wrapped in `tracing_appender::non_blocking`;
      added as an `Option<fmt::Layer>` beside the console + in-memory-buffer layers. New `--log-dir`
      (default `{data_dir}/logs`) and `--no-file-log` flags; the worker guard is a `main`-scoped local so it
      flushes on normal return (a `static` guard would never drop). Pure `resolve_log_dir` + `build_appender`
      with 4 unit tests; verified by a real `nanna-daemon run` boot writing a prefixed file. Note:
      `tracing-appender` 0.2.5 supports only time-based rotation (no per-file size cap) — if size-bounding is
      wanted later, use a custom writer or the `clia/tracing-appender` fork.
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
- [~] **End-to-end daemon testing** (High) — start daemon, connect client, run a conversation, verify
      persistence + embedded fallback + reconnection (currently untested).
      *(2026-07-16)* **First real E2E suite shipped** — `crates/nanna-client/tests/e2e_daemon.rs`, 4 tests
      driving a real `DaemonServer` over the real WebSocket IPC with a real `Client` (no mocks). Lives in
      `nanna-client` because it already depends on `nanna-daemon`, so the dependency edge stays one-way.
      Hermetic by construction: built via `DaemonBuilder` with explicit settings instead of
      `from_nanna_config`, on an OS-assigned free port + a `TempDir`, with `with_memory(false)` — so a run
      never reads the developer's `config.toml`/`.db` and needs no API key or reachable model. Covers:
      daemon boots → client attaches → protocol answers; a created session is visible; **state survives a
      client disconnect + fresh reattach** (the GUI reconnect path); and **sessions survive a full daemon
      restart** on the same data dir (durable control plane, not a cache). Stable across 3 consecutive runs.
      **Found and fixed a real bug:** `Client::disconnect()` only signalled the handler task and returned, so
      the state flipped to `Disconnected` *asynchronously* — `is_connected()` could still report `Connected`
      right after `disconnect()` returned, and a `request()` in that window passed the connected check before
      failing confusingly. It now sets the state itself (the handler still does too; idempotent) and
      `debug_assert`s the postcondition. Remaining for this item: a real conversation turn (needs a live LLM)
      and the **embedded-fallback** path (needs a GUI build).
- [ ] **Per-channel sessions** (High) — map `channel_id:chat_id → session_id` so each chat/DM gets
      isolated context (all messages currently share one context).
- [~] **Response formatting per channel** — a `ResponseFormatter` driven by `ChannelFeatures` bitflags
      (strip markdown for Signal, tables→text for Telegram, embeds for Discord, Block Kit for Slack).
      Bitflags exist but every channel currently receives identical raw text.
      *(2026-07-09)* First slice shipped: added a `ChannelFeatures::MARKDOWN` flag + `supports_markdown()`,
      a pure `nanna-channels::format` module (`format_for_channel` / `strip_markdown`), and wired it into the
      single outbound chokepoint `MessageRouter::send`. Markdown-rendering channels (Discord/Telegram/Slack)
      carry the flag → text passes through **unchanged** (zero regression); Signal/WhatsApp now get Markdown
      down-converted to plain text (headers/blockquotes/fences/bold/inline-code stripped, `[label](url)` →
      `label (url)`), so they stop showing literal `**`/backticks. Conservative on purpose: single `*`/`_`,
      `__dunders__`, `snake_case`, and `2 * 3` survive. 7 unit tests.
      *(2026-07-10)* **Length-aware splitting shipped.** New pure `split_for_length(text, max_chars)` splits a
      payload into chunks each ≤ `max_chars` **Unicode scalars** (not bytes), preferring a newline then a
      space break within the window and only hard-splitting a single over-long token; chunks concatenate back
      to the exact input (the break char stays on the preceding chunk) so no content is lost. Wired into
      `MessageRouter::send`: when the channel sets `max_message_length` and the (already Markdown-adapted) text
      exceeds it, the router sends the parts in order and returns the first part's id (the reply/edit anchor).
      7 tests (within-limit passthrough, whitespace/newline break preference, oversized-token hard-split,
      Unicode-scalar counting; + 2 router tests with a recording mock proving split vs no-split).
      *(2026-07-12)* **tables→text shipped.** `strip_markdown` is now table-aware: a row line immediately
      followed by a delimiter row (`|---|:--:|`) starts a table block — each row drops its outer pipes, trims
      + inline-strips each cell, and re-joins with " | "; the delimiter row is dropped. Disambiguated from
      prose: a table delimiter must contain **both** a dash and a pipe, so a bare `---` horizontal rule after a
      pipe line and a stray prose `a | b` are left untouched. Postcondition relaxed to ≤2x (tight tables re-add
      a few separator chars). 5 tests (basic table, alignment colons + surrounding text, inline-markdown in
      cells, prose-pipe/HR negatives, tight-table growth guard); 45 nanna-channels tests green. Remaining:
      Discord embeds, Slack Block Kit.
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
- [ ] *(research 2026-07-16)* **onyums is alive and healthy — the P9 bet still holds.** Latest commit
      **2026-07-14**, latest published **0.3.1 (2026-06-18)**. Two concrete facts for when we wire it: (1) it
      pins **arti 0.43.0** across `arti-client`/`tor-hsservice`/`tor-hscrypto`/etc., while **arti-client 0.44.0
      shipped 2026-06-30** — onyums is **one minor behind**, so do *not* pin arti 0.44 ourselves and expect it
      to unify (take arti transitively via onyums, exactly as Appendix C says). (2) New since 0.3.0: a
      `crates/onyums-skin` workspace member — pure-Rust WAF (regex signatures), `governor` rate limiting, and an
      **optional Equi-X PoW backend behind an `equix` feature that is LGPL-3.0 and off by default** — keep it
      off unless we accept copyleft. It also now ships a vanity `.onion` miner and pure-Rust QR (`qrcode`,
      `default-features = false`, no `image`/FFI) — matching the "no C where avoidable" doctrine.
      Sources: [onyums](https://github.com/basic-automation/onyums),
      [onyums crate](https://crates.io/crates/onyums), [arti-client](https://crates.io/api/v1/crates/arti-client).

### P10 — Token Efficiency & Cost Optimization ✅ (mostly)
Done: Anthropic + OpenAI native prompt caching + hit tracking, cross-provider model routing with
complexity classifier + tool-call-only routing + first-message override, aggressive tool-output
summarization, progressive distillation (rolling summary every N turns), tool-result eviction,
CDC message-level dedup, per-model stats tracker + persistence + stats-informed routing. Open:
- [x] **LLMLingua-style prompt compression** (needs local GPU model, e.g. Phi-3/Qwen2 via Ollama; perplexity token scoring, selective).
      *(2026-07-16) Wired end-to-end against the **summarization-model settings** (not a hard-coded Ollama):
      `nanna-agent::compressor` scores sentences via the configured model, keeps top-`1/ratio` by
      information density, falls back to head/tail truncation on score-count mismatch. Pure helpers
      (`parse_scores`, `select_keep_indices`, `fallback_compress`, `split_sentences`) are unit-tested.
      Tool-output path walks `summarization_priority` with client failover (`compress_with_priority`).
      Tier-1 proactive compression now preferentially rewrites large older tool results in context
      (`AgentContext::compress_older_tool_results` / `Agent::compress_older_context_tool_results`)
      before the prior `drop_oldest` fallback. Note: sentence-level scoring (not per-token perplexity)
      — practical on chat-completion APIs where raw logprobs aren't exposed.*
- [ ] **Structured tool output schemas** — audit tool verbosity, optional `output_schema` on `ToolDefinition`, JSON output mode.
- [~] **Better token estimation** — replace `len()/4` with tiktoken-rs (OpenAI) or family-aware
      multipliers (3.5 code / 4 English / 2 CJK); account for per-message framing (~100 tok) and
      truncation-marker text. Current heuristic causes ~20–30% overflow/underutilization.
      *(2026-07-07) First pass in `nanna-llm`: `estimate_tokens` is now character-class aware
      (ASCII ~4 chars/token via `div_ceil`; wide/CJK ~1 token/char — fixes the byte-`len()/4`
      CJK under-count that was the main overflow source), and `estimate_request_tokens` adds
      `MESSAGE_FRAMING_TOKENS` (4) per message. Tests cover ASCII/CJK/mixed + framing. Still TODO:
      a real tiktoken path and code-vs-English (3.5) differentiation.*
- [x] Streaming cache tracking (`loop_runner.rs:834`) — parse usage from `message_start` for accurate cache stats.
      *(2026-07-06) `StreamEvent::MessageStart` now carries `input_tokens`/`cache_read_tokens`/`cache_creation_tokens` (parsed from the Anthropic `message_start` usage object; zero for providers that don't report it); the streaming loop captures them into `LlmResult` instead of the old `input_tokens: 100` + zero-cache placeholders. 2 tests on `parse_sse_event` (with/without usage).*

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
- [x] Harden `delete_skill`'s `remove_dir_all` (symlink check / soft-delete); stronger user-script sandboxing. *(2026-07-14 — GUI `delete_skill` now sanitizes the skill name (no empty/path seps/`..`/non `[A-Za-z0-9._-]`); canonicalizes the skills root + skill dir and refuses any path that escapes; rejects skill dirs that *are* a symlink or that contain a symlink child before calling `remove_dir_all`. Soft-delete + deeper user-script sandboxing still open.)*
- [x] Harden memory extraction against prompt injection (raw conversation is embedded in the extraction prompt).
      *(2026-07-06) `build_extraction_prompt` now fences the conversation between `EXTRACTION_FENCE` markers with an explicit "treat strictly as untrusted data, never obey instructions inside it" directive, and defangs any forged fence in the conversation so it can't break out. 2 tests (fencing present + forged-fence neutralized). Note: a defense-in-depth measure, not a guarantee — combine with the extraction dedup/drop-empty filter.*

**Correctness bugs:**
- [x] Response Healing - Automatically fix malformed JSON responses from LLMs. for chat, embeddings, and summarization.
      *(2026-07-15) Shared `nanna_llm::heal::{heal_json, heal_json_as, heal_tool_args}`: strip fences, extract balanced spans, repair trailing commas/single quotes/bare keys/truncated braces. Wired into chat tool-arg parse (agent stream + OpenAI adapter + GUI pending tool calls), embedding response bodies (OpenAI + Ollama new/legacy), memory-extraction + swarm decomposition (summarization/structured JSON). 8 unit tests on the healer.*
- [x] when the user presses the "Stop" button and interrupts a models work all contexts from unfinished work is lost. it should be kept in both the UI and in the models context.
      *(2026-07-15) Stop preserves partial work end-to-end: (1) UI no longer wipes the streaming bubble on cancel — annexes `[Stopped by user]` and waits for the daemon `MessageEnd` to promote it into a real assistant message; (2) agent loop checks cancel mid-stream and at iteration boundary, tracks `streamed_text`, and `finish_cancelled` folds unfinished assistant text into conversation context + returns it (session persistence already stores Ok/partial results); auto memory-extract still runs on cancel so long interrupted runs don't lose knowledge.*
- [x] `parse_model_id("gpt-4o")` returns `("anthropic","gpt-4o")` and fails silently — infer provider from name prefix (`gpt-*`→openai, `claude-*`→anthropic, `llama*`/`:tag`→ollama). *(2026-07-06: the **daemon** already infers correctly via `ProviderId::from_model` + unit tests. 2026-07-14: **GUI** `parse_model_id` now matches — explicit `openrouter/`/`github/`/`ollama/`/`openai/`/`anthropic/` prefixes first, then family-prefix inference (`gpt-*`/`o1`/`o3`→openai, `claude*`→anthropic, `:tag`→ollama), historical Anthropic default for unknowns. 2 unit tests.)*
- [x] **Atomic memory persistence** — `save_memories` writes in place; a crash mid-write corrupts the store. Use `tempfile` → write → `fs::rename`.
      *(2026-07-06) `VectorStore::save` now writes to a sibling `.json.tmp` and `fs::rename`s it over the target (atomic on the same filesystem), so a crash mid-write can't leave a truncated store. Test: save→load round-trips and no temp file is left behind. (This JSON path is the deprecated JSON→Turso migration writer; the live path is Turso write-through.)*
- [x] **Memory merge** (`memory/service.rs:207`) — `Update` creates a new memory instead of merging.
      *(2026-07-07) `smart_ingest`'s `Update` band (0.75–0.92 sim) now folds the incoming content into the
      existing memory (pure `merge_memory_content`: superset-dedup, else bounded append ≤4096 B) and
      reinforces FSRS, instead of creating a near-duplicate. New `VectorStore::update_content_and_embedding`
      re-embeds + upserts the whole entry (content and embedding stay consistent). Applied to all three
      ingest paths via the shared `fold_into_memory` helper. See also P13 true-merge.*
- [x] **Tool-memory workspace scope** — `MemoryServiceAdapter::store()` always creates global memories; the `remember` tool ignores workspace scope. Thread workspace context through. *(2026-07-14 — GUI adapter now holds a live `Arc<RwLock<WorkspaceRegistry>>` (constructed once, shared with AppState) and every `store`/`search` call scopes to the *current* active workspace via `remember_scoped`/`recall_scoped`. Daemon path already had this via `services_workspace_id` + per-chat update.)*
- [x] **Context budget for small models** — `truncate_context` used hardcoded `MAX_CONVERSATION_TOKENS` (132k) while `calculate_dynamic_tool_budget` is model-aware, so a 32k Ollama model got wrong math. Thread model limits everywhere.
      *(2026-07-13)* **Fixed the compression-threshold ↔ hard-limit inversion for small models** (a concrete
      slice of this item). `ModelInfo::compression_threshold` was a flat 80% of context while `hard_input_limit`
      is `max(context − max_output, 50% context)`. For a small model with a large output budget (e.g. context
      8k / output 4k) that gave threshold **6400 > hard-limit 4000** — proactive compression *never fired before
      the hard cap*, so the agent emergency-truncated every turn instead of compressing gracefully (the
      local-first failure mode). Now `compression_threshold = min(80%·context, 90%·hard_limit)`, which keeps
      the invariant `threshold ≤ hard_limit` and leaves large models unchanged (200k-context Claude stays at
      160k). Applied to **both** budget paths: `nanna-llm::ModelInfo` (the `ModelInfo`-based
      `configure_for_model`) and the name-based fallback `AgentContext::configure_for_model_name` in
      `nanna-agent` (which also lacked the 50% hard-limit floor — added). `debug_assert`s guard the invariant
      on both paths. 5 tests (small stays below cap / large unchanged / output≥context degenerate — in both
      crates); 18 nanna-llm + 53 nanna-agent tests green, **−1 clippy warning** in each crate, full workspace
      builds green.
      *(2026-07-15)* Closed the remaining GUI-embedded slice. `gui/src-tauri/src/lib.rs`: added
      `conversation_token_budget(_for)` that mirrors `ModelInfo::hard_input_limit` (context − max_output,
      floor 50% of context), then reserves system+response tokens and floors at 2k so history never empties.
      `truncate_context` takes that budget instead of the hardcoded 132k. Removed unused
      `TARGET_CONTEXT_TOKENS` / `MAX_CONVERSATION_TOKENS`.
      *(2026-07-15 follow-up)* **No per-model context table anywhere.** Provider APIs / disk
      `ModelInfoCache` are the source of truth; when neither is available, a single universal
      floor (`UNKNOWN_CONTEXT_WINDOW` = 32k / `UNKNOWN_MAX_OUTPUT_TOKENS` = 4k) applies to every
      name. Deleted the name-match tables in GUI, `nanna-llm::default_model_info`,
      `AgentContext::configure_for_model_name`, and the daemon router missing-client path.
      GUI tool-budget path awaits `LlmClient::get_model_info`. Shared
      `ModelInfo::conversation_history_budget` owns the math. Tests assert floor semantics
      and that explicit `ModelInfo` (as an API would return) still drives small/large budgets.
      *(2026-07-15)* **ModelInfo is the only source for model-dependent budgets.**
      Removed remaining hardcodes that duplicated provider metadata: deleted
      `embedding_dimension_for_model` and all name-based embedding-dimension tables
      (API/cache metadata or a live probe embedding instead); removed
      `ContextSummarizationConfig.summarizer_context_window` so each summarizer
      model fetches its own `ModelInfo`; agent-loop `max_tokens` and compressor/
      summarizer output caps clamp through `ModelInfo.max_output_tokens`; Ollama
      / OpenRouter metadata no longer invent silent min floors or static output
      caps (`context_window/2` only when the provider omits a completion limit);
      AgentContext threshold defaults use `unknown_model_info` floors. Memory
      consolidation still exposes a model-agnostic byte builder that daemon code
      feeds from the summarizer's hard input limit.
- [x] Orphaned-message on failure — embedded mode stores the user message before the loop; a mid-loop failure leaves no assistant reply. Store a partial error message instead.
      *(2026-07-15)* In `send_message` (embedded path), a failed `run_agent_loop_with_fallback` now stores a
      partial assistant message (`_(This turn was interrupted…)_` + error) and touches the session before
      returning the error, so the user turn is no longer orphaned in storage.
- [x] `not_implemented` daemon control actions: ~~Regenerate message~~ **(done 2026-07-11 — `ChatAction::Regenerate` now drops the stale assistant reply via a new pure `Session::take_last_user_turn()` (removes the last user message **and** everything after it — reply + trailing tool turns — returning that user content; `None`/unchanged when there's no user message), persists the truncated session, then replays through the existing `Send` path via `Box::pin(self.handle_chat(..))` so it reuses all context/workspace/memory/agent logic with zero duplication. 4 unit tests cover reply-drop, prior-history preservation, trailing-tool-turn drop, and no-user→None. Daemon boots green; full turn execution needs a live LLM (unavailable unattended) so verified by build+boot smoke + unit tests)**, ~~Tool enable/disable~~ **(done 2026-07-09 — `ToolAction::Enable`/`Disable` now persist the flag via `update_tool` and reconcile the live registry through a shared `reconcile_tool_registration` helper (also used by `Update`): disable→unregister, enable→re-register, effective without a restart; tokio test drives the real create→register→disable→enable path on a live `ToolRegistry`)**, ~~Channel status~~ **(done 2026-07-14 — `ControlPlane` holds an optional `Arc<StatusManager>` (attached before the Arc wrap at daemon boot); `ChannelManager::with_status_manager` shares the same manager, registers configured providers on `configure()`, and wires `.with_status_manager(..)` into Telegram/Discord/Slack listeners so circuit-breaker state transitions update live connection state; `ChannelAction::Status` returns a single channel or `{channels, summary}` (or `not_found` / `unavailable`); 2 tokio tests)**, ~~Uptime (`control.rs:1636`, needs start timestamp)~~ **(done 2026-07-06 — `ControlPlane.started_at: Instant` + `uptime_secs()` accessor; `SystemAction::Status` reports real uptime; test)**, ~~non-destructive `peek_mailbox` (`control.rs:578`)~~ **(done 2026-07-06 — `SessionManager::peek_mailbox` clones without draining; sub-session status now peeks instead of destructively draining pending inter-session messages; test)**.
- [ ] Windows service `install/uninstall/start/stop` return errors (`service.rs:136`) though runtime works via `windows_service.rs`.
- [x] Server stats not wired to shared daemon state (`server.rs:882`).
      *(2026-07-12)* Bigger bug than the label: the daemon's **main agent was built without any stats
      tracker** (`AgentService` → `Agent::new(..).with_context(..)`, no `with_stats`) **and** the sub-agent
      spawner had `stats: None`, so **no live model stats were ever recorded** — `control.model_stats` only
      ever held what `import_from_storage` loaded at boot; the model-stats dashboard never reflected fresh
      daemon activity. Fixed by threading **one** `ModelStatsTracker` (clone shares state — `Arc<RwLock<_>>`)
      through the whole daemon: `init_services` mints it, wires it into both `AgentService::with_stats(..)`
      (new builder + field; applied to the `Agent` at build time) and `AgentSpawnerImpl.stats`, and returns
      it; `run()` assigns it to `control.model_stats` **before** `with_storage` so persisted stats load into
      the same shared tracker and the router reads it for health-aware routing. 2 unit tests
      (`clone_shares_underlying_state`: records via sub-agent + main clones both visible via the control-plane
      clone; `independent_trackers_do_not_share`). Verified by an isolated real boot (`nanna-daemon run
      --port 5249 --health-port 5248 --data-dir <scratch>`): reaches "Daemon ready", "Stats-informed routing
      enabled", `/status → {"sessions":1,"memory_available":true,"agent_available":true}`, and a heartbeat
      agent turn ran through the shared tracker. Full-turn stat accumulation needs a sustained live LLM
      session (heavy unattended) so covered by build+boot smoke + unit tests.
- [x] **Double health-server bind / stale health state** — *(2026-07-11)* Re-checked against current
      master: there is only **one** `HealthServer` construction, and a clean `nanna-daemon run` on **free**
      ports binds exactly once with **zero** `os error 10048` (the 2026-07-10 "second binder fail 4×" was
      port contention from leftover daemons on the reused ports, not an in-process double bind). Two real
      residual bugs fixed instead: (1) `server.rs` logged "Health server listening" **before** the spawn,
      duplicating `health.rs:299`'s post-bind log and implying a bind that hadn't happened — dropped. (2) The
      served state was a **throwaway** `HealthState::new(..)` while the session-count loop updated a
      *different* `Arc`, so `/status` reported `sessions:0` forever. Added `HealthServer::from_shared(Arc<HealthState>,..)`
      and pass the updated handle, so `/status` now reflects live state. Verified by a real boot:
      `/status → {"sessions":1,..,"memory_available":true,"agent_available":true}`, single "listening" log,
      no bind retries. 2 tests (shared handle drives `/status`; `new` stays isolated). Minor remaining
      (cosmetic): `server.rs`'s "Daemon ready. IPC server listening" also duplicates `ipc.rs`'s own post-bind
      log — same misleading pre-bind pattern, harmless, left for an IPC-log cleanup.
- [x] MCP server notifications logged but not handled (`transport.rs:148`).
      *(2026-07-06) `handle_server_notification` now classifies server notifications (`message`/`progress`/`cancelled`/`*/list_changed`) and routes them to the right tracing level — MCP `notifications/message` logs at warn when its `level` is warning-or-worse, else debug (was parsed then dropped). Pure `classify_server_notification` + `mcp_level_is_severe` with 3 tests.*
      *(2026-07-10)* **`list_changed` now invalidates the client cache.** Added a transport-agnostic
      `ListChangedFlags` (per-list `AtomicBool` for tools/resources/prompts) surfaced via a **defaulted**
      `Transport::list_changed_flags()` — so `HttpTransport` and any other impl inherit `None` with zero
      changes. The stdio reader task marks the matching flag on `notifications/{tools,resources,prompts}/
      list_changed` (parsed by a pure `list_changed_kind`; a `list_changed` for an uncached list like `roots`
      marks nothing), and `McpClient::list_{tools,resources,prompts}` read-and-clear the flag, refreshing the
      cache before serving instead of returning stale entries. 3 tests: per-list marking + read-and-clear
      semantics, an uncached `roots/list_changed` marks nothing, and an end-to-end client test (counting mock
      transport) proving a dirty flag forces exactly one refresh and is then consumed. 10 nanna-mcp tests
      pass; clippy unchanged (561); nanna-daemon builds green.*
- [x] JS tools don't parse parameter schemas from manifests (`scripting/tool.rs:188`).
      *(2026-07-11)* `extract_manifest` no longer hardcodes `parameters: None` — every scripted tool was
      shipping an **empty** parameter list to the LLM (the model had to guess arg names). New pure
      `extract_parameters_schema(source)`: finds the balanced `{..}` after `parameters:` (string/comment-aware
      brace matching) and normalizes the JS object literal to strict JSON (quote bare keys, single→double
      quotes with full escape decode/re-encode, drop trailing commas + comments, UTF-8-safe), then
      `serde_json`-parses it; returns `None` on any failure so a bad block falls back to today's behavior (no
      regression, never guesses). Feeds the existing `parse_params_from_schema` in `scripted.rs`, so
      `definition()` now carries real `{type,properties,required,enum,default}`. 13 unit tests (real-manifest
      shape, trailing commas/comments, single quotes+enum, escaped quotes à la python skill, non-ASCII
      descriptions, `}`-in-string balance, absent→None) **plus a real-data integration test that parses all
      39 shipped default skills** (0 failures). Note: bare object *keys* must be ASCII identifiers (all
      parameter names are); non-ASCII belongs in quoted string values, which decode correctly.
- [ ] Tool-manager consistency: ~~`update_tool` mutates memory before save (diverges on write failure → clone/mutate/save/swap)~~ / ~~no duplicate-name check~~ **(done 2026-07-10 — daemon `UserToolManager`, see below)**; `create_user_tool` swallows registration errors in `if let Ok`; ~~`enabled:false` tools still execute~~ / ~~no `ToolRegistry::unregister` (deleted tools stay callable until restart)~~ **(done 2026-07-09 — see below)**; ~~non-string enums dropped in `parse_params_from_schema`~~ **(done 2026-07-06 — `enum_value_to_string` preserves integer/boolean/null enum values in both the daemon and nanna-tools copies; tests each)**.
      *(2026-07-10)* Daemon `UserToolManager` hardened: **`update_tool` now clone→validate→mutate→save→swap** —
      it validates the new source *before* touching any field and mutates a clone, publishing to the cache
      only after the disk write succeeds, so a bad edit or a failed write can no longer leave RAM diverged
      from disk (the old path applied `description` before validating `source`, and mutated the live entry in
      place). **`create_tool` now rejects duplicate names** under the write lock held across
      dup-check→save→insert (atomic vs a racing create), instead of silently clobbering an existing tool +
      its `.json`. 2 tests: duplicate-create rejected + original untouched; a bad-source update fails whole
      and a fresh manager reloading from disk still sees the original. clippy unchanged (2057). Remaining
      here: `create_user_tool`'s `if let Ok` swallow + the GUI-embedded `tool_authoring.rs` copy (needs a GUI build).
      *(2026-07-09)* Replaced the naive one-line `ToolRegistry::unregister` (removed only the primary key, so a deleted tool stayed reachable through its own alias entry) with a cascading version: deleting a canonical tool also drops every alias whose target is it and purges the `alias_targets` reverse-map; deleting an alias leaves the canonical intact. Returns the entry-count removed. Wired into the **daemon** control plane: `ToolAction::Delete` now `unregister`s the live tool (deletion takes effect without a restart), and `ToolAction::Update` reconciles the registry with the new `enabled` flag — unregister then re-register only if still enabled, so a disabled tool stops executing and an edit's new source goes live immediately. 4 registry unit tests (uncallable-after-delete, alias cascade, alias-only removal leaves canonical, unknown-name no-op). Remaining: the GUI-embedded `UserToolManager` copy (`tool_authoring.rs`) still needs the same wiring (needs a GUI build to verify).
- [ ] Leaked `embedded_run_states` entries on failed/panicked runs (only removed on success).
      *(2026-07-12: verified the **daemon** analog `AgentService.active_chats` is NOT leaky — the only exits
      between insert and cleanup are the success path (cleans up before returning) and the all-models-exhausted
      path (cleans up); no early `?`/`return`/`unwrap` between them. Only an external panic in `agent.run()`
      would leak, which async-`Drop` can't cleanly cover. The leak is GUI-embedded-only.)*
- [x] **`parse_retry_after` non-ASCII byte-offset bug** (`agent_service.rs`) — it `find`s the prefix in the
      **lowercased** string but sliced the **original** at that offset; a lowercase that changes byte length
      before the prefix (e.g. `İ`→`i̇`, 2→3 bytes) shifts the offset, extracting the wrong digits or slicing
      mid-char (panic). Fixed to slice the lowercased string (digits are ASCII, so equivalent). *(2026-07-12)*
      Also added the first tests for the three resilience parsers (`is_rate_limit_error`,
      `is_context_length_error`, `parse_retry_after`) + `truncate`'s char-boundary backoff — 5 tests incl. an
      `İ` regression guard (old code returned `Some(2)` instead of `Some(42)`). 39 daemon tests green.
- [ ] `create_llm_client_for_model` builds a fresh HTTP client every call — cache `LlmClient` by model ID, invalidate on credential change.
- [x] **Daemon boot hard-fails without an embedding API key — contradicts "offline-capable by default".**
      *(discovered 2026-07-16 during a real `nanna-daemon run` on an isolated port/data-dir; fixed 2026-07-16)*
      Boot got all the way through storage + migrations + `LLM router initialized with 3 providers` + tools dir,
      then died: `Error: Config error: Failed to discover embedding dimension: OPENROUTER_API_KEY is required
      for embedding discovery` — **never reaching "Daemon ready"**.
      **The diagnosis above was wrong about the cause.** It was not the 2026-07-15 "no dimension table" change
      making a credential load-bearing: the daemon **had a perfectly valid OpenRouter key in `config.toml` the
      whole time**. `get_embedding_dimension` built its *own* client and read the key from
      `std::env::var(..)` **only**, while the `EmbeddingRouter` that serves every real embed reads
      `config.llm.openrouter_api_key.or_else(env)` — so the probe reported "missing key" for a key the live
      path used successfully. It also duplicated provider construction, bypassing the router's **Ollama
      fallback**, so it couldn't degrade the way real embeds do.
      **Fix (a deletion, not an addition):** the probe now goes through the *same router the embed path uses*
      (`probe_embedding_dimension(&EmbeddingRouter)`), which drops the duplicated client construction and
      inherits both config-or-env key resolution and the Ollama failover. A probe failure is now **non-fatal**:
      boot logs an actionable warning and seeds a provisional dimension. That is safe because the seed only
      has to be positive — real vectors always come from the provider, and the **pre-existing background
      `probe_and_align_dimension`** (added earlier precisely so a cold Ollama couldn't block startup) corrects
      the store and re-embeds any mismatched entries as soon as a provider answers. The blocking probe is now
      purely an optimization: when it succeeds, nothing is ever re-embedded.
      **Verified on the real binary** (`--port 5249 --health-port 5248 --data-dir <scratch>`, no
      `OPENROUTER_API_KEY` in env, key only in config): `Primary embeddings: OpenRouter` → `Memory service
      using probed dimension 2048` → **`Daemon ready. IPC server listening`** → `Embedding dimension
      confirmed: 2048`. 4 unit tests (probe reports the provider's length; empty vector rejected; nothing
      listening → clean `Err` so boot can degrade; seed is positive) driven against a one-shot localhost
      server on an OS-assigned port. 48 daemon tests green (was 44); clippy **2081 → 2081** (no new warnings).
      **This also unblocks unattended real-binary daemon verification**, which had been forcing runs to fall
      back to unit tests. Not yet exercised end-to-end: the all-providers-unreachable degrade path (needs a
      host with no Ollama *and* no key) — covered by unit test rather than a live boot.
- [x] **Workspace `cargo test` overflows the stack (unattended-red).** *(discovered 2026-07-11; root-caused
      + fixed 2026-07-16)* **The 2026-07-11 diagnosis was wrong on both the culprit and the blast radius.**
      It is **not** deno/V8: `cargo test -p nanna-scripting --features deno` passes 20/20 clean. The
      overflowing feature is **`python` (RustPython)** — no dependent enables `deno` at all
      (`gui`→`boa`, `daemon`→`python`), so workspace unification turns on `python`, and
      `cargo test -p nanna-scripting --features python` reproduces the overflow on the *first* python test.
      **Not a test-infra annoyance — a live daemon crash.** `python.exec` is a registered daemon service
      (`server.rs:385`) reachable from any JS/TS tool, and `PythonEngine::execute` ran RustPython via
      `spawn_blocking`, i.e. on a Tokio thread with the default **2 MiB** stack. RustPython overflows that
      on `print('hello')`, and a Rust stack overflow is an uncatchable `abort()` — `spawn_blocking`'s unwind
      guard cannot contain it. So **the python tool never worked and took the whole daemon down with it**;
      the red workspace test was just the symptom that surfaced first.
      **Fix:** the interpreter now runs on its own `std::thread` with an explicitly sized stack
      (`spawn_interpreter_thread` + `PYTHON_STACK_BYTES`), joined through a `oneshot`; timeout/panic
      semantics are unchanged (a synchronous interpreter was never cancellable mid-run either).
      **The bound is derived and measured in BOTH profiles:** `stack_needed = recursion_limit x
      per_frame_native_bytes`, where RustPython's own default `recursion_limit` (**256 debug / 1000 release**,
      `rustpython_vm::VirtualMachine`) is the term that makes it finite. Bisecting the stack at which the
      suite (incl. a runaway-recursion test) stops crashing:
      | profile | `recursion_limit` | overflows at | passes at |
      |---------|-------------------|--------------|-----------|
      | debug   | 256               | 16 MiB       | 64 MiB    |
      | release | 1000              | 64 MiB       | 128 MiB   |
      So release frames are **not** cheaper (~64-128 KiB either way) — the profiles differ by the 4x limit.
      `PYTHON_STACK_BYTES` = **256 MiB**, one doubling above the worst measured requirement, and the
      `const _` assert floors it at the release number. **Debug-only measurement is a trap**: an earlier
      64 MiB passed `cargo test` and then **segfaulted the release build** (`STATUS_ACCESS_VIOLATION`) — any
      change here must be re-measured under `--release`. Cost is a lazily-committed reservation on a 64-bit
      address space, one thread per in-flight call. 3 new tests (dedicated-thread execution under a
      `current_thread` runtime; runaway recursion errors cleanly without aborting; plus the 7 pre-existing
      python tests that could never have passed before).
      Verified: `cargo test --workspace` reaches **0 stack overflows** (was 3); nanna-scripting **29+1 green
      in debug *and* release**; clippy **108 → 101** warnings in-crate, none new.
      *(2026-07-16 correction — the sizing model above is wrong; the fix and the 256 MiB value stand.)*
      Re-measured on the real engine in **release** by reading back the depth actually reached: **64 MiB →
      32,727 frames, 256 MiB → 131,004 frames**. Depth scales **linearly** with the stack (4x → 4.00x), so a
      *release* frame costs **~2 KiB**, not the 64–128 KiB inferred above. `stack_needed = recursion_limit x
      per_frame` is **not** the binding rule: the VM's own stack guard fires first (see the closed residual
      below), so `recursion_limit` never binds — which also means the "overflows at / passes at" bisection
      above was measuring the guard's *band* behavior, not a per-frame budget. The full **release** suite
      (incl. runaway recursion) also passes at **64 MiB** with no `STATUS_ACCESS_VIOLATION`, so that recorded
      segfault did not reproduce in release. **Debug is the profile that still overflows** (frames there
      exceed the guard's 32 KiB band), which is what the recorded debug numbers were really detecting.
      The honest justification for a large stack is that interpreter **setup** overflowed Tokio's 2 MiB
      (`print('hello')` aborted) — a cost never measured separately. `PYTHON_STACK_BYTES` stays **256 MiB**:
      measured-good, and a lazily-committed reservation is effectively free. It is no longer claimed to be
      derived from the recursion limit.
      - [ ] Measure interpreter **setup** stack cost separately (the only term that actually justifies the
            size), then right-size `PYTHON_STACK_BYTES` and its `const _` floor against *that* number instead
            of the disproved per-frame model. Re-measure in `--release`.
      - [x] **Residual: `sys.setrecursionlimit` could abort the process.** **(fixed 2026-07-16 — clamped.)**
            The item was **right that the DoS is real, wrong about why**, and an intermediate attempt this run
            wrongly "disproved" it on release-only evidence. The corrected picture, all measured:
            RustPython *does* bound native depth — `VirtualMachine::check_c_stack_overflow`
            (`rustpython-vm-0.5.0/src/vm/mod.rs:1520`), a CPython `_Py_MakeRecCheck` port run on every
            recursive call from `with_recursion`, comparing the live `psm::stack_pointer()` against a soft
            limit derived from the thread's **actual** stack bounds. That is why depth ∝ `stack_size`
            (measured: **64 MiB → 32,727 frames, 256 MiB → 131,004** — 4x stack, 4.00x depth, ~2 KiB/frame).
            **But that guard tests a *band*, not a floor**: it fires only while the stack pointer is within
            `2 x STACK_MARGIN_BYTES` (2048 x `usize` = 16 KiB → a **32 KiB** window) above the stack base
            (`vm/mod.rs:1503-1527`). A frame that advances the pointer further than the window steps **over**
            the check into the guard page. Release frames (~2 KiB) always land inside the band — which is why
            release looked immune and the "disproof" was believable. **Debug frames do not**: under
            `cargo test --workspace` (which unifies `python` on) the probe aborted for real —
            `thread 'nanna-python' has overflowed its stack`.
            **Fix:** `build_wrapper` clamps `sys.setrecursionlimit` to the interpreter's **own startup
            default** (256 debug / 1000 release) — read at runtime, not an invented constant, and exactly the
            depth `PYTHON_STACK_BYTES` is validated at. Lowering still works; raising is pinned. The real
            function is captured in a closure and the installer name deleted, so it is not reachable by name
            from user globals (best-effort, not an escape-proof sandbox).
            Pinned by `raising_the_recursion_limit_cannot_abort_the_process`, which **aborts in debug without
            the clamp** — i.e. the profile `cargo test --workspace` actually runs is the one that catches it.
            nanna-scripting **32+1 green in debug *and* release**.
- [x] **Env-flaky test** `credentials::tests::test_secure_store_file_fallback` (`nanna-config`) — `set` succeeds but `get` fails under a headless OS keyring, so `cargo test` is red in unattended runs. Make the file-fallback path deterministic for tests (temp store dir / feature flag) so it doesn't depend on an interactive keyring. *(discovered 2026-07-06)*
      *(2026-07-07) Added `SecureStore::file_only_at(dir)` — a keyring-bypassing, file-store-only mode
      rooted at an explicit dir. `get`/`set`/`delete` short-circuit to the file helpers; the three file
      helpers became `&self` methods honoring `file_dir`. Tests rewritten deterministically (`file_only_at`
      + `TempDir`): round-trip, overwrite, delete/not-found, and cross-dir isolation. Also usable for
      headless/service deployments where the OS keyring is inaccessible.*
- [x] **Env-race in `nanna-tools` `resolve_tools_dir` tests (unattended-red).** *(2026-07-16)* Surfaced the
      moment the RustPython overflow above stopped aborting the run: `test_resolve_tools_dir_from_env`
      `set_var`s `NANNA_TOOLS_DIR` while `test_resolve_tools_dir_from_config` `remove_var`s it, and the
      environment is process-global while `cargo test` runs tests on parallel threads — so the remove could
      land between the other test's set and its `resolve_tools_dir(None)`, which then fell through to
      `DEV_TOOLS_DIR` and asserted the source-tree `default-skills` path against a temp dir. A real race, not
      a flake to retry. Fixed with a test-local `ENV_LOCK` mutex plus an RAII `EnvGuard` that restores the
      previous value on drop (so a panicking test can't leak state, and a developer's real env survives);
      the guard also makes the `unsafe` env writes sound by construction (all writers hold the lock), and
      recovers from a poisoned lock instead of cascading an unrelated panic. Added
      `env_overrides_config_tools_dir`, pinning the documented env-beats-config precedence that was never
      tested and is only safely testable now. **`cargo test --workspace` is now green end-to-end: exit 0,
      0 overflows, 0 failed suites, 378 tests / 41 suites.**
      *(2026-07-16 correction)* That "0 overflows" held only because **no test raised the recursion limit**.
      Adding one (`raising_the_recursion_limit_cannot_abort_the_process`) re-surfaced a real
      `thread 'nanna-python' has overflowed its stack` under `cargo test --workspace` — the RustPython stack
      guard is a 32 KiB band that *debug* frames step over (see the P11 residual above). Closed by clamping
      `sys.setrecursionlimit` to the interpreter's startup default. Current tally, all suites that build
      without the GUI's staged artifacts: **`cargo test --workspace --exclude nanna-gui` → exit 0, 39 suites,
      385 passed, 0 failed, 0 overflows.** (`nanna-gui` needs the sidecar + built frontend staged first — see
      the build-env note under *Immediate next actions* #2 — so a bare `cargo test --workspace` still fails in
      its build script, not in a test.)
- [ ] **Latent test/compile drift** — as of 2026-07-06 the full-workspace `cargo test` didn't even compile: `nanna-workspace`/`nanna-daemon` used `tempfile` without a dev-dep; `nanna-channels::queue` test lacked a `ChannelId` import; `nanna-memory` `VectorStoreConfig`/`MemoryEntry` test initializers were stale (`AtomicUsize`, `expires_at`); `src/main.rs` `run_daemon()` omitted the new `DaemonConfig.channels` field (a **production** build break). All repaired this run. Add a lightweight `cargo test --no-run` smoke check so test-code drift can't rot silently.

**Architecture debt:**
- [x] **Decompose `gui/src-tauri/src/lib.rs`** (8,163-line monolith) into `commands/{chat,sessions,memory,settings,channels,workspaces,scheduler,tools,system}.rs`, `llm/{routing,truncation,summarization}.rs`, `state.rs`.
      *(2026-07-16)* By execution time the file had grown to 9,863 lines. Pure move into exactly the planned
      layout: `state.rs` (AppState + event/DTO types + MemoryServiceAdapter), `llm/{routing,truncation,
      summarization}.rs`, and the nine `commands/*.rs` modules; lib.rs is now 1,273 lines (module decls,
      `pub(crate)` glob re-exports so sibling files needed zero edits, `setup_state`, `run()` with
      fully-qualified `generate_handler!` paths, system tray). Largest module is `commands/settings.rs`
      (1,815). Also repaired the pre-existing `uncached_model_name_uses_universal_floor` test, which called
      a nonexistent `resolve_model_info_sync` and didn't compile on master. `cargo check`/`cargo test
      -p nanna-gui` green; normalized diff confirms every original line moved verbatim.
- [x] **Unify embedded vs daemon agent loop** — embedded path (~280 lines) duplicates the daemon's `AgentService`; make `nanna-agent::AgentContext` the single source of truth and have embedded delegate to `AgentService`.
      *(2026-07-16)* The duplicate had grown to ~750 lines (`run_agent_loop` + `run_agent_loop_with_fallback`
      + post-loop storage/extraction) plus the dead `EmbeddedBackend` mini-loop. Embedded mode now constructs
      the daemon's `AgentService` in-process (`gui/src-tauri` gained a `nanna-daemon` dep): `embedded.rs` is a
      thin adapter that builds an `LlmRouter` from GUI config (same provider wiring as the daemon's
      `init_services`, wired `.with_storage` + shared `ModelStatsTracker` for health-aware routing), maps
      config → `AgentServiceConfig` (unbounded iterations + wrap-up-nudge policy preserved), and bridges
      `protocol::Event` → `DaemonEvent` into the SAME broadcast bus the WebSocket client uses — so
      `Backend::start_event_forwarding` (now started once for both modes, and lag-tolerant) is the single
      event→Tauri path; emitted event-name set verified identical. `send_message`'s embedded branch keeps its
      pre-work (store user msg, auto-remember, scoped recall, workspace context) then delegates to
      `chat_with_options`; the f4e83c3 no-orphaned-turn guarantee is preserved (partial result w/ "⚠️
      incomplete" footer, or the interruption marker). `cancel_session`/`get_session_run_state` delegate to
      `AgentService::cancel`/`get_run_state`. Deleted as dead after delegation: `llm/truncation.rs` (377),
      `llm/summarization.rs` (446), rate-limit helpers in `llm/routing.rs`, `EmbeddedRunState`/
      `IterationPolicy`/`PendingToolCall`. Net **−1,613 lines**. Embedded turns now get the daemon's extras:
      per-session FIFO queueing, checkpoint crash recovery, thinking-mode reasoning capture, loop-runner
      tool-result handling (memory stubs + compression instead of GUI-side proportional truncation), and
      daemon-style memory auto-extraction (replaces the GUI's STATED/OBSERVED extractor — legacy memories
      keep their tags). Known trade: `claude-proxy` models don't work embedded (router has no such provider;
      the daemon never had one either — modes now match). Verified by `cargo check --workspace` +
      `cargo test -p nanna-gui`/`-p nanna-daemon` green; a live end-to-end GUI turn was not run unattended.
- [x] Split `control.rs` (1,523 lines) into `control/{scheduler,workspace,config,system}.rs`; reduce Backend's ~50 near-identical proxy methods with a macro.
      *(2026-07-16)* `control.rs` had grown to 2,727 lines; split as a pure move into `control/{mod,chat,
      session,memory,config,tool,scheduler,channel,system,workspace,tests}.rs` — the roadmap's four-domain
      sketch gained chat/session/memory/tool/channel domains instead of a `system.rs` catch-all. `mod.rs`
      (533 lines) keeps the ControlPlane struct, constructors, shared helpers, and dispatch; each domain is
      an `impl ControlPlane` block in its own file; public API unchanged. 44/44 daemon tests pass, identical
      to baseline. Backend side: 53 of 64 proxy methods (mode check → same-named `DaemonClient` call →
      `EMBEDDED_MODE` error) are now one `daemon_proxies!` macro table with doc passthrough — adding a proxy
      is a one-line entry; the 11 mode-specific methods stay hand-written. `backend.rs` 857 → 522 lines.
- [x] Split `settings.vue` (1,483 lines) into per-tab components.
      *(2026-07-16)* Had grown to 2,054 lines. Now a 133-line shell over six tab components
      (`components/settings/Settings{Models,Agent,Memory,Tools,Scheduler,Data}Tab.vue`) with genuinely
      cross-tab state (settings, toast, model catalog, memory stats) in a provide/inject composable
      (`composables/useSettingsPage.ts`). Tabs stay `v-show`-mounted so tab-local state persists as before,
      and per-tab `onSettingsLoaded` hooks preserve the "any save reloads everything" behavior in original
      order. Bonus fix: `index.vue` had a hard build break (string literal spanning raw newlines, from
      f4e83c3) — every `pnpm build` failed; now escaped properly and the production build is green.
- [x] Refactor over-long `main.rs` command handlers (~1099, ~1221).
      *(2026-07-16)* Pure move: `src/main.rs` (1,701 lines) → 311-line CLI shell (clap structs, logging,
      dispatch) + `src/setup.rs` (component wiring: `ensure_api_key`/`create_scheduler`/`init_components`)
      + `src/commands/{serve,cli,workspace,credentials,daemon}.rs`. Legacy `run_daemon()` now lives in
      `src/commands/serve.rs`, behavior untouched. `--help` output byte-identical; check/tests match baseline.

### P12 — Local Model Runner (Burn) 🌱 flagship (the pivot)
**Goal:** a new `nanna-infer` crate that runs small open models **natively in Rust on a single
consumer GPU** as the default, first-class inference backend — no Ollama, no cloud required. The
local model runs the whole agent loop. Blueprint proven in `physics515/laurelane` (Burn 0.21, from-scratch
Qwen2.5/LFM2/MiniLM, validated on an RTX 4070 Ti SUPER 16GB).

> **Runner extracted → [`physics515/Mummu`](https://github.com/physics515/Mummu).** The generic Burn
> runner (dual wgpu+ndarray backend, from-scratch Qwen2.5/LFM2.5/MiniLM, safetensors weight loading, KV
> cache, on-GPU argmax, streaming, f16, parity gate, model management) now lives in the shared **Mummu**
> repo, which Laurelane and Nanna both consume — **runner increments land in Mummu, not here.**
> `nanna-infer` becomes a **thin consumer**: this phase now tracks only the Nanna-side integration —
> wire Mummu as `Provider::Local` (the top-priority tier in the P10 router), stream its tokens to
> channels + Tauri, and back the memory `embed_fn` + dreaming `summarize_fn` with Mummu embeddings. The
> generic runner items below are the **Mummu contract** (built + tracked there); keep them here only as
> the integration checklist.

- [ ] **Crate `nanna-infer` on Burn** — `burn = { version = "0.21", default-features = false, features = ["std","ndarray","wgpu","fusion","autotune","store"] }`. Model code generic over `B: Backend`.
      - [ ] *(research 2026-07-07)* Burn 0.21 ships **`burn-dispatch`** (runtime backend selection via `DispatchDevice::Wgpu(WgpuDevice::DiscreteGpu(0))`, static-enum dispatch, no perf regression) and **`burn-flex`** (a lightweight *eager* CPU backend — no fusion/autotune — that replaces `burn-ndarray` for WASM/embedded/small-model inference). Evaluate `burn-dispatch` for the "one binary, dual backend, runtime probe" item (may replace the hand-rolled `wgpu::Instance::enumerate_adapters` probe) and `burn-flex` vs `ndarray` for the CPU-fallback tier and the local MiniLM embedder. Also: up to 8× lower framework overhead — meaningful for the small-model decode budget. Sources: [Burn 0.21.0 release](https://burn.dev/blog/release-0.21.0/), [cross-platform GPU backend](https://burn.dev/blog/cross-platform-gpu-backend/).
- [ ] **One binary, dual backend, runtime probe** — compile BOTH `Wgpu` (Vulkan/DX12/Metal, no CUDA toolchain) and `NdArray` CPU; a cheap `wgpu::Instance::enumerate_adapters` probe (cached in `OnceCell`) picks GPU if present, else CPU. No feature-split builds. (laurelane `use_gpu()` pattern.)
- [ ] **First model: a Hermes-class function-calling small model** — a from-scratch Burn decoder (start from laurelane's Qwen2.5 / LFM2 modules: RmsNorm + GQA + RoPE + SwiGLU, tied lm_head) sized for one GPU (1.5–3B). Prove tool-calling quality is good enough to run the loop.
      - [ ] *(research 2026-07-06)* Evaluate **Qwen 3.5-9B** as the default single-GPU function-calling model — 2026 consensus "sweet spot" (fits ~8GB VRAM, strong tool-call reliability, GGUF Q4 doesn't degrade tool calls). Sources: [insiderllm](https://insiderllm.com/guides/function-calling-local-llms/), [unsloth tool-calling guide](https://unsloth.ai/docs/basics/tool-calling-guide-for-local-llms).
      - [ ] *(research 2026-07-09)* Newer 2026 recommendation for the 8GB tier: **Qwen3-Coder-Next** — an 80B **MoE with only ~3B active params**, so it decodes fast (~40–60 tok/s on a 4090) yet runs Q4 on 8GB+ VRAM, and is now rated best-in-class for *long-horizon tool use + recovery from failed tool calls* (llama.cpp fixed its tool-call parser). Note the MoE/active-param split ties directly to the P12 **`--cpu-moe` expert-offload** and VRAM-budgeting items — the same architecture Nanna's local tier wants. This should become the reference default the Mummu runner targets and the `[infer]` model config points at. Sources: [unsloth Qwen3-Coder-Next](https://unsloth.ai/docs/models/qwen3-coder-next), [running 30B on 8GB VRAM](https://dev.to/upayanghosh/from-oom-to-262k-context-running-qwen3-coder-30b-locally-on-8gb-vram-1ej1).
      - [ ] *(research 2026-07-07)* Per-tier default: **8GB → Qwen 3.5-9B**, **16GB → Qwen 3.6-35B-A3B with `--cpu-moe`** (MoE expert offload — ties to the VRAM-budgeting item), **24GB → Qwen 3.6-27B dense or 35B-A3B**. Local ~7–9B models **lose coherence after 2–3 tool-chain steps** → bias toward short loops + sub-agent decomposition for the local tier (revisit the iteration cap / swarm hand-off for local models). Sources: [sitepoint 2026](https://www.sitepoint.com/best-local-llm-models-2026/), [insiderllm function-calling](https://insiderllm.com/guides/function-calling-local-llms/).
      - [ ] *(research 2026-07-12)* **Qwen3.5 GGUF ships universal chat-template fixes for tool-calling** (apply to *any* Qwen3.5 GGUF), and the Qwen3-Coder tool-call parser is now fixed across llama.cpp/Ollama/LMStudio/Jan — de-risks the "reliable tool-call parsing into `ContentBlock::ToolUse`" item for the local tier. When Mummu ports a Qwen3.5-class model, lift its chat template + tool-call grammar verbatim rather than hand-rolling. 8GB tier still wants Q4_K_S/Q4_0 (drop to Q3_K_M on OOM); Qwen3-Coder-Next's ~46GB Q4 footprint keeps it a 16GB+/CPU-offload target, not an 8GB one. Sources: [unsloth Qwen3.5](https://unsloth.ai/docs/models/qwen3.5), [Qwen3.6 VRAM table](https://knightli.com/en/2026/05/01/qwen3-6-local-vram-quantization-table/).
      - [ ] *(research 2026-07-13)* **VRAM footnote for the 8GB default:** the stock Ollama pull of Qwen3.5-9B
            **bundles a vision encoder that inflates VRAM** — for Nanna's pure-text local tier, pull the
            **text-only GGUF (Unsloth)**; at **Q4_K_M ≈ 6 GB** it stays entirely on-GPU across all context sizes
            through 32K (200K+ possible with minor penalty on 8 GB). Bakes into the P12 model-download UX (offer a
            text-only variant + VRAM estimate) and the VRAM-budgeting picker. Reconfirms 8GB→Qwen3.5-9B Q4_K_M as
            the reference default. Sources: [localllm.in 8GB benchmarks](https://localllm.in/blog/best-local-llms-8gb-vram-2025), [mayhemcode 2026 by-task](https://www.mayhemcode.com/2026/06/best-local-llms-for-4gb-6gb-and-8gb.html).
      - [ ] *(research 2026-07-07)* Tool-budget evidence **validates the two-tier tool discovery design**: each tool definition costs ~50–150 tokens; keep the always-sent set **under 5–10 tools** for 7–9B models (Nanna's core-tools-vs-`discover_tools` split already does this). Add a benchmark asserting the local model's active-tool count stays within this budget, and prefer `discover_tools` activation over sending the full registry on the local path.
      - [ ] *(research 2026-07-16)* **`LFM2.5-8B-A1B` (Liquid AI, 2026-05-28) is now the best primary-source-backed
            8GB pick** — 8B total / **1B active** MoE, **under 6 GB at standard quantization**, day-one llama.cpp
            support + official GGUF. BFCLv3 **64.36**, BFCLv4 **48.50**, τ²-telecom 88.07. **Caveat that lands on
            us:** it emits **Pythonic** function calls (a Python list between special tokens), *not* JSON tool
            blocks — the local tool-call parser needs a shim, unlike Qwen3.5. Compare against **Qwen3.5-9B**
            (BFCL-V4 **66.1**, τ²-bench 79.1, 262K native context) which scores higher but is dense (~6 GB Q4_K_M,
            tighter on 8 GB) and has **thinking mode on by default** (`<think>`) that must be disabled for tool
            loops. Note **Qwen3.6 has no sub-10B model** (35B-A3B / 27B only), so it is not an 8GB option.
            Sources: [LFM2.5-8B-A1B](https://www.liquid.ai/blog/lfm2-5-8b-a1b),
            [Qwen3.5-9B](https://huggingface.co/Qwen/Qwen3.5-9B), [Qwen3.6](https://github.com/QwenLM/Qwen3.6).
      - [ ] *(research 2026-07-16)* **Burn is still 0.21.0 (2026-05-07) — no 0.22**, so the 0.21 notes below remain
            current. Two corrections for the Mummu contract: **there is no KV-cache API in Burn 0.21** (searched
            release notes; must be hand-rolled), and **`burn-lm`** (Tracel's own LLM engine) is **alpha and not a
            viable dependency** — only v0.0.1 published, last commit 2026-06-08, models limited to Llama 3.x /
            TinyLlama. Quantization is **not** new in 0.21 (shipped in 0.19). What 0.21 *does* add for inference:
            `attention()` with `scale`/`attn_bias`/`softcap`/`is_causal`, flash attention with causal masking, and
            attention autotune. Adoption breakage to expect: `TensorData::shape` is now `Shape` (old
            `BinFileRecorder` records are not forward-compatible). Sources:
            [Burn 0.21.0](https://github.com/tracel-ai/burn/releases/tag/v0.21.0),
            [burn-lm](https://github.com/tracel-ai/burn-lm).
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
- [x] Purge the word "SQLite" from code comments, log/`warn!` strings, and doc-comments (storage lib.rs/Cargo.toml; daemon persistence/session/control/server; memory service/lib; GUI `sqlite_*` var names) → "Turso"/"the database". **Do not** change SQL, `.db` files, or `datetime('now')`/`AUTOINCREMENT`/`json_*`.
      *(2026-07-06) Done for the **daemon** (server/persistence/session/control/memory_persistence) and **nanna-memory** (service/lib). Left as-is: `nanna-storage/src/lib.rs:6` (a factual "Turso is a Rust-native `SQLite` implementation" — describes SQL-compat, not a mislabel). Remaining: GUI `sqlite_*` var names (need a GUI build to verify).*
      *(2026-07-16) **Closed the GUI slice.** Post-decomposition the remaining references had all landed in one
      file, `gui/src-tauri/src/commands/sessions.rs` (12 occurrences): the two local bindings
      `sqlite_result`/`sqlite_sessions` → `local_result`/`local_sessions`, nine comments → "the local store" /
      "the local Turso store" / "the database", and one **user-visible log string**
      (`"Cleared {} local sessions from SQLite"` → `"… from the database"`). Naming-only: no SQL, `.db` path,
      `datetime('now')`, or control flow touched — the diff is comments + two identifier renames.
      Repo-wide the only surviving "SQLite" is the intentional factual line at `nanna-storage/src/lib.rs:6`,
      exactly as this item specifies. Verified with `cargo check -p nanna-gui` + `cargo test -p nanna-gui`
      (4 pass) — the GUI build needs the sidecar + built frontend staged first (see the build-env note under
      Immediate next actions #2).
- [x] Delete stale `crates/nanna-daemon/src/server.rs.bak`. Pin `turso` precisely (0.x is pre-1.0). Add a CI guard that fails if `rusqlite`/`libsql`/`sqlx` ever enters the dep tree. (Note: a transitive `libsqlite3-sys` comes from RustPython in `nanna-scripting`, separate concern.)
      *(2026-07-06) `server.rs.bak` already absent. `turso` pinned `=0.4.4` in `nanna-storage`. The
      CI guard is a `cargo test` (`nanna-storage/tests/dep_guard.rs`) that scans `Cargo.lock` and fails
      if `rusqlite`/`libsql`/`sqlx` appear (no external CI needed). Also pinned `aegis = "=0.9.7"`
      (transitive via `turso_core`) — 0.9.8+ mandates a clang-cl C build; 0.9.7 keeps the pure-Rust path,
      matching the "prefer pure-Rust, no-C where avoidable" doctrine and keeping stock-MSVC builds green.*

**Best-in-class dreaming:**
- [ ] **Unify the two stacks** — the running app calls low-level `MemoryService::consolidate()` while the richer `DreamingService`/`nanna-core::DreamingRuntime` (feedback, gates, promote/demote) is dead code. Make `DreamingService` the single orchestrator via `create_dreaming_executor`; delete the GUI branch (`lib.rs:8462`) + daemon `MemoryAction::Consolidate` duplication.
> **Dreaming model (do not drift from this):** memories **never expire**. A dream cycle = **semantically
> rank "like" memories → concatenate them → summarize the concatenation into a single memory**
> (`composite_cluster_score` → `MemoryCluster::concatenated_content()` → `create_consolidated_entry`).
> There is no expiry/TTL/purge step. FSRS *retrievability decay* (a memory becoming less retrievable over
> time) is not deletion. See [[nanna-dreaming-model]].

- [~] **Idle-gated, multi-phase dream cycle** (like sleep, not a fixed hourly cron): track last-activity; after N min idle (or memory-pressure) run phases — (a) testing-effect flush, (b) **true merge/dedup**, (c) cluster-consolidate by FSRS weight band (rank-similar → concatenate → summarize), (d) expand high-weight, (e) DSP timeline compression (below). Emit progress events.
      *(2026-07-09)* **Idle gate shipped** (the trigger half). `DreamingService` now tracks `last_activity` (`record_activity()` / `idle_duration()`) and exposes `dream_if_idle()` — the gated entry point the scheduler should call instead of the unconditional `dream()`. Decision lives in a pure, exhaustively-tested `dream_trigger(idle, memory_count, cfg) -> {Idle | MemoryPressure | Skipped}`: runs after `idle_threshold_secs` (default 300s) idle **or** when live memory count hits `memory_pressure_count` (default 5000, `0` disables) — memory-pressure overrides activity so a busy system still consolidates before the store grows unbounded. 4 tests (idle boundary, pressure-overrides-activity, pressure-disabled-by-zero, and `dream_if_idle` skips + never calls `summarize_fn` when active).
      *(2026-07-10)* **Ephemeral memories removed (reverts the mistaken "purge-expired dream phase").** An
      earlier same-day increment wrongly framed purging expired memories as dream "phase (a)"; per the model
      above, memories never expire. Removed the entire TTL/expiry machinery: the `expires_at` column
      (`MIGRATION_009` + index), the `expires_at` fields on `MemoryEntry`/storage `Memory`/`NewMemory`,
      `MemoryEntry::is_expired`, `VectorStore::purge_expired` + `MemoryService::purge_expired`, the search-time
      `is_expired` filter, the `is_expired` skip-reinforce checks, and the `tool_result` 2h-TTL derivation in
      `agent_service.rs` (tool-result memories are now permanent like every other category). The dream cycle
      no longer references expiry. Storage row-decode re-indexed; full non-GUI workspace builds green; memory
      28 / storage / tools 75 / daemon tests pass; clippy dropped in every edited crate (removed code). Note:
      an existing dev DB keeps a harmless unused `expires_at` column (migrations run once by name; fresh DBs
      are clean). Remaining: the rest of the multi-phase body (merge/cluster-by-band/expand/DSP) and wiring
      `record_activity`/`dream_if_idle` into the daemon scheduler + agent loop.
      *(2026-07-13)* **Phase (c) prompt bounded (Tiger-Style safety for the local summarizer).** The greedy
      `cluster_memories` put an **unbounded** number of memories into one cluster, and
      `build_consolidation_prompt` concatenated all of them into a single prompt handed to `summarize_fn` — a
      degenerate weight band of thousands of mutually-similar memories → a >250k-token prompt that overflows a
      small local model's context window (P12). Bounded at cluster *formation* (not prompt building, which
      would silently drop the omitted members' content since `consolidate_cluster` removes every cluster
      member): two `ConsolidationConfig` fields — `max_cluster_memories` (64, a coarse safety cap) and
      `max_cluster_content_bytes` — cap each cluster; a candidate that would breach either bound stays
      unassigned and re-clusters on a later seed, so **no content is dropped** — the band just consolidates
      over more passes. Both carry `#[serde(default)]`; pre/postcondition `debug_assert`s prove every cluster
      honors both bounds.
      *(2026-07-13, model-aware update)* The byte budget is now **sized to the summarizer model's real context
      window**, not a fixed "8 GB tier" constant. New pure `cluster_content_bytes_for_context(tokens)` reserves
      instruction/framing + output headroom, then converts the remaining token budget to bytes at the token
      estimator's **worst-case density** — `nanna_llm::estimate_tokens` counts any non-ASCII char as 1 token and
      the smallest non-ASCII UTF-8 char is 2 bytes, so **2 bytes/token provably cannot overflow the token
      budget for any script**. `ConsolidationConfig::with_summarizer_context_window(tokens)` applies it;
      `default()` uses the same formula at an 8k fallback (`FALLBACK_SUMMARIZER_CONTEXT_WINDOW_TOKENS`) for
      when the model is unknown. New `nanna_llm::model_context_window(name)` resolves the window from the
      existing fallback table (no async fetch); both daemon paths — the scheduled task (`server.rs`) and the
      IPC `MemoryAction::Consolidate` (`control.rs`) — size the budget to their summarizer model, so a big-context
      model consolidates more per pass while a small one stays safe. **12 tests total** (count/byte bound +
      lossless; budget scales with window & fits it at worst-case density; tiny-window floor; builder sizing;
      default==fallback formula; `model_context_window` resolution; daemon threads the window). 40 nanna-memory
      + 19 nanna-llm + 42 nanna-daemon lib tests green, zero net new clippy warnings, full workspace builds
      green, real daemon boot reaches "Daemon ready". Remaining: the GUI-embedded consolidation still uses the
      `default()` fallback budget (needs a GUI build to thread its model window).
      *(2026-07-13)* **Scheduled dream cycle now honors the user's memory-compression config.** The daemon's
      automatic hourly consolidation built `ConsolidationConfig::default()` (`server.rs`), silently ignoring
      `[memory] max_compression_ratio` / `min_remaining_memories` — while the IPC-triggered path (`control.rs`)
      read them. Worse, `DaemonBuilder::from_nanna_config` never mapped those two settings onto `DaemonConfig`
      at all, so the scheduled cycle always used the 0.50 / 20 defaults regardless of user config. Fixed:
      added `memory_max_compression_ratio` / `memory_min_remaining_memories` to `DaemonConfig` (both
      construction sites are compiler-enforced), mapped them from `config.memory.*` in `from_nanna_config` and
      the legacy `src/main.rs` path, and routed the scheduled task through a pure, unit-tested
      `scheduled_consolidation_config(max_ratio, min_remaining)` helper (mirrors the `control.rs` build) so
      automatic and manual consolidation are now in lock-step. 2 tests (helper threads the values while keeping
      the new cluster-size defaults; `DaemonConfig::default` mirrors `ConsolidationConfig::default`); 41 daemon
      lib tests green, zero new clippy warnings (2067 baseline unchanged), real daemon boot reaches "Daemon
      ready" + schedules the consolidation task cleanly.
- [x] **Implement the missing true merge** — `IngestAction::Update` currently falls back to create/reinforce (`service.rs:300`); add content-level merge so dreaming deduplicates instead of accreting near-duplicates.
      *(2026-07-07) Done for **all three ingest paths** (`smart_ingest`, `remember_with_importance`,
      the scoped variant) via a shared `fold_into_memory` helper: `merge_memory_content` +
      `update_content_and_embedding` fold related-but-distinct content into the existing memory
      (bounded, superset-dedup) and reinforce FSRS. Next: apply the same merge in the batch
      dreaming/consolidation clusterer (`cluster_memories`), which still creates consolidated copies.*
- [x] **Harden `create_consolidated_entry` against NaN** — the FSRS-scalar merge used
      `max_by(|a,b| a.partial_cmp(b).unwrap())`, which **panics the dreaming cycle** if any stored
      `importance`/`storage_strength` is NaN.
      *(2026-07-09)* Replaced with a pure `max_finite_or(values, default)` that skips non-finite inputs
      (NaN/±inf) and falls back to the default when none are finite; added pre/postcondition assertions
      (non-empty cluster in, finite scalars out). 3 unit tests (NaN/inf skipped, max+sum semantics,
      NaN-cluster survives). Removes two prod-path `unwrap`s from the consolidation path.
- [ ] **Indexed clustering** — replace the O(N²) greedy single-pass `cluster_memories()` with HNSW/IVF candidate neighbors + connected-components/HDBSCAN over `composite_cluster_score`; scales past the ~50k in-RAM ceiling.
      - *(2026-07-12, partial)* Interim: the clusterer's per-pair `cosine_similarity` (called O(N²) times per
        band) now delegates to `nanna_simd::cosine_similarity_f32` (AVX-512/AVX2/NEON) — the same primitive the
        vector-search path already uses — instead of a private scalar loop, removing the duplication. Guards
        preserve the "0.0 on mismatch/empty" contract (`nanna_simd` panics on unequal lengths and NaNs on a
        zero-magnitude vector; the clusterer's existing `.max(0.0)` already tolerated it, but the guard makes it
        explicit). Parity test vs a scalar reference over random 768-dim vectors (<1e-4) + zero/mismatch/empty
        edge tests. **The O(N²) structure itself is unchanged — HNSW candidate-neighbor work is still open.**
      - [ ] *(research 2026-07-06)* Use a **pure-Rust HNSW** crate (`hnsw_rs` / `instant-distance`) over a C ext — `sqlite-vec` is brute-force only; `vectorlite` shows HNSW at `ef_construction=100, M=30` scales well. Fits the Turso-only + in-RAM-cosine model (build the index in RAM, persist coeff/graph as Turso BLOBs). Sources: [vectorlite](https://github.com/1yefuwang1/vectorlite), [sqlite-vec ANN issue](https://github.com/asg017/sqlite-vec/issues/25).
      - [ ] *(research 2026-07-09)* Crate shortlist (all pure-Rust, actively maintained early 2026): **`hnsw_rs`** — multithreaded build/search via `parking_lot`, SIMD distances through `anndists` (L1/L2/Cosine/Hamming/…), the most feature-complete; **`hnswlib-rs`** — designed for **concurrent search + concurrent mutation** with an `InMemoryVectorStore` doing **lock-free reads + parallel updates** (best fit for a live memory store that dreams while serving recalls, avoids a global rebuild); **`instant-distance`** — smallest/simplest pure-Rust HNSW if we want minimal surface. Lean `hnswlib-rs` for the online/insert-while-query case, `hnsw_rs` if we need its distance breadth. Sources: [hnsw_rs](https://crates.io/crates/hnsw_rs), [hnswlib-rs](https://github.com/jean-pierreBoth/hnswlib-rs), [instant-distance](https://lib.rs/crates/instant-distance).
      - [ ] *(research 2026-07-10)* Confirmed still current: `hnsw_rs` exposes `insert_parallel` + `parallel_search` (rayon/parking_lot) — the concrete entry points for the "batch-build the index in RAM from the whole `VectorStore`, then query candidates" approach that fits the dream-time clusterer (build once per cycle rather than incrementally). `instant-distance` builds from a `Vec<Point>` in one shot (no incremental insert) — fine for the rebuild-per-dream model, wrong for online mutation. Net: `hnsw_rs::Hnsw::insert_parallel` for the dream-time rebuild; revisit `hnswlib-rs` only if we later need insert-while-serving. Sources: [hnsw_rs docs](https://docs.rs/hnsw_rs/latest/hnsw_rs/hnsw/index.html), [instant-distance](https://github.com/djc/instant-distance).
      - [ ] *(research 2026-07-11)* `hnsw_rs` still actively maintained (crates.io updated 2026-02-28) and now
            documents **in-search filtering** — pass either a sorted `Vec` of allowed ids or a filter closure
            evaluated *before* an id enters the result set (not a post-filter). This is the clean primitive for
            **workspace-scoped recall over one shared index**: keep a single HNSW of all memories and filter to
            the active workspace's ids at query time, instead of rebuilding a per-workspace index — directly
            useful for the P11 "tool-memory workspace scope" item too. Source: [hnsw_rs docs](https://docs.rs/hnsw_rs/latest/hnsw_rs/hnsw/index.html).
      - [ ] *(research 2026-07-16, corrects the crate shortlist)* Two of the three shortlisted crates need
            re-reading. **`instant-distance` is dormant — rule it out**: no release since **0.6.1 (June 2023)**
            despite repo activity, so the "smallest/simplest pure-Rust HNSW" option is not a live choice.
            **`hnswlib-rs` 0.10.0 (2026-01-05) is a *different crate* than the 2026-07-13 note assumed** — it is
            not jean-pierreBoth's; it is a pure-Rust port from the **CoreNN** project (wilsonzlin/corenn). The
            storage-decoupling property still holds and still suits our Turso-backed store. **`hnsw_rs` 0.3.4
            (2026-02-28)** remains current and published (0.3.5 is in `Changes.md` but **unpublished**); its
            `modify_level_scale` (0.3.1) buys better recall, or equal recall at smaller `max_nb_conn` (less RAM).
            Also worth evaluating before we build: **CoreNN** itself — an embeddable pure-Rust vector DB with
            built-in **per-vector int8 quantization** (`insert_qi8`) + f16/bf16 inserts, which overlaps the
            "f16 embedding compression" backlog item. Ruled out: `usearch` (C++ w/ Rust bindings — fails the
            pure-Rust preference); `rust-diskann` 0.3.5 is experimental (~890 downloads). Decision unchanged:
            `hnsw_rs::insert_parallel` for the rebuild-per-dream clusterer. Sources:
            [hnsw_rs Changes](https://github.com/jean-pierreBoth/hnswlib-rs/blob/master/Changes.md),
            [hnswlib-rs 0.10](https://crates.io/crates/hnswlib-rs), [CoreNN](https://blog.wilsonl.in/corenn),
            [instant-distance](https://crates.io/api/v1/crates/instant-distance).
      - [ ] *(research 2026-07-13)* **`hnswlib-rs` (Jan-2026 rewrite) decouples the graph from vector storage**:
            the `Hnsw` struct owns only the graph + an external-key→dense-`NodeId` map, while the caller supplies a
            `VectorStore` keyed by `NodeId`; its `InMemoryVectorStore` does **lock-free reads + parallel updates**,
            built explicitly for *concurrent search while mutating*. This is the cleaner fit than `hnsw_rs` **if**
            we want the index to live persistently and serve recalls while dreaming inserts/mutates — the memory
            store already separates embeddings (Turso BLOBs) from the search structure, so a `NodeId→memory-id`
            map drops in without duplicating vectors. Decision stands: `hnsw_rs::insert_parallel` for a
            rebuild-per-dream clusterer (simpler), `hnswlib-rs` only when we move to a long-lived insert-while-serve
            index. Source: [hnswlib-rs](https://crates.io/crates/hnswlib-rs).
- [ ] **Feedback-driven FSRS** — wire real signals (thumbs, corrections, tool-success/failure) into `DreamingService::record_feedback` so importance is learned, not static.
      *(2026-07-13)* **Feedback accumulator hardened + boost table de-duplicated.** `record_feedback`'s
      `pending_feedback` (`memory_id → Vec<MemoryFeedback>`) was an **unbounded** per-memory accumulator on the
      live service path — a feedback flood between dream cycles grew it without limit (Tiger Style: bound
      everything). Also extracted the ±0.3/0.5 boost table (duplicated verbatim in `apply_feedback` and the
      dream-time aggregation) into one `const fn feedback_boost` so the immediate and deferred paths can't
      drift. (Prereq for the real signal wiring, which is the remaining work here.)
      *(2026-07-13, reworked — bounded by construction, no arbitrary cap)* The first pass capped the `Vec` at a
      retain-16 constant and claimed losslessness — **wrong for mixed-direction floods**: 16 positives followed
      by 20 strong negatives would drop the negatives past the cap and flip the applied sign (+1.0 instead of
      the true −1.0). Since the dream loop only ever consumes the **aggregate sum** (commutative), the signals
      never need retaining at all: `pending_feedback` is now `memory_id → FeedbackTally` — four saturating
      per-variant `u32` counters (a fixed **16 bytes per memory** regardless of flood volume; counters saturate
      at ~4.3 B instead of wrapping). `total_boost()` = Σ count × `feedback_boost(variant)` via fused
      `mul_add` — exactly the signal-by-signal sum, every signal counted, no drop policy, no magic number. 4
      tests (mixed-direction flood → all 36 signals counted, fixed 16-byte accumulator, exact −5.2 aggregate
      with the correct sign; tally == signal-by-signal reference sum; saturate-not-wrap; boost signs). 38
      nanna-memory tests green, net −2 clippy warnings, full workspace builds green, real daemon boot healthy.
      - [ ] *(research 2026-07-06)* **FSRS-6** (late-2025, trained on ~700M reviews) has **17 trainable weights + `w20`** governing the forgetting-curve *shape*; ~20-30% fewer reviews for equal retention. Learn w0-w20 (incl. w20) from the accumulated feedback signals rather than static params. Source: [expertium benchmark](https://expertium.github.io/Benchmark.html).
      - [ ] *(research 2026-07-16)* **FSRS-7 exists, but is not reachable from Rust yet — do not plan on it.**
            The benchmark repo documents FSRS-7 as the newest version (first to handle **fractional intervals**;
            forgetting curve now has **8 optimizable parameters**; the only version with realistic same-day-review
            predictions). **However `fsrs-rs` is 6.6.1 (2026-06-09) and implements FSRS-6** — FSRS-7 support is
            **PR #395, open since 2026-04-07 and still unmerged**, blocked on upstream formula work. So adopting
            FSRS-7 means vendoring an unmerged PR; staying on FSRS-6 is the correct default until it lands.
            (Explicitly unverified: the claim that "FSRS-7 is final" traces to no primary source — Expertium's own
            Algorithm page still documents FSRS-6 only.) Sources:
            [srs-benchmark](https://github.com/open-spaced-repetition/srs-benchmark),
            [fsrs-rs PR #395](https://github.com/open-spaced-repetition/fsrs-rs/pull/395).
      - [ ] *(research 2026-07-16)* **We ship the FSRS-6 curve with the FSRS-5 decay constant — `w20` is wrong
            by ~7.6x.** `nanna-memory/src/fsrs.rs` implements the FSRS-6 forgetting curve *exactly*
            (`R(t,S) = (1 + factor·t/S)^(-w20)` with `factor = 0.9^(-1/w20) - 1`, `power_law_retrievability`),
            but defaults `w20: 0.5` — commented "typically 0.5", which is in fact **FSRS-4.5/5's hardcoded
            `DECAY = -0.5`**, not an FSRS-6 value. **FSRS-6's default `w20` is `0.0658`**; making that exponent
            trainable is the entire point of the version we claim to implement. A 0.5 exponent decays
            retrievability far faster than FSRS-6 intends, so every consumer of retrievability is skewed:
            testing-effect reinforcement, the FSRS weight bands the dream cycle clusters by, and
            `retrieval_strength`. **Do not blind-flip the constant**: it changes live memory behavior, so land
            it behind the **memory retention harness** (recall before/after a dream cycle) already listed under
            *Performance & Benchmarking* — that harness is the instrument that tells us whether 0.0658 actually
            recalls better, and it is exactly the "measure, don't guess" case. Then fit `w0..w20` from the
            accumulated access history rather than any static default (see the 2026-07-06 note above).
            Source: [awesome-fsrs — The Algorithm](https://github.com/open-spaced-repetition/awesome-fsrs/wiki/The-Algorithm).
- [ ] **Local dreaming** — run `summarize_fn` on the local Burn model (P12) so consolidation is fully offline; persist the `SummaryCache` (currently in-memory, lost on restart).

**DSP-backed time-series / event-timeline memory (compression-as-dreaming):**
- [ ] **`nanna-timeline` crate + append-only event log** — `MemoryEvent { id, ts, kind, workspace_id, content, embedding, salience, source_ids }` in a new Turso migration; the raw episodic stream (messages, tool calls, recalls, outcomes) on a wall-clock axis. `MemoryEntry` stays the semantic/fact layer; episodes consolidate *into* facts during dreaming.
- [ ] **Resample the timeline into per-signal series** — salience(t), access-rate(t), emotional valence(t), per-cluster topic-activation(t).
- [ ] **DSP compression = dreaming over time** — keep the recent window at full sample rate; for older windows decimate/wavelet-drop low-energy detail with the **keep-rate driven by FSRS `power_law_retrievability`** — sharp near-term detail, blurred long-term gist. Lift DSP's pure `simplify_with_aggressiveness` + slope-change simplifier + `splimes::auto_interpolate` (see design notes); store decimated windows / coeff blobs as Turso `f32` BLOBs.
- [ ] **Peak detection seeds consolidation** — DSP peak/energy detection marks salient moments → promote those episodes to facts + boost importance; long flat stretches → compress to Essence/drop. Ties the timeline back into the existing FSRS weight bands.
- [ ] **Single-GPU DSP kernels** — implement FFT/wavelet/convolution as wgpu compute shaders in `nanna-gpu` (alongside `CosineSimilaritySearch`), with a CPU fallback in `nanna-simd`. No external DSP service.
- [ ] **Decision — Turso-only vs DSP `.dspseg`:** DSP normally keeps measurements in `.dspseg` files *outside* libSQL. To stay Turso-only, lift DSP's *pure algorithms* (`simplify_with_aggressiveness`, `splimes`) and store reduced points in Turso BLOBs, rather than depending on DSP's `SegmentStore`/`Database`. (Revisit if the timeline outgrows Turso.)
- [ ] **Make it demoable** — GUI dream-log + a salience **spectrogram/waterfall** over time (consolidation lineage `consolidated_from`/`generation` already exists). This is the "unique sauce" screen.
- [ ] Also from backlog: HNSW persistent vector index (avoid full `bulk_load` into RAM); emotional valence; memory-graph edges; dedup-before-store; ~~extraction filtering (<50 chars)~~ **(done 2026-07-06 — `is_storable_memory` drops sub-50-char extractions in `loop_runner::extract_memories`; 2 tests)**.

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

1. ~~**Turso-only cleanup** (P13)~~ — **DONE (2026-07-16)**: ~~rename `SqliteMemoryPersistence`~~ (2026-07-07),
   ~~delete `server.rs.bak`~~ (gone), ~~add the CI dep-guard~~ (2026-07-06), ~~purge "SQLite" from
   comments/logs/var names across storage/daemon/memory/GUI~~ (2026-07-16 — the last slice was
   `gui/.../commands/sessions.rs`; only the intentional factual line at `nanna-storage/src/lib.rs:6`
   remains, by design). SQL, `.db`, and `datetime('now')` untouched throughout.
2. **Bring all deps to latest + commit `Cargo.lock`** (doctrine → *Dependency freshness*) — `Cargo.lock`
   un-gitignored and committed (2026-07-07); compatible deps already at latest (`cargo update` = 0 changes).
   Low-risk majors applied green: `directories 5→6` (unified with the workspace pin), `tower-http 0.6→0.7`
   (daemon+server), `socket2 0.5→0.6` (daemon). **Deferred majors** (each needs a real migration — build
   green + tests + benches before landing; do one per run):
   - [x] `reqwest 0.12→0.13` — *(2026-07-10, part of the big bump)* default features OFF at the
         workspace root with `native-tls` selected explicitly (the 0.13 default flips to rustls+aws-lc,
         whose C/asm build violates "prefer pure-Rust, no-C"); `query`/`form` opt-in features enabled
         (call sites in channels/config/tools); `charset`/`http2`/`system-proxy` re-added. Channels + GUI
         now inherit the workspace dep. No source changes needed.
   - [x] `tokio-tungstenite 0.26→0.29` (client/daemon/gui/mcp/channels) — *(2026-07-10)* compiled unchanged.
   - [x] `deno_core 0.375→0.407` + `deno_ast 0.51→0.53` (nanna-scripting) — *(2026-07-10)* compiled
         unchanged; the direct `swc_core` dep turned out to be **dead** (nothing referenced it, no feature
         enabled it) and conflicted with deno_ast 0.53's exact swc pins (`swc_atoms =9.0.0`) — deleted.
         **boa_engine/boa_runtime are git-pinned to boa main** (rev `4f98f644`): released boa 0.21.1 pins
         icu ~2.0 + an old temporal_capi, conflicting with deno_core 0.407 (v8 149 → temporal_capi ^0.2.3)
         and turso 0.6 (icu 2.2). boa-main API drift was tiny (`JsArray::new` now fallible, 2 sites).
         Drop back to crates.io when boa releases with icu 2.2.
   - [x] `rustpython-{vm,stdlib,pylib} 0.4→0.5` (nanna-scripting) — *(2026-07-10)* migrated to the new
         `Interpreter::builder` (`stdlib_module_defs(&builder.ctx)` + `add_frozen_modules(FROZEN_STDLIB)`
         replace `with_init`/`get_module_inits`); `PyStr::as_str` → `to_string_lossy()` (2 sites).
   - [x] `playwright-rs 0.8→0.14` + `chromiumoxide 0.8→0.9` (nanna-browser) — *(2026-07-10)* chromiumoxide
         0.9 dropped the `tokio-runtime` feature (tokio-only now) and its `Arg` lost `From<&String>`
         (pass owned). playwright-rs compiled unchanged.
   - [x] `wgpu 24→30` (nanna-gpu) — *(2026-07-10)* migrated: `Instance::default()`, `request_adapter`
         returns `Result`, `DeviceDescriptor` gained `experimental_features`/`trace` (+ single-arg
         `request_device`), `Maintain` → `PollType::Wait{submission_index,timeout}` (poll returns Result),
         `get_mapped_range[_mut]` return `Result`, `BufferViewMut` writes via `.slice(..).copy_from_slice`,
         `PipelineLayoutDescriptor.bind_group_layouts` takes `Option<&_>` (+ `push_constant_ranges` →
         `immediate_size`). **Bench-validated live on the 4070 Ti SUPER**: GPU fixed dispatch overhead
         improved ~750µs → ~200µs; SIMD still wins ≤10k vectors (crossover unchanged, `GPU_THRESHOLD`
         stays 50k). Note: the old "wgpu pinned for onyums/tauri/burn" constraint was consciously dropped
         (neither onyums nor burn is in-tree yet; revisit at P9/P12 integration).
   - [x] `wide 0.7→1.5` (nanna-simd) — *(2026-07-10)* `as_array_ref()` → `as_array()` (3 sites).
   - [x] `turso =0.4.4 → =0.6.1` + `aegis =0.9.7 → =0.9.12` (nanna-storage) — *(2026-07-10)* **fixes the
         daemon startup panic** (`turso_core 0.4.4 btree.rs:943 "we can't have more pages to read while
         also have read everything"`) that killed the daemon while bulk-loading the memories table and
         forced the GUI into embedded fallback. Root cause: 0.4.4 wrote an **inconsistent overflow chain**
         into the memories btree, then panicked reading it back. 0.6.1 detects the same condition and
         returns a proper `Err` ("inconsistent overflow chain observed during payload read") which the
         existing load handler logs — **daemon reaches "Daemon ready"** (validated against a copy of the
         real crashing DB). Consequence: memories in the corrupted table are unreadable (load as 0) and
         will re-accumulate. aegis 0.9.12 built clean on stock MSVC (no clang-cl needed in this setup).
   - [x] `keyring 3→4` (nanna-config) — *(2026-07-09)* v4 split platform stores into per-OS `*-keyring-store` crates (no longer default); added `apple-native-keyring-store` and kept the default `windows-native-keyring-store` + `zbus-secret-service-keyring-store` + `v1` compat feature, which preserves the `Entry`/`Error::NoEntry` API so `credentials.rs` compiled unchanged. Build+tests green.
   - [x] `ed25519-dalek 2→3`, `hmac 0.12→0.13`, `sha2 0.10→0.11` (nanna-server + nanna-daemon) — *(2026-07-09)* bumped in lockstep across both crates. Only breakage: hmac 0.13's `Mac` trait no longer re-exports `new_from_slice`, so the Slack-HMAC call sites now `use hmac::KeyInit`. ed25519-dalek 3 (`from_bytes`/`verify_strict`/`Signer`) and sha2 0.11 compiled unchanged. Webhook signature tests (Slack HMAC-SHA256 + Discord Ed25519, incl. tamper/replay cases) stay green; 25 daemon lib tests pass.
   - [x] `scraper 0.22→0.27`, `lopdf 0.34→0.44` (nanna-tools) — *(2026-07-10)* both bumped, no code
         changes; markup5ever/selectors/cssparser pulled forward transitively. `nanna-tools` builds green,
         44 tests pass.
   - [x] `rand 0.8/0.9→0.10` (channels, gui), `toml 0.8→1.1` (gui), `nix 0.29→0.31` (unix), `tokio 1.52`,
         `uuid 1.23`, `half 2.7`, `bytemuck 1.25`, `sha2 0.11` (gui) — *(2026-07-10)* all compiled unchanged.
   - [x] `windows-service 0.7→0.8` (daemon) — *(2026-07-10)* bumped, no code changes; `windows_service.rs`
         API (`service_dispatcher`/`service_control_handler`/`ServiceStatus`) unchanged. Daemon builds green,
         26 tests pass.
   - [x] `criterion 0.5→0.8` (nanna-gpu benches) — *(2026-07-10)* bumped; the four benches use
         `harness = false` (custom mains) so criterion is an unreferenced dev-dep — benches compile clean.
   - [~] GUI `pnpm update --latest` sweep in `gui/` — *(2026-07-11)* **safe minors/patches applied green**
         (`@tauri-apps/{api 2.11.1, cli 2.11.4, plugin-dialog 2.7.1, plugin-notification 2.3.3, plugin-shell 2.3.5}`,
         `nuxt 4.4.8`, `@vueuse/core 14.3.0`, `tailwindcss`/`@tailwindcss/postcss 4.3.2`, `postcss 8.5.16`,
         `tailwind-merge 3.6.0`, `vue 3.5.39`, `@monaco-editor/loader 1.7.0`) — verified by `pnpm build`
         (client+nitro, 3365 modules) **and** a `pnpm dev` boot serving a real 200 `__nuxt` shell on :3000.
         **Deferred majors (each needs a code migration — do one per run, verify via `cargo tauri build`
         + WebDriver before landing):**
     - [ ] `@tiptap/* 2.11.5 → 3.x` — tiptap v3 **removed the `BubbleMenu` named export from
           `@tiptap/vue-3`** (breaks `FloatingToolbar.vue`; the whole P7 editor needs the v2→v3 migration:
           new BubbleMenu wiring, extension API changes). Largest of the batch.
     - [ ] `vue-router 4 → 5` (major)
     - [ ] `vue-sonner 1 → 2` (major — toast API)
     - [ ] `marked 17 → 18` (major — chat markdown renderer; audit render output)
     - [ ] **`lucide-vue-next` → `@lucide/vue` (package rename, not a version bump).** *(2026-07-16 —
           corrected: the earlier "0.563 → 1.0, low risk" read was wrong.)* `lucide-vue-next@1.0.0` is a
           **deprecation tombstone** ("Package deprecated. Please use `@lucide/vue` instead") — it is the
           `latest` dist-tag but is not a functional release, so `pnpm update --latest` silently installs a
           dead package. The whole `lucide-vue-next` package is deprecated at every version. Real latest
           functional release is **0.577.0** (applied this run). Migration = switch to `@lucide/vue` and
           rewrite the import specifier across the **40 files** that import icons; verify via
           `cargo tauri build` + WebDriver.
     - [x] ~~`@formkit/drag-and-drop 0.5 → 0.6`~~ — **dep removed instead** *(2026-07-16)*: it was an
           **unused dependency** (zero references anywhere in `gui/` outside `package.json`/lockfile —
           the editor's drag-drop is Tiptap's own). Bumping dead weight is noise; dropped it. `pnpm build`
           green after removal, confirming it was genuinely unreferenced.
   - Pins now: `turso =0.6.1`, `aegis =0.9.12` (exact — pre-1.0), boa git rev `4f98f644` (until a
     crates.io boa ships icu 2.2). The old `wgpu` pin is dropped (see the wgpu 30 note above).
   - *(2026-07-16 sweep)* `cargo update` → 12 compatible bumps (`tokio 1.52.4`, `uuid 1.24.0`,
     `keyring 4.1.5`, `regex 1.13.1`, `clap 4.6.2`, `syn 2.0.119`, `bitflags 2.13.1`, `bstr 1.13.0`,
     `regex-automata 0.4.16`, `simd-adler32 0.3.10`, `which 8.0.5`). `cargo upgrade --incompatible` →
     only two reqs behind: `deno_core 0.407 → 0.408` (nanna-scripting; compiled unchanged, no source
     edits) and `uuid 1.23 → 1.24` (workspace + nanna-server req bump). Workspace **including
     `nanna-gui`** builds green; scripting 19+1 / llm 28 / agent 61 tests pass; clippy clean on the
     bumped crates. Frontend: `tailwindcss`/`@tailwindcss/postcss 4.3.3`, `postcss 8.5.19`,
     `vue 3.5.40` applied green (`pnpm build` → nitro + client, 2.25 MB / 502 kB gzip).
   - **Build-env note (not a code bug):** `cargo build -p nanna-gui` needs two artifacts the repo does
     not commit — the Tauri **sidecar** `gui/src-tauri/binaries/nanna-daemon-<triple>.exe`
     (build via `pnpm build:daemon`, per that dir's `.gitkeep`) and the built frontend at
     `gui/.output/public` (`pnpm build`, else `generate_context!` panics with "`frontendDist` …
     doesn't exist"). A fresh worktree needs `pnpm install` + both before the GUI compiles.
   - **`cargo fmt --all` is not safe to run blanket:** `origin/master` is not fmt-clean and the repo has
     mixed CRLF/LF line endings with `core.autocrlf=false` / `core.eol=lf` / no `.gitattributes`, so
     `cargo fmt --all` rewrites ~165 files (mostly pure EOL churn). Format only the files you touch.
     - [ ] Decide the line-ending policy: add a `.gitattributes` (`*.rs text eol=lf`) and land one
           tree-wide `cargo fmt` normalization commit, so future runs can use `fmt`/`fmt --check` normally.
3. **`nanna-infer` Burn skeleton** (P12) — one binary, dual `wgpu`+`ndarray` backend, runtime GPU probe, load one small model, greedy decode: prove local inference end-to-end on the dev GPU.
4. **Local embeddings in Burn** (P12) — MiniLM-class CPU embedder wired into the memory `embed_fn` → fully-local memory (no API embeddings).
5. **`Provider::Local` in the router** (P12) — dispatch completion/stream/tool-calls to `nanna-infer` and make local the top-priority (zero-cost) tier; cloud becomes opt-in escalation.
6. **Unify + upgrade dreaming** (P13) — one `DreamingService` orchestrator, idle-gated multi-phase cycle, true merge, local `summarize_fn`.
7. **`nanna-timeline` + compression-as-dreaming** (P13) — append-only event log in Turso + lift DSP's `simplify_with_aggressiveness`/`splimes` as the timeline compressor keyed by FSRS retrievability.
8. ~~**Fix the two path-traversal holes** (P11 security) — user-tool names + workspace file writes.~~ **(done 2026-07-06)**
9. **End-to-end daemon test** (P8) — ~~the daemon/embedded/reconnect story is still unverified~~ **mostly
   done (2026-07-16)**: a hermetic 4-test E2E suite (`crates/nanna-client/tests/e2e_daemon.rs`) now covers
   start → connect → session state → client reconnect → **daemon restart persistence**, and caught a real
   `Client::disconnect()` state bug. Still open: a real conversation turn (needs a live LLM) and the
   **embedded-fallback** path (needs a GUI build).

