---
name: daily-dev
description: Nanna's daily dev routine. Reads ROADMAP.md, picks the single next unimplemented item, builds it Tiger-Style with tests + benchmarks, updates the roadmap, and commits. Depth over breadth — one item per iteration. Designed to run under /loop. Use when the user says "work the roadmap", "do the next thing", "daily dev", or runs /loop over this skill.
---

# Nanna daily dev routine

The single source of truth is [`ROADMAP.md`](../../../ROADMAP.md) — a clean checklist of WHAT to build.
This skill is the routine that advances it **one item at a time**, and the home for HOW we build: the
**Engineering doctrine (Tiger Style)**, **benchmark methodology + dependency policy**, and **system
reference notes** are the appendices at the bottom of this file. Modeled on the Utter/DSP daily dev routine.

**How to run:**
- **Interactive** — `/loop /daily-dev` (continuous) or `/daily-dev` (single pass), with you driving.
- **Autonomous nightly** — the `nanna-dev-routine` scheduled task runs this same discipline unattended,
  **worktree-isolated off `origin/master`** (so the working-tree WIP is never touched), looping ≥4h and
  delivering the run as **one pull request** — it **never pushes `master`**.

## Prime directive

**One roadmap item per iteration. Depth over breadth.** Finish it fully — designed, implemented,
tested, benchmarked (if perf-affecting), roadmap updated, committed — before looking at the next.
A half-done item is worse than no item.

## Hard guardrails (read every iteration)

- **NEVER touch the user's uncommitted work-in-progress.** This repo carries a large uncommitted WIP.
  Stage and commit **only the files you changed for the current item** (`git add <explicit paths>`),
  never `git add -A`/`git add .`. Verify the staged set before every commit.
- **Never ship red.** If `cargo build`, `cargo test`, `cargo clippy --all-targets`, or a benchmark
  budget is failing, fix it or revert your change — do not commit a broken tree.
- **No performance regressions.** A perf-affecting change that regresses a budget in
  `bench/BASELINE.md` past threshold is rejected, even if it "works".
- **Stop and ask, don't guess.** If the next item is ambiguous, underspecified, or needs a product
  decision (not an engineering one), surface it to the user instead of inventing scope.
- **Don't rewrite the roadmap wholesale** — edit it surgically (tick a box, append a dated note).

## The loop

### 1 — Sync & scope
- Confirm you're on the intended branch and note the working-tree state; identify the user's WIP files
  so you never stage them.
- **Dependency freshness (once per run, its own increment):** bring all deps to the latest version that
  builds green + passes tests + holds benchmarks — `cargo update` + `cargo upgrade --incompatible`
  (cargo-edit) across the workspace, `pnpm update --latest` in `gui/`, majors included (fix the
  breakage). Commit as a standalone increment. Revert + log a `[ ]` for any bump that can't be made
  green; respect intentional pins. Full policy in **Appendix B**.
- Read `ROADMAP.md`. Pick **the single next item** in this priority order:
  1. **Immediate next actions** (top of that list first).
  2. Otherwise the highest-priority open `[ ]` in the active phases — bias to **P11 (security/correctness)**,
     then the flagship **P12 (local runner)** / **P13 (memory & dreaming)**, then the rest.
- If the chosen item is blocked, ambiguous, or product-level, **skip to the next** and note why — or
  stop and ask if nothing is safely actionable.

### 2 — Design first (Tiger Style)
- **Napkin math up front** against the scarce resources, in order: **VRAM → token/latency budget →
  RAM → CPU/GPU compute.** Write the estimate down. Being roughly right now beats profiling later.
- State the **invariants** the change must hold and the **assertions** you'll add (≥ 2 per non-trivial fn).
- If the change is **perf-affecting**, write or extend the benchmark **first** (it's a gate, not an
  afterthought) and capture the pre-change number.

### 3 — Implement, Tiger-Style
Follow the **Engineering doctrine (Appendix A)**. The always-check subset:
- **Bounded** loops/queues/caches/retries/context — explicit maxima, no unbounded growth. No unbounded recursion.
- **Assertions:** `debug_assert!` hot-path invariants, `assert!` for cheap always-on + memory/VRAM/security guards; positive **and** negative space; split compound asserts. Assertions = programmer errors; `Result`/`?` = expected failures.
- **No `.unwrap()`/`.expect()`/`panic!` on production paths.** Handle every `Result`.
- **Functions ≤ ~70 lines**, few params; push `if`s up, push `for`s down; keep leaf functions pure. Simpler return types (`()` > `bool` > `T` > `Option` > `Result`).
- **Naming:** `snake_case`, no abbreviations, units/qualifiers last (`latency_ms_max`), nouns over adjectives, related names equal length. Distinguish index/count/size; show rounding intent (`div_ceil`).
- **Batch** GPU/DB/embedding/tool work; extract hot loops into primitive-argument functions.
- **Zero tech debt** — solve it right now; don't punt.

