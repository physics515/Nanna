# Benchmark baselines

The committed baseline the daily-dev routine diffs against. A perf-affecting change
ships only when the relevant number here holds or improves (see the `daily-dev` skill,
Appendix B for methodology, suites, and the reference hardware tier). Update a number
only on a legitimate, measured improvement, and cite the commit.

**Governing metric — task success @ budget.** The agent-eval suite denominator (which
live/harness evals count, how pass rate is scored, reference tiers) is defined in
[`AGENT_EVAL.md`](./AGENT_EVAL.md). Machine-readable budget rows: [`budgets.toml`](./budgets.toml).

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
| Consolidation compression ratio | **0.90** | `retention::tests::dreaming_shrinks_store_while_holding_recall` | 60 → 6 memories |
| Recall retention across a dream cycle | **1.000** (recall@3 1.0 → 1.0) | same test | same-topic merges keep every topic reachable at its centroid |
| **Summarizer calls per cycle** (this corpus) | **0** (was 6) | same test | *(2026-07-23)* dream phase (b) folds the 54 removable memories deterministically — `memories_deduped: 54`, `clusters_formed: 0` |
| w20 aged-recall (FSRS-6 `0.0658`) | **6/6 topics** | `retention::tests::w20_experiment_aged_recall` | 800-day-aged corpus, FSRS-gated recall |
| w20 aged-recall (FSRS-5 `0.5`, the old default) | **0/6 topics** | same test | evidence the shipped constant was wrong; default flipped 2026-07-17 |

Budget: consolidation must not regress **recall retention below 1.0** on this fixed corpus,
and must hold **compression ≥ 0.90**. The w20 rows are a correctness fixture (they assert the
FSRS-6 exponent strictly out-recalls the old FSRS-5 one on aged memories), not a tunable budget.

### Summarization drift (content fidelity, not recall)

Instrument: `nanna-memory::retention::clause_survives` + the two `drift` fixtures. Measures
whether a **rare, safety-critical clause** survives a dream cycle — deliberately *not* a recall
metric, because the two come apart exactly here: the topic stays perfectly recallable while the
one clause that made it actionable is gone. That gap is what makes drift easy to ship blind.

| Metric | Baseline | Source | Notes |
| --- | --- | --- | --- |
| Rare clause survives a **summarized** cluster | **NO** (lost in 1 cycle) | `retention::tests::summarized_memories_lose_a_rare_critical_clause_in_one_cycle` | the known exposure, reproduced against our own pipeline |
| Rare clause survives a **deduplicated** band | **YES** (verbatim) | `retention::tests::deduplicated_memories_keep_their_rare_critical_clause` | dream phase (b) folds without paraphrasing; store still compresses |

Budget: the **dedup arm must never regress to NO** — that is a correctness fixture, and it is the
concrete payoff of folding restatements deterministically. The summarized arm is a **baseline to
beat, not a budget to hold**: it currently asserts the clause *is* lost, so when a drift mitigation
lands (generation ceiling, or verbatim-pinning user-STATED memories — both logged in P13) that test
will fail loudly and should be flipped to assert survival. A failing baseline arm means the system
got better, and the assertion says so in its message.

*(2026-07-23)* The summarizer-call row is the newest budget and the point of dream phase (b):
this corpus's within-topic pairs sit in the `IngestAction::Reinforce` band (cosine > 0.92), so
they are true restatements and are now folded **deterministically, with zero LLM calls**, where
the cycle previously paid 6 summarizer prompts to paraphrase them. Compression and recall are
*unchanged* (0.90 / 1.000) — the win is entirely in the token budget, the governing scarce
resource on the single-GPU local tier. Do not "fix" a future regression here by lowering the
similarity threshold: the 0.92 line is the project's existing same-fact definition, and folding
below it would paraphrase-merge genuinely distinct memories.

---

## Suite 4 — Long-horizon harness (task-success @ tokens)

Denominator (which evals count, tiers, pass-rate rules): [`AGENT_EVAL.md`](./AGENT_EVAL.md).
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

### Live run (2026-07-18, after harness tuning)

Instrument: `nanna-daemon/tests/live_long_horizon.rs` (`#[ignore]`d; needs Ollama). Five
minutes-scale tasks with real acceptance checks (regex-on-file ×3, command-exit-0 ×2, one
dependency edge) in a temp workspace, driven end-to-end: harness → fresh agent per step →
Ollama → real file/exec tools → harness-run verification. Run with
`NANNA_EVAL_MODEL=qwen3.5:9b cargo test -p nanna-daemon --test live_long_horizon -- --ignored --nocapture`.

