# Agent-eval suite — task-success @ budget denominator

The **governing metric** for Nanna performance is **task success @ budget**: the
fraction of this suite the *default local model* completes within the reference
GPU's VRAM ceiling and a p95 wall-clock target per task.

> A faster model that fails more tasks is **not** an improvement.

Secondary metrics (reported alongside, never instead of task success):

| Metric | Definition |
| --- | --- |
| **Capability density** | task-success rate ÷ peak VRAM (GB) on the tier under test |
| **Cost of escape hatch** | % of tasks forced to escalate to a cloud provider |
| **Tokens per completed item** | total in+out tokens ÷ verified completions (Suite 4 bookkeeping) |
| **Tool-call validity rate** | fraction of model tool calls the registry accepts and runs |

Methodology, the six criterion suites, and hardware notes live in the
`daily-dev` skill (Appendix B). Measured numbers live in [`BASELINE.md`](./BASELINE.md).
Machine-readable budget rows live in [`budgets.toml`](./budgets.toml).

---

## What counts (the denominator)

Only **seeded, machine-checked** tasks count toward task success. Model-created
extra tasks (self-decomposition via `todo`) are reported but **must not inflate
the score** — see `assert_seeded_verified` in the live harness.

A task is a **verified completion** only when:

1. Its status is `done`, **and**
2. The harness recorded an activity entry `completed` with `detail.verified == true`
   (the acceptance check ran in the harness and passed).

A model claim of `TASK COMPLETE` that the environment check refutes is a
**false-success claim** — counted, never admitted as done. **False-success
completions admitted must stay 0** (correctness fixture).

### Tiers that form the suite

| Tier | Instrument | Tasks | What "pass" means | Status |
| --- | --- | --- | --- | --- |
| **Smoke** | `live_long_horizon::live_task_success_at_tokens` | 5 minutes-scale items (regex-on-file ×3, command-exit-0 ×2, one `depends_on` edge) | verified completions / 5 | **Live baseline** — 5/5 @ 22.6k tok/item (qwen3.5:9b, 16 GB) |
| **pass^k (smoke)** | `live_long_horizon::live_pass_k` | same 5, repeated k independent runs | fraction of runs with 5/5 | Open (instrument exists; k-series not yet baselined) |
| **Endurance** | `live_long_horizon::live_endurance` | 42 dependency-chained fail-to-pass features (`minidb`) | verified **seeded** features / 42 | **Live datapoint** — 14/42 local @ 6 h; 33/42 openrouter/free @ 3.3 h; frozen-harness series 32/42 (qwen3.5:9b) |
| **Harness integrity** (model-free) | `nanna-agent::harness` tests | scripted step runners | false-success admitted = 0; drift ≤ budget | **Baselined** in Suite 4 — gates the control loop, not the model |

The **denominator for the governing metric** is:

```
task_success = verified_seeded_completions / seeded_tasks
```

reported **per tier** (smoke and endurance separately — do not average them).
A change that improves smoke while regressing endurance is not a win until both
hold or the BASELINE row is deliberately updated with a cited measurement.

### What does **not** count

- Model-created extra tasks (reported, excluded from numerator and denominator).
- Cancelled / abandoned items (containment working as designed — not failures of
  the acceptance contract, but they do lower the pass rate).
