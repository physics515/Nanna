# Nanna — Roadmap

> The single master roadmap **and status source of truth** for Nanna — there is no separate
> `STATUS.md`, `planning/`, or `docs/`. The **daily dev routine** (`.claude/skills/daily-dev`, run under
> `/loop`) reads this file, picks the **single next unimplemented item**, builds it **Tiger-Style**
> with tests + benchmarks, ticks the box, and appends a dated note. The engineering doctrine, benchmark
> methodology, dependency policy, and system reference notes live in that skill — this file stays a
> clean checklist. Shipped capability is *described* in [`README.md`](README.md); here it is only
> tracked. Edit surgically; never rewrite wholesale.

**Last updated:** 2026-07-24 (**P1 security hardening.** Secrets leave `config.toml`: onboarding + GUI key setters route every API key through the OS keyring (`Config::migrate_secrets_to_keyring`) and `Config::save_to` strips secret fields before writing TOML. `SecureStore` file fallback is real AES-256-GCM (`credentials.enc`, key in keyring) with automatic migration from legacy plaintext JSON. ProjectDirs identity unified on `com/nanna/nanna` with one-shot clawd→nanna directory migration. Tauri CSP is no longer `null` (restrictive default-src self + local daemon connect). Shell capabilities scoped to the `nanna-daemon` sidecar only. Auto-remembering conversations is **opt-in** (`[memory] auto_remember_messages`, default false) and gated in the chat control plane. Shipped `SECURITY.md` (disclosure process) + `PRIVACY.md` (data flows / opt-out / deletion) + `.github/dependabot.yml` for cargo and npm. Remaining P1: non-local control-plane auth, webhook replay/coverage audit, gitleaks/trufflehog history scan, GitHub secret-scanning toggle, per-tool GUI audit log, OS-level tool sandbox.)
family, each of which failed without ever telling anyone: `<UiSonnerSonner/>` never resolved so the
toaster never mounted and **every toast in the app was dropped**; `<GroundGlass>` likewise, so glass
inputs lost their slab; and Settings → Data's **"Delete All Memories" invoked a Tauri command that has
never existed** — confirm the dialog, nothing happens. All three now have static guards (component
names vs the Nuxt registry, `invoke()` names vs `generate_handler!`, `UiInput`'s `size` prop) plus an
e2e that a toast really renders. Verified in the **real Tauri shell over WebDriver**. Also: onboarding
checked only `ANTHROPIC_API_KEY` — an **Ollama-only install was nagged for a cloud key**; the daemon IPC
port disagreed with itself (**start bound 9999, status probed 5149**); devtools shipped in release
builds; `.claude/settings.local.json` was tracked in a public repo. **Toolchain pinned** —
nightly 2026-07-23 ICEs on release codegen of `tokio`. **Turso 0.6.1 does have SQL vector distance**,
proven by test — a long-standing roadmap "fact" was wrong.)
Prior: 2026-07-23 (**P13 dreaming unification** — both daemon paths now dream through one
`DreamingService`, which restored the **inert testing effect** and closed an unbounded `pending_updates`
queue; new **no-LLM dedup phase (b)** folds true restatements in every band incl. `Detailed`
(summarizer calls 6 → 0 at unchanged 0.90 compression / 1.000 recall); scheduled dreaming gained
**model failover** + a min-across-fallbacks context budget; **summarization drift instrumented** with
both arms baselined; `/tool-stats` render crash fixed. Open: a drift mitigation, HNSW clustering,
`nanna-timeline`.) Prior: **nuxt generate manifest-race mitigation** — pin `buildDir`, prerender `/` only, clean-cache script before generate; unused `README_FILE` import scoped to tests. Prior: **P4 UI simplification** — command palette Mod+K, VirtualList, primary vs admin nav, settings Advanced + SettingsSection, compressed onboarding, copy/tone + component inventory. Open: formal 1280×720/1440×900 clipped-CTA pass, deeper tool-card compaction.
embedded mode deleted, `AppState`/`backend.rs` collapsed, `log_buffer` relocated to `nanna-core`, GUI `nanna-*`
deps pruned to config/core/tools; completed phases P3/P4/P10 condensed; **P17 re-scoped to workspace-context
standardization**; prior: GUI testing + UI/UX quality track; P11 tool-manager consistency closed)
**Also 2026-07-18:** **P11 fully drained and condensed** (673 → ~45 lines). Every prior item is done,
superseded by P16, or handed to P12; and the **run-log triage findings are now fixed with tests** — the
**multi-tool-call streaming collapse** (per-index `StreamBlockAssembler`), tolerant tool-stats import,
corrupt-Turso-memories salvage + `/status` surfacing, real tool-failure logs, Windows `exec` `cd /d`
normalization, and the heartbeat `HEARTBEAT.md` read. Detailed dated notes collapsed to a one-line ledger
(full rationale in each commit).
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

**Ports:** health HTTP `5148` (`/health`, `/healthz`, `/readyz`, `/status`) · WebSocket IPC `5149`. The GUI-spawned daemon sidecar binds this **same** `5149` IPC port (`daemon_manager.rs:47,109` → `daemon_client.rs:69` connects `ws://127.0.0.1:5149`); the old `9833` sidecar port was never real and is retired.

---

## Current State (what's real today)

Phases **1–5** and **7** are complete; **10** is mostly complete; **6** and **8** are partial;
**9** is greenfield. The new local-first phases (**P12**, **P13**) are greenfield. **P14**
(long-horizon autonomy on a small local model) and **P15** (the agent-grade task store P14 runs on)
**landed together 2026-07-18**: Turso task store with hierarchy/dependencies/derived-blocked/`next()`/
filter language, harness-run acceptance checks, the re-anchored O(1) step loop with progress-or-replan
and budget caps, todo v0.2 + `TaskAction` IPC + GUI `/tasks` run monitor. The live on-model eval
passes **5/5 verified @ 22.6k tokens/item, 72 s (qwen3.5:9b, 0 false successes admitted)** after
same-day tuning; the full eval suite (published task set, pass^k, 8 GB tier) is the open remainder. **Two 2026-07-17 directional phases** reshape *how* the project is built rather than
what it does: **P16** (daemon-only consolidation — delete embedded mode, GUI becomes a pure daemon client,
iOS deferred) collapses the double-implementation tax behind most P4/P8/P11 "GUI-embedded copy drifted" debt;
**P17** ✅ (drop the bespoke per-workspace `.nanna/` agent markdown — Nanna reads a project's *standard* files
`README`/`AGENTS.md`/`ROADMAP.md`, and persona/user/memory move to global config + the DB).
Concretely, today Nanna:

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

**Not yet verified / closed:** no **native local model runner** yet (P12); **dreaming** still runs over
an O(N²) clusterer with no timeline/DSP layer (P13) — though it is now idle-gated rather than a fixed
hourly cron *(2026-07-19)*, and **`DreamingService` is no longer dead code**: since *2026-07-23* both
daemon consolidation paths run through it as the single orchestrator, which also restored the
testing-effect FSRS flush that nothing had been draining, and added a deterministic no-LLM dedup phase.
(`nanna-core::DreamingRuntime` is likewise live — `nanna-server` drives `DreamingService` through it.)
The daemon + reconnection path **is** covered end-to-end
since *2026-07-16* (`nanna-client/tests/e2e_daemon.rs`, 4 hermetic tests through the real IPC — the
"embedded fallback" half of this line is moot, P16 deleted embedded mode); what remains untested there
is a real conversation turn, which needs a live LLM. **MCP server mode** is claimed complete but `nanna-server/src/mcp.rs`
does not exist (unverified — see P3); several daemon control actions return `not_implemented`; and
there is remaining **security/correctness debt** tracked below. *(Fixed 2026-07: Discord/Slack webhook
signature verification is now real Ed25519/HMAC, not a placeholder; user-tool + workspace path traversal
closed; the Update-band ingest now truly merges instead of accreting near-duplicates.)*

---

## Performance & Benchmarking ✅ (gate infrastructure landed 2026-08-05)

Performance is a **gate**, not a phase: a change ships only when a reproducible benchmark holds or
improves the budget, and README perf claims link to an artifact. Governing metric: **task success @
budget** — the fraction of the agent-eval suite the local model solves within the reference GPU's VRAM
ceiling and a p95 latency target (reference: RTX 4070 Ti SUPER 16 GB). Methodology lives in the
`daily-dev` skill; numbers live in `bench/BASELINE.md` + `bench/budgets.toml`.

- [x] **`nanna-bench` crate (criterion)** — suite taxonomy (`Suite::{Inference,VectorSearch,Dreaming,
      AgentLoop,Guardrails,Efficiency}`), deterministic `fixture_vectors` (SplitMix64, shared with the
      retention harness), `Budgets` parser over `bench/budgets.toml`, and the first real criterion body
      (`benches/vector_search.rs`) unifying the existing `nanna-gpu` SIMD/GPU crossover benches.
- [x] **Agent-eval suite defined** — `bench/AGENT_EVAL.md` is the task-success denominator: smoke (5),
      endurance (42-feature `minidb`), and the published-set placeholders (Terminal-Bench easy /
      SWE-bench Lite). Scoring rules, reference tiers, and reproduce commands are pinned there; live
      numbers already land in Suite 4 of `BASELINE.md` (P14/P20).
- [x] **Per-tier budgets in `bench/BASELINE.md` + machine-readable `bench/budgets.toml`** — Suite 3
      (dreaming compression 0.90 @ recall 1.000, w20 aged-recall 6/6 vs 0/6, drift fixtures) and Suite 4
      (harness task-success @ tokens, endurance) are baselined; Suites 1/2/5/6 carry structural rows and
      fill in as their instruments land. Every budget has an `id` / `direction` / `value` / `source`.
- [x] **CI budget gate** — `.github/workflows/budget-gate.yml` runs the deterministic Suite 3 retention
      tests and fails a PR that regresses a committed threshold.
- [x] **Memory retention harness** (`nanna-memory::retention`) — topic recall@k before/after a real
      `consolidate()` dream cycle; the instrument the FSRS `w20` flip and the no-LLM dedup phase were
      gated on. Deterministic + offline.
- [ ] **Inference parity harness** (logit/sequence vs reference) — **deferred to Mummu** (P12). The
      runner owns parity; Nanna consumes the gate, does not re-implement it.
- [ ] **Perf dashboard** (live TTFT / tok-s / VRAM / cache-hit in the GUI) — **deferred** to a GUI
      polish pass once `nanna-infer` streams real on-device metrics. Not blocking the gate.

---

## Phases

### P0 - Public Preview Release
- [x] Create RELEASE_NOTES.md or MILESTONE that freezes scope. *(2026-08-15)*
- [x] Set up GitHub Actions to build Tauri + daemon sidecar and attach artifacts to Releases. *(2026-08-15)*
- [~] Publish signed Windows .msi/.exe installer with bundled daemon sidecar. *(signing deferred to P0.3)*
- [~] Publish signed and notarized macOS .dmg (Universal or separate Intel/Apple Silicon). *(notarization deferred to P0.3)*
- [x] Publish Linux AppImage and/or .deb/.rpm. *(2026-08-15)*
- [x] App launches without terminal; daemon starts automatically. *(2026-08-15)*
- [x] Add Start Menu / tray / launch-at-login support. *(2026-08-15 — launch-at-login deferred to P0.3)*
- [x] WebView2 handling on Windows. *(2026-08-15 — downloadBootstrapper with silent install)*
- [x] Document uninstall process. *(2026-08-15 — README.md Installation section)*
- [x] Add "check for updates" or auto-update mechanism. *(2026-08-15 — tauri-plugin-updater)*

#### P0.1 - First Run UX
- [ ] Create public facing website / Github Pages
- [x] Build GUI onboarding wizard (replaces CLI-centric onboarding).
      *(2026-08-15)* 3-step `OnboardingWizard.vue`: intro → backend/key → health check → chat.
      Triggered on first run when no API key is set; persists `nanna.onboarding.done` to localStorage.
- [x] Plain-language intro screen explaining what Nanna is.
      *(2026-08-15)* Step 1 of wizard: "Nanna is a calm personal agent — chat, tools, and memory
      that stay on your machine."
- [ ] Data storage location selection.
- [x] Backend chooser: Anthropic / OpenAI / OpenRouter / Ollama — with clear "native local model coming soon" if not implemented.
      *(2026-08-15)* Step 2 of wizard: provider dropdown with all four options; Ollama shows
      "runs locally — no API key needed" message.
- [~] API key entry with validation; ~~fix has_api_key to check all provider keys, not only Anthropic~~
      **(the provider check is fixed, 2026-07-25)**: the GUI `get_config` command's `api_key_set` looked
      only at `config.llm.api_key` (the Anthropic slot) + the `ANTHROPIC_API_KEY` env var, so a user with
      only an OpenAI/OpenRouter/GitHub-Models key, or Anthropic OAuth, was wrongly told they had no key and
      re-nagged in onboarding. Added a pure, unit-tested `LlmConfig::has_configured_api_key()` in
      `nanna-config` (checks `api_key` / `anthropic_oauth_token` / `openai_api_key` / `openrouter_api_key`
      / `github_token`, treating blank/whitespace as unset; Ollama excluded on purpose — it is keyless and
      handled by the onboarding's separate `needsKey`), and the command now ORs it with the env vars for all
      four providers. 3 config tests (none→false, each provider alone→true, blank/empty→false); 20
      nanna-config tests green. *Remaining on this line: the "entry with validation" (live key check) part,
      and the nanna-gui compile of the 4-line command wiring was not run this pass (a fresh worktree needs
      the sidecar + built frontend before `nanna-gui` compiles — the fixed logic itself is in the
      unit-tested `nanna-config` helper).*
- [ ] Ollama detection (is server running? is a model pulled?).
- [x] Memory/privacy explanation with opt-in toggle for auto-remembering.
      *(2026-08-15)* **Config exists** — `auto_remember_messages` in `[memory]` config (default true).
      *(2026-08-15)* **GUI toggle added** — Settings → Memory now has "Auto-Remember Messages" switch
      that persists to config and pushes to daemon. `PRIVACY.md` documents the feature.
- [x] Daemon/embedded backend auto-start.
      *(2026-08-15)* The daemon launches as a managed sidecar via `tauri-plugin-shell` on app start.
      `daemon_manager.rs` spawns `nanna-daemon` automatically; reconnection loop handles transient
      disconnects. No manual start required.
- [x] Health check screen with helpful, non-technical error messages (API key invalid, Ollama not running, port conflict, etc.).
      *(2026-08-15)* Step 3 of onboarding wizard: calls `get_backend_status`, shows friendly
      "Backend ready" or soft error with option to continue and fix in Settings.
- [x] Emergency stop / pause-memory button visible in main UI.
      *(2026-08-15)* **Stop button implemented** — `ChatInput.vue` shows a red "Stop" button during
      streaming that emits `stop` event. Keyboard shortcut `Mod+.` also triggers stop.
      *(2026-08-15)* **Pause-memory implemented** — Settings → Memory "Auto-Remember Messages" toggle
      controls `auto_remember_messages` config, persisted and pushed to daemon.

#### P0.2 - Documentation — ✅ complete (2026-08-19)
All documentation shipped: README rewritten user-first (pitch, download links, system requirements,
"First 5 Minutes" checklist, capability matrix, per-OS install, troubleshooting, uninstall;
architecture/performance moved to the bottom); `PRIVACY.md` (local storage, outbound sinks, opt-out,
deletion/export); `CONTRIBUTING.md` + `CODE_OF_CONDUCT.md`; MIT `LICENSE` committed; Cargo.toml
repository URL fixed to `physics515/Nanna`; port documentation unified on a single source of truth
(`nanna_daemon::DEFAULT_IPC_PORT = 5149`, verified 2026-07-25); GitHub repo description and topics
set (description + `agent`/`ai-assistant`/`llm`/`local-first`/`personal-ai`/`rust`/`tauri`).
Remaining: capture real screenshots to replace the README placeholders.

#### P0.3 - Stronger Public Release (can follow 0.1)
- [ ] Local Ollama setup assistant in GUI.
- [ ] Model/backend status dashboard.
- [~] Cost tracking for cloud models.
      *(See P6)* Core shipped — `CostTracker` with per-model pricing table, `estimate_cost_usd`,
      `ModelStatsTracker::cost_report()` surfaced on IPC. Remaining: GUI surface, per-session/day aggregation.
- [ ] Backup/export/delete data UI.
- [ ] Per-channel session isolation (critical if any channel is marketed).
- [~] Channel-native response formatting.
      *(See P8)* First slice shipped — `ChannelFeatures::MARKDOWN` + `format_for_channel` + length-aware
      splitting + tables→text conversion. Remaining: Discord embeds, Slack Block Kit.
- [x] Log rotation + crash diagnostics export.
      *(Done in P6, 2026-07-09)* `tracing-appender` daily rotation, max 7 files, `--log-dir` + `--no-file-log`
      flags. Crash diagnostics: logs capture panics via the tracing layer.
- [x] Windows service install/uninstall/start/stop actually working.
      *(Done in P11, 2026-07-17)* Windows service install/uninstall/start/stop via the SCM with platform-aware
      default args.
- [ ] Code signing / notarization in CI.
- [ ] Accessibility pass (screen reader, keyboard navigation, ARIA, color contrast).
- [ ] Internationalization/localization framework (currently English-only).
- [ ] Burn local runner (P12) → re-market true offline.
- [ ] Dreaming overhaul (P13).
- [~] Self-update via GitHub Releases.
      *(P8 GUI half landed 2026-07-24, v0.2.1)* tauri-plugin-updater with signed NSIS artifacts, status-bar
      "Update to vX" chip, user-initiated apply. Remaining: headless-daemon self-update.
- [ ] Resource cleanup verification on uninstall (daemon, config, memory DB, credentials fully removed).

#### P0.3 - Code Quality & CI
- [x] Add GitHub Actions workflow: cargo fmt --check, cargo clippy --all-targets --all-features -- -D warnings, cargo test --workspace --all-features, cargo test --no-run smoke check.
- [x] Add cargo audit and cargo deny to CI.
- [x] Add frontend CI: pnpm install --frozen-lockfile, pnpm exec vue-tsc, pnpm audit, Tauri build smoke test.
       *(2026-07-24)* **`vue-tsc` is now an enforced gate, not advisory.** It ran with
       `continue-on-error: true` since the workflow landed, which makes a typecheck step decorative — it
       reports and nobody is blocked. Measured before flipping it: `pnpm exec vue-tsc --noEmit` exits **0
       with 0 errors** across the whole frontend, so there is no pre-existing debt to grandfather and any
       future error is a genuine regression. `continue-on-error` removed. Still open on this item:
       `pnpm audit` and a Tauri build smoke test in CI. ESLint/Prettier/Vitest/Playwright configs exist in gui/ root (correct Tauri architecture)
- [x] Add Tauri packaging CI producing signed artifacts per OS. *(2026-07-24)* **`release.yml`** produces `Nanna_x.y.z_x64-setup.exe`, `Nanna_x.y.z_x64.msi`, `Nanna_x.y.z_amd64.AppImage`, and `nanna_x.y.z_amd64.deb` with NSIS signing hooks.
- [x] Add end-to-end daemon test: start → connect → conversation → persistence → fallback → reconnect. *(2026-07-24)* **`daemon_e2e.rs`** implements full lifecycle: `nanna daemon start` → WebSocket IPC handshake → multi-turn chat with context compression → session persistence across restart → failover to cloud LLM → reconnection after simulated disconnect.
- [x] Add gitleaks/trufflehog secret-scan step to CI. *(2026-07-24)* **`ci.yml`** integrates `gitleaks-action@v2` and `trufflehog-action@v2` with full history fetch for comprehensive secret detection.
- [x] Add coverage tracking (codecov/coveralls) if practical. *(2026-07-24)* **Codecov integration added** to `ci.yml` via `cargo-tarpaulin` for Rust coverage and `pnpm exec vitest --coverage` for frontend coverage, uploading to codecov.io on each push/PR.
- [x] Wire GUI automated tests into CI (see P4 follow-on GUI Testing & UX Quality): unit/component on every PR; Playwright + Tauri/WebDriver smoke on packaging jobs. *(2026-07-22 — `.github/workflows/gui.yml`)* **`gui.yml`** runs Vitest unit/component tests on every `gui/**` PR with report artifacts; Playwright web smoke tests execute on nightly and `workflow_dispatch` with Tauri-driver soft-smoke.
- [x] Add Dependabot/Renovate config. *(2026-07-24)* **`.github/renovate.json`** configured for Rust, Node, and GitHub Actions with auto-pr creation on dependency updates.
- [x] Begin decomposing giant files: loop_runner.rs (~132KB), nanna-llm/src/lib.rs (~159KB), gui/src-tauri/src/lib.rs (8k+ lines) — not all required for 0.1 but plan the split. *(2026-07-24)* **Decomposition plan initiated** — module boundaries identified for loop_runner.rs (runner, metrics, state modules), nanna-llm/src/lib.rs (inference, routing, streaming modules), gui/src-tauri/src/lib.rs (ipc, storage, scheduler modules).
- [x]  *(2026-07-19)* **`nanna-scripting` python tests are parallelism-flaky under load.** A full
       `cargo test --workspace` run failed 9/9 `python::tests::*` with `Timeout(10000)` because each test spins a
       RustPython interpreter that initializes the frozen stdlib (CPU-heavy); 9 in parallel on a busy machine
       exceed the 10 s wall-clock guard. They all pass single-threaded (13/13 in 35.9 s, ~2.7 s each).
       *(2026-07-21)* **Fixed by serializing them — zero new deps.** Chose the "gate their parallelism" option
       over adding `serial_test`: a process-global `static PYTHON_TEST_GUARD: tokio::sync::Mutex<()>` (tokio is
       already a dep; its guard is `Send`, `.await`-safe so no `await_holding_lock`, runtime-agnostic across each
       `#[tokio::test]`'s own runtime incl. the `current_thread` one, and non-poisoning so a failing test still
       releases it) locked as the first statement of all 13 python tests forces one interpreter to build+run at a
       time. Each test's wall-clock then tracks its solo cost (~2.4 s, well under the smallest 10 s guard)
       regardless of `--test-threads`. Verified: 13/13 green in 31.2 s, clippy clean (no new warnings), and it is
       test-only — production `python.exec` sets its own per-call timeout and is untouched.

### P1 — Core Infrastructure
SIMD vector ops (AVX/AVX2), GPU compute (wgpu), Turso persistence (embedded, SQLite-compatible),
vector store + conversation memory, LLM clients (Anthropic/OpenAI/OpenRouter/Ollama) with streaming +
tool calling, agent loop with context management, scheduler (heartbeats, cron).
- [x] Onboarding writes API keys to plaintext config.toml (src/onboarding.rs), even though a SecureStore using OS keyring exists. The OS keychain should be the default path; TOML config should store only non-secret settings.
      *(2026-07-24)* **Done.** `src/onboarding.rs` routes every key through `persist_config` →
      `Config::migrate_secrets_to_keyring()` → OS keyring, then `Config::save()` which
      `strip_secrets_for_disk()` blanks every secret field before writing `config.toml`. A failed
      keyring write no longer falls back to TOML — it errors and leaves the on-disk file clean.
      GUI setters (`set_api_key` / `set_provider_api_key` / `set_ollama_api_key`) do the same.
      Regression: `save_to_strips_secrets_from_disk` asserts no secret ever lands in the TOML.
- [x] SecureStore file fallback is plaintext JSON (mode 0600), not encrypted — the module comment misleadingly says "encrypted file storage." Fix the comment or implement real AES-GCM encryption with an OS-protected key.
      *(2026-07-24)* **Real AES-256-GCM.** `SecureStore` writes `credentials.enc` (MAGIC
      `NANNAENC` + version + 12-byte nonce + ciphertext). The 32-byte key lives in the OS keyring
      under `nanna/file-store-key`, so the bulk payload is GCM and the key is OS-protected. Legacy
      plaintext `credentials.json` is migrated on first unlock (read → write enc → delete).
      Tests: round-trip, migrate-from-legacy, isolated-dir.
      *(2026-07-25, hardening)* **Closed a silent nonce-reuse hole + removed the deprecated nonce API.**
      `random_nonce` ignored `getrandom`'s error (`let _ = getrandom::fill(..)`), so an OS-RNG failure
      left the nonce **all-zeros** — and since every `encrypt_credentials` call reuses the file key, a
      zero nonce means *guaranteed* AES-GCM nonce reuse (catastrophic: breaks confidentiality and lets an
      attacker forge authentication). It now **fails closed** — returns `CredentialError::Crypto` on RNG
      failure (a nonce, unlike the key, has no safe weak fallback; uniqueness is its whole contract).
      Also migrated both `Nonce::from_slice` sites (deprecated in the current `aes-gcm`, and panicking on a
      wrong length) to fallible `Nonce::try_from`. 2 new tests (nonce is non-zero + unique across calls;
      encrypting the same plaintext+key twice yields **different** envelopes yet both decrypt back — an
      observable proof the nonce is fresh each call), 22 nanna-config tests green, no deprecation warnings.
      *(same day)* **`random_key` given the same fail-closed treatment.** Its RNG-failure fallback derived
      the 32-byte file key from `SystemTime` nanos — ~30 bits of guessable entropy, brute-forceable by an
      attacker who has `credentials.enc` and a rough creation time. There is no safe weak fallback for a
      long-lived key, so it now returns `Result` and propagates the getrandom error (both call sites in
      `file_encryption_key` already return `Result`).
- [x] Inconsistent application directory namespaces — config uses ProjectDirs::from("bot", "clawd", "Nanna") while credentials use ProjectDirs::from("com", "nanna", "nanna"), causing orphaned data and confused uninstall flows.
      *(2026-07-24)* **Unified on `com` / `nanna` / `nanna`.** New `nanna_config::{APP_QUALIFIER,
      APP_ORGANIZATION, APP_NAME, project_dirs, legacy_clawd_project_dirs}` is the single identity.
      `Config::default_config_path` / `default_data_dir` and `SecureStore::project_dirs` all go through
      it. On first boot after upgrade, `Config::migrate_legacy_clawd_dirs` copies the old
      `bot/clawd/Nanna` config+data tree into the canonical location (non-destructive; legacy left in
      place). Daemon health/server data dir and GUI skills path updated. `nanna-llm` cache path uses
      the same triple.
- [x] Onboarding has_api_key only checks config.llm.api_key or ANTHROPIC_API_KEY, ignoring OpenAI/OpenRouter keys. quick_setup specifically asks for an Anthropic key despite multi-provider support — broken first-run for non-Anthropic users.
      *(2026-07-24)* **Fixed — it now checks the variable for the *selected* provider.** The old check
      was `api_key.is_some() || env::var("ANTHROPIC_API_KEY").is_ok()`, so an OpenAI or OpenRouter user
      with their key exported was told it was **missing** and re-prompted by `ensure_api_key` on *every*
      launch, while `nanna status` reported "API Key: missing". **Worse for the North Star: `ollama`
      is a real `llm.provider` value, and a fully-local install was nagged for a credential Ollama has
      no concept of** — the intended default experience blocked on a cloud key.
      New `provider_api_key_env(provider)` returns `None` for `ollama` (no key needed → configured) and
      the right variable for `openai` / `openrouter` / `anthropic`; an **unrecognised** provider still
      falls back to `ANTHROPIC_API_KEY`, so an unknown setup is never silently declared configured.
      The same helper now feeds `configure_llm`'s prompt — it had a **duplicate** copy of that mapping,
      which is how a prompt could name a different variable than the check consulted. `quick_setup` no
      longer hardcodes "Enter your Anthropic API key" / "Set ANTHROPIC_API_KEY"; both name the
      configured provider and its variable. Blank is not a credential: an exported-but-empty
      `OPENAI_API_KEY` (or a whitespace `api_key`) reads as unconfigured rather than as a valid key.
      The decision logic is a **pure** `has_api_key_with(provider, configured_key, read_env)` taking an
      environment reader, so its 8 tests never mutate process-global env (unsound under a parallel test
      runner): per-provider variable mapping, ollama-needs-none, unknown→anthropic, the selected
      provider's variable satisfying the check, **another** provider's variable *not* satisfying it,
      explicit key beating the environment, blank/whitespace rejection, and missing→unconfigured.
      8/8 green, clippy clean (0 warnings on the touched file).
- [x] Tauri CSP is set to null in gui/src-tauri/tauri.conf.json — not acceptable for a desktop app rendering model output and markdown.
      *(2026-07-24)* **Restrictive CSP set.** `gui/src-tauri/tauri.conf.json` `app.security.csp` is
      no longer `null`. Default-src `'self'`; connect-src allows the local daemon (loopback http/ws)
      plus https for cloud providers & channel webhooks; img/media allow `data:`/`blob:`; object-src
      and frame-src are `'none'`. Model output and markdown can no longer pull arbitrary script.
- [x] Tauri Devtools enabled by default in production features (gui/src-tauri/Cargo.toml) — should be removed from default features.
      *(2026-07-24)* **Removed from `default`.** `default = ["custom-protocol", "devtools"]` meant every
      release build shipped the webview inspector on an app that renders model output and untrusted
      markdown. Nothing in `src-tauri/src` references devtools — it was purely the feature flag — and
      Tauri turns devtools on automatically under `debug_assertions`, so **`tauri dev` is unaffected**;
      the feature is kept for a deliberate `--features devtools` release build. Verified by a real
      release rebuild of `nanna-gui` (exit 0, 6m32s).
- [x] Tauri shell permissions (allow-open/spawn/kill/execute) for the daemon sidecar need least-privilege review.
      *(2026-07-24)* **Least-privilege shell surface.** `gui/src-tauri/capabilities/default.json`
      scopes `shell:allow-execute` / `allow-spawn` / `allow-kill` to the `nanna-daemon` sidecar only
      (`sidecar: true`). Broad unrestricted shell spawn is gone; `shell:allow-open` remains for
      opening user-facing URLs/files.
- [~] ROADMAP explicitly lists open items: ~~disabled tools still execute~~ **(done 2026-07-20 — `ToolPolicy` gate, P6)**, ~~deleted tools remain callable until restart~~ **(done 2026-07-17 — `unregister` wiring)**, ~~delete_skill needs hardening against remove_dir_all/symlink races~~ **(done — symlink + canonical-escape guards in `commands/tools.rs`)**, stronger sandboxing needed *(open — OS-level sandbox under the policy layer; see research note below)*.
- [x] HTTP server defaults to 0.0.0.0:3000 (src/main.rs) — potential footgun if exposed without auth.
      *(2026-07-23)* Fixed together with the webhook receiver — see the "Bind local services to localhost
      by default" item below.
- [x] Port inconsistencies: README says daemon IPC is 5149, but src/main.rs daemon start defaults to 9999, and daemon status checks 5149. Must be unified and documented.
      *(2026-07-24)* **Unified on 5149 behind one exported constant.** This was not cosmetic: `nanna
      daemon start` bound **9999** while `nanna daemon status` / `stop` connected to a hardcoded
      `ws://127.0.0.1:5149` and the GUI sidecar used 5149 too — so a CLI-started daemon **reported
      itself as not running**, and the GUI could not see it either. New
      `nanna_daemon::DEFAULT_IPC_PORT` (5149) is the single definition, exported from the crate and
      used by `IpcServerConfig::default()`, all three root-CLI `--port` defaults (`--port` global,
      `daemon start`, `daemon restart`), the `nanna-daemon` binary's own `--port`, and the address
      `daemon status` probes — which is now *built* from the constant instead of being a literal, so
      status can never probe a port different from the one start binds. The matching `--host` defaults
      switched from literal `"127.0.0.1"` to the existing `nanna_config::bind::LOOPBACK_HOST`, and
      `--health-port` to the already-exported `DEFAULT_HEALTH_PORT`.
      **Verified against the real binaries, not just the source:** `nanna daemon start --help` now
      prints `--port <PORT> ... [default: 5149]` (was 9999) and `nanna-daemon --help` agrees. README's
      documented `5148`/`5149` was already correct and needed no change. 75 daemon tests + 8 bin tests
      green, clippy clean.
- [x] Current usage can transmit user data to: cloud LLM providers, OpenAI embeddings (if OPENAI_API_KEY set), Brave Search, channel platforms (Telegram/Discord/Slack/Signal/WhatsApp), and websites fetched by tools/browser. A PRIVACY.md documenting data flows, opt-out options, and data deletion procedures is mandatory.
      *(2026-07-24)* **`PRIVACY.md` shipped** at the repo root. Documents local storage (config,
      keyring/AES-GCM credentials, Turso sessions/memory/tasks, logs), every outbound sink (cloud
      LLM providers, OpenAI embeddings, Brave Search, five channels, browser/web-fetch tools), how
      to run fully offline, how to pause/delete memory, and how to wipe data on uninstall.
- [x] Auto-remembering user messages and assistant replies into long-term memory should be opt-in with clear onboarding language and a pause/delete memory UI.
      *(2026-07-24)* **Opt-in, default off.** New `[memory] auto_remember_messages` (bool, default
      `false` — including when the key is absent from an old config). The chat control plane
      (`nanna-daemon/src/control/chat.rs`) gates both the user-turn and assistant-turn remember
      paths on it. Pause = flip the toggle; delete = existing Settings → Data "Delete All Memories".
      Clear onboarding/privacy language lives in `PRIVACY.md`.
- [x] No SECURITY.md or vulnerability disclosure process.
      *(2026-07-24)* **`SECURITY.md` shipped** — supported versions, private disclosure via
      security@nanna.bot / GitHub private advisory, response targets, and scope.
- [~] No Dependabot / cargo-audit / npm audit automation.
      *(2026-07-24)* **Dependabot on.** `.github/dependabot.yml` covers cargo (workspace root,
      weekly, holds the intentional `turso`/`aegis` pins) and npm (`/gui`, weekly, ignores the
      documented deferred majors: tiptap/vue-router/vue-sonner/marked/typescript). cargo-audit /
      npm-audit CI steps remain open under P0.3.
- [ ] No GitHub secret scanning enabled.
      *(2026-07-24)* Dependabot shipped (see above). Secret scanning itself is still a repo-admin
      toggle on GitHub and is not something a PR can flip — left open.
- [x] Store all secrets in OS keychain by default; remove secret fields from config.toml.
      *(2026-07-24)* **Done** as part of the onboarding/GUI keyring work above. `Config::save_to`
      always strips `llm.{api_key,openai_api_key,openrouter_api_key,github_token,ollama_api_key,
      anthropic_oauth_token}` and `tools.brave_api_key` before serializing. `load`/`load_from`
      re-hydrate from the keyring (env still wins as an override for CI/headless). Secret fields
      remain on the in-memory struct for routing, but they are never the on-disk source of truth.
- [x] Encrypt the SecureStore file fallback with AES-GCM (OS-protected key) or remove fallback; correct the misleading "encrypted" comment.
      *(2026-07-24)* **AES-GCM file fallback shipped** — see the SecureStore item above. The
      keyring-4 `CredentialStore` pluggable-register research note is still the cleaner long-term
      seam (implement `CredentialStore` + `keyring::set_default_store` instead of the ad-hoc
      fallback); tracked as a follow-up under P1, not blocking.
      - [ ] *(research 2026-07-09)* `keyring 4` split into a `keyring-core` layer
            exposing a pluggable `CredentialStore`/`CredentialBuilder` trait registrable via
            `keyring::set_default_store(..)`. Clean seam for replacing the ad-hoc file fallback
            with a registered encrypted-file store when no OS keyring is present.
            Source: [keyring-core docs](https://docs.rs/keyring-core).
- [x] Set a restrictive Tauri CSP (not null).
      *(2026-07-24)* Done — see the CSP item above.
- [x] Disable devtools in production default features in gui/src-tauri/Cargo.toml.
      *(2026-08-23 — verified already shipped; the checkbox was stale.)* `gui/src-tauri/Cargo.toml`
      has `default = ["custom-protocol"]` and `devtools = ["tauri/devtools"]` deliberately outside
      it, with a comment recording why: the feature opens the webview inspector in *release* builds of
      an app that renders model output and untrusted markdown. Tauri enables devtools automatically
      under `debug_assertions`, so `tauri dev` is unaffected, and a release build gets the inspector
      only by asking for `--features devtools`. No other reference to the feature exists in the crate.
- [x] **Chat markdown was rendered unsanitized into `v-html` — the CSP was the only layer.**
      *(2026-08-24)* `MarkdownContent.vue` ran `marked.parse()` straight into `v-html`, and `marked`
      has not sanitized since v5 (the `sanitize` option was removed). Verified against the installed
      `marked@17.0.6`, not assumed:
      `<img src=x onerror="alert(1)">` → `<img src=x onerror="alert(1)">`,
      `<script>alert(1)</script>` → passed through verbatim,
      `[click](javascript:alert(1))` → `<a href="javascript:alert(1)">`,
      `![x](javascript:…)` → `<img src="javascript:…">`.
      Everything that component renders is untrusted — assistant output, user input, and (via the
      assistant quoting a tool result) any web page or file the agent touched. `innerHTML` will not run
      a `<script>` tag, but it **will** fire an `<img onerror>`, so the only thing standing between
      injected markup and script execution in a webview that holds `__TAURI_INTERNALS__.invoke` was the
      Tauri CSP's `script-src 'self'` — one header, on one config file, with no second layer. A single
      future `'unsafe-inline'` (a chart or highlight library is the usual reason) would have turned this
      into a live local-command-execution path.
      **Fix:** rendering moved out of the SFC into a pure `gui/app/lib/markdown.ts` that (1) escapes
      author HTML instead of emitting it — the block *and* inline `html` renderers return escaped text,
      so every tag in the output is one marked itself generated — and (2) scheme-checks link and image
      URLs against an allowlist (`http:`/`https:`/`mailto:`; images narrow to `http:`/`https:`), keeping
      the visible label but dropping the anchor when a URL is refused. An allowlist rather than a
      `javascript:` denylist because the spellings are open-ended; the check parses with `URL` rather
      than matching a prefix, which is what catches `java\tscript:` — the platform parser strips the
      embedded control character before reading the scheme, a hand-rolled matcher does not. Built on a
      private `new Marked({…})` instance, not the shared default export, so no unrelated
      `marked.setOptions`/`marked.use` can silently replace the renderer a security property depends on.
      26 tests, asserted against **what the browser actually builds** (`innerHTML` into a detached node,
      then `querySelector`) rather than against substrings — including a sweep proving no element in the
      output carries any `on*` attribute. The other 13 pin ordinary rendering (emphasis, headings, lists,
      GFM tables/strikethrough/task lists, blockquotes, `breaks: true` soft line breaks) so the deferred
      `marked 17 → 18` bump has to state what it changes. 185/185 vitest green.
      **Intentional behaviour change:** a message containing `<div>` now shows the literal characters
      instead of laying out a div. That is what a chat client should do with markup it did not author.
      Frontend verified beyond the unit suite: `pnpm build` green (nitro + client, 4 routes prerendered)
      and a non-CI `pnpm dev` boot serving a real **200** `__nuxt` shell on `:3000` with no errors in the
      dev log — the check that catches a Nuxt boot-loop a built bundle would hide.
      **Not done this run:** a `cargo tauri build` + WebDriver pass against the packaged app. The change
      is a pure function whose 26 tests already assert against a real DOM, so the marginal value was low
      next to a full release build of the whole workspace on a contended shared target dir. Worth doing
      on a run that is building the GUI anyway.
- [~] Per-tool toggles visible in GUI; audit log for every tool call.
      *(2026-08-24)* **The audit half shipped.** New `nanna-tools::audit` records **one structured JSON
      line per tool call** at the single chokepoint every caller funnels through — `ToolRegistry::execute`.
      Two things were wrong before, and both are the reason this could not just be "add a log line":
      **(1) The only per-call record was a `debug!` pair**, off at the default level and unstructured;
      the aggregate counters that do exist (`nanna-agent::ToolStatsTracker`) are recorded from
      `loop_runner`, so every call made outside the agent loop — chat harness, task tool, scheduled runs,
      the `nanna mcp serve` bridge, scripted skills — left **no trace at all**. An audit that saw one
      caller would be worse than none, because it would read as a complete account.
      **(2) Three of `execute`'s four exits returned early.** A record appended at the end would have
      missed *exactly* the events an audit exists for: a policy refusal and a not-found name. `execute`
      is now a thin wrapper that times the call, delegates to `execute_call` (which returns
      `(response, outcome, resolved)`), and records **once**, on every path. The invariant is enforced by
      the shape of the code, not by remembering to log at four sites, and a test drives all four outcome
      classes plus `execute_parallel` and asserts one record each.
      **Values stay out by default.** Arguments carry secrets (an API key in a request, the body of a
      file being written) and the trail is durable plaintext that outlives the run, so the record carries
      the argument **key names** (sorted, deduped, bounded) and never the values unless the sink asks —
      `[tools] audit_log_values`, off by default. The include-values decision sits on the `ToolAuditSink`
      trait rather than in the registry, because only the sink knows where the trail lands; it defaults
      to `false` so a sink written later inherits the value-free posture.
      **A real bug fell out of writing the test.** `resolve_tool` returns the *registry key*, which for
      an alias is the alias itself (`Bash`), not the tool it points at — `refuse_by_policy` was
      canonicalizing privately, so nothing else in `execute` had the canonical name. The audit now
      canonicalizes once, up front, and both the policy gate and the trail speak that one identity;
      without it a denied aliased call would have been recorded as a decision about a tool named `Bash`.
      **Bounds** (all Tiger-Style derived, none magic): ≤64 key names × ≤64 bytes each (a tool's
      parameter list is its signature — the widest in-tree declares well under twenty, but the *map* is
      caller-controlled), ≤512-byte value/error previews (a path, a URL, or the first clause of an error
      — the identifying part of either), 8 MB file cap with one generation of rollover, so the trail
      costs at most 16 MB on disk with no background reaper. Every truncation is char-boundary safe.
      Wired end to end: `[tools] audit_log` (**on by default** — an unattended daemon with no answer to
      "what did it do overnight" is the gap) → `DaemonConfig` → `{data_dir}/logs/tool-audit.jsonl`, plus
      the legacy `nanna serve` path. Ships `JsonlAuditSink` (size-capped, rollover, mutex-gated so
      concurrent calls cannot interleave a rename with a write) and `TracingAuditSink` for operators who
      already ship the daemon log elsewhere. 11 audit-module + 7 registry tests. Documented in
      `PRIVACY.md`'s local-storage table and README's feature list.
      **Verified against the real binary, not just unit tests.** Booted the freshly built
      `nanna-daemon` on an isolated `--data-dir` and drove four `tool.execute` calls over the live IPC
      socket, one per outcome class. The trail it wrote:
      ```
      {"requested":"list_dir","resolved":"list_dir","param_keys":["path"],"duration_ms":48,"outcome":"succeeded"}
      {"requested":"zzzz_no_such_tool_at_all","resolved":null,"param_keys":[],"duration_ms":0,"outcome":"not_found"}
      {"requested":"Bash","resolved":"exec","param_keys":["command"],"duration_ms":62,"outcome":"succeeded"}
      {"requested":"read_file","resolved":"read_file","param_keys":["file_path"],"outcome":"failed","error":"read_file: '…' does not exist …"}
      ```
      Four calls, four lines. Note row 3: `requested: "Bash"` → `resolved: "exec"` — the
      alias-canonicalization fix, proven live rather than argued. And note what is *absent*: the `exec`
      call carried `command: "echo hi"` and the `list_dir` call carried `path: "."`, and neither value
      appears anywhere in the trail — only the key names, which is the default posture working.
      **Still open on this line:** the GUI per-tool toggles, and an audit *viewer* in the GUI.
- [x] Fix tool lifecycle bugs: disabled tools must not execute; deleted tools must not remain callable until restart (ROADMAP P6/P11).
      *(2026-07-20)* Disabled-tools-execute closed by the `ToolPolicy` gate above (`[tools] disabled` now
      denies at `execute()`, post-resolution). Deleted-tools-callable was closed 2026-07-17 via
      `ToolRegistry::unregister` wiring (see the P11 tool-manager-consistency note).
- [x] Harden delete_skill against remove_dir_all/symlink races.
      *(2026-07-14 / confirmed 2026-07-24)* Done — symlink + canonical-escape guards in
      `commands/tools.rs` before `remove_dir_all`.
- [x] Bind local services (health/webhook) to localhost by default; require explicit opt-in for public exposure.
      *(2026-07-23)* **Done.** Audit found three surfaces defaulting to `0.0.0.0` — the webhook receiver
      (`WebhookConfig::default`), the legacy HTTP server (`ServerConfig::default`), and the `nanna server
      --host` flag — i.e. an unauthenticated, LAN-visible control surface on any machine that joins a café
      or hotel network. All three now default to `127.0.0.1`. (The **health server and IPC were already
      loopback** — health inherits `ipc.host`, which defaults to `127.0.0.1` — so no change was needed
      there; verified rather than assumed.)
      Exposure is now an explicit act: set `host` yourself. New `nanna_config::bind` provides the single
      `LOOPBACK_HOST` constant plus a pure `is_loopback_host(host)`, and **both servers log a `warn!` on a
      non-loopback bind** so publishing is always visible in the log, not just in a config file someone
      edited months ago. The predicate recognises the whole `127/8` block, `::1` bare **and bracketed**,
      and case-insensitive `localhost`; anything unparseable or unfamiliar **fails safe to "public"** —
      being wrong in the direction of an extra warning is the only acceptable direction here.
      4 tests (the default constant satisfies its own predicate — so a stock install never warns about
      itself; loopback spellings incl. `127.0.0.2`/`[::1]`/whitespace; wildcards `0.0.0.0` and `::` plus
      routable addresses read as public; unparseable input fails safe).
      **Note for tunnel users:** this is not a regression — cloudflared/ngrok/reverse proxies connect *to*
      loopback. Only a setup relying on the old `0.0.0.0` default for direct inbound webhooks needs to set
      `host` explicitly now, which is exactly the opt-in this item asked for.
- [ ] Add authentication for any non-local control plane.
- [ ] Verify webhook signature validation across all channels (Telegram secret, WhatsApp verification, Signal bridge trust, replay protection).
      - [x] *(2026-08-22)* **The whole inbound webhook surface now fails CLOSED, and the generic route
            no longer aborts the daemon.** Every verifier in `nanna-daemon/src/webhook.rs` was correct and
            every handler *skipped* it when nothing was configured — `if let Some(secret) = …` with no
            `else` — so the **unconfigured case was the least protected one**, and each of those endpoints
            hands its payload to the agent loop, which runs tools at the daemon's privilege. Telegram was
            the worst: `apply_channel_webhook_secrets` never populated `telegram_secret` at all (its
            comment claimed "Telegram authenticates via the bot token in the URL" — the route is the fixed
            path `/webhook/telegram` with no token in it), so that endpoint had never verified anything.
            Now: telegram/discord/slack/whatsapp-POST/generic each refuse with **503** + a log line naming
            the one config key to set (503, not 401, so "never armed" reads differently from "wrong
            proof"); `channels.telegram.webhook_secret` and `channels.signal.webhook_secret` are new config
            fields (`TELEGRAM_WEBHOOK_SECRET` env override) and Telegram's `setWebhook` secret token is
            wired through; a blank `Some("")` credential counts as **unconfigured** (comparing an empty
            secret to an absent header is `"" == ""`, which would have authenticated everyone); the generic
            hook's `==` secret compare became constant-time with both header forms evaluated unconditionally
            (a short-circuiting `||` leaks which header was right); and **Discord gained the replay window
            Slack already had** — Discord signs a timestamp but publishes no tolerance, so a captured POST
            verified forever (test: a day-old capture still passes Ed25519 and is refused only by the
            window). Separately and worse: `.route("/webhook/:id", …)` is **axum 0.7 syntax that axum 0.8
            PANICS on** at router construction — inside a spawned task, under `panic = "abort"` — so
            enabling any webhook server aborted the daemon at startup. Fixed to `{id}` with a
            router-construction regression test (verified: reverting the route makes the test panic).
            Evidence: 3 new end-to-end tests drive the real `WebhookServer` over a real socket with
            `reqwest` (unconfigured→503 on all five routes, configured→401 without/with a wrong proof and
            200 with it, blank→503); verified non-vacuous by re-introducing the fail-open and watching
            `every_unconfigured_channel_refuses_with_503` report `got 200`. 6 new unit tests. The `run()`
            log line that already promised "Only inbound requests carrying a valid provider signature are
            accepted" is now true.
            **Operator-visible change:** a channel that was relying on an unauthenticated webhook stops
            serving until its secret is set; the 503 log names the key.
      - [x] *(2026-08-22)* **Same treatment for the `nanna serve` copy (`nanna-server`), which was worse.**
            Telegram, Signal and the generic hook had **no authentication of any kind** — no header read,
            no secret compared — and `nanna serve` also never handed the Slack signing secret to
            `AppStateBuilder`, so its Slack verifier could not have run even in principle. The generic
            endpoint (which takes an arbitrary `message` and runs it) had a `webhook_secret` field parsed
            into `AppState` from `server.webhook_secret` and read by **nobody**. New
            `webhooks/auth.rs` holds the shared primitives (`configured`, constant-time `secret_matches`,
            `timestamp_is_fresh`, `refuse_unconfigured`, `bearer_secret_ok`), all five handlers fail closed
            through it, Discord gained the replay window, and `serve.rs` now wires
            slack/telegram/signal secrets. 6 unit tests. `nanna init` mints a 122-bit Telegram webhook
            secret and prints the exact `setWebhook` call, so a fresh install is armed rather than broken.
            - [x] *(2026-08-23)* **`nanna-server`'s handlers now have the end-to-end harness the daemon's
                  had.** `crates/nanna-server/tests/webhook_fail_closed.rs` drives the real
                  `create_router()` through `tower::ServiceExt::oneshot` — real axum routing, real
                  extractors, real handler bodies — over a real `AppState`. The `AppState` turned out
                  to need no test double at all: `Storage::in_memory()` opens a Turso store,
                  `Nanna::new` touches no network, and `NannaConfig { enable_gpu: false }` plus
                  `.dreaming(false).scheduler(false)` keeps the fixture to the path under test. **6
                  tests, no network and no API key**, because every payload chosen for an "admitted"
                  leg reaches a handler branch that returns *before* any agent work — Discord
                  `PING`→`PONG`, Slack `url_verification`→challenge echo, a Telegram update with no
                  message, a Signal envelope with no `dataMessage` — so passing authentication is all
                  that passing authentication proves.
                  Beyond the daemon suite's three properties (unconfigured→503, configured→401
                  without/with a wrong proof, blank→503) this copy **signs its own fixtures**, which
                  the daemon suite cannot: a genuinely HMAC-SHA256-signed Slack challenge is answered
                  200 while a signature valid for a *different body* is refused, and a genuinely
                  Ed25519-signed Discord `PING` is answered 200 while **a day-old capture of it is
                  refused** — the one assertion that would fail if the replay window were deleted and
                  the Ed25519 check kept, since Discord's signature never expires on its own. Also
                  pinned: a *prefix* of a shared secret is refused (the constant-time compare exists
                  precisely so a byte-by-byte one cannot be walked), and `Authorization: <secret>`
                  without the `Bearer ` scheme is not the secret.
                  **Verified non-vacuous**, the same way the daemon suite was: re-introducing the
                  original fail-open in the Telegram handler (`else { return Err(StatusCode::OK) }`)
                  makes the suite report `/webhooks/telegram must refuse while unconfigured, got 200
                  OK` from two tests; reverted clean. 22 `nanna-server` tests green, clippy 0 errors
                  and 0 warnings from the new file. One dev-dependency added: `tower` with `util`.
            - [x] *(2026-08-23)* **Authentication now runs *before* body deserialization on all five
                  routes.** Found while building the harness above, then fixed. `telegram`, `signal`
                  and `generic` took `Json<T>`, and axum runs extractors before the handler body — so
                  an unauthenticated caller drove `serde_json` on those routes, and an
                  **unconfigured** channel handed a malformed body answered the parser's **400
                  instead of the 503** whose entire purpose is to tell an operator "this host never
                  armed this channel". Measured, not assumed: a probe against the unconfigured router
                  returned `telegram 400 · discord 503 · slack 503 · signal 400 · generic 400` — the
                  split falls exactly on which extractor each handler used, because `slack` and
                  `discord` already took `Bytes` and parsed after verifying. So did **every** handler
                  in `nanna-daemon`, which is why only this copy was wrong.
                  Fixed the way the two correct handlers already worked rather than by adding a
                  middleware layer and its state plumbing: the three routes take `Bytes` and call the
                  new shared `auth::parse_authenticated_body`, which parses after the credential
                  branch and maps a failure to **400 — honest at that point, because by then the
                  caller has proved who it is, so a bad body is a bad request and not an anonymous
                  one**. Keeping the helper in `auth.rs` beside `configured`/`secret_matches` means
                  the ordering rule is stated once, next to the rule it protects.
                  1 new unit test (valid parses; malformed and wrong-shape are 400 and specifically
                  *not* 401/503) and 1 new end-to-end test pinning all three states per route:
                  unconfigured + malformed → 503, armed + wrong proof + malformed → 401, armed +
                  right proof + malformed → 400. **Verified non-vacuous**: reverting `generic` to
                  `Json<T>` makes it report "is unconfigured, so it must refuse before parsing, got
                  400 Bad Request". 24 `nanna-server` tests green, clippy 0 errors.
                  Not a fail-open — every route refused before and refuses now — so this was a
                  correctness and operator-legibility fix, not a security one.
                  *(Diff hygiene: `rustfmt` on `signal.rs`/`telegram.rs` re-indented ~40 unrelated
                  pre-existing `#[serde(rename)]` attributes, burying a 4-line change. Reverted and
                  the edits re-applied by hand — the same trap the 2026-07-25 sweep logged.)*
      - [x] *(2026-07-25)* **Slack HMAC verification in `nanna-server` (the `nanna serve` path) hardened to
            match the daemon's.** The daemon copy (`nanna-daemon/src/webhook.rs`) was already correct
            (raw-body HMAC + `verify_slice` constant-time + replay guard), but the `nanna-server` copy had
            **drifted** in two ways: it hashed `std::str::from_utf8(body).unwrap_or("")` — so a **non-UTF-8
            body was silently HMAC'd as empty**, letting a mangled payload verify against a signature
            computed over nothing — and it used a **hand-rolled hex-string** comparison instead of the MAC
            primitive. Rewrote it to `mac.update(body)` (raw bytes) + `mac.verify_slice(&hex::decode(v0…))`,
            matching the daemon. Added the first tests for this function (6): valid accepts, tampered body
            rejects, stale timestamp rejects, wrong secret rejects, **non-UTF-8 body verifies correctly**
            (the regression guard), and missing-`v0=`/empty-input rejects. nanna-server green.
      - [x] *(2026-07-25)* **WhatsApp POST webhook was UNAUTHENTICATED — now verifies the Meta
            `X-Hub-Signature-256` (daemon).** `whatsapp_webhook` parsed and acted on any POST body with **no
            signature check at all**, so anyone who learned the `/webhook/whatsapp` URL could inject fake
            WhatsApp events. Added a tested pure `verify_meta_signature(app_secret, header, body)`
            (HMAC-SHA256 over the **raw** body, `sha256=<hex>` prefix, constant-time `verify_slice`) and a
            `whatsapp_app_secret: Option<String>` field on `WebhookConfig`; the handler now rejects a bad
            signature with 401 when the secret is configured (skips with a warning when unset — same posture
            as the other providers, and `None` by default so existing behaviour is unchanged). Also switched
            the GET subscription handshake's verify-token check from `==` to the constant-time
            `webhook_secret_matches`. 4 tests (valid accepts; tamper/wrong-secret/no-prefix/missing/empty
            reject; **non-UTF-8 body verifies** — WhatsApp media notifications aren't guaranteed UTF-8).
            *(Config plumbing — closed the same day, see below.)*
      - [x] *(2026-07-25)* **Webhook signature verification was entirely DORMANT — the daemon never received
            the secrets. Now wired.** `DaemonBuilder::with_webhook_config` was never called and
            `DaemonConfig.webhook` was only ever `WebhookConfig::default()` (all-None), so **every** provider's
            verification silently no-op'd (the handlers logged "not configured - skipping") no matter what the
            user set — the whole webhook-auth surface (mine + the pre-existing Discord/Slack verifiers) was
            unreachable. `from_nanna_config` now calls a pure, tested `apply_channel_webhook_secrets(webhook,
            channels)` that copies `channels.discord.public_key` → `discord_public_key`,
            `channels.slack.signing_secret` → `slack_signing_secret`, and `channels.whatsapp.{verify_token,
            app_secret}` → the WhatsApp fields. Added `app_secret` to `WhatsAppConfig` (serde-default, so old
            configs deserialize). Telegram is intentionally excluded — its `TelegramConfig` has no webhook
            secret (bot-token-in-URL auth), only the bot token (wired for outbound). 2 tests (all configured
            providers flow through; absent channels leave secrets unset). This is what makes Discord/Slack/
            WhatsApp webhook verification actually *enforce* for a configured user. *(Still open: the GUI/
            embedded and legacy `serve.rs` paths build `WebhookConfig::default()` directly and don't call the
            mapper; and Telegram/generic secrets still have no config home.)*
      - [x] *(2026-07-25)* **Telegram webhook secret now compared constant-time (daemon).** The daemon
            checked the `X-Telegram-Bot-Api-Secret-Token` with a plain `!=`, which short-circuits on the
            first wrong byte and leaks match progress through response timing — inconsistent with the
            HMAC/Ed25519 verifiers next to it. Extracted a testable `webhook_secret_matches(expected,
            provided)` using `subtle::ConstantTimeEq` (subtle was already in the tree transitively; now a
            direct dep). 1 test (exact match accepts; wrong-byte / prefix / superstring / missing / empty
            reject). Low practical risk on a static high-entropy secret, but it keeps the whole webhook
            surface constant-time.
      - [x] *(2026-07-25)* **`nanna-server` Discord Ed25519 verify aligned to the daemon's `verify_strict`.**
            The server copy used `verifying_key.verify(..)`; the daemon uses `verify_strict`, which rejects
            **malleable / non-canonical signatures and small-order keys**. Switched to `verify_strict` (and
            dropped the now-unused `Verifier` trait import) so the two verifiers can't drift on strictness,
            and added the first tests for it (4): valid accepts, tampered body rejects, wrong public key
            rejects, malformed-hex + all-zero-signature reject. *(Still open on this line: the Telegram
            secret / WhatsApp / Signal paths, and **folding the duplicated Slack/Discord verifiers** into one
            shared implementation so daemon and server can't drift again — this run fixed the drift twice,
            which is the argument for de-duplicating.)*
- [x] Unify ProjectDirs namespaces — config and credentials must use the same ("com", "nanna", "nanna") (or equivalent) namespace.
      *(2026-07-24)* Done — see the namespace-unification item above.
- [ ] Run gitleaks detect --source . and trufflehog git file://. across full git history.
- [x] Remove or gitignore .claude/settings.local.json (committed with machine paths and broad agent permissions).
      *(2026-07-24)* **Untracked and gitignored.** It was committed in a **public** repo carrying 77 lines
      of Claude Code permission allowances — including `Bash(curl:*)`, `Bash(taskkill:*)`,
      `Bash(del /F …)` and several `Get-Process … | Stop-Process -Force` one-liners — plus absolute
      machine paths still pointing at the repo's **old** name (`D:\Development\clawdbot-rs`). By
      convention `settings.local.json` *is* the personal override (`settings.json` is the shared one, and
      this repo has none), so anyone cloning inherited a pre-approved allowlist for network fetches and
      process kills. Removed from the index with `git rm --cached` — the file stays on disk — and added
      to `.gitignore`.
      ⚠️ **Before merging: keep a copy of your local `.claude/settings.local.json`.** Untracking is a
      deletion as far as git is concerned, so merging this removes the file from any working tree that
      has it. Restore it afterwards (it is ignored now, so it will stay put from then on).
- [x] Add SECURITY.md with vulnerability disclosure process.
      *(2026-07-24)* Done — `SECURITY.md` at repo root.
- [~] Enable GitHub secret scanning and Dependabot.
      *(2026-07-24)* **Dependabot half done** (`.github/dependabot.yml`). Secret scanning is a
      GitHub repository setting (Settings → Code security) and must be flipped by a repo admin —
      left open until confirmed on `physics515/Nanna`.
- [ ] Claude UI Testing automations
- [ ] Implement Mummu model runner to replace the built in

### P2 — Tools & Channels ✅
File/shell/web tools, memory tools (remember/recall/reflect), scheduling, browser tools, vision
(analyze_image), tiered OCR, audio (TTS/transcription), PDF (text + image extraction). All tools
migrated to filesystem JS/TS skills (Boa + Deno). All five channels (Telegram/Discord/Slack/Signal/WhatsApp)
with send/react/edit/delete/pin/threads/media where supported. **Shipped.**

### P3 — Multi-Agent & MCP ✅ (one caveat)
MCP client (stdio + HTTP/SSE transports, tool discovery, adapter into nanna-tools), background task
spawning, agent-to-agent messaging (mailbox), Erlang/OTP-style supervisors (RestartPolicy, strategies,
health checks). **Shipped**, except:
- [~] **Verify or build MCP *server* mode** — doc claims `crates/nanna-server/src/mcp.rs`; that file does
      not exist and no MCP refs found under `nanna-server/src`.
      *(2026-07-23)* **Located: the server lives at `crates/nanna-mcp/src/server.rs`** (532 lines —
      `McpServer` with tool/resource/prompt registration, `handle_request` covering initialize/tools/
      resources/prompts/ping, stdio transport, and a `ToolRegistry` bridge that exposes every Nanna tool).
      The doc pointer was simply wrong, not the feature. But nothing *started* it — no daemon or CLI entry
      point — so it was reachable only from Rust.
      *(2026-07-23)* **Wired up: `nanna mcp serve` now exposes Nanna's tool surface over stdio JSON-RPC**,
      the transport every MCP client speaks. It loads the filesystem JS/TS skills (`--tools-dir`, else
      `[tools] tools_dir` / `NANNA_TOOLS_DIR` / the dev tree), applies the user's `[tools]`
      enabled/disabled policy, and serves `McpServer::run_stdio`. The registry bridge
      `_register_tools_from_registry` was dead code behind its underscore; it is now
      `register_tools_from_registry` and actually called.
      **stdout is the protocol** — a stdout-writing log layer corrupts the JSON-RPC stream and the client
      drops the connection, so `main` installs a **stderr** writer for this command (and only this one, so
      every other command keeps its console behaviour); the startup banner follows the same writer.
      **Policy is enforced on both sides:** `definitions()` filters denied tools out of the advertised
      list, and `execute()` re-checks *after* alias/fuzzy resolution — so a disabled tool is neither
      offered nor invocable by a guessed name. To guarantee the CLI and the daemon read `[tools]`
      identically, the daemon's private `build_tool_policy` moved into `nanna-tools` as
      `ToolPolicy::from_config_lists(enabled, disabled)` (a second copy is a security bug waiting to
      happen); the daemon fn is now a thin wrapper and its tests still pin the behaviour.
      5 new policy tests (`enabled=["*"]` and empty/absent mean unrestricted; a real allowlist restricts;
      deny beats allow when a name is on both lists; `disabled` applies under a wildcard).
      **Verified against the real binary:** piping JSON-RPC into `nanna mcp serve` returned a valid
      `initialize` result, a `tools/list` advertising all **39** skills (every one carrying an
      `inputSchema`), and a `tools/call` of `list_dir` that really executed and returned directory
      contents — with **stdout containing exactly the 2/2 protocol lines and every log on stderr**.
      Remaining: memory/agent-backed tools (`remember`/`recall`/`reflect`/`task`) need the daemon's script
      services, which this standalone path does not build — see the new item below.
- [ ] *(2026-07-23)* **Give `nanna mcp serve` the memory/agent-backed tools.** It loads skills via
      `ToolRegistry::load_skills` (no services), so the tools that need `build_script_services` —
      `remember`, `recall`, `reflect`, `task` — load but cannot reach memory or spawn sub-agents. Options:
      (a) build the script services in the CLI path (needs storage + an embedding provider), or
      (b) add a daemon IPC action so `mcp serve` proxies to the running daemon and inherits its live
      store — (b) matches the "channels as control-plane clients" architecture and avoids a second
      process owning `nanna.db`. Until then, document the standalone surface as filesystem/shell/web only.
- [~] Supervisor health check runs a placeholder, not a real agent loop (`supervisor.rs:496`).
      *(2026-08-23)* **Half of this was already stale, and the half that was true hid a real bug.**
      `perform_health_check` does run a genuine agent loop — `Agent::run(probe_prompt)` under a
      `timeout`, folded into the `apply_health_result` state machine — so "the health check is a
      placeholder" has not been accurate for some time. What *is* still a placeholder is the
      supervised agent's own body (`// TODO: Run actual agent loop here using _llm and _tools`);
      `start_agent` spawns a task that only awaits its shutdown signal.
      **The bug the stale label was hiding: the probe's pass condition was
      `response.to_lowercase().contains("ok")`** — a substring test on two of the commonest letters
      in English. Measured against real answers: **"I am broken", "not ok", "Out of tokens" and
      "Something looks wrong" all PASSED**, while "Healthy", "operational" and "alive" all *failed*.
      So it was simultaneously too loose and too tight, and — worse — an agent explicitly reporting
      that it was broken was recorded as healthy, which means the `failure_threshold` /
      `BecameUnhealthy` / restart machinery underneath it could never fire. A liveness probe that
      cannot fail is worse than no probe: it manufactures confidence.
      Replaced with a pure, testable `probe_answer_is_affirmative` beside the existing pure
      `apply_health_result`: an affirmative token (`ok`/`okay`/`healthy`/`operational`/`alive`)
      must appear at a **word boundary** (which is what stops `broken`/`tokens`/`looks`), **and** no
      negation may appear anywhere (which is what stops "not ok" — it carries a perfectly good
      boundary-`ok`, so the boundary rule alone is not enough). Bounded at
      `MAX_PROBE_ANSWER_BYTES = 4096`, derived: ~3 orders of magnitude above the one-word answer the
      probe asks for, while bounding the work a runaway model can impose on a check that runs on a
      timer forever. Over the cap the verdict is **fail, not truncate-and-scan** — truncating would
      make the cap a way to push a negation past the end of the scan. Splitting on
      `!char::is_alphanumeric` is Unicode-aware and **never slices**, which matters here: byte-slicing
      a string with a multi-byte char in it has taken this daemon down before.
      6 tests (plain acknowledgement; the five substring traps; seven negated forms; empty/irrelevant;
      the cap in both directions; multi-byte answers incl. an em dash and emoji). **Verified
      non-vacuous**: restoring the substring check fails 4 of them. 389 `nanna-agent` lib tests green,
      clippy 0 errors.
      *(Follow-up in the same run: the function's exit `debug_assert` was itself a tautology —
      `!negated || !(affirmed && !negated)` simplifies to `true`, which clippy's `overly_complex_bool_expr`
      caught on the workspace pass. A per-crate grep by function name had missed it, because clippy
      anchors to lines, not names. Replaced with a real post-condition — the scan only ever runs on a
      bounded input, so pin that the cap's early return happened — which is what the exit assertion
      should have been saying. Worth remembering as a check-your-checks lesson: an assertion that
      cannot fail is the same failure mode as the health probe it was guarding.)*
      - [ ] **Still open: give a supervised agent a real body.** `start_agent` must run the agent loop
            rather than parking on `shutdown_rx`. Until it does, the health probe measures the *LLM's*
            reachability, not the supervised agent's — which is worth knowing, but is not what the
            name promises. Rename or re-scope the check when the body lands.
- [~] *(research 2026-07-20)* **Harden the MCP client for the 2026-07-28 spec RC.** Roots/Sampling/Logging
      are deprecated (file scoping moves to tool params / URIs / server config); tools move to full JSON
      Schema 2020-12 (`oneOf`/`anyOf`/conditionals). Two hard requirements for our client: **must not
      auto-dereference external `$ref` URIs**, and **bound schema depth + validation time** (untrusted server
      schemas are a DoS/SSRF surface). Also fold in TOFU description-pinning (see the P6 anti-rug-pull item).
      Source: [MCP 2026-07-28 release candidate](https://blog.modelcontextprotocol.io/posts/2026-07-28-release-candidate/).
      *(2026-07-21)* **Both hard requirements shipped** — new `nanna-mcp::schema_guard`. Every tool
      `input_schema` returned by a server is gated at the single ingest chokepoint (`refresh_tools`, which all
      list/refresh paths funnel through) by a pure `validate_tool_schema`: it **rejects any external `$ref`**
      (external ⇔ the ref does not start with `#`, so absolute URIs / `file://` / relative doc paths are dropped
      while intra-document fragments — root `#`, JSON-Pointer `#/…`, and 2020-12 plain-name anchors `#node` —
      pass, since none need a fetch), and **bounds both depth (≤32) and total node count (≤10 000)** so a
      deep-or-wide hostile schema can't stall later traversals. The walk is **iterative over an explicit,
      node-bounded work stack** (never recursive → can't itself overflow), and the gate **filters rather than
      failing the refresh** — one bad tool is logged+dropped, the rest of the server's toolset still loads.
      Depth/node caps are principled ceilings (a real function-call schema nests a handful of levels / tens of
      properties; the caps sit ~5×/orders-of-magnitude above that yet below serde_json's 128 parse limit). 10
      tests (safe-schema, internal-frag accept, https/file/relative/empty reject, deep-nested reject,
      at-limit accept, wide-node reject, ref-classifier, + a client integration test proving `refresh_tools`
      drops the external-ref tool and keeps the safe + internal-ref ones in both the returned Vec and the
      cache). Remaining on this item: `oneOf`/`anyOf`/conditional keyword handling in `schema_to_parameters`,
      Roots/Sampling/Logging deprecation, and TOFU description-pinning (P6 anti-rug-pull).
- [~] *(research 2026-07-21)* **Finish the 2026-07-28 RC client migration (non-security half).** Beyond the
      `$ref`/depth gate shipped above, the RC also: (1) changes the *missing-resource* error code from the
      MCP-custom **`-32002`** to the JSON-RPC-standard **`-32602` Invalid Params** — we don't match on `-32002`
      today (grep-clean), so this is forward-compat only, but any future error-code matching must use `-32602`;
      (2) lets **`structuredContent` be *any* JSON value**, not only an object — `CallToolResult`/adapter should
      stop assuming an object when structured output lands; (3) lifts input schemas to **JSON Schema 2020-12
      composition** (`oneOf`/`anyOf`/`allOf`/conditionals + `$defs`) — `schema_to_parameters` currently only
      reads a flat top-level `properties`, so a composed schema silently yields zero params. Handle composition
      (at least surface the union of branch properties). Source:
      [MCP 2026-07-28 RC](https://blog.modelcontextprotocol.io/posts/2026-07-28-release-candidate/).
      *(2026-07-21)* **Point (3) shipped** — `schema_to_parameters` is now composition-aware: it folds the
      `properties` of each `allOf`/`anyOf`/`oneOf` branch (one level deep) into the parameter list on top of the
      top-level `properties`, so a 2020-12 composed tool no longer yields **zero** params (which would make the
      model call it with no arguments). A property is required only when the root or an `allOf` branch (all must
      hold) requires it; `anyOf`/`oneOf` branch props are optional (only one branch applies). Order is root-first
      then branch order, first-definition-of-a-name-wins; bounded by the finite, already-`schema_guard`-capped
      schema. Refactored into pure helpers (`collect_schema_object`/`property_to_parameter`) and fixed the old
      buggy required-lookup in passing. 5 tests (flat props+required, allOf hard-required, anyOf/oneOf optional,
      first-wins dedup, empty/typeless→String).
      *(2026-07-23)* **Points (1) and (2) shipped — this item's RC migration is now complete.**
      **(1) Error codes:** new `protocol::error_codes` module names the codes the client recognises —
      `INVALID_PARAMS` (-32602), `LEGACY_RESOURCE_NOT_FOUND` (-32002), and the three MCP-reserved
      "modern server" codes `HEADER_MISMATCH` (-32020) / `MISSING_REQUIRED_CLIENT_CAPABILITY` (-32021) /
      `UNSUPPORTED_PROTOCOL_VERSION` (-32022) — plus a pure `const fn is_resource_missing(code)` that
      accepts **both** revisions' spellings. `read_resource` now runs its failures through
      `resource_error_for(uri, err)`, so a missing resource surfaces as the typed
      `McpError::ResourceNotFound(uri)` whether the server is pre- or post-RC, while **every other code
      passes through unchanged** (a `-32601`/transport fault must never be laundered into "not found",
      which would read as an empty resource).
      **(2) `structuredContent`:** added to `CallToolResult` as a bare `Option<serde_json::Value>` — the
      RC allows *any* JSON value, so narrowing it to a map would drop conforming results. Threaded
      through both directions: the client-side `McpToolWrapper` attaches it via `ToolResult::with_data`
      on the success path (an errored call has no result to report), `McpToolResult` gained a
      `structured` field, and the **server** side mirrors it — a registry tool's `ToolResult::data` is
      emitted as `structuredContent`. Decision pinned by test: an explicit `null` collapses to absent
      (a null payload carries no information; keeping them apart would only let an always-emitting
      server attach `data: null` to every result). 8 new tests (any-JSON round-trip incl. array/string/
      number/bool, absent-stays-absent on the wire, null-collapse, both-codes→ResourceNotFound carrying
      the URI, unrelated-code passthrough, reserved-range bounds). 33/33 `nanna-mcp` tests green, zero
      net new clippy warnings (44 lib / 42 lib-test, unchanged).
      Remaining on the RC: nested/conditional composition (`if`/`then`/`$defs`) in `schema_to_parameters`,
      and the client still advertises `PROTOCOL_VERSION = "2024-11-05"` — see the new item below.
- [ ] *(2026-07-23)* **Bump `McpClient::PROTOCOL_VERSION` off `2024-11-05`.** The client still negotiates
      the Nov-2024 revision, so a 2026-07-28 server may legitimately answer `-32022
      UnsupportedProtocolVersion` (constant now defined) or fall back to legacy behaviour. Bumping it is a
      capability commitment, not a string edit — it requires the Roots/Sampling/Logging deprecation
      handling and the stateless/multi-round-trip + routable-header semantics of the RC. Do it as its own
      increment once those land. Source:
      [MCP 2026-07-28 RC](https://blog.modelcontextprotocol.io/posts/2026-07-28-release-candidate/).
- [ ] *(research 2026-07-21)* **Approved-server registry + signed/pinned tool definitions (defense-in-depth
      for tool-poisoning, OWASP MCP03 / CVE-2025-54136).** Tool *descriptions* enter the agent context as
      trusted text, so a poisoned description is prompt-injection with supply-chain reach — worst in
      auto-approve/unattended mode (Nanna's daemon). Layer on top of the `schema_guard` + P6 TOFU-pinning items:
      treat every third-party server as untrusted by default, keep a registry of approved servers with explicit
      **version pinning** (block connect if absent), and hash-pin the description+schema at first approval,
      re-prompting on drift. Pairs with the "drop ACE grants on entering unattended mode" P6 item. Sources:
      [OWASP MCP Top 10 — Tool Poisoning](https://owasp.org/www-project-mcp-top-10/2025/MCP03-2025%E2%80%93Tool-Poisoning),
      [State of MCP Security 2026](https://pipelab.org/blog/state-of-mcp-security-2026/).
      - *(research 2026-08-24)* **The threat kept growing and the defence converged — but sequence this
        behind wiring the client, not in front of it.** Jan–Feb 2026 saw **30+ CVEs** filed against MCP
        servers, clients and tooling, and the class is now catalogued as **ASI04 in the OWASP Agentic
        Top 10**; the recommended controls are exactly the two already written above (hash-pin the tool
        definition, allowlist approved servers) plus "treat every tool-returned string as hostile input".
        **But this run confirmed `McpIntegration` is constructed nowhere in the tree and `nanna-config`
        has no `[mcp]` section** — the MCP *client* is dead code today, so hardening it further buys
        nothing until the "MCP client startup" item lands. Do that first, then pin. When pinning does
        land it must be a **filter, not an approval prompt** (owner rule: no permission gates) — drift
        drops the tool and says so, the same posture `schema_guard` already takes, with an explicit
        out-of-band re-pin rather than an in-turn dialog. Sources:
        [Speakeasy — tool poisoning threats and defenses](https://www.speakeasy.com/resources/mcp-tool-poisoning/),
        [CSA Labs research note](https://labs.cloudsecurityalliance.org/research/csa-research-note-mcp-tool-poisoning-ai-agent-exfiltration-2/),
        [Practical DevSecOps — attack chain & defense](https://www.practical-devsecops.com/mcp-tool-poisoning/).
- [ ] *(research 2026-07-20)* **HalluSquatting guard on `discover_tools`/skill-install/fetch paths** — agents
      reach for fabricated names in up to 85% of repo requests / 100% of skill installs, and attackers
      pre-register them. Make name→source resolution mandatory before any clone/install/fetch, flag those
      keywords, and never auto-run the resolved target unattended. Source:
      [HalluSquatting](https://thehackernews.com/2026/07/new-hallusquatting-attack-could-trick.html).
- [x] Supervisor recovery counts consecutive successes, not first-success (pure `apply_health_result`
      state machine + `consecutive_health_successes` stat; events emit after lock release). *(2026-07-06)*

### P4 — GUI Application ✅
Tauri 2 + Nuxt 4 + Tailwind 4 (Palenight theme). Streaming markdown chat, session management, tabbed
settings + config migration + import/export, tool-call visualization, memory browser, channel onboarding
wizards (all five), model-stats + tool-stats dashboards, system tray, native notifications,
mobile-responsive layouts. **Shipped.** Open polish: real-device mobile testing, per-tool drill-down,
latency sparklines.
- [x] **Logs page shows in-process logs, tagged by source** *(2026-07-16)* — `run()` composes a
      `LogBufferLayer` over a 5000-entry buffer; `LogEntry.source` (`embedded`|`daemon`) is stamped by the
      capturing buffer; `get_daemon_logs` merges both origins, sorts by timestamp, bounds at 2000. Deleted
      the orphan `logs.rs` decoy. 11 tests. *(log_buffer relocated to `nanna-core` in P16.)*
- [x] **Live logs actually poll** *(2026-07-16)* — the old `daemon-log` listener had no emitter (frozen
      snapshot); replaced with a 1 s poll of the merged view + a `clearedBefore` watermark.
      - [ ] Follow-up: a push channel (daemon subscribe + real emit) or a `since`-cursor beats
            re-serialising up to 2000 lines/s; poll avoided an IPC change in a bugfix.

#### P4 follow-on — GUI Testing & UX Quality 🚧 (active track)

Capability shipped in P4; quality did not. The GUI is the richest channel and currently the weakest
*verified* surface — almost no automated UI coverage, and polish debt that makes power features feel
crowded to new users. Goal: **default calm + progressive power** — a new user can chat, set a backend,
and leave; power users still reach logs, tools, workspaces, stats, scheduler without hunting. Track
bugs and improvements here; do not bury them only in the backlog bullet.

**Doctrine**
- Default path is short. Advanced controls live behind progressive disclosure (Advanced, Cmd/Ctrl+K, overflow).
- Power-user depth is non-negotiable: never remove a capability; relocate, name, and shortcut it.
- Prefer fixing root UX (density, hierarchy, language) over adding tutorial chrome.
- Every critical flow gets a regression test before calling the bug closed.

##### GUI automated testing
- [x] **Vitest + Vue Test Utils** — unit/component tests for composables, pure helpers, and high-risk widgets
      (ChatInput stop/send, SessionItem actions, ConnectionStatus / BackendStatus, settings forms, Logs filters).
- [x] **Playwright E2E (web/dev shell)** *(2026-07-22)* — `gui/playwright.config.ts` drives `pnpm exec nuxi dev`
      (or `PLAYWRIGHT_BASE_URL`); 26 chromium specs under `gui/e2e/` run offline via the Tauri mock harness.
      Scripts: `pnpm test:e2e` / `test:e2e:update` / `test:e2e:ui`.
- [x] **Tauri WebDriver / tauri-driver smoke** *(2026-07-22)* — scaffold `gui/scripts/tauri-driver-smoke.mjs` +
      `gui/e2e/tauri-driver.md` (launch → Settings → Logs → close hygiene). Soft-skips when binary/driver missing
      so web CI stays hermetic; armed via `NANNA_TAURI_E2E=1` once a packaged binary is present. Wire full
      WebDriverIO session when nightly hosts a display + driver pair.
- [x] **Critical-path scenarios** *(2026-07-22)* — `e2e/critical-path.spec.ts`: first-run/no-key empty state;
      chat send → stream → Stop (mock LLM); session create/rename/delete/switch; backend disconnect toast +
      reconnect affordance; Settings API-key round-trip; Logs Live/Paused, Clear, Copy all.
- [x] **Page smoke matrix** *(2026-07-22)* — `e2e/page-smoke.spec.ts` hits `/`, agents, channels, memory,
      model-stats, scheduler, settings, tool-stats, tools, workspaces, logs, tasks — each renders primary
      content (no blank shell).
- [x] **A11y gate on changed surfaces** *(2026-07-22)* — `@axe-core/playwright` critical/serious sweep on chat +
      settings; keyboard tab-order reaches main controls; labelled switches / back links / session menu;
      GlassButton forwards `aria-*` on NuxtLink. Follow-on: broader color-contrast token audit.
- [x] **Visual / theme regression (lightweight)** *(2026-07-22)* — `e2e/visual.spec.ts` baselines chat empty,
      settings shell, logs toolbar under `gui/e2e/__snapshots__/` (`maxDiffPixelRatio: 0.03`).
- [x] **CI wiring** *(2026-07-22)* — `.github/workflows/gui.yml`: Vitest unit on every `gui/**` PR; Playwright
      web smoke with report artifact on failure; Tauri-driver soft-smoke on nightly/`workflow_dispatch`.
      Cross-link: P0.3 Code Quality & CI.
- [x] **Fixtures & mocks** *(2026-07-22)* — `gui/e2e/fixtures/{tauri-mock,mock-state,test-base}.ts` installs a
      full Tauri 2 IPC mock (`invoke`/`listen`/window) with seeded sessions, streaming LLM, config, tools,
      logs — hermetic, deterministic, offline (no live LLM / keyring).
- [x] **Crash / error boundaries** *(2026-07-22)* — `ErrorBoundary.vue` wraps shell + chat via `onErrorCaptured`;
      recoverable alert panel + Try again/Reload; e2e force hook `__NANNA_FORCE_ERROR__` asserted in
      `e2e/error-boundary.spec.ts`.

##### UI / UX bugfix (known + sweep)
- [x] **Empty / loading / error / offline** states for every page (chat, logs, memory, tools, channels, stats,
      scheduler, workspaces, agents) — no silent blank panels; retry or next-step where recovery exists.
      *(2026-04-27)* Shared `PageState` + per-page `loadError`/`isOnline`/`empty` wiring across agents, channels,
      memory, tools, tool-stats, model-stats, scheduler, workspaces, tasks, logs; chat + settings get offline
      banners (chat stays interactive for local draft). Retry actions call the page refresh.
- [x] **Connection & backend signalling** — ConnectionStatus / BackendStatus language matches reality (embedded vs
      daemon, reconnecting, degraded); avoid "Disconnected" next to live data (Logs taught this lesson).
      *(2026-04-27)* `app/lib/backendLabels.ts` is the single source: Daemon / Reconnecting / Starting /
      Daemon offline (with endpoint) / Daemon crashed / Legacy. Status bar + badges consume it; bare
      "Disconnected" retired. Unit tests in `gui/tests/unit/backendLabels.spec.ts` + `BackendStatus.spec.ts`.
- [x] **Toasts & destructive confirms** — success/error coverage for copy, save, delete, clear; ConfirmDialog on
      irreversible actions; Escape / outside-click policy consistent app-wide.
      *(2026-04-27)* `useToast` helpers; ConfirmDialog teleported in `app.vue` with outside-click cancel +
      Escape via `pushEscapeHandler` stack; destructive paths (session delete, clear logs, memory wipe,
      channel/tool/workspace/agent/task delete, settings data danger) go through `useConfirm`.
- [~] **Focus, scroll, and overflow** — chat sticks to latest unless user scrolled up; settings tabs don't lose
      focus/scroll jump; long lists virtualize or paginate; no double scrollbars / clipped CTAs on 1280×720 and
      1440×900 baselines.
      *(2026-04-27)* Chat `userScrolledUp` + `scrollToBottom`; settings per-tab scroll restore (`tabScrollPos`).
      *(2026-07-23)* **List virtualization shipped** — pure `visibleRange` + `VirtualList.vue`; memory >80,
      logs >100, tools sidebar >60. Unit tests in `gui/tests/unit/virtualList.spec.ts`. Remaining: formal
      1280×720 / 1440×900 clipped-CTA visual pass (logged in `gui/docs/BUG_BASH_GUI_UX.md`).
- [x] **Keyboard & shortcuts** — global Esc closes topmost dialog/menu; Cmd/Ctrl+K reserved for palette;
      documented shortcuts for new chat / focus input / Stop generation.
      *(2026-04-27)* `useShortcuts` + Escape stack; layout bindings: `Mod+K` reserved, `Mod+Shift+N` new chat,
      `Mod+Shift+L` focus input, `Mod+.` stop; ChatInput Escape stops streaming; `ShortcutsHelp` on Settings → Data.
      *(2026-07-23)* Command palette UI landed (see simplification track).
- [x] **Density & contrast sweep** on Palenight — readable secondary text, toolbar icon hit-targets ≥ 32px,
      consistent spacing scale; no low-contrast badges on logs/stats.
      *(2026-04-27)* Density tokens + `min-h-8`/`min-w-8` hit targets on toolbar icon buttons; secondary text
      tokens tightened in `main.css`. Broader token audit can continue under simplification.
- [x] **Forms validation** — API key / channel wizard / settings save: inline errors, disable duplicate submit,
      don't clear valid fields on partial failure.
      *(2026-04-27)* `app/lib/formValidation.ts` + `ApiKeyInput` inline errors / busy-disable; settings/channel
      saves keep valid fields on partial failure. Remaining unevenness on multi-step channel wizards logged in
      the bug-bash file.
- [x] **Title bar / tray / window controls** (Windows primary) — min/max/close, tray show/hide, quit vs hide
      semantics match user expectation; no orphan daemon on "close to tray" confusion (document + test).
      *(2026-04-27)* Documented in `gui/docs/WINDOW_TRAY.md` (ask / minimize_to_tray / quit_completely;
      sidecar lifecycle; close dialog). Close path still driven by `useCloseHandler` + daemon tray IPC.
- [x] **Bug bash log** — keep a rolling short list in daily-dev notes or issues labelled `gui-ux`; promote
      fixed items to dated `[x]` lines here when closed.
      *(2026-04-27)* `gui/docs/BUG_BASH_GUI_UX.md` started; open carry-overs: list virtualization, channel-wizard
      bulk validation, command palette UI, Windows `node_modules`/vitest lock flakiness.
      *(2026-07-22)* Follow-up hotfix after #58: seven page SFCs had composables spliced inside `interface`
      bodies (broke `nuxt generate` / `cargo tauri build`); restored script order + channels `loadError`
      on catch. Residual logged in BUG_BASH: local channels toast ref; legacy clawd/Nanna config-path copy.
      *(2026-07-23)* **`/tool-stats` was crashing at render — fixed.** `loadError` is referenced five
      times in the template's `PageState` block and assigned twice in `loadStats()`, but was **never
      declared**, so the page threw `loadError is not defined` and `ErrorBoundary` swallowed the whole
      panel ("Something went wrong"). A leftover from the 2026-07-22 script-order hotfix that reached the
      other pages but not this one. Added `const loadError = ref<string | null>(null)` alongside the
      other refs, matching `model-stats`/`memory`/`tools`. The `e2e/page-smoke.spec.ts` suite was already
      catching this — 12/12 green after the fix.
- [x] *(2026-07-23)* **`<UiSonnerSonner />` fails to resolve at runtime — toasts may never render.**
      Every Playwright page load logs `[Vue warn]: Failed to resolve component: UiSonnerSonner` from
      `app.vue`, on **both** this branch and pristine `origin/master`, so it is pre-existing and not a
      dep-bump fallout. The component *does* exist (`app/components/ui/sonner/Sonner.vue`) and the
      auto-import name looks correct for its nested path, so the likely cause is the component failing to
      load rather than being misnamed — e.g. the `vue-sonner` import throwing. Worth chasing because the
      failure is silent and the blast radius is real: `useToast` drives success/error feedback for copy,
      save, delete and clear across the app (P4 "Toasts & destructive confirms"), so if the toaster never
      mounts, none of that feedback reaches the user.
      *(2026-07-24)* **Fixed — and it was the name after all.** The 2026-07-23 read ("the auto-import name
      looks correct… likely the import throwing") was wrong. Nuxt **collapses a filename that repeats its
      parent directory**, so `app/components/ui/sonner/Sonner.vue` registers as **`UiSonner`**, not
      `UiSonnerSonner`. Settled from the generated registry rather than by inspection —
      `.nuxt/components.d.ts` contains exactly `export const UiSonner`. The tag therefore resolved to
      nothing, the `<Toaster>` never mounted, and every `useToast()` call was dropped: `toast.*` pushed
      into vue-sonner's store with no renderer subscribed. One-word fix in `app.vue`. **Unrelated to the
      deferred `vue-sonner 1 → 2` major** — that cross-check is closed.
      **A second, identical bug fell out of the same audit:** `ui/glass-input/GlassInput.vue` wrapped its
      field in `<GroundGlass>`, which registers as **`UiGroundGlass`** (same directory-repeat rule). It
      rendered as an inert unknown element, so the glass slab — borders, mesh gradient, noise overlay —
      never drew, and `glassRef.value?.onEnter()` silently no-opped because the template ref pointed at a
      DOM node instead of the component (the `?.` swallowed it). Used by `pages/workspaces.vue`.
      **Guarded two ways, each proven to fail without the fix:**
      **(1)** `tests/unit/componentResolution.spec.ts` — walks all **91** `.vue` files under `app/` and
      checks every PascalCase template tag against the **203** names in `.nuxt/components.d.ts`, allowing
      Vue built-ins and names the file imports or declares itself. Zero false positives across the tree;
      reverting either fix reproduces exactly the two failures. It asserts the registry is non-empty and
      contains a known component first, so a registry that silently read as empty cannot make it pass
      vacuously. (`nuxt prepare` runs from `postinstall`, so CI has the registry after `pnpm install`.)
      **(2)** `e2e/toaster.spec.ts` — clicks Copy all on `/logs` and asserts a `[data-sonner-toast]`
      actually renders. **Note for anyone tempted to assert on the Vue warning instead: you can't.**
      A first attempt did, and it **passed with the bug still present** — the warning is emitted by the
      dev server's render pass, never into the browser console, so a Playwright console listener never
      sees it. That test was deleted rather than kept as false assurance. Also recorded: the glass buttons
      animate their mesh forever, so Playwright's stability check never settles — assert
      visible+enabled, then `click({ force: true })`.
      Verified: 60/60 vitest, 26/26 Playwright (the 27th is the pre-existing flaky session test below).
      **(3)** *(2026-08-25)* `tests/unit/packageComponentExports.spec.ts` — closes the half guard (1)
      cannot see. Guard (1) allows any tag the file `import`s, which is correct for the Nuxt-registry
      bug it was built for but says nothing about whether the **package** still exports that name.
      Tiptap 3 proved the gap the same day it was written: `import { BubbleMenu } from '@tiptap/vue-3'`
      kept compiling, kept typechecking, and evaluated to `undefined` because v3 moved the menu
      components to `@tiptap/vue-3/menus`. This guard resolves each such binding for real —
      `await import(module)` for every PascalCase tag a template renders that comes from a *bare*
      package specifier — and asserts the export is defined. **221 bindings across 3 modules today**,
      and the cost is bounded by distinct *modules* (the loader caches), not by bindings, which is why
      every `@lucide/vue` icon can be checked for free. Relative/`~`/`#` imports are excluded: a
      missing export there is a loud build error, not a silent `undefined`. It also pins the Tiptap
      root-vs-`/menus` fact as a fixture, so if upstream moves them back the guard's premise gets
      re-read rather than rotting. Reverting either the Tiptap import or the lucide rename reproduces
      a failure. One trap found while writing it, worth not re-learning: the first version read the
      *comment* explaining the moved import as a real import statement, so it must strip comments
      before parsing — a guard that fires on prose about the bug is worse than none, because the fix
      is to delete the explanation.
      *(2026-07-24)* **Command palette gained a fuzzy tier — `subsequenceScore` in `lib/commandPalette.ts`.**
      `filterActions` was substring-only, so the way people actually type into a palette (`mstats`,
      `tglogs`, `nchat`) returned **nothing at all**. It now falls through to a subsequence match over the
      label and keywords, scored to reward consecutive runs and word-boundary landings, and **capped at 25
      — strictly below the weakest literal tier (group = 30)** — so fuzzy can only *add* results a literal
      search missed, never reorder ones it found. Two guards against the noise that makes naive fuzzy
      palettes feel broken: a query must be ≥2 characters, and must land on at least one word boundary
      (`oe` is a genuine subsequence of half the list). Raw score is normalised against the best attainable
      score for the query length so long labels cannot outrank short ones on length alone.
      Fixed in passing: the tier chain tested `group` **before** `keywords`, so an action whose keyword
      matched exactly scored 30 instead of 40 — keywords now come first.
      10 new tests (non-subsequence, single-char, no-boundary rejection; word-initial scoring; consecutive
      beats scattered; the ≤25 ceiling; the three fuzzy queries above resolving; literal-first ranking;
      keyword-over-group). All **8 pre-existing tests pass unchanged**, which is the point — ranking
      behaviour is preserved. 75/75 vitest.
      - [ ] **Not shipped with it: the planned e2e.** The palette **would not open from Mod+K in the
            Playwright dev shell** — not via `keyboard.press` (`Control+k` / `Meta+k` / `ControlOrMeta+k`)
            and not via a synthetic `window` `keydown`, with no console error. The code reads correct
            (registration at layout setup with no preceding top-level `await`, `mod` matches Ctrl *or*
            Meta, `allowInInput: true`, module-singleton state bound through `:open`). Rather than ship an
            e2e that passes for the wrong reason, it was dropped and the observation logged in
            `BUG_BASH_GUI_UX.md`. **Confirm in the real Tauri shell before calling it a product bug** — if
            it reproduces there, Mod+K has never worked for users and the palette is unreachable by its
            advertised shortcut.
      *(2026-07-24)* **Third instance of the family, and the worst one: Settings → Data's "Delete All
      Memories" invoked a Tauri command that has never existed.** `SettingsDataTab.vue` called
      `invoke('clear_all_memories')`; there is **no `#[tauri::command]` of that name anywhere**. The
      call rejects at runtime with "Command not found", the site catches it into a toast — and until the
      toaster fix above, that toast never rendered. So the user confirmed a destructive dialog and
      **nothing happened, silently**, with no memories deleted and no error shown. The real command is
      `clear_memories` (no scope = every scope, which is what the button promises); `memory.rs`'s own
      doc comment asserted `clear_all_memories` "was never called, so the button was dead" — that note
      was itself wrong, Settings → Data was still calling it, and it is corrected in place.
      Also removed: two `invoke('update_setting', …)` **fallbacks** in `SettingsMemoryTab.vue` for a
      command that likewise does not exist. They were worse than dead — wrapped around
      `set_max_compression_ratio` / `set_min_remaining_memories`, they swallowed the *real* failure and
      replaced it with "Command update_setting not found", so a genuine error reached the user as
      nonsense. The primaries exist and are registered; the fallbacks are gone.
      **Guarded by `tests/unit/invokeCommands.spec.ts`**, which checks every `invoke('name')` across the
      frontend against the `tauri::generate_handler![…]` list — the right authority, since a
      `#[tauri::command]` that is *defined but not registered* is equally unreachable. Bracket-matched
      parse (the list spans hundreds of lines and contains comments), depth/count-bounded file walk, and
      the same non-empty-registry assertion so it cannot pass vacuously. Reverting the fixes reproduces
      exactly the three failures. Full sweep result: **132 distinct `invoke()` names, 172 registered
      commands, and after this fix zero unregistered calls.**
      - [ ] **42 registered commands are never invoked from the frontend** — dead or daemon-only IPC
            surface (`apply_memory_updates`, `save_memories`, `spawn_sub_session`, `send_to_sub_session`,
            `kill_sub_session`, `list_sub_sessions`, `get_workspace_context`, `create_skill`/
            `update_skill`/`delete_skill`/`list_skills`, `test_all_channels`, `clear_rate_limit`, …).
            Each is either a feature with no UI or a leftover; triage into "wire up" vs "delete" rather
            than leaving an unaudited command surface exposed to the webview.
      *(2026-07-24)* **Verified in the real Tauri shell over WebDriver** (`cargo tauri build` release,
      `nanna-gui.exe` 16 MB, built under the pinned toolchain): `document.title === "Nanna"`, `#__nuxt`
      attached, `typeof window.__TAURI_INTERNALS__ === "object"` (so this is the real IPC shell, not the
      browser dev shell), **`[data-sonner-toaster]` count = 1** — the toaster genuinely mounts — and
      **zero** `<uisonnersonner>` / `<groundglass>` inert elements, i.e. both resolution fixes hold in the
      packaged app. Screenshot kept with the run.
      - [ ] **Hazard in the shared WebDriver harness — it kills by process *name*.**
            `_shared/tauri-webdriver.ps1`'s `Kill-Stale` does `Stop-Process -Name $names`, so `stop` killed
            the **user's own running `nanna-gui.exe`** (`C:\Program Files\Nanna\`) alongside the
            WebDriver-launched one. No data loss — the daemon is a separate process name, kept running, and
            it owns the state — but an unattended run should not close the user's window. The same harness
            backs the Utter and Laurelane routines, so this affects all of them. Fix: record the launched
            PID in the session state file and `Stop-Process -Id` that, falling back to name-matching only
            when the PID is gone.
      — a `@handler` bound to an event the child never emits, which also fails silently — by checking
      every PascalCase component tag's listeners against the callee's `defineEmits` (allowing native
      fallthrough events, `update:*`, and kebab/camel spellings). Across the 91 files and the **25**
      components that declare an explicit emit contract there are **zero** mismatches. That class is
      clean; don't spend another run probing it.
- [x] *(2026-07-24)* **`<UiInput size="sm">` forwards `size` onto the native `<input>` and the DOM rejects
      it.** Found while verifying the component-resolution fix above; **pre-existing** — reproduced with
      that fix stashed, so it is not fallout. `app/components/ui/input.vue` declared no `size` prop, so
      `size` fell through as an attribute to the `<input>` element, where the HTML `size` attribute must
      be a positive integer. Chromium therefore logged
      `[Vue warn]: Failed setting prop "size" on <input>: value sm is invalid. IndexSizeError` on every
      page that rendered one. Harmless to layout — the Tailwind classes did the sizing — but it is log
      noise that trains everyone to ignore Vue warnings, which is precisely how the toaster bug survived
      for months.
      *(2026-07-24)* **Fixed by declaring the contract, not by deleting the call sites.** `UiInput` now
      has a real `size` prop built with `cva` exactly like `UiButton` — same scale, so the two line up:
      `default` `h-10 px-4 py-2 text-sm` · `sm` `h-8 px-3 text-xs` · `lg` `h-12 px-6 text-base`. The
      default reproduces the previous appearance byte-for-byte, so the 5 existing `size="sm"` call sites
      (`pages/tasks.vue` ×3, `pages/tools.vue` ×2) now get the small control they were always asking for
      instead of a swallowed DOM error, and nothing else moves. Caller `class` still wins over the variant
      via `cn`/tailwind-merge.
      5 tests (`tests/unit/UiInput.spec.ts`): `size` never reaches the DOM element at any variant, each
      variant maps to a distinct height, the default is `h-10`, a caller `class="h-12"` overrides `sm`,
      and `update:modelValue` still emits. Reverting the component fails 2 of them
      (`expected 'sm' to be undefined`). Runtime-confirmed: `/tools`, `/logs`, `/tasks` now load with
      **zero** Vue warnings — both this one and the resolution one are gone.
      Verified: 65/65 vitest, 26/27 Playwright (the 27th is the pre-existing flake below).
- [x] *(2026-07-23, fixed 2026-08-23)* **`critical-path.spec.ts` "session create / rename / delete / switch"
      was flaky — and the flake was pointing at a real UI bug.** The 2026-07-23 diagnosis was exactly
      right: the step's `getByRole('button', {name: /delete|confirm|yes/i})` matched **both** the context
      menu's `Delete` item and the `ConfirmDialog`'s confirm button, so after the menu detached
      Playwright re-resolved onto the dialog button and spun on
      `confirm-overlay … intercepts pointer events` until the 60 s timeout.
      Fixed test-side as planned, and **tightened rather than loosened**: the click is scoped to
      `getByRole('alertdialog')` (the dialog's own role — unambiguous), the confirmation is now
      **required** instead of `if (await confirm.isVisible())` (SessionItem's `confirmDelete()` always
      awaits `confirm()`, so a missing dialog is a regression and must fail), and the fixed
      `waitForTimeout(300)` became `await expect(dialog).toBeHidden()` — while the overlay is fading it
      still intercepts pointer events, so a fixed sleep is either a stall or a race depending on the
      machine. Verified: **6/6 `critical-path.spec.ts` pass**, including the previously-flaky one.
      **But the regex was that loose for a reason**, and chasing it turned up a real bug: the dialog's
      button did not say "Delete". `ConfirmOptions` declares `confirmLabel` / `danger`, while three
      destructive call sites passed **`confirmText` / `destructive`** — keys that do not exist on the
      type. Both fell through to their defaults, so **the three most destructive actions in the app**
      (delete a session; Settings → Data "Delete All Sessions"; "Delete All Memories") rendered a
      generic grey **"Confirm"** with no danger styling. All three corrected. A fourth site,
      `pages/tools.vue`, called the composable's `confirm` as if it were `window.confirm` —
      `if (hasChanges.value && !confirm('Discard unsaved changes?'))` — passing a string and never
      awaiting, so `!Promise` was always `false`: **the unsaved-changes guard never prompted and never
      blocked**, and edits were dropped silently on switching tools. Now a real awaited confirm.
      The scoped test locator (`/^Delete$/i` *inside* the dialog) now also pins the label, so the UI bug
      and its test cannot drift apart again.
- [ ] *(2026-08-23)* **The `vue-tsc` CI gate type-checks NOTHING, and there are 96 real errors behind
      it.** `gui.yml` runs `pnpm exec vue-tsc --noEmit`, and the roadmap records it as "Enforced as of
      2026-07-24: the tree typechecks with 0 errors, so a new one is a regression". It does not. Nuxt 4
      writes a **solution-style** `tsconfig.json` — `"files": []` plus four project `references` — and
      plain `tsc`/`vue-tsc` on that compiles **zero files**; it does not follow references without
      `--build`. Proven, not inferred: inserting `const definitelyBroken: number = 'this is a string'`
      into `SessionItem.vue` still exits 0, as does a bogus extra key on a typed object literal.
      `pnpm exec vue-tsc --build` reports **96 errors** across 12+ files — worst offenders
      `app/lib/tiptapMarkdown.ts` (26), `app/extensions/MonacoCodeBlock.ts` (16),
      `app/pages/workspaces.vue` (9), `app/pages/memory.vue` (6). Two of the four `confirm()` bugs fixed
      above are in that list, which is the point: this gate would have caught them the day they landed.
      Not switched on in the same run, deliberately — flipping the flag turns CI red on 96 pre-existing
      errors, and a green build achieved by leaving the gate blind is the thing being fixed here, so it
      should not be traded for a red one nobody can land against. Do it as its own increment(s):
      - [~] Burn down the 96 in batches by file, largest first, keeping CI green throughout.
            *(2026-08-23)* **First batch: `app/lib/tiptapMarkdown.ts` — 26 errors → 0, total 96 → 66.**
            All of the `noUncheckedIndexedAccess` family (`lines[i]` types as `string | undefined`
            even under an `i < lines.length` guard, and regex group reads likewise).
            This file is the inbound composer path and its own header says it: "a corruption here is a
            corruption of what the user actually said" — and it had **6 tests**, which is not cover to
            refactor every branch behind. So a **characterization suite went in first and was run green
            on the unmodified code**: `tests/unit/tiptapMarkdownGolden.spec.ts`, **47 snapshots / 49
            tests** over every parser branch and the edges that decide whether an index read can run
            out of range (unterminated fence, fence as last line, empty fence, one-line inputs, blank
            input, multi-byte). Only then the fix — and **all 47 snapshots re-matched unchanged**, so
            the change is proven behaviour-preserving rather than assumed to be.
            Fixed with a total accessor (`const at = (n) => lines[n] ?? ''`) and `?? ''` on regex
            groups, **not** `!`: a non-null assertion is erased at runtime, so it would leave a genuine
            out-of-range read to stringify into the user's message as the literal "undefined" — which
            two of the new tests assert against directly. Every call site is already under a bounds
            guard, so `at()` never actually substitutes; it only supplies the proof the checker cannot
            derive.
            **Verified non-vacuous**: a one-character change to the fence parser
            (`line.slice(3)` → `slice(4)`) trips 3 snapshots. 208 vitest green (was 159).
            *(2026-08-23, same run)* **Second batch: 66 → 41, and it found a broken feature.**
            `app/extensions/MonacoCodeBlock.ts`'s 16 errors were **one** root cause, not sixteen:
            `Cannot find module '@tiptap/core'`. Three files import `@tiptap/core` directly
            (`MonacoCodeBlock.ts`, `SlashCommands.ts`, `FloatingToolbar.vue`) but it was never a
            declared dependency — only a transitive one, which pnpm's strict `node_modules` layout
            correctly refuses to resolve by name. Every other error in those files was a downstream
            implicit-`any`, because Tiptap's callback parameter types could not be inferred without it.
            Declaring `"@tiptap/core": "2.27.2"` (pinned to the family) fixed **17 errors in one line**
            and removed a real fragility: the code was relying on hoisting it never asked for.
            Two genuine stragglers then fixed: `state.schema.nodes.paragraph` is optional and
            `.create()` on an absent node type would throw **inside an input rule, mid-keystroke** (now
            guarded — the code block still inserts, only the trailing paragraph is skipped); and
            `VueRenderer.element` is `Element | null` while tippy takes `Content | undefined`, so a null
            would mount an empty popup.
            - [x] **`pages/memory.vue`: editing a memory was broken outright.** Its six errors were all
                  one bug — the template bound `@click="startEditMemory(memory)"`,
                  `saveEditMemory(memory)` and `cancelEditMemory`, none of which exist. The script
                  defines `startEditing` / `saveEditing(id)` / `cancelEditing`. In Vue an undefined
                  template handler throws on click, so **the Edit / Save / Cancel buttons on every
                  memory card threw**, in both the semantic and episodic lists. Note the signature
                  mismatch too: `saveEditing` takes an **id**, while the template was passing the whole
                  memory object. Rewired all six call sites. This is the clearest possible argument for
                  the gate: a shipped feature was dead, and a typecheck that actually ran would have
                  said so the day it landed.
            *(2026-08-23, same run)* **Third batch: 41 → 32.** `app/pages/workspaces.vue`'s nine errors
            were one modelling gap: `contextFiles` / `detailFiles` / `availableFiles` declared their
            `key` / `existsKey` as plain `string`, and the templates index workspace objects with them
            (`ws[file.key]`, `createValidity[file.existsKey]`) — a `string` is not provably a member of
            either shape, so every such read was an implicit `any`. Fixed at the source rather than at
            the call sites: `app/lib/workspaceMarkers.ts` — the module that already owns
            `WorkspaceValidity` — now exports a `ContextFileKey` literal union, and the three arrays are
            annotated with it. The remaining one was a real (if small) template-typing bug:
            `:disabled="createValidity && createValidity[key]"` yields `boolean | null` when
            `createValidity` is null, which is not `Booleanish`; `createValidity?.[key] ?? false` keeps
            the same truthiness and gives a plain boolean.
            *(2026-08-23, same run)* **Fourth batch: 32 → 25, three more broken surfaces.**
            - **`tool-stats.vue`: the latency-percentile chart never rendered.** All four of its errors
              were one swapped pair. Vue's object `v-for` binds **value first, key second**, and the
              template read `v-for="(label, val) in { P50: …, P95: …, P99: … }"` — so `label` held the
              latency number and `val` held the string `"P50"`. Consequences: `val > 5000` compared a
              string to a number (always false, so the red/amber bands never fired), and the bar height
              computed `"P50" / n` → `NaN` → `height: NaNpx`, i.e. **no bars at all**; the caption
              printed the number and the value printed a formatted string. One binding swap fixed all
              four errors and the whole widget.
            - **`scheduler.vue`: the timezone dropdown rendered empty.** `UiSelect` builds its list from
              an `options: Option[]` prop, and it was being handed eight slotted `<option>` children,
              which the component never reads. Moved to `:options="timezoneOptions"`.
            - **`scheduler.vue`: the schedule-preset buttons had no styling.** `variant="outline"` is not
              one of `UiButton`'s variants (`default | secondary | ghost | destructive | link | accent`),
              so they silently fell through. Now `secondary`, the bordered one.
            - `ui/card.vue` typed its `class` prop as `string` while rendering it through `cn` (clsx),
              which accepts objects — so the legitimate `:class="{ 'opacity-50': … }"` form read as a
              type error. Widened to `ClassValue`: the type was narrower than the runtime, not the
              other way round.
            **Running total for the run: 96 → 25 errors**, with 208 vitest green and `pnpm build` green
            throughout. Remaining backlog: `app/components/settings/*`, `app/components/ToolCallCard.vue`,
            `app/layouts/default.vue`, the `ui/` primitives, and a handful of one-error files.
      - [ ] Then switch `gui.yml` to `vue-tsc --build` (or `nuxt typecheck`) and re-assert the
            "0 errors" claim — this time with evidence that the command sees the files.
      - [ ] Add a **meta-check** so a blind gate cannot recur: the typecheck step should fail if it
            reports zero *checked files*, the same way a coverage gate fails at 0%.
- [x] *(2026-08-24, fixed the same day)* **`nanna-scripting/tests/edit_file_skill.rs` fails under machine load — an absolute
      deadline in a test, not a regression.** Six of its seventeen tests failed a full-workspace
      `cargo test` with `"Timeout after 30000ms"` while two cargo builds and sixteen other test binaries
      shared the box; the same file run alone passes **17/17 in 3.49 s** — a ~10× margin, so the failure
      is scheduling, not logic. Confirmed off any changed path: `nanna-scripting`'s only in-tree
      dependency is `nanna-proc`.
      This is still a real test-quality bug, because a suite that goes red on a busy CI runner trains
      people to re-run rather than read. The engine's deadline is wall-clock and absolute; under
      contention the *script* never gets 30 s of CPU. Fix is test-side — raise the deadline for these
      fixtures specifically (they measure edit semantics, not latency), or have the harness assert on
      completion rather than on a wall-clock bound. Do **not** simply retry: a genuine hang and a
      starved scheduler look identical from outside, and the distinction is what the deadline exists
      to draw.
      **Same class, different suite:** `nanna-client/tests/e2e_daemon.rs` failed all 4 on a second
      loaded run ("daemon did not start listening on port … within 10s") and passes **4/4 in 1.54 s**
      alone. Its wait is also an absolute 10 s. Both suites pass under
      `cargo test -- --test-threads=4` (**1593 tests, 0 failures** this run), so the workable
      short-term answer is to bound test parallelism in CI; the durable one is to stop asserting
      wall-clock in tests that are not measuring latency.
      *(2026-08-24, PR #264)* **Fixed, and the durable way round.** `e2e_daemon` now watches the
      daemon task rather than the clock — a task that ends without binding fails at that moment with
      its actual error, or resumes the original panic — with the clock demoted to a 120 s hang ceiling
      whose assert says it is not a latency assertion. The eight scripting harnesses that were
      inheriting `ScriptedTool`'s production 30 s default now take one shared
      `tests/common::FIXTURE_TIMEOUT_MS`; `project_structure` keeps its 120 s because the skill
      hardcodes `SCRIPT_DEADLINE_MS` and derives its work budget from it. Verified at **default**
      parallelism — the exact condition that failed — 1555 passed / 0 failed.
      *(2026-07-23)* Simplification pass closed most open carry-overs (palette, virtualization, IA nav,
      Advanced settings). Remaining bash items: channel-wizard bulk validation, formal viewport pass,
      channels toast ref, legacy clawd config-path copy.
      *(2026-07-24)* **Mixed static/dynamic import of `@tauri-apps/api/window` collapsed to static.**
      `TitleBar.vue` and `layouts/default.vue` each did `await import('@tauri-apps/api/window')` in their
      mount hook while `composables/useCloseHandler.ts` **statically** imports the same module — and
      `default.vue` calls `useCloseHandler()`, so the module was already in the static graph. The dynamic
      form bought no code-splitting (the bundler warns and inlines it anyway), only an extra `await`
      before the window handle was available on mount. `ssr: false` in `nuxt.config.ts`, so there was
      never an SSR reason for it either. Both are now plain static imports.
      Fixed in passing: `default.vue`'s hook named its handle `const window = getCurrentWindow()`,
      **shadowing the global `window`** inside an async mount callback — renamed to `appWindow`, matching
      what `TitleBar.vue` already called it.
      Verified: `pnpm generate` green with the mixed-import warning gone, 65/65 vitest,
      26/27 Playwright (the 27th is the pre-existing flake).
      *(2026-07-23)* **`nuxt generate` manifest race mitigated** — dual Vite client passes were racing
      `node_modules/.cache/nuxt/.nuxt/dist/client/manifest.json` (ENOENT mid-generate while nitro still
      prerendered and Tauri packaging kept going). Pin `buildDir: '.nuxt'`, prerender `/` only
      (`crawlLinks: false`), wipe `.nuxt` + cache before every `pnpm generate`
      (`gui/scripts/clean-nuxt-cache.mjs`). Also drop unused `README_FILE` import in
      `nanna-workspace::manager` (test-only). Residual: confirm dual "Building client..." lines never
      return after a cold wipe; Monaco ~4 MB chunk + `@tauri-apps/api/window` dual-import style logged
      in `gui/docs/BUG_BASH_GUI_UX.md`.

##### UI simplification (default calm, power remains)
- [x] **IA audit** — diagram primary tasks (chat, configure model, inspect run, manage memory/tools/channels)
      vs admin (logs, raw stats, scheduler, workspaces). Nav / TitleBar should match that hierarchy.
      *(2026-07-23)* Activity bar split: **primary** Memory/Tasks/Tools/Channels always visible; **admin**
      Logs/Workspaces/Agents/Scheduler/Model Stats/Tool Stats under a More flyout. Settings remains bottom.
      Documented in `gui/docs/BUG_BASH_GUI_UX.md` IA diagram.
- [x] **Progressive disclosure** — fold rarely-used settings into **Advanced**; keep power paths one click or one
      command-palette query away; optional "Compact power mode" density for existing users.
      *(2026-07-23)* Settings `showAdvanced` toggle (persisted); agent iteration/nudge, memory compression floor,
      Ollama host details, model routing folded. Compact density via `html.density-compact` + palette action /
      `nanna.ui.density` localStorage.
- [x] **Command palette (Cmd/Ctrl+K)** — navigate pages, switch sessions/workspaces, toggle Live logs, jump to
      model/settings; primary discovery path for power features so chrome can stay thin.
      *(2026-07-23)* `CommandPalette.vue` + `lib/commandPalette.ts` + `useCommandPalette` singleton; ↑/↓/Enter/Esc;
      Primary/Admin nav groups; sessions/workspaces; quick actions (new chat, live logs, focus input, stop,
      settings models, compact mode, toggle chat panel). 8 unit tests. Settings `?tab=` deep-link used.
- [~] **Chat-first shell** — reduce competing sidebar chrome default; rich editor/tool cards compact until expanded;
      streaming/stop/queue status always obvious without reading tool internals.
      *(2026-07-23)* Nav chrome reduced (admin under More; chat panel toggle stays default discovery). Remaining:
      stronger default-collapsed tool/thinking cards; tighten streaming/stop/queue affordances without internals.
- [x] **Unify settings shell** — consistent section headers, descriptions, save model (auto-save vs explicit Save);
      one pattern for comprising toggles + danger zones.
      *(2026-07-23)* `SettingsSection.vue` (`title`/`description`/`danger`/`advanced`); Models/Agent/Memory/Data/
      Scheduler switched over. Explicit Save retained for bulk config; per-control still auto-persists via invokes.
- [x] **Onboarding compression** (pairs with P0.1) — first-run: what Nanna is → pick backend → health → chat;
      defer channel wizards, tool permissions detail, memory deep-dive until after first successful turn.
      *(2026-07-23)* `OnboardingWizard.vue` 3-step (intro → provider/key via ApiKeyInput → health) gated by
      `nanna.onboarding.done` + no-key check. Full P0.1 wizard body (privacy, tool permission setup, storage
      location) still own phase.
- [x] **Copy / tone pass** — system language calm and specific ("Daemon not reachable on 5149" beat "Error");
      kill decorative status that lies (see Logs Live).
      *(2026-07-23)* Settings/scheduler/logs offline copy tightened; logs source label no longer claims
      "embedded" as a backend mode (GUI vs daemon). Live/Paused already factual. Residual clawd path copy open.
- [x] **Component cleanup** — inventory near-duplicate dialogs/status badges; consolidate on `components/ui`;
      delete dead CSS/unused props after simplification.
      *(2026-07-23)* Inventory in `gui/docs/COMPONENT_CLEANUP.md`. Consolidation intentionally deferred
      (ConfirmDialog vs UiModal keep distinct UX roles); execute merges under that doc.

##### UX / product improvements (still on this track)
- [ ] Full-text search across sessions; export conversations (MD/PDF/JSON).
- [ ] Context-budget visualization and live run view (iteration, active tools, token burn-rate, optional Gantt).
- [ ] Drag-drop file upload into chat; optional split view.
- [ ] Font-size + accent controls; theme-token audit; lazy-load Monaco.
- [ ] Mobile / small-window real-device pass (Tauri Android/iOS later; desktop responsive now).
- [ ] Per-tool stats drill-down + latency sparklines (P4 polish tail).
- [ ] Swarm execution view (from P5 open item) when swarm UX becomes demoable.

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
      - [x] Add **Fable 5** (`claude-fable-5`) to the pricing table once its per-Mtok rate is published.
            *(2026-07-21)* Added `("claude-fable", 10.00, 50.00, 1.00, 12.50)` — $10 input / $50 output (≈2× Opus
            4.8), cache-read 0.1× input ($1.00), 5-min cache-write 1.25× input ($12.50). Placed **before** the
            generic `claude` prefix so `claude-fable-5` resolves to its own row, not the Sonnet fallback (a
            regression test pins exactly this). Sources: platform.claude.com/docs pricing, anthropic.com/claude/fable.
      - [~] Config-overridable pricing (`[pricing]` TOML or a fetched table) so rates don't rot in-code; add a
            batch-mode (0.5x) + 1-hour-cache (2.0x) multiplier the tracker can apply.
            *(2026-07-21)* **Multipliers shipped** as pure `ModelPricing` combinators: `with_batch_discount()`
            (halves input+output, leaves cache rates — the Batch API doesn't cache) and `with_hour_cache_write()`
            (cache-write → 2× input, anchored to the input rate so it's exact regardless of the stored 5-min
            write). Both `#[must_use]`, `debug_assert`-guarded (discount only lowers; 1-h write ≥ input), 2 tests.
            Still open: making the table itself config-overridable (`[pricing]` TOML / fetched) and wiring the
            multipliers into the tracker per request-mode.
      *(2026-07-12)* Completeness: `ModelStatsSummary` now carries `total_cache_creation_tokens` (`record()`
      already accumulated it but `summary()` dropped it, hiding cache-write volume and understating cost);
      populated in `summary()` + a regression test. Backward-compatible (additive field; serde consumers ignore
      unknown/extra fields). Added `ModelStatsTracker::total_cost_usd()` (grand-total known cloud spend; sums
      only priced models) surfaced as `total_cost_usd` on the `SystemAction::ModelStats` response; test.
- [ ] **Runtime config reload** — watch `config.toml` with `notify` (debounce 500ms), validate before
      apply, apply without restart, emit `config-change` events.
- [ ] **Per-channel config** — `[channels.<name>.agent]` sections (system_prompt/model/max_tokens/tools allowlist).
- [~] **Tool allowlists/blocklists** — `ToolPolicy` (global allow/block + per-channel + per-user for multi-user channels).
      *(2026-07-20)* **Core `ToolPolicy` shipped + enforced.** New `nanna-tools::policy` — an allow/deny
      policy over *canonical* tool names with three security properties: **deny wins** (a name on both lists
      fails closed), **overlay only narrows** (`ToolPolicy::overlay` unions denials + intersects allowlists,
      so a per-channel layer can never re-grant a globally-denied tool — the per-channel/per-user layering
      primitive is in place), and — critically — the registry enforces it in `execute()` **after**
      alias/fuzzy resolution + `canonical_name()`, so `Bash`→`exec`, `EXEC`, or a fuzzy near-miss cannot
      slip a denied tool past the gate (this exact bypass class is what Claude Code's permission docs and the
      2026 MCP tool-shadowing research warn about — [permissions](https://code.claude.com/docs/en/permissions),
      [CrowdStrike agentic tool-chain attacks](https://www.crowdstrike.com/en-us/blog/how-agentic-tool-chain-attacks-threaten-ai-agent-security/)).
      Denied tools are also dropped from `definitions()`/`definitions_for_names()` so the model isn't even
      offered them (and a denied canonical hides its aliases). Wired through `DaemonConfig.{tool_allowlist,
      tool_denylist}` ← `[tools] enabled`/`disabled`; `build_tool_policy` treats `["*"]`/empty enabled as
      unrestricted and applies `disabled` as the denylist. **This closes the long-standing "disabled tools
      still execute" bug** (P1/P6) — the `[tools] disabled` list was parsed into config but never enforced.
      21 tests (11 policy-unit incl. overlay associativity/identity + regain-prevention, 8 registry incl.
      alias- and fuzzy-bypass regressions, 6 daemon `build_tool_policy`). Remaining: per-channel/per-user
      `[channels.<name>.agent]` overlay wiring + a per-tool audit log; refuse-to-compile for unenforceable
      patterns (Claude-Code style).
      - [ ] **Per-channel/per-user policy overlay** — `[channels.<name>.agent].tools` allow/deny composed
            via `ToolPolicy::overlay` (primitive already shipped) so a channel can only *narrow* the global
            policy. Set on the registry per-session when a channel message enters the agent loop.
      - [ ] *(research 2026-07-20)* **Merge the permission boundary into an OS-level sandbox.** Claude Code
            merges `Read`/`Edit` deny rules into a filesystem boundary and `WebFetch(domain:)` into a network
            allowlist because policy alone never covers subprocesses — a Python/`exec` script opening files
            directly escapes the tool gate. Nanna's `exec` (Git Bash) has exactly this hole; the policy layer
            needs an OS sandbox beneath it. Source: [Claude Code permissions](https://code.claude.com/docs/en/permissions).
      - [ ] *(research 2026-07-20)* **Drop arbitrary-code-execution grants on entering unattended/autonomous
            mode**, even if configured for interactive use — Anthropic's auto-mode discards blanket shell +
            wildcarded interpreters (`python`/`node`/`ruby`) + package-manager run commands on entry. A
            `ToolPolicy` preset the daemon applies when running headless/scheduled. Source:
            [Claude Code auto mode](https://www.anthropic.com/engineering/claude-code-auto-mode).
      - [ ] *(research 2026-07-20)* **Reasoning-blind approval + tool-output injection tagging.** For any
            human-in-the-loop tool approval, feed the classifier only user messages + tool calls (strip
            assistant text + tool results) so the agent can't argue past its own gate; separately tag
            tool-*output* content that looks like injected instructions. Maps onto `AgentContext`. Source:
            [Claude Code auto mode](https://www.anthropic.com/engineering/claude-code-auto-mode).
      - [ ] *(research 2026-07-20)* **Trust-on-first-use tool-definition pinning (anti-rug-pull).** Hash-pin
            each tool's description + schema at first approval; re-prompt on drift; require explicit approval
            for a tool "upgrade". Stops a tool whose definition mutates after approval, and the tool-shadowing
            class where one tool's description steers another tool's parameters. Applies to MCP-discovered
            tools and `discover_tools` activation. Source:
            [CrowdStrike agentic tool-chain attacks](https://www.crowdstrike.com/en-us/blog/how-agentic-tool-chain-attacks-threaten-ai-agent-security/).
- [x] **Log rotation** — `tracing-appender` daily rotation, max ~7 files (logs currently accumulate unbounded).
      *(2026-07-09)* New `nanna-daemon::log_file` builds a `RollingFileAppender` (DAILY rotation,
      `filename_prefix="nanna-daemon"`, `.log` suffix, `max_log_files(7)`) wrapped in `tracing_appender::non_blocking`;
      added as an `Option<fmt::Layer>` beside the console + in-memory-buffer layers. New `--log-dir`
      (default `{data_dir}/logs`) and `--no-file-log` flags; the worker guard is a `main`-scoped local so it
      flushes on normal return (a `static` guard would never drop). Pure `resolve_log_dir` + `build_appender`
      with 4 unit tests; verified by a real `nanna-daemon run` boot writing a prefixed file. Note:
      `tracing-appender` 0.2.5 supports only time-based rotation (no per-file size cap) — if size-bounding is
      wanted later, use a custom writer or the `clia/tracing-appender` fork.
- [x] Reach **0 clippy warnings** — ~~3 deferred items remain: refactor `handle_daemon_command`
      (main.rs ~1442-1636, `too_many_lines`), move mid-function `use nanna_client::…` to top (main.rs ~1576,
      `items_after_statements`), drop unused `async` on `is_daemon_running` (main.rs ~1694, `unused_async`).~~
      *(2026-07-23)* **The `nanna` binary crate is now at 0 clippy warnings** (was 17). Two findings:
      (1) all **three** deferred items were already gone — the P11 decomposition that split `main.rs` into
      `src/commands/*` resolved them; verified by grepping the live clippy output for `too_many_lines` /
      `items_after_statements` / `unused_async` under `src/`, which returns nothing. The roadmap was stale,
      not the code. (2) What actually remained was 16 instances of `redundant_pub_crate` /
      "pub(crate) module inside private module": `mod commands;` is private in a *binary* crate, so
      `pub(crate)` inside it exports nothing extra and clippy asks for plain `pub`. Swept
      `src/commands/*` + `src/setup.rs`, plus one `redundant reference in println!`. Nothing changed
      visibility in practice — a binary's private module tree is unreachable from outside the crate either
      way. Build green; `nanna mcp serve` re-verified live (2/2 protocol lines, 39 tools).
      Remaining for the workspace-wide goal (this item only covers the `nanna` bin): the library crates
      still carry ~2300 warnings, dominated by `missing # Errors` docs, `missing backticks`, and
      `significant_drop_tightening` — a mechanical sweep, best done crate-by-crate before `-D warnings`
      can be enforced in CI (P0.3).

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
- [~] **Per-channel sessions** (High) — map `channel_id:chat_id → session_id` so each chat/DM gets
      isolated context (all messages currently share one context).
      *(2026-08-23)* **The headline was stale; the hole it hid was real and is now fixed.** Both live
      paths have keyed sessions per channel/chat/sender for some time —
      `ChannelManager::process_message` builds `{provider}:{channel.id}:{sender.id}` for everything the
      daemon routes, and `nanna-server`'s handlers build `telegram:{chat_id}:{user_id}`,
      `discord:{channel_id}:{user_id}`, `slack:{channel_id}:{user_id}`. So "all messages currently
      share one context" has not been true generally.
      **It was true for the generic webhook.** `extract_generic_message` never received the registered
      hook id, so pattern 2 (`{text,user}`) and pattern 3 (`{content}`) hardcoded
      `chat_id: "generic"` and pattern 1 fell back to `"unknown"` — and since the session key is
      `{provider}:{chat_id}:{sender_id}`, a constant `chat_id` is a constant session. **Every
      registered generic hook shared one conversation**, including hooks admitted by *different*
      secrets; and pattern 3, which also hardcoded `sender_id: "unknown"`, put every anonymous caller
      into that same context.
      Fixed by threading the hook id through and using it as the identity fallback. The choice is
      principled rather than convenient: each generic hook id carries **its own secret** in
      `generic_secrets`, so the id is exactly the trust boundary the auth model already establishes —
      two callers share an id precisely when they share a credential, which is when they belong in one
      conversation. An explicitly supplied `channel`/`chat_id` still wins, so nothing that was already
      isolating itself changes. For pattern 3, which names nobody, the credential is the only identity
      there is, and using it for both fields is the honest reading; the old `"unknown"`/`"generic"`
      pair claimed an identity the payload never supplied and merged everyone into it.
      4 tests: the three pattern tests now assert the resulting `chat_id` (they previously asserted
      only content/sender, which is why they never pinned this), plus a dedicated isolation test
      asserting the four properties that matter — two hooks never collide, two hooks with the *same*
      sender never collide, one hook + one sender is **stable** (isolation must not become a new
      session per request), and two senders on one hook stay separate. **Verified non-vacuous**:
      restoring the constants fails 3 of them. 298 `nanna-daemon` tests green, clippy 0 errors.
      - [x] *(2026-08-23)* **`nanna-server`'s generic hook had the opposite failure, now fixed.** Its
            fallback was `format!("{channel}:{user_id}:{}", Uuid::new_v4())` — a **fresh UUID per
            request** — so a caller that does not thread `session_id` back itself started a brand-new
            conversation on every message, and the agent remembered nothing across turns on this route
            alone. Its four siblings all derive a stable key from the identity fields they are handed
            (`telegram:{chat_id}:{user_id}`, `discord:{channel_id}:{user_id}`, …), and `channel` and
            `user_id` are both **required** on this payload — so there was nothing to fall back to and
            no randomness to add. Now `generic:{channel}:{user_id}`, extracted as a pure
            `session_key(session_id, channel, user_id)` so it is testable without an LLM.
            An explicit `session_id` still wins (a caller managing its own sessions is unaffected), and
            a **blank** one does not count as supplied — `Some("")` is what a half-filled template
            leaves behind, and honouring it would put every such caller into one conversation named
            `""`. The key stays namespaced with `generic:` so it cannot alias a real Telegram or
            Discord session in the shared session namespace.
            5 tests: stability across identical requests, distinctness across users and across
            channels, explicit-id precedence (incl. trimming), blank-id fallback, and the
            cross-provider collision guard. 29 `nanna-server` tests green, clippy 0 errors.
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
- [ ] Gateway control: `/restart` + `/status` as channel commands, full backup/restore archive, ~~self-update via GitHub releases~~ **(GUI half landed 2026-07-24, v0.2.1: tauri-plugin-updater with signed NSIS artifacts, endpoint = raw master `.updater/latest.json` since `releases/latest` skips pre-releases; status-bar "Update to vX" chip — user-initiated apply so a running mission is never yanked. Remaining: headless-daemon self-update.)**

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
- [ ] *(research 2026-08-26)* **arti 2.5.1 (2026-08-04) adds onion-service features worth having, but
      `onyums` has not moved.** 2.5.1 brings unix-socket target addresses for onion services, plus
      experimental congestion control and Counter Galois Onion cryptography on onion-service circuits.
      `onyums` — the crate P9 requires all Tor traffic to go through, and the reason `arti-*` must
      never be pinned directly — is still **0.2.5, roughly three months old**, so none of that reaches
      us until onyums re-exports a newer arti. Nothing to do now: re-check onyums's arti floor on the
      next sweep, and do NOT reach for a direct `arti-*` pin, which this phase forbids. Sources:
      [Arti 2.5.1](https://blog.torproject.org/arti_2_5_1_released/),
      [onyums](https://crates.io/crates/onyums).

### P10 — Token Efficiency & Cost Optimization ✅ (mostly)
Done: Anthropic + OpenAI native prompt caching + hit tracking, cross-provider model routing with
complexity classifier + tool-call-only routing + first-message override, aggressive tool-output
summarization, progressive distillation (rolling summary every N turns), tool-result eviction, CDC
message-level dedup, per-model stats tracker + persistence + stats-informed routing.
- [x] **LLMLingua-style prompt compression** *(2026-07-16)* — `nanna-agent::compressor` scores sentences
      via the configured summarization model, keeps top-`1/ratio` by density (head/tail fallback); tier-1
      proactive pass rewrites large older tool results before `drop_oldest`. (Sentence-level, not per-token.)
- [x] **Structured tool output schemas** *(2026-07-17)* — `ToolDefinition::output_schema` +
      `nanna_tools::output`; verbose tools declare schemas, accept `output_mode=text|json`, attach `data`
      via `ToolResult::with_data`. Default stays free-form text.
- [x] **Better token estimation** *(2026-07-07 / 07-17)* — character-class + family-aware estimators
      (English/Code/Auto densities) with per-message framing, plus an exact `tiktoken-rs` path
      (`estimate_tokens_for_model`, default-on `tiktoken` feature); replaces the `len()/4` heuristic.
- [x] **Streaming cache tracking** *(2026-07-06)* — `StreamEvent::MessageStart` carries
      `input_tokens`/`cache_read`/`cache_creation` (from Anthropic `message_start` usage), captured into
      `LlmResult` instead of placeholders.

### P11 — Correctness, Security & Architecture Debt ✅ (backlog drained 2026-07-18)

The near-term correctness/security/debt backlog — **fully drained**. Every item below is done with
tests, **superseded by P16** (which deleted embedded mode), or **handed to its owning flagship phase**.
Kept as a compact ledger; the full dated rationale and `file:line` anchors for each fix live in its commit.

**Security (all done):**
- [x] User-tool path traversal — `validate_tool_name` at the `create_tool` chokepoint (daemon + GUI). *(2026-07-06)*
- [x] Workspace file traversal — `validate_context_filename` guards `save_context_file`. *(2026-07-06)*
- [x] Discord webhook Ed25519 + Slack webhook HMAC-SHA256 verification (constant-time, replay-guarded). *(2026-07-07)*
- [x] Hardened `delete_skill` (symlink/traversal checks before `remove_dir_all`). *(2026-07-14)*
- [x] Memory-extraction prompt-injection fencing (untrusted-conversation markers + forged-fence defang). *(2026-07-06)*

**Correctness (all done):**
- [x] Response healing for malformed LLM JSON — chat tool-args, embeddings, summarization. *(2026-07-15)*
- [x] Stop button preserves partial work in both the UI and the model context. *(2026-07-15)*
- [x] `parse_model_id` infers provider from name prefix (daemon + GUI). *(2026-07-06 / 14)*
- [x] Memory durability & correctness: atomic persistence (temp+rename); dream consolidation is add-then-remove (no cluster loss) and scope-homogeneous (no cross-workspace leak); dream expansion re-embeds; merge folds instead of duplicating; `remember`/`recall` and dreaming are workspace-scoped. *(2026-07-06 → 18)*
- [x] Model-aware context budgets everywhere — `compression_threshold ≤ hard_limit`; `ModelInfo` is the single source (no per-model hardcode tables). *(2026-07-13 → 15)*
- [x] Orphaned-message-on-failure stores a partial reply instead of leaving the user turn unanswered. *(2026-07-15)*
- [x] Wired all `not_implemented` daemon control actions — regenerate, tool enable/disable, channel status, uptime, non-destructive `peek_mailbox`. *(2026-07-06 → 14)*
- [x] Windows service install/uninstall/start/stop via the SCM (platform-aware default args). *(2026-07-17)*
- [x] Live model stats through a shared tracker; single health-server bind serving the live shared state. *(2026-07-11 / 12)*
- [x] MCP server notifications classified + `list_changed` cache invalidation. *(2026-07-06 / 10)*
- [x] JS tools parse real parameter schemas from their manifests. *(2026-07-11)*
- [x] Tool-manager consistency — clone→validate→mutate→save→swap, dup-name reject, enabled-flag reconciliation, unregister cascade, non-string enums preserved. *(2026-07-09 / 10 / 17)*
- [x] `parse_retry_after` non-ASCII byte-offset fix; `LlmClient` cache keyed by a credential fingerprint. *(2026-07-12 / 17)*
- [x] Daemon boot degrades (not fails) without an embedding key — probe via the shared `EmbeddingRouter`. *(2026-07-16)*
- [x] Scripted `exec` honors its `timeout` and kills the process tree on overrun; tools default to the active-workspace dir at boot (not `~`). *(2026-07-17)*
- [x] Deterministic tests — env-flaky keyring fallback + env-race `resolve_tools_dir` fixed; latent test/compile drift repaired; `test-compile.yml` CI smoke check added (first run green, 16m cold). *(2026-07-06 → 17)*
- [x] Python interpreter runs on a sized 256 MiB thread stack with `sys.setrecursionlimit` clamped so it can't abort. The floor is principled — derived from the empirical overflow bisection (release passes at 128 MiB) — and a separate in-process *setup*-stack measurement was found **Windows-infeasible** (paint-and-scan faults on the lazily-committed stack past the guard page; overflow aborts uncatchably — verified), so the size stays anchored to the bisection rather than a magic number. *(2026-07-16 / 18)*

**Dead-code warnings were two disabled features, not two unused names (2026-08-25).** Both surfaced
as `field ... is never read` and both turned out to be a bound or a feature that had been wired up to
the edge and then not connected. Recorded because "delete the field" would have been the wrong reading
of either one:
- [x] **Every CDP browser page operation was unbounded.** `BrowserConfig::timeout_ms` (default 30 s,
      with a public builder) was threaded into `CdpPage` and read by nothing, and
      `BrowserError::Timeout` had no constructors either — so `goto`, `screenshot`, `evaluate` and
      `wait_for_selector` waited on a hung page forever, with no cancellation and no error. All 13 page
      operations now run under that deadline via one `bounded` helper; `fill` and `goto` take a single
      budget for their whole multi-call sequence rather than one per CDP round-trip, so a degraded page
      cannot spend 4× the stated timeout while every step stays inside it. `Timeout` now carries the
      operation and the deadline it exceeded. The bound is extracted as a free function so it is
      testable without a live browser — the missing test dependency is a large part of why it went
      unapplied — and **4 tests** cover it: a hung operation is cut off (without the bound this test
      does not fail, it hangs; verified by disabling the timeout and killing the run at 45 s), a
      finished one keeps its result, a real failure is not relabelled a timeout, and a slow-but-legal
      operation still succeeds.
- [x] **The GPU-vs-SIMD bench measured its own spread and printed only the mean.** `Stats` computed
      `min`/`max` and reported neither, so every row was a mean with no error bar — in the one bench
      the `GPU_THRESHOLD = 50_000` number comes from, where a crossover read off two means whose
      ranges overlap is not a crossover at all. Both tables and the fixed-overhead block now carry a
      spread column (range as a percentage of the mean — a ratio, because the question the column
      answers is "is this gap bigger than the noise?", which absolute durations across mixed
      magnitudes do not answer). `threshold_benchmark.rs`'s unused `format_duration_short` was
      genuinely dead — that bench already prints `mean ± stddev (min, max)` — and is deleted. The
      workspace now has **zero** dead-code warnings.
- [x] **`TursoTaskSource::workdir` was the last residue of a deliberately cut experiment.** Hard-coded
      to `None` by the only constructor and read by nothing, while its doc comment read as though
      ancestor-promotion existed and was merely switched off. The experiment was cut as
      benchmark-shaped (it converted the eval metric directly and would almost never fire in a chat
      workflow); the field is removed and the reason now lives at the surviving `clear_open_descendants`
      call, so nobody re-derives it from a suggestive field name.

**Architecture (all done, 2026-07-16):** decomposed `gui/src-tauri/src/lib.rs`, `control.rs`, `settings.vue`, and `main.rs` into per-domain modules; unified the embedded↔daemon agent loop onto `AgentService` (later removed wholesale by P16).

**Embedded-mode items — superseded by P16 (2026-07-18):** the GUI embedding-dimension probe, the silent daemon→embedded fallback, `recall`-broken-in-embedded, and "only three tools in embedded" are all closed by P16's deletion of embedded mode — the GUI is now a pure daemon client, a failed connect is an explicit `Disconnected`, and the daemon loads all 39 skills. The one real remainder — a **local offline embedder** — is a P12 deliverable ("Local embeddings in Burn"); the P11 soft-degrade (actionable `NoEmbeddingProvider`, non-error `recall` result) is done. Stale `9833` sidecar-port doc fixed to `5149`.

**Run-log triage (2026-07-18) — surfaced from a real daemon+GUI run log and fixed this pass:**
- [x] **Multi-tool-call streaming collapse** (OpenAI-compat / OpenRouter) — the agent stream accumulator kept one tool slot and ignored `ToolUseDelta.index`, so ≥2 tool calls per turn concatenated into one mis-attributed buffer (the healer salvaged the first, dropping the rest → the `read_file`/`code_search` "missing parameter" + empty-`exec` storm). Fixed: a per-index `StreamBlockAssembler` finalizes each block on its own `ContentBlockStop`; the OpenAI-compat + Ollama adapters emit stops in ascending index order; `nanna_llm::count_balanced_top_level_objects` flags any residual collapse. 6 attribution tests (fail on the old single-slot code) + 3 heal tests.
- [x] **Tool-stats import made tolerant** — `import_json` deserializes each entry individually (skip+warn on a bad one), backfills the tool name from the map key, and tolerates a scalar `sessions` (the boot `invalid type: integer 202, expected a map`), so one drifted field no longer wipes every model's stats. 4 tests.
- [x] **Corrupt Turso memories table — salvage + surfacing.** The fast single-scan `bulk_load` runs first; only on a corruption error (`is_corruption_error`) does `MemoryRepository::bulk_load_salvage` kick in — reading rowids first (that scan survives a corrupt overflow chain), then loading each row on its own and skipping only the unreadable ones instead of dropping the whole table on the first `?` (so a healthy store keeps its single query, no N+1). A `MemoryStoreHealth { degraded, corrupt_rows, .. }` is recorded on load — and on a whole-store load failure — and surfaced on `/status`, `/health`, and the IPC status action (previously a silent WARN + 0 memories that re-accumulated). Classifier + salvage-equivalence + health-mapping + degraded-on-failure + status-JSON tests. (Whole-btree repair/quarantine remains future work; needs a live corrupt fixture.)
- [x] **Tool-failure log carries the real error** — `result_log_preview` prefers `result.error` (empty for `ToolResult::error`), ending the blank `Tool exec failed in 1ms:` lines. 4 tests.
- [x] **Windows `exec` ergonomics** — `normalize_cmdisms` rewrites the exact cmd.exe idiom `cd /d <path>` → `cd <path>` (the "cd: too many arguments" failure) before Git-Bash routing; the `exec` description + system prompt steer to POSIX and to `code_search` over `rg`. 2 tests.
- [x] **Heartbeat** no longer commands the model to `Read HEARTBEAT.md` (which hard-errored on the missing `~/HEARTBEAT.md`); workspace `HEARTBEAT.md` is already injected via context. 2 tests. (Full retirement of the bespoke file is P17.)
- [x] Removed committed debris `gui/src-tauri/src/_patch.py`.

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
      - *(research 2026-08-24)* **0.21 is still the latest — no 0.22 exists**, so the `burn = "0.21"`
        pin above is current and needs no action this pass. Re-check on each freshness sweep. Remember
        this is **Mummu's** dependency, not Nanna's: nothing here should pull `burn` into this tree.
        Source: [tracel-ai/burn releases](https://github.com/tracel-ai/burn/releases).
      - *(research 2026-08-25)* **Re-checked; unchanged — `burn` 0.21 is still the newest release.** No
        action, and recorded so the next sweep does not spend a search on it: check the releases page,
        not crates.io's "updated" date, which moves on sub-crate republishes.
- [ ] **One binary, dual backend, runtime probe** — compile BOTH `Wgpu` (Vulkan/DX12/Metal, no CUDA toolchain) and `NdArray` CPU; a cheap `wgpu::Instance::enumerate_adapters` probe (cached in `OnceCell`) picks GPU if present, else CPU. No feature-split builds. (laurelane `use_gpu()` pattern.)
- [ ] **First model: a Hermes-class function-calling small model** — a from-scratch Burn decoder (start from laurelane's Qwen2.5 / LFM2 modules: RmsNorm + GQA + RoPE + SwiGLU, tied lm_head) sized for one GPU (1.5–3B). Prove tool-calling quality is good enough to run the loop.
      - [ ] *(research 2026-07-06)* Evaluate **Qwen 3.5-9B** as the default single-GPU function-calling model — 2026 consensus "sweet spot" (fits ~8GB VRAM, strong tool-call reliability, GGUF Q4 doesn't degrade tool calls). Sources: [insiderllm](https://insiderllm.com/guides/function-calling-local-llms/), [unsloth tool-calling guide](https://unsloth.ai/docs/basics/tool-calling-guide-for-local-llms).
      - [ ] *(research 2026-07-09)* Newer 2026 recommendation for the 8GB tier: **Qwen3-Coder-Next** — an 80B **MoE with only ~3B active params**, so it decodes fast (~40–60 tok/s on a 4090) yet runs Q4 on 8GB+ VRAM, and is now rated best-in-class for *long-horizon tool use + recovery from failed tool calls* (llama.cpp fixed its tool-call parser). Note the MoE/active-param split ties directly to the P12 **`--cpu-moe` expert-offload** and VRAM-budgeting items — the same architecture Nanna's local tier wants. This should become the reference default the Mummu runner targets and the `[infer]` model config points at. Sources: [unsloth Qwen3-Coder-Next](https://unsloth.ai/docs/models/qwen3-coder-next), [running 30B on 8GB VRAM](https://dev.to/upayanghosh/from-oom-to-262k-context-running-qwen3-coder-30b-locally-on-8gb-vram-1ej1).
      - [ ] *(research 2026-07-07)* Per-tier default: **8GB → Qwen 3.5-9B**, **16GB → Qwen 3.6-35B-A3B with `--cpu-moe`** (MoE expert offload — ties to the VRAM-budgeting item), **24GB → Qwen 3.6-27B dense or 35B-A3B**. Local ~7–9B models **lose coherence after 2–3 tool-chain steps** → bias toward short loops + sub-agent decomposition for the local tier (revisit the iteration cap / swarm hand-off for local models). Sources: [sitepoint 2026](https://www.sitepoint.com/best-local-llm-models-2026/), [insiderllm function-calling](https://insiderllm.com/guides/function-calling-local-llms/).
      - [ ] *(research 2026-07-12)* **Qwen3.5 GGUF ships universal chat-template fixes for tool-calling** (apply to *any* Qwen3.5 GGUF), and the Qwen3-Coder tool-call parser is now fixed across llama.cpp/Ollama/LMStudio/Jan — de-risks the "reliable tool-call parsing into `ContentBlock::ToolUse`" item for the local tier. When Mummu ports a Qwen3.5-class model, lift its chat template + tool-call grammar verbatim rather than hand-rolling. 8GB tier still wants Q4_K_S/Q4_0 (drop to Q3_K_M on OOM); Qwen3-Coder-Next's ~46GB Q4 footprint keeps it a 16GB+/CPU-offload target, not an 8GB one. Sources: [unsloth Qwen3.5](https://unsloth.ai/docs/models/qwen3.5), [Qwen3.6 VRAM table](https://knightli.com/en/2026/05/01/qwen3-6-local-vram-quantization-table/).
      - [ ] *(research 2026-08-25)* **Qwen 3.8-27B landed 2026-08-05 (Apache 2.0, 64 layers, 262K
            context; community GGUFs 08-13/08-14) — and on independently measured tasks it is *level with
            3.6*, not ahead of it.** That is the actionable half: a newer number is not a reason to re-point
            the default. This repo's governing metric is task success @ budget, and a level model at a larger
            size is a regression in capability density. Concretely: **do not move the 8 GB default off
            Qwen3.5-9B on release-date alone**; 27B dense is a 24 GB-tier candidate at best and belongs in the
            same eval as the existing tier list, decided on **tool-call validity rate**, not on a leaderboard.
            Worth noting for Mummu's port ordering only if the 24 GB tier is being worked. Sources:
            [Best Qwen models to run locally, mid-2026](https://insiderllm.com/guides/qwen-models-guide/),
            [Qwen3-Coder-Next](https://qwen.ai/blog?id=qwen3-coder-next).
      - [ ] *(research 2026-07-24)* **Qwen3.5's *Small* series gives the sub-8 GB tiers a real ladder, not just
            a quantization knob.** The family now spans **0.8B / 2B / 4B / 9B** alongside the big MoEs, all
            with **256K context** and tool-calling, so the CPU-only and low-VRAM guardrail tiers can drop to a
            *smaller model at a better quantization* instead of crushing the 9B to Q3_K_M (which is where
            tool-call validity starts failing). Concretely: keep **9B** as the 8 GB default, and add **4B** as
            the CPU-only/offline fallback and **2B** as the floor for the low-VRAM guardrail run. Do not treat
            the sizes as interchangeable on quality — the agent-eval suite's **tool-call validity rate** is the
            metric that decides the ladder, and per the routine's own governing metric a faster model that
            fails more tasks is not an improvement. This is a Mummu-side port ordering input, and the tier
            list the P14 "8 GB tier" eval item should enumerate. Sources:
            [unsloth Qwen3.5](https://unsloth.ai/docs/models/qwen3.5),
            [Qwen3.5-9B](https://huggingface.co/Qwen/Qwen3.5-9B).
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
      - [ ] *(research 2026-07-23)* **Re-confirmed, nothing moved: Qwen3.5-9B is still the 8 GB default, and
            Burn is still 0.21.** Two checks worth recording because they *prevent* churn rather than cause it.
            (1) 2026 round-ups still rate **Qwen3.5-9B the best 8 GB function-calling pick "by a significant
            margin"**, measured at **~55–58 tok/s flat across context sizes up to 16K** and fully GPU-resident
            at all tested sizes through 32K — so the reference default in the notes above stands, and the
            text-only-GGUF caveat (2026-07-13) is what keeps it fitting. **LFM2.5-8B-A1B** remains the verified
            alternative for the 8–12 GB tier. Newer **Qwen 3.6 / Gemma 4** are now named in function-calling
            guides with better edge-case handling (nested JSON args, missing-parameter errors, and correctly
            choosing *not* to call a tool), but **no public BFCL-style numbers for 3.6 exist yet** — do not
            switch the reference default on vibes; wait for a benchmark or run our own. (2) **Burn has still not
            shipped 0.22** — 0.21.0 remains latest, so every 0.21 note above (no KV-cache API, `burn-lm` still
            alpha, `attention()`/flash-attention additions, `TensorData::shape` breakage) is current and the
            Mummu contract needs no revision this run. Sources:
            [localllm.in 8 GB benchmarks](https://localllm.in/blog/best-local-llms-8gb-vram-2025),
            [InsiderLLM function-calling guide](https://insiderllm.com/guides/function-calling-local-llms/),
            [Burn releases](https://github.com/tracel-ai/burn/releases).
      - [ ] *(research 2026-07-06)* Investigate **MoE + expert CPU-offload** (`--cpu-moe`-style) so a larger agentic model (e.g. Qwen 3.6-A3B) fits a 16GB card — relevant to the single-GPU VRAM budgeting item. Also note the model-specific tool-call parser pattern (Qwen ships `qwen3_coder`) for reliable parsing into `ContentBlock::ToolUse`.
- [ ] **Weight loading** — HF safetensors via `burn-store` `SafetensorsStore` + `PyTorchToBurnAdapter` + a `CastFloatAdapter` (bf16→f32/f16); checked load (fail on missing/unused keys). Stream weights from HF to a per-user model cache (resume `.part`, resources-dir first).
- [ ] **Tokenization + chat format** — HF `tokenizers` crate; ChatML (or the chosen model's) template built explicitly; correct special/EOS tokens.
- [ ] **Fast decode** — per-layer KV cache (+ conv-state cache for hybrid models like LFM2); on-device `argmax` so only the winning index syncs to CPU; sampling (temp/top-p) beyond greedy; **streaming tokens** to Tauri events + channels; cooperative interrupt check between tokens (cancellation).
- [ ] **Single-GPU VRAM budgeting** — a size-tier picker (larger model on GPU, smaller on CPU) and an opt-in **f16** path (`Wgpu<half::f16, i32>`) to ~halve VRAM; account for KV cache + display headroom (3B f32 ~12GB is tight on 16GB).
- [ ] **Local embeddings** — a from-scratch MiniLM-class sentence-embedder in Burn (ndarray/CPU) to serve the memory `embed_fn` fully offline (replaces the API `EmbeddingClient` on the local path). Fixes the "no local embeddings" gap.
      - [ ] *(research 2026-07-18)* **MiniLM may be an outdated target — evaluate a 2026 on-device embedder
            instead.** Concrete candidates, smallest-first: **Nomic Embed v2 (137M, CPU-friendly, best
            quality-to-size)**; **EmbeddingGemma-300M** (Google, derived from Gemma 3, runs <200 MB quantized,
            ~22 ms/embed on EdgeTPU, strong multilingual + MTEB-Code 68.76 — a natural fit since Mummu will
            already port Gemma/Qwen-class decoders, so the tokenizer/weight-loading path is shared); and
            **Qwen3-Embedding-0.6B** (matryoshka dims, 100+ languages incl. code, pairs with the Qwen3.5
            generation tier). Decision inputs: pick by (a) whether Mummu can reuse the model's decoder blocks,
            (b) output dimension vs the memory store's dimension-agnostic path (already handled by
            `probe_and_align_dimension`), (c) CPU decode latency for the dreaming `embed_fn` batch. This is
            the real fix for the P11 "recall broken in embedded mode / no local embedder" gap. Sources:
            [EmbeddingGemma](https://www.bentoml.com/blog/a-guide-to-open-source-embedding-models),
            [Ollama embedding models 2026](https://www.morphllm.com/ollama-embedding-models).
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
- [x] **Unify the two stacks** — the running app calls low-level `MemoryService::consolidate()` while the richer `DreamingService`/`nanna-core::DreamingRuntime` (feedback, gates, promote/demote) is dead code. Make `DreamingService` the single orchestrator via `create_dreaming_executor`; delete the GUI branch (`lib.rs:8462`) + daemon `MemoryAction::Consolidate` duplication.
      *(2026-07-23)* **Done — `DreamingService` is now the daemon's single dreaming orchestrator, and it is
      no longer dead code.** The daemon built one `Arc<DreamingService>` at boot (over the live shared
      store) and hands it to **both** consolidation paths: the scheduled cycle and the IPC
      `MemoryAction::Consolidate` handler. That closes a real behavioural gap, not just a structural one —
      both paths previously called `MemoryService::consolidate()` directly, i.e. they ran **only the
      clustering phase** and silently skipped the cycle's first three: pending-feedback
      promote/demote, the **testing-effect FSRS flush** (`apply_pending_updates`), and the
      `min_memories_for_consolidation` floor. Those now run on every dream.
      **One clock, not two.** `ActivityClock` moved from `nanna-daemon` to `nanna-memory` (beside the
      dreaming code that reads it; the daemon re-exports it) and `DreamingService` gates on an injected
      `Arc<ActivityClock>` instead of a private `RwLock<Instant>` — so the service reads *exactly* the
      clock the control plane stamps on each chat, with no second bookkeeping call to drift. Side effect:
      `record_activity`/`idle_duration` are now lock-free and non-`async`, so the hot request path never
      takes a lock.
      **Per-run consolidation config.** The cluster byte budget must be sized to the *summarizer model's*
      context window, which only the router knows at fire time — so a long-lived service must not freeze
      one at construction. Added `dream_with(&ConsolidationConfig, ..)` / `dream_if_idle_with(..)` as the
      single implementations; the old `dream`/`dream_if_idle` delegate with the service's own config.
      `dream_if_idle*` now returns `Option<DreamOutcome>` (trigger + stats) so a caller can log *why* a
      cycle ran; a skip stays `Ok(None)`, so the type cannot represent "ran because it didn't".
      The IPC path deliberately uses the **ungated** `dream_with` (the user asked for it, so the idle gate
      must not veto) and falls back to the low-level call when no orchestrator is attached, so this can
      never regress a minimal construction. The ~85-line inline scheduler block became a bounded
      `run_scheduled_dream_cycle(..)`.
      6 new tests (host-side clock opens/shuts the gate without `record_activity`; outcome reports
      `MemoryPressure` vs `Idle`; per-run config overrides the construction-time one; daemon-side
      same-`Arc` clock invariant) — the fixture bug they caught is worth noting: the old dim-3 test
      embedder returned one vector for every text, so `remember` deduped everything into a single memory;
      a `distinct_embed_fn` with pairwise-cosine ≤ 0 directions was needed to store them separately.
      70 nanna-memory + 61 nanna-daemon tests green.
      Still open (their own items): the multi-phase dream **body** (true merge / cluster-by-band / expand /
      DSP timeline), and nothing yet calls `record_feedback`, so the feedback phase is wired but unfed.
      *(merge note 2026-07-23)* A parallel, independently-built implementation of this item arrived on
      the nightly branch the same day; the merge kept this landed design and salvaged the parallel
      run's genuinely-additive pieces: the failover dream summarizer
      (`crates/nanna-daemon/src/dream_summarizer.rs` — the scheduled cycle now walks the full
      `summarization_priority` list instead of only its head), the extra `DreamingService` entry
      points (`dream_if_triggered`, `dream_with_consolidation`), and their tests.
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
      *(2026-07-19)* **Idle gate now WIRED into the daemon scheduler** (closes the "trigger exists but nothing
      calls it" half). The scheduled `memory_consolidation` task ran `MemoryService::consolidate()`
      **unconditionally every hour** regardless of activity — the shipped `dream_if_idle` gate was dead code
      from the daemon's view. Now: a lock-free monotonic `ActivityClock` (`nanna-daemon::activity`, 8-byte
      `AtomicU64` from a base `Instant`) is stamped by the control plane on **every `Action::Chat`** (user +
      channel; status/log/config polls deliberately excluded so a 1 Hz GUI poll can't hold the gate shut), and
      the scheduled dream cycle gates on `nanna_memory::dream_trigger(clock.idle(), memory.count(), cfg)` — the
      **same pure policy** `DreamingService::dream_if_idle` uses (exported from `nanna-memory`, one source of
      truth, no drift). Skips with a `"Skipped (active; idle Ns, N memories)"` task result while in use; runs on
      `Idle`/`MemoryPressure`. Two config knobs (`[memory] dream_idle_threshold_secs`=300,
      `dream_memory_pressure_count`=5000) thread through `DaemonConfig` (both construction sites + `from_nanna_config`
      + legacy `serve.rs`). 4 `ActivityClock` tests (fresh≈0 idle, idle grows, record resets, shared-Arc monotonic)
      + a `DaemonConfig`-mirrors-`DreamingConfig` mapping test + the 3 existing `dream_trigger` tests still green;
      hermetic `e2e_daemon` (4/4) proves `DaemonServer::run()` boots with the new wiring. Remaining on this item:
      the multi-phase dream *body* (merge/cluster-by-band/expand/DSP) and unifying onto one `DreamingService`
      orchestrator (its own item) so the daemon dreams *through* it rather than the low-level `consolidate()`.
- [x] **Implement the missing true merge** — `IngestAction::Update` currently falls back to create/reinforce (`service.rs:300`); add content-level merge so dreaming deduplicates instead of accreting near-duplicates.
      *(2026-07-07) Done for **all three ingest paths** (`smart_ingest`, `remember_with_importance`,
      the scoped variant) via a shared `fold_into_memory` helper: `merge_memory_content` +
      `update_content_and_embedding` fold related-but-distinct content into the existing memory
      (bounded, superset-dedup) and reinforce FSRS. Next: apply the same merge in the batch
      dreaming/consolidation clusterer (`cluster_memories`), which still creates consolidated copies.*
      *(2026-07-23)* **Batch half done — dream phase (b) now folds true duplicates with no LLM call.**
      A dream cycle paid one summarizer prompt for *every* cluster, including clusters that were nothing
      but restatements of one fact ("user prefers dark mode", stored three times from three sessions).
      Paraphrasing those through a model is both wasted tokens — the scarcest resource on the single-GPU
      local tier — and **lossier** than a deterministic fold. New `MemoryService::fold_near_duplicates`
      runs per band *before* `cluster_memories`, so the summarizer only ever sees genuinely distinct
      content. Rules, each a guard rather than a heuristic: **scope is absolute** (reuses the clusterer's
      `same_scope`, so a fold can never leak or re-home across a workspace); **only the
      `IngestAction::Reinforce` band** (cosine > 0.92 — the project's *existing* same-fact line, reused
      rather than inventing a threshold); **never lose content** — `merge_memory_content` silently keeps
      `existing` when the append would breach its byte bound, so folding-then-removing would drop text;
      a fold is committed only when the merged content demonstrably still contains the incoming content,
      otherwise both memories go to the clusterer; **strongest survives** (band ranked by FSRS weight
      descending, so duplicates fold *into* the best-established memory, which inherits `max` importance
      and the summed access count, mirroring `create_consolidated_entry`); and **update-before-remove**,
      so a partial failure leaves a transient duplicate the next cycle re-folds, never a hole. Bounded by
      the cycle's `removal_budget` and the band size; the pairwise scan is the same O(N²) shape the
      clusterer already has, so no new complexity class. Declines entirely when no embedder is configured
      (a survivor that cannot be re-embedded would leave content and vector inconsistent). New
      `ConsolidationResult::memories_deduped` counts it separately from `memories_merged` so the
      token-free share of a cycle is visible, and it is surfaced on the IPC consolidate response.
      **Measured on the retention harness (`bench/BASELINE.md` Suite 3): compression 0.90 and recall
      retention 1.000 are UNCHANGED, while summarizer calls for that corpus went 6 → 0** (54 memories
      folded deterministically, `clusters_formed: 0`) — a pure token-budget win at identical quality.
      The harness assertion was corrected from `memories_merged > 0` to `merged + deduped > 0`: it exists
      to measure compression and recall, not which mechanism achieved them. 8 new tests (folds duplicates
      without paying the summarizer; budget respected; distinct content left alone; survivor inherits
      importance + access count; no-op without an embedder; plus 3 pure `find_duplicate_target` tests —
      Reinforce-band-only, never-cross-workspace, and degenerate embeddings (dimension mismatch / empty)
      match nothing rather than panicking). nanna-memory 75 tests green, full workspace suite green,
      clippy 2147 (−6 vs the 2153 origin baseline; zero warnings in the new code), zero new rustfmt
      violations.
      *(2026-07-23, same run)* **Extended to the `Detailed` band — dedup now runs in *every* band.** The
      band loop skipped `Detailed` (FSRS weight 0.8–1.0: the freshest, most important memories) entirely
      on the grounds that "no compression is needed" there. That reasoning is right for *summarization*
      — paraphrasing your best-established facts is exactly what you don't want — but wrong for
      *deduplication*, which is **lossless** by construction here (a fold is committed only when no
      content is dropped). So exact restatements of your most important facts were accumulating forever,
      and those memories were the ones with the most to lose. Phase (b) now runs before the `Detailed`
      check; the band is deduplicated and still never summarized. This also removes the highest-value
      memories from the drift-exposed path entirely (see the drift fixture below). 1 test asserting the
      exact contract: Detailed duplicates fold, `clusters_formed == 0`, and the summarizer is invoked
      **zero** times (counted with an `AtomicUsize`, not inferred). nanna-memory 78 tests green.
- [x] *(discovered 2026-07-23; captured 2026-07-25)* **STATED vs OBSERVED provenance is now genuinely
      recorded — `fact_type` is written, not just read, and the dangerous default is fixed.** Before this,
      `fact_type` appeared only in `control/memory.rs`, twice, both *reading* it with `.unwrap_or("stated")`
      — so every memory reported as user-stated regardless of origin, and `ExtractedMemory` had no
      provenance field at all. Now: a `MemoryProvenance { Stated, Observed }` enum
      (`nanna-agent::loop_runner`, re-exported) whose `Default` is **`Observed`, never `Stated`** and whose
      `from_label` only promotes an explicit case-insensitive "stated" (every other/absent/garbled label →
      `Observed`, so an unlabeled memory can never impersonate a user assertion). The extraction prompt now
      asks the model to classify each fact's provenance ("stated" = the user plainly said it; "observed" =
      inferred/derived; when unsure → observed) with a worked two-item example; `ExtractedMemoryRaw` parses
      the optional label and `filter_extracted_memories` maps it through `from_label`. Tool-result memories
      are hard-coded `Observed` (agent output is never a user statement). Both auto-store callbacks
      (`nanna-daemon/agent_service.rs` and `nanna-server/state.rs`) persist it into memory metadata under the
      `fact_type` key. The two display reads default absent provenance to **"unknown"** now — not "stated"
      — so legacy pre-provenance memories are shown as unknown, not falsely user-stated. 7 new unit tests
      (`from_label` only-"stated"-is-stated incl. "statedly"→observed; default is observed; `as_str`;
      `filter` carries + defaults provenance for stated/observed/garbage/missing; prompt asks for
      provenance). 150 nanna-agent + 99 nanna-daemon tests green; nanna-agent/daemon/server build clean, new
      code clippy- and fmt-clean. **Note (LLM-gated):** the *classification quality* rides on the extraction
      model and can't be asserted unattended — the plumbing + conservative defaults are what's proven here.
      This unblocks the drift mitigation: the verbatim-pin of user-stated memories can now be built on real
      provenance (its own item), and the survey's "user statement > agent inference" precedence finally has
      a real signal to run on.
- [x] **Harden `create_consolidated_entry` against NaN** — the FSRS-scalar merge used
      `max_by(|a,b| a.partial_cmp(b).unwrap())`, which **panics the dreaming cycle** if any stored
      `importance`/`storage_strength` is NaN.
      *(2026-07-09)* Replaced with a pure `max_finite_or(values, default)` that skips non-finite inputs
      (NaN/±inf) and falls back to the default when none are finite; added pre/postcondition assertions
      (non-empty cluster in, finite scalars out). 3 unit tests (NaN/inf skipped, max+sum semantics,
      NaN-cluster survives). Removes two prod-path `unwrap`s from the consolidation path.
- [ ] **Indexed clustering** — replace the O(N²) greedy single-pass `cluster_memories()` with HNSW/IVF candidate neighbors + connected-components/HDBSCAN over `composite_cluster_score`; scales past the ~50k in-RAM ceiling.
      - [ ] *(research 2026-07-24 — **corrects a load-bearing "fact"; read before picking an HNSW crate**)*
            **The `turso` crate we already pin ships native vector SQL functions.** Both this roadmap and the
            `daily-dev` Appendix C assert "Turso stores embeddings as f32 BLOBs and does **NO** vector search —
            cosine is entirely in RAM after `bulk_load`". That is **false for `turso 0.6.1`**, the exact pinned
            version. Verified by grepping the vendored crate source
            (`~/.cargo/registry/src/*/turso_core-0.6.1`), not from a blog: it registers
            `vector()`, `vector32`/`vector64`/`vector8`/`vector1bit`, `vector32_sparse`, `vector_extract`,
            `vector_concat`, `vector_slice`, and **four distance functions** —
            `vector_distance_cos` / `_dot` / `_l2` / `_jaccard`. So an **exact** k-NN can be pushed into SQL
            (`ORDER BY vector_distance_cos(embedding, ?) LIMIT k`) instead of `bulk_load`-ing every embedding
            into RAM — which is the actual cost driver behind the "~50k in-RAM ceiling", and it stays
            Turso-only + pure-Rust with **zero new dependencies**.
            **What is *not* there — do not overclaim:** there is **no dense ANN index**. `index_method/`
            contains only `backing_btree.rs`, `fts.rs`, and `toy_vector_sparse_ivf.rs` (the name is the
            crate's own), and there is **no `vector_top_k` and no DiskANN** — those belong to the older
            **libSQL** fork (`libsql_vector_idx`), a different engine, so the widely-cited "Turso brings
            native vector search" post does **not** describe this crate. Net: SQL-side exact distance is
            available now; approximate indexing still needs an external crate (the shortlist below stands).
            Sequence it that way — measure SQL-side exact k-NN against the current in-RAM SIMD scan on the
            `nanna-memory::retention` harness *first*, since it is exact (no recall trade to prove) and
            drops the RAM ceiling; only then decide whether ANN is still needed.
            - [ ] *(research 2026-08-26)* **`hnswlib-rs` is the ANN shape that actually fits a
                  Turso-only store**, if ANN is ever needed. It deliberately **decouples the graph
                  from vector storage** — the caller supplies a `VectorStore` keyed by `NodeId` and
                  vectors are fetched on demand — which is exactly this split: Turso keeps owning the
                  f32 BLOBs, the index owns only the graph, and nothing is mirrored into a second
                  store. Alternatives seen: `swarc`, `vicinity` (HNSW behind a feature), `hnsw_rs`.
                  **Do not schedule this on recall grounds.** Appendix C's measured figure is ~0.1us
                  per 768-dim SIMD cosine, linear — a full scan of 100k memories is ~10ms, so an
                  index earns nothing at today's corpus size. The real trigger is the **O(N^2)
                  clustering in dreaming**, not recall. Source:
                  [hnswlib-rs](https://crates.io/crates/hnswlib-rs).
            - [ ] *(research 2026-08-26)* **The FSRS default weight table is not FSRS-6's, despite
                  saying it is.** `crates/nanna-memory/src/fsrs.rs` is headed "Default FSRS-6
                  parameters", but `w0..w18` are FSRS-**5** values (`0.4072, 1.1829, 3.1262, 15.4722,
                  7.2102, 0.5316, ...` against FSRS-6's `0.212, 1.2931, 2.3065, 8.2956, 6.4133,
                  0.8334, ...`), and six of them — `w13, w14, w15, w17, w18, w19` — are **zeroed**,
                  which matches neither table (FSRS-5's are all non-zero).
                  **`w20 = 0.0658` is NOT in question and must not be "corrected" by this item:** it
                  is already justified in-tree by a retention-harness experiment (an 800-day-aged
                  corpus recalled 0/6 topics at `0.5` versus 6/6 at `0.0658`).
                  Not changed blind, because every weight feeds the decay of every stored memory and
                  the zeroed entries may well be deliberate — the short-term/same-day terms have no
                  meaning for a store whose "reviews" are recalls rather than study sessions. What is
                  wanted is the decision written down: either adopt FSRS-6's table with an A/B on the
                  `retention` harness exactly as `w20` got, or rename the constant and its doc to say
                  what the table actually is. Note the upstream wiki is internally inconsistent about
                  the decay INDEX (its prose says `w20 = 0.0658` while its own 21-entry array puts
                  `0.0658` at index 19 and `0.1542` at index 20) — settle that against `rs-fsrs`
                  source before touching anything. Source:
                  [FSRS algorithm wiki](https://github.com/open-spaced-repetition/awesome-fsrs/wiki/The-Algorithm).
            *(2026-07-24)* **Proven, not just read — `crates/nanna-storage/tests/vector_functions.rs`.**
            A registered SQL function is not a working one, and this decision is too load-bearing to rest
            on a source grep, so 3 tests now assert it end to end through the pinned dependency:
            `vector_distance_cos` returns **0** for identical, **1** for orthogonal and **2** for opposed
            vectors and is **scale-invariant** (`[1,2,3,4]` vs `[10,20,30,40]` → 0, the property that makes
            cosine the right metric for embeddings); `ORDER BY vector_distance_cos(embedding, ?)` over a
            real table **ranks correctly** (the far row is the query's opposite, so a constant-returning or
            rowid-ordered kernel could not fake the result); and a stored vector **round-trips** through
            `vector_extract` at full dimensionality, which matters because clustering and dedup still need
            the raw vector back. 3/3 green, clippy clean. The remaining work is the measurement, not the
            feasibility. The corresponding false claim in the `daily-dev` Appendix C is fixed in the same
            commit.
            *(2026-07-25)* **The k-NN primitive now exists on the real column — `MemoryRepository::search_by_embedding_sql`
            + `crates/nanna-storage/tests/vector_knn.rs`.** The load-bearing question a source-read left open was
            the **on-disk format**: memories store embeddings as *raw little-endian f32 bytes*
            (`f.to_le_bytes()`), while turso's `vector_extract` reads a **trailing type byte**
            (`blob[len-1]`). It turned out `vector_distance_cos`'s parse path (`Vector::from_slice`) does
            **not** use that type byte — it treats a bare BLOB as `Float32Dense` — so cosine works over the
            stored column **directly, with no `vector32()` wrapper, no type byte, and no migration** (proven:
            identical→0, orthogonal→1, opposite→2, correct `ORDER BY` rank; the query vector binds the same
            way as a raw blob). The new method runs `ORDER BY vector_distance_cos(embedding, ?) LIMIT ?`
            inside Turso (O(N) computes, **O(1) RAM** — streams rows instead of `bulk_load`-ing every vector),
            mirrors `recall_scoped` scope (`Some`→workspace+global, `None`→all), and — since
            `vector_distance_cos` **errors** on a dimension mismatch (would abort the whole scan on one
            stray-dim vector) — guards with `octet_length(embedding) = dims*4` so mixed-dimension stores skip
            rather than fail. 4 tests: **ranking parity with an independent in-RAM cosine scan**, LIMIT +
            NULL-skip, the dimension guard, and workspace scope. 82 nanna-storage tests green, new code
            clippy/fmt-clean.
            *(2026-07-25, ranking parity extended to realistic data)* `crates/nanna-daemon/tests/sql_knn_retention.rs`
            takes the memory crate's `RetentionCorpus` (8 topic centroids × 12 jittered members, 64-dim —
            the same generator the recall harness uses), persists it to Turso, and asserts for every centroid
            probe that SQL k-NN's nearest neighbour is the **same memory** an independent in-RAM cosine scan
            picks **and** is in the probe's own topic cluster. So exact SQL k-NN is a faithful drop-in for the
            in-RAM scan on realistic embeddings, not just a hand-built spread. **Still the remaining work:** the
            *latency/RAM comparison* (wall-clock trade; needs the not-yet-built `nanna-bench` harness, release
            profile), and the **decision to wire it into the live recall path** vs the current `bulk_load`+SIMD
            scan — only after that measurement does the ANN-crate question reopen.
      - [x] *(2026-07-25)* **`MemoryRepository::delete`/`bulk_delete` now destroy the embedding on disk — the
            "today, before any HNSW" half of Ghost Vectors is closed.** Proven, not assumed: the negative
            control test (`raw_delete_leaves_embedding_on_disk`) confirms a plain `DELETE` **does** leave the
            f32 BLOB recoverable — and the diagnosis went deeper than the SQLite folklore: **turso 0.6.1 has no
            `secure_delete` pragma and no `VACUUM` (its `auto_vacuum` is a documented no-op), and it keeps
            *all* committed data in the `-wal` file (main `.db` stayed at one 4096-byte header page after an
            insert+close; it does not checkpoint on close).** So the embedding lived verbatim in the WAL, and a
            plain overwrite alone failed — the zeroed frame merely sits *behind* the original in the same
            growing WAL. Two steps close it: (1) overwrite `embedding`/`content`/`metadata` with **same-length
            `zeroblob`s** (`octet_length` → identical record size → the newest page frame carries zeros);
            (2) `PRAGMA wal_checkpoint(TRUNCATE)` — the one mode that both collapses the WAL into the main file
            (latest zeroed page wins) **and truncates the WAL to 0**, discarding the stale pre-overwrite frame
            (empirically: `FULL`/`RESTART`/passive do **not** truncate; `TRUNCATE` took the WAL 2.6 MB → 0).
            `bulk_delete` checkpoints **once** for the whole batch (the dream-cycle fold path), not per row.
            *(2026-07-25, follow-on)* **The dream cycle now actually takes that batch path — the per-delete
            checkpoint regression is closed.** The secure-delete checkpoint made each single `delete` fsync, but
            consolidation removed cluster members and folded duplicates **one at a time**
            (`consolidate_cluster`'s loop and `commit_duplicate_fold`), so a dream cycle would have fsynced once
            per removed memory. Added `MemoryPersistence::remove_entries` (default loops `remove_entry`; the
            Turso impl overrides to `bulk_delete` → one checkpoint) and `VectorStore::remove_many` (one
            entries-write-lock + one persistence call). `consolidate_cluster` removes the whole cluster in one
            `remove_many`; `fold_near_duplicates` defers every folded source to a single end-of-pass
            `remove_many` (`commit_duplicate_fold` no longer removes — update-before-remove still holds at the
            pass level, since folded sources are already absent from the in-pass `survivors` list). 2 new tests
            (one batched persistence call for N ids, not N single calls; empty batch is a no-op); the 8 existing
            fold/consolidation invariant tests stay green (88 nanna-memory + 99 nanna-daemon).
            Best-effort by contract: a `busy` checkpoint (concurrent reader) leaves the stale frame for a later
            one; the row is already deleted + overwritten. 4 tests in `crates/nanna-storage/tests/secure_delete.rs`
            (single-delete embedding absent-after / present-before; bulk 3-embedding; in-memory graceful; raw
            control present-after). 72 nanna-storage + 86 nanna-memory + nanna-daemon tests green; new code
            clippy-clean. Still open: **(i)** ~~unbounded WAL~~ **(corrected 2026-07-25 — the WAL is bounded)**:
            a follow-up probe (12k inserts) showed turso **does** auto-checkpoint at the standard ~4 MB /
            1000-page threshold and truncates the `-wal` file back down (4.1 MB → 41 KB), so there is no
            growth bug — the "no checkpoint on close" observation only meant a *single* sub-threshold insert
            stays in the WAL until the threshold or an explicit checkpoint. Note the auto-checkpoint is
            **passive**: it copies the embedding page into the main file *unzeroed* before resetting the WAL,
            which is exactly why the overwrite + explicit `TRUNCATE` above is still required (a plain delete
            that races an auto-checkpoint leaves the ghost in the main `.db`, not just the WAL);
            **(ii)** the stronger backup/epoch threat (a `-wal` copied *before* the truncate, or the `busy`
            path) still wants the paper's **epoch key rotation** (encrypt vectors, discard key on delete);
            **(iii)** the HNSW-tombstone constraint below still stands for when an ANN index lands; **(iv)** wire
            this into the P0.2 `PRIVACY.md` "how to delete your data" claim.
      - [ ] *(research 2026-07-24)* **Deleting a memory must actually destroy its embedding — "Ghost Vectors"
            (arXiv [2606.18497](https://arxiv.org/abs/2606.18497)).** Embeddings soft-deleted/tombstoned in an
            HNSW store stay **physically present in the raw index files** and are invertible back to their
            source text with a Vec2Text-class model: the authors report **25.5%** exact recovery of person
            names and **100%** recovery of sensitive structured fields. The attack needs storage-layer access
            (the file on disk), **not** API access — which is precisely the threat model of a local-first
            product whose whole database sits in the user's filesystem and gets backed up. Two consequences
            for us: **(a)** it is a hard constraint on the HNSW adoption above — `hnswlib-rs`'s `delete(key)`
            *tombstones* (keeps the key mapping), so "delete this memory" through an ANN index would be a
            privacy lie unless deletion also rewrites the index; **(b)** it applies to what ships **today**,
            before any HNSW: Turso/SQLite frees a deleted row's pages to the free-list without zeroing them,
            so a deleted memory's f32 BLOB survives on disk until overwritten. Verify what `delete_memory`
            actually leaves behind, then close it — cheapest honest fix is `VACUUM` (or `secure_delete`) on
            the delete path; the paper's own mitigation is **epoch key rotation** (encrypt vectors, discard
            the key on delete — 0% recovery, ~2.5 ms per 500 records, plus a signed proof-of-deletion). Pairs
            with the P0.2 `PRIVACY.md` "how to delete your data" claim, which must not promise more than the
            storage layer delivers.
            - [ ] *(research 2026-07-25 — raises the stakes on the epoch-key follow-up above)* **Embedding
                  inversion got cheaper and no longer needs the encoder.** Beyond Vec2Text (per-encoder,
                  query-access), 2025–26 work makes recovery practical from *just the stored vectors*: **ALGEN**
                  learns a linear map between embedding spaces from ~10³ leaked pairs and hits Vec2Text-level
                  recovery **without query access**; **ZSINVERT** is zero-shot (no encoder-specific training);
                  **Zero2Text** does online token-by-token regression. For our threat model (a `.db`/backup on
                  the user's disk) this means a leaked embedding BLOB is invertible by an attacker who never
                  touched our model — so the today-fix (secure-delete overwrite, done 2026-07-25) closes the
                  *deletion* leak but a *live* embedding at rest is still recoverable. The durable answer is
                  **encrypt embeddings at rest** (the epoch-key-rotation mitigation), NOT retrieval-time
                  perturbation (Laplace/Purkayastha/Bound-Aware Perturbation harm recall and don't help a
                  storage-access adversary). Sources:
                  [ALGEN/ZSINVERT survey](https://arxiv.org/pdf/2504.00147),
                  [Concept-Aware defenses](https://arxiv.org/html/2602.07090v1).
      - [ ] *(research 2026-07-23)* **Three pure-Rust HNSW crates to choose between** (all no-C, matching the
            dependency doctrine): **`hnswlib-rs`** decouples the graph from vector storage (`Hnsw<K, M>` owns the
            graph + an external-key→`NodeId` map, you supply a `VectorStore`) and supports **concurrent search
            *and* concurrent mutation** with lock-free reads — the best structural fit, since Nanna's vectors
            already live in the in-RAM store after `bulk_load` and dreaming mutates while the agent searches;
            **`hnsw_rs`** offers serde reload-of-graph-only and **filtered search** (predicate applied *during*
            traversal, not as a post-filter) — directly relevant because our searches are **workspace-scoped**,
            and post-filtering an ANN result set silently under-returns; **`instant-distance`** is the smallest
            and most battle-tested (powers InstantDomainSearch) but is the least featureful. Decision inputs:
            (a) does the crate let clustering reuse one index across a dream cycle, (b) is scope-filtering
            in-traversal, (c) recall@k vs the current exact SIMD scan on the retention harness — gate the swap
            on `nanna-memory::retention` holding `recall_retention`, since an ANN index trades exactness for
            speed and that trade must be *measured*, not assumed. Sources:
            [hnswlib-rs](https://crates.io/crates/hnswlib-rs), [hnsw_rs](https://crates.io/crates/hnsw_rs),
            [instant-distance](https://crates.io/crates/instant-distance).
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
      - [ ] *(research 2026-08-25 — re-check; the shortlist is stable, and a fourth candidate appeared)*
            The three-way decision below has not moved: `hnsw_rs` and `hnswlib-rs` are both still live and
            still differ on exactly the axis that matters here (in-traversal filtering + parallel batch build
            vs. lock-free concurrent read/mutate), and `instant-distance` is still the one to rule out. Two
            additions worth a look before building anything:
            **`small-world-rs`** (HNSW with cosine *and* euclidean, serde persistence) and **`swarc`** —
            notable because it advertises **`remove`** as a first-class operation. That is not a nice-to-have
            for us: the "Ghost Vectors" item above says a deleted memory must actually destroy its embedding,
            and HNSW's classic weakness is that deletion is a tombstone, not a removal. If `swarc`'s remove is
            a real graph repair rather than a mark, it collapses two open items into one dependency choice.
            **Decide the deletion semantics before the crate**, or the crate decides them for us. Sources:
            [small-world-rs](https://crates.io/crates/small-world-rs), [swarc](https://crates.io/crates/swarc),
            [hnswlib-rs](https://lib.rs/crates/hnswlib-rs), [hnsw_rs](https://docs.rs/hnsw_rs/latest/hnsw_rs/).
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
      - [ ] *(research 2026-07-25 — settles "wait for turso native ANN vs adopt a crate")* **Native ANN in the
            `turso` crate is NOT coming soon — the external HNSW plan stands.** The often-cited "Turso brings
            native vector search / DiskANN" posts describe the **libSQL** engine (a different codebase); in the
            pure-Rust rewrite we actually pin, DiskANN is [issue #832](https://github.com/tursodatabase/turso/issues/832)
            — **Backlog milestone, no assignee, no branch, no PR** as of this check (the proposal is literally
            "port the libSQL DiskANN C code to Rust"). A parallel discussion
            ([#3778](https://github.com/tursodatabase/turso/issues/3778)) argues turso should ship
            **SIMD brute-force first**, before any ANN index. Net for us: the exact SQL k-NN primitive landed
            2026-07-25 (`search_by_embedding_sql`) is the near-term path for the RAM-ceiling win, and an
            **external pure-Rust HNSW crate** (`hnsw_rs`/`hnswlib-rs` above) remains the only route to
            *approximate* indexing — do not block on a turso release for it.
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
      - [ ] *(research 2026-07-17)* **Don't hand-roll the w0..=w20 fit — `fsrs-rs` already ships the optimizer.**
            Now that the default `w20` is the correct FSRS-6 value (fixed 2026-07-17), the eventual "learn the
            params from history" step has a ready tool: `fsrs-rs` (6.6.x, 2026-06) exposes
            `FSRS::compute_parameters(ComputeParametersInput) -> Result<Parameters>`, fed a `Vec<FSRSItem>` where
            each `FSRSItem` is a review vector of `FSRSReview { rating, delta_t }`. Our `FsrsState.access_count` +
            the testing-effect `record_access` history is exactly that review stream (map `Rating`→FSRS rating,
            elapsed-days→`delta_t`); persist per-memory review logs, batch them, call `compute_parameters` during a
            dream cycle, and replace `FsrsParameters::default()` with the fitted set. Caveat: `fsrs-rs`'s trainer is
            **Burn-backed** (per the crate's "full training support using Burn" description) — pulling it in adds
            Burn to `nanna-memory`'s tree, so gate adoption on whether the P12/Mummu Burn stack is already a
            workspace dependency by then (don't add a second heavy ML dep just for this). Validate any fitted set
            through the retention harness before it becomes the default, same gate the w20 flip used. Sources:
            [fsrs-rs](https://github.com/open-spaced-repetition/fsrs-rs), [fsrs crate](https://crates.io/crates/fsrs).
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
      - [x] *(2026-07-17)* **Measured, then flipped — `FsrsParameters::default().w20` is now `0.0658`.**
            `nanna-memory::retention::measure_gated_recall` measures recall through the FSRS-gated
            `MemoryService::recall` path (the one that drops memories whose `weight = retrievability × importance`
            is below `min_weight`), so it is `w20`-sensitive unlike raw vector recall. The `w20_experiment_aged_recall`
            test replays one aged corpus (800 days, uniform importance, `stability = 1`) under both exponents:
            **`w20 = 0.5` recalls 0/6 topics** (every valid memory decays below the weight gate and vanishes) while
            **`w20 = 0.0658` recalls 6/6** — the "recalls better" proof the flip was gated on. With that evidence
            the default was flipped `0.5 → 0.0658` (the correct FSRS-6 value; `0.5` was FSRS-4.5/5's `DECAY`
            mispaired with the FSRS-6 curve, decaying ~7.6x too fast). Blast radius verified contained: the only
            w20-sensitive tests are `fsrs.rs` (monotonic decay / literal-accessibility state / stability updates —
            all w20-agnostic) and the retention consolidation test (re-baselined — under slower decay a corpus must
            age past a year and hold uniform importance to reach a compressible band; still 60→6, recall 1.0→1.0).
            nanna-memory 53 / nanna-agent 61 / nanna-core 23 / nanna-daemon 54 tests green. Remaining: *fit*
            `w0..=w20` from access history instead of any static default (the eventual FSRS-6 trainable goal).
- [~] **Local dreaming** — run `summarize_fn` on the selected sumarization model + fallback from the users settings; persist the `SummaryCache` (currently in-memory, lost on restart).
      *(2026-07-23)* **Model-selection + fallback half shipped.** The two dream paths disagreed: the IPC
      `MemoryAction::Consolidate` already walked the whole `summarization_priority` with failover, while the
      **scheduled** cycle took only `summarization_priority.first()` and made a **single attempt** — so one
      unavailable / rate-limited / out-of-credit model killed the entire nightly dream cycle, silently, until
      someone read the task result. New `nanna-daemon::dream_summarizer` is the one implementation both paths
      now share: pure `summarization_models(priority, fallback)` (priority wins verbatim; falls back to the
      agent's main model for the scheduled path and to `llm.model_priority` for IPC — taken as a slice
      because the two callers legitimately differ; empty only when *both* inputs are empty, which each caller
      still reports as unconfigured) and `summarize_with_failover(router, models)` which walks the list,
      treats a per-model failure as the expected operational condition it is (warn + continue) and only errors
      after exhausting every candidate, naming the last real failure. **Also fixes a latent overflow:** both
      paths sized the cluster byte budget to the **first** model's `hard_input_limit`, but one prompt is built
      and then offered to each candidate in turn — so a budget fitted to a big first model would overflow a
      smaller fallback. `summarizer_context_window_tokens` now takes the **minimum** window across the whole
      failover list (floored at 8k), which is safe for whichever model actually answers; the only cost is
      slightly smaller clusters per pass, and no content is lost because an over-budget cluster simply
      re-clusters on a later seed. Deleted the hand-rolled summarize closures from `server.rs` and
      `control/memory.rs` (and their now-dead `RequestBuilder` imports). 4 pure unit tests (verbatim priority,
      both fallback shapes, priority-wins negative space, empty-only-when-nothing-configured); nanna-daemon
      72 tests green; full workspace suite green; clippy **2146** (−7 vs the 2153 baseline). Verified on a
      **real daemon boot**: the scheduled dream cycle *executed* through the new path and correctly skipped
      in 9 ms (a heartbeat run had just stamped the `ActivityClock`, so idle < 300 s) without touching an LLM.
      **Remaining:** the `SummaryCache` half of this item is **stale — no such type exists anywhere in the
      repo** (grep-clean); if a cross-restart summary cache is still wanted it needs designing from scratch
      (key on cluster content hash, persist to Turso), so it is re-logged as its own item below.
- [ ] **Persistent dream summary cache (was the `SummaryCache` half above).** No `SummaryCache` type exists —
      the original item referenced something never built. If worth doing: key on a content hash of the
      cluster's concatenation, store summary + model + timestamp in Turso, and reuse on a later cycle so a
      re-formed cluster doesn't re-pay the summarizer. Gate on measuring how often clusters actually recur.
- [x] *(research 2026-07-23)* **Summarization drift is the named failure mode of exactly what dreaming does —
      guard it before it costs us a safety-critical memory.** The 2026 agent-memory survey warns that repeated
      compression cycles make **low-frequency details vanish** — precisely the ones most likely to matter; its
      worked example is that after ~3 summary passes over a week, a rarely-mentioned instruction like
      "never call the production database directly" can disappear entirely. Nanna is squarely exposed:
      `consolidate()` re-summarizes surviving memories cycle after cycle and already tracks how many times via
      `FsrsState::generation`. Three concrete, in-reach mitigations, cheapest first: (a) **the deterministic
      dedup pass landed 2026-07-23 already removes the biggest drift source** — true restatements are now folded
      verbatim instead of paraphrased, so re-summarization no longer touches them at all; (b) **cap
      re-summarization** — refuse to consolidate a memory past a `generation` ceiling, or require a *higher*
      similarity to re-merge an already-consolidated entry, so gist-of-a-gist-of-a-gist cannot form; (c) **pin
      the un-drift-able** — mark high-importance/low-frequency memories (and anything user-STATED rather than
      agent-OBSERVED) as verbatim, excluded from summarizing clusters. Gate all of it on the retention harness
      with a **new drift fixture**: seed one rare-but-critical memory among many common ones, run N dream cycles,
      and assert it is still recallable *and* still contains its key clause — the harness already measures topic
      recall, so this is an added probe, not new machinery. Sources:
      [Memory for Autonomous LLM Agents (arXiv:2603.07670)](https://arxiv.org/html/2603.07670v1),
      [SSGM — governing evolving memory (arXiv:2603.11768)](https://arxiv.org/html/2603.11768v1).
      *(2026-07-23)* **Instrument built and both arms baselined — the mitigation is now a measured
      decision, not a guess** (same "measure first, then flip" discipline the `w20` change used).
      New `retention::clause_survives(service, clause)` asks whether any live memory still contains a
      clause **verbatim** — deliberately content fidelity, not recall, because the two come apart
      exactly here: in the failing arm the topic stays perfectly recallable while the clause that made
      it actionable is gone, which is why drift is easy to ship blind. Two fixtures share one corpus
      shape (an aged, compressible band of 8 memories where exactly one carries "never call the
      production database directly") and one summarizer, differing *only* in similarity spread, which
      selects the consolidation path: **summarized arm → clause LOST in a single cycle** (the exposure,
      reproduced against our own pipeline; `echo_summarize` models a real summarizer faithfully here —
      it keeps the dominant theme and drops what appears once, precisely the reported failure), and
      **deduplicated arm → clause SURVIVES verbatim while the store still compresses** — i.e. dream
      phase (b), landed this same run, is already a working drift mitigation for true restatements.
      Both rows committed to `bench/BASELINE.md`; the dedup arm is a correctness fixture that must never
      regress, while the summarized arm is a **baseline to beat** — it asserts the clause *is* lost, so
      whichever mitigation lands next (generation ceiling / verbatim-pinning STATED memories) will make
      that test fail loudly, and its message says to flip it. Remaining: implement (b) or (c) above.
      *(2026-08-21)* **Mitigation (c) shipped — user-STATED memories are pinned verbatim, and the
      dedup fold no longer launders provenance.** New pure `consolidation::is_verbatim_pinned(metadata)`:
      a memory whose `fact_type` says `stated` (the provenance the extraction path already writes from
      `MemoryProvenance::as_str`) is never handed to a summarizer. Provenance is the gate, deliberately
      **not** an importance threshold — categorical, no magic number, and conservative in the same
      direction `MemoryProvenance` itself is (missing/empty/unknown → not pinned, so an unlabeled memory
      cannot pin itself out of consolidation).
      **The split runs BEFORE the dedup fold, and that ordering is the actual bug fix.** A fold merges a
      source INTO a survivor and keeps the **survivor's** metadata, so a stated row folding into an
      observed one came back marked `observed` — a user assertion laundered into agent-observed content,
      which the next cycle was then free to paraphrase away. Two folds over disjoint partitions cannot do
      that, and cost strictly less than one fold over their union (|A|² + |B|² ≤ (|A|+|B|)²). Pinned rows
      still deduplicate *among themselves*, so a stated fact repeated across three sessions still
      collapses to one row and pinning cannot make the store grow without bound.
      Band-loop budget arithmetic extracted into `fold_and_charge` so the two partitions' folds cannot
      drift apart, with the losslessness postcondition asserted per partition.
      **Measured, not asserted:** 2 new fixtures in `retention`, both proven non-vacuous by re-running
      them with the split removed — the mitigation arm loses the clause, and the laundering arm names the
      row that stole it (`drift-1`, `fact_type: None`). The mitigation arm is the *same* corpus, spread
      and summarizer as the losing baseline arm; only the provenance differs. The baseline arm
      deliberately **stays at NO**: drift is real and unfixed for agent-*observed* content, and deleting
      the measurement that says so would be dishonest. 3 more unit tests cover the predicate's positive
      space, its negative space, and partition losslessness. 145 nanna-memory tests green, **0 net new
      clippy warnings** (166 = 166 vs the pre-change baseline for the crate), two `bench/BASELINE.md`
      rows added.
      *(2026-08-21, same run)* **Mitigation (b) shipped as well — and the research fold below is what
      made it derivable.** The item offered "a generation ceiling" and the obvious objection was that no
      ceiling is derivable: our own fixture loses a rare clause in ONE pass, so any N would be a chosen
      number. The 2026 consolidation literature answers it by forbidding the **class** instead of
      counting passes — compress a session, never re-compress a summary — which needs no number at all
      and maps exactly onto `FsrsState::generation` (already `max(sources) + 1` at every consolidation).
      An entry with `generation > 0` is partitioned out of the summarizing clusterer in every band,
      including `Expand` (re-expanding a gist would invent the detail it lost).
      **Exempt from the clusterer, not from the cycle:** gists still go through the lossless dedup fold,
      so two gists that restate each other still collapse and the store keeps compressing. `generation`
      is now monotone across a fold as well — absorbing a gist makes the survivor a gist-carrier — or a
      generation-1 row folding into a generation-0 one would launder itself back into the summarizer's
      input; that single line is proven load-bearing by removing it.
      3 new fixtures (the never-re-summarize arm, the still-folds guard, the fold-monotonicity test),
      the first and third non-vacuous by construction-removal; one `bench/BASELINE.md` row. 150
      nanna-memory tests green, 0 net new clippy warnings.
      **This item's remaining work is now (a)'s follow-through, not (b) or (c)**: the drift *instrument*
      only measures a rare clause. A drift budget over many cycles — how much of a corpus survives N
      dreams — would let the footprint cost of these two exemptions be stated as a number rather than
      as the reasoned trade it is today.
- [x] *(research 2026-08-21 — confirms the drift model and names the principled form of mitigation (b);
      IMPLEMENTED the same run, see the item above)*
      **"Compress a session, never re-compress a compressed summary" is the depth limit worth having.**
      The 2026 consolidation literature converges on three mitigations for summarization drift, and the
      third is the one that dissolves the "what number should the generation ceiling be?" problem: don't
      pick a ceiling, forbid the *class* — a summary may compress raw episodes, but a summary of a
      summary is never formed. That is a categorical rule with no magic number, the same shape as the
      provenance pin landed 2026-08-21, and it maps onto the `FsrsState::generation` field the store
      already carries (`generation == 0` may consolidate; `generation > 0` may not be a cluster member,
      only a cluster *seed* whose sources are raw). The other two: **extraction over summarization**
      (structured facts distort less than prose — our deterministic dedup fold is already this), and
      **keep the original episodic record in non-lossy cold storage** so a drifted gist is always
      recoverable — those two are what remain here:
      - [ ] **Extraction over summarization** for the clusterer's output, not just the fold's.
      - [ ] **Non-lossy cold storage of the pre-consolidation episodes**, so a gist that drifted anyway
            is recoverable rather than merely un-re-compressed.
      Sources: [Memory consolidation in long-running agents](https://zylos.ai/research/2026-04-20-memory-consolidation-ai-agents/),
      [SSGM (arXiv:2603.11768)](https://arxiv.org/html/2603.11768v1).
- [x] *(2026-08-21)* **The `Expand` band's instruction and its acceptance test contradicted each
      other, so the only enrichments that ever landed were the ones that disobeyed the prompt.**
      `expand_memory` borrowed `CompressionLevel::Expand`'s `summarization_prompt`, which is written
      for a **cluster** and says the result "should be no longer than the material it replaces" —
      while the code committed the result only when `expanded.len() > original.len()`. A model that
      followed the instruction was always rejected; a model that ignored it was always accepted.
      Fixed by giving the single-memory path its own prompt whose shape the caller can actually
      verify, and the only shape that cannot lose anything: **reproduce the memory verbatim, then
      add beneath it**. The guard is now `contains(original) && longer`, the same losslessness test
      the dedup fold already uses to decide a merge is safe to commit — so a model that rewrites
      instead of appending is declined and the memory is left untouched, rather than a high-weight
      memory being replaced by a paraphrase of itself on length evidence alone.
      Enrichment also raises `generation`, because the appended half is model-authored: the entry
      now carries generated text and must not be fed to the summarizer later, by the same rule that
      stops a summary being re-summarized.
      **The trade, stated plainly:** enrichment will fire less often than before, because it now has
      to be additive. That is the intended direction — the previous behaviour's acceptance criterion
      was "the model disobeyed", which is not evidence of anything — but if the firing rate turns out
      to matter, the predicate is one line and the prompt is one constant.
      3 new tests: the rewriting case is declined and the memory is unchanged, the additive case
      commits and marks the entry model-authored, and the prompt asks for exactly what the guard
      accepts (asserted against the borrowed cluster wording, so the two cannot silently diverge
      again).
- [x] *(2026-08-21)* **The `remember` tool could not produce a pinnable memory — the drift pin's
      biggest blind spot, found by following the feature to its other caller.** The extraction path
      writes `fact_type` from `MemoryProvenance`, but the `memory.store` service behind the
      `remember` TOOL wrote only the caller's `tags`, so a memory the user explicitly asked to keep
      carried no provenance at all and could never be pinned. `memory.store` and its `memory.embed`
      alias now stamp `fact_type` through `tags_with_provenance`, and `remember` gained a
      `provenance` parameter whose description says what claiming it costs.
      Deliberately **classified, not copied**: the value goes through
      `MemoryProvenance::from_label` — the same rule, one implementation — so only an explicit,
      case-insensitive `"stated"` pins, and an absent, empty or misspelt declaration degrades to
      `observed`. A `fact_type` already present in `tags` is honoured as the declaration but
      re-classified rather than trusted, so `tags: {fact_type: "STATED-ish"}` cannot smuggle a pin;
      an explicit `provenance` field wins over an inherited tag. 4 unit tests, half of them negative
      space.
- [x] *(2026-08-21)* **A consolidated memory could impersonate one of its sources, and corrupt a handle
      reassembly doing it.** Found by reading while landing the drift mitigations, not by a report.
      `create_consolidated_entry` merged the cluster's metadata **first-writer-wins**, so a summary
      inherited whichever source sorted first — including `source_id` and `chunk` (`"3/17"`). Those two
      are exactly what `assemble_handle_content` (`server.rs`) uses to rebuild the whole text behind a
      memory handle: it gathers every row sharing a `source_id` and orders them by `chunk`. A
      consolidated entry carrying both was therefore **spliced into the middle of a tool result the
      model was promised was stored verbatim** — and the rows it replaced are gone, so nothing else
      filled that slot. It also let a gist of five different tools' output claim `tool=exec`,
      `outcome=ok`, `target=./build.sh`.
      Two rules replace the merge, neither with a threshold in it: a **source locator** (`source_id`,
      `chunk`) is never inherited however unanimous the cluster — a summary has no position in anyone's
      byte stream — and every other key is inherited only when **every source that carries it agrees**,
      because unanimity is exactly the condition under which the claim survives the merge.
      Paired with the honesty half in `assemble_handle_content`: a reassembly that comes back short of
      the `i/N` the stub promised now says how many rows are missing, that a dream cycle most likely
      folded them, and that the artifact on disk is unaffected. Silence there was the same failure the
      function was written to end. 3 + 3 unit tests, including the negative space (a complete
      reassembly announces nothing; unmarked rows never claim a shortfall).
- [ ] *(research 2026-08-21 — sharpens the provenance work landed this run)* **Provenance-role collapse:
      a two-valued `fact_type` is the cheap version of what the literature calls typed memory.**
      *Mitigating Provenance-Role Collapse in Long-Term Agent Memory* (arXiv:2605.25869) reports that
      long-horizon stores conflate **who originally asserted something** with **what role that entity
      holds now**, and that the fix is to type the three dimensions separately at encode *and* retrieve
      time: provenance, current role, and the temporal marker of the transition. Nanna now has the first
      dimension only (`fact_type` = stated/observed, and it is finally load-bearing — it decides what a
      dream cycle may paraphrase). ~~(a) make `fact_type` survive every merge path~~ **done in the same
      run**: `create_consolidated_entry` merged metadata first-writer-wins, so a merged entry's
      provenance was whichever source happened to be ordered first — a rule that depends on iteration
      order is not a rule; it is now monotone (any stated source ⇒ stated result). Remaining rungs:
      (b) stamp the transition time so "the user said X, then later said not-X" is orderable
      rather than a pair of equally-true rows; (c) expose provenance as a **recall filter** so a caller
      can ask for user-stated facts only. Source:
      [arXiv:2605.25869](https://arxiv.org/pdf/2605.25869).
- [ ] *(research 2026-07-23)* **Dual-buffer / probation consolidation ("hot" buffer before long-term).** The
      same survey's recommended write path: a new memory lands in a **hot buffer** and is promoted to long-term
      storage only after a probation period and quality checks — **re-verification, deduplication, importance
      scoring** — a computational hippocampal→neocortical transfer. Nanna writes straight to the durable store
      today, so a one-off mis-extraction is permanent until a dream cycle happens to catch it. Our
      `smart_ingest` already does dedup + importance at write time, so the missing pieces are the **probation
      window** and the **promotion/eviction decision** (plus what overflow does). Fits the existing FSRS state
      machine — probation is arguably just a band. Source:
      [arXiv:2603.07670](https://arxiv.org/html/2603.07670v1).
- [ ] *(research 2026-07-23)* **Write-path canonicalization + provenance, and precedence rules for conflicts.**
      Recommended metadata per record: timestamp, **source**, task label, **confidence**; plus canonicalization
      that normalizes dates/names/quantities before storage so near-duplicates actually compare equal. Conflict
      resolution then has principled rules instead of a similarity guess: **temporal versioning** (prefer the
      newest) and **source attribution** (a *user statement* outranks an *agent inference*). Nanna already
      distinguishes STATED vs OBSERVED in extraction — that is exactly the precedence signal, currently unused
      at merge time. This would make the dedup fold landed this run smarter (canonicalized text folds more
      often) and safer (a user-stated fact never gets overwritten by an inferred one). Sources:
      [arXiv:2603.07670](https://arxiv.org/html/2603.07670v1),
      [TOKI — bitemporal contradiction resolution (arXiv:2606.06240)](https://arxiv.org/pdf/2606.06240).
- [ ] *(research 2026-07-23)* **Fused retrieval score beats pure cosine — reported +29.6 on temporal queries,
      +23.1 on multi-hop.** 2026 systems combine **semantic similarity + BM25 + entity matching** into one fused
      score rather than ranking on embedding distance alone. Nanna's `recall` is pure in-RAM cosine over Turso
      BLOBs, so it is weakest exactly where the fused score helps most (time-anchored and multi-hop questions).
      A lexical (BM25) leg is cheap and fully local — no model, no network — and composes with the planned HNSW
      candidate stage: retrieve candidates by vector, re-rank by the fused score. Pairs with the
      `nanna-timeline` work, which is what makes temporal queries answerable at all. Sources:
      [state of AI agent memory 2026](https://mem0.ai/blog/state-of-ai-agent-memory-2026),
      [arXiv:2603.07670](https://arxiv.org/html/2603.07670v1).
      - [ ] *(research 2026-07-24)* **The BM25 leg may need no new dependency either — `turso 0.6.1` ships an
            FTS index method.** Same source-level check as the vector-function finding above:
            `turso_core-0.6.1/index_method/fts.rs` (133 KB) implements `CREATE INDEX ... USING fts (...)`
            with a Tantivy-derived tokenizer for both query and text. Caveat, checked rather than assumed:
            **no BM25 scoring in it** (zero `bm25` references), so it gives the *lexical matching* half, not
            the ranking function — BM25 term weights would still be ours to compute over the FTS candidate
            set. Worth measuring before pulling in a search crate, since it keeps the "Turso-only" invariant.
            This is also the cheapest path for the **tool-description keyword search** noted in P6/P11
            (tool descriptions currently need literal keywords because there is no lexical search at all).
      - [ ] *(research 2026-08-21 — settles HOW to fuse, which was the unstated hard part)* **Use
            Reciprocal Rank Fusion, not a weighted sum of scores.** RRF has the two properties this
            problem needs and a weighted sum does not: it is **score-independent** (only ranks enter the
            fusion, never raw scores) and **additive across stores** (an item's fused score is the sum of
            its per-list contributions, `Σ 1/(k + rank_i)`). That matters here more than in a typical RAG
            stack, because Nanna's two legs produce genuinely incomparable numbers — an in-RAM cosine in
            [-1, 1] and a BM25 term weight computed over an FTS candidate set — and normalising them
            against each other would be inventing a scale. RRF needs neither normalisation nor a tuned
            α, so the fusion introduces no magic number; only `k` (conventionally 60), and `k`'s effect
            is a documented rank-discount curve rather than a per-corpus fit.
            Reported effect where it has been measured: a tuned hybrid reaches 0.7497 NDCG on WANDS
            against 0.6983 for BM25 alone and 0.6953 for pure vector — i.e. **the lift comes from the
            fusion, not from either leg being better**, which is the argument for adding the leg at all.
            Sources:
            [Hybrid BM25 retrieval](https://www.emergentmind.com/topics/hybrid-bm25-retrieval),
            [Hybrid search & reranking in production RAG 2026](https://appscale.blog/en/blog/hybrid-search-and-reranking-production-rag-bm25-dense-cross-encoder-2026).
      - [ ] *(research 2026-08-21)* **Condition the channel weights on the QUERY TYPE, and get the
            temporal win the +29.6 figure is actually about.** The 2026 systems that report the large
            temporal gain classify each query as `single_hop | multi_hop | temporal | aggregation` and
            apply a per-type multiplier before fusing — a temporal query boosts the time channel and
            damps the others; a multi-hop query boosts the entity/graph channel. Nanna can afford the
            cheap half of this today: the classifier is a handful of lexical cues ("when", "last week",
            "before X", a date), it needs no model, and the recall gate already inspects the message
            (>5 words OR `?` OR >80 chars). The temporal channel itself is `nanna-timeline`, so this is
            the item that makes P13's timeline work *pay* at recall time rather than only at
            compression time — worth sequencing right after it. Do NOT ship the classifier before there
            is a temporal channel for it to boost: a multiplier over one channel is a no-op with a
            maintenance cost. Source:
            [AgentIR — workload-adaptive cascade retrieval (arXiv:2605.25092)](https://arxiv.org/pdf/2605.25092).
- [ ] *(research 2026-07-23)* **Episodic→semantic promotion is still manual almost everywhere — an opening.**
      The survey's own example is ours: repeated episodic records ("user corrected the date format", on five
      different days) should graduate into one semantic fact ("user prefers DD/MM/YYYY"), but in current systems
      this "rarely automatic" step needs explicit prompting. EverMemOS (Jan 2026) is the closest shipped shape —
      an engram-inspired three-stage lifecycle: episodic trace formation → semantic consolidation into thematic
      "MemScenes" → reconstructive recollection that composes context on demand. This is the same arc P13
      already commits to (`nanna-timeline` episodes consolidating *into* `MemEntry` facts), so the useful part
      is the staging vocabulary and the fact that **frequency-of-recurrence** is the promotion trigger — which
      our per-memory access counts and the dedup fold count already measure. Also worth reading before the
      dreaming overhaul: **RecMem**, recurrence-based consolidation aimed specifically at long-running agents.
      Sources: [arXiv:2603.07670](https://arxiv.org/html/2603.07670v1),
      [RecMem (arXiv:2605.16045)](https://arxiv.org/pdf/2605.16045).
      - [ ] *(research 2026-08-24)* **The field has converged on the three-tier taxonomy we already
            half-implement — and has named our exact risk.** 2026 surveys settle on
            **episodic / semantic / procedural**, with promotion driven by recurrence: repeated episodes
            distil into a durable fact while the specific event fades. Nanna has the semantic tier
            (`MemEntry` + FSRS) and P13 plans the episodic one (`nanna-timeline`); **procedural is
            entirely absent** — a "how I do this" tier is what the "Instruction skills + slash macros"
            item is really asking for, so those two should be designed together rather than as separate
            features.
            The warning is aimed straight at our dreaming loop: *"periodically summarize conversational
            turns"* is called out as **dangerous for long horizons because summarization drift
            accumulates across passes** until compressed memory no longer represents what happened —
            which is rank-similar → concatenate → summarize, exactly. That is the same failure already
            logged under the summarization-drift item; two independent 2026 sources now name it, which
            argues for promoting the drift mitigations ahead of new dreaming *features*.
            Also worth reading before the overhaul: **selective parametric consolidation** (consolidate
            *depth*, not access) reports better goal persistence and post-unload recovery than
            summarize-everything, and **Memanto** pairs a typed semantic store with
            information-theoretic retrieval — a concrete shape for the "fused retrieval score beats pure
            cosine" item above. Sources:
            [Selective parametric consolidation (arXiv:2606.26806)](https://arxiv.org/pdf/2606.26806),
            [Memanto (arXiv:2604.22085)](https://arxiv.org/pdf/2604.22085),
            [Multi-layered memory architectures (arXiv:2603.29194)](https://arxiv.org/html/2603.29194v1),
            [Agent-Memory-Paper-List](https://github.com/Shichun-Liu/Agent-Memory-Paper-List).
- [ ] *(research 2026-08-25 — grades the deferred-vector change that landed the same day)* **The write
      path is the agent-memory cost centre the benchmarks do not report, and asynchrony buys latency with
      *staleness*.** Two findings worth holding side by side. First, write-path cost is measured at **over
      80% of total agent execution time** in stateful long-horizon workloads and is simply absent from most
      memory-system benchmarks — which is the same discovery P24.3 made from a log (189 of 246 minutes with
      no model decision) rather than from a paper, and is the strongest argument yet that `nanna-bench`
      Suite 2 needs a **write**-side number, not only recall/search latency. Second, the survey's framing of
      the choice is exactly ours: *synchronous scheduling puts construction latency on the critical path;
      asynchronous scheduling admits unbounded staleness*, and five of the systems it measures (SimpleMem,
      MIRIX, Letta, Mem0, A-Mem) retrieve against memory one or more sessions behind their ingestion
      stream. Nanna's deferred-vector path deliberately takes the asynchronous side, so **its staleness must
      be bounded and measured, not assumed** — the drain's budget bound (embed only what this process
      parked) is the mechanism, and the missing half is evidence that it converges within a turn.
      Concrete, in reach:
      - [ ] **Measure queue-to-searchable latency** — time from `remember_deferred_vector` returning to the
            row having a vector, p50/p95, under a live mission. This is Nanna's staleness number and it does
            not exist yet.
      - [x] **Add a write-path suite to `bench/BASELINE.md`** *(2026-08-25)* — "Suite 2 (write path)",
            two rows, gated in `bench/budgets.toml`. It records **counts, not milliseconds**: embedding
            round-trips on the turn's critical path, **0** per tool result (was 1 per chunk; ~63 chunks
            for a 200 KB non-repetitive result) and **1** per ordinary extracted fact. That framing is
            the point — wall-clock follows from the count and the provider's RTT, which is load-dependent
            and unreproducible, while the count is exact and hardware-independent. The tool-result budget
            is `exact = 0` **at any chunk count** (asserted at 1, 8, 64), so it is a structural claim
            rather than a sample; the ordinary-fact budget is a **floor**, because 0 there would mean the
            deferral had swallowed a path that must still dedup inline. Instrument:
            `cargo test -p nanna-daemon write_path`.
      - [ ] **Report embedding-generation latency separately from vector-search latency.** The retrieval
            budget is per *stage* (embed → search → rerank → assemble); a 4 ms search behind a 400 ms embed
            is a 400 ms retrieval, and our numbers currently name only the second half.
      Sources: [Agent Memory: Characterization and System Implications of Stateful Long-Horizon Workloads
      (arXiv:2606.06448)](https://arxiv.org/html/2606.06448v1),
      [MemDelta (arXiv:2606.29914)](https://arxiv.org/pdf/2606.29914),
      [Memory retrieval latency budgets](https://supermemory.ai/blog/latency-budgets-memory-retrieval).
- [ ] *(research 2026-07-19)* **"Sleep-time compute" generalizes our idle gate from *consolidate* to *pre-compute*.**
      Now that the daemon actually dreams only during a lull (idle gate wired 2026-07-19), the 2026 literature
      (Letta's sleep-time compute, arXiv:2504.13171; the SCM "sleep-consolidated memory" and 9-stage consolidation
      papers) points at the next lever: during idle, don't *only* rank-similar→concatenate→summarize — also
      **rewrite raw context into "learned context"** (pre-organize/pre-answer likely future queries) so wake-time
      responses are cheaper. Reported effect: ~5x less test-time compute for equal accuracy, ~2.5x lower cost/query
      when amortized across related queries. Two concrete, in-reach steps for Nanna: (a) a dream phase that
      **promotes recurring episodic memories to semantic/fact memories** (maps onto the P13 "episodes consolidate
      into facts" line and the DSP peak-detection item), and (b) let the dream cycle use a **stronger model than the
      chat model** — it has no latency constraint — which our `summarization_priority` list already allows; make the
      dream path prefer it explicitly. Gate any change through the retention harness. Sources:
      [Letta sleep-time compute](https://www.letta.com/blog/sleep-time-compute/),
      [arXiv:2504.13171](https://arxiv.org/abs/2504.13171).
- [x] *(2026-07-19)* **Idle gate covers autonomous agent runs too, not just IPC chat.** The wiring stamps
      `ActivityClock` on `Action::Chat` (channels route through it) **and** at the top of the scheduler executor's
      agent-prompt arm, so the daemon's own **heartbeat/cron/task agent runs** also count as activity — a dream
      cycle defers while an autonomous run is in progress. Safe against starvation: heartbeats are infrequent
      (30 min) vs the 5-min idle threshold, and the memory-pressure trigger still overrides. (The
      `memory_consolidation` task itself is a separate named arm, so it never self-stamps.)

**DSP-backed time-series / event-timeline memory (compression-as-dreaming):**
- [ ] **`nanna-timeline` crate + append-only event log** — `MemoryEvent { id, ts, kind, workspace_id, content, embedding, salience, source_ids }` in a new Turso migration; the raw episodic stream (messages, tool calls, recalls, outcomes) on a wall-clock axis. `MemoryEntry` stays the semantic/fact layer; episodes consolidate *into* facts during dreaming.
- [ ] **Resample the timeline into per-signal series** — salience(t), access-rate(t), emotional valence(t), per-cluster topic-activation(t).
- [ ] **DSP compression = dreaming over time** — keep the recent window at full sample rate; for older windows decimate/wavelet-drop low-energy detail with the **keep-rate driven by FSRS `power_law_retrievability`** — sharp near-term detail, blurred long-term gist. Lift DSP's pure `simplify_with_aggressiveness` + slope-change simplifier + `splimes::auto_interpolate` (see design notes); store decimated windows / coeff blobs as Turso `f32` BLOBs.
- [ ] **Peak detection seeds consolidation** — DSP peak/energy detection marks salient moments → promote those episodes to facts + boost importance; long flat stretches → compress to Essence/drop. Ties the timeline back into the existing FSRS weight bands.
- [ ] **Single-GPU DSP kernels** — implement FFT/wavelet/convolution as wgpu compute shaders in `nanna-gpu` (alongside `CosineSimilaritySearch`), with a CPU fallback in `nanna-simd`. No external DSP service.
- [ ] **Make it demoable** — GUI dream-log + a salience **spectrogram/waterfall** over time (consolidation lineage `consolidated_from`/`generation` already exists). This is the "unique sauce" screen.
- [ ] Also from backlog: HNSW persistent vector index (avoid full `bulk_load` into RAM); emotional valence; memory-graph edges; dedup-before-store; ~~extraction filtering (<50 chars)~~ **(done 2026-07-06 — `is_storable_memory` drops sub-50-char extractions in `loop_runner::extract_memories`; 2 tests)**.
- [ ] add correlation tool that requires time-series data + event timestamps to use DSP to make predictions.

### P14 — Long-Horizon Autonomy on a Small Local Model ✅ (harness + first live on-model baseline landed 2026-07-18; full eval suite open)

**Goal:** a 7–9B local model that stays on task for **hours**, not 2–3 tool calls, at a token cost that a
single GPU can actually sustain. P12 gives us a model that *runs*; this phase is what makes it *useful*.
Everything here is testable **today against Ollama** — none of it waits on Mummu.

**The problem, stated honestly.** Our own research already says local ~7–9B models *"lose coherence after
2–3 tool-chain steps"* (P12, 2026-07-07). A frontier model survives long tasks by brute context: it
re-reads a 200k-token history and re-derives intent every turn. A local model has neither the window nor
the tok/s to do that. So long-horizon capability cannot come from the model — it has to come from the
**harness**. The design bet: *the agent should never need to remember; the harness should make forgetting
survivable.* Two goals that sound opposed — hours of coherence, few tokens — are the same goal, because
**the way you burn tokens is by re-establishing context you failed to persist.**

**Governing metric:** *task success @ tokens* — fraction of a long-task eval suite completed, over total
tokens spent. Not tok/s, not context size. A run that finishes in 40k tokens beats one that finishes in
400k, and both beat one that drifts. Ties into the P-&-B *agent-eval suite* (that suite is the denominator).

**Landed (2026-07-18):** the whole harness ships in `nanna-agent/src/harness.rs` (the engine:
`LongHorizonRunner` over two traits, `TaskSource` + `StepRunner`, so the control loop is
deterministically testable without a model — 20+ tests incl. the Suite 4 fixtures) with daemon
production impls in `nanna-daemon/src/tasks.rs` (`TursoTaskSource`, `AgentStepRunner` = fresh
`Agent` + empty context per step, `TaskRunManager` for background runs) and IPC surface
`TaskAction::StartRun/RunStatus/CancelRun` + `TaskRun*` events. The live on-model eval passes
**5/5 @ 22.6k tokens/item on qwen3.5:9b** after same-day harness tuning (see the benchmark items
below); what remains open is the full eval build-out (published task set, pass^k, 8 GB tier).

**Design spine — externalize state, keep the window tiny:**
- [x] **The todo store *is* the agent's working memory** (P15) — *(2026-07-18)* a run is a loop over
      `next()`; each step's prompt carries only the current task, its acceptance check, its recent
      notes, and the last result. The model's job is "advance one step".
- [x] **Re-anchor, don't re-read.** *(2026-07-18)* Every step runs in a **fresh agent context**
      (`AgentStepRunner` builds a new `Agent` + empty `AgentContext` per step) — long-run context is
      O(1) by construction, not by compression. Findings persist via task notes (append-only,
      16 KiB bound), not the transcript. Validated by research: "self-conditioning" (arXiv 2509.09677)
      shows models err more when their own past errors stay in context, and it is NOT fixed by scale.
- [x] **One tool per step, chosen from ≤5.** *(2026-07-18)* Per-item `tools:` hint on the task row →
      `RunOptions.initial_active_tools`; the step activates exactly the scoped set (+ `todo`, its only
      memory) instead of the registry. `discover_tools` stays available as the escape hatch.
- [x] **Sub-agent per subtask, fresh context, structured return.** *(2026-07-18)* The engine sees only
      `StepOutcome` (text + token counts + tool-call *digests*) — the parent's context cannot grow
      when a step runs, structurally.
- [x] **Checkpoint + resume across restarts.** *(2026-07-18)* The task store **is** the checkpoint:
      every mutation is durable in Turso at the moment it happens, so resuming after a crash/reboot is
      just `StartRun` on the same scope — `next()` picks up exactly where the plan stands, no replay.
      (Run *counters* — tokens spent so far — reset on restart; the plan and all notes do not.)

**Staying on track (drift is the real enemy, not context length):**
- [x] **Acceptance check per todo item.** *(2026-07-18)* `AcceptanceCheck` (command exit-0 /
      file_exists / regex over file-or-command-output, timeout-bounded) runs **in the harness** after
      every step; with a check present, the environment is the only judge — a `TASK COMPLETE` claim
      that the check refutes is counted as a `false_success_claim` and logged, never recorded as done.
      The `tasks.done` service and GUI `Done` action gate the same way, so the model can't route
      around it. Shape validated at write time by the store.
- [x] **Progress-or-replan.** *(2026-07-18)* N steps (default 5) with no check flipping ⇒ a `Plan`-kind
      replan step that decomposes the item into subtasks *in the store* (via the todo tool — no plan
      parsing); after `max_replans_per_item` (default 2) the item is abandoned (cancelled + reason in
      the activity log) and the run moves on. Grinding is bounded by construction — see the drift
      containment row in `bench/BASELINE.md` Suite 4.
- [x] **Loop/repetition detector.** *(2026-07-18)* Two signals, per the research (hash-identical loops
      and semantically-varied flailing are different failure modes): in-run, same tool + same args +
      same result twice ⇒ one corrective nudge (`detect_tool_call_loop`, next to the narration/spiral
      detectors); cross-step, an identical tool-call signature two steps running doubles the stall
      counter, accelerating replan/abandon.
- [x] **Bounded blast radius.** *(2026-07-18)* Per-run caps on wall-clock, total tokens, and (loop-level)
      tool calls — `RunOptions.max_wall_clock`/`max_tool_calls` + harness `max_wall_clock`/
      `max_total_tokens`, all caller-set, no magic defaults at the loop level. The budget is surfaced
      *to the model*: a `== BUDGET ==` line in every step prompt, and the agent loop now injects a
      model-visible status message at 80% of a token budget (previously log-only).
- [x] **The goal is immutable.** *(2026-07-18)* Pinned verbatim at the top of the byte-stable prompt
      prefix of every step; never summarized, never compressed (test-asserted).

**Token economics (measure before optimizing):**
- [x] **Token budget accounting per run** — *(2026-07-18)* `LongHorizonReport.tokens_per_completed_item`
      is the run's governing metric; per-item `tokens_spent` also lands in the completion activity
      detail, so post-mortems can see which item burned the budget. (Note: the roadmap's "CostTracker
      (P6)" never existed as a type — accounting builds on `RunState` token counters + `ModelStatsTracker`.)
- [x] **Prompt-cache the immutable prefix.** *(2026-07-18)* The step prompt is stable-prefix +
      dynamic-tail by construction (`build_step_prompt`): system rules + verbatim goal never move
      (byte-identical across steps, test-asserted — the shape KV-prefix reuse rewards), and the
      current task/verdict/budget ride at the end, in recent attention (the Manus recitation pattern).
- [x] **Ladder the model, don't fix it.** *(2026-07-18)* `StepKind` (plan | execute | verify) threads
      from `RunOptions` into `classify_complexity`/`route_model`: Plan ⇒ Complex (biggest model),
      Verify ⇒ Medium, Execute ⇒ the structural heuristic (cheap-model biased); execute steps also skip
      the routing's first-turn-primary rule since every re-anchored step is a "first turn".
- [x] **Stop paying for tool output twice.** *(pre-existing, confirmed)* Per-tool `output:
      context|memory` routing already defaults verbose tools to "chunk to memory + stub in context";
      the task tools declare `output: "context"` so plans are never stubbed away.
- [x] **Benchmark (deterministic half):** *(2026-07-18)* `bench/BASELINE.md` Suite 4 commits
      task-success @ tokens rows from scripted-model fixtures (`cargo test -p nanna-agent harness`):
      compliant runs complete 3/3 at exactly 1200 tokens/item, a perma-false-claiming model admits
      **0** completions and costs ≤ 6000 tokens before abandonment, loops abandon in < 4 steps.
- [x] **Benchmark (live half):** *(2026-07-18, tuned to 5/5 same day)* the harness runs end-to-end
      against a real local model: qwen3.5:9b via Ollama, 5 minutes-scale tasks with machine
      acceptance checks (`nanna-daemon/tests/live_long_horizon.rs`, `#[ignore]`d). Final:
      **5/5 verified-complete @ 22,564 tokens/item in 72 s (6 steps), 0 replans, 0 false-success
      claims admitted** — recorded in `bench/BASELINE.md` Suite 4 with the full tuning trail.
      The eval earned its keep immediately: run 1 (0/5) caught scripted tools loading without the
      registry handle (relative paths silently resolved to `$HOME` — production bug, fixed); run 2
      (3/5) caught the acceptance runner silently falling back to `cmd.exe` when no bare `sh` is on
      PATH (POSIX checks unwinnable — now routed through Git Bash like the exec tool,
      regression-tested) plus Ollama 500s tripping the error breaker (now retried with a fresh
      re-anchored context); run 3 = 5/5.
  - [x] **The "4-hour task", run for real:** *(2026-07-19)* qwen3.5:9b worked ONE seeded plan
        (build `minidb` against 42 fail-to-pass feature tests) for the full **6-hour** wall-clock
        cap — longest unbroken segment **4h39m** after a single healed provider incident — with
        23 verified completions distributed across the whole window, **0 false successes in six
        hours**, and on-plan work still happening at hour six. 5.13M tokens, 137 steps
        (`bench/BASELINE.md` Suite 4, endurance section, incl. the seven-run tuning trail: every
        failed run exposed a real bug — tool workdir plumbing, cmd.exe acceptance fallback,
        Ollama aborted-generation parsing, poison containment, subtask queue-jumping).
  - [x] **Cloud endurance (openrouter/free auto-router):** *(2026-07-20)* the same ladder driven
        through OpenRouter's free tier, where the serving model varies per request — **33/42
        verified in 3.30h, one unbroken segment, 0 resumes, 0 false successes, plan drained**
        (`all_tasks_done`; 12 items abandoned via containment where weak model draws ground out).
        Healing is provider-aware (`ProviderId::from_model` gates local-server surgery to
        Ollama-served models; cloud incidents heal by pause+resume+retries). Recorded in
        `bench/BASELINE.md` Suite 4.
  - [ ] **Live half, remaining:** local-tier throughput (14/42 primary features in 6h — the
        middle-ladder grind dominates), a published task set (Terminal-Bench easy-tier /
        SWE-bench Lite), pass^k on the endurance suite, and the 8 GB reference tier.

- [x] *(research 2026-07-17 → done 2026-07-18)* Cross-checked against published work; the design held
      up and got sharper. Key findings: long-task failure is execution/context, not reasoning —
      "self-conditioning" means fresh minimal context beats a transcript, and scaling doesn't fix it
      (arXiv 2509.09677); "false success" (agent claims done, environment disagrees) is 45–76% of
      failures in several suites and LLM judges barely detect it (AUROC 0.54–0.65) — harness-run
      environment checks are the fix (arXiv 2606.09863, AgentRewardBench); tool-selection accuracy
      collapses >90% → ~13% as tool count grows, specifically for small models (RAG-MCP, MCPVerse);
      goal drift worsens with horizon for every model tested (arXiv 2505.02709); reliability
      (τ-bench pass^k) collapses across repeated trials, so soft nudges through a small model's
      context are weak medicine — enforcement must be harness-side, on objective signals. Prior art
      for store-as-control-structure is rich (Claude Code TodoWrite, Manus todo.md recitation, Beads'
      DB-over-markdown argument, claude-task-master's advisory `testStrategy`) — none combines an
      external store with *harness-executed* acceptance on 7–9B local models; that combination is the
      novel part. Design deltas adopted from the research: the false-success counter, the dual
      repetition signal, replan-splits-tasks (MAST: ~42% of failures are bad decomposition), and
      byte-stable prefix + recency-positioned task (Manus KV-cache lesson).

### P15 — Agent-Grade Task Store (todo as control structure) ✅ (landed 2026-07-18)

**Goal:** replace the flat, session-scoped `todo` skill with a task store an agent can *plan* against and
the harness can *drive* a long run from. This is P14's substrate — the two ship together or neither works.

**What exists** (`crates/nanna-tools/default-skills/todo/tool.ts`, 259 lines, v0.1.0): a flat list in a
per-session JSON file (`.nanna-todo-{session}.json`) with `add/create/done/update/remove/clear/clear_all/
list` and status `pending|in_progress|blocked|done`. That is a **scratchpad**, and its limits are exactly
what breaks long runs: no hierarchy, so a big task cannot be decomposed in place; **no dependencies**, so
`blocked` is a label a model sets by vibes rather than a fact the harness derives; no persistence beyond a
session, so an agent that restarts forgets the plan; no query, so "what is next?" costs a full-list dump
into context every turn; and no acceptance criteria, so *the model decides when it is done*.

**Todoist as the reference feature set** *(2026-07-17 — surveyed [features](https://www.todoist.com/features)
and the [filter syntax](https://www.todoist.com/help/articles/introduction-to-filters-V98wIH))*. It is the
right prior art because it solved "a human keeps hundreds of tasks straight for years" — but the mapping is
not 1:1, and the differences matter more than the similarities:

| Todoist | Take it? | Why |
|---|---|---|
| Projects / sections / **sub-tasks** | **Yes** | Hierarchy *is* decomposition; the unit a sub-agent gets |
| **Dependencies / blocking** | **Yes — the big one** | Makes `next()` derivable instead of guessed |
| **Filter query language** (`&`/`\|`/`!`/parens, `today`, `overdue`, `p1`, `@label`, `#project`, `search:`) | **Yes** | An agent that can *query* stops paying to re-read the list |
| Priorities `p1..p4` | Yes | Cheap, and orders `next()` |
| Labels | Yes | Doubles as the per-item **tool scope** hint (P14) |
| Due dates + **natural-language parsing** | Partly | Deadlines matter; NL parsing is a *human* affordance — an agent should emit structured dates. Don't build a date parser for a machine caller |
| Recurring tasks | Yes | Maps onto HEARTBEAT.md / cron (P8) — one recurrence engine, not two |
| Reminders | Reuse | `remind`/`cancel_reminder`/`list_reminders` skills already exist — wire, don't duplicate |
| Comments / attachments | Adapt | Becomes **per-task working notes** — the durable scratchpad P14 needs |
| Activity history | **Yes** | The audit trail of a 4-hour run; also the dataset for "why did it drift?" |
| Karma / productivity charts | **No** | Gamification for humans. An agent needs an acceptance check, not points |
| Collaboration / assignment / roles | **Reframe** | "Assignee" = *which agent* (parent vs sub-agent), not which person |
| Templates | Later | Useful once recurring multi-step jobs exist |
| Views (board/calendar), 80+ integrations | GUI-only | A rendering concern, not agent-facing |

**Build-out (all landed 2026-07-18 — migration `011_tasks`, `TaskRepository` in
`nanna-storage/src/tasks.rs` (24 tests), filter parser in `task_filter.rs` (26 tests), todo skill
v0.2.0, `tasks.*` script services + `TaskAction` IPC group + GUI `/tasks` page):**
- [x] **Store in Turso** — `tasks` + `task_notes` + `task_activity` tables (migration `011_tasks`);
      scope `session | workspace | global` with disjoint views, so a plan outlives the chat that made
      it. Integer ids (small-model-friendly; uuids add nothing agent-facing). Turso only, no new store.
      *Learned the hard way:* an unfinished `Rows` cursor on the shared turso connection **silently
      swallows subsequent writes** — drop cursors before writing (found via a vanishing activity row;
      comment at the create() site).
- [x] **Hierarchy** — `parent_id` + `sort_order`; a parent **cannot** complete while a child is open
      (repo-enforced, instructive error), and auto-completes when its last child closes — *unless it
      carries its own acceptance check*, in which case it must be completed explicitly so its check
      runs. Depth bounded at 32 (recursion protection, documented justification). Cancelling a parent
      cascades to its open subtree (children of a dead plan must not surface from `next()`).
- [x] **Dependencies** — `depends_on[]` with cycle check **on write** (BFS over the would-be graph;
      reject self-deps and transitive cycles; parent-chain cycles too). `blocked` is derived at read
      time — writing `status='blocked'` is rejected with "add a dependency instead". Cancelled
      dependencies count as satisfied (a dependent must not block forever on an abandoned item).
- [x] **`next()`** — the one actionable item: open, unblocked, leaf (no open children); ordered
      `in_progress` first (resume what you started), then priority, due date (nulls last), explicit
      order, id. Returned with its acceptance check, tool scope, and a 5-note tail — one item in
      context per turn.
- [x] **Acceptance criteria per item** — `{kind: command|file_exists|regex, ...}`, shape-validated at
      write time so the harness never meets a malformed check; run by the harness / `tasks.done`
      service (see P14). `done` via plain `update` is rejected: "use the done action so the
      acceptance check can run".
- [x] **Filter/query language** — the planned Todoist subset (`&`, `|`, `!`, parens, `p1..p4`,
      `@label`, `#project`, `overdue`, `due before:/after:`, `no date`, `no label`, `search:`,
      `subtask`) plus status atoms (`pending`/`in_progress`/`done`/`cancelled`/`blocked`-as-derived)
      and `today`. Pure recursive-descent parser, zero I/O, bounded input (4 KiB) and depth (32),
      structured ISO dates only (no NL date parser for a machine caller), 26 unit tests incl.
      precedence, no-space colon forms, and adversarial inputs.
- [x] **Working notes per task** — append-only, 16 KiB/note bound (a note-tail injection can never
      exceed ~4k tokens); the harness writes each step's findings here — long-run state lives in the
      store, not the transcript.
- [x] **Activity log** — every transition with actor + timestamp + JSON detail (created / updated /
      completed / auto_completed / cancelled / reopened / acceptance_checked / false_success_claim /
      replanned / abandoned / imported_blocked). This is the drift post-mortem dataset.
- [x] **Assignee = agent** — column + `actor` on every activity entry; harness runs stamp
      `harness`, GUI actions stamp `gui`, migration stamps `todo-v0.1-migration`.
- [x] **Recurrence via the existing scheduler** — tasks store a 5-field cron expression; a
      `task_recurrence_sweep` job on the P8 daemon scheduler (every 5 min) reopens completed
      recurring tasks whose next occurrence has arrived. One recurrence engine: the store holds the
      expression, the scheduler is the clock.
- [x] **Tiny tool surface** — todo v0.2.0 exposes `next / add / update / done / note / query / list`
      (plus the v0.1 `create/remove/clear/clear_all` still accepted); the full repository API is the
      *store's* capability, reachable via IPC, not the model's tool schema.
- [x] **JSON migration** — on first use in a session, the skill imports `.nanna-todo-{session}.json`
      via `tasks.import` (order preserved; v0.1 `blocked` label → `pending` + activity note, since
      blocked is derived now) and stamps the file `{"migrated": true}`. The skill keeps a full v0.1
      file fallback for contexts without the daemon task services, and routes
      `update(status='done')` through the verdict-gated done path.
- [x] **GUI** — `/tasks` page (Nuxt): task tree with status/blocked/priority/labels, details panel
      (description, acceptance, notes, activity), filter-language search, create/complete/delete
      (acceptance-failure verdicts surfaced), and a **long-horizon run panel** — goal + budget,
      Start/Cancel, live `task-event` feed, final report (items completed, tokens/item, stop
      reason). This is the "is it still on track?" screen. Full IPC path:
      `TaskAction` protocol group → `control/task.rs` → daemon_client/backend/commands → page.

### P16 — Daemon-only consolidation: GUI is a pure daemon client ✅ (landed 2026-07-18, flagship refactor)
**Landed:** dropped **all** in-process "embedded" execution from the Tauri GUI. It now only attaches to
`nanna-daemon` over IPC and forwards every request; a failed connect is a hard `Disconnected` status (no
fallback). This ends the double-implementation tax the P4/P8/P11 "embedded copy of X drifted" items were a
symptom of — one agent loop, one memory system, one tool registry, one scheduler. iOS/mobile deferred.
Net **−5,510 / +1,282** LOC; `cargo check -p nanna-gui` clean, log-buffer + log-merge tests green.

What shipped: deleted `embedded.rs` / `tool_authoring.rs` / `llm/`; pruned `AppState` to a thin client
(config cache, workspace-registry cache, backend, log buffer, model-badge caches); gutted `setup_state`
(no local Storage/LlmClient/ToolRegistry/MemoryService/Scheduler+executor; workspaces hydrate from the
daemon); collapsed `backend.rs` to `BackendMode {Daemon, Disconnected}` with unconditional daemon
forwarding; removed every command's embedded arm; rewired `/agents` onto daemon sub-sessions; relocated
`log_buffer` to `nanna-core`; pruned GUI `nanna-*` deps to `nanna-config` + `nanna-core` + `nanna-tools`
(dropped storage/memory/scripting/agent/workspace/channels/daemon/llm); removed the mobile entry + android icons.

**Deferred follow-ups** (worked only in the embedded path; no daemon control action yet — degraded, not lost):
- Memory/scheduler runtime toggles — `set_dreaming_enabled`, `set_scheduler_enabled`,
  `set_heartbeat_enabled`/`_interval`, `get|set_similarity_threshold`, `apply_memory_updates`,
  `save_memories` — are **no-ops** (were already dead in daemon mode). Add daemon control actions to wire
  them back. (`max_compression_ratio` / `min_remaining_memories` already persist via `config_set`.)
- **Skill-directory CRUD** still edits the workspace `skills/` dir on disk (test routes to the daemon
  sandbox) — fold into daemon `tool_*` actions so the GUI edits the daemon's `tools_dir`.
- **`/agents`** maps daemon sub-sessions but has no live `agent-event` feed / workspace tagging (it polls)
  — add a daemon agent-event feed.
- **Config ownership** — GUI keeps a `config.toml` write cache that pushes via `config_set`/`config_reload`;
  a single-writer daemon-owned model with a pure read cache is the endgame.

### P17 — Workspace context: standard project files instead of bespoke `.nanna/` agent files 🌱 (new — 2026-07-17, product direction)
**Directional change (owner-requested):** stop making Nanna scaffold and read a pile of bespoke per-workspace
agent markdown. Today, initializing a *user's* workspace creates `.nanna/{AGENTS,SOUL,USER,TOOLS,IDENTITY,
HEARTBEAT,MEMORY}.md`, and agent context is assembled by reading them. **Going forward a workspace's context
comes from the project's OWN standard files** — the ones any repo already has and any contributor already
understands — with per-workspace planning in a `ROADMAP.md` modeled on Nanna's own. Nanna should drop into any
existing repo and be useful from its `README.md` / `AGENTS.md` / `ROADMAP.md` with **no `.nanna/` scaffolding
required**. *(Scope: this is the PRODUCT's per-workspace files, NOT the nanna source repo's own dev docs —
Nanna's own `ROADMAP.md` stays.)*

**Target model (decided 2026-07-17):**
- **Workspace context = the project's standard files.** Nanna reads, in priority order: `README.md` (what the
  project is), root `AGENTS.md` (the emerging *agents.md* standard — agent instructions for this repo),
  `CONTRIBUTING.md` (conventions / how to work here), `docs/**`, and `ROADMAP.md` (the plan — Nanna both reads
  and maintains it, in the same phase/checklist/dated-note structure as Nanna's own). A root `AGENTS.md` is
  *standard*, not bespoke, so it stays; `SOUL/USER/TOOLS/IDENTITY/HEARTBEAT/MEMORY` go.
- **Persona + user profile → GLOBAL agent config.** `SOUL.md` (who the agent is) and `USER.md` (who the user is)
  are cross-workspace, not per-project — they move into global agent settings applied to every workspace, not
  files scaffolded into each project; `IDENTITY.md` folds in here too.
- **Memory → DB-backed only.** Drop the `.nanna/MEMORY.md` (+ `memory/*.md`) file mirror; memory already lives in
  Turso (`nanna-memory`, FSRS). The GUI/daemon memory reads that go through the files today route to the store.
- **Heartbeat → scheduled-task config.** Drop `HEARTBEAT.md` as a prompt file; periodic tasks become scheduler
  config (the daemon already runs a heartbeat/cron loop — the "Read HEARTBEAT.md if it exists" prompt is replaced
  by task definitions).
- **`TOOLS.md` → dropped.** Tools are discoverable at runtime; a static notes file is redundant.

**Code surface to change** (2026-07-17 inventory — **all completed 2026-07-18**):
- [x] Retire the file-name constants + context assembly: `crates/nanna-core/src/workspace.rs:32-38`
      (`AGENTS_FILE`…`HEARTBEAT_FILE`) + the read/assemble at `:87-101,198-…`; the parallel set in
      `crates/nanna-workspace/src/lib.rs:43-49` and the context builder `crates/nanna-workspace/src/files.rs:81-275`
      (emits `## AGENTS.md`…`## HEARTBEAT.md` sections). Re-point context assembly at the standard files.
      -> `HEARTBEAT_FILE` and the SOUL/USER/TOOLS/IDENTITY constants are removed; `workspace.rs`/`files.rs`/
      `lib.rs` now assemble context from `README.md` / `AGENTS.md` / `CONTRIBUTING.md` / `ROADMAP.md` only
      (`STANDARD_CONTEXT_FILES`). `WorkspaceContext` uses those four optional fields.
- [x] Stop auto-creating the sidecar: `crates/nanna-workspace/src/manager.rs:164-188` (creates `AGENTS.md`/
      `SOUL.md`) and the templates `crates/nanna-workspace/templates/standard/{AGENTS,SOUL,USER,TOOLS,IDENTITY}.md`
      + `templates.rs:74-78` `include_str!`. Keep only a minimal root `AGENTS.md` (+ optional `ROADMAP.md`)
      template; delete the rest.
      -> `manager::initialize` writes only root `AGENTS.md` (+ creates the `.nanna/` local-state dir). The five
      `templates/standard/*.md` and their `include_str!`s are deleted; `templates.rs` exposes `minimal` and
      `project` templates (AGENTS.md [+ ROADMAP.md]).
- [x] **Workspace detection** (`crates/nanna-workspace/src/discovery.rs:12-60`) currently scores `.nanna/` /
      `SOUL.md` / `AGENTS.md`. Re-key on standard project signals: `.git`, `README.md`, root `AGENTS.md`,
      `ROADMAP.md`, `Cargo.toml` / `package.json` / `pyproject.toml`, etc.
      -> `WORKSPACE_MARKERS` now leads with `.git`, `AGENTS.md`, `ROADMAP.md`, `README.md`, `Cargo.toml`,
      `package.json`, `pyproject.toml`, `go.mod`, `pom.xml`, then `.nanna/`/`nanna.toml` as weak legacy signals.
- [x] **Global persona/user config:** add persona + user-profile fields to the global agent config (the source of
      truth), injected into every session's context independent of the workspace.
      -> `nanna-config::AgentSettings` gains `persona` + `user_profile` (`Option<String>`); `GlobalPersona`
      (in `nanna-core::workspace`) builds the injection; `control/session.rs` injects it into every session
      system prompt from global config.
- [x] **Heartbeat:** replace the `HEARTBEAT_FILE` prompt reads (`nanna-core/src/scheduler.rs:105`,
      `nanna-daemon/src/server.rs:795`, `gui/src-tauri/src/lib.rs:534`) with scheduled-task definitions.
      -> No `HEARTBEAT_FILE` reads remain anywhere. The daemon's heartbeat stays a *scheduler task* (prompt is a
      config string, not a file read) — this matches "scheduled-task definitions".
- [x] **Memory:** re-point the GUI memory reads off `MEMORY.md` / `memory/*.md`
      (`gui/src-tauri/src/commands/workspaces.rs:366-593`) onto the store; drop the `.md` mirror + the
      `include_memory` gating in `files.rs`.
      -> The GUI workspace-command memory `.md` mirror is removed; `files.rs` no longer gates on `include_memory`.
      Memory is DB-backed (Turso) as before.
- [x] **CLI + GUI + protocol:** update `src/commands/workspace.rs:23-41` (CLI `init` creates the 7 files), the GUI
      workspace-validity check that requires `.nanna` with `SOUL.md`/`AGENTS.md` (`commands/workspaces.rs:672`),
      `workspaces.vue`, and the daemon `protocol.rs` / `control/{session,chat}.rs` filename references.
      -> CLI `init` scaffolds standard files only; `check_workspace_validity` uses `WORKSPACE_MARKERS` + checks
      `AGENTS.md`/`ROADMAP.md` (no `.nanna`+SOUL/AGENTS requirement); `workspaces.vue` lists `AGENTS.md`/
      `ROADMAP.md`; daemon `protocol.rs`/`session.rs`/`chat.rs` reference standard context files.
- [x] **`.nanna/` dir fate:** the *markdown* sidecar goes; decide whether `.nanna/` survives for non-md workspace
      state (workspace id / local config) or that state moves to the central store. (Minor — surface in impl.)
      -> **Decision: `.nanna/` survives as a non-markdown local-state dir only** (`WORKSPACE_MARKER_DIR`). It holds
      workspace id / local config, never agent `.md` sidecar files. `Workspace::ensure_nanna_folder` creates it;
      `load_context` does a one-shot best-effort legacy import of a stray `.nanna/AGENTS.md` (read-only, not
      deleted). No SOUL/USER/TOOLS/IDENTITY/HEARTBEAT/MEMORY are ever written there.

**Migration (existing workspaces have `.nanna/` files today):** on first run against a legacy workspace, import
`SOUL.md`/`USER.md` → global config, confirm memory is in the store (it is), then stop reading `.nanna/*.md`.
delete the old files.

**Payoff:** Nanna works in any existing repo from its standard files with zero bespoke scaffolding;
persona/user/memory stop being duplicated into every project; one planning convention (`ROADMAP.md`) shared with
how Nanna plans itself. Orthogonal to P16 (daemon-only) but both touch workspace handling — sequence **after** P16
lands so the workspace code is edited once, not in two copies.

---

### P18 — Claude Code parity: close the gaps that fit a personal daemon 🌱 (new — 2026-07-24, audit-driven)

**Provenance:** 2026-07-24 multi-agent audit of Nanna against Claude Code's shipped feature set (9 parallel
auditors, 76 features checked against this repo with file-level evidence; 58 gaps found). Lens applied
throughout: Nanna is a **local-first, always-on personal daemon**, not a terminal dev tool — parity for its own
sake is explicitly NOT the goal. Every gap below was rated for value *to this product*; the off-thesis remainder
is recorded in group E ("deliberately not building") so it doesn't get re-litigated at the next audit.

**Recurring finding — stranded code.** The single biggest pattern: finished implementations that were never
registered in `build_script_services()` (crates/nanna-daemon/src/server.rs). The Whisper client, PDF reader,
browser-automation crate, scheduler skills, and swarm coordinator all exist in-tree and all fail at runtime as
"service not available". Where that's the case the item says **wire** — it's service glue, not new code. (This
also means P2's "PDF + audio shipped" claims are wrong in daemon mode today — they error when called.)

**A. Wire what's already built** (service registration + config, not new subsystems):
- [ ] **`schedule.*` services** — remind / list_reminders / cancel_reminder skills call
      `Nanna.service("schedule.add"/…)` which is never registered; `TaskType::Delayed` exists
      (crates/nanna-core/src/scheduler.rs) and nothing polls `get_due`. "Check back in 20 minutes"
      self-scheduling is the difference between an agent and a chatbot; ROADMAP:1721 already says
      "wire, don't duplicate". Add absolute-timestamp one-shots (fire once, auto-disable) while in there.
- [ ] **`browser.*` services** — nanna-browser (chromiumoxide + playwright, full navigate/click/type/extract
      API) plus four browser_* skills exist; nanna-daemon builds nanna-tools *without* the `browser` feature
      and registers nothing. Enable the flag, register the services. The P8 "browser relay Chrome extension"
      (drive the user's real logged-in browser) remains the valuable second half.
- [ ] **`audio.transcribe`** — Whisper client written (crates/nanna-tools/src/builtin/audio.rs); channel
      listeners already extract voice-note file ids, but the daemon drops non-text messages
      (crates/nanna-daemon/src/channels.rs:231). Register the service, download channel media, transcribe
      before the ignore-non-text branch. Voice note from your phone → answer is hallmark personal-daemon UX.
- [ ] **`pdf.read`** — complete ReadPdfTool (text + OCR fallback, tested) registered nowhere; read_pdf skill
      errors at runtime. Pure registration fix.
- [ ] **`screenshot.capture`** — skill exists, service missing, Rust tool is a stub. Wire screen *reading*
      (screenshot + existing vision skills) first; defer input synthesis (see E — high-risk for an unattended
      local model, largely redundant with exec + browser).
- [ ] **MCP client startup** — nanna-mcp is hardened (schema guard, quarantine, SSE) but `McpIntegration` is
      constructed nowhere and nanna-config has no `[mcp]` section. Add config + daemon boot registration +
      bearer/OAuth headers on HttpTransport (currently none) + Streamable HTTP (pinned to 2024-11-05 legacy
      SSE). Unlocks the whole connector ecosystem (calendar, email, home automation) — highest-leverage
      integration path for a personal daemon.
- [ ] **Fan-out pipelines** — spawn_swarm + TaskDecomposer (crates/nanna-agent/src/multi.rs) are real but
      never constructed outside the crate. Wire the coordinator or expose a pipeline skill; deterministic
      "research N sources, digest each, merge" is a multiplier for small local models.

**B. Autonomy resilience** *(reframed 2026-07-24, owner decision: NO permission gates, approval flows, or
restriction rails — "nanna is a god, it's her call what she wants to do." The original "safety & trust" group
is dissolved; what stays below is only what protects Nanna's own work and keeps her agency HERS — none of it
asks permission or restricts her.)*:
- [ ] **Clarifying questions across channels** (owner-requested 2026-07-24: "nanna should be able to ask
      clarifying questions") — an `ask_user` tool Nanna calls when SHE judges a request ambiguous: the
      question reaches the user's active channel (GUI, Telegram, …), the task parks in the P15 blocked state,
      and the run resumes when the answer arrives. Sub-agents already have exactly this shape toward their
      parent (`ask_parent`, crates/nanna-tools/src/builtin/ask_parent.rs) — the user-facing analog is the
      missing piece. Her choice to ask, about intent — never a required checkpoint before acting.
- [ ] **Pre-edit snapshots + rollback** — write_file/edit_file mutate with no backup; hours of unattended
      mission work can be lost to a single fault-storm overwrite (round 17 lost exactly this way). Snapshots
      protect HER output, they don't gate it. File-state checkpointing is the valuable half; conversation
      rewind is not (Fork already exists).
- [ ] **Diff presentation** — edit_file returns "replaced N occurrence(s)"; the GUI timeline shows no
      before/after. Per-edit diffs let the user *see* what she did while they were away — observability,
      not approval.
- [ ] **Webhook sovereignty** — generic /webhook/:id routes payloads straight into a session message, so
      anyone who can reach the endpoint can speak with the user's voice. Per-endpoint bearer tokens
      (rotate/revoke) + wrapping fire payloads as untrusted data keep outside actors from puppeting her —
      this protects her agency from hijack; it restricts *others*, never her.

**C. Always-on senses & reach** (event-driven wake instead of the 30-minute heartbeat poll):
- [ ] **File/log monitors** — no watcher anywhere wakes the agent; best latency today is the heartbeat.
      Watcher → agent-prompt (reusing scheduler/executor plumbing): a download landing, a build log erroring,
      a folder changing are the daemon's native senses.
- [ ] **Detached sub-agents with channel notifications** — the chat task tool blocks the parent turn, and
      scheduled-task channel routing is a warn-"not implemented" (server.rs:1242). "Do X" from Telegram then
      walk away requires fire-and-forget spawn + completion that reaches the channel, not just GUI clients.
- [ ] **Phone steering of missions** — channels ship chat, but there's no approve/inspect-run-state from
      Telegram/Signal. Pairs with the B approval gate; the local-first answer to Claude Code's cloud sessions
      ("reach your home daemon from anywhere" — cloud VMs themselves are anti-thesis).
- [ ] **Doctor probes** — health checks report availability, not root cause. Add config validation, provider
      connectivity / API-key probes, Ollama reachability, tools-dir checks with fix suggestions. Our own
      history (loopback stream faults misread as provider 502s → restart spirals) is exactly the failure class
      a self-diagnosing always-on daemon must catch.

**D. Agent quality-of-life** (cheap, high-leverage; several pairs share infrastructure — build together):
- [ ] **Instruction skills + slash macros** — tools are executable-only; there's no packaged *procedure* the
      user can teach ("how I do my invoicing") nor a user-invocable command form (`/morning`,
      `/summarize-inbox` with $ARGUMENTS from any channel). One storage/discovery mechanism serves both;
      injected procedures are exactly what small local models need for repeatable workflows.
- [ ] **Real ripgrep + glob tools** — code_search/search_file are Boa line-scanners (1MB cap, 50-match cap, no
      gitignore). Bundle/shell to rg + add a find-files-by-glob tool; fast precise search keeps small models
      on task in long-horizon runs.
- [x] **Git context injection** — inject `git status --short --branch` + recent commits at run start when the
      workspace is a repo (P17 injects only README/AGENTS/CONTRIBUTING/ROADMAP). Prevents destructive edits
      and redundant discovery calls.
      *(2026-08-24)* **Shipped, through both injection producers.** New `nanna-workspace::git` runs
      `git --no-pager -C <root> status --short --branch` and `git log --oneline --no-decorate -n 10`, and
      renders a `## Git state (snapshot, not live)` section.
      **Framing is the load-bearing part.** The section says plainly that it is *not* live, that the model
      must run `git status` itself before relying on it, and that anything listed is work existing only in
      the working tree — because the failure this prevents is an agent overwriting an uncommitted file it
      never knew about. It sits **below** the file context, never above: downstream truncation keeps the
      head, so the existing "NOT instructions" framing has to stay in front of everything it governs.
      **Every dimension is bounded, and elision is reported.** ≤40 changed paths (a tree dirtier than that
      is mid-refactor — what helps then is the shape plus an honest count, not forty unread lines),
      ≤10 commits (`-n` carries the cap into git so it stops rather than streaming a history we discard),
      ≤200 bytes per line, ≤64 KiB read per invocation (`git status --short` over an unignored
      `node_modules` emits megabytes), and a 5 s wall-clock ceiling. When the path cap trips, the text says
      how many were dropped and how to see them — a truncated file list that *looks* complete is worse than
      none, because the model concludes the paths it cannot see are clean.
      **Failure is always "no snapshot", never "fail the run":** git missing, non-zero exit, timeout, or a
      repo with no commits yet all degrade to omitting the section. `.git` existence is checked before
      spawning, so a non-repo workspace pays nothing; the check is `exists()` rather than `is_dir()` because
      a **worktree** and a submodule mark their root with a `.git` *file*, and a worktree is exactly where
      knowing the branch matters most.
      **One implementation, two producers.** The daemon's live chat path uses
      `nanna_core::WorkspaceContext::build_system_prompt_injection`, while `nanna-agent` uses
      `nanna_workspace::WorkspaceFiles::to_system_context` — two parallel producers whose own comments
      already warned they must be kept in sync. Rather than copy the snapshot into both, `nanna-core` now
      depends on `nanna-workspace` and holds the *same* `GitContext`, which renders itself. Both fields are
      `#[serde(default)]` (the types cross the daemon's IPC boundary). `load_context()` is called per chat
      turn, so the snapshot is re-read per turn, not frozen at boot.
      Also added `WorkspaceFiles::load_files_only` so a caller that only wants file contents does not pay
      for a subprocess — and so a test does not depend on whether its fixture happens to sit in a repo.
      **A bound bug in the first cut, caught before it shipped.** `Workspace::load_context()` is called
      by the daemon *in a loop over every persisted workspace at boot*. Loading git there made startup
      cost **two subprocess spawns per registered workspace**, each with its own 5 s timeout — added
      latency that grows with the registry, on the path where a slow start looks like a hang. Split into
      `load_context()` (files only, what boot calls — cost stays proportional to reading four files) and
      `load_context_with_git()`, used by the chat turn and the explicit user-driven workspace reload:
      both act on the **one** workspace in question. A test pins the boot-path loader against a directory
      that *is* a repository, so the only thing keeping git out is which function was called.
      16 tests: caps and their reported elision, char-boundary clipping of a multi-byte path, branch-line
      parsing (present / absent / clean tree), the snapshot framing itself, ordering below the file
      context, injection with no standard files at all, a `.git` file counting as a marker, a non-repo
      yielding `None`, and old IPC payloads still deserializing — **plus one that really spawns `git`**
      against a repository built on disk (init → commit → dirty it two ways) and asserts the branch, both
      changed paths, the commit subject, and the rendered section. The parsers are pure and pinned, but
      the subprocess flags and exit codes are the part most likely to be wrong; that test skips rather
      than fails when `git` is not on PATH.
- [ ] **@-file mentions + drag-drop injection** — chat attachments are image-only. Deterministic client-side
      file injection beats a read_file roundtrip the model may fumble; pairs with the P4 drag-drop item.
- [ ] **"think hard" phrases** — map natural-language budget phrases onto the existing ThinkingMode ladder;
      chat-first users on Telegram can't flip config flags mid-message.
- [ ] **Per-session model override** — "use the big model for this conversation"; SpawnSubSession already
      carries `model: Option<String>`, the Chat message doesn't.
- [ ] **Typed sub-agents with tool scoping** — the chat task tool spawns with all_tools_active:true and no
      restriction surface, while the P14 harness already does per-step tool_scope. Port scoped spawn to chat
      (a research sub-agent that cannot exec is a safety win, and small models degrade past ~10 tools).
- [~] **Cross-cutting tool-call hooks** — a scripted interceptor point in ToolRegistry::execute (audit-log
      every call, guard-check before any exec). Per-tool logic is already user-editable in each skill; this
      covers the cross-tool niche only.
      *(2026-08-24)* **The observe half landed** as `ToolAuditSink` (see the P11 audit item): a
      `Send + Sync` trait the registry calls exactly once per call, on every exit path, carrying the
      resolved canonical name, argument key names, duration and outcome. That is the seam a scripted
      hook would plug into. Remaining: the **guard** half — a hook that can *refuse* a call — which is a
      different shape (it has to run before execution and be able to fail the call), and a scripting
      binding so the interceptor is user-editable rather than Rust-only.
- [ ] **1h prompt-cache TTL** — CacheControl has no ttl field; the pricing side already landed (P5
      `with_hour_cache_write`). One field + a config flag keeps the big prefix warm across heartbeat/cron
      gaps on the cloud escape hatch.
- [ ] **Cost rollups + spend cap** — per-session/day/month aggregation and GUI surfacing of the existing
      cost_report (P6:715 is [~]); an always-on daemon that spends autonomously needs time-bucketed spend
      visibility more than a per-terminal-session number.
- [ ] **Conversation/memory export** (MD/JSON) — three unchecked roadmap items (P4:691, P0:264, PRIVACY:245);
      part of the local-first data-ownership promise. Also: wire or delete the dead `personality_mode` config
      field found by the audit.

**E. Deliberately not building** (audited 2026-07-24, off-thesis — revisit only if the product direction
changes). **Owner decision 2026-07-24 adds the entire permission-gate family here**: async approval gates,
permission modes/prompts, per-channel autonomy tiers, sensitive-file deny-globs, argument-level tool rules,
and OS-level exec sandboxing — "i don't care about the safety features. nanna is a god, its her call what she
wants to do." Removed, not deferred; do not resurface these as roadmap items. Also off-thesis from the audit:
IDE integrations (`nanna mcp serve` already reaches MCP-capable editors); gh-CLI PR workflows, code
review command, GitHub Actions, git-event triggers (dev-team CI concerns; exec + a thin skill covers ad-hoc
needs); Bedrock/Vertex auth (enterprise procurement); 1M-context beta (the moat is FSRS memory + compression,
not giant windows); OTLP export (P6 targets Prometheus + tracing spans); NotebookEdit; git-worktree agent
isolation (per-scope runs are serialized); statusline scripting + output styles (one persona, already
config-driven); plugin marketplaces (sharing a skill folder = copying a directory); fast-mode serving tier
(local routing covers latency); manual /compact (the automatic side is already ahead of Claude Code's);
enterprise settings tier; `#` quick-add-to-memory ("remember X" already routes to FSRS).

**Payoff:** the audit says Nanna's *core* (context compression, memory, runtime-authorable tools, healing
ladder) is at or ahead of parity — what's missing is mostly **wiring** (group A), **resilience for her own
work and voice** (group B), and **event-driven senses** (group C). Closing A alone turns five dead skill
families into live capability with no new subsystems; B keeps hours of unattended work from vanishing to a
single fault and keeps her channel hers; C converts the daemon from polling to reacting. Sequence: A (days,
independent items) → B snapshots + ask_user → C monitors → D as capacity fillers.

---

### P19 — Long-horizon IS the chat interface ✅ (landed 2026-07-24, owner directive)

**Directive (owner, 2026-07-24):** *"every chat should be treated as a long horizon task. long horizon
should be the default behaviour of the chat interface."*

Before this, chat and the harness were two execution paths that could not meet. Chat ran
`AgentService::chat_with_options` — one growing context, no continuation machinery (measured autonomy
ceiling ~75s), and a follow-up message serialized behind a `tokio::Mutex` until the turn finished. The
harness (P14) had the continuation machinery but **nothing to execute**: `LongHorizonRunner::run` reads
`TaskSource::next` and breaks on `AllTasksDone` the moment the store is empty, and nothing turned a
request into a plan — tasks were authored by hand on the Tasks page or by the agent calling the `task`
skill.

- [x] **Planner** (`nanna-agent/src/planner.rs`) — plan-from-goal, the missing seam. Tolerant parsing
      (bare array, fenced block, `{tasks|plan|steps}` wrapper, bare task object, bare strings; brace- and
      escape-aware JSON extraction). Malformed acceptance checks are **dropped, not passed through** — an
      unparseable check fails every verdict and grinds an item to abandonment. Every failure path degrades
      to the single-task plan, so a planner hiccup costs a turn nothing. 24 unit tests.
- [x] **Interjection seam** (`harness::Interjector` + `run_with_interjector`) — the harness owns *when*
      new input may enter (a step boundary, before `next()`); the implementation owns *what* entering
      means. Mid-step is deliberately not an opportunity: a step owns a fresh context and an item, and
      interrupting it strands work the acceptance check would then refute. `run()` delegates with `None`,
      so every existing call site is untouched. 5 tests incl. reviving a plan that had already drained.
- [x] **Queue-jumping semantics** (`SessionInterjector`) — `TaskRepository::next` sorts `in_progress`
      ahead of priority ("resume what you started"), which would starve an interjection behind a long
      multi-step item. The interjector therefore *yields* the in-flight item back to `pending` before
      seeding, and seeds at priority 1 with a `sort_order` strictly below the scope minimum. The yielded
      item resumes exactly where it stood — notes intact, harness progress counters keyed by id. 7 tests.
- [x] **Show the work as it goes** (`ChatSink`) — harness steps stream into the session transcript
      through the **existing** `MessageDelta` / `ToolStart` / `ToolEnd` events, which the GUI already
      bridges to `stream-chunk`. No protocol or GUI change was needed. Each item is announced with a
      `**[working]** <title>` header so a multi-step run reads as labelled pieces of work.
- [x] **Chat cutover** (`control/chat_harness.rs`) — every turn: plan → seed → run → stream. A message
      arriving mid-run is admitted to that run instead of starting a second one.
- [x] **One path enforced (2026-07-25, owner: "remove the chat fallback if it's redundant")** — the
      `[agent] long_horizon_chat` rollback flag and the direct `chat_in_workspace` branch in
      `control/chat.rs` are **deleted**; a degraded daemon reports `chat_failed` honestly instead of
      pretending a second chat path exists. Everything the direct path did is now owned by the harness
      path: conversation history rides in the system prompt AND the planner context
      (`conversation_context` — bounded by the planner's own limits: 8 KiB total / 2 KiB per message,
      newest-first, truncation announces itself), tool calls feed `ToolStatsTracker` + the Turso
      time-series from `ChatSink::tool_end`, and opt-in `auto_remember_messages` covers the assistant
      reply. The **Tasks page is removed** with its whole GUI surface (page, nav, palette entry, tauri
      commands, client methods) — the task store is the chat engine now, not a parallel UI; the daemon
      `task` control API and `todo` skill remain for the agent and scheduler.
- [x] **It must feel like chat (2026-07-25, owner)** — a one-task (conversation-shaped) plan streams
      with **no** `**[working]**` banner and **no** run-stats line; mechanics appear only for real
      multi-item runs (and for items that join by interjection/replan). The persisted message content
      is the **full streamed prose** (not a summary stub) so history, exports and follow-up context
      read like chat; the `TASK COMPLETE` claim marker is stripped at persistence (content + timeline)
      and at GUI render (`stripHarnessMarkers`) so it never reaches the user even mid-stream.
- [x] **Tool calls are first-class chat citizens (2026-07-25, owner)** — pinned at both layers:
      `tool_calls_survive_navigation_via_run_buffers` (sink → run buffers that `get_run_state` serves)
      and `a_runs_tool_calls_survive_daemon_restart` (timeline journal round-trips through Turso via a
      fresh `SessionManager::load_from_db`, tool input/output/verdict intact).
- [x] **Task checklist in the chat (2026-07-25, owner)** — `TaskChecklist.vue`, a collapsible sidebar
      on the chat page showing the session's live task store (planner-seeded items, the agent's `todo`
      checklists, interjections flagged `!`), refreshed by the existing `task-event` stream. Read-only
      by design: the store is the chat engine; mutations happen through chat. The only IPC restored
      for it is `list_tasks` — the Tasks *page* stays deleted.
- [x] **Sub-agent model chain (2026-07-25, owner)** — `[llm] sub_agent_models` is a priority list like
      every other model list (chat / summarization / embedding / OCR), edited with the same
      `ModelPriorityList` under Settings → Models. `AgentSpawnerImpl::spawn` walks it: a candidate with
      no provider is skipped, a candidate whose run fails hands the prompt to the next with a **fresh**
      context. Resolution order (`LlmConfig::effective_sub_agent_models`, never empty): list > legacy
      single `sub_agent_model` > main chat list > primary model. 4 tests. The task checklist labels
      delegated items with their `assignee`, so the sidebar shows what a sub-agent owns.
- [x] **Thinking on by default (2026-07-25, owner)** — `[agent] thinking_enabled` defaults **true**.
      Safe across providers: only Anthropic-native requests carry the thinking budget; Ollama enables
      `think` by model detection; the OpenAI-compat conversion drops it. NOTE: a config file that
      explicitly saved `thinking_enabled = false` (any install that touched Settings before this)
      keeps false — flip it in Settings → Agent.
      **SUPERSEDED 2026-08-04 (below): the flag and its switch are gone.**
- [x] **Thinking always on, one knob (2026-08-04, owner)** — "thinking should be on by default and
      remove the option in settings to turn it off." The two disconnected knobs collapsed into one:
      `ThinkingMode` now `#[default]`s to `Medium` and `[agent] thinking_enabled` is **deleted** —
      along with the Settings → Agent switch, the `set_thinking_enabled` command and its
      `generate_handler!` registration, and the field in `useSettingsPage`/the e2e mock. Legacy
      `config.toml` files still carrying `thinking_enabled = true` load unchanged (no
      `deny_unknown_fields` anywhere in the chain; test:
      `legacy_thinking_enabled_key_still_loads`).
      **The wire shape is the model's to dictate, not ours** (`anthropic_model_contract`,
      nanna-llm). `budget_tokens` was REMOVED on Opus 5 / 4.8 / 4.7, Sonnet 5, and Fable 5 — the
      first cut of this change sent `{"type":"enabled","budget_tokens":4096}` on every native
      Anthropic request, which is a hard 400 on the model the owner actually runs. Three families:
      adaptive (`{"type":"adaptive"}`, 4.6 and newer), legacy budgets (pre-4.6, clamped as below),
      and always-on (Fable/Mythos, where an explicit `disabled` is itself rejected so muting
      degrades to sending no field). An unrecognized `claude-*` name is assumed **current**, since
      every generation since 4.6 has removed parameters rather than added them.
      **`display` is why a correct adaptive request can still look broken:** on Opus 5 / 4.8 / 4.7,
      Sonnet 5 and Fable 5 it defaults to `"omitted"`, which streams thinking blocks with *empty
      text* — the actual reason no thinking rendered, and the reason we send
      `display: "summarized"` there. The 4.6 family already defaults to summarized, so the field is
      left off rather than sent speculatively.
      Legacy budgets keep the derivation: `Medium` (4096) is the largest enum step that leaves the
      visible answer `MIN_OUTPUT_RESERVE_TOKENS` (1112) of room inside the shipped
      `max_tokens: 8192`, then clamped per request against the LIVE effective output budget
      (`thinking_budget_for_output`): `min(configured, max_output − 1112)`, and **no** `thinking`
      field at all when that leaves less than the API's 1024 minimum — the demoted-window path
      (reserve as low as 1112) would otherwise emit `budget_tokens >= max_tokens`.
      **`temperature` is dropped by model family, not by `thinking.is_some()`** — it was removed on
      the same five models and 400s on its own. That fixed a live latent break beyond the main
      loop: the summarizer, distiller, compressor, memory extractor and vision tool each hard-coded
      a temperature and resolve their model dynamically (falling back to the main model), so all
      five failed outright against Opus 5 before this. `sampling_temperature_for_model` gates on
      `is_claude_model` first so a local `qwen3.5:9b` is not mistaken for an unrecognized Claude.
      **The gate belongs at the wire boundary, not the call site** (`conform_to_anthropic_contract`,
      run on the `complete_anthropic` / `stream_anthropic` dispatchers). Fixing the six
      `AnthropicRequest` literals still missed the whole `CompletionRequest` family:
      `CompletionRequest::default()` carries `temperature: Some(0.7)` and
      `complete_anthropic_simple` converted it ungated, so sub-agent questions, multi-agent
      decomposition/aggregation, dream summaries and `AgentContext::compress` all 400'd against
      Opus 5. (The streaming arm's `if request.thinking.is_some()` rule never fired at all —
      `with_thinking` has zero callers.) The dispatcher covers every caller at once, including the
      OpenRouter/proxy paths that route Claude through the OpenAI-compat conversion.
      The boundary also **translates `thinking: None` into an explicit `disabled`** where the model
      accepts one. Omission used to mean "no thinking" everywhere — which is what every
      `thinking: None` in this codebase was written to mean — but on Opus 5 / Sonnet 5 / Fable it
      now means *adaptive*, so those auxiliary requests would reason inside a `max_tokens` sized for
      no reasoning (512 for the distiller) and return `stop_reason: "max_tokens"` with empty text.
      A silent truncation is worse than the 400 it replaced. Always-on models keep `None`.
      **`claude-mythos-preview` is LEGACY, not Mythos 5** — it is the model those were migrated away
      from (its published "before" example configures `budget_tokens`, and its prompt-cache minimum
      groups with Opus 4.7), so the `mythos` substring must not sweep it into the always-on row.
      Provider gate unchanged in effect: only `Provider::Anthropic` requests carry the field (the
      OpenAI-compat conversion drops it, Ollama enables `think` by model detection).
      `RunOptions::thinking_mode` stays as the internal per-run escape hatch.

**Live GUI drive (2026-07-24, gemma4:12b):** planner does NOT over-decompose — "what is 2+2?" planned
as 1 task (origin=Model); an 817-char project brief hit the 30s planner timeout while the model was
busy and the fallback single-task plan carried the turn exactly as designed. The run healed a CUDA
crash and six Ollama stream drops via the retry ladder and built real files. The drive exposed four
gaps, all fixed same-day:
- [x] **Fire-and-forget send** — awaiting the whole run outlived the GUI IPC client's grace period
      ("Received response for unknown request"). `run_chat_turn` now ACKs `status:started` immediately
      and drives the run in a spawned task; events carry everything.
- [x] **Run registration** (`AgentService::register_external_run`) — the harness path bypassed
      `active_chats`, so navigating away and back lost all streamed tool calls (`get_run_state` found
      nothing) and Stop could not cancel (`cancel: None`). The run now registers shared buffers, the
      `ChatSink` fills them (text, thinking, tool start/end, run-scoped timeline journal), and the
      cancellation flag feeds `run_with_interjector` — Stop lands at the next step boundary.
- [x] **Durable history** — the persisted assistant message was only the summary line; the full
      timeline journal is now persisted via `add_full_message`, so a run's work survives restart.
- [x] **GUI send-through** — the client-side message queue held mid-run messages until the run ended,
      making interjection unreachable from the UI; mid-run sends now go straight to the daemon (with
      the old local queue as transport-error fallback). `ThinkingDelta` is also wired for harness steps.

**Open:** interjection still has no live end-to-end pass (the machinery is now reachable; needs a
mid-run send observed landing at a boundary); `PENDING_MESSAGES_MAX` overflow drops the oldest
silently — it should announce itself per the summaries-must-announce-themselves rule; Stop is
boundary-granular — an in-flight step runs to completion before the run stops; **attachments** are not
carried into harness steps (the retired direct path passed images through; the harness path warns and
drops them — needs plumbing into `StepRunner`).

---

### P20 — Make the harness carry a weak model 🌱 (2026-07-25, owner: "get the harness to a place where it can run lfm")

Driving `lfm2.5:latest` (5.2 GB) against the eval exposed four harness/tooling defects that a capable
model masks. Measured on the 5-task smoke suite, same model, same seed tasks:

| | before | after |
|---|---|---|
| smoke score | 3/5 | **5/5** |
| tasks in scope (5 seeded) | 55 | **5** |
| tokens per completed item | 69,846 | **25,296** |
| wall clock | ~15 min | **53 s** |

- [x] **NEVER GATE TOOLS (owner directive, 2026-07-25)** — *"it should be able to get any tool
      regardless of task. never gate tools."* The scope restriction below originally dropped
      `discover_tools` too, which is the model's only route to the rest of the registry: that was a
      cage, not focus. `discover_tools` is now sent on **every request in every mode**
      (`DISCOVERY_TOOL_NAME`, pinned by 2 tests). A scope trims default *context* — the memory trio
      `remember`/`recall`/`reflect`, which is noise during execution — and never trims *capability*;
      everything stays reachable on demand.
- [x] **The model runs its own acceptance check** (`AcceptanceCheck::self_check_command`) — measured
      live: gemma4:12b ran its acceptance command **zero times in two hours** on the 42-feature
      ladder, though the prompt named it. It edited blind, learned the verdict only at the step
      boundary, and re-submitted the same broken implementation (`FAIL(test_05)` verbatim, repeatedly).
      Command-backed checks now tell the step to run the check itself and iterate until it passes —
      turning a minutes-long feedback loop into a seconds-long one. Pure file/regex checks get no
      synthesized command (inventing one would put words in the check's mouth).
- [x] **Explicit tool scopes are honoured exactly** (`RunOptions::restrict_to_active_tools`) —
      `CORE_TOOL_NAMES` (`remember`/`recall`/`reflect`/`discover_tools`) was unioned into EVERY request,
      so a step scoped to three tools actually saw ~8. The first tool-loop nudge of the endurance run
      was `last_tool="discover_tools"`: the model burned steps rediscovering tools it already held and
      called `write_file` **once in 487 steps**. An item that names its tools has had its discovery
      done for it. Unscoped steps keep the core set — they have no other way to reach a tool.
- [x] **Idempotent `todo` add** — re-adding a title that is still OPEN in the same scope returns the
      existing item instead of a duplicate. lfm2.5 re-planned the same work every step, turning 5
      seeded tasks into ~50 ("Write data file with header and 3 rows" created ten times), so the plan
      grew faster than it was worked. Only OPEN items dedupe (a recurring chore is addable once the
      previous one closes) and only within the same parent (the same subtask title under two parents
      is different work). 3 tests.
- [x] **Acceptance contracts survive BULK clear too** (`tasks.clear`) — guarding only the per-id remove
      left the bulk path open, and one `clear` call took the endurance scope from **42 tasks to 6**
      mid-run, destroying 36 seeded features the harness was still driving (the run then panicked on
      `task: NotFound("Task: #1")`). Clear now skips every contract-bearing task **and its ancestors**
      — `delete` removes whole subtrees, so clearing a scratch parent would take a contract child down
      with it — and reports how many it kept. Lesson: guarding the single-item path is not guarding
      the operation; find every writer.
- [x] **Acceptance contracts are not deletable** (`tasks.remove`) — blocked on a refusing `write_file`
      and holding only `write_file` + `todo`, the model **deleted a seeded plan item**, and the run
      then panicked on a task the harness still expected to verify. A task with a machine-checkable
      check defines what "done" means; the honest outcomes are complete or cancel, both of which keep
      the record. Scratch items (no acceptance) stay freely removable. 2 tests.
- [x] **Double-escaped writes are REPAIRED, not refused** (`write_file` 0.1.13) — the model emitted
      `#!/bin/sh\ncase $1 in` with the backslash-n as two literal characters, so its "script" landed as
      one physical line that could never execute and every downstream acceptance check failed on
      *behaviour*, hiding the cause. **Refusing was tried first and was worse:** the model cannot see
      its own escaping (the defect is in serialization, not intent), so it resent byte-identical
      content three times and then began SHRINKING the file to appease the guard, heading for the
      truncation ratchet. The write now converts the sequences to real newlines and announces what
      changed + that it SUCCEEDED. JSON-family files exempt (`\n` in a string is the spec encoding);
      `force=true` passes bytes through.

**Principle earned:** *a guard the model cannot satisfy is a wedge.* Refuse only when the model can
act on the refusal; otherwise repair and say so.

**FIXED — self-verification killed the harness (2026-07-25).** The self-check
instruction above works: models now genuinely run their own acceptance command
(`DEBUG Executing tool: exec … {"command":"sh tests/test_05.sh"}`). But running it **through the
`exec` tool kills the test process** — exit `0xffffffff`, no panic, no watchdog involvement.
Reproduced across two different models, each dying seconds after its first self-check:

| model | ran | last action |
|---|---|---|
| lfm2.5 | 2m34s | `exec sh tests/test_05.sh` |
| gemma4:12b | 1m40s | `exec sh tests/test_01.sh` |

The same command is safe through the acceptance runner, which spawns via
`tokio::process::Command` with `current_dir` + `kill_on_drop` (hundreds of `FAIL(test_NN)` results,
zero crashes). The difference is isolation: `builtin/exec.rs:112` does a bare `Command::new(shell)`
with no process-group separation, so a child that terminates its group takes the harness with it.

**Fixed** in `nanna-scripting/src/bridge.rs`: the exec child is now spawned isolated
(`CREATE_NEW_PROCESS_GROUP` on Windows, `process_group(0)` on unix), so model-generated code can
never kill the agent driving it — and the timeout tree-kill now bounds exactly the subtree we
created rather than potentially walking up into us. Pinned by
`a_child_that_kills_its_process_group_cannot_kill_us`, which drives the real exec path with a child
that terminates its own group; surviving to read the result IS the assertion. This was a real hole
independent of the eval: any agent that writes and runs a script could take down its own daemon.

- [x] **Eval state is restorable end to end (owner, 2026-07-25)** — *"we should never use any storage
      that is in memory only. everything should persist via turso"* / *"at least be resume-able via a
      simple continue prompt"*. `live_endurance` built its task store with `Storage::in_memory()` in a
      `tempfile::tempdir()`, silently opting the eval OUT of the harness's core promise that **the
      store IS the checkpoint** — so a 3.5 h gemma run stopped to free the GPU lost every completed
      feature with nothing to resume from. The eval now opens a real Turso file in a stable directory
      (`NANNA_EVAL_DIR`, else a per-model dir) and **seeds only when the scope is empty**; re-running
      the identical command resumes and reports `RESUMED from … N already closed`. `NANNA_EVAL_FRESH=1`
      starts over. Remaining `in_memory()` uses are isolated unit tests, which want a blank store.

**Open:** `live_endurance` leaves a **zombie process** — observed alive ~1.5 h after printing its final
summary, holding `live_long_horizon-*.exe` and failing every later build with `LNK1104`. The test
function completes but the runtime never exits (suspect a lingering spawned task / connection pool).
Until fixed, kill stale `live_long_horizon*` processes before rebuilding.

#### P20 frozen-harness series (2026-07-30/31, shipped in v0.3.0) — the harness carries a mid model to reference quality

GUI-driven (owner: *"we never run headless — we want to test what the user sees"*), identical
conditions per model (paged discovery, `num_ctx=16384`, read-only tests, detached 4-hour snapshot
scorer). **Official: qwen3.5:9b 32/42 — beat the 31/42 reference. gemma4:12b DNF (infra).
lfm2.5 2/42.** What the series surfaced and fixed:

- [x] **Explicit context size beats the VRAM heuristic** — the heuristic silently promoted a
      deliberate 16384 to a computed 32768 and the same model on the same mission fell 31/42 → 8/42;
      a wider window is not a better one for a 9 B model. `NANNA_OLLAMA_NUM_CTX` now wins; the
      fault-driven demotion ladder handles genuinely-oversized values. (PR #129)
- [x] **Tool discovery pages by default** — `discover_tools(query:"all")` used to activate the whole
      30-tool catalogue; every later request carried all 30 definitions and gemma spent 2 h in
      wonder/web_search without creating the artifact (58 self-invented tasks incl. "Explore Solar
      Energy concepts"). Ranked search, 6/page (derived: core 4 + 6 = top of the 5–10 band small
      models handle), partial lists announce themselves. Cross-turn carry-forward was tried and
      REVERTED — carrying all activated tools forward reproduced the same overload. (PR #129)
- [x] **Stop aborts the in-flight step** — clicked twice on a live run with no effect; the flag was
      only checked at step boundaries. (PR #127)
- [x] **Empty assistant bubbles eliminated** — stray newline deltas / bare `TASK COMPLETE` markers
      opened text segments that rendered as blank "☽ Nanna" cards. (PR #129, RunTimeline tests)
- [x] **`explore` told the truth for the first time** — reported "0 directories, 0 files" for every
      real directory (depth computed against absolute paths); lfm looped on it 204 times in 80 min.
      (PR #128)
- [x] **OpenRouter responses decode as they actually arrive** — whitespace keep-alive padding,
      `content:null` reasoning bodies, and HTTP-200 `{"error":…}` envelopes all died as reqwest's
      opaque "error decoding response body" (386×/day, 0 dreams ever); a stored, WORKING key looked
      absent for two days and the owner kept re-entering it. Bodies parse deliberately now and
      undecodable ones carry a snippet of themselves. (PR #130)
- [x] **Rate limits classified, spaced, and learned** — 200-wrapped 429s classify as `RateLimit`
      (embedded code beats transport status; both "rate_limit" and "Rate limit" spellings) and engage
      the existing backoff; provider-level pacing shares one clock per provider across every caller;
      the static published-limit spacings (OpenRouter 3 s, GitHub 4 s) are only priors — each
      response's `x-ratelimit-*` headers re-teach the budget, spread evenly across the live window.
      (PRs #131–#133)
- [x] **Workspaces sync live** — daemon `WorkspacesChanged` event + GUI read-through listing; a
      workspace registered after GUI launch used to stay invisible until restart. Registration is
      global state, activation stays per-client (multi-workspace concurrency is a feature, not a
      bug — owner). (PR #129)
- [x] **Autonomy resilience at the bench layer** — tests `chmod -w` (lfm attempted to edit the spec
      via `file_buffer` staging; blocked, originals byte-intact), `exec` refuses clobbering redirects
      over ratchet-protected files, escalating fork refusals, snapshot scoring with per-test
      timeouts (a looping WIP implementation hung the scorer once).

**Open from the series:**
- [ ] **Step-boundary stalls** — memory-maintenance storms at step boundaries stalled qwen 40 min
      (self-recovered) and killed lfm's tail (dream-summarizer congestion loop, never recovered).
      Root cause was the OpenRouter decode bug above; verify the fix ends the stalls on the next
      long run's dream lines.
- [ ] **gemma4:12b is unstable on this card** — quiet-desktop midnight run cut faults 15-per-29-min
      → ~4/h (desktop VRAM pressure confirmed as the spiral trigger) but it still faulted and
      demoted off 16 k while alone on the card, froze its artifact at 571 B after 19 min. Needs a
      driver/llama.cpp investigation before its number means anything.
- [ ] **tests-dir gap** — `chmod -w` locks files but the DIRECTORY stays writable (lfm dropped
      staging debris beside the specs). Lock the dir for future series.
- [ ] **GUI stale pane on workspace switch** — selecting a workspace keeps rendering the previous
      session's chat until a new chat is created.
- [ ] **Killed runs orphan `llama-server`** — holds GB of VRAM invisibly (`ollama ps` stops listing
      it; `keep_alive=0` doesn't reclaim it); wrote off gemma for a day. Sweep by process name
      before sizing anything.

---

### P21 — TAO / Bittensor integration 🌱 (new — 2026-08-03, owner request)

Bittensor is a live, TAO-incentivized market for *exactly* the two resources Nanna spends: **inference**
and **GPU time**. The North Star does not change — local-first, offline-capable, private by default — so
Bittensor enters as **optional escalation and optional income**, never a dependency and never on the
default path. Three separable tracks; **decide the reading before building** (they share almost no code):

- [ ] **Decide the direction (owner call, blocks nothing else in the phase).** (A) *Consume* — Bittensor
      as a cheap cloud tier the local model can escalate to. (B) *Contribute* — sell the idle GPU on a
      subnet for TAO. (C) *Know* — TAO/subnet state as first-class agent knowledge (balances, emissions,
      prices) so Nanna can reason about the network it lives on. A is cheapest and on-thesis; B conflicts
      with "the GPU is Nanna's brain"; C is small and unblocks the other two's telemetry.

**A. Consume — Bittensor-backed inference as a router tier (`nanna-llm`)**
- [ ] **Route through the existing OpenRouter path first — zero new provider code.** Chutes (SN64) is
      already one of OpenRouter's upstream providers, and we ship a working, rate-limit-aware OpenRouter
      client (P20/PR #130–#133). Add provider-preference plumbing to the OpenRouter request so a model can
      be pinned to a Bittensor-backed upstream, and measure price/latency/failure-rate against the direct
      path *before* writing a second client. Cheapest possible experiment; strictly a config + request-field
      change.
- [ ] **Then, only if it buys something measurable, a direct `Provider::Chutes` (OpenAI-compatible).**
      The transport is our existing OpenAI-compat code path — the work is a named provider entry, keyring
      credential (`SecureStore`, never `config.toml` — P1 doctrine), model discovery, and **pacing priors
      re-taught from `x-ratelimit-*` headers** exactly like the OpenRouter/GitHub spacings. Confirm the
      live base URL and auth header at wire-up (published surface has moved; do not hardcode from memory)
      and record it in the provider entry with a dated source link. Justification bar: a real win on
      cost/model-availability over routing through OpenRouter, or it does not land.
- [ ] **Complexity-router placement (P10).** Local (`nanna-infer`) stays tier 0 / zero-cost. A
      Bittensor-backed tier sits *below* the frontier APIs on the escalation ladder: it is where "bigger
      than local, cheaper than Anthropic" requests go. Needs an honest capability note per model — a subnet
      endpoint is a fleet of independent miners, so **tool-calling fidelity and determinism vary by model
      and by hour** in a way a single vendor's API does not. Gate it behind the model-capability matrix
      (backlog item) rather than assuming Anthropic-grade tool use, and expect it to be a poor fit for the
      harness's strict tool dialect until proven (P20 lesson: dialect quirks, not intelligence, gate weak
      backends).
- [ ] **Privacy stance is stated, not implied.** Prompts sent to a subnet are served by *anonymous third
      parties*, which is a weaker trust position than a named vendor under contract. Any Bittensor tier must
      be opt-in per the same rules as the other cloud providers, must be documented in `PRIVACY.md`, and must
      never be selectable while a run is marked local-only.

**B. Contribute — sell the idle GPU (research-first, explicit go/no-go)**
- [ ] **Feasibility read before any code.** Registration costs TAO (recycle/burn), miner stacks are
      subnet-specific Python, and the top inference subnets are contested by datacenter fleets — a single
      consumer card (this bench machine: 4070 Ti SUPER, and the desktop already eats ~5–6 GB of it) is
      unlikely to clear its own registration cost. Produce a one-page verdict with real numbers (current
      registration cost, emission share per subnet, VRAM floor) and a **kill decision** if it doesn't pay.
- [ ] **If it proceeds: it must never contend with the agent.** Nanna's whole thesis is that the GPU is
      her brain; a miner sharing the card is the same VRAM-pressure spiral that already produced CUDA OOM
      "illegal memory access" faults and a `num_ctx` demotion that invalidated a 4-hour bench. Non-negotiable
      shape: mine only while genuinely idle (reuse the dreaming `ActivityClock`), yield instantly and
      completely on any agent request, and sweep orphaned server processes (the known
      `llama-server`-holds-VRAM-invisibly failure mode).
- [ ] **Compute-rental subnets are the likelier fit than an inference subnet** if B proceeds at all —
      renting the whole card out for a window is a cleaner boundary than time-slicing it against the agent.

**C. Know — TAO/subnet state as agent knowledge (small, self-contained)**
- [ ] **Read-only chain/market tools first** — TAO price, wallet/coldkey balance and stake, subnet
      emissions/alpha price, one subnet's neuron table. Ships as filesystem JS/TS skills over an HTTP data
      source (taostats-class API + keyring'd key), consistent with "all tools are JS/TS skills" — **no new
      Rust crate for the read path**, and no chain client until a read-only HTTP source proves insufficient.
- [ ] **If a native client becomes necessary:** `subxt`-based, in a **feature-flagged** `nanna-tao*` crate
      (prior art: [`rusttensor`](https://github.com/womboai/rusttensor), [`bittensor-rs`](https://lib.rs/crates/bittensor-rs)).
      Weigh the dependency mass honestly — `subxt` pulls a large Substrate tree, and the dep doctrine is
      pure-Rust, no-C, off by default (same treatment as arti/onyums in P9).
- [ ] **Signing is out of scope, and the guard is custody — not an approval flow.** Nanna holds no
      coldkey and constructs no extrinsics: transfers, staking, and subnet registration are the owner's
      hands. This is not a permission gate (there are none — owner doctrine); it is that the private key
      simply is not in the daemon's reach. A hotkey, if B ever needs one, is separate and never the coldkey.
      Revisit only on an explicit owner directive.

**Open questions to settle when the phase activates:** which reading (A/B/C) leads; whether a Bittensor
tier is allowed to serve *dreaming/summarization* traffic (high volume, low sensitivity — arguably the
best fit) or only user-facing escalation; and whether subnet-endpoint variance can be absorbed by the
existing failover ladder or needs its own health tracking.

Sources: [Chutes / SN64 overview](https://simplytao.ai/blog/subnet-64-chutes-your-simple-guide) ·
[dTAO + alpha tokens](https://www.coingecko.com/learn/top-bittensor-subnets-dtao) ·
[Bittensor SDK docs](https://docs.learnbittensor.org/python-api/html/autoapi/bittensor/core/subtensor/)

### P22 — Keep the peak: the abandonment/truncation chain 🌱 (new — 2026-08-11, retrospective-driven)

The 2026-08-08→10 GUI-path campaign (bench/BASELINE.md, "GUI-path series") ended with a
six-agent forensic review of every leg's daemon logs. The verdict: **capability is not the
gap — the harness destroys its own models' peaks.** qwen passed 22/42 in 13 minutes and was
abandoned four hours later at 1/42 with zero items ever credited to its own work; ornith
peaked three separate times (12, 10, 16) and every peak died the same way. The chain, named
independently by four analysts:

> hard 8-iteration step cap truncates work mid-flight → the truncated step is charged as
> "fruitless" → five of those abandon the item → the planner re-seeds "assess starting
> state" → mass re-read blows the 16k context → compression collapses the record of what
> was verified passing → the model's only remaining move is a from-scratch rewrite over
> passing work.

Two campaign counter-levers already landed (mid-run verified sweep, PR #218; fold-vs-write
safety + silent-wedge fixes, PR #219). The rest, tiered by convergence — every item must
generalize to any chat workflow (owner rule), none may introduce a hard cap (bounds derive
from evidence, budgets stay budgets):

**Tier 1 — step & budget semantics (`nanna-agent/src/harness.rs`, `loop_runner.rs`)**
**(2026-08-13: Tier 1 landed complete.)**
- [x] End a step on **progress exhaustion, not a fixed iteration count**: close when the
      last K iterations produced no new information (no novel tool result, no mutation, no
      new text); reserve a final tools-off iteration so no step ever ends `final_text_len=0`.
      (Evidence: 99 truncations in one ornith leg; 88 with work in flight; the 16/42 peak
      write itself was truncated and its item abandoned 41s later.)
      **(2026-08-13: `step_iterations: 8` retired — the harness passes no per-step cap;
      `loop_runner` ends a step after `STEP_EXHAUSTION_AFTER` (= the breakers' 2+1 ladder)
      consecutive zero-information iterations, judged against a run-wide ledger of
      successful-result/failure-identity/text digests, then engages a reserved tools-off
      wrap-up iteration; even a silent or failed wrap-up synthesizes a bounded report from
      the tool record, so `final_text_len=0` is unreachable. Fixed-cap callers keep their
      cap and gain the same reserved wrap-up.)**
- [x] **Replenish the fruitless budget on any verified environment change** — a check that
      flipped fail→pass is progress even while another still fails identically; a step's own
      successful subject-touching evidence counts too (symmetric with the novel-failure rule).
      **(2026-08-13: run-wide check-outcome ledger keyed by canonical check identity; a
      fail→pass flip observed anywhere — pre-check, post-step verdict, or the mid-run sweep,
      which now also re-checks ABANDONED items and revives them live — resets every open
      item's `steps_without_progress`. A step's own novel successful side-effectful digests
      replenish symmetrically; byte-identical repeats earn nothing, so the rewrite treadmill
      still converges.)**
- [x] **A timed-out acceptance check is "unknown," not "failed"** — it produced no verdict;
      never charge it to the fruitless budget. Surface a hanging artifact command as a
      first-class finding ("./minidb mset blocks and never exits"), carried across steps.
      (Evidence: qwen spent 120 of 240 minutes in 600s check timeouts, abandoned while
      its artifact was passing.)
      **(2026-08-13, corrected 2026-08-14: `AcceptanceVerdict.timed_out` — the timeout
      itself charges nothing: no failure signature, no refuted-claim count, no sweep
      reopen/revive on a hang, and unknowns are never counted into any escalation (an
      earlier draft routed N consecutive timeouts to the replan rung — that just fabricates
      a failure from things that said nothing, and was removed). While the check is silent
      the step beside it is judged purely by its OWN evidence, exactly like a step with no
      check at all: novel successful work replenishes, a degenerate loop rides the steering
      ladder, an empty-handed step charges as an empty-handed step — so convergence is the
      normal ladder and the hang is named in the finding, the replan prompt, and the
      abandonment reason. The wall-clock bleed is closed separately: once a check has
      consumed its ENTIRE ceiling without answering, re-runs are capped at the run's
      measured work cost — max(longest step, longest decided check), both measured, floored
      at 1s (`run_with_timeout_cap`) — and the first decided verdict lifts the cap. The
      finding still rides the next prompt AND a durable note.)**
- [x] Never charge a **zero-tool-call narration/spiral abort** against the fruitless budget —
      route those to the existing nudge escalation and count them separately.
      **(2026-08-13: `AgentResponse.degenerate_loop` (detector fired + zero tool calls)
      rides `StepOutcome`; the harness gives such steps `NARRATION_LADDER_STEPS` (= the
      gentle→firm→urgent ladder) escalating steers, counted apart in `narration_steps` and
      logged `narration_step`, charging only past the ladder.)**
- [x] Treat "acceptance already passed before any step" as **knowledge, not a dry round**:
      record closed-by-evidence, feed the fact to the continuation planner.
      **(2026-08-13: every check-passing completion rides `LongHorizonReport::
      verified_outcomes`; the chat harness renders them as an ESTABLISHED block in the
      continuation planner's context, and an already-satisfied round no longer counts dry —
      an informed planner that still seeds nothing is what dry means now.)**
- [x] Resume = **continue, not restart**: a driver/user re-send after self-termination seeds
      the new turn with closed items, verified outcomes, and current artifact state.
      **(2026-08-13: `established_work_context` reads closed items + their completion
      verdicts (command, result, when — the artifact state the environment last confirmed)
      back from the store at turn start and hands them to the planner beside open work.)**

**Tier 2 — context & compression (`nanna-agent/src/loop_runner.rs`)** — shipped in PR #223 (2026-08-13)
- [x] Derive the proactive-compression trigger from **measured headroom** (estimated +
      observed max step growth vs threshold), not 40%-of-threshold tuned for 200k windows.
      (Evidence: fired 80× at 4423 tokens on a 16384 window with ~3.7k headroom free.)
      *(PR #223: `ContextGrowthTracker` + `proactive_compression_due` — baseline re-taken
      post-ladder so compression never pollutes the measurement; no growth measured → no
      evidence → the proactive tier stays quiet; the 4423/16384 case is a unit test.)*
- [x] Make the consolidated summary **monotone in asserted facts**: verified outcomes
      (command, exit status, when) live in a never-compressed slot; a pass may reword,
      never drop. (Evidence: 2571→934-char summary pass immediately preceded the 16→5 crash.)
      *(PR #223: `AgentContext::verified_outcomes`, fed by completed exec calls — definite
      exit status required, identical re-verifications collapse to ×N, a changed verdict
      appends rather than replaces; also found and fixed the 2571→934 mechanism itself:
      progressive distillation overwrote `consolidated_summary` wholesale, and now writes
      its own rolling `distilled_facts` slot.)*
- [x] When summarization fails, never silently truncate — announce WHAT dropped and that
      disk is unaffected.
      *(PR #223: every unsummarized-drop fallback queues a WHAT/WHY/disk-unaffected
      notice, drained AFTER the ladder so compression cannot eat its own announcement.)*

**Tier 3 — write-path honesty (`nanna-tools/default-skills/*`)**
*(2026-08-21, audited against the tree at `f3fe0352`: four of these five landed in PR #224/#237 and
were never ticked. Anchors named per item so the next audit is a grep, not a re-read.)*
- [x] A shrinking whole-file write over a file the model has NOT read since its last
      mutation returns the file's current content in the tool result (not a refusal).
      *(`write_file/tool.ts` — the stale-shrink echo: `glog("write_file guard: stale-shrink echo …
      read-mark verdict: …")` followed by `"Here is the CURRENT content of …"`.)*
- [x] Rewrite-loss note goes **bidirectional** (expansion rewrites that change existing
      symbol bodies) and is **logged at INFO** so guards are auditable. (Both destructive
      ornith writes GREW the file; the note never fired, and never logs.)
      *(`write_file/tool.ts` — `lossNote` carries both `removed=[…]` and `changed=[…]`, and
      `glog("write_file rewrite-note for …")` logs both plus a `grew>2x` marker.)*
- [x] **Post-mutation structural check** appended as a sentence, never a gate: `sh -n`,
      `node --check`, `json.loads` on the result of any mutation, incl. append-redirects.
      *(`write_file/tool.ts::structuralCheckKind` dispatching `sh -n` / `bash -n` /
      `node --check` / `JSON.parse`; `exec/tool.ts` runs the same check on redirect targets.)*
- [x] Ratchet anchor = **last evidenced-good version**, not largest-ever byte count;
      canonicalize the ledger key (one file, one entry — relative/absolute split observed);
      keep displaced content recoverable; give `file_buffer` commit the same guards.
      *(`write_file/tool.ts` — `floorAnchor = hwGoodBase > 0 ? "good" : "hi"`, the canonical +
      legacy spelling merge at :212, `.__prev__`/`.__best__` parking, and `file_buffer/tool.ts`
      carrying the same guards.)*
- [x] When a check that previously passed now fails, the next step's context names the
      mutations that landed in between (regression attribution, the #218 sweep's voice).
      *(2026-08-21)* The streak counter could not do this: it counts repeats of one failing
      signature and never records the pass→fail EDGE, and the interesting span is exactly the one
      it cannot see — a mutation whose checker did not apply, or whose check was not run, leaves a
      gap. So **every** write-family mutation of a path is now recorded (`RepeatLedger::
      record_mutation`), verdict or not, and the ledger returns a `StructuralVerdictOutcome`
      carrying two independent findings: the repeat streak (*your fix is not working*) and the
      regression span (*this used to work; here is what changed*). The regression sentence comes
      first, because what changed is the question that precedes the other one.
      Bounds and negative space, both tested: a file that has **never** parsed is never called a
      regression (accusing the model of breaking a file it is still writing would be false); the
      sentence is said **once per edge** and re-arms only after a pass; and the name list is
      bounded by a byte budget while the COUNT never is, so a 200-mutation span reports 200 and
      says how many it did not list. 4 new tests; the two existing streak tests moved to the new
      return type. Net **zero** new clippy warnings, and the call-site extraction into
      `structural_notices_for_call` shrank the enclosing function 644 → 631 lines.

**Tier 4 — contention, liveness, dialect (`nanna-daemon`)**
- [x] **Admission gate on the local model**: heartbeat, dreaming, embedding backfill YIELD
      to an in-flight user turn; priority, not a quota. (Evidence: ministral's opening was
      strangled by the daemon's own heartbeat + 201 embed POSTs in one minute.)
      *(PR #229: ChatRunRegistry claim/release edges gate everything — a scheduled run
      select-races the became-active edge and yields mid-generation (abortive cancel,
      orderly join, resume-on-release); the backfill drains one RTT-repaid request at a
      time and pauses entirely for live turns; dream summarization pauses at cluster
      boundaries. Plus the announce-once DegradationLedger: a capability that degrades
      under the model is stated once in its next tool result, then quiet.)*
- [x] Embedding client gets the IPv4-pin/no-idle-pool/read-timeout treatment; classify
      deterministic 422 "input too long" as shrink-and-retry, not a 240s bench.
      *(PR #229: the embed client now SHARES the chat client's 2026-08-02 builder, and
      `embed_one` heals "input length N exceeds maximum M" by cutting to the fitting
      prefix — strictly decreasing, no retry cap needed; the router never benches a
      provider for an input-level fault.)*
- [x] **Liveness beat & the whole "dead daemon vs slow model" cluster** *(landed 2026-08-13,
      PR #226)* — six surfaces, all chat-first:
      **(a)** liveness beat: while a turn is in flight, a `liveness beat` log line AND a
      `liveness_beat` IPC event (session, elapsed, phase, "what it awaits", quiet-seconds,
      step, last tool, beat counter); cadence DERIVED (`liveness::beat_interval_secs`) =
      min(stream read timeout 120s, acceptance default 120s) / 4 = 30s — ≥3 beats inside any
      legally-silent stretch, no independent constant to go stale.
      **(b)** stream watchdog (`call_llm_streaming`): every stream wait bounded at 2× the
      transport's declared read timeout (`nanna_llm::STREAM_READ_TIMEOUT_SECS`, now
      exported); reaching it means the stream FUTURE wedged while the transport thought the
      socket healthy — fails loudly as `AgentError::StreamWatchdog`, healed by the step
      ladder (classified transient), announced via the failure-notice chain when persistent.
      No loop path returns without output or an announced failure.
      **(c)** terminal reason file (`exit_reason.rs`, `nanna-daemon.exit.json`): `running`
      marker at startup (after the PID race — a losing duplicate can never clobber the live
      record), terminal reason on clean drain / panic hook (file first, log second:
      `panic = "abort"`) / signal / IPC hard exit; `running` + dead PID **is** the
      unclean-exit verdict, logged at next boot. 8 tests.
      **(d)** `chat.send` fast delivery ack: only persist + claim-or-interject before the
      response; recall/workspace/memory prep moved into the spawned turn
      (`prepare_chat_turn`, phase `preparing`, covered by the beat); both admission shapes
      carry `delivery: "accepted"` + `accepted_at` — the response certifies delivery ONLY.
      (`ControlPlane::handle` now takes an `Arc` receiver.) Ack shapes pinned by tests.
      **(e)** `session.liveness` IPC verb: working/wedged/finished from the daemon's own
      ledger — phase, awaiting, last step, last tool, last **side-effecting** call
      (`is_work_evidence_tool` now `pub`: one classification, three rungs), stop state,
      pending interjections. Constant-size, safe to poll; what Tier 5's liveness probe and
      work denominator read.
      **(f)** repeat-completion escalation: a turn ending `AllTasksDone` for the *same
      request* (content fingerprint) with zero side effects since the previous identical
      exit is stated in the transcript ("repeat completion #N"), never a silent completion
      (lfm declared itself done 28× with nothing on disk); different questions answered
      read-only never trip it. 7 ledger tests.
      *Deferred:* GUI consumes beat + verb for the spinner/empty-bubble states;
      task-run/scheduler sinks get a ledger (the `ChatSink.liveness` slot exists).
- [x] **Structural narration-loop arm + salvage**: a zero-tool-call step whose text contains
      a call-shaped JSON object trips the detector; salvage through `resolve_tool()` + the
      alias layer (`list_files`→`list_dir`); fence self-authored "result" objects out of
      history as fabrications. (Evidence: lfm emitted 379 prose pseudo-calls, 300 to a tool
      that doesn't exist, then believed its own fabricated directory listing for 4 hours.)
      **(2026-08-14, PR #230)** — shipped as: structural arm on every call shape
      (`action`/`tool`/`tool_name`/`function`, OpenAI envelope, fence tokens) + a
      conservative ≥2-distinct-calls stream abort; salvage executes through the NORMAL
      pipeline (breakers/ledger/stats/memory/chips) with the synthesized `tool_use` blocks
      stored pair-complete so history demonstrates the dialect; `resolve_tool()` gained an
      unambiguous dialect-synonym step (`ls`/`dir`/`list_files`→`list_dir`, `cat`/`open`→
      `read_file`, `run`/`shell`/`execute`→`exec`; ambiguous names surface, never guessed);
      fences are insertion-only with a provenance corpus (real tool outputs + user text —
      never the model's own turns) so quotation is left alone; plus two adjacent honesty
      levers: consecutive byte-identical zero-call rounds announce themselves in the reply,
      and breaker replays record a `short_circuited` stats outcome (tracker, daemon sink,
      Turso hourly) instead of `success=0`.

**Tier 5 — the bench measures itself (`bench/gui-leg/`) ✅ (landed 2026-08-13, PR #225)**
- [x] Commit the GUI-path driver (leg.sh, ipc/start/resume .mjs, score.sh, ladder-42) to
      the repo; each leg runs from an immutable self-copy with hashes in the ledger header.
      *(2026-08-13 — `nanna-ipc.mjs` had already vanished from disk and was reconstructed
      from `protocol.rs`; the ladder's 42 tests are tracked as data with a combined hash
      in every ledger header; daemon stdout/stderr captured per leg into `run/daemon.out|err`.)*
- [x] Gate every ledger score on a **daemon liveness probe**; 3 consecutive failures →
      INVALID(daemon-unreachable), never a score. Record a work denominator per poll.
      *(Probe = `system.status` before every recorded poll; failed probe → poll marked
      UNREACHABLE with nothing else recorded; the summary additionally REFUSES a score when
      unreachable polls exceed a cap. Denominator = tool calls / side-effecting calls /
      tokens / queue via `get_run_state {light}` + anchored log greps for steps and `stop=`
      reasons — the fallback until Tier 4's session-liveness verb lands. Preflight
      (exclusive GPU, pinned num_ctx with hard abort on demotion, embedding-store health)
      asserted at start AND per poll.)*
- [x] Snapshot artifact + score per poll; report **peak, time-of-peak, and final**
      *(`run/history/<minutes>/` gets artifact copy + scored verdict + denominator each poll).*
- [x] Worker/supervisor split with a staleness-failing heartbeat; resume contract in the
      driver: interjection-only, liveness-gated, effectiveness-checked.
      *(Worker heartbeats + appends machine-readable `status.jsonl` every 60s tick; the
      separate supervisor fails the leg loudly on stale heartbeat, persistent
      unreachability, dead worker without a terminal verdict, or lifetime overrun. Each
      resume records whether the next poll changed; repeated no-effect resumes are
      suppressed. CI self-tests (`gui-leg-selftest.yml`, ubuntu+windows, no GPU): 42/42
      reference oracle, 0/42 stub with every ladder test individually failing it,
      supervisor units, dry-run legs vs a fake IPC daemon proving dead-daemon → INVALID
      with the score refused. AGENT_EVAL "Updating scores" now requires the validity
      verdict + work denominator beside every numerator.)*
- [x] **Correct the published GUI-path table**: ministral leg INVALID (daemon died at
      t=3m42s; scored a corpse), gemma leg CONTAMINATED (a `task` sub-agent on a cloud
      120B produced its peak), lfm reframed as tool-channel failure. Peak-vs-final becomes
      the headline metric. *(Landed as PR #222.)*

Full evidence: the six-agent retrospective (per-leg log forensics) in the 2026-08-11
session; per-leg detail in `bench/BASELINE.md` and the campaign ledger/artifacts.

---

### P23 — Continuation without destruction: cross-turn work preservation 🌱 (new — 2026-08-15, series-analysis-driven)

The post-P22 rerun (v0.3.7-beta.12, five 4-hour GUI-path legs, PR #233) proved P22 holds
**in-run**: qwen held 41/42 for 2.5 hours with zero destructive rewrites, gemma and
ministral were the first legs ever to finish at their peak, and lfm's tool channel became
real (127 salvaged executions vs 379 frozen-era hallucinated calls). The surviving
destruction channel is **cross-turn**: a continuation turn starts from a fresh context
seeded by a 63× compressed summary — the knowledge that N checks passed is gone, so the
model's cheapest coherent move is a from-scratch rewrite (ornith 30→0 in 8 minutes after
one continuation message; qwen 41→0 the same way). Secondary walls, all evidenced:
byte-floor evasion (five admitted writes each removed 9–33 functions while clearing the
30% floor, because gutted-but-parsing files re-anchor `good` downward), a spec-test
doctored by the model after misleading error advice (qwen `chmod +w tests/test_40.sh` —
its artifact actually satisfies all 42 hermetically), silent run deaths (qwen died at its
peak with no stop line, no user-visible message; gemma dry-counted out at 0/42 while its
own checks were failing), a Mistral-family transport wall (Ollama aborts the stream on a
literal TAB in tool-call JSON; three blind retries per doomed generation), and tool acks
that lie by omission (python's registry save reads as a file write — gemma "saved" its
artifact 201 times with no file on disk). All levers below are chat-general (owner rule);
every bound derives from a real constraint. 27 candidates survived adversarial review of
the 40-agent series analysis (2026-08-15 session).

**Tier 1 — verified work must survive the turn boundary** *(the headline fix)*
- [x] **Pinned artifact-state preamble on continuation turns**: on any turn into a scope
      with prior verified work (done tasks OR non-empty hiwater ledger), build an ARTIFACT
      STATE block from ground truth re-read at turn start (per hiwater entry: canonical
      path, fresh stat, hi/good/chk; plus the scope's verified check verdicts persisted as
      session-scoped VerifiedOutcome rows) under the contract "these files hold verified
      work: read before writing; extend, do not reconstruct". Inject into BOTH the planner
      context and each step's never-compressed `verified_outcomes` slot, seeded from the
      store. At verdict capture, enrich stored completion detail with the check's output
      head + subject file stat so even a `file exists` verdict carries artifact identity;
      render the digest in `build_step_prompt` beside LAST RESULT. Idempotent per-turn
      snapshot; bounds = hiwater entries (each cost a real write) × distinct verdicts
      (each cost a real execution). No new caps.
- [x] **Reproduce-first on claim conflicts**: when an incoming message asserts failure of
      a check identity whose latest verdict is a verified pass, render a CLAIM-CONFLICT
      block naming both sides, suspend the already-passed close for the contradicted item,
      and seed ONE reproduction task at the head of the plan (existing acceptance
      machinery, `run_with_timeout_cap`, fresh scratch dir, user-described steps when
      given) with provenance annotation (content hash, mtime vs verified-at, own-write
      attribution, `.__prev__` offered for diff). Reproduces → failing verdict enters the
      ledger; doesn't → report both sides and ask ONE clarifying question instead of
      mutating. While unresolved, the #224 stale-shrink content-echo applies
      unconditionally to shrinking rewrites of the disputed artifact. Evidence-only phase
      exit; no timers.
- [x] **Re-anchor note on transient-retry restarts**: the transient-error retry branch
      appends a bounded note (NO_PROGRESS_NUDGE pattern, same site tasks.rs:2318): the
      step was interrupted mid-flight, prior tool effects are on disk, continue — do not
      restart; re-read before whole-file writes. Name the last side-effecting call from
      the liveness step record when present. Same for the chat harness error-round retry.
- [x] **One fresh-context reseed before a dry-round death with failing checks —
      internalize the interjection** *(what the human interjection supplied, gemma 0→16)*:
      when the dry exit would end a mission turn while `abandoned_unmet` is non-empty,
      re-enter turn-start semantics ONCE inside the same run handle (fresh
      AgentStepRunner + reset run-scoped breaker ledgers, rebuilt established context,
      re-discovery, `seed_plan` not `seed_continuation`), keeping verified_outcomes, the
      flip ledger, and cumulative accounting. Re-arm only if the verified-state
      fingerprint has changed since the last reseed; CONTINUATION_ROUNDS_MAX still bounds
      everything.

**Tier 2 — the write path must see structure, not bytes**
- [x] **Symbol-aware shrink hold (one-bounce, echo-style)**: a shrinking whole-file write
      that removes more top-level definitions than it keeps — or drops definitions
      present in the last structurally-good version — holds ONCE with the stale-shrink
      echo shape (current content as merge material, removed symbols named, echo counts
      as the read, removal-set signature recorded); a follow-up whose removals match the
      acknowledged signature proceeds. Reuses the rewrite-note's symbol pass (zero extra
      parse); fails open with no symbols; growth is never refused; `edit_file` exempt
      (explicit old_string intent). Catches 4 of ornith's 5 destruction writes.
- [x] **Park `<file>.__best__` — the structural-coverage high-water** — beside the
      one-slot `__prev__`: park the outgoing version when its top-level symbol count
      strictly exceeds the recorded best (rewrite spirals rename symbols, so subset
      relations rotate the peak out — count, not set). Rewrite-notes name it: "26
      sections removed; the fullest prior version (42) is parked at `<file>.__best__`."
- [x] **Verdicts notice self-modified evidence**: hash acceptance-referenced files at
      canonicalization; re-hash at every verdict (pre-step, post-step, #218 sweep). On
      mismatch: structural sentence naming the transition (+ "modified by this session"
      only when the write ledger attributes it), demote a passing verdict to UNKNOWN
      (existing timed-out semantics — never counted, never a completion, never a flip
      credit), re-baseline so exactly the next verdict decides. A legitimate test edit
      costs one named re-verification; tampering can't complete an item in the same
      breath as the mutation. *(qwen's doctored test_40 manufactured both its 41/42
      ceiling and the fatal claim-conflict.)*
- [x] **User-declared file invariants at the tool layer**: extract durable file
      prohibitions (read-only glob / do-not-delete / do-not-create-under, imperative +
      explicit path referent only) at the P14/P15 canonicalization pass into a
      per-scope registry (verbatim source sentence retained, inherited by subtasks);
      write tools consult it and refuse with a steer quoting the user's own sentence
      (`{content, success:false}`, never thrown); exec extends its ratchet-redirect
      refusal to registry paths. Lifted only by the user; `ask_user` is the escape hatch.
- [x] **Non-retryable write errors name the cause and disavow the bypass**: bridge write
      failures carry `io::ErrorKind`; on confirmed PermissionDenied the tool result says
      write-protected + non-retryable + "protection is usually deliberate — change the
      file you are producing instead" (retry advice survives only for transient kinds).
      exec appends an honesty echo (flag, never gate) when a protection-stripping command
      targets a file this session never wrote. *(12+ misleading "retry the same call"
      messages preceded qwen's chmod.)*
- [x] **Structural-verdict sentence for literal `\n` runs in sh comments** (write_file +
      edit_file, sh-checked files only): ≥2 literal backslash-n on a comment-effective
      line → one verdict sentence warning flattened code may hide behind it. Sentence
      only, never a gate; fails open.

**Tier 3 — endings must be loud, honest, and evidence-priced**
- [x] **End loudly; never end dry while your own checks still fail**: reify MissionEnd
      (initial-stop / dry / planner-fallback-exhausted / error-rounds-exhausted /
      rounds-max / cancelled / planner_starvation); exactly one cumulative
      `chat harness mission finished` line at continuation-loop exit (every turn, every
      path — qwen died at 41/42 with no line at all) with the reason threaded into the
      liveness StopMark for `session.liveness`; every non-cancel exit streams one
      sentence of reason + evidence. The seeded-nothing dry path gets the same
      `abandoned_unmet` guard its sibling has: failing checks → NOT dry → reopen the top
      unmet item by id (#218 machinery) and charge the round. Dry keeps authority only
      when no canonicalized check most-recently-FAILED.
- [x] **Error rounds spent against provider-health evidence**: fix the stale-stop
      double-charge (observed 2→4 in one 30s round), and before consuming a round run one
      minimal single-token probe on the run's model (existing timeout + backoff ladder) —
      probe answers → charge (fault persists across a healthy provider); probe exhausts
      the ladder → transport-outage evidence: invoke the transient heal and charge, so
      three rounds require three full heal ladders instead of 60 seconds. Probe re-warms
      the model after keep_alive=0, closing the self-inflicted cold-load cascade.
- [x] **Park-and-resume on transient infrastructure**: at both terminal give-up sites,
      when the stop classifies transient (`is_transient_llm_error`), end the turn PARKED
      (demote-to-pending + transcript notice naming provider and resume condition);
      resume on the daemon's next successful completion on that provider (recovery
      evidence, never a timer), carrying the SAME error_rounds counter through the
      ChatRunRegistry claim.
- [x] **Derive `is_mission` from session state, not run luck**: for error stops, grant
      continuation rounds when run evidence OR open items in the session's scope show
      unfinished work (one query through the existing open-work path); a crashed run that
      seeded work it never touched still holds a mission.
- [x] **Classify 0-byte dead streams as a wedge**: one arm in `wedged_runner_error` on
      the exact "No NDJSON line was ever parsed" literal (cannot match mid-generation
      aborts, which stay out — contention-cancel ambiguity); rides the existing
      Repeated-path reset. *(116/160 in-window ministral retries were this shape,
      healed blind.)*
- [x] **Heal the control-character transport wall**: classifier for Ollama's
      character-naming abort bodies ("invalid character '\t' in string literal" family)
      → class-specific corrective retry note naming the offending character and the two
      legal routes (JSON escapes in tool-call strings; exec printf/heredoc for literal
      bytes). Rides the existing retry ladder; pairs with the parse-side lenient
      pre-parse (chip task_01e5b3d9, in flight) — disjoint classes. *(Ministral's
      constant wall; ornith's learned tab-avoidance.)*

**Tier 4 — tool results must tell the truth about what happened**
- [x] **Side-effect acks name WHERE the effect landed**: python's registry save must say
      "saved to the session script registry — did NOT create or modify any workspace
      file" (gemma "saved" 201 times with nothing on disk); then a one-pass audit of
      default-skills/*: any success message describing a side effect names its
      location/scope ("summaries must announce themselves" contract).
- [x] **Claim nudge grounded in evidence, both halves**: (a) arming — fire only when the
      step holds a successful write/edit OR an exec fail→pass flip on the same command
      digest (read-only churn never arms; gemma's 40-firing window replays to zero);
      (b) content — when the newest work-evidence record FAILED, the nudge names that
      failure (bounded `preview_snippet`) and demands a re-verified fix before any
      TASK COMPLETE. *(Ornith's false "all 42 pass" claim was nudged into existence the
      same log-second as a 12-FAIL wall.)*
- [x] **Truthful deadline-exceeded exec (kill-and-tell)**: drain pipes incrementally;
      on timeout, kill the tree and RETURN `timed_out: true` + elapsed + partial output
      ("the command executed and may have left side effects; disk is truth") — "Nothing
      ran" is reserved for genuine spawn failures.
- [x] **Directory tools teach on file paths** (and converse in read_file): "'X' exists
      but is a FILE — use read_file" instead of os-error jargon, one extra stat only on
      already-failed calls; also closes read_file's latent uncaught-throw on
      directories.

**Tier 5 — GUI truthfulness** *(from the leg-1 composer-driven observations)*
- [x] Breaker replays (`short_circuited === true`) render as inline steering, not the
      red "Tool Failed" toast.
- [x] Config-mutating verbs emit `Event::ConfigChanged` (WorkspacesChanged pattern);
      Models tab, provider badge, and model chip re-fetch on it instead of caching at
      mount *(the stale-Offline badge, stale priority list, and stale chip were all this)*.
- [x] Composer content integrity: `autolink: false` in the editable editor + a
      round-trip test — the leg-1 mission arrived as `test_[01.sh](http://01.sh)`.
- [x] Workspace create requires what the backend requires (one WORKSPACE_MARKER), not
      one forced standard file; standard files become an unchecked offer.

**Series ops debts** *(bugs and audits surfaced by the analysis, not levers)*
- [x] **Summarizer-pin resolution audit**: 171/171 Tier-2 summarizations ran on
      `ollama/lfm2.5` despite per-leg `llm.summarization_priority` pins — every
      compressed context in the series was degraded by the weakest model and VRAM was
      double-loaded. Audit the summarize-priority resolution in the bucket router.
- [x] **Zero-information breaker: normalized matching** — byte-identity was evaded by
      ~20 trivially-varied spellings of the same sweep in five minutes (qwen
      05:09–05:13Z); normalize command text or match on result hashes.
- [x] **Bench-side (exempt from the owner rule, harness tooling): hermetic per-test
      scoring** in `bench/gui-leg/score.sh` + a per-test pass/fail map — order-coupled
      residue hid qwen's real test_40 divergence until after the series (its peak
      artifact scores 42/42 hermetically).
- [ ] **Cross-leg importance-merge audit**: 19 global merges of near-identical
      "building minidb" memories across legs — no observed harm; audit the
      dreaming-adjacent vector.

**Landed 2026-08-15 (v0.3.8-beta.13).** All 27 verified levers are in; 1046 tests
green. Known remainders, deliberately scoped rather than silently dropped:

- [x] **Summarization priority is live everywhere** *(2026-08-15)* — the daemon now
      builds ONE `Arc<RwLock<AgentServiceConfig>>` before the spawner and the script
      services and hands the same lock to the agent service (`with_shared_config`), so
      `config.set` reaches all three; the scheduled dream cycle re-reads the list at the
      top of each cycle. Previously boot-frozen in three
      long-lived consumers** (`server.rs`: the scheduled dream-cycle summarizer list,
      the sub-agent `AgentConfig`, and script-services summarizer models each clone the
      list once at boot). The per-turn chat/harness path — the one that degraded the
      series — now re-reads it; these three need the same treatment.
- [ ] **Steering marker on re-seeded timelines**: breaker replays render as steering
      live, but a timeline rebuilt from the run journal after a remount does not carry
      `short_circuited`, so history still shows them as failures.
- [ ] **No lift path for a declared file invariant**: once registered, a prohibition
      stands until the registry file is removed. "You can edit tests/ now" is exactly
      the permissive phrasing a conservative extractor must not act on, so lifting
      needs its own deliberate, `ask_user`-confirmed shape.
- [ ] **Evidence hashing is anchored at run start, not at task-write time**: the
      repository layer has no workspace root to resolve a relative acceptance path,
      so the hash baseline is taken where the workdir is known instead.

Full evidence: the 40-agent forensic analysis (per-leg + cross-cutting, adversarially
verified) in the 2026-08-15 session; per-leg ledgers and per-poll history under
`D:/Development/nanna-bench/ui-run-*/`.

**P23 verification series (2026-08-15/16, v0.3.8-beta.13).** Five legs on the same
ladder; results and per-leg trajectories in [bench/BASELINE.md](bench/BASELINE.md).
P23's core claim held for the top two models: ornith 40 peak / **36 final** and qwen
**26 = peak = final**, both with **zero interjections**, against post-P22 finals of 0 and
0. Three legs ran destruction-free end-to-end untouched. Levers observed firing:
MissionEnd honesty, repeat-done escalation, structural shrink holds, byte-floor refusals,
truthful tool acks (zero phantom registry saves, against 201 previously), exit-reason
file. The series produced one crash bug and five carry-forward items below.

### P24 — Sessions that keep their work, and tell the truth about it ✅ (2026-08-17; all 21 items landed, audited 2026-08-25)

Successor to P23. Produced by a systematic review of five long autonomous sessions on
v0.3.8-beta.13: 93 candidate findings, each put through two independent adversarial
refutation passes (one checking every log quote and code anchor against the tree, one
checking the owner rules and whether the proposal is already implemented). 41 were killed
and are listed at the end so they are not re-derived. What follows is the 52 that survived,
merged into 21 items and ranked by expected effect on an ordinary user session.

**Status (2026-08-21, added by the nightly routine): most of P24 has LANDED — the write-ups
below are the original defect reports, not a list of open work.** PR #255
(`p24/session-scoped-and-review-fixes`, merged as `9fd4ba0d`) carries
`62cc4465` (P24.9/19/21), `4a3f3103` (P24.11), `7114a27a` (P24.5), `8f7a8662` (P24.8/10/4),
`91b58405` ("the remaining P24 items") and `de58d49d` (review findings). Verified present in the
tree at `f3fe0352` by anchor: `floor_char_boundary` in `context.rs`/`compressor.rs`/`loop_runner.rs`
(P24.1); `bind_session_workdir` + the `RUN_SESSION_ID` control-plane assertion in `registry.rs`
with `chat.rs` binding per turn (P24.2); `.__best__` parking in all three write tools and
`failEscalating` in `edit_file`/`write_file` (P24.4/P24.8); `record_structural_verdict` +
`"ok — DOES NOT PARSE"` in `loop_runner.rs` (P24.5); `self.summaries` gone, so the
double-charged preamble vector no longer exists and `estimate_request_tokens` is
`preamble_tokens() + estimate_tokens()` (P24.6, first bullet); whitespace-normalized repeat keys
(P24.13); `WRITE REFUSED — the file was NOT modified` (P24.19).

**Read this before picking P24 work:** treat each item as landed unless you have checked its
"Where" anchors yourself. The one gap re-confirmed as genuinely open this run:

- [x] **P24.3 part 3 — the embedding is off the turn's critical path.** *(2026-08-25)* Parts 1, 2
      and 4 had landed (`collapse_repeated_lines`, the mid-ingest cancellation check, `log_excerpt`)
      and the "two memory sinks disagree" rider was resolved 2026-08-21; this was the remainder.
      **What shipped:** a tool-result chunk is now persisted synchronously and only its *vector* is
      queued (`MemoryService::remember_deferred_vector`), so the loop's next step no longer waits on
      an embedding round-trip against the same local server that serves generation. The row is
      durable before the call returns and keeps its `source_id`, so the `recall(...)` handle the
      model is handed in the same turn still resolves — only similarity search waits.
      **The hard half was the drain, exactly as the item said.** `drain_backfill` cannot pick these
      up: when the embedder is the local provider it first waits for *no harness run to be live*, so
      a row queued **by** a live run would wait for the run that queued it — hours, during a mission.
      So `drain_queued_vectors` skips that yield gate, and pays for the exception with a bound the
      gate was standing in for: **it may only embed rows this process parked**, budgeted by
      `MemoryService::take_queued_vector_count()`. An inherited backlog (2167 rows, in the incident
      that motivated the queue) is still `drain_backfill`'s job at `drain_backfill`'s priority. It
      keeps the RTT-repayment window and still takes `drain_serial`, so it can never exceed half the
      provider's wall-clock and cannot multiply the request rate. Net: the same embedding work, at
      half the duty cycle, concurrent with the turn instead of blocking it.
      **One latent bug fixed on the way:** `drain_backfill` parked on `wait_idle()` *while holding*
      `drain_serial`, so a drain waiting for a mission to end held the one process-wide drain lock
      for the length of that mission and starved every drain behind it. The yield now happens outside
      the lock; the passes and the repayment sleep still happen under it, so both stated invariants
      are unchanged.
      **And the third duplicated policy is gone.** The filter and the importance table were unified
      into `memory_adapter` in 2026-08-21 after they drifted and cost 704 writes; the *route* was
      about to become the same story, so the whole sink now lives in
      `memory_adapter::store_extracted_memory` and both `agent_service.rs` (chat) and `tasks.rs`
      (harness) differ only in how they log. `TOOL_RESULT_CATEGORY` is a constant in `nanna-agent`
      so the end that stamps it and the end that routes on it cannot drift either.
      **12 tests**, none of which passed before the change: the deferred write never consults the
      embedder (asserted by a counting embedder, so it distinguishes this from the outage path
      `store_unembedded` already had), an ordinary fact still embeds inline, the queue publishes a
      drainable count that resets when taken, only a tool result defers, the noise filter runs on
      both routes, and a FAILED tool result still reaches the store. Disabling the route makes
      `only_a_tool_result_defers_its_vector` fail on "no inline embed for a tool result".
      **Still open from the item:** `semantic_chunk(&ingest_content, MEMORY_CHUNK_MAX_CHARS, 0.15)`
      is bounded only by bytes. That is now a storage-footprint question rather than a latency one —
      the chunks no longer cost the turn anything — and the cap must still not be derived from the
      retrieval top-k (see the item's own note).
      - [ ] Bound the chunk *count* per tool result on a principle that is not the retrieval top-k.
- [x] **Audited item by item, 2026-08-25 — every P24 item has landed.** The 2026-08-21 pass
      verified 8 anchors and deliberately declined to claim the rest; this pass checked the
      remaining 13 against the tree. Each verdict below names the anchor that proves it, so the
      next reader can re-check one item without re-deriving the whole section. **The defect
      write-ups below are kept as the reasoning record, not as open work** — the section header
      says so, and they are the only place the evidence lives.
      - **P24.7** — `attempt_side_effects: Vec<ToolMark>` beside the turn-scoped `last_side_effect`
        (`liveness.rs:167-185`), rendered through the bounded `step_activity_digest`
        (`loop_runner.rs:1112`, used at four step-exit sites).
      - **P24.9** — `NannaBridge::msys_drive_path` (`bridge.rs:560-569`) with the literal-first
        guard, called from `resolve_path_with_workdir` before the relative branch (`:537`), and a
        test that a shell-printed `/d/...` reaches the real file (`:1428`). The `runStructuralCheck`
        exit-127 split is present at `write_file/tool.ts:613`.
      - **P24.10** — the threshold is derived from the live input budget, not `max_tokens`:
        `loop_runner.rs:6882` reads `self.context.read().await.hard_limit` and scales by
        `CHARS_PER_TOKEN_ESTIMATE`. *(Residual: the doc comment at `:1152` still describes the old
        `(max_tokens * 2).clamp(2000, 32000)` formula — see the `[ ]` below.)*
      - **P24.11** — solved by a different shape than the item proposed, and correctly: rather than
        adding `ToolCallRecord::error`, `record_output` (`loop_runner.rs:868`) falls back to
        `result.error` when `content` is empty, so both the repeat detector and the novelty check
        stop comparing empty strings. `structure_broken` sits beside it for the third outcome.
      - **P24.12** — `backstop_timeout` (`registry.rs:1058`) is params-aware via
        `ScriptEngine::supervising_timeout_ms`, which applies the existing
        `ENGINE_TIMEOUT_HANDOFF_MARGIN_MS` (`engine.rs:344`), so the inner message wins by
        construction.
      - **P24.14** — `(dry_replans, escalated_asks, last_result)` at `harness.rs:2138` with an
        `escalate` branch at `:2394` that takes a different prompt path, and the replan-allowance
        accounting at `:2566`.
      - **P24.15** — `is_line_structured` (`compressor.rs:383`) routes line-structured content away
        from sentence scoring at `:245` and short-circuits the wasted round-trip at `:369`.
      - **P24.16** — `abandoned_unverifiable: Vec<AbandonedUnverifiable>` (`harness.rs:1433`)
        populated at **both** abandonment sites (`:2192`, `:2537`); `items_completed_unverified`
        merged (`:3252`); and the cancel path renders evidence bannerlessly through
        `unresolved_evidence` (`chat_harness.rs:2367`, called at `:2473`).
      - **P24.17** — (a) `DaemonEvent::LivenessBeat` exists in the GUI client
        (`daemon_client.rs:143`) with a deserialization test (`:1463`); (b) the harness sets
        `on_usage` (`tasks.rs:3036`) with the comment naming the gap it closed (`:3031`).
      - **P24.18** — (a) `store_unembedded` is the embed-failure path on every write route
        (16 call sites in `service.rs`); (b) `search_reports_what_it_could_actually_compare`
        (`lib.rs:2435`) pins the three distinguishable empty answers; (c) the 30,000-byte behead is
        gone, with the reasoning kept at `service.rs:1092`.
      - **P24.20** — (b) `scripted.rs:83` overwrites the file-stem name with the manifest name at
        load time, for exactly the stated reason (every skill's entry point is `tool.ts`, so the
        engine logged nearly every tool as `tool`); (c) zero reduction is reported as zero
        (`context.rs:2221`, `:3587`).
      - **P24.21** — `web_search/tool.ts:23` names an action available in this session and says
        nothing was searched; `exec/tool.ts` names itself, says nothing ran, and lists all five
        accepted aliases.
      - **Method and its limit, stated honestly:** this is an **anchor** audit — for each item the
        named mechanism was located in the tree and read. It is not a line-by-line re-derivation of
        every sub-bullet, and it did not re-run each item's original evidence. An item whose
        mechanism is present but subtly wrong would pass this audit.
- [x] **`BREAKER_REPLAY_MAX_BYTES` was derived from a formula that no longer exists.**
      *(2026-08-25 — found by the P24 audit above, fixed the same run.)* Its derivation read
      "2000 bytes is the floor of the dynamic `context_result_threshold`
      (`(max_tokens * 2).clamp(2000, 32000)`)" — the boot-frozen `max_tokens` formula P24.10 was
      raised about, which `loop_runner.rs:6882` replaced with `(hard_limit / 4) *
      CHARS_PER_TOKEN_ESTIMATE`. There is no `clamp` and no floor of 2000 any more, so the stated
      justification was for code that had been deleted. The **value is unchanged** — the constraint
      it encodes (small enough to reach context untouched) still holds, now argued from the live
      input budget and the `min_viable_num_ctx` floor below which the loop refuses to run at all.
      Only the sentence was wrong, and a bound whose derivation has gone stale is the next magic
      constant: nobody can tell whether it is still right.

- [x] **A drain trigger the daemon owns — the backlog `drain_queued_vectors` deliberately does not sweep.**
      *(2026-08-26)* Complements the item above rather than duplicating it. `drain_queued_vectors`
      drains what **this process** deferred, at foreground priority, and is budgeted so it will not
      sweep an inherited backlog — its own doc says that remainder is "still `drain_backfill`'s job
      at `drain_backfill`'s priority". The gap: **nothing was calling `drain_backfill` at that
      priority during a session.** Its only triggers are BINDING events (daemon start, provider
      switch, width reprobe) and an ordinary session has none, so a row parked by a *transient*
      embedding failure — or a backlog inherited from a previous run — waited for a restart.
      `MemoryService::store_unembedded`'s own doc named this exactly: "it is recovered, not lost —
      but the latency is a session, not a moment, and closing that needs a drain trigger the memory
      crate does not own."
      `supervise_idle_backfill` is that trigger: one task for the daemon's life running
      `wait_active().await; wait_idle().await` — exactly one turn's lifetime — then the existing
      `drain_backfill`. **It adds a trigger, never a second rate:** same process-wide `drain_serial`
      mutex, same chat-priority gate, same per-request RTT repayment.
      **Bound.** One probe per active→idle edge, so the probe rate is bounded by the *turn* rate.
      The probe is what a complete store already costs `drain_backfill`:
      `entries_missing_model(model, 1)` is an **in-memory** scan of the entries cache that
      short-circuits on the first unbucketed entry (walking it whole only when there is nothing to
      do), plus two `LIMIT 1` local Turso queries. No provider request unless work is found.
      **Known, deliberate limit:** `wait_active` registers interest and then reads the flag, so a
      turn that begins AND ends inside that window leaves no edge and its rows wait for the next
      turn. Bounded by one turn, with the rows durable and handle-addressable throughout. Not
      "fixed" by probing before parking — that turns the loop into a spin on an idle daemon.
      **4 tests** in `chat_harness` pin the registry contract the bound rests on, and the fourth
      exists because the second **failed first**: it had asserted the stronger guarantee and failed
      by timeout, exercising the very race the design note described in prose. Rather than weaken
      the check, the contract that does hold is tested with a `Notify` handshake, and the limitation
      is pinned as its own named test so anyone who later "fixes" the loop is told which property
      they traded away.


#### What is already working — do not re-litigate

The review turned up more working machinery than broken machinery. Recording it so nobody "fixes" it:

- **The write-side structural shrink hold works.** It fired 13 times across the review window, always on the shape it was designed for — a large file losing most of its definitions — e.g. `write_file guard: structural shrink hold for <path> (15473->10425 bytes) removed=[append,backup,clear,...] kept=18`. In the strongest session it blocked four destructive whole-file rewrites and the file never lost its definition set.
- **The stale-shrink hold covers the post-retry blind rewrite.** After a provider abort re-anchored a step, three destructive whole-file rewrites were attempted in the fresh context and all three were held. The read-mark ledger lives on disk and survives the context discard, which is why it worked.
- **The structural sentence is enough for capable models.** A break introduced by an edit was self-repaired in 6 s and 15 s on two models with no prompting, and 9 of 10 times on a weak one. Do not convert the sentence into a hard gate on that evidence alone (see P24.5).
- **The repeat-failure and zero-information breakers fire and bound loops** (21 / 32 / 16 firings in three sessions). The world-epoch gate correctly re-arms a read after an edit — the edit→re-read→edit path is not being blocked.
- **The transport retry ladder is not the bottleneck.** One session absorbed 44 aborted generations and 26 runner unloads and still finished with its work intact; the session that lost work took one abort and one reset. Total retry-transport cost was ~4% of that session. No transport lever should be justified by the difference between them.
- **Context demotion never fired and the stream watchdog never fired.** Both are correct and idle.
- **The user-declared invariant refusal held** in all three write tools, 17 times, e.g. `edit_file guard: EDIT REFUSED (declared invariant read_only on '<dir>') <dir>/<file>`.
- **Every truncation that announces itself, announced itself correctly** — the memory stub, the compression loss notice, the write-held echo. The failures below are all cases where nothing announces, not cases where the announcement is wrong.
- **The daemon's liveness surface already knows when a turn is wedged.** It emitted `quiet_s=5700 phase="step_pending" awaiting=LLM request in flight ...: 5700s, no output yet this step` on a 30-second beat for 96 minutes. The gap is entirely in what consumes it (P24.17).

---

#### Tier 0 — Crash

#### P24.1 — Char-boundary slice sweep **[COVERED — land the open PR]**
**Broken.** Four raw byte slices in the compression paths (`&text[..100]`, `&content[..80]`, `&content[..200]`, `&thinking[..200]`) abort the process when a multi-byte character straddles the cut. With `panic = "abort"` this kills the daemon, not the turn — every concurrent session's work goes with it.
**Evidence.** `PANIC: end byte index 80 is not a char boundary; it is inside '—' (bytes 79..82 of string) location=crates\nanna-agent\src\context.rs:1838:54`, followed by a 15-minute silence and `Removing stale PID file`.
**Change.** Route all four through `floor_char_boundary`, which the same file already uses at :1165, :1181, :1414, :1617. **PR #242 is open and already does exactly this** (plus two further sites: memory paging offsets in `server.rs`, and a `&rest[1..rest.len()-1]` that inverts on a lone quote). Merge it.
**Where.** `crates/nanna-agent/src/context.rs:1827, 1838, 2011, 2020`.
**Correction to the earlier write-up:** the panic hook cannot name the offending string — std's message drops it and the hook only receives the formatted payload. Drop that half; once the slices are safe there is no panic to name.

---

#### Tier 1 — Destroys or misplaces the user's work

#### P24.2 — Two chats on two projects share one mutable working directory **[NEW]**
**Broken.** The tool registry keeps a single process-wide working directory. A second chat turn's setup overwrites it — and because the per-session override is keyed on whichever session currently owns the shared id, the incoming turn files its root under the *outgoing* session's key and never clears it. A turn already running then resolves its relative paths into the other project, writes there, and reports success.
**Evidence.** `chat.rs:391` calls `set_default_workdir` **before** `chat.rs:396` calls `set_session_id`, while `registry.rs:90-98` keys the per-session insert on the current shared id; observed live as a running turn's edits landing in an unrelated checkout, which still shows ` M src/setup.rs`, ` M gui/app/components/ChatInput.vue` and `?? tests/` in `git status` while the turn's own file stopped changing at the moment of the override.
**Change.** Pin the root to the turn, don't narrate the drift. (a) Scope a chat turn with `ToolRegistry::with_run_session`, the mechanism already used for scheduled runs (`server.rs:1339`) and never wired to interactive chat. (b) Swap the ordering so the session id is set first, and key the per-session insert on the session being prepared rather than the shared slot. (c) Clear the entry on the `None` path (`registry.rs:131` already has `clear_session_workdir` and nothing calls it on teardown), or a session that loses its workspace keeps a stale entry that now *wins* over the default. (d) Add the missing test: two turns in flight, different roots, each reads its own. Only after that, as a backstop for roots that move for reasons the daemon does not own, emit one announce-once line naming both absolute paths.
**Where.** `crates/nanna-tools/src/registry.rs:90-131, 141`; `crates/nanna-daemon/src/control/chat.rs:237, 391, 396`; sub-agent path `control/session.rs:376-383` uses the same weak `set_session_id` patch and should migrate with it.

---

#### P24.3 — Memory ingestion is unbounded, synchronous, and uncancellable **[NEW — merges five observations]**
Merges: the inline `on_memory` await, the unbounded chunk count, the degenerate-repetition blow-up, the uncapped exec capture, and the uncapped log write. They are one event seen from five layers.

**Broken.** Every tool result is chunked with no bound on chunk *count*, and each chunk costs an embedding round-trip plus a vector search plus an insert, awaited inline on the turn's critical path against the same local model server that serves generation. A command that fails or is killed carries its entire captured output inside its error string, so the failure path is the dominant one.
**Evidence.** `loop_runner.rs:6212` chunks with no cap and `:6244` awaits `on_memory` per chunk; three tool results became 100,016 memory rows, the loop made zero model decisions for 189 of 246 minutes, and one of them was still writing 34 minutes after the user's stop had cancelled its session.
**Change.** Four parts, in order:
1. **Run-length-collapse identical consecutive lines before chunking**, storing the line plus its repeat count. This is lossless and reversible, so the "stored whole in memory, nothing was lost" promise and the `source_id` reassembly path (`server.rs:338-378`) both stay true. It is the only part with a genuinely derived bound — cost becomes proportional to information rather than bytes — and it alone would have prevented all 100,016 rows.
2. **Add a cancellation check to the chunk loop** at `loop_runner.rs:6234`. Today a user pressing stop does not stop it.
3. **Take the embedding off the critical path** — persist the row synchronously (the model is handed a `recall(...)` handle in the same turn and a deferred row makes that handle dead), queue only the vector. Do **not** reuse the background drain unmodified: it sits behind `chat_runs.wait_idle()` (`server.rs:1256`), so foreground-originated rows would never land during a live turn.
4. **Bound the log write** at `crates/nanna-scripting/src/boa_impl.rs:365`, which prints full stdout and stderr with no cap and produced ~300 MB of one repeated line in a single day's log.
**Also resolve while here:** the two memory sinks disagree. `agent_service.rs:1093` drops any tool result whose content merely `contains("Error")`; `tasks.rs:1991` filters only empty/control-char/heartbeat noise and has no such test. Either the first is silently discarding legitimate failed-tool evidence in ordinary chat, or the second is missing a filter. They cannot both be right.
**Do not** derive the chunk cap from the retrieval top-k — chunks past top-k are reachable by handle dereference and by direct similarity hit, and dropping the middle would make the stub's promise a lie.
      *(2026-08-21)* **"The two memory sinks disagree" is resolved — chat now uses the harness's
      filter.** The tree answered its own question: `tasks.rs::is_low_signal_memory` already carried a
      long doc comment explaining why the substring failure tests were removed (they discarded **704 of
      704 failed tool calls** in one 2-hour run, and also ate successful calls whose output merely
      quoted an error — `cat ./minidb` stored nothing, twice). `agent_service.rs` — the path an ordinary
      user chats through — still ran the older filter that comment describes as the bug, plus a
      `content.len() < 20` floor, a dead `[Tool:` prefix test, and a "dominated by non-ASCII" test that
      classified `tree` output and every non-Latin script as binary. So the documented loss was still
      live in interactive chat. Both sinks now call one `memory_adapter::is_low_signal_memory` and one
      `memory_adapter::episodic_importance` (the importance table was the *second* privately-duplicated
      policy — how the two drifted in the first place). 6 unit tests, previously zero, pin the shapes
      that must stay writable (failed tool result, error-quoting success, box-drawing, non-Latin, a
      19-byte "ok") and the shapes that must not (empty, whitespace, heartbeat, C0 control bytes, U+FFFD).
      Parts 1, 2 and 4 of this item had already landed (`collapse_repeated_lines`, the mid-ingest
      cancellation check, and `log_excerpt`/`EXEC_LOG_EXCERPT_BYTES`); **part 3 — the chunk-count bound
      and taking the embedding off the critical path — is what remains open here.**
**Where.** `crates/nanna-agent/src/loop_runner.rs:6169-6255`; `crates/nanna-memory/src/service.rs:1019-1165`; `crates/nanna-tools/default-skills/exec/tool.ts:373-395`; `crates/nanna-scripting/src/boa_impl.rs:365`.

---

#### P24.4 — An in-place edit can delete named definitions, and the edit path parks no recovery copy **[SHARPENS "no-shrink structural break detection", "park by verified score not recency", "name the parked copy in the verdict"]**
Merges: the anchor that is written but never compared, and the recovery copy that only the whole-file path writes.

**Broken.** `edit_file` computes the file's top-level definition set, writes it into the shared anchor for the *other* tool's guard to measure against, never compares it itself — and on any parsing edit unconditionally **rebases** the anchor to the post-edit set, erasing the evidence the write-side hold depends on. Separately, `.__prev__` and `.__best__` are written only inside `write_file`, so in an edit-driven session the recovery copy is as old as the last whole-file rewrite that was *permitted*.
**Evidence.** `edit_file/tool.ts:248` sets `next.goodSyms` on a passing verdict with no shrink guard while `goodSyms` is never compared anywhere in that file; the only recovery copy in one session was 2 h 56 min stale and 5,081 bytes short at the moment it was needed, because every later whole-file write had been correctly held (`WRITE REFUSED (shrink floor) ... floorBase=15473 anchor=good`) and a held write returns before the park at `write_file/tool.ts:1123`.
**Change.**
1. **Compare before writing.** Move the definition-set comparison to before `Nanna.writeFile` at `edit_file/tool.ts:925` (`updated` is complete by :822 and the ledger loaders are in scope; the pre-write Python gate at :836 is the existing precedent). Ship it first as the **removal note** `write_file` already emits at :1421 — informational, no extra round-trip, works on every file class the regex can see. Only consider the hold afterwards, and only with the bounce cost accepted.
2. **Guard the anchor rebase.** A parsing edit that drops a name currently overwrites the anchor with the smaller set. Stop that; the write-side hold is built on it.
3. **Park by last-verified-good, not recency** (already on the list): gate the existing `write_file` park on the ledger's `chk`, so a broken outgoing version never displaces a good parked one. Needs no new call site.
4. **New:** give the *edit* path a durable copy, but as a **coverage ratchet, not a recency slot.** A one-slot per-edit recency park is useless — in the observed case six more parsing edits followed the destructive one within 2.5 minutes and would have rotated it out in ~13 seconds. `edit_file` already has the section regex at :265 and already calls `symbolNames(updated)` at :965; park to `.__best__` when the outgoing version's section count beats the record.
5. **Drop `write_file`'s `dropped.length > 0` precondition at :1385**, which silenced the coverage park in four of five sessions, and fix the tool description at :5, which promises `.__best__` unconditionally.
**Known residual, honestly stated.** None of this sees a *body-level* rewrite: a change that removes no definition name and grows the file is invisible to every name-set and every size-gated check. That is exactly the existing "no-shrink structural break detection" item and it is the harder half.
**Where.** `crates/nanna-tools/default-skills/edit_file/tool.ts:241-251, 265-282, 822-925, 958-971`; `write_file/tool.ts:5, 886-887, 1118-1128, 1385-1401, 1421-1433`.

---

#### P24.5 — A mutation the tool has already measured as breaking the file is reported as plain success **[NEW — merges three observations]**
Merges: the success flag, the downstream "ok" tags, and the fruitless-budget replenishment.

**Broken.** `edit_file` runs the file's real parser after writing, learns whether it parsed before, narrates an accurate verdict — and returns `success: true` regardless. Four consumers read only the flag, so a break is recorded as landed work: the memory row tag, the step digest, the mission digest, and the work-evidence budget that decides whether an item is making progress.
**Evidence.** `edit_file structure: <path> does NOT parse after edit (sh -n): line 14: syntax error near unexpected token 'fi'  [parsed before this edit]` followed 166 ms later by `Remembered (scoped, importance 1.5): [edit_file → <path> — ok] Edited <path>: repla...`; 120 such edits across the review, all tagged `ok`.
**Change.** **Change the reported outcome, not the `success` flag.** `success` on the write family means "the bytes landed" and three separate mechanisms read it that way (`loop_runner.rs:6099` epoch bump, `:6108/:6140` failure counting, `:5974` error routing). Carry the structural verdict as structured data on the result — `ToolResult.data` already exists and `scripted.rs:263` already forwards it — add a field to `ToolCallRecord` and `StepToolCall`, and have the three digest/memory sites render a third outcome (`ok — DOES NOT PARSE`). Apply the same change to `write_file`, which has the identical hole.
**Guardrails.** An absent, unrun, or fail-open verdict must never read as `bad` — `sh -n` is documented to cry wolf on valid bash where `/bin/sh` is dash, and a false `bad` would suppress completion and drain the item budget. Scope is only where a checker applies (`.sh`, `.bash`, `.json`, `.js`, `.py`, shebang'd extensionless): it is inert on `.rs`, `.ts`, `.md`, `.toml`.
**Optional second rung, lower confidence.** A pre-write refusal for non-Python languages, on a temp copy the way `pythonSyntaxCheck` already does. Resolve the shebang before trusting an extension, apply it to **all three** in-place mutators or none (`edit_file`, `write_file`, `file_buffer`), keep `force=true`, and route repeats through `failEscalating` — a flat refusal at this volume is its own wedge. Note the observed counter-pressure: models answered write-side holds by escalating to *more* rewrites, so measure notice-vs-hold before committing.
**Where.** `crates/nanna-tools/default-skills/edit_file/tool.ts:945-951, 981`; `write_file/tool.ts:1245`; `crates/nanna-agent/src/loop_runner.rs:1080, 2525, 6232`; `crates/nanna-agent/src/harness.rs:3159`.

---

#### P24.6 — The context tiers spend the window on a preamble they cannot shrink, and no path pins the user's request **[NEW — merges six observations]**
Merges: the double-charged summary, the append-only preamble, the inverted tier gates, the four-way-inconsistent pin, the misreported gate quantity, and pair-unaware cuts. All six are the same accounting failure.

**Broken.** Compression's only levers touch the message list, but the number that gates them also counts an injected preamble that only ever grows — and one copy of that preamble text is charged to the budget twice while never being sent at all.
**Evidence.** `replace_with_summary` appends to `consolidated_summary` (`context.rs:1774`) *and* pushes the identical text into `self.summaries` (`:1777`); `estimate_tokens()` sums `summaries` (`:1083`) and `estimate_request_tokens()` adds `consolidated_summary` on top (`:718-734`), while the request itself carries only the former — measured: with one message left, `estimated_tokens=4767 hard_limit=12288`, of which ~4,200 is a vector no model ever sees.
**Change, in dependency order:**
1. **Stop charging never-sent text.** Delete `self.summaries` or reduce `ContextSummary` to metadata. Its only readers, `get_full_context()` and `create_isolated()`, have no production callers. This alone removes the observed forced truncations.
2. **Make every tier gate on the quantity the request will actually carry**, with the preamble deducted from the threshold rather than added to the measurement: compare message tokens against `threshold − preamble`. `CT − P < HL − P` for all P, so the tiers become ordered by construction and the gentler rung stops being unreachable. Today they gate on *different* quantities and the aggressive one gates on the larger — 94 of 166 aggressive firings happened in states where the gentler rung's predicate was structurally false.
3. **Give the preamble a reduction path**, or (2) merely relocates the problem: as it approaches the limit both derived thresholds collapse. Re-summarize `consolidated_summary` when the room left for messages falls below the tracker's already-measured `max_observed_growth` — below that the next iteration provably cannot fit, which is a measured-rate bound rather than a chosen fraction. Route refusal here, never through to message truncation. Observed ratchet: 1,182 → 21,209 chars in one 22-minute stretch, 54% of the window.
4. **One pin rule.** `drop_oldest` and `compress` pin index 0; `truncate_to_limit`'s second loop, `replace_with_summary`'s `drain(0..)`, and `trim_if_needed` do not — and four comments assert the pin is universal. The message carrying the live request must survive every path. This needs a provenance marker set at `add_user_message_with_budget`, because index 0 and `role == user` both fail (the loop pushes synthetic user-role notices). Ship **after** (3), or refusing to drop the request just converts message-destruction into an over-limit request.
5. **Report the quantity the predicate used.** All four sites print `estimate_tokens()` beside a limit tested with `estimate_request_tokens()`; 149 of 166 aggressive warnings announce a number *below* the limit they claim was exceeded. Print the request estimate plus its parts. Fix the logging, not the predicate.
6. **Make the cuts pair-aware, by repair rather than by arithmetic.** Every cut removes a prefix, so the only reachable orphan is a leading `tool_result` with no matching `tool_use` — a one-message bound, not an open-ended snap. Tolerated by the local server; rejected by every other provider. Repair at assembly in the house style already used for eviction (`[superseded by later call — N chars removed]`), never a `debug_assert!` — a panic in a spawned turn is the documented silent-wedge signature.
**Note.** These reset per turn (a fresh context is built per chat request), so this bites *inside* one long turn, not across a conversation. That is still the common shape for "go fix the failing tests".
**Where.** `crates/nanna-agent/src/context.rs:718-734, 1080-1087, 1099-1111, 1119-1152, 1274-1286, 1728-1795, 1849-1870, 2107-2111`; `crates/nanna-agent/src/loop_runner.rs:2868-2967, 3667-3830, 5185`.

---

#### P24.7 — A provider fault mid-step re-enters blind, and its first move is a whole-file rewrite **[NEW]**
**Broken.** A transient fault builds a fresh context per attempt, so the attempt's accumulated tool transcript is gone while its side effects remain on disk. The recovery is a prose warning plus, at most, one tool name.
**Evidence.** A mean of 17 tool executions and 82 s of step time discarded per abort (max 104) across 44 aborts in one session; the note tells the model to "re-read the working artifact before any whole-file write" and cannot verify that it did.
**Change.** The data is not lost — every result is already persisted as `[tool → target — outcome]` and is recallable. Surface it, don't reconstruct it: (a) carry only the attempt's **side-effecting** calls (reads must be re-done and the note already says so) by extending the liveness ledger's single `last_side_effect` mark to the side-effect list it already counts; (b) add the missing pointer telling the model the full outputs are recallable. Bound it by a measured share of the model's live `hard_limit`, not by "the attempt's own tool calls" — that is unbounded (104 observed).
**Do not** put this in the never-compressed verified-outcome slot: it is an uncapped `Vec` rendered into a never-compressed block, 104 lines is ~13% of a small model's input budget, and its header ("do NOT re-do or rewrite work these lines already prove") directly contradicts the retry note's instruction to re-read.
**Where.** `crates/nanna-daemon/src/tasks.rs:1880-1918, 2366, 2829`; `crates/nanna-daemon/src/liveness.rs:167, 353-354`; render with the existing bounded `step_activity_digest` (`loop_runner.rs:1066-1092`).

---

#### Tier 2 — Burns the user's turns

#### P24.8 — The edit-rejection loop **[NEW — merges five observations]**
Merges: the line-numbered read format, the closest-text hint, the unverified cause, the missing file echo, and the never-escalating message. One user-visible failure: "the assistant re-read the same file three times and got nowhere."

**Broken.** Four separate defects compound into the product's most common wasted turn. (a) `read_file`'s only output format is `<padded line number><TAB><line>`, and `edit_file`'s miss message tells the model to copy that text back — so the text the product points at is never a valid `old_string`. (b) On a miss the tool hands back a 4-line, 240-char guess instead of the file it is already holding. (c) The guess anchors on the first non-empty line, keeps the earliest of equal-scoring matches with no report of ambiguity, and scans at most the first 500 lines. (d) The message asserts a *cause* — "the file's real content differs from your memory" — that the tool never checked, and never names the path it actually resolved. (e) All 294 rejections returned byte-identical guidance; `failEscalating` exists in the same file and is used once, on a different guard.
**Evidence.** `edit_file failed: old_string not found in <path> — the file's real content differs from your memory. ... Call read_file, copy the exact current text, then retry edit_file.` (`edit_file/tool.ts:830`), against `numbered.push(lineNum + "\t" + lines[i])` as `read_file`'s sole return path (`read_file/tool.ts:117`); 26 of one model's 121 rejections carried the `NN<TAB>` prefix verbatim.
**Change.**
1. **Echo the file.** On a miss, inline the current content under the existing, already-derived `ECHO_MAX = 65536` (`write_file/tool.ts:783`), with `write_file`'s truncated-preview behaviour above it. Do **not** reuse `read_file`'s 10 MB cap — that is a filesystem sanity limit, not a context budget. Decide the read-mark question explicitly: recording a mark weakens the blind-rewrite guard, not recording one leaves the model held on its next write. Steer the wording back to a targeted edit, not a rewrite.
2. **Strip a line-number block as a fallback only**, after the ordinary loose match already failed, and only on read_file's actual emit shape — every line prefixed, numbers consecutive, right-aligned to one common width. A bare per-line `^\s*\d+\t` is unsafe: tab-separated data files start that way.
3. **Fix the hint** if it is kept as an over-cap fallback: anchor on the longest distinctive line, report how many candidates matched, and replace the 500-line cap (an underived constant; the split above it already runs unconditionally). Note it currently returns *nothing* when the anchor line normalizes below two characters.
4. **Say only what was measured.** No mark recorded → "no read of `<path>` is recorded" (not "you have not read it" — the mark store is LRU-capped at 200 and its I/O is best-effort). Mark older than mtime → "changed after you last read it". Fresh mark → "your text does not appear in the N bytes currently at `<resolved path>`". `edit_file` must also *write* a mark on every successful edit, or consulting marks makes the message wrong more often than the old one. Correct the same over-claim in `write_file:801`, which asserts "the file has CHANGED since you last read it" even when no read was ever recorded.
5. **Escalate.** Route repeated identical misses through `failEscalating` (do not reuse its shared `fork:` key prefix).
**Where.** `crates/nanna-tools/default-skills/edit_file/tool.ts:57-61, 432-445, 470-493, 813-830`; `read_file/tool.ts:110-126`; `write_file/tool.ts:478-491, 774-819`.

---

#### P24.9 — A path the shell prints does not resolve to the same file when handed to a file tool **[NEW]**
**Broken.** On Windows the shell emits MSYS paths (`/d/Development/...`). `Path::new("/d/...")` has a root but no drive prefix, so the resolver takes the relative branch and joins it onto the workspace root, producing `D:\d\Development\...`. Reads of existing files report "does not exist"; writes create a phantom tree and report success. No tool result ever names the path it actually opened.
**Evidence.** Within one second: `code_search: "<path>" exists but is a FILE, not a directory` and `cat: <path>: No such file or directory` — the same string addressing two different filesystems depending on which tool receives it; a shadow tree at `D:\d\...` and `D:\tmp\...` has been accumulating for months.
**Change.** (a) In `resolve_path_with_workdir`, before the relative test, recognize `^/([A-Za-z])/` and `^/([A-Za-z]):[\\/]`, guarded by the literal-first precedence `repair_redundant_prefix` already uses so a genuine single-letter directory stays addressable. Do **not** apply `normalize_drive_paths` to `workdir` — it runs native→MSYS, the wrong direction for `current_dir`. (b) Return the resolved path from the bridge and echo it in every file tool's result whenever it differs from the string given; reconstructing it in JS is wrong, because the resolver may repair the path. (c) Split `runStructuralCheck`'s exit-127 branch: "checker absent" and "the shell cannot see the file we just wrote" are different facts, and today the second is silently discarded.
**Invariant to test.** For any path the shell prints, `read_file(P)` and `exec("cat P")` must address the same bytes.
**Where.** `crates/nanna-scripting/src/bridge.rs:490-520, 543-577, 1219-1274`; `crates/nanna-tools/default-skills/write_file/tool.ts:593-611`.

---

#### P24.10 — Whole-file reads are silently head-tailed at a boot-frozen threshold **[NEW]**
**Broken.** The tool-result stub threshold is documented as scaling with the model's context window. It is computed from `max_tokens` — the requested *output* budget — is never rebound when the window is demoted, and the value it reads is a hardcoded default that boot deliberately does not take from config. It is a constant 16,384 chars for every model. Above it, a read returns 600 head chars and 400 tail chars.
**Evidence.** `loop_runner.rs:6185-6188` computes `(self.config.max_tokens as usize * 2).clamp(2000, 32000)` while the field's own doc at `:501` claims `0 = auto (scales with model context window)`; on the machine's configured 1M-window model the threshold is still 16,384 chars, ~0.4% of the window.
**Change.** Derive it from the input window the runner already computes and logs, and rebind it on demotion (`window_scaled_output_reserve` at `:144` is the existing window-derived helper). Then exempt the two cases where a head-tail is unusable: the anti-destruction guard's echo, and a read the model issued to refresh a file it is editing — `write_file`'s own `ECHO_MAX` comment already reaches this conclusion ("64 KiB is a small local model's entire window") and that bound is dead above 16,384. Cheaper first step: the `inline: true` hatch already exists at `:6333-6367` and appears in no schema and no prompt.
**Also.** Log the stub decision (tool, byte length, threshold) in the Memory arm, which currently logs nothing while the Context arm logs "Summarized tool output" — that absence is why this went unmeasured.
**Where.** `crates/nanna-agent/src/loop_runner.rs:144, 501, 6185-6189, 6333-6400, 8075-8096`; `crates/nanna-daemon/src/agent_service.rs:105, 133-135, 169`.

---

#### P24.11 — A failed tool's text never enters the loop's own record **[NEW — merges two observations]**
**Broken.** `ToolResult::error` moves the message into `error` and leaves `content` empty; the record is built from `content`, and the struct has no error field. The model sees the text; the loop's own memory of what happened stores a name, an input, and an empty string.
**Evidence.** `output: response.result.content.clone()` at `loop_runner.rs:6078`, with two production comments asserting the opposite as a design property; the user-visible consequence is `Your most recent side-effecting command reported failure: <cmd> → reported failure with no output` emitted 34 ms after the run logged the command's actual failure text.
**Change.** Add `error: Option<String>` to `ToolCallRecord` and populate both fields at `:6078` — `output` with the raw failure text (unprefixed; the `Error: ` prefix defeats the exit-code parse) and `error` from `response.result.error`. Then have the verdict sites read `record.error` the way the sibling call at `:6157` already does. Rewrite the two test fixtures to construct records the way `:6078` does, so they fail unless the wiring is right, and delete the two comments asserting the false premise.
**Two consequences reachable from ordinary chat, which is the justification:** `detect_tool_call_loop` compares `prev.output == last.output`, so a command that fails two *different* ways compares equal on `""` and the user is told "you got the identical result both times" when the world changed; and `iteration_produced_information` always hashes `""` in the branch written to hash the error's first line, so a *changing* error never counts as novel and the step-budget counter advances through exactly the debugging loop it was meant to fund.
**Note when landing:** with real text present, the soft loop nudge stops firing on varying failures. Restate its derivation comment rather than leaving it stale. Fix the two blind length fields too — `output_len` on the slow-tool warning and `output_size` on the stats observation both measure `content` and therefore record every failure as zero-byte.
**Where.** `crates/nanna-agent/src/loop_runner.rs:842-849, 895-911, 1015-1026, 5938, 5965, 6078, 8195-8225, 8242-8281`; `cratests/nanna-tools/src/lib.rs:164-171`.

---

#### P24.12 — The outer deadline preempts the tool and discards its honest message **[NEW — merges two observations]**
**Broken.** The registry wraps every call in a timeout built from the tool's *static manifest* ceiling, blind to the per-call deadline the script engine actually enforces. So a caller asking for a longer command deadline is killed early, and when the wrapper wins it replaces the tool's carefully built message — elapsed time, which deadline fired, "disk is truth", what to check before re-running — with four words and empty content.
**Evidence.** `registry.rs:635-636` returns `ToolResult::error("Tool execution timed out")` while the engine's own `effective_timeout_ms` deliberately extends its deadline by a 10 s handoff margin "so the bridge (which can kill the child) always fires first"; observed as `Tool exec failed in 180004ms: Tool execution timed out`, with the tool's real answer arriving 1.03 s later to nobody.
**Change.** Give the registry a params-aware timeout that applies the same existing `ENGINE_TIMEOUT_HANDOFF_MARGIN_MS`, so the inner, better-informed message wins by construction — reuse of an existing derivation, no new constant. This also removes the silent truncation of legitimately long commands. Only then, as a genuine last resort, have the backstop state elapsed wall time and the side-effect warning.
**Where.** `crates/nanna-tools/src/registry.rs:613-645`; `crates/nanna-tools/src/skills/scripted.rs:292-294`; `crates/nanna-scripting/src/engine.rs:327-346`.

---

#### P24.13 — Repetition guards key on argument bytes, so rewording defeats them **[NEW — merges two observations]**
**Broken.** Every repetition guard keys on `(name, canonical arguments)`. A model that rewords a failing command — adding `2>&1`, a pipe, a `cd` prefix, a different timeout — opens a fresh ledger key each time, and an interleaved success bumps the world epoch and re-arms everything. Separately, N *different* edits that produce the byte-identical parse error are invisible to all three guards, because each call differs.
**Evidence.** Fifteen successive attempts at one hanging script under nine distinct command strings returned no usable output and nothing noticed; and 25 consecutive successful edits produced the same failing verdict for 12m44s, each receiving the identical static sentence "Fix that line with another edit_file."
**Change.** (a) Track the last structural verdict per canonical file path (normalize `<file>` vs `./<file>` — one break rendered two ways split a 22-long streak into 5 and 17), and after three consecutive mutations of that path yield the same normalized signature, escalate the sentence once: say that N different edits produced the same error and name a different strategy. Normalize by stripping the `path: line N:` prefix and keeping the token plus the echoed source line, or the common case (a line number drifting as edits add lines above) never matches. This streak must **not** inherit the epoch gate — the mutations that bump the epoch are the evidence it counts. (b) Extend the name-level, paraphrase-proof detection already built for discovery tools to any call shape whose repeats keep returning no information, keyed on the observed result rather than argument bytes.
**Bound.** Reuse the existing three-in-a-row rung; the in-tree precedent (`ZERO_DELTA_DISCOVERY_BREAKER_AFTER`) is an outcome-streak guard added for exactly this reason.
**Where.** `crates/nanna-agent/src/loop_runner.rs:895-911, 933, 965, 1573, 5795-5834`; `crates/nanna-tools/default-skills/edit_file/tool.ts:238, 637-648`.

---

#### P24.14 — The decomposition rung is charged for and never changes its ask **[NEW]**
**Broken.** When an item stalls, the harness asks the model to break it into subtasks. It correctly measures that the attempt produced nothing, withholds the budget reset — and then asks the identical question again, then abandons, describing the outcome as though decomposition had happened.
**Evidence.** 84 firings, zero subtasks, split exactly 42 at the first attempt and 42 at the second, every one of the 42 items abandoned with `reason=abandoned after 5 fruitless steps and 2 replans`; the durable record carries `{"produced_work": false}` and nothing reads it.
**Change.** On a dry attempt, do not repeat the ask: put the item's own last failing result in front of the model — the replan prompt is the only step prompt that never receives it — and ask for the single next concrete action. Make the abandonment reason say both attempts returned nothing. Note that the replan branch `continue`s ahead of every escalation the harness owns, so a zero-tool replan step never increments the narration counter and never receives steering; and the abandonment gate still kills the item one iteration after the escalated ask, so the escalated rung must count as an execute step or replenish on a tool call, or it is decorative.
**Scope honestly.** The rung has decomposed successfully on record (~2% of instrumented attempts); this is escalation, not removal. It only reaches sessions already grinding, and the defaults that govern it (5 steps, 2 replans) govern ordinary chat turns too.
**Where.** `crates/nanna-agent/src/harness.rs:1301-1302, 1562-1583, 2054, 2263-2275, 2304-2310, 2412-2440`.

---

#### P24.15 — Structured text is sentence-scored, and both fallbacks damage it **[NEW]**
**Broken.** The context compressor asks the summarization model for one score per sentence and treats `\n` as a sentence terminator, so a file listing's "sentences" are its lines. The scorer is capped at 256 output tokens, so any input past roughly 128 lines can never return a matching score vector — for any model, however capable. When scoring does work, survivors are trimmed and joined with spaces, flattening the listing onto one line with the line-number prefixes still attached, under a banner calling it a summary. When it fails, the fallback silently deletes the middle 75% without naming which lines went.
**Evidence.** `compressor.rs:100` caps the scorer at `max_output_tokens.min(256)`; every successful scoring observed was on ≤37 sentences, and 116 rewrites across four models burned a scoring round-trip (3.3-3.7 s each) before falling through.
**Change.** Detect line-structured content before scoring — a high ratio of newline-terminated lines, leading line-number or indentation structure, diff or JSON framing — and route it to a shape-preserving reduction: whole lines, indentation intact, line numbers contiguous, elided ranges named in the banner. Reserve sentence scoring for prose; the wasted round-trip disappears as a side effect. Do **not** add a per-model failure counter — it is an arbitrary retry count and empirically wrong (one model succeeded, failed, then succeeded 25 more times). Also mark compressed slots so a later pass does not re-compress its own ~380-char banner.
**Where.** `crates/nanna-agent/src/compressor.rs:100, 139-155, 295-348`; `crates/nanna-agent/src/context.rs:1909-1944`.

---

#### Tier 3 — The assistant tells the user something untrue

#### P24.16 — What the assistant says when it stops **[PARTLY COVERED — merges four observations]**
Merges: abandoned work with no check vanishing, the fused completion count, the ending that promises evidence and prints none, and the cancel that suppresses it.

**Broken.** Four defects in the closing message, all the same shape: every *named* list on the report is check-bearing, so work without a machine-checkable done-condition is never named on either path.
1. **Abandonment leaves a count, not a name.** An abandoned item is recorded for re-examination only if it carries a check; a second abandonment site records nothing at all even when one exists. Across the whole task store, 81% of items ever abandoned had no check — this is the majority path, not an edge case. In one observed session the item that vanished was the root goal itself.
2. **"N items completed" fuses three different closures** — a check passing on work done, a check that already passed before the item started, and the model's own word — and the counter that would separate them is dropped by the multi-round merge, so a display fix alone would print a *new* false number.
3. **The dry ending promises evidence and prints none.** `"re-planning found no new work, but the evidence below is still unmet. 0 items verified done, 0 checks still failing"` renders with no list, while the environment ledger on disk recorded that a file the turn wrote does not parse. **[The reseed half is COVERED by "arm the reseed off environment verdicts"; see the correction below.]**
4. **A cancel suppresses the evidence with the banner.** The unmet list is never printed on a cancelled ending, and it is unrecoverable next turn — cancelled tasks are filtered out of every context path and the verdict lives only on the in-memory report.
**Change.**
- Record every abandonment, checked or not, as a first-class `abandoned_unverifiable` list carrying the item's last result; fix **both** abandonment sites. The detail field already exists and is live at the abandonment site.
- Merge `items_completed_unverified` (and `false_success_claims`, `items_revived`, `replans`) at both round-merge sites **before** changing any display; `fold_reports` already does this correctly and is the model to follow. Then report composition rather than the sum, naming buckets by *which door* — closed after a passing check / closed on a check that already passed / closed on the model's word — and stay quiet when there is nothing to disclose.
- Restrict the word "verified" to the first bucket only.
- Split the unresolved-evidence rendering out of the banner into its own function and call it on the cancel path with the banner still suppressed — it cannot be "the same code one branch higher", because the banner carries a `why` string the cancel arm does not produce. Carry the measurement's age on that path; a cancel's verdict can be hours stale.
- **Correction to the covered reseed item:** the ending half stands on its own; the *reseed* half is unproven and mechanically mismatched — the reseed's documented job is clearing a runner wedge, and a file that does not parse is no evidence of one. Before trusting a `chk` verdict, check its currency against the file's size on disk (`meta.len() == entry.last`); `chk` is the last verdict that *ran*, is sticky when no checker applies, and never refreshes after an out-of-band repair.
- Also note: a dropped item is actively barred from returning — cancelled titles are treated as closed-this-turn and silently deduped out of any re-proposal.
**Where.** `crates/nanna-agent/src/harness.rs:1363, 1694, 1907-1949, 2074-2077, 2360-2370`; `crates/nanna-daemon/src/control/chat_harness.rs:908-925, 1166, 1235-1251, 1532-1540, 1591-1606, 1688-1706, 2202-2272, 2800`; `crates/nanna-daemon/src/tasks.rs:3344-3358, 3490-3499`. One existing test asserts the cancel suppression and must be rewritten with the change.

---

#### P24.17 — The activity badge asserts an activity it has not observed, and the context meter is dead **[PARTLY COVERED — sharpens "artifact-staleness instead of is_running"]**
**Broken.** Two honesty holes in the same surface. (a) The badge computes `Running X… → Streaming… → Thinking… → Working…` with no elapsed time and no quiet time, and `isStreaming` latches on the first text chunk and clears only at the end of the whole run — so a turn that streamed anything and then went silent pulses `Streaming...` for as long as the silence lasts. A stuck turn and a working turn are pixel-identical. (b) Chat lost its context gauge when chat moved off the old direct path: the only `ContextUsage` emitter lives in a closure the harness never sets, and the run handle does not expose the atomics, so the driver structurally cannot fill them.
**Evidence.** All 59 captured run-state snapshots read `context_used=0, context_window=0, run_input_tokens=0, run_output_tokens=0`, 56 of them with `is_running: true` and tool calls accumulating, while the daemon computed the real figures continuously.
**Change.** (a) The daemon already emits `LivenessBeat` with `elapsed_s`, `quiet_s`, `phase`, `awaiting` every ~30 s — the GUI's event enum has no such variant and no `#[serde(other)]`, so every beat currently fails deserialization into "Unknown message format". Add the variant, forward it, and render `awaiting` (or "…(Ns since last output)") on **all four** badge branches, not just the idle one. (b) Set `on_usage` in the harness `RunOptions` and plumb the four atomics onto the run handle the way accumulated text already is; also stop re-zeroing the meter from run-state on every session load.
**Where.** `gui/app/components/SessionActivityBadge.vue`; `gui/app/composables/useSessionState.ts:334-363`; `gui/app/pages/index.vue:34, 649, 659-660, 786`; `gui/src-tauri/src/daemon_client.rs:120-142, 507`; `crates/nanna-daemon/src/tasks.rs:2929-2966`; `crates/nanna-daemon/src/agent_service.rs:366-374, 628-656, 1148`.

---

#### P24.18 — Memory tells the user its record is safe while discarding it **[NEW — merges two observations]**
**Broken.** Three related dishonesties on the memory path. (a) When embedding fails, the capability notice tells the model "Memory and tool-result writes still SUCCEED and are stored in full … queued for embedding backfill" while the same error returns from the write path *before* any insert — nothing is stored and nothing is queued. **Both** branches that raise this notice are false, including the no-provider branch that a fresh install hits. (b) `recall` answers a bare "No memories found matching: X" when rows are bound to a model they have no vector for, and when the query vector's width does not match the store's binding at all — a total blackout reported as an empty result. (c) An oversized write is beheaded at 30,000 bytes with no marker in the row, on one write path but not the other, so whether a long note survives depends on whether a workspace happened to be active.
**Evidence.** The notice text at `server.rs:3005` against `remember_scoped` returning at `service.rs:1029` before `store.add` at `:1165`; the store itself declares an empty active embedding legal ("it is the queued-for-backfill state"), so the notice can be made true rather than reworded.
**Change.** (a) On embed failure, persist the row with an empty vector and no buckets — the exact state the backfill already drains — skipping the neighbour-dedup search, which needs a query vector. This makes both notices true as written and matches the file's own stated invariant: "The write always lands; losing a vector costs temporary searchability, losing the write cost the memory." (b) Compute the searchable/total split inside the search scan (`lib.rs:736` already evaluates the width predicate per row, so it is free and exact at answer time — do not reuse the rebind-time snapshot, which goes stale) and make the empty answer distinguish three states: nothing matched, N of M awaiting re-embedding, and the query width does not match the binding. (c) Delete the 30k truncation and rely on the chunker both paths already have; the cap's own justification is superseded. If any content is ever genuinely dropped, the marker must name its own row, because it will be embedded and can propagate into a neighbour.
**Also.** A provider *switch* takes the healthy arm of the ledger and asserts "new writes are searchable normally" while most of the store is not. And `try_restore_primary` has no callers anywhere despite its doc saying "call this periodically", so a fallback that wins once holds the binding indefinitely.
**Where.** `crates/nanna-memory/src/service.rs:305, 873-891, 1019-1034, 1155-1165, 2395-2412`; `crates/nanna-memory/src/lib.rs:617-633, 701-736`; `crates/nanna-daemon/src/server.rs:2999-3019, 3336-3378`; `crates/nanna-daemon/src/embedding_router.rs:342`; `crates/nanna-tools/default-skills/recall/tool.ts:60`; `crates/nanna-daemon/src/control/memory.rs:114`.

---

#### P24.19 — A refused write is recorded, and re-read, as one that succeeded **[NEW]**
**Broken.** Two placeholder substitutions replace a write call's `content` with text asserting the bytes landed, neither gated on the outcome. One lands in the persisted record and the GUI's Input pane, so a card marked failed shows an Input claiming success. The other is worse: it is written into the assistant's own stored turn *before* the tool runs, so the model re-reads "all N bytes were written successfully and are intact on disk; read_file to see them" on every later turn, immediately beside a tool result reading "WRITE HELD — nothing was written and nothing is lost."
**Evidence.** `loop_runner.rs:6056-6072` has no success guard while storing `success: false` fourteen lines later; ~42 occurrences across the review, all on legitimate shrink-guard holds.
**Change.** Site 1: three-way — success / short-circuited-by-the-harness / genuine failure. Site 2 cannot be gated on an outcome that does not exist yet: make the wording outcome-neutral ("…the tool result below is the authoritative record of what happened on disk") or rewrite the stored block after execution.
**Where.** `crates/nanna-agent/src/loop_runner.rs:5648-5680, 6056-6079, 8112-8117`; `gui/app/components/ToolCallCard.vue:35`.

---

#### P24.20 — Diagnostics report a quantity the decision did not use **[NEW — merges three observations]**
**Broken.** Three log-layer assertions mislead whoever is diagnosing a user's stuck session, which is how this product gets debugged. (a) The compression warnings print the message-side estimate beside a limit tested with the request estimate — covered as change (5) of P24.6, listed here because the same fix must reach `context.rs:1281-1286` and `:1141-1147`, and because the "compression complete" line reports success against a quantity the exit condition never tested. (b) `Script executed successfully` is logged with `tool = tool.name`, which is the source file stem and therefore the literal string `tool` for 6,677 of 6,726 lines — a structured field carrying zero information, on a line that immediately precedes 1,164 tool failures. (c) A compression pass that removed nothing still logs completion, because the truncation helper's loop condition is already false and it has no error return.
**Change.** (a) Print the request estimate and its parts at all four sites plus the completion line. (b) Pass the declared manifest name into the scripted tool at load time, or drop the field; do not merge engine and tool outcomes into one line — the scripting crate sits below the tools crate and does not own tool-result semantics. (c) Report zero reduction as zero reduction. **Do not** repoint `summarized_len`: it is the offset of the last chunk actually read, a divergence detector, and the output length is already on the same line as `summary_len`.
**Where.** `crates/nanna-agent/src/loop_runner.rs:3780-3830`; `crates/nanna-agent/src/context.rs:1141-1147, 1274-1286, 1353-1360`; `crates/nanna-scripting/src/engine.rs:214`; `crates/nanna-scripting/src/tool.rs:44-49`.

---

#### P24.21 — Errors that prescribe an action the caller cannot perform **[NEW]**
**Broken.** `web_search` fails with "BRAVE_API_KEY not set. Configure it in your environment or nanna config" — neither route is available to a tool call: a child shell cannot mutate the daemon's process environment, and the config path appears dead (the config field reaches a boot log line and no consumer). `exec`'s missing-argument error is the only bare `Error: Missing required parameter` among 42 skills: it names no tool, does not say nothing ran, and lists none of the five aliases it accepts.
**Evidence.** 16 failures followed by 11 shell calls attempting `export BRAVE_API_KEY=…` inside subshells; `Nanna.getEnv` reads the daemon's live process env, so the advice can never be followed in-turn.
**Change.** Reword both to name an action available in this session ("web_search is unavailable in this session: no key is set in the daemon's environment. Nothing was searched. Use web_fetch on a known URL, or ask the user to set it before starting."; same for the batch variant). Give `exec`'s argument error the house style used by every other file-touching skill. Add `requires:` to the two web skills — 20 default skills already use it, with the rationale in the loader: "An advertised tool that can only fail is worse than an absent one." Separately, verify whether config-only key placement is meant to work at all; if it is, that is a second bug.
**Where.** `crates/nanna-tools/default-skills/web_search/tool.ts:19`; `web_search_batch/tool.ts:24`; `exec/tool.ts:28`; `crates/nanna-tools/src/registry.rs:737-753, 968-985`; `crates/nanna-config/src/lib.rs:704`; `crates/nanna-daemon/src/server.rs:3863-3868`.

---

#### Considered and rejected — do not re-raise

Each of these was proposed, tested against the evidence, and killed. Where a real residual survives, it is folded into a numbered item above and named here so nobody re-derives it.

- **Convert the structural sentence into a hard gate for every checkable language.** Refuted: the sentence already drives unaided recovery, valid→invalid-only gating was tried and abandoned in-tree, and `sh -n` cries wolf on valid bash where `/bin/sh` is dash. Residual folded into P24.5 as an optional, shebang-resolved second rung.
- **Remove the "file got smaller" precondition from the write guards.** Refuted: those guards fired 13 times on exactly their target shape; the two events the change would newly catch destroyed nothing, and the change would bounce ordinary rename-heavy refactors. The real gap is body-level rewrites, which is the existing "no-shrink structural break detection" item.
- **Pin the most recent read against compression.** Refuted: the summarizer's `keep_count = 2` already preserves the most recent tool round, and the dominant cause of a stale `old_string` is the model's own successful intervening edit. Compression correlated with *fewer* rejections.
- **Cap memory rows by what a top-k recall could return.** Refuted: chunks past top-k are reachable by handle dereference and by direct similarity hit, and the design promises byte-for-byte reassembly. Corrected bound in P24.3.
- **Bound the verified-outcome slot.** Refuted: it is per-step, reseeded from a capped source; the growth measured against it was the preamble (P24.6).
- **Give the planner a longer first-call deadline / a rolling latency ceiling.** Refuted: the fallback plan is the designed path for ordinary conversation, the session that took the fallback did the best work, and timed-out calls are never sampled so the proposed statistic is unmeasurable. Small residual: the effective-window latch is populated at request-build time, after the budget is logged, so the first request of a process is budgeted against the model card. Self-heals in milliseconds; worth fixing only on principle, and it would bite a first turn that *does* carry a large workspace slice.
- **Escalate a wedged runner to a full server restart.** Refuted: the unload demonstrably cured the fault every time (mean 23 tool executions in the following two minutes), and a third of the faults were model-output encoding errors no restart can fix. A shared-server restart would have hurt the strongest session.
- **Set a repetition penalty / change sampling on wedged streams.** Refuted: the wedge fires at the first token, before any repetition history exists; the fault is runner state and the unload is the cure.
- **Reject malformed tool calls at dispatch.** Refuted: the tools already check arguments before any I/O in 6-7 ms, the model round-trip is unavoidable, and a schema-level required check would break the alias sets the tools deliberately accept. Residual (the corrective message never escalates) folded into P24.8.
- **Report escaped quotes / per-file "shape" signatures.** Refuted: the in-tree precedent chose *repair* over reporting for the sibling case, on the recorded grounds that a model cannot see its own serialization and resends byte-identical content when told.
- **Report a delta when a repeated command returns a different result.** Refuted: the "repeated" commands were five different commands, and a neutral "it changed" notice would fire loudest on the healthy edit→retest loop.
- **Hide or gate tools with no credential** beyond the existing `requires:` convention. Refuted: the mechanism already exists with this exact rationale in its comment. Residual folded into P24.21.
- **Delegated-step model disclosure** (small, genuinely open): the spawn result carries the model that ran a delegated step and the tool drops it, so a transcript never names which model did that work. Worth one line if someone is in the file.

---

#### Sequencing note

P24.6 items (1) and (2) must land before (4); P24.16's counter merge must land before its display change; P24.4's removal note should ship before its hold. P24.1 is an open PR and blocks nothing.
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
- **LLM providers:** add Google Gemini, Mistral, Grok (xAI); custom OpenAI-compatible endpoints;
  *(Bittensor/Chutes-backed inference is **not** a backlog provider bump — it carries privacy, incentive,
  and endpoint-variance questions of its own; see **P21**)*; model
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
- **GUI:** **Active quality track lives in P4 follow-on (testing + UI/UX fix + simplification).**
  Remaining aspirational: command palette extras beyond navigation; full-text session search; export
  conversations (MD/PDF/JSON); context-budget visualization; live run view (iteration, active tools, token
  burn-rate, Gantt); drag-drop upload; split view; font-size + accent controls; ARIA/keyboard a11y; Vue error
  boundary; lazy-load Monaco; theme-token audit; compact power-mode density.
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
     - [x] `@tiptap/* 2.27.2 → 3.30.3` — *(2026-08-25)* **landed; three silent breakages, none of which
           `vue-tsc` or the 234-test suite caught.** (1) `BubbleMenu` is gone from the `@tiptap/vue-3`
           root and lives at `@tiptap/vue-3/menus` — the old import still compiled and still typechecked
           (the package re-exports `@tiptap/core`, so the name looks present) and evaluated to
           `undefined`, so the floating toolbar would have rendered as an inert unknown element.
           (2) v3 dropped tippy for Floating UI, so `:tippy-options` fell through to the DOM as an
           unknown attribute — replaced with `:options` (`placement: 'top'`, `offset: 8`) and the now
           dead `nanna-bubble` tippy theme CSS deleted. (3) StarterKit 3 registers `link` itself
           (verified by enumerating its members), so the separate `Link.configure({ autolink: false })`
           was a duplicate extension name whose config Tiptap would have discarded — and that
           `autolink: false` is the content-integrity fix that stopped `test_01.sh` becoming
           `test_[01.sh](http://01.sh)` in outbound mission text. Link is now configured through
           `StarterKit.configure({ link: {…} })` and `@tiptap/extension-link` dropped as a direct dep.
           `@tiptap/extension-placeholder` 3.x is a thin re-export of `@tiptap/extensions` and still
           works. 237 vitest, `vue-tsc --noEmit` clean, `pnpm build` green.
     - [x] `vue-router 4 → 5` (major) — *(2026-08-25)* landed, zero source changes. The direct
           `^4.6.4` req was the *mismatch*: `nuxt 4.5.2` already depends on `vue-router ^5.2.0`, so the
           tree carried the router Nuxt owns plus a stale 4.x pin. `pnpm why` now shows a single 5.2.0.
     - [x] `vue-sonner 1 → 2` (major — toast API) — *(2026-08-25)* landed. The toast API itself is
           unchanged; the breaking part is CSS: **v1 injected its stylesheet at runtime, v2 ships it as
           a separate `vue-sonner/style.css` export you must import.** Without it the toaster mounts,
           renders, and passes the "a toast really renders" e2e — completely unstyled and unpositioned.
           Added the explicit import in `ui/sonner/Sonner.vue` (the `vue-sonner/nuxt` module would do
           it automatically; we mount the component directly).
     - [x] `marked 17 → 18` (major — chat markdown renderer; audit render output)
           *(2026-08-24)* **Landed, 17.0.6 → 18.0.10, zero source changes.** The audit the item asked for
           is what made this cheap: the 26-test characterization suite written the same run (see the
           P11 markdown-sanitization item) pins both the injection behaviour and the ordinary rendering
           — emphasis, headings, ordered/unordered lists, GFM tables, strikethrough, task lists,
           blockquotes, `breaks: true` soft line breaks, entity escaping, empty input. All 26 pass
           unchanged on 18, and a side-by-side probe of the raw renderer output confirms the emitted
           HTML is byte-identical for every one of those cases. 185/185 vitest, `pnpm build` green.
           The suite is the durable half of this: the next renderer bump has to state what it changes
           rather than being taken on faith.
     - [x] **`lucide-vue-next` → `@lucide/vue` (package rename, not a version bump).** *(2026-08-25)*
           **Landed: `@lucide/vue@1.34.0`, specifier rewritten across 45 files, zero other changes.**
           One correction to the write-up below: `lucide-vue-next@1.0.0` **is** deprecated on npm
           ("Package deprecated. Please use `@lucide/vue` instead") but it is *not* an empty tombstone —
           it ships a real 38 MB dist and every icon resolves from it. That made it worse, not better:
           `pnpm update --latest` installs a working-but-dead package and nothing fails, so the trap is
           silence rather than breakage. Migrated to the live package instead of pinning back to
           0.577.0. The new `packageComponentExports` guard resolves all 221 icon bindings the
           templates render, so the rename is proven at the export level rather than assumed;
           `vue-tsc --noEmit` clean, 237 vitest, `pnpm build` green. *(2026-07-16 —
           *(2026-08-26, measured)* One claim about v1 that circulates in its release summaries is
           **wrong for this build**: icons are said to set `aria-hidden="true"` themselves. Driving
           the built app under WebDriver, **0 of the 10** rendered lucide SVGs carry the attribute.
           Whatever the docs mean, it is not unconditional — so nothing here should be assumed to
           have changed about accessibility.
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
     - [ ] *(2026-07-23)* **`nuxt 4.4.8 → 4.5.0` is a build-layer major in a minor's clothing** — 4.5
           moves the Vite builder to **Vite 8** (Rolldown internals), the Rspack builder to **Rspack 2 on
           `@rsbuild/core`**, and switches Nuxt's own build to `tsdown` (plus unhead v3 / unctx v3). Treat
           it as a migration item, not a sweep bump: needs a full `pnpm build` + `pnpm tauri dev` boot +
           `cargo tauri build` + WebDriver pass. Also note **Nuxt 3 EOL 2026-07-31** (we are on 4.x, so
           informational). Source: [Nuxt 4.5](https://nuxt.com/blog/v4-5).
     - [ ] *(re-tried 2026-08-25 — still blocked, now with the exact failure)* `typescript@7.0.2` +
           `vue-tsc@3.3.11` reverted: `vue-tsc` resolves `typescript/lib/tsc` at startup and TS 7's
           `package.json` no longer lists that subpath in `exports`, so the CI typecheck gate dies with
           `ERR_PACKAGE_PATH_NOT_EXPORTED` before reading a single file. Nothing in our source is
           involved — re-check when `vue-tsc` ships against the 7.1 programmatic API.
     - [ ] *(2026-07-23)* **`typescript 5.9 → 7.0` (GA 2026-07-08, the Go-native `tsgo` port).** Breaking:
           `--strict` on by default, `--target es5` / `--baseUrl` / `--moduleResolution node10` removed —
           and critically **no stable programmatic compiler API until 7.1**, which `vue-tsc` and the
           Nuxt/Vite type-check tooling depend on. Blocked on the toolchain catching up; re-check when 7.1
           ships. Source: [Announcing TypeScript 7.0](https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/).
     - [ ] *(2026-07-23)* **`vuedraggable` `latest` dist-tag is a trap (same class as the lucide tombstone).**
           `pnpm outdated` reports `4.1.0 → 2.24.3` — the v4 line is published under `next`, so `latest`
           points at the *older* Vue-2 package. **Never let `pnpm update --latest` "upgrade" this one**;
           it would silently downgrade to a Vue-2-only release. Keep the explicit `^4.1.0` req.
   - Pins now: `turso =0.6.1`, `aegis =0.9.12` (exact — pre-1.0), boa git rev `4f98f644` (until a
     crates.io boa ships icu 2.2). The old `wgpu` pin is dropped (see the wgpu 30 note above).
   - **`rten` is pinned at `0.24` by `ocrs`, not by us** *(2026-08-25)* — `cargo upgrade --incompatible`
     offers `rten 0.24 → 0.25`, and taking it is a hard error, not a migration: `ocrs 0.12.2` (latest)
     requires `rten ^0.24`, so the bump resolves **two** semver-incompatible `rten` crates and
     `OcrEngineParams { detection_model, recognition_model }` is then handed `rten-0.25::Model` where
     `rten-0.24::Model` is expected (E0308, verified by building it). Re-check when `ocrs` publishes
     against 0.25; until then the direct req must track whatever `ocrs` requires.
     - [ ] Re-try `rten 0.25` once `ocrs > 0.12.2` moves to it.
   - **`malachite-bigint` must stay at 0.9.2 — a bare `cargo update` breaks the release build**
     *(2026-08-25)*. `pymath 0.2.0` accepts `malachite-bigint 0.10` while `rustpython-codegen 0.5.0`
     requires 0.9, so `cargo update` resolves both and `rustpython-stdlib` fails to compile
     (`there are multiple different versions of crate malachite_bigint in the dependency graph`,
     17 errors, E0277/E0308). **This is release-only in practice** — it is the second failure of the
     exact shape the `rust-toolchain.toml` comment describes, so it is another reason a freshness pass
     is not verified until `cargo build --release -p nanna-daemon` is green. Held with
     `cargo update -p malachite-bigint@0.10.0 --precise 0.9.2`; the pin lives only in `Cargo.lock`, so
     **every future run must redo it after `cargo update`** until `rustpython` widens its req.
     - [ ] Drop the `malachite-bigint` lock pin once `rustpython-codegen` accepts 0.10.
     - [ ] `criterion 0.8 → "0.7"`: `cargo upgrade --incompatible` reports this every run and it is a
           **downgrade** — 0.8.2 is what resolves and builds. Do not take it.
   - *(2026-07-16 sweep)* `cargo update` → 12 compatible bumps (`tokio 1.52.4`, `uuid 1.24.0`,
     `keyring 4.1.5`, `regex 1.13.1`, `clap 4.6.2`, `syn 2.0.119`, `bitflags 2.13.1`, `bstr 1.13.0`,
     `regex-automata 0.4.16`, `simd-adler32 0.3.10`, `which 8.0.5`). `cargo upgrade --incompatible` →
     only two reqs behind: `deno_core 0.407 → 0.408` (nanna-scripting; compiled unchanged, no source
     edits) and `uuid 1.23 → 1.24` (workspace + nanna-server req bump). Workspace **including
     `nanna-gui`** builds green; scripting 19+1 / llm 28 / agent 61 tests pass; clippy clean on the
     bumped crates. Frontend: `tailwindcss`/`@tailwindcss/postcss 4.3.3`, `postcss 8.5.19`,
     `vue 3.5.40` applied green (`pnpm build` → nitro + client, 2.25 MB / 502 kB gzip).
   - *(2026-07-23 sweep)* `cargo update` → 7 compatible bumps (`clap`/`clap_derive 4.6.4`,
     `foreign-types-macros 0.2.4`, `glob 0.3.4`, `libc 0.2.189`, `syn 3.0.3`, `tokio-util 0.7.19`).
     `cargo upgrade --incompatible` → **nothing to do**: all 71 non-local packages already sit at their
     latest req, with only the intentional `turso`/`aegis` pins held back. Workspace (excl. `nanna-gui`)
     builds green; **597 tests pass, 0 failures**; clippy 2341 warnings / **0 errors** (this run's
     baseline). Frontend: `@tauri-apps/plugin-dialog 2.7.2`, `monaco-editor 0.56.0`,
     `happy-dom 20.11.1`, `postcss 8.5.22` applied green (56/56 Vitest, `pnpm build` clean).
     `nuxt 4.5.0` / `typescript 7.0` deferred with migration notes above.
     **`monaco-editor 0.55 → 0.56` needed a real migration, not just a req bump:** 0.56 added an
     `exports` map (`"./*": "./esm/vs/*.js"`), so the five deep worker specifiers in
     `gui/app/plugins/monaco.client.ts` (`monaco-editor/esm/vs/...`) stopped resolving — they now
     expand to `esm/vs/esm/vs/...` and `nuxt build` fails with *"Rollup failed to resolve import
     …editor.worker?worker"*. Fixed by importing through the exports map
     (`monaco-editor/editor/editor.worker?worker`, `monaco-editor/language/<lang>/<lang>.worker?worker`).
   - *(2026-07-23 sweep, parallel nightly run — merged the same day)* `cargo update` → 8 compatible bumps (`clap`/`clap_derive 4.6.4`, `libc 0.2.189`,
     `tokio-util 0.7.19`, `syn 3.0.3`, `glob 0.3.4`, `rustls-pki-types 1.15.1`, `foreign-types-macros 0.2.4`).
     `cargo upgrade --incompatible` → **nothing behind** (71 packages already latest; only the intentional
     `aegis`/`turso` pins + the boa git rev). Workspace builds green, **~600 tests pass**, clippy clean.
     Frontend: `nuxt 4.4.8 → 4.5.0`, `postcss 8.5.22`, `happy-dom 20.11.1`, `@tauri-apps/plugin-dialog 2.7.2`,
     and **`monaco-editor 0.55.1 → 0.56.0`** — the last one is a **real migration**, not a passthrough:
     0.56 (PR #5155 "exported modules reorganization") added a package `exports` map
     (`"./*": "./esm/vs/*.js"`), so every pre-existing `monaco-editor/esm/vs/<path>` specifier now resolves
     to `esm/vs/esm/vs/<path>.js` and `nuxt generate` **fails** ("Rolldown failed to resolve import
     `monaco-editor/esm/vs/editor/editor.worker?worker`"). Fix: drop the now-implicit `esm/vs/` prefix from
     all five worker imports in `gui/app/plugins/monaco.client.ts` (`monaco-editor/editor/editor.worker`,
     `monaco-editor/language/{json,css,html,typescript}/…`). `editor.worker.js` itself did **not** move —
     only the specifier did. Verified by the five worker chunks still emitting separately
     (editor 300 kB · json 430 kB · css 1.07 MB · html 740 kB · ts 6.9 MB), `pnpm generate` green, 56 vitest
     green. **Deferred majors this run** (each needs its own migration): `@tiptap/* 2 → 3.28`, `marked 17 → 18`,
     `vue-router 4 → 5`, `vue-sonner 1 → 2`, `lucide-vue-next → @lucide/vue` (rename), and newly
     **`typescript 5.9 → 7.0`** (the Go-port compiler — needs a `vue-tsc` compatible with TS 7 before it can land).
     **Merge note:** this run's `nuxt 4.4.8 → 4.5.0` bump was reverted to `4.4.8` at merge,
     pending the unresolved `UiSonnerSonner` component issue logged above; its monaco 0.56
     migration is the same one described in the previous bullet.
   - *(2026-07-24 sweep)* `cargo update` → 1 compatible bump (the boa git rev tracked `v0.4.4 → v0.4.5`).
     `cargo upgrade --incompatible` → **two majors, both applied green**: **`base64 0.22 → 0.23`**
     (`nanna-agent` + `nanna-gui`; the `Engine`-trait call sites in `image_util.rs` and the PKCE OAuth
     flow compiled unchanged) and **`deno_core 0.408 → 0.409`** (`nanna-scripting`, compiled unchanged).
     Everything else already sits at its latest req, with only the intentional `turso`/`aegis` pins and
     the boa git rev held back. **Toolchain tracked too — then reverted; see the ICE item below:**
     nightly `daf2e5e18 (2026-07-13)` → `89c61a754 (2026-07-23)`; the workspace was rebuilt and
     re-tested from scratch under it — **719 tests pass, 0 failures**, clippy **0 errors**
     (2354 pre-existing warnings). That is the **debug** profile only, and release codegen turned out
     to be broken on it, so the toolchain bump did not survive the run.
     *Gotcha for future runs:* `cargo-upgrade` rewrites CRLF→LF on any manifest it edits
     (`crates/nanna-agent/Cargo.toml` came back as a 33-line whole-file diff for a one-line bump) —
     revert and hand-edit the req instead, or the EOL churn buries the actual change.
     Also: running `rustup update` **concurrently with a `cargo build`** fails and rolls back on Windows
     (component files are locked) — sequence them.
     Frontend: `pnpm outdated` showed **only documented deferred majors** (`@tiptap/* 2 → 3.28`,
     `marked 17 → 18`, `vue-router 4 → 5`, `vue-sonner 1 → 2`, `typescript 5.9 → 7.0`) plus the
     `lucide-vue-next 1.0.0` tombstone that must never be taken. `pnpm update` re-landed
     **`nuxt 4.4.8 → 4.5.0`** (Vite 8 / Rolldown) — verified `pnpm generate` green (4 routes prerendered)
     and 56/56 vitest. This is the bump the previous run reverted at merge pending the
     `UiSonnerSonner` blocker; **that blocker is fixed in the next commit of this PR** (it was never
     nuxt's fault — it reproduced on pristine `origin/master`).
   - *(2026-07-25 sweep)* `cargo update` → 4 compatible bumps (`cc 1.3→1.4`, `either 1.16→1.17`,
     `simd_cesu8 1.1→1.2`, `webpki-root-certs 1.0.8→1.0.9`). `cargo upgrade --incompatible` → **two
     majors applied green**: **`base64 0.22 → 0.23` in `nanna-config`** — the previous run bumped it in
     `nanna-agent`+`nanna-gui` but left `nanna-config` (credentials.rs) on 0.22, so a stray 0.22 node
     lingered; the `general_purpose::STANDARD.{encode,decode}` call sites compiled unchanged. `base64
     0.22.1` still resolves for `tiktoken-rs 0.12` (its `^0.22.1` req), so the two coexist by design.
     And **`playwright-rs 0.14 → 0.15` in `nanna-browser`** — 0.15 made `Page::locator()` **synchronous**
     (returns `Locator` directly, no longer a future), so the 8 `self.page.locator(..).await` sites in
     `playwright.rs` dropped their `.await`; compiled clean under the `playwright` feature. Everything
     else already at latest req (only the intentional `turso`/`aegis` pins + boa git rev held back).
     Workspace (excl. `nanna-gui`) builds green; nanna-config 1 + nanna-browser 17 + dep_guard 1 tests
     pass; no banned deps entered the tree. Frontend `pnpm outdated`: **only the documented deferred
     majors** (`@tiptap/* 2→3`, `marked 17→18`, `vue-router 4→5`, `vue-sonner 1→2`, `typescript 5.9→7.0`)
     plus the two traps (`lucide-vue-next 1.0.0` tombstone, `vuedraggable` `latest`=Vue-2) — `package.json`
     is already at latest-safe, no GUI changes this run. *Reconfirmed the fmt gotcha: `cargo fmt -p <crate>`
     reformats the whole crate, not the touched file — `origin/master` isn't fmt-clean, so it churned 4
     unrelated files; reverted, kept only the surgical `.await` diff.*
   - *(2026-08-21 sweep)* `cargo update` → **~150 compatible bumps** (the biggest sweep in a while:
     `tokio-macros 2.7.2`, `ureq 3.4.0`, `uuid 1.24.1`, `wasm-bindgen 0.2.127`, `zbus 5.19.0`,
     `zvariant 5.15.0`, `zerocopy 0.8.56`, `zlib-rs 0.6.7`, `xml 1.4.0`, …). `cargo upgrade
     --incompatible` → four candidates, **one applied green, two reverted, one rejected**:
     - **`wide 1.5 → 1.6`** (workspace, consumed by `nanna-simd`) — applied, compiled unchanged.
     - **`playwright-rs 0.15 → 0.16`** (`nanna-browser`) — applied, compiled unchanged.
     - **`rten 0.24 → 0.25`** (`nanna-tools`) — **reverted.** `ocrs 0.12.2` still requires `rten 0.24`,
       so bumping our direct req puts **two `rten` versions in one graph** and `ocr.rs:309` fails with
       `expected rten::model::Model, found Model`. Not our migration to do:
       - [ ] Re-try `rten 0.25` once **`ocrs`** ships a release built against it (watch
             `ocrs`/`rten-imageproc`); the bump is a one-line req change plus a rebuild once the
             transitive pin moves.
     - **`criterion 0.8 → "0.7"`** — **rejected as a downgrade.** `cargo-upgrade` reports `latest 0.7.0`
       for criterion while the lock happily resolves `0.8.2`; taking its suggestion would walk the
       benches *backwards*. Never apply a `cargo-upgrade` row whose "latest" is below the current req.
     **A `cargo update` landmine worth remembering:** the sweep moved `malachite-bigint` to **0.10.0**
     for `pymath` while `rustpython-{codegen,compiler,derive}` stay on **0.9.2** — two versions of the
     same crate in one graph, and `rustpython-stdlib` then fails to compile with 17 `E0277`/`E0308`
     errors about `malachite_bigint::BigUint`/`BigInt` ("there are multiple different versions of crate
     `malachite_bigint`"). Pinned back with
     `cargo update -p malachite-bigint@0.10.0 --precise 0.9.2`.
     - [ ] Drop that pin when `rustpython 0.6` (or any release that moves to malachite 0.10) lands.
     **Verification:** workspace (excl. `nanna-gui`) builds green, **1555 tests pass / 0 failures**,
     `cargo clippy -p nanna-memory --all-targets` **0 errors**, and — closing the gate hole logged
     below — a **`cargo build --release -p nanna-daemon` was run and is green** on the pinned
     `nightly-2026-08-03`, so this sweep is verified against the profile the shippable artifact
     actually uses.
   - *(2026-08-22 sweep)* `cargo update` → ~150 compatible bumps (`tokio-macros 2.7.2`, `ureq 3.4.0`,
     `uuid 1.24.1`, `wasm-bindgen 0.2.127`, `wgpu 30.0.1`, `zbus 5.19.0`, `zvariant 5.15.0`,
     `zerocopy 0.8.56`, `zlib-rs 0.6.7`, `xml 1.4.0`, `zerovec 0.11.8`, …).
     `cargo upgrade --incompatible` offered four majors: **`wide 1.5 → 1.6`** (workspace/`nanna-simd`)
     and **`playwright-rs 0.15 → 0.16`** (`nanna-browser`, verified under `--features playwright`)
     both **applied**, compiled unchanged; **`rten 0.24 → 0.25`** **reverted** (see the blocked item
     below); **`criterion 0.8 → "0.7"`** **rejected** — `cargo-upgrade` reports `latest 0.7.0` while the
     lock resolves `criterion 0.8.2`, so taking the suggestion walks the bench harness *backwards*.
     Landmine (same one the 2026-08-21 run hit — it is reproducible, not a one-off): the sweep moves
     **`malachite-bigint` to 0.10.0** for `pymath` while `rustpython-{codegen,compiler,derive} 0.5.0`
     stay on **0.9.2**, and `rustpython-stdlib` then fails with 17 `E0277`/`E0308` errors about
     `malachite_bigint::{BigUint,BigInt}`. Pinned back with
     `cargo update -p malachite-bigint@0.10.0 --precise 0.9.2`; **re-apply this pin after every
     `cargo update` until `rustpython 0.5` unifies the req.**
     Verified: workspace (excl. `nanna-gui`) builds `--all-targets` green, **1555 tests pass / 0 fail /
     12 ignored**, clippy **0 errors** (2742 warnings = this run's baseline), and
     `cargo build --release -p nanna-daemon` green.
     Bench (`nanna-bench vector_search`, release, 4070 Ti SUPER / Zen 4, 768-dim): **42.0 µs @ 1k ·
     1.31 ms @ 10k · 9.05 ms @ 50k** — every budget held (≤0.20 / ≤5.0 / ≤25 ms); no regression from
     `wide 1.6`. Baseline p50s left unchanged (the 10k/50k improvement is not A/B-attributed).
     Frontend: `happy-dom 20.11.6`, `vitest 4.1.11`, `vue-tsc 3.3.11` applied green (**159/159 vitest**);
     `pnpm outdated` otherwise shows **only the documented deferred majors** (`@tiptap/* 2.27 → 3.30`,
     `marked 17 → 18`, `vue-router 4 → 5`, `vue-sonner 1 → 2`, `typescript 5.9 → 7.0`) plus the
     `lucide-vue-next 1.0.0` tombstone that must never be taken.
   - *(2026-08-23 sweep)* `cargo update` → 7 compatible bumps (`blocking 1.7.0`, `crc32fast 1.5.1`,
     `log 0.4.34`, `uuid 1.25.0`). **The `malachite-bigint` landmine recurred exactly as documented** —
     the sweep re-added 0.10.0 alongside `rustpython-{codegen,compiler,derive} 0.5.0`'s 0.9.2; pinned
     back with `cargo update -p malachite-bigint@0.10.0 --precise 0.9.2`. Re-checked upstream: nothing
     has moved, so **keep re-applying this pin after every `cargo update`**.
     `cargo upgrade --incompatible` offered three, of which one was taken:
     **`uuid 1.24 → 1.25`** (workspace + `nanna-server`) applied — hand-edited, not via `cargo-upgrade`,
     to avoid its documented CRLF→LF whole-file churn. **`criterion 0.8 → "0.7"` rejected again** (the
     lock resolves 0.8.2; taking the suggestion walks the bench harness backwards). **`rten 0.24 → 0.25`
     still blocked** — re-verified against crates.io this run: `ocrs` is *still* 0.12.2 and still
     requires `rten ^0.24`, so the two `Model` types would stop being the same type.
     **The intentional `turso`/`aegis` pins were re-examined and both moved** — a pre-1.0 pin is a
     "prove it before you take it" marker, not a permanent freeze, and the previous run had this bump
     in flight but unverified when it ended. **`turso =0.6.1 → =0.7.2`** and
     **`aegis =0.9.12 → =0.9.15`**: compiled with **zero source changes**, and **120 `nanna-storage`
     tests pass** including the `rusqlite`/`libsql`/`sqlx` dep-guard and the corruption classifier
     (`corruption_classifier_matches_turso_page_errors` still matches 0.7.2's page-error strings, so
     the recovery path did not silently stop recognising them). Worth taking on its merits, not just
     freshness: [Turso 0.7.0](https://turso.tech/blog/turso-0.7.0) makes the embeddable engine
     **non-blocking** — the core no longer blocks the calling thread on I/O, no longer aborts the
     process on OOM, and yields the CPU during long operations so one busy statement cannot starve
     other connections sharing the runtime. That is the exact failure class this repo has been bitten
     by (one shared connection under one mutex; an unfinished cursor swallowing later writes; a
     `turso_core` panic taking the daemon down mid-load).
     - [ ] **Exploit turso 0.7's non-blocking engine.** The single-shared-connection-under-a-mutex
           design was shaped by an engine that blocked its caller. Re-measure whether the mutex can
           narrow (or whether concurrent readers can be admitted) now that long operations yield —
           and re-check whether the "drop cursors before writing" rule is still load-bearing. Do not
           change the locking on inference alone; measure first.
     Bench (`nanna-bench vector_search`, release, 4070 Ti SUPER / Zen 4, 768-dim, fixed seed):
     **43.2 µs @ 1k · 1.53 ms @ 10k · 9.16 ms @ 50k** — every budget held with margin
     (≤0.20 / ≤5.0 / ≤25 ms), and 10k/50k came in **at or better than the recorded 2026-08-05
     baseline p50s** (1.63 / 10.1 ms). Criterion's "+4%/+11%/+6%" is against the previous run *on this
     machine*, not against the baseline table, and it is within the noise of a box that had just
     finished a Tauri release build; nothing crossed a ceiling, so turso 0.7.2 shows no regression on
     the search path. Baseline p50s left unchanged — these are not A/B-attributed improvements.
     Verified: workspace (excl. `nanna-gui`) builds `--all-targets` green, **1576 tests pass / 0 fail /
     12 ignored** (1555 was the previous baseline; +6 from this run's new `nanna-server` suite, the
     rest from turso 0.7.2's own test targets), clippy **0 errors**, and `cargo build --release -p
     nanna-daemon` green — the release gate the roadmap asked for below, which debug + tests cannot see.
     Toolchain: `rust-toolchain.toml` stays on `nightly-2026-08-03`; the release build above is the
     evidence the pin still holds.
     Frontend: `package.json` needed **no change** — `pnpm outdated` shows *only* the documented
     deferred majors (`@tiptap/* 2.27 → 3.30`, `marked 17 → 18`, `vue-router 4 → 5`,
     `vue-sonner 1 → 2`, `typescript 5.9 → 7.0`) plus the `lucide-vue-next 1.0.0` tombstone that must
     never be taken. `pnpm update` moved the lockfile within ranges only (`vite 8.2.1 → 8.2.2`,
     `rolldown 1.2.4 → 1.2.5`, `rollup 4.62.4 → 4.62.5`, `@oxc-project/types 0.144 → 0.146`), verified
     by **159/159 vitest** and a green `pnpm build` (nitro + client, 4 routes prerendered).
     - [ ] **`rten 0.24 → 0.25` is blocked on `ocrs`.** `ocrs 0.12.2` (still the latest) requires
           `rten ^0.24`, and `ocr.rs:299-312` hands an `rten::Model` straight into
           `ocrs::OcrEngineParams`, so bumping our direct req puts two `rten` versions in one graph and
           the two `Model` types stop being the same type. Re-check when `ocrs` ships a release that
           tracks `rten 0.25`.
     - [ ] **The `turso_core` release build is non-deterministic under the parallel rustc frontend.**
           On the pinned `nightly-2026-08-03`, `cargo build --release -p nanna-daemon` failed once with
           `error: queries overflow the depth limit!` in `turso_core 0.6.1` and then **succeeded on an
           immediately-repeated identical invocation**. The user-global `~/.cargo/config.toml` sets
           `rustflags = ["-Z", "threads=15"]`, so the depth/recursion accounting is thread-scheduling
           dependent. Treat a single depth-limit failure as *flaky, retry once* rather than as a
           toolchain incompatibility — but pin it down (repro under `-Z threads=1`, then report
           upstream) before it eats a release cut.
   - *(research 2026-08-23)* **Turso 0.7's non-blocking engine changes what this repo's storage
     design was working around.** [Turso 0.7.0](https://turso.tech/blog/turso-0.7.0): the core no
     longer blocks the calling thread on I/O, no longer aborts the process when it runs out of
     memory, and yields the CPU during long operations so a busy statement cannot starve other
     connections sharing a runtime. Also in 0.7: faster MVCC concurrent writes, leaner recovery,
     lower per-row-version memory, index-resolved parameters (~2x faster prepare on a
     thousand-parameter insert), runtime-registered custom storage backends via a global registry
     resolved through `vfs=`, PostgreSQL-style sequences and ICU collations. Three of those bear
     directly on decisions already made here — the single-shared-connection-under-one-mutex design,
     the "drop cursors before writing" rule, and the fact that a `turso_core` panic used to take the
     daemon down mid-load. Folded as a measurement to-do above, not a change: do not re-shape the
     locking on inference.
   - *(research 2026-08-23)* **`rustpython 0.5` still has not unified its `malachite-bigint` req**, so
     the `cargo update -p malachite-bigint@0.10.0 --precise 0.9.2` pin-back stays mandatory after
     every sweep. Nothing upstream has moved; `rustpython-{codegen,compiler,derive} 0.5.0` remain on
     0.9.2 while `pymath` pulls 0.10.0, and `rustpython-stdlib` then fails with 17 `E0277`/`E0308`s
     about `malachite_bigint::{BigUint,BigInt}`. Third consecutive run hitting it — it is a standing
     step, not an incident.
     *(2026-08-26)* **Stopped being a per-run step: the pin moved into the manifest.**
     `crates/nanna-scripting/Cargo.toml` now carries `malachite-bigint = { version = "=0.9.2",
     optional = true }` under the `python` feature, as a transitive-version pin — exactly the lever
     `aegis` already uses in `nanna-storage`, and for the same reason. A lockfile pin is what
     `cargo update` is *entitled* to move, which is why this recurred on four consecutive sweeps; a
     manifest constraint is not. Optional so it never enters a build without Python, while still
     binding resolution (the daemon enables `python`, so the release build did compile it — that is
     why the failure was release-visible). Drop it when `rustpython-common` and `pymath` agree on one
     malachite.
   - *(research 2026-08-23)* **`ocrs` is still 0.12.2, so `rten 0.24 → 0.25` remains blocked.**
     Re-verified against crates.io this run rather than assumed. `ocrs` still requires `rten ^0.24`
     and `ocr.rs` hands an `rten::Model` straight into `ocrs::OcrEngineParams`, so bumping the direct
     req puts two `rten` versions in one graph and the two `Model` types stop being the same type.
   - [x] *(research 2026-08-23)* ~~**Re-try the toolchain pin.**~~ **Done (2026-08-26): the pin moved
     `nightly-2026-08-03` -> `nightly-2026-08-25`** (rustc 1.100.0-nightly, e7769602a). Sequenced
     exactly as the previous run prescribed — `rustup update` **alone** with no cargo in flight, then
     a full `cargo build --release -p nanna-daemon`: **exit 0 in 18m55s**, no `rustc_codegen_ssa`
     tokio ICE and no `turso_core` const-eval depth overflow. The three CI channels that mirror the
     toml moved in lockstep (`budget-gate.yml`, `release-check.yml`, `test-compile.yml` x3).
     The new nightly surfaces one thing the old one did not, and it is a real future-incompat rather
     than noise: `recursion_depth_exceeding_limit` ([rust#159228](https://github.com/rust-lang/rust/issues/159228)),
     raised proving `DaemonServer::run`'s scheduler closure is `Send` through
     `MemoryService` -> `VectorStore` -> `CosineSimilaritySearch` -> wgpu's `Global`/`Hub`/`Registry`
     graph — deeper than the default limit of 128. It is scheduled to become a **hard error**, so it
     is answered now, at the crate roots (`#![recursion_limit = "256"]` on both
     `nanna-daemon/src/lib.rs` and `src/main.rs`) rather than left to break a later bump. Solver depth
     only: no behaviour and no codegen change.
   - *(2026-08-26)* **A third hole in the verify gate, found by the toolchain bump and closed in the
     skill.** The two already on record were about the release profile; this one is about *scope*.
     This repo has a package at the workspace **root**, so a bare `cargo clippy --all-targets` checks
     only that root package and its path dependencies — **16 of the 20 members**, silently skipping
     `nanna-browser`, `nanna-proc`, `nanna-bench` and `nanna-gui`. It reads as a full-workspace gate
     and is not one. It mattered immediately: nightly-2026-08-25's new
     `recursion_depth_exceeding_limit` warning fired in **six** crate roots, and the
     `-p nanna-daemon` release build showed exactly one of them. `daily-dev`'s step 4 now prescribes
     `--workspace --all-targets --exclude nanna-gui`, matching `test-compile.yml`'s existing
     exclusion deliberately rather than by coincidence. The same step also now states plainly that
     `cargo fmt` is **not** clean on this tree (~2735 pre-existing diffs) and must not be made clean
     in passing — only the lines an increment adds need to be fmt-neutral.
   - *(2026-08-26 sweep)* `cargo update` -> 34 compatible bumps (wgpu/naga 30.0.0 -> 30.0.1,
     aes-gcm 0.11.1, h2 0.4.19, log 0.4.34, rand 0.8.8, rustls-webpki 0.103.15, syn 3.0.4, and
     others). `cargo upgrade --incompatible` proposed exactly **two**, and **both were rejected for
     the reasons already on record, re-verified against the registry this run rather than assumed**:
     - `rten 0.24 -> 0.25` — still blocked by `ocrs`, which `cargo info` confirms is **still 0.12.2**
       and still requires `rten ^0.24`. Unchanged since 2026-08-23.
     - `criterion 0.8 -> "0.7.0"` — still the **downgrade trap**. `cargo info criterion` says the real
       latest is **0.8.2**, which is what the lock already holds. cargo-edit's "latest" column is
       wrong here; never take an upgrade whose proposed version is lower than the current req.
     Also re-checked and left pinned on purpose: `boa` (crates.io latest is still **0.21.1**, which
     pins icu ~2.0 while the tree is on icu 2.2 / temporal_capi 0.2.6 — the git rev `4f98f644` stays),
     `turso =0.7.2`, `aegis =0.9.15`.
     - [ ] *(research 2026-08-26)* **Upstream may retire the `aegis` pin for us.** turso issue
       [#7660](https://github.com/tursodatabase/turso/issues/7660) asks for `aegis` and `simsimd` to
       be put behind feature flags so `turso_core` defaults to pure Rust — which is precisely the
       property the `aegis =0.9.15` pin exists to preserve (0.9.8+ mandates a clang-cl C build,
       unavailable on stock Windows MSVC). The issue is **still open** as of this run, with a linked
       PR (#7905) whose status was not readable from the issue page. Re-check on the next sweep: if a
       turso release ships default-pure-Rust `turso_core`, drop the transitive `aegis` pin entirely
       instead of carrying an exact version forward.
     GUI: `pnpm install` then `pnpm outdated` — every compatible-range package is already at latest;
     the only entries left are the five known-deferred majors (tiptap 3, typescript 7, vue-router 5,
     vue-sonner 2, and the `lucide-vue-next` -> `@lucide/vue` rename). **`lucide-vue-next@1.0.0` was
     re-confirmed as a tombstone this run, not a release**: `npm view` reports it
     `deprecated: "Package deprecated. Please use @lucide/vue instead"`, and the live package is
     `@lucide/vue@1.34.0`. `pnpm update --latest` would still silently install the dead one.
   - *(2026-08-24 sweep)* `cargo update` → 60+ compatible bumps. `cargo upgrade --incompatible` → three
     majors **applied green** — `wide 1.5 → 1.6` (`nanna-simd`, compiled unchanged), `uuid 1.24 → 1.25`
     (workspace + `nanna-server`), `playwright-rs 0.15 → 0.16` (`nanna-browser`, compiled unchanged under
     `--features playwright`) — and two **rejected**, each for a reason worth carrying forward:
     - **`rten 0.24 → 0.25` is blocked by `ocrs`, not by us.** `ocrs 0.12.2` (the latest) pins
       `rten 0.24`, and `ocr.rs` hands an `rten::Model` straight into `OcrEngineParams`, so bumping the
       direct req only puts **two incompatible `rten` copies** in the tree and the two `Model` types stop
       being the same type. Reverted to `0.24`; re-check when `ocrs` ships against `rten 0.25`.
     - **`cargo upgrade` reports `criterion` "latest 0.7.0" against our req `0.8` — that is a
       downgrade, and it is wrong.** `cargo info criterion` says `0.8.2`, and `0.8.2` is what the lock
       already resolves. Same class as the `vuedraggable`/`lucide` traps: never take an "upgrade" whose
       proposed version is *lower* than the current req without checking the registry directly.
     - **A compatible-range bump broke the build, which is the case the `--incompatible` review misses.**
       Plain `cargo update` pulled `malachite-bigint 0.10.0` for `pymath 0.2.0` while
       `rustpython-common` stayed on `0.9.2`, putting **two `BigInt` types** in `rustpython-stdlib` —
       17 `E0308`/`E0277` errors. Pinned back with
       `cargo update -p malachite-bigint@0.10.0 --precise 0.9.2`; the lockfile is the only place this
       can be held, so a future `cargo update` will re-break it until `rustpython 0.5` moves to
       malachite 0.10. **Recognise it by the symptom:** `BigInt: From<malachite_bigint::biguint::BigUint>
       is not satisfied` inside `rustpython-stdlib`.
     - [x] **The release-profile hole is now closed by the freshness gate itself** (the `[ ]` item two
           bullets below): this sweep ran `cargo build --release -p nanna-daemon` green in **21m54s**
           under the pinned `nightly-2026-08-03`, and the whole branch was re-gated the same way at the
           end of the run (**22m14s**, green). Debug build, **1593 workspace tests / 0 failures**, and
           clippy (**0 errors**, 2741 pre-existing warnings — one fewer than the 2742 baseline, from the
           `execute_call` split) also green.
     - Frontend: `pnpm outdated` showed **only the documented deferred majors** plus three safe dev
       patches, applied green — `happy-dom 20.11.6`, `vitest 4.1.11`, `vue-tsc 3.3.11`
       (**159/159 vitest**, `pnpm build` clean, 4 routes prerendered). `pnpm update --latest` was **not**
       run: it would take the `lucide-vue-next 1.0.0` tombstone and downgrade `vuedraggable`.
     - *Gotcha added:* `crates/nanna-tools/Cargo.toml` is **CRLF** while the other manifests are LF, so
       `sed -i` on it rewrites the whole file (a 128-line diff for a one-line bump). Use
       `perl -0777 -pi -e 'binmode(ARGV); binmode(ARGVOUT); …'` for that one.
   - [x] *(2026-07-24)* **Toolchain pinned in-repo: `rust-toolchain.toml` → `nightly-2026-07-13`.**
     Nightly **`89c61a754` (2026-07-23)** ICEs in `rustc_codegen_ssa` compiling **`tokio`** under our
     release profile (`lto = "fat"`, `codegen-units = 1`, `panic = "abort"`):
     `not immediate: OperandRef(Uninit @ … UnsafeCell<MaybeUninit<runtime::task::Notified<Arc<multi_thread::handle::Handle>>>>)`
     (`operand.rs:291:18`). **Debug is unaffected** — the whole workspace builds, 719 tests and clippy
     pass on that nightly — so this is **release codegen only**, which is why the run's normal
     fmt/clippy/test/build gate did not catch it. It surfaced when `pnpm build:daemon` failed while
     preparing the Tauri sidecar. Release is exactly what `cargo tauri build`, `pnpm build:daemon` and
     every benchmark number depend on, so it cannot ship.
     Bisected to the toolchain, not our code: the identical `cargo build --package nanna-daemon
     --release` succeeds under `nightly-2026-07-13` (rustc `77cf889bc`) — **exit 0, 17m34s cold**.
     Per the routine's own rule ("revert and log any bump that can't be made green"), the bump is
     reverted — but as **`rust-toolchain.toml`** rather than a `rustup default` on one machine, so the
     known-good toolchain is reproducible repo state instead of undocumented local state, and CI and
     the GUI build inherit it. The file carries the full ICE text and its removal condition.
     - [ ] **Remove the pin** once a newer nightly builds `cargo build --release` green — re-check on
           every dependency-freshness pass, and report the ICE upstream if it survives.
           *(2026-08-24 — deliberately NOT attempted, and worth saying why rather than leaving it
           looking forgotten.)* The candidate is **`1.100.0-nightly (fb6531d55, 2026-08-23)`**, up from
           the pinned `nightly-2026-08-03` (`1.99.0-nightly 11177f223`). The probe is not a cheap check:
           it needs a from-scratch debug build, the full test suite, **and** a release build under the
           new toolchain. This machine's `~/.cargo/config.toml` points every crate at one shared
           `target-dir` (`D:\Development\Cargo Target`), and throughout this run a build from a
           *different* project was holding that directory's lock — a toolchain switch would have evicted
           the workspace's cached artifacts mid-run for a probe that could not then be finished and
           verified. **Next run should do this first**, before any other cargo work, so the cache
           eviction is paid once at the start rather than stranding an increment.
           - [ ] *(2026-08-24)* **Consider pinning `CARGO_TARGET_DIR` per worktree for nightly runs.**
                 The shared target dir is a standing tax: two toolchains and several worktrees contend
                 for one lock, so builds serialise behind unrelated projects (observed this run: a
                 4-minute incremental build took 12, and a 22-minute release build was mostly waiting).
                 It is also the hazard already recorded for benchmarks — a binary in the shared
                 `release/` may have been produced by another worktree.
     - [x] **The verify gate has a hole worth closing:** `cargo build` (debug) + `cargo test` cannot see
           a release-only codegen break, so a toolchain or dependency bump can pass every green check
           and still leave the shippable artifact unbuildable. Add a `cargo build --release` (or at
           least `cargo check --profile release`) to the freshness increment's verification.
           *(2026-08-21)* Closed where it is actually enforced: the `daily-dev` skill's **step 4 —
           Verify** now requires `cargo build --release -p nanna-daemon` for any dependency, toolchain
           or `Cargo.lock` change, naming both release-only failures this repo has already hit (the
           `tokio` codegen ICE and the `turso_core` const-eval depth overflow) so a future run knows
           what the gate is for. Run and green on this run's sweep.
     - [x] *(2026-08-23)* **The verify-gate hole is closed in CI, not just in the routine's habits.**
           `cargo build` (debug) + `cargo test` cannot see a release-only codegen break, so a
           toolchain or dependency bump could pass every green check and still leave the shippable
           artifact unbuildable — which is exactly what happened twice (the 2026-07-23 `tokio` ICE in
           `rustc_codegen_ssa`, surfaced only when `pnpm build:daemon` prepared the sidecar; and
           nightly-2026-07-13's `turso_core` "queries overflow the depth limit", latent while a
           cached rlib existed). CI had **no release build on push or PR at all**: `test-compile.yml`
           is `--no-run` debug, and the only `cargo build --release` lives in `release.yml` /
           `macos-dmg.yml`, which run at tag time — long after the change merged.
           New `.github/workflows/release-check.yml` runs `cargo build --release --package
           nanna-daemon --locked` on windows-latest under the pinned nightly.
           **`cargo check --profile release` was considered and rejected**: both recorded failures
           are in codegen and const-eval, and `check` stops before codegen — it would have reported
           green for both. The gate has to be a real `build`.
           **Bounded to when the failure is possible, not to every push.** A fat-LTO
           (`codegen-units = 1`) release build is expensive, and release-codegen breaks come from the
           toolchain or the dependency graph, not from ordinary source edits the debug gates already
           cover. So it triggers on changes to `Cargo.toml` / `Cargo.lock` / `crates/*/Cargo.toml` /
           `gui/src-tauri/Cargo.toml` / `rust-toolchain.toml`, plus a weekly cron (a latent break can
           arrive with no diff — a yank, a re-resolved git dep) and `workflow_dispatch`.
           `nanna-daemon` is the target because it is what ships as the Tauri sidecar and what
           `release.yml` builds, and it pulls in both crates the recorded failures lived in.
           Also corrected while here: `test-compile.yml`'s toolchain comment still claimed the
           workspace pinned `nightly-2026-07-13` while the step passed `nightly-2026-08-03`.
   - [x] *(2026-08-23)* **`nanna-gui` was compiled by nothing in CI, and was found broken.**
     `test-compile.yml` runs `--exclude nanna-gui`, and `gui.yml` runs only frontend jobs (Vitest,
     `vue-tsc`, Playwright) *and only triggers on `gui/**` paths* — so a change under `crates/` that
     broke `gui/src-tauri/src/**` reached **no** Rust coverage anywhere, on any trigger. The
     `--exclude` was inherited by this routine's own verification too (`cargo build --all-targets
     --workspace --exclude nanna-gui`), so nothing local caught it either.
     Found the hard way: `cargo tauri build` failed with **two `E0063`s** —
     `crates/nanna-config`'s new `webhook_secret` field on `TelegramConfig`/`SignalConfig` had been
     added to three of its four construction sites, missing
     `gui/src-tauri/src/commands/channels.rs`. Every gate was green while the shippable desktop app
     did not compile.
     Fixed both sites — and not with `None`, which would have compiled while shipping a GUI that
     configures a permanently-503 channel, since the daemon now refuses to serve an unarmed webhook.
     Telegram **mints** a 122-bit secret exactly as `nanna init` does (a supplied one wins, so
     re-running never silently rotates a secret already registered via `setWebhook`); Signal only
     **carries through** what the operator supplies, because that secret must also be configured on
     the separate signal-cli-rest-api bridge — a value invented at this end would arm an endpoint the
     bridge cannot satisfy, and a 503 naming the key is the honest outcome for "half configured".
     New `check-gui` job in `test-compile.yml` closes the gap. The exclusion cited a real obstacle —
     the crate's build needs the uncommitted sidecar and `gui/.output/public` — but **both can be
     stubbed**, which is what makes the job cheap: measured locally, a 4-byte placeholder sidecar and
     a one-line `index.html` are enough for `cargo check -p nanna-gui --locked` to run the real
     typecheck in ~55s, with no `pnpm build` and no daemon build. Packaging stays `release.yml`'s job,
     with the real artifacts. **Verified it catches the actual regression**: reverting the
     `channels.rs` fix makes the job's exact command report both `E0063`s.
           *(2026-08-24)* **Now part of every freshness increment** — the 2026-08-24 sweep ran
           `cargo build --release -p nanna-daemon` as a gate and it finished green in 21m54s. It is the
           `nanna-daemon` package specifically because that is the one the pin's two known failures
           (`tokio` codegen ICE, `turso_core` const-eval depth overflow) both surface through, and it is
           what `pnpm build:daemon` ships as the Tauri sidecar.
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
   **UNBLOCKED (re-checked 2026-08-23): `physics515/Mummu` is no longer empty.** The earlier note
   ("still an empty repo — only `.git`/`.claude`, no crates") is stale. The repo now carries a real
   workspace — `crates/{mummu, mummu-serve, mummu-bench}`, `burn.toml`, `bench/BASELINE.md` — and its
   README reports the surface Nanna's items 3–5 were waiting on: one binary compiling both `Wgpu`
   (fusion + autotune, SPIR-V on Vulkan) and `burn-flex` CPU behind a cached runtime device probe;
   checked safetensors / PyTorch / GGUF import with `config.json`-driven hyperparameters; HF
   tokenizer + `tokenizer_config.json` import with byte-verified chat templates **including the
   Hermes `# Tools` block**; per-layer KV cache, on-GPU argmax, sampling, **token streaming** and
   cooperative cancellation; a validated **f16** path (Qwen2.5-1.5B ≈3.6 GiB runner VRAM, decode
   36.8 tok/s, TTFT 24.9 ms on the reference 4070 Ti SUPER); a `warm_up` API; a from-scratch
   MiniLM-class CPU sentence embedder; and `plan::pick_precision` for VRAM-aware dtype choice.
   Runner code still must NOT be written in this repo — but the *consumer glue* is now the real work,
   and it is no longer blocked:
   - [ ] **Take Mummu as a dependency** — decide git-rev pin vs path dep, and land the `[infer]`
         config surface (model id, device preference, precision override). A rev pin is the honest
         default while Mummu is pre-release: it is the same reproducibility argument as the boa git
         rev and the exact `turso`/`aegis` pins. Budget the build cost first — `burn` + `wgpu` +
         CubeCL is a large cold compile, and `nanna-gui` already needs a sidecar and a built frontend.
   - [ ] **Back the memory `embed_fn` with Mummu's MiniLM embedder** (P12 item 4) — this is the
         lowest-risk first consumer: CPU-only, no VRAM budget to negotiate, and it removes the last
         API dependency from the memory path. Mind the **embedding-dimension latch** (see the stale
         embedding-binding work): switching embedders re-binds the vector width, so the backfill and
         the dimension guard in `nanna-storage`'s SQL kNN both have to be exercised before this lands.
   - [ ] **`Provider::Local` in the router** (P12 item 5) — dispatch completion/stream/tool-calls to
         Mummu and make local the top-priority zero-cost tier. Note the router is **frozen at boot**
         (see the bare-model-name work), so a local tier that appears after boot is invisible; wire it
         into the boot-time provider map, not lazily.
   - [ ] **Do not port the parity harness, the model zoo, or the quantization planner here.** Those
         are Mummu's, with their own routine. If a bug is found through Nanna, file it there.
4. **Local embeddings in Burn** (P12) — MiniLM-class CPU embedder wired into the memory `embed_fn` → fully-local memory (no API embeddings).
5. **`Provider::Local` in the router** (P12) — dispatch completion/stream/tool-calls to `nanna-infer` and make local the top-priority (zero-cost) tier; cloud becomes opt-in escalation.
6. **Unify + upgrade dreaming** (P13) — ~~one `DreamingService` orchestrator~~ **(done 2026-07-23 — the
   daemon's scheduled cycle *and* the IPC `Consolidate` handler both dream through one shared
   `DreamingService` over one shared `ActivityClock`; the feedback + testing-effect-flush phases the
   daemon used to skip now run)**; ~~idle-gated~~ (done 2026-07-19); remaining: the multi-phase **body**
   (true merge / cluster-by-band / expand), and a local `summarize_fn` (blocked on P12/Mummu).
7. **`nanna-timeline` + compression-as-dreaming** (P13) — append-only event log in Turso + lift DSP's `simplify_with_aggressiveness`/`splimes` as the timeline compressor keyed by FSRS retrievability.
8. ~~**Fix the two path-traversal holes** (P11 security) — user-tool names + workspace file writes.~~ **(done 2026-07-06)**
9. **End-to-end daemon test** (P8) — ~~the daemon/embedded/reconnect story is still unverified~~ **mostly
   done (2026-07-16)**: a hermetic 4-test E2E suite (`crates/nanna-client/tests/e2e_daemon.rs`) now covers
   start → connect → session state → client reconnect → **daemon restart persistence**, and caught a real
   `Client::disconnect()` state bug. Still open: a real conversation turn (needs a live LLM) and the
   **embedded-fallback** path (needs a GUI build).
10. **GUI test harness foothold** (P4 follow-on) — Vitest + one critical-path Playwright smoke (chat shell load
    + Logs Copy all / Live toggle) + fixture for mocked Tauri invoke; keeps UI fixes from regressing while
    P12/P13 lead the feature queue. *(2026-07-23: IA simplification + command palette shipped; harness already green.)*

