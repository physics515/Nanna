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

## Suites not yet baselined

The remaining suites from Appendix B have no committed baseline yet — establishing them is
open work in *Performance & Benchmarking* / P12 / P13:

- **Suite 1 — Inference** (TTFT, prefill/decode tok/s, peak VRAM, load time): belongs to the
  Mummu runner; not measured here.
- **Suite 2 — Memory & vector search** (recall p50/p95, search latency at N=1k/10k/50k/100k,
  RAM/100k). The SIMD↔GPU crossover is characterized (`nanna-gpu` benches: `GPU_THRESHOLD` = 50k;
  GPU fixed dispatch ~200µs after the wgpu 30 bump) but not yet folded into a committed budget row.
- **Suite 4 — Agent loop e2e** (task-success, tokens/task, tool-call validity): needs the
  agent-eval suite (open).
- **Suite 5 — Resource guardrails** (binary size, idle RAM, VRAM ceiling, cold-start).
- **Suite 6 — Efficiency** (cache-hit rate, tokens saved by routing/compression/dedup).
