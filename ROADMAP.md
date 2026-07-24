# Nanna — Roadmap

> The single master roadmap **and status source of truth** for Nanna — there is no separate
> `STATUS.md`, `planning/`, or `docs/`. The **daily dev routine** (`.claude/skills/daily-dev`, run under
> `/loop`) reads this file, picks the **single next unimplemented item**, builds it **Tiger-Style**
> with tests + benchmarks, ticks the box, and appends a dated note. The engineering doctrine, benchmark
> methodology, dependency policy, and system reference notes live in that skill — this file stays a
> clean checklist. Shipped capability is *described* in [`README.md`](README.md); here it is only
> tracked. Edit surgically; never rewrite wholesale.

**Last updated:** 2026-07-23 (**P13 dreaming unification** — both daemon paths now dream through one
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

## Performance & Benchmarking

Performance is a **gate**, not a phase (small single-GPU budget): a change ships only when a benchmark
holds or improves the budget, and README perf claims link to an artifact. Governing metric: **task
success @ budget** — the fraction of the agent-eval suite the local model solves within the reference
GPU's VRAM ceiling and a p95 latency target (reference: RTX 4070 Ti SUPER 16 GB). *Methodology, the six
benchmark suites, and per-tier budgets live in the `daily-dev` skill.* Build-out:

- [ ] `nanna-bench` crate (criterion) — unify the existing `nanna-gpu` benches
- [ ] Define the **agent-eval suite** (the task-success denominator)
- [~] Per-tier budgets in `bench/BASELINE.md` (VRAM ceilings, min decode tok/s, max TTFT, max dream-cycle time)
      *(2026-07-17)* **`bench/BASELINE.md` created** — the committed diff-target the routine was missing.
      First rows seeded from the Suite 3 (dreaming/compression) retention harness: consolidation
      compression 0.90 @ recall retention 1.000, plus the w20 aged-recall correctness fixture (6/6 vs 0/6).
      Other suites (inference/vector-search/agent-loop/guardrails/efficiency) are listed as not-yet-baselined.
- [ ] CI gate — fail a PR that regresses a budget past threshold
- [~] Inference **parity** harness (logit/sequence vs reference); memory **retention** harness (recall before/after a dream cycle)
      *(2026-07-17)* **Memory retention harness shipped** (`nanna-memory::retention`) — the instrument the FSRS
      `w20` fix (P13) is gated on. Measures **topic recall@k** (fraction of probe queries whose raw top-`k`
      vector neighbours still include a same-topic memory) once before and once after a real `consolidate()`
      dream cycle, and reports compression alongside `recall_retention` (after/before). Deterministic + offline:
      a `RetentionCorpus` fabricates topic clusters from a `SplitMix64` seed with per-topic **era + salience +
      access** separation (so the composite clusterer keeps topics apart instead of merging everything — the
      non-similarity signals otherwise dominate the fixed clustering weights and cross-cluster). Replay the same
      corpus under two `FsrsParameters` to compare `recall_retention` — that is the w20 experiment. Added thin
      `MemoryService::{add_entry, search_by_embedding}` accessors (controlled vectors/aged FSRS + raw top-k,
      bypassing the recall gating). Demonstration run: **60 → 6 memories (0.90 compression) with recall
      1.000 → 1.000** (each 10-memory topic collapsed to one, recall perfectly held). 5 unit tests
      (determinism, tag-parse, ratio-math edge cases incl. empty/appeared, fresh-corpus recall, shrink-while-
      holding-recall); 51 nanna-memory tests green. Inference parity harness still open (belongs to Mummu).
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
- [x] Commit LICENSE file (MIT) — appears absent despite README reference.
      *(2026-07-23)* Added. Both `Cargo.toml` manifests already declared `license = "MIT"` and the
      README claimed MIT, but the file itself was missing — so every published crate asserted a licence
      with no text behind it. Copyright line reads `2026 physics515` (the repo owner); change it if you
      want a legal name there.
- [ ] Add CONTRIBUTING.md and CODE_OF_CONDUCT.md.
- [x] Fix Cargo.toml repository URL from clawdbot/nanna to physics515/Nanna.
      *(2026-07-23)* Fixed in both the root package and `[workspace.package]`.
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
- [x]  Wire GUI automated tests into CI (see P4 follow-on GUI Testing & UX Quality): unit/component on every PR; Playwright + Tauri/WebDriver smoke on packaging jobs. *(2026-07-22 — `.github/workflows/gui.yml`)*
- [ ]  Add Dependabot/Renovate config.
- [ ]  Resolve deferred clippy warnings (too_many_lines, etc.) — enforce -D warnings in CI.
- [ ]  Begin decomposing giant files: loop_runner.rs (~132KB), nanna-llm/src/lib.rs (~159KB), gui/src-tauri/src/lib.rs (8k+ lines) — not all required for 0.1 but plan the split.
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
- [ ] Onboarding writes API keys to plaintext config.toml (src/onboarding.rs), even though a SecureStore using OS keyring exists. The OS keychain should be the default path; TOML config should store only non-secret settings.
- [ ] SecureStore file fallback is plaintext JSON (mode 0600), not encrypted — the module comment misleadingly says "encrypted file storage." Fix the comment or implement real AES-GCM encryption with an OS-protected key.
- [ ] Inconsistent application directory namespaces — config uses ProjectDirs::from("bot", "clawd", "Nanna") while credentials use ProjectDirs::from("com", "nanna", "nanna"), causing orphaned data and confused uninstall flows.
- [ ] Onboarding has_api_key only checks config.llm.api_key or ANTHROPIC_API_KEY, ignoring OpenAI/OpenRouter keys. quick_setup specifically asks for an Anthropic key despite multi-provider support — broken first-run for non-Anthropic users.
- [ ] Tauri CSP is set to null in gui/src-tauri/tauri.conf.json — not acceptable for a desktop app rendering model output and markdown.
- [ ] Tauri Devtools enabled by default in production features (gui/src-tauri/Cargo.toml) — should be removed from default features.
- [ ] Tauri shell permissions (allow-open/spawn/kill/execute) for the daemon sidecar need least-privilege review.
- [~] ROADMAP explicitly lists open items: ~~disabled tools still execute~~ **(done 2026-07-20 — `ToolPolicy` gate, P6)**, ~~deleted tools remain callable until restart~~ **(done 2026-07-17 — `unregister` wiring)**, ~~delete_skill needs hardening against remove_dir_all/symlink races~~ **(done — symlink + canonical-escape guards in `commands/tools.rs`)**, stronger sandboxing needed *(open — OS-level sandbox under the policy layer; see research note below)*.
- [x] HTTP server defaults to 0.0.0.0:3000 (src/main.rs) — potential footgun if exposed without auth.
      *(2026-07-23)* Fixed together with the webhook receiver — see the "Bind local services to localhost
      by default" item below.
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
- [x] Fix tool lifecycle bugs: disabled tools must not execute; deleted tools must not remain callable until restart (ROADMAP P6/P11).
      *(2026-07-20)* Disabled-tools-execute closed by the `ToolPolicy` gate above (`[tools] disabled` now
      denies at `execute()`, post-resolution). Deleted-tools-callable was closed 2026-07-17 via
      `ToolRegistry::unregister` wiring (see the P11 tool-manager-consistency note).
- [ ] Harden delete_skill against remove_dir_all/symlink races.
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
- [ ] Supervisor health check runs a placeholder, not a real agent loop (`supervisor.rs:496`).
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
- [ ] *(2026-07-23)* **`<UiSonnerSonner />` fails to resolve at runtime — toasts may never render.**
      Every Playwright page load logs `[Vue warn]: Failed to resolve component: UiSonnerSonner` from
      `app.vue`, on **both** this branch and pristine `origin/master`, so it is pre-existing and not a
      dep-bump fallout. The component *does* exist (`app/components/ui/sonner/Sonner.vue`) and the
      auto-import name looks correct for its nested path, so the likely cause is the component failing to
      load rather than being misnamed — e.g. the `vue-sonner` import throwing. Worth chasing because the
      failure is silent and the blast radius is real: `useToast` drives success/error feedback for copy,
      save, delete and clear across the app (P4 "Toasts & destructive confirms"), so if the toaster never
      mounts, none of that feedback reaches the user. Check whether `useToast` renders through this
      component or an independent path, then fix or delete it. Cross-check against the deferred
      `vue-sonner 1 → 2` major.
- [ ] *(2026-07-23)* **`critical-path.spec.ts` "session create / rename / delete / switch" is flaky —
      pre-existing, confirmed against pristine `origin/master`** (where that file fails **3** tests; on
      the current branch it fails 1). Diagnosis from the trace: the step's locator
      `getByRole('button', {name: /delete|confirm|yes/i})` is **ambiguous** — it matches both the context
      menu's `Delete` item and the `ConfirmDialog`'s `Confirm` button, so after the menu detaches
      Playwright re-resolves onto `Confirm` and then spins on
      `confirm-overlay … intercepts pointer events` until the 60 s timeout. Fix is test-side: scope the
      confirmation click to the dialog rather than the page, and wait for the overlay's transition to
      settle. Deliberately **not** patched under time pressure in the run that found it — loosening an
      assertion to get green is how a real regression gets hidden later.
      *(2026-07-23)* Simplification pass closed most open carry-overs (palette, virtualization, IA nav,
      Advanced settings). Remaining bash items: channel-wizard bulk validation, formal viewport pass,
      channels toast ref, legacy clawd config-path copy.
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
- [ ] *(discovered 2026-07-23)* **STATED vs OBSERVED provenance is not actually recorded — the
      `fact_type` metadata key is read but never written.** Chasing the drift mitigation "verbatim-pin
      user-stated memories" turned up that the distinction the reference notes describe under extraction
      ("importance 1–5, STATED vs OBSERVED") does not exist in the code: `fact_type` appears only in
      `nanna-daemon/src/control/memory.rs`, twice, both times *reading* it with
      `.unwrap_or("stated")` for display — so **every memory reports itself as user-stated regardless of
      origin**, and `ExtractedMemory` (`loop_runner.rs`) carries only `content`/`category`/`tags` with no
      provenance field at all. Consequences: the GUI's fact-type column is decorative; the survey's
      "source attribution (user statement > agent inference)" precedence rule has nothing to run on; and
      the drift mitigation that would pin user-stated memories verbatim is **blocked** until provenance
      is genuinely captured. Fix = add a provenance field to `ExtractedMemory`, have the extraction
      prompt classify it, persist it into memory metadata, and only then build the pin on top. Note the
      safe default when it lands: **absent provenance must NOT be treated as "stated"** (absence of
      evidence isn't evidence of a user statement) — the current display default has it backwards.
- [x] **Harden `create_consolidated_entry` against NaN** — the FSRS-scalar merge used
      `max_by(|a,b| a.partial_cmp(b).unwrap())`, which **panics the dreaming cycle** if any stored
      `importance`/`storage_strength` is NaN.
      *(2026-07-09)* Replaced with a pure `max_finite_or(values, default)` that skips non-finite inputs
      (NaN/±inf) and falls back to the default when none are finite; added pre/postcondition assertions
      (non-empty cluster in, finite scalars out). 3 unit tests (NaN/inf skipped, max+sum semantics,
      NaN-cluster survives). Removes two prod-path `unwrap`s from the consolidation path.
- [ ] **Indexed clustering** — replace the O(N²) greedy single-pass `cluster_memories()` with HNSW/IVF candidate neighbors + connected-components/HDBSCAN over `composite_cluster_score`; scales past the ~50k in-RAM ceiling.
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
- [ ] *(research 2026-07-23)* **Summarization drift is the named failure mode of exactly what dreaming does —
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
- [ ] **Git context injection** — inject `git status --short --branch` + recent commits at run start when the
      workspace is a repo (P17 injects only README/AGENTS/CONTRIBUTING/ROADMAP). Prevents destructive edits
      and redundant discovery calls.
- [ ] **@-file mentions + drag-drop injection** — chat attachments are image-only. Deterministic client-side
      file injection beats a read_file roundtrip the model may fumble; pairs with the P4 drag-drop item.
- [ ] **"think hard" phrases** — map natural-language budget phrases onto the existing ThinkingMode ladder;
      chat-first users on Telegram can't flip config flags mid-message.
- [ ] **Per-session model override** — "use the big model for this conversation"; SpawnSubSession already
      carries `model: Option<String>`, the Chat message doesn't.
- [ ] **Typed sub-agents with tool scoping** — the chat task tool spawns with all_tools_active:true and no
      restriction surface, while the P14 harness already does per-step tool_scope. Port scoped spawn to chat
      (a research sub-agent that cannot exec is a safety win, and small models degrade past ~10 tools).
- [ ] **Cross-cutting tool-call hooks** — a scripted interceptor point in ToolRegistry::execute (audit-log
      every call, guard-check before any exec). Per-tool logic is already user-editable in each skill; this
      covers the cross-tool niche only.
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
     - [ ] *(2026-07-23)* **`nuxt 4.4.8 → 4.5.0` is a build-layer major in a minor's clothing** — 4.5
           moves the Vite builder to **Vite 8** (Rolldown internals), the Rspack builder to **Rspack 2 on
           `@rsbuild/core`**, and switches Nuxt's own build to `tsdown` (plus unhead v3 / unctx v3). Treat
           it as a migration item, not a sweep bump: needs a full `pnpm build` + `pnpm tauri dev` boot +
           `cargo tauri build` + WebDriver pass. Also note **Nuxt 3 EOL 2026-07-31** (we are on 4.x, so
           informational). Source: [Nuxt 4.5](https://nuxt.com/blog/v4-5).
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
   **Blocked here by design (checked 2026-07-23): `physics515/Mummu` is still an empty repo** — only
   `.git`/`.claude`, no crates — so there is no runner surface for Nanna to consume. Items 3–5 stay
   blocked until Mummu exposes one; runner code must NOT be written in this repo.
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