### 4 — Verify (must be green)
```bash
cargo fmt
cargo clippy --all-targets      # pedantic + nursery — clean, no new warnings
cargo test                      # (or -p <crate> for the touched crate)
cargo build                     # release if perf-relevant
```
- If perf-affecting: run the relevant bench (`cargo bench --bench <name> -p <crate>`), compare to
  `bench/BASELINE.md`, and **reject regressions past budget**. Update the baseline if it legitimately improved.
- If it changes user-facing behavior, run the app path or a smoke test to confirm it actually works.

### 5 — Update the roadmap (surgically)
- Tick the item `[x]` and append a short dated note: `(2026-07-06) what shipped + the key number/decision`.
- If priorities shifted, adjust **Immediate next actions**. Update **Current State** only if a
  phase's status genuinely changed.

### 6 — Commit (only your change)
- `git add <explicit files you changed>` — then **verify** with `git diff --cached --name-only` that
  no WIP leaked in.
- One focused commit; descriptive message (it's the permanent `git blame` record); cite the benchmark
  artifact/number if perf-affecting. Co-author trailer per repo convention.
- **Delivery depends on mode.** *Autonomous nightly routine:* commit increments on the worktree's
  branch and open **one pull request** to `master` as the last act — **never push `master`**, never
  merge. *Interactive, you driving:* pushing to `master` is fine for this solo repo when you say so;
  otherwise open a PR.

### 7 — Loop or stop
- Under `/loop`: return to step 1 for the next single item.
- Single pass: report what shipped (item, tests, bench delta, commit) and what's now next.

## Definition of done (per item)
Designed with napkin math · implemented Tiger-Style with ≥2 assertions/fn and no prod `unwrap` ·
`fmt`/`clippy`/`test`/`build` green · benchmark holds budget (if perf-affecting) · roadmap ticked +
dated · one clean commit containing only this change · WIP untouched.

---

# Appendix A — Engineering doctrine (Tiger Style)

All new code follows **Tiger Style** (TigerBeetle's safety-and-performance doctrine), adapted to Rust
and a local-first, single-GPU async agent. Adopted *because* Nanna runs advanced work on a **small
resource budget** — correctness and performance are designed in, not bolted on.

**Safety & control flow**
- **Bound everything** — every loop, queue, cache, retry, and context window has an explicit maximum; no unbounded growth. No unbounded recursion.
- Explicit, shallow control flow: **push `if`s up, push `for`s down** (branch in parents; leaf functions branch-free and pure). Smallest scope; declare vars next to first use; positive-space conditions (`if idx < len`). Prefer explicitly-sized ints over `usize` where width is semantic.

**Assertions (Rust form)**
- **≥ 2 assertions per non-trivial function** — arguments, returns, pre/postconditions, invariants.
- `debug_assert!` for hot-path invariants; `assert!` for cheap always-on checks and memory/VRAM/security guards. Assert in pairs across paths (before-write AND after-read); positive **and** negative space; split compound (`assert!(a); assert!(b);`).
- Assertions catch **programmer** errors (panic on the impossible); `Result`/`?` handle **expected** operational failures. Never conflate.

**Errors**
- Handle every error; never silently drop a `Result`. **No `.unwrap()`/`.expect()`/`panic!` on production paths** (tests + startup invariants excepted). `thiserror` for libs, `anyhow` at the app edge. Enforce with clippy `unwrap_used`/`expect_used` in touched crates.

**Functions & simplicity**
- **≤ ~70 lines/function**; split longer. Few params; options struct when two args could transpose. Centralize control flow + state mutation in parents; keep helpers pure. Simpler return types: `()` > `bool` > `T` > `Option<T>` > `Result<T>`.
- **Zero technical debt** — solve it right at design/implementation time; never punt. Simplicity is earned over multiple passes.

**Performance (design-first — twin of the benchmark gate)**
- **Napkin math up front** against the scarce resources, in order: **VRAM → token/latency → RAM → CPU/GPU compute** — before writing code. Optimize the scarcest first (on one GPU, usually VRAM).
- **Batch** GPU dispatches, DB writes, embeddings, tool calls (recall the ~750µs wgpu dispatch floor). Large predictable chunks; no zig-zag. Extract hot loops into standalone functions over primitives (no `self`).
- Every perf-affecting change is **benchmark-gated** (Appendix B) — no regression past budget.

**Naming & off-by-one**
- `snake_case`; no abbreviations; units/qualifiers last (`latency_ms_max`, `vram_bytes_max`); nouns over adjectives; related names equal length (`source`/`target`); infuse meaning. Distinguish **index/count/size**; show rounding intent (`div_ceil`, `checked_div`).

**Tooling & dependencies**
- `cargo fmt` + `cargo clippy --all-targets` (pedantic + nursery) **clean before every commit**. Descriptive commit messages (permanent `git blame` record).
- **Dependency discipline:** justify every new dep; prefer pure-Rust, no-C where avoidable (Turso not libsqlite3, wgpu not CUDA). CI bans `rusqlite`/`libsql`/`sqlx`.
- **Dependency freshness — stay at latest** (see Appendix B).

**Deliberately NOT adopted verbatim** (Tiger Style targets a zero-alloc DB in Zig; Nanna is an async Tokio app):
- *Full static allocation* → adopt the spirit: bound + preallocate hot paths, cap growth, avoid per-token/per-event allocation churn.
- *Zero external dependencies* → impossible (Tokio, Burn, wgpu…); replaced by dependency discipline + freshness.
- Zig mechanics (`zig fmt`, 4-space, Zig scripts) → Rust equivalents (`cargo fmt`/rustfmt, 100-col).

Source: TigerBeetle `docs/TIGER_STYLE.md`.

---

# Appendix B — Performance, benchmarking & dependency freshness

**Why:** Nanna targets one consumer GPU + a small model — every feature competes for VRAM, tokens,
latency, watts. Performance is a **gate**: no change ships unless a reproducible benchmark holds/improves
the budget; every README perf claim links to an artifact.

**Governing metric — "task success @ budget":** the fraction of the **agent-eval suite** the *default
local model* completes within the reference GPU's VRAM ceiling and a p95 wall-clock target per task.
Secondary: **capability density** (task-success per GB VRAM) and **cost-of-escape-hatch** (% of tasks
forced to escalate to cloud). *A faster model that fails more tasks is not an improvement.*

**Reference hardware (name the tier in every number):** Reference GPU = RTX 4070 Ti SUPER 16 GB
(Vulkan/wgpu) + AMD Zen 4 (AVX-512); Low-VRAM guardrail = 8 GB card (forces f16 + smaller tier to still
pass); CPU-only = Zen 4 / Apple Silicon NEON (offline fallback).

**Harness:** `nanna-bench` crate (criterion + html reports, extending the `nanna-gpu` benches) + a
committed `bench/BASELINE.md` the routine diffs against. Reproducible: fixed seeds, warmup, pinned
weights, release profile for reported numbers. Runtime telemetry (`model_request_log`, `tool_call_log`,
`tool_stats_*`) feeds the same metrics live.

**Suites (each metric gets a target once a baseline exists):**
1. **Inference (`nanna-infer`):** TTFT, prefill/decode tok/s, peak VRAM, load time, GPU (wgpu) vs CPU (ndarray), f16 vs f32, across sizes×context; **correctness gate:** logit + short-sequence parity vs a reference (Candle/Ollama).
2. **Memory & vector search:** recall p50/p95, embedding tok/s, search latency at N=1k/10k/50k/100k (SIMD→GPU crossover ~50k), `bulk_load` time, RAM/100k.
3. **Dreaming & DSP compression:** dream-cycle wall-clock, memories/sec, clustering time (O(N²)→HNSW), compression ratio + reconstruction error, and **information retention** (recall accuracy before/after a dream cycle — prove dreaming shrinks footprint while holding recall).
4. **Agent loop e2e:** task-success rate, tokens/task, **tool-call validity rate**, iterations/task, wall-clock/task.
5. **Resource guardrails (hard CI-failing ceilings):** binary size, idle RAM, VRAM ceiling/tier, cold-start, tokens/turn.
6. **Efficiency:** cache-hit rate, tokens saved by routing/compression/dedup, local-vs-cloud split, $/task on cloud.

**Regression gating:** CI runs a fast subset per PR; fail if a budget regresses past threshold (e.g. >10% slower or over a VRAM ceiling). The routine updates `bench/BASELINE.md` after perf-affecting changes and cites artifacts in commits.

**Dependency freshness — stay at the latest version:** keep every dep (all `Cargo.toml`s +
`gui/package.json`) at the **latest release that builds green, passes tests, holds benchmarks** —
including **major** bumps (edit the req, fix breakage). Each run: `cargo update` + `cargo upgrade
--incompatible` (cargo-edit) / `cargo outdated`; `pnpm update --latest` / `pnpm outdated`; track the
toolchain too. A bump that can't be made green is **reverted + logged as a `[ ]`** — never shipped red,
never regressed. **Intentional pins** are the only exception, each with a one-line why (`wgpu`/`arti`
matched to `onyums`; `burn` parity-pinned; DSP's `turso`/`wgpu`). Recommended: **commit `Cargo.lock`**
(currently gitignored) for reproducible benchmarks + reviewable bumps.

**Current benchmarking state (honest):** only `nanna-gpu` has benches (`gpu_vs_simd*`,
`threshold_benchmark`; criterion) — source of the GPU-vs-SIMD reversal data. No inference/memory/agent/e2e
benchmarks, no CI gating, no eval suite, no defined budgets yet — building that harness is the first
performance work and a prerequisite for honestly claiming P12/P13 progress.

---

# Appendix C — System reference notes (facts the routine should know)

**GPU vs SIMD (load-bearing):** empirical bench (2026-02-07, Zen 4 AVX-512 + RTX 4070 Ti SUPER, wgpu,
768/1536-dim): **GPU never beats SIMD ≤ 10,000 vectors** — 100 vec 52× slower, 1,000 23×, 5,000 5.5×,
10,000 3.5×. GPU has a ~750µs fixed per-dispatch overhead (upload ~200 + dispatch ~50 + sync readback
~500µs); AVX-512 does one 768-dim cosine in ~0.1µs, linear. **`GPU_THRESHOLD` = 50,000** (below it SIMD
is strictly superior). The old "GPU wins at 512–768 dims / 10×" predictions were **wrong** — never restate as fact.

**Memory / FSRS:** five-stage lifecycle — Extraction (importance 1–5, STATED vs OBSERVED) → Storage
(embeddings + FSRS params + workspace scope) → Recall (semantic search that *also* records an FSRS
"review" — testing effect strengthens recalled memories) → Consolidation/"dreaming" (clusters merge) →
Decay. State bands: Active/Dormant/Silent/Unavailable. Dedup >0.9 similarity → update FSRS not create.
Importance is static (feeds initial difficulty); retrievability decays. Recall gating: only when message
is non-trivial (>5 words OR `?` OR >80 chars); skip a memory already in the last 4 messages. `weight()` =
retrievability × importance is the consolidation signal and the natural DSP keep-mask.

**Dreaming today:** `MemoryService::consolidate()` runs on a fixed hourly cron over an O(N²) greedy
clusterer; the richer feedback-driven `DreamingService`/`nanna-core::DreamingRuntime` is dead code
(P13 unifies them). `IngestAction::Update` falls back to create (no true merge yet). Turso stores
embeddings as f32 BLOBs and does NO vector search — cosine is entirely in RAM after `bulk_load`.

**DSP integration (P13):** lift the **pure** functions `database::compression::algorithm::simplify_with_aggressiveness`
+ `simplify_by_slope_change` + `splimes::auto_interpolate` (extrema-preserving, lossy-analytical) as the
timeline compressor; keep-rate driven by FSRS `power_law_retrievability`. Stay Turso-only: store reduced
points as BLOBs, don't adopt DSP's `SegmentStore` (it keeps measurements in `.dspseg` files outside the DB).

**Context/agent defaults:** 200k budget ≈ 10k system + 8k response + ~132k conversation; per-tool
truncation 20/80 (cmd) · 80/20 (web) · 40/40 (code); summarize threshold 10k chars; per-message hard
truncate 50KB; `max_block_chars` floored 100 (Anthropic rejects empty blocks). Iteration cap 10; model
fallback resets to top of `model_priority` each call; Ollama auto-detected by a `:tag` in the id.
ThinkingMode budgets: Instant 0 / Low 1024 / Medium 4096 / High 16384 / Maximum 32768. Sub-agent limits:
5-min timeout, 25 iterations, fresh context. Rate limits: Telegram 30@1/s, Discord 5@5/s, Slack 1@1/s,
default 10@2/s; backoff base×2^n, cap 60s, jitter 0–500ms.

**Storage:** Turso (the `turso` crate — pure-Rust, SQLite-compatible) is *already the only DB*.
"Remove SQLite" = a naming cleanup (comments/log strings/`SqliteMemoryPersistence`), NOT an engine swap;
SQL, `.db` files, `datetime('now')`, `json_*` are load-bearing.

**P9 Tor (onyums):** all Tor communication via `onyums` (arti-backed axum-over-Tor; re-exports
`arti_client`/`tor_hsservice`/`tor_hscrypto`, bundles TLS + QR + abuse-defense + v3 client-auth). Don't
pin `arti-*`/`tor-hsservice`/`qrcode` directly — take them via onyums. Identity crypto: `ed25519-dalek`
2.1, `aes-gcm` 0.10, `argon2` 0.5, `zeroize` 1. One identity per device; reject requests older than 5 min.

**Historical (do NOT resurrect as fact):** stop-button cancels **per-session** via daemon `Cancel {session_id}` (no `cancel_message`); the GPU CUDA `BatchProcessor`/`MemoryPool` API is design-intent only (shipped code is wgpu: `BatchedSearch`/`GpuVectorStore`); all tools are filesystem JS/TS skills (the `builtin/*.rs` paths are historical).
