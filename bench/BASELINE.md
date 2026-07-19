# Benchmark baselines

The committed baseline the daily-dev routine diffs against. A perf-affecting change
ships only when the relevant number here holds or improves (see the `daily-dev` skill,
Appendix B for methodology, suites, and the reference hardware tier). Update a number
only on a legitimate, measured improvement, and cite the commit.

Reference tier (name it in every number): RTX 4070 Ti SUPER 16 GB (Vulkan/wgpu) +
AMD Zen 4 (AVX-512). Numbers without a hardware note are deterministic, hardware-independent
harness results (fixed-seed synthetic corpora), reproducible on any host.

---

## Suite 3 — Dreaming & compression (information retention)

Instrument: `nanna-memory::retention` (recall before/after a dream cycle). Deterministic,
offline, fixed-seed — these are exact, reproducible values, not timing samples. Run with
`cargo test -p nanna-memory retention`.

| Metric | Baseline | Source | Notes |
| --- | --- | --- | --- |
| Consolidation compression ratio | **0.90** | `retention::tests::dreaming_shrinks_store_while_holding_recall` | 60 → 6 memories, 6 same-topic clusters of 10 each |
| Recall retention across a dream cycle | **1.000** (recall@3 1.0 → 1.0) | same test | same-topic merges keep every topic reachable at its centroid |
| w20 aged-recall (FSRS-6 `0.0658`) | **6/6 topics** | `retention::tests::w20_experiment_aged_recall` | 800-day-aged corpus, FSRS-gated recall |
| w20 aged-recall (FSRS-5 `0.5`, the old default) | **0/6 topics** | same test | evidence the shipped constant was wrong; default flipped 2026-07-17 |

Budget: consolidation must not regress **recall retention below 1.0** on this fixed corpus,
and must hold **compression ≥ 0.90**. The w20 rows are a correctness fixture (they assert the
FSRS-6 exponent strictly out-recalls the old FSRS-5 one on aged memories), not a tunable budget.

---

## Suite 4 — Long-horizon harness (task-success @ tokens)

Instrument: `nanna-agent::harness` — the P14 control loop driven by scripted step runners
and an in-memory task source. Deterministic, offline, model-free: these rows measure the
*harness* (acceptance gating, progress-or-replan, drift containment), which is the P14
design bet — long-horizon capability comes from the harness, not the model. Run with
`cargo test -p nanna-agent harness`.

| Metric | Baseline | Source | Notes |
| --- | --- | --- | --- |
| Task success (compliant scripted model) | **3/3 items** | `harness::tests::compliant_run_success_at_tokens_baseline` | 1 step/item at 1200 tokens |
| Tokens per completed item (compliant) | **1200** | same test | 3600 total / 3 items — the governing metric's bookkeeping is exact |
| False-success completions admitted | **0** | `harness::tests::false_success_claim_is_refuted_replanned_then_abandoned` | model claims TASK COMPLETE every step; env never changes; harness never records completion |
| Drift containment cost | **≤ 6000 tokens** (5 steps) | `harness::tests::drift_containment_cost_baseline` | perma-claiming model is replanned once then abandoned — grinding is bounded |
| Loop acceleration | **< 4 steps** to abandon | `harness::tests::repeated_tool_signatures_accelerate_the_stall_counter` | identical tool signatures double the stall counter |

Budget: **false-success completions admitted must stay 0** (correctness fixture — the
anti-drift keystone), and drift containment must stay **≤ 6000 tokens** on the fixed script.
The compliant-run rows are exact bookkeeping fixtures, not tunable budgets.

### Live run — first datapoint (2026-07-18)

Instrument: `nanna-daemon/tests/live_long_horizon.rs` (`#[ignore]`d; needs Ollama). Five
minutes-scale tasks with real acceptance checks (regex-on-file ×3, command-exit-0 ×2, one
dependency edge) in a temp workspace, driven end-to-end: harness → fresh agent per step →
Ollama → real file/exec tools → harness-run verification. Run with
`NANNA_EVAL_MODEL=qwen3.5:9b cargo test -p nanna-daemon --test live_long_horizon -- --ignored --nocapture`.

| Metric | Value | Notes |
| --- | --- | --- |
| Task success | **3/5 (0.60)** | qwen3.5:9b (9.7B) via Ollama — RTX 4070 Ti SUPER 16 GB |
| Tokens per completed item | **129,093** | 375k in + 12k out; input dominates (each step's ≤8 internal iterations re-send the step context) |
| Wall clock | **252 s** (19 steps) | run ended by the runner-error circuit breaker: 3 consecutive Ollama 500s ("XML syntax error … \<parameter\> closed by \</function\>" — model-side tool-call template corruption) with task #5 in progress |
| False-success claims admitted | **0** | harness integrity held on a real model |
| Unverified completions | **0** | every task carried a machine check |
| Dependency ordering | ✅ | the depends_on pair completed in order (data file → row count) |

These are recorded datapoints, **not budgets yet** — they will move with model choice, retry
policy, and task set. The first run of this eval also immediately caught a real production bug
(scripted tools loaded without a registry handle resolved relative paths to `$HOME` instead of
the workspace — every artifact landed in the wrong directory and 0/5 verified), which is
exactly the class of evidence this suite exists to produce.

Still open: expand to a reused benchmark task set (Terminal-Bench easy-tier / SWE-bench Lite
per the P14 research note), report pass^k (k=3–5), run the 8 GB reference tier, and tune the
step-runner retry policy so a transient provider 500 does not end an otherwise-progressing run.

---

## Suites not yet baselined

The remaining suites from Appendix B have no committed baseline yet — establishing them is
open work in *Performance & Benchmarking* / P12 / P13:

- **Suite 1 — Inference** (TTFT, prefill/decode tok/s, peak VRAM, load time): belongs to the
  Mummu runner; not measured here.
- **Suite 2 — Memory & vector search** (recall p50/p95, search latency at N=1k/10k/50k/100k,
  RAM/100k). The SIMD↔GPU crossover is characterized (`nanna-gpu` benches: `GPU_THRESHOLD` = 50k;
  GPU fixed dispatch ~200µs after the wgpu 30 bump) but not yet folded into a committed budget row.
- **Suite 4 (live)** — the on-model half of the long-horizon suite (see above): task-success @
  tokens for a real local model on the 8 GB tier.
- **Suite 5 — Resource guardrails** (binary size, idle RAM, VRAM ceiling, cold-start).
- **Suite 6 — Efficiency** (cache-hit rate, tokens saved by routing/compression/dedup).