| Metric | Value | Notes |
| --- | --- | --- |
| Task success | **5/5 (1.00)** | qwen3.5:9b (9.7B) via Ollama — RTX 4070 Ti SUPER 16 GB |
| Tokens per completed item | **22,564** | 109k in + 3.4k out over 6 steps |
| Wall clock | **72 s** (6 steps) | run ended `all_tasks_done` |
| Replans / abandonments | **0 / 0** | verdict feedback worked: task #4's first check failed, the fed-back verdict fixed it next step |
| False-success claims admitted | **0** | harness integrity held on a real model |
| Unverified completions | **0** | every task carried a machine check |
| Dependency ordering | ✅ | the depends_on pair completed in order (data file → row count) |

These are recorded datapoints, **not budgets yet** — they will move with model choice and task
set. The tuning trail is itself the evidence this suite exists to produce — each run caught a
real harness/production bug:

- **Run 1 — 0/5.** The model did every task correctly; every artifact landed in `$HOME`.
  Production bug: scripted tools loaded via `load_skills_with_services` never got the registry
  handle, so relative paths silently resolved to the home directory. Fixed.
- **Run 2 — 3/5 @ 129k tokens/item.** Both command-based checks were unwinnable: no bare `sh`
  on PATH ⇒ the acceptance runner silently fell back to `cmd.exe`, which cannot run
  `test`/`$(...)`. Also 3 consecutive Ollama 500s (model-side tool-call template corruption)
  tripped the error breaker mid-task. Fixed: acceptance commands route through Git Bash like
  the exec tool (regression-tested), and the step runner retries transient 5xx with a fresh
  re-anchored context.
- **Run 3 — 5/5 @ 22.6k tokens/item, 72 s.** Above.

### Endurance run — the "4-hour task" (2026-07-19)

Instrument: `live_endurance` in the same test file — build `minidb` (a POSIX-shell key-value
store CLI) against 42 dependency-chained fail-to-pass feature tests. qwen3.5:9b,
RTX 4070 Ti SUPER 16 GB, full healing stack (bash-routed acceptance, abort-as-error parsing,
5xx/empty retries with runner reset, poison containment, gated server-restart healing, no
per-step token cap).

| Metric | Value | Notes |
| --- | --- | --- |
| Wall clock, one plan | **6.00 h** (cap) | single seeded plan, worked continuously start to finish |
| Longest unbroken segment | **4 h 39 m** | one provider incident at t=81m, healed by server restart; segment 2 alone clears the 4-hour bar |
| Verified completions | **23** (14/42 seeded features + 9 model-created subtasks) | progress distributed across the entire window (t=2m → t=360m, still advancing at hour six) |
| Tokens | **5.13 M** over 137 steps | ~854k tokens/hour sustained on the local GPU |
| False-success claims admitted | **0** | across all six hours |
| Drift | none observed | at hour six the model was decomposing and fixing the append feature — on-plan work, not looping or wandering |

The tuning trail to get here (each run caught a real bug): run 1 — tool workdir plumbing
(`$HOME` writes); run 2 — cmd.exe acceptance fallback + Ollama tool-template 500s; runs 3–4 —
Ollama's degraded-runner state (aborted `done:false` generations parsed as empty successes —
fixed in nanna-llm) and item-level poison containment; run 5 — subtask `sort_order 0`
queue-jumping (task explosion); run 7 — the result above.

**Cloud variant — `openrouter/openrouter/free` (2026-07-20).** The same 42-feature ladder
driven through OpenRouter's free-model auto-router, where the serving model varies per
request — the harness must carry ALL continuity. Result: **33/42 features verified in 3.30 h,
one unbroken segment, 0 resumes, 0 false successes**, 97 steps, 3.36 M tokens
(~102 k/verified item), stop = `all_tasks_done` (plan drained: 33 verified + 12
abandoned-with-containment). Abandonments clustered where weak models happened to be serving
(even trivial features), while stronger draws later carried the ladder — per-request model
variance handled by design. Smoke on the same router: 5/5 @ 17 k tokens/item. Healing is
provider-aware: cloud incidents heal by pause+resume+retries only (local-server surgery is
gated to Ollama-served models via `ProviderId::from_model`).

