# Nanna v0.3.9-beta.18 — The Embedding Gets Out of the Way

## What's New

### Memory: tool-result ingestion is off the turn's critical path (this release)

Every chunk of every tool result used to be embedded **inline** — one round-trip to the same local
model server that serves generation, plus a vector search, plus an insert, awaited before the agent
could take its next step. One measured run made **zero model decisions for 189 of its 246 minutes**.

A chunk is now persisted synchronously and only its *vector* is queued
(`MemoryService::remember_deferred_vector`). The row is durable before the call returns and keeps its
`source_id`, so a `recall(...)` handle handed to the model in the same turn still resolves — only
similarity search waits for the drain.

The hard part was the drain. `drain_backfill` could not do it: when the embedder is the local
provider it first waits for *no harness run to be live*, so a row queued **by** a live run would wait
for the run that queued it — hours, during a long mission. `drain_queued_vectors` skips that gate and
pays for the exception with a bound: it may only embed rows **this process parked**, it still repays
every request with an equal idle window, and it still takes the process-wide drain lock. Same
embedding work, half the duty cycle, concurrent with the turn instead of blocking it.

**One latent bug fixed on the way:** `drain_backfill` parked on `wait_idle()` *while holding* the
process-wide drain lock, so a drain waiting for a mission to end starved every drain behind it for
the length of that mission.

**Trade, stated plainly:** the neighbour-dedup search is skipped for deferred rows (there is no query
vector to search with), so near-identical tool results land as separate rows instead of folding.
Squeezing the store is dreaming's job, and run-length collapse upstream already removed the case that
made this expensive.

### Memory: queued vectors no longer wait for a restart (this release)

The companion to the change above. `drain_queued_vectors` drains what the current
process deliberately deferred, at foreground priority — and it is deliberately budgeted **not** to
sweep an inherited backlog, leaving that to `drain_backfill`. The gap was that nothing actually
called `drain_backfill` at that priority during a session: its only triggers are *binding* events
(daemon start, provider switch, width reprobe), and an ordinary session has none of them. A row
parked by a **transient** embedding failure therefore stayed unsearchable until the next restart —
recovered, but a session late rather than a moment late.

`supervise_idle_backfill` closes that: one task for the daemon's life that waits for a turn to start
and then finish, and drains at the first moment no run is live. It adds a *trigger*, never a second
request rate — same process-wide drain lock, same chat-priority gate, same per-request repayment
window. Its probe is one in-memory scan that short-circuits on the first unembedded row plus two
`LIMIT 1` queries, once per turn.

### Toolchain and dependencies (this release)

Pinned nightly moved `2026-08-03` → `2026-08-25`, verified by a full release build before the pin
moved. The new nightly raises `recursion_depth_exceeding_limit` ([rust#159228](https://github.com/rust-lang/rust/issues/159228)),
scheduled to become a **hard error**, while proving futures are `Send` through wgpu's type graph —
answered at **seven** crate roots (six libraries plus an integration-test crate root, which does not
inherit the library attributes).

`malachite-bigint` is now pinned in the *manifest* rather than the lockfile. It had been hand-repaired
on four consecutive dependency sweeps, because a lockfile pin is exactly what `cargo update` is
entitled to move; a manifest constraint is not.

### New: point Nanna at a config it does not own (this release)

`NANNA_CONFIG_PATH=<file>` overrides config resolution. Previously `--data-dir` isolated the database
but **not** the settings, and the settings path resolves through the platform's known-folder API, so
even `%APPDATA%` could not redirect it — meaning any second instance, or any test of a
configuration-dependent startup path, had to edit the config you actually use. Pair it with
`--data-dir` for a fully isolated instance.

### Frontend: four major dependency migrations (this release)

`@tiptap/* 2 → 3`, `vue-router 4 → 5`, `vue-sonner 1 → 2`, and `lucide-vue-next → @lucide/vue 1.34`.

The lucide one is a **package rename, not a version bump**, and it is a trap worth naming:
`lucide-vue-next@1.0.0` is deprecated but is *not* an empty tombstone — it ships a real dist where
every icon resolves. So `pnpm update --latest` installs a working-but-dead package and nothing fails.
The failure mode is silence. A new `packageComponentExports` guard now resolves all **221** icon
bindings the templates render, so the rename is proven at the export level rather than assumed.

### Release engineering: `release.yml` can actually produce a release (this release)

The workflow had never completed a successful dispatch, and several steps could not have worked as
written: three jobs installed `stable` while `rust-toolchain.toml` pins a nightly that rustup uses
anyway; macOS requested `universal-apple-darwin`, which is not a rustup target; all three ran
`cargo tauri build` with the CLI never installed and no Node or pnpm present at all; **no signing key
was wired in anywhere**, though `createUpdaterArtifacts` requires one; and the publish job set `TAG`
in one shell and used it in three others, so every upload targeted the tag `""`.

All fixed. Windows builds by default; macOS and Linux are opt-in inputs defaulting to false, because
neither has ever been verified on a runner and defaulting them on would let an unverified platform
take the Windows release down with it. The build now **fails loudly if an installer is produced
without a signature**.


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
      — *repaired across beta.17/18.* Every blocker named in beta.16's note is fixed, and a real
      dispatch has now **built and signed** a Windows installer on a runner: the toolchain pin is
      honoured, Node/pnpm and the Tauri CLI are actually installed, and the signing secrets work
      (the collect step fails when the `.sig` is missing, and it passed). That first attempt still
      did not publish — a `cache: pnpm` **post-job** step failed after the build succeeded, and a
      failed post step fails the job — so the cache was removed. Box stays `[~]` until a dispatch
      has published end to end.
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
- `release.yml` now builds and signs on a runner, but has not yet completed a publish end to end;
  macOS and Linux remain opt-in and unverified, so those artifacts are still not published
- beta.17 was tagged for release but never published (see above); beta.18 supersedes it and carries
  the same scope plus the 2026-08-26 nightly
- Burn local runner still in development (in the `Mummu` repo)

## Installation

1. Install [Ollama](https://ollama.com) and pull `qwen3.5:9b`
2. Download installer from [Releases](https://github.com/physics515/Nanna/releases)
3. Run installer (expect SmartScreen warning → "Run anyway")