- Cloud-only runs when scoring the **local** default model (cloud is a separate
  datapoint / escape-hatch cost, never the local tier's score).
- Unchecked GUI chat turns, unit tests, or criterion microbenchmarks (those are
  Suites 1–3 / 5–6 — necessary gates, not the agent-eval denominator).

---

## Reference hardware tiers

Name the tier in every reported number. Numbers without a hardware note are
deterministic harness results (fixed-seed, model-free) and reproduce on any host.

| Tier | Hardware | Role | Default model target |
| --- | --- | --- | --- |
| **Reference (16 GB)** | RTX 4070 Ti SUPER 16 GB (Vulkan/wgpu) + AMD Zen 4 (AVX-512) | Primary score; every BASELINE live row names this unless noted | qwen3.5:9b (text-only GGUF Q4_K_M ≈ 6 GB) |
| **Low-VRAM guardrail (8 GB)** | 8 GB consumer card | Must still pass smoke; may drop to a smaller tier (Qwen3.5-4B) + f16 | Open — not yet baselined |
| **CPU-only** | Zen 4 / Apple Silicon NEON | Offline fallback; smoke only, relaxed wall-clock | Open — not yet baselined |

**VRAM ceiling** is a hard gate on the tier under test: a run that exceeds the
card's usable VRAM (or forces an unplanned cloud escalation) fails the budget
even if every acceptance check passed.

**Wall-clock targets** (p95, per tier — refine when more n exist):

| Eval | Reference 16 GB | 8 GB guardrail | CPU-only |
| --- | --- | --- | --- |
| Smoke (5 tasks) | ≤ 5 min | ≤ 10 min | ≤ 30 min |
| Endurance (42 features) | ≤ 6 h wall-clock cap | *not required* | *not required* |

---

## How pass rate is scored

### Smoke — `task success @ tokens`

```text
NANNA_EVAL_MODEL=qwen3.5:9b \
cargo test -p nanna-daemon --test live_long_horizon \
  -- --ignored --nocapture live_task_success_at_tokens
```

| Field | Rule |
| --- | --- |
| Numerator | seeded tasks with `done` + verified activity |
| Denominator | 5 (fixed seed plan) |
| Tokens/item | `(input_tokens + output_tokens) / verified_completions` |
| Pass | rate = 1.00 on reference tier **and** false-success admitted = 0 |
| Record | update Suite 4 "Live run" in `BASELINE.md` only on a real measured change; cite commit |

### pass^k

```text
NANNA_EVAL_MODEL=qwen3.5:9b NANNA_EVAL_K=5 \
cargo test -p nanna-daemon --test live_long_horizon \
  -- --ignored --nocapture live_pass_k
```

Reliability across repeated trials. A single 5/5 does not establish the budget;
pass^k is the number that detects flaky harness or model variance. **Not yet
baselined** — when first measured, land the row under Suite 4 and add a
`budgets.toml` entry.

### Endurance — verified features @ wall-clock

```text
NANNA_EVAL_MODEL=qwen3.5:9b \
NANNA_EVAL_HOURS=4.5 \
NANNA_EVAL_LOG="warn,nanna_tools::registry=debug" \
cargo test -p nanna-daemon --test live_long_horizon \
  -- --ignored --nocapture live_endurance
```

| Field | Rule |
| --- | --- |
| Numerator | seeded minidb features with verified completion |
| Denominator | 42 |
| Resume | store is the checkpoint (`NANNA_EVAL_DIR`); re-run resumes unless `NANNA_EVAL_FRESH=1` |
| Pass (local reference) | *datapoint, not budget yet* — current best frozen-harness **32/42** @ 4 h (qwen3.5:9b) |
| False-success admitted | must stay **0** across the whole window |

Do **not** read the harness's in-flight `done=N` counter as the score — it counts
*closed* tasks, and closed includes **cancelled**. Only `assert_seeded_verified`
(or the equivalent activity scan) is authoritative.

### Harness integrity (gates every model run)

```text
cargo test -p nanna-agent harness
```

Model-free. If these regress, no live score is meaningful:

- false-success completions admitted = **0**
- drift containment ≤ **6000** tokens on the fixed script
- loop acceleration abandons in **< 4** steps

---

## Relationship to the six criterion suites

| Suite | Role vs agent-eval |
| --- | --- |
| 1 Inference | Can the local model *run* inside the VRAM ceiling? (precondition) |
| 2 Vector search | Memory retrieval latency at scale (precondition for recall tools) |
| 3 Dreaming | Information retention under compression (memory quality gate) |
| **4 Agent loop / long-horizon** | **This document — the task-success denominator** |
| 5 Guardrails | Hard ceilings (binary size, idle RAM, VRAM) — CI-fail |
| 6 Efficiency | Tokens saved / cache-hit — cost of achieving the Suite 4 score |

Suites 1–3 and 5–6 can block a ship on their own budgets; they do not substitute
for a Suite 4 task-success number when claiming agent capability.

---

## Updating scores

1. Run the eval on the named tier (release tools, pinned model id, noted
   `num_ctx` / harness flags). GUI-path endurance legs run **only** through the
   committed [`bench/gui-leg/`](./gui-leg/) driver — a leg whose ledger lacks
   the driver-hash header is not a result.
2. Record **verified seeded / total seeded**, tokens/item, wall-clock,
   false-success admitted, model id, tier, commit — **and, for GUI-path legs,
   the leg's validity verdict plus its work denominator** (tool calls /
   side-effecting calls / tokens from the leg summary). Only `VALID` /
   `VALID_WITH_CAVEATS(...)` legs may contribute a numerator; an
   `INVALID(reason)` leg is recorded as exactly that, never as 0/42 (the
   ministral lesson: a plausible number from a dead system reads like a real
   one and confirms whatever you already believed).
3. Update the matching table in [`BASELINE.md`](./BASELINE.md) **only** on a
   legitimate measured change; never hand-edit a live row "to match expectations".
4. If the number is a budget (not just a datapoint), mirror it in
   [`budgets.toml`](./budgets.toml) and ensure the CI gate (when present) reads
   that file.

### Open build-out (honest)

- [ ] 8 GB guardrail smoke baseline
- [ ] CPU-only smoke baseline
- [ ] pass^k series on smoke (and later endurance)
- [ ] Published task set reuse (Terminal-Bench easy-tier / SWE-bench Lite) as an
      external denominator alongside minidb
- [ ] CI gate: fail a PR that regresses a Suite 4 budget past threshold
- [ ] Machine-readable smoke/endurance rows in `budgets.toml` once budgets (not
      only datapoints) are declared

---

*"Task success @ budget" is the fraction of **this** suite. Everything else is
supporting evidence.*