Still open: throughput on the local tier (14/42 primary features in 6 h — the middle-ladder
grind dominates), a reused benchmark task set (Terminal-Bench easy-tier / SWE-bench Lite),
pass^k on the endurance suite, and the 8 GB reference tier.

---

## Suite 1 — Inference (not yet baselined)

Instrument: Mummu / `nanna-infer` (P12). Local-model decode on the reference tier
(RTX 4070 Ti SUPER 16 GB, Vulkan/wgpu) and the low-VRAM / CPU-only guardrails.

| Metric | Baseline | Source | Notes |
| --- | --- | --- | --- |
| TTFT (p95) | *not yet baselined* | — | time-to-first-token |
| Prefill tok/s | *not yet baselined* | — | |
| Decode tok/s | *not yet baselined* | — | min decode rate at the reference tier |
| Peak VRAM | *not yet baselined* | — | must fit the 16 GB ceiling (8 GB guardrail tier separate) |
| Model load time | *not yet baselined* | — | cold load from cache |

Blocked on the Mummu runner surface. Machine-readable rows land in `bench/budgets.toml`
under `suite = "inference"` once measured.

---

## Suite 2 — Vector search (SIMD default path)

Instrument: `nanna-bench` criterion body `benches/vector_search.rs` (unifies the ad-hoc
`nanna-gpu` SIMD half onto the harness) + `nanna_simd::cosine_similarity_f32`. SIMD is the
default path; GPU engages only above `GPU_THRESHOLD = 50_000` (`nanna-memory`). Run with
`cargo bench -p nanna-bench --bench vector_search`.

Reference measurement *(2026-08-05, release, AMD Zen 4 / AVX-512, 768-dim, fixed seed
`0x0A_11A_B01`)* — batch cosine of one query against N store vectors:

| Metric | Baseline (p50 / p95) | Budget (p95 max) | Source | Notes |
| --- | --- | --- | --- | --- |
| SIMD batch search @ N=1k | **0.042 / 0.072 ms** | **≤ 0.20 ms** | `nanna-bench` `vector_search` / `simd_batch/1000` | default store size |
| SIMD batch search @ N=10k | **1.63 / 2.55 ms** | **≤ 5.0 ms** | same / `simd_batch/10000` | |
| SIMD batch search @ N=50k | **10.1 / 11.0 ms** | **≤ 25 ms** | same / `simd_batch/50000` | GPU crossover threshold |
| GPU fixed dispatch overhead | ~200 µs (characterized) | not budgeted | `nanna-gpu` after wgpu 30 | was ~750 µs; GPU path still needs a live adapter |

Budget: SIMD p95 must not exceed the ceilings above on the reference tier (≈2× measured headroom
for CI noise). N=100k and RAM/100k remain unbaselined until a criterion body covers them. The
GPU half of the crossover stays in `nanna-gpu` benches (adapter-dependent) and is not a CI gate.
Machine-readable rows: `suite = "vector_search"` in `bench/budgets.toml`.

---

## Suite 5 — Resource guardrails (not yet baselined)

Instrument: release-binary size, idle RSS, VRAM ceiling under the reference tier, cold-start
to first healthy `/readyz`.

| Metric | Baseline | Source | Notes |
| --- | --- | --- | --- |
| Release binary size (`nanna-daemon`) | *not yet baselined* | — | stripped, fat LTO |
| Idle RAM (daemon, no model loaded) | *not yet baselined* | — | |
| VRAM ceiling (local model loaded) | *not yet baselined* | — | must hold the 16 GB reference / 8 GB guardrail |
| Cold-start to `/readyz` | *not yet baselined* | — | |

Machine-readable rows land under `suite = "guardrails"`.

---

## Suite 6 — Efficiency (not yet baselined)

Instrument: prompt-cache hit rate, tokens saved by routing / compression / dedup, wall-clock
per unit of agent work.

| Metric | Baseline | Source | Notes |
| --- | --- | --- | --- |
| Prompt-cache hit rate | *not yet baselined* | — | Anthropic / OpenAI native cache |
| Tokens saved by tiered compression | *not yet baselined* | — | |
| Tokens saved by dream-phase dedup | *not yet baselined* | Suite 3 summarizer_calls row | already 6 → 0 on the retention corpus |
| Wall-clock per completed harness item | *not yet baselined* | — | pairs with Suite 4 tokens/item |

Machine-readable rows land under `suite = "efficiency"`.
