# Nanna v0.3.16-beta.25 — What Dreaming Was Actually Merging

Nanna's memory consolidates: it clusters related memories and folds them into one. This release fixes
a defect in how "related" was decided — for the commonest pair of memories in a real store, it was not
being decided at all.

## What's Fixed

### Cosine similarity could not veto a merge

`composite_cluster_score` blends four signals: semantic similarity, recall affinity, importance
proximity, and age proximity. Three of those are **maximal by construction** for the most ordinary
pair a store contains — two memories that are equally unimportant, that have both never been recalled,
and that were written in the same session. `recall_affinity` and `importance_proximity` return 1.0
whenever the two values are *equal*, including `0 == 0`; `age_prox` sits near 1.0 within a session.

With the shipped weights that put a floor of **0.50** under every score, against a clustering threshold
of **0.45**. The bar was cleared before similarity was consulted, so no embedding could prevent a
merge. Measured, not inferred:

```
two orthogonal unit vectors        score 0.500  -> clustered
two anti-correlated vectors        score 0.500  -> clustered
four mutually unrelated memories   -> ONE cluster of four
```

A dream cycle would summarize those four into a single gist and record it as a merge. In effect the
clusterer was grouping by *"written around the same time and equally unremarkable"*.

The fix is the default, not the algorithm. The threshold is judged against the composite score, not
against raw cosine, and the drift fixture's own 0.65 threshold always satisfied the invariant — it was
the shipped default that demanded a cosine of **0.000**.

### The bar is now 0.50, and the range up to it was free

A new sweep (`cargo bench -p nanna-memory --bench clustering_threshold_sweep`) prices the trade:

| threshold | cosine actually demanded | clusters | compression | recall |
| --- | --- | --- | --- | --- |
| 0.55 | 0.10 | 3 | 0.450 | 1.000 |
| 0.65 | 0.30 | 3 | 0.450 | 1.000 |
| **0.75** (shipped) | **0.50** | 3 | 0.450 | 1.000 |
| 0.85 | 0.70 | 5 | 0.383 | 1.000 |

Those first three rows are *identical* in every outcome while the cosine demanded rises five-fold, so
the default takes the top of the flat range: a five-times stricter semantic bar at zero measured cost.
Recall is 1.000 throughout — this trades compression against merge *precision*, never retrievability.

Consolidation now **refuses to run** under a configuration where similarity has no veto, rather than
warning about it: consolidation rewrites memories, and a warning arrives after the merge has already
happened.

## What's New

### `nanna doctor`

```bash
nanna doctor
```

Six configuration checks, each of which prints **the fix** rather than only the verdict — a missing
tools directory, an `[infer]` section naming no model, a clustering configuration that would merge
unrelated memories. Exits non-zero on a real fault, so it works from a script or a health probe.

Deliberately offline: no provider call, no network probe, no keyring read. A clean report means your
*configuration* is sound, not that a provider is reachable.

Writing it surfaced its own finding: `[server].host` is read by nothing. The bind address comes from
the `--host` flag (default loopback), so a user setting that field to `127.0.0.1` has secured nothing,
and the shipped `0.0.0.0` reads as exposed while binding nothing of the sort. The doctor now reports
the effective answer instead of raising a false alarm about it.

### Local-inference configuration (`[infer]`)

The config surface for the on-device runner — model, embedding model, device, precision, VRAM budget —
plus the boot-time decision the router will read. Inert by default.

`InferPrecision::Auto` cannot choose f16 on Linux, and the code says so out loud rather than quietly
serving f32: the precision planner needs a VRAM budget, and that number is only available through a
Windows-specific path today. The remedy (`[infer].precision = "f16"`) is named in the message.

## Also In This Release

- **Toolchain** moved to `nightly-2026-09-08`, release-verified.
- **Dependencies** swept to latest — `playwright-rs 0.18`, `reqwest 0.13.5`, `tantivy 0.26.2` and
  seven more. The `libc` and `malachite-bigint` ceilings still stand and are still enforced by tests.
- **One definition of the daemon IPC port.** Two call sites still hardcoded `ws://127.0.0.1:5149`
  beside the constant meant to prevent exactly that; a guard test now fails on any Rust source that
  spells the port into a URL.
- **A flaky test fixed** — the log-capture tests failed intermittently under a wide parallel run
  because tracing's process-wide level hint drops between subscriber installs.
- **Two new benchmarks**, `clustering_scaling` and `clustering_threshold_sweep`, with baselines
  recorded in `bench/BASELINE.md`.

## Known Issues

- **The Tauri GUI does not build on Linux.** `tauri-build 2.6.3` infers its target directory assuming
  cargo's classic build-script layout; current cargo emits a nested one, and the sidecar destination
  resolves onto a directory. Pre-existing and upstream, not caused by this release — but it means this
  build is **unverified on Linux desktop**, and Windows builds are unaffected.
- **`[server].host` is still a dead field.** Reported honestly by `nanna doctor`; deliberately not
  "fixed" by wiring it, since doing so with its current `0.0.0.0` default would turn an inert field
  into a real exposure of an unauthenticated HTTP surface.
