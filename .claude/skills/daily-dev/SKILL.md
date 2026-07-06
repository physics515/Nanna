---
name: daily-dev
description: Nanna's daily dev routine. Reads ROADMAP.md, picks the single next unimplemented item, builds it Tiger-Style with tests + benchmarks, updates the roadmap, and commits. Depth over breadth — one item per iteration. Designed to run under /loop. Use when the user says "work the roadmap", "do the next thing", "daily dev", or runs /loop over this skill.
---

# Nanna daily dev routine

The single source of truth is [`ROADMAP.md`](../../../ROADMAP.md). This routine advances it **one item
at a time**, to the standard set by two governing sections in that file: **Engineering doctrine —
Tiger Style** and **Performance & Benchmarking**. Modeled on the Utter/DSP daily dev routine.

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
Follow *Engineering doctrine* in `ROADMAP.md`. The always-check subset:
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
