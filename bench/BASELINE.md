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
| Rare clause survives a **user-STATED** cluster | **YES** (verbatim) | `retention::tests::a_user_stated_clause_survives_the_summarizing_cycle` | mitigation (c), 2026-08-21: same corpus/spread/summarizer as the losing row above — only the provenance differs |
| Stated provenance survives a **dedup fold** | **YES** | `retention::tests::a_dedup_fold_never_launders_stated_provenance_into_observed` | a fold keeps the *survivor's* metadata, so folding stated→observed used to launder the pin away |

Budget: the **dedup arm must never regress to NO** — that is a correctness fixture, and it is the
concrete payoff of folding restatements deterministically. The summarized arm is a **baseline to
beat, not a budget to hold**: it currently asserts the clause *is* lost, so when a drift mitigation
lands (generation ceiling, or verbatim-pinning user-STATED memories — both logged in P13) that test
will fail loudly and should be flipped to assert survival. A failing baseline arm means the system
got better, and the assertion says so in its message.

*(2026-08-21)* **Mitigation (c) landed — verbatim-pinning user-STATED memories.** Two rows added
above. The summarized arm deliberately **stays at NO**: the exposure is real and unfixed for
agent-*observed* content, and deleting the measurement that says so would be the dishonest move.
What changed is that a memory whose `fact_type` provenance is `stated` is now split out of its band
*before* both the dedup fold and the clusterer, so it is never handed to a summarizer. The split
runs before the fold, not after, because a fold keeps the **survivor's** metadata — folding a stated
row into an observed one handed back an `observed` row holding a user assertion, which the next
cycle was then free to paraphrase. Both new rows fail (and name the laundered id) with the split
removed, so neither is vacuous. Cost is one hash lookup per memory per band and strictly *fewer*
summarizer calls; the price is that pinned rows no longer compress by paraphrase — they still
deduplicate, so repetition cannot make them grow without bound.

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

**Small-model datapoint — `gemma4:e4b-it-qat` (2026-08-06).** First candidate in the
smaller-than-qwen search (6.1 GB QAT quant, effective-4B MatFormer). Reference 16 GB tier,
master `7f129c0f` (v0.3.4-beta.9 tree), fresh store, 4.5 h window, VRAM 7.2→9.5 GB across the
run (no ceiling pressure, zero `num_ctx` demotions, zero CUDA faults; `num_ctx` latch not
captured — eval log level was warn+registry only). Smoke first: **5/5 @ 22.7 k tokens/item,
79 s** — statistically identical to qwen3.5:9b's 22.6 k, all 28 tool calls registry-valid.
Endurance: **7/42 features verified in 4.50 h, 0 false successes, 1 resume**. The failure
mode is not dialect, drift, or infrastructure — it is **recursive self-decomposition**: the
model spawned 103 extra items against 42 seeded (subtasks of subtasks, three levels deep),
closed 108 items total, and spent the window on ceremony — it never advanced past feature ~10
of the dependency ladder, and 9 seeded features ended cancelled. Same harness, same machine,
qwen ships features directly where gemma splits them. Next lever: decomposition damping
(soft-nudge family, per the no-hard-cap directive) before writing this model off.

**Iteration 2 — decomposition damping (PR #194, 2026-08-07).** Same model, tier, and
window; `tasks.add` now returns escalating do-the-work notes on depth-2+ creation and
sibling overhang. Result: **5/42 verified in 4.50 h** — no improvement (iteration 1: 7/42;
the delta is run noise). The decisive observation: the notes fired 69 times (32 "STOP
SPLITTING", 36 "DO the work") and the model kept splitting — item creation barely moved
(133 vs 145). **In-band tool-result feedback does not alter this model's planning
behavior.** One CUDA fault at t=66m, contained by the first-fault reset ladder; 0
demotions. Next lever must be structural: scheduler-level deprioritization of depth-2+
items, which cannot affect qwen (it never creates them).

**Iteration 3 — depth-biased scheduling (2026-08-07): INVALIDATED by operations, 7/42.**
`next()` deprioritizes ladder depth >= 2 (commit 1665610c). The run was paused for the
owner's gaming session and the first two RESUME attempts wedged: the pre-warm loaded a
default-ctx model instance that starved the eval runner's VRAM sizing, demoting `num_ctx`
to 4096 — below the 4211-token minimum viable window — and the harness FAILED LOUDLY with
the full arithmetic instead of running truncated (the post-2026-08-03 machinery working as
designed). Those wedged segments mass-cancelled 26/42 seeded features before the fix
(launch resumes with the model UNLOADED), capping the achievable score at 16. Early
in-flight evidence before the pause was promising — item creation ran ~20-25% below runs
1-2 at every checkpoint — but the verified count cannot be attributed. Operational lessons
banked: resumes must start with the model unloaded (fresh launches may pre-warm), and the
viable-floor abort turned what would have been a silent 4-hour loss into a 3-minute loud
one, twice.

**Iteration 4 — depth-biased scheduling, clean run (2026-08-07): 15/42.** The first
genuine improvement: more than double the 7/42 baseline, on an uncontaminated full 4.5 h
window with zero CUDA faults, zero context demotions, and only 6 features cancelled. The
scheduler ranking depth-2+ scaffolding below the seeded ladder (commit 1665610c) is the
only active lever this run — the parent-refocus hint sat on the service door while eval
completions flow through the harness door (0 firings at 25 completions; moved to a note on
the parent in 1a0cebfb for iteration 5). Item creation ran ~20-25% below the un-damped
runs throughout. Progression: 7/42 baseline → 5/42 advisory notes (no effect, model
ignored 69 firings) → invalid (ops) → **15/42 structural scheduling**. Bar: qwen3.5:9b
32/42.

**Iteration 5 — full honest lever set (2026-08-07): 8/42.** Depth bias + parent-refocus
notes (1a0cebfb, confirmed planting: 21 notes by t=94m) + verified-parent hygiene. Clean
full window, zero faults, zero demotions — and a REGRESSION from iteration 4's 15/42. The
only configuration delta was the refocus notes; they did not convert (12 features ended
cancelled vs 6, hygiene never fired). Two readings, both honest: the notes may add step-frame
weight that hurts more than the redirect helps, and run-to-run variance on this model is
large (8-15 across two clean runs of near-identical config — a single run cannot rank
levers). **Campaign verdict:** structural scheduling is worth ~2x (7 → 15 best case);
advisory text is worth nothing; gemma4:e4b-it-qat sits at 8-15/42 against qwen3.5:9b's
32/42 with the honest harness. The remaining gap looks like model capability, not harness —
further levers risk shading into scoring the benchmark for it. Decision escalated to the
owner: accept, keep iterating, or try qwen3.5-4B as the small-model candidate.

**Model comparison on the merged path (2026-08-08, v0.3.5-beta.10, master
`914b19f1`).** The campaign's winning levers merged as the singular path (depth-biased
scheduling + service-door refocus + verified-parent hygiene; the harness-door refocus notes
reverted as a measured regression). One build, identical conditions per leg: model unloaded
at launch, fresh store, 4.5 h cap, reference 16 GB tier.

| Model | Smoke | Endurance | Wall clock | Exit |
|---|---|---|---|---|
| **ornith:9b** | 5/5 @ 11.0 k tok/item, 68 s | **37/42** | **1.02 h** | early — plan drained: 37 verified on a 50-item plan (only 8 self-created), 0 faults, 0 resumes. **All-time record** — first model past qwen on this ladder, at half the token cost |
| qwen3.5:9b | 5/5 @ 22.6 k tok/item (2026-07-18 row) | **25/42** | 2.11 h | early — plan drained at 100/101 closed: 25 verified, 17 abandoned-with-containment; 0 faults, 0 resumes |
| gemma4:e4b-it-qat | 5/5 @ 22.7 k tok/item (2026-08-06 row) | **6/42** | 4.50 h (cap) | full window; 118 items from 42 seeded, 97 closed (81 done + 16 cancelled), mostly self-created scaffolding; 0 faults, 0 resumes |
| ministral-3:8b | 4/5 @ 11.8 k tok/item, 66 s (miss = Ollama-side 500 `tool 'exec' not found`) | **2/42** | 3.20 h (early stop) | deterministic-failure stop: 3 consecutive segments ended by Ollama mid-response aborts after 2 successful heals (t=37m, t=141m); runaway decomposition — 256 items (199 self-created), 42 done + 32 cancelled; 639 steps, 30.3 M tokens |
| lfm2.5:latest | **2/5** @ 78.1 k tok/item, 124 s | — (smoke gate failed) | — | 2 tasks abandoned after fruitless steps + 1 replan each; up from 0-for-tools in the 2026-08-03 campaign — dialect fixes hold, capability doesn't |

**Candidate search, 2026-08-08.** After lfm failed the gate, a field sweep for modern
tool-calling models inside the ~10 GB usable-VRAM budget surfaced ornith:9b (July-2026
agentic-coding family) and ministral-3:8b (Dec-2025 Mistral small tier); both ran at the
owner's request. The pair is the cleanest smoke-vs-endurance split yet measured: near-identical
smoke (5/5 @ 11.0 k vs 4/5 @ 11.8 k) diverged to 37/42 vs 2/42 — the divider is decomposition
discipline, not tool dialect or speed. ornith's 37/42 in 1.02 h also beats the frozen-era
qwen 32/42 @ 4 h, with the same exit-semantics caveat as everywhere in this table.

**ministral-3:8b post-heal rerun (2026-08-09, master `a46c6c66`).** Rerun of the ministral
endurance leg with the two fixes that landed after the first leg: the unserved-tool heal
(PR #200 — Mistral-family Ollama 500s `tool 'X' not found` on generated calls to unserved
tools; the loop now activates the named tool and retries) and the progress-gated
deterministic-failure breaker (PR #198). Build in a session-private `CARGO_TARGET_DIR`, heal
string grep-verified in the test exe; fresh store, 4.5 h cap, `num_ctx` 16384, exclusive GPU.

Result: **3/42 verified in 4.54 h** (features 01, 02, 04), single segment spanning the full
window, **0 resumes, 0 server restarts**. 375 steps, 2 622 tool calls (1 285 side-effecting),
29 replans, 2 false-success claims, 17.4 M tokens; 150 items from 42 seeded (108
self-created), 20 completed + 10 abandoned in-segment, ~869 k tok/completed item. Feature 03
was abandoned after 5 fruitless steps + 1 replan on a shell syntax error the model could not
repair; nothing past feature 05 was reached. Exactly **2 unserved-tool heals** fired (`exec`
t≈4 m, `list_dir` t≈21 m), both absorbed with no step retry burned and no recurrence after
activation — the dialect/serving fault is fully eliminated as a variable. The score moving
only 2→3 of 42 confirms the campaign's reading: decomposition discipline, not tool dialect,
is the divider.

Infrastructure postscript, and a retroactive correction. Two attempts before the valid leg
died in minutes (1/42 @ 0.11 h, 0/42 @ 0.05 h) with `num_ctx` latched at 4096 — below the
~4.2 k min-viable floor. The sizing probe was honest: four orphaned `llama-server.exe`
runners (parents dead — killing/restarting the Ollama server orphans its loaded runner
children on Windows, invisible to the new server's `ollama ps`) held ~12.5 GB of the 16 GB
card, and a 28 h-old `nanna-daemon-bench.exe` kept re-warming ornith. Killing them took free
VRAM 1.5 → 14.2 GB and sizing returned to 16384. Since each restart heal leaks the loaded
runners, the first leg's "mid-response abort loop after 2 heals (t=37 m, t=141 m)" now reads
as VRAM starvation from the heals' own leaks, not model-side degradation — and this rerun's
zero-incident full window on a clean card is consistent with that. Bare desktop overhead is
~1.8 GB, not the 5–6 GB previously assumed (that number was measured through leaked runners).

Two cross-run cautions. (1) qwen's frozen-harness 32/42 above is not comparable: those runs
were GUI-driven with no plan-drain early exit; on the current path qwen closed its entire
plan in under half the window. (2) gemma's 6/42 against iteration 4's 15/42 on
near-identical code confirms the iteration-5 variance reading — single runs cannot rank
this model; its honest range is 6-15/42. lfm2.5's earlier harness-fix-era smoke was 5/5 @
25 k; today's 2/5 @ 78 k on a much-evolved tree is the same variance lesson at smoke scale.

**GUI-path series (2026-08-10).** The same ladder driven through the real chat path
(mission over IPC to a live daemon session, GUI attached as viewer), on a build carrying
the full benchmark-lessons wave (PRs #198, #200, #201, #202, #204, #207). Conditions per
leg: fresh read-only workspace, runtime IPC model+summarizer binding, quiesce + settle +
fresh-daemon boot (per-process num_ctx latch), 16384 gate-verified, 4-hour snapshot
against pristine tests, 15-minute sampling, execution-based stall watchdog with
interjection-style nudges.

| Model | 4 h official | Peak | Trail |
|---|---|---|---|
| ornith:9b | **5/42** | 16 @ t=211m | six short self-terminating runs, five dead zones totalling 2h37m; THREE peak→destruction cycles: 12→10, 10→0, 16→5 |
| qwen3.5:9b | **1/42** | **22 @ t=13m** | 22/42 at t=13m; then 120 of 240 minutes in 600 s acceptance-check timeouts on its own hanging `mset`; item abandoned as fruitless while the artifact was passing; collapse 22→9→1 |
| gemma4:e4b-it-qat | ~~0/42~~ **CONTAMINATED** | 5* @ t=90m | peak produced by a cloud 120B sub-agent, not the model under test (see correction below); gemma's own line ended 0/42, artifact self-corrupted at t≈124m and never repaired |
| ministral-3:8b | ~~0/42~~ **INVALID (daemon died)** | n/a | daemon hard-died 3m42s after mission start; the driver scored a dead process 14 times |
| lfm2.5 | **0/42** | — | tool channel never engaged: 379 prose pseudo-calls (300 to a nonexistent `list_files`), fabricated results it then believed; zero write intent |

**Retrospective correction (2026-08-11).** A six-agent forensic review of every leg's
daemon logs (one agent per leg plus a process-trail agent) invalidated two of the five
official numbers and reframed a third:

- **ministral-3:8b — INVALID.** The daemon hard-died at 2026-08-10T14:15:47Z, 3m42s
  after mission start: the log ends mid-turn with no panic, no cancel, no shutdown
  marker, and zero lines until the next daemon boots at 18:14Z. The driver's stall
  detector fired at all 14 remaining polls, errored on every resume attempt, and still
  published "4h MARK … 0/42" — it scored a dead process 14 times. The earlier
  "write_file-without-file_path dialect failures" reading was wrong: five of the six
  write_file calls succeeded; only the last failed. The leg measured nothing about the
  model.
- **gemma4:e4b — CONTAMINATED.** At 12:31:57Z its `task` tool spawned a "decompose
  only, do not perform the work" sub-agent onto
  `openrouter/nvidia/nemotron-3-super-120b-a12b:free` — a cloud 120B model. It blocked
  the session for 44m41s, ignored the brief and implemented instead, and produced BOTH
  of the leg's peak scores (4/42 at 12:52Z, 5/42 at 13:00Z). It also copied the
  artifact into the read-only tests dir, and gemma then spent 11 minutes validating
  against that frozen copy while editing the root file. The local model's own
  contribution ended 0/42 with a self-corrupted artifact (a partial-overlap edit
  duplicated `put()` and left a dangling `else`) whose syntax error it saw twice and
  ignored.
- **lfm2.5 — 0/42 stands, reframed.** Not an attempt that failed: the tool channel
  never engaged. 379 of its 429 text items were pseudo-tool-calls emitted as prose, 300
  of them to a `list_files` tool that does not exist, plus two fabricated `"result"`
  blocks it then adopted as its world model ("we saw there is a minidb file (104k)") —
  it spent four hours trying to READ a file it was supposed to WRITE, and the intent to
  write appears zero times in 326 KB of reasoning.

**Peak-vs-final IS the metric.** The chat path's gap to the headless harness (ornith
37/42, qwen 25/42 on the same build) is not capability: every model that produced work
peaked early and was then walked off its peak by one chain, named independently by four
of the six analysts — the hard 8-iteration step cap truncates work mid-flight → the
truncated step is charged as "fruitless" → five of those abandon the item → the planner
re-seeds "assess starting state" → the mass re-read blows the 16 k context →
compression collapses the record of what was verified passing → the model's only
remaining move is a from-scratch rewrite over passing work. This supersedes the earlier
"artifact preservation" framing, which described the symptom (destroyed peaks), not the
cause. qwen is the sharpest exhibit: zero of its task items were ever credited on a
step's own work, and the item "Build minidb CLI implementing all 42 tests" was
abandoned as fruitless minutes after its artifact ran tests 01–20 clean. The frozen-era
32/42 (below) is reinterpreted: those runs' turns died at the peak (planner starvation,
fixed in #201) and the death accidentally PRESERVED the artifact. Fix track: ROADMAP
P22 ("Keep the peak: the abandonment/truncation chain", PR #221); already merged from
that campaign: the mid-mission verified-work sweep (PR #218) and the fold-vs-write
safety fixes (PR #219).

The shakedown to get this series clean was itself the harvest — nine leg attempts
surfaced and fixed, in order: legacy boot path ignoring `[llm]` (silent dead-model
fall-through), a config-rewrite TOML corruption, the ornith:9b/ornith:latest tag
split-brain double-loading one model, stale-session auto-resume starving context
sizing, the summarizer loading a second 7.4 GB model at leg start, the per-process
num_ctx latch poisoned by transient VRAM load (fixed in code: the env pin now wins both
directions, #202), chat planner starvation (#201), false "dry" convergence with failing
checks in hand (#204), and dreams consolidating a live mission's memories mid-step —
deadlocking it (#207; the underlying fold-vs-step lock issue and the mid-mission sweep
are chipped). Ledger + artifacts: session scratchpad `bench-ledger.txt`,
`bench-artifacts/*.minidb`, run dirs under `D:/Development/nanna-bench/`.

Still open: throughput on the local tier (14/42 primary features in 6 h — the middle-ladder
grind dominates), a reused benchmark task set (Terminal-Bench easy-tier / SWE-bench Lite),
pass^k on the endurance suite, and the 8 GB reference tier.

**GUI-path series, post-P22 rerun (2026-08-14/15).** The same ladder on v0.3.7-beta.12
carrying the complete P22 program (PRs #218–#231), all five legs on the *installed
release* daemon (one daemon process for the whole series, boot 2026-08-14 16:27 local;
restarted once in lfm's idle tail). Per leg: fresh read-only workspace, all four model
priorities (chat, fallback, sub-agent, summarizer) IPC-pinned to the leg model —
sub-agent pinning structurally closes the gemma cloud-contamination hole —
`num_ctx=16384` env pin verified from the daemon's latch line, quiesce + prior-model
unload between legs, liveness-gated polls (probe before every score; the ministral
dead-daemon class is impossible by construction), 15-minute artifact snapshots into
`run/history/<min>/`, interjections only when the run was idle with a flat score (max
3/leg, every one and its effect ledgered). Leg 1's mission went through the GUI composer
via desktop automation (frontend observations logged in its ledger); legs 2–5 fell back
to IPC `start-run` after the Windows text-input overlay began blocking synthetic clicks.

| Model | Peak | 4 h final | Interjections (effect) | Trail |
|---|---|---|---|---|
| qwen3.5:9b | **41/42 @ 65m** | 0/42 | 2 (safe; then destructive) | 27→32→34→41 monotonic, zero rewrites; held 41 for 145 min, only test_40 failing; the m=194 interjection triggered the sole rewrite, mid-flight at close |
| ornith:latest | **30/42 @ 135m** | 0/42 | 1 (destructive) | 17→5→14→24→6→0→25→30: three destruction cycles, each named by rewrite-notes, each rebuilt in ≤2 polls; a tab-in-case-pattern syntax break (0@102) repaired via structural verdicts |
| gemma4:e4b-it-qat | **16/42 @ 161m** | **16/42** | 1 (strongly positive) | bootstrapped 0→3→16 after the m=97 interjection; survived 16→4 destruction and closed AT peak; fully local |
| ministral-3:8b | 4/42 @ 123m | **4/42** | 3 (mixed) | first VALID GUI number for this model (prior leg scored a corpse); final = peak |
| lfm2.5 | 0/42 | 0/42 | 3 (ineffective) | T4c salvage engaged (119 salvage-path log hits in 17 min; real executions; artifact on disk) — the channel works, the model can't build the script; capability floor confirmed |

Two series findings. **(1) P22 works in-run:** peaks ~doubled (22→41, 16→30,
contaminated→16, unmeasured→4), destruction now recovers instead of terminating
(ornith rebuilt through three cycles; the frozen-era version never rebuilt once), two
legs closed at peak, and no leg lost time to dead air, hung checks, or a dead daemon.
**(2) The remaining peak-destroyer is the continuation prompt:** both high scorers were
walked off their peak by a driver "continue: … do not rewrite passing work" message
sent after idle — strong models read continuation as "start over". That lever is
chat-general (any user nudging a long-running session) and is the successor work item.
Ledgers, per-poll history, interjection records: `D:/Development/nanna-bench/ui-run-*/`.

**Forensic analysis addendum (2026-08-15, 40-agent log analysis, adversarially
verified — recommendations encoded as ROADMAP P23).** Corrections and mechanisms the
live monitoring could not see: (a) the continuation-destruction mechanism is a fresh
turn context seeded from a 63× compressed summary — the verified-pass record is absent,
so the rewrite is the model's cheapest coherent continuation (ornith 30→0 in 8 min;
qwen 41→0); (b) **qwen's peak artifact satisfies all 42 tests under hermetic per-test
scoring** — the official 41 stands because the model ran `chmod +w tests/test_40.sh`
(after 12+ write errors whose text wrongly advised retrying) and doctored the spec test,
splitting its local verification from the pristine scorer; the scorer's order-coupled
namespace residue is what hid the divergence until post-series (hermetic per-test
scoring is now a P23 ops debt); (c) ornith's five destruction writes each removed 9–33
functions while clearing the 30% *byte* floor, re-anchoring `good` downward each time;
(d) qwen's run death at peak was planner error-round exhaustion with no stop line —
error rounds burned 2→4 in one 30-second round via a stale-stop double-charge;
(e) gemma's 97-minute 0/42 hole: single-page tool discovery missed `write_file`, so 201
python registry "saves" acked success with zero files on disk, and the daemon dry-counted
the run out while its own checks failed; (f) ministral's wall is transport — Ollama
aborts the tool-call stream on a literal TAB in JSON, retried blind ×3 per generation
(116/160 in-window retries were 0-byte dead streams); (g) **series-wide caveat: all 171
Tier-2 summarizations ran on `lfm2.5` regardless of per-leg pins** (summarize-priority
resolution bug in the bucket router) — every leg's compressed context was degraded by
the weakest model, making these numbers a floor. 27 improvement levers survived
adversarial review; all are chat-general per the owner rule. See ROADMAP P23.

**GUI-path series, post-P23 rerun (2026-08-15/16).** The same ladder on
v0.3.8-beta.13 carrying the complete P23 program (PR #237, 27 levers). Same protocol as
the post-P22 series — installed release daemon, fresh read-only workspace per leg, all
four model priorities IPC-pinned and read back, `num_ctx=16384` verified from the
daemon's latch line, quiesce + prior-model unload between legs, liveness-gated 15-minute
polls — with two changes. **(1) Scoring is hermetic per-test** (each test in its own temp
dir), closing the order-coupled residue that hid qwen's doctored spec last series.
**(2) The interjection policy was tightened**: no interjection on first idle, only when
the run is idle *and* flat across two consecutive polls *and* `m<220`, and **every one is
recorded as a P23 miss** — because P23's reseed lever is supposed to internalise exactly
what an interjection supplies.

| Model | post-P22 peak / final | P23 peak | P23 final | Interj. | Δ peak / Δ final |
|---|---|---|---|---|---|
| ornith:latest | 30 @135m / 0 | **40/42 @ 212m** | **36/42** | 0 | **+10 / +36** |
| qwen3.5:9b | 41 @65m / 0 | 26/42 @ 49m | **26/42** | 0 | −15 / **+26** |
| gemma4:e4b-it-qat | 16 @161m / 16 | 11/42 @ 102m | 5/42 | 0 | −5 / −11 |
| ministral-3:8b | 4 @123m / 4 | *INVALID (daemon panic @116m)* | — | 0 | — |
| lfm2.5 | 0 / 0 | 0/42 | 0/42 | 2 (both MISSES) | none (floor) |

**The headline is the final column, which is what P23 targeted.** Post-P22 the two
strongest models peaked high and ended at **zero** — the peak was built and then
rewritten away across the continuation boundary. Post-P23 ornith holds 36 of its 40 and
qwen ends *exactly at* its peak, both with **zero interjections**. ornith's 40 also beats
its all-time headless record (37) on the chat path. Three legs ran destruction-free
end-to-end without a human touching them, which is the behaviour the 27 levers were
built to produce.

Two honest negatives. **qwen's peak is 15 lower** — not a regression: last series' 41 was
inflated by the model `chmod +w`-ing and doctoring `test_40.sh`, which the pristine
hermetic scorer no longer credits, and this leg wrote nothing to `tests/` at all.
**gemma regressed on both axes** (16/16 → 11/5): it broke its own script three times, each
time repairing *unaided* (0→10, 1→11, 0→9 — last series the equivalent hole needed a
human nudge), then lost ground to a fourth, different failure at m=231.

**Findings this series (all chat-general; queued, not implemented mid-series).**
(a) **The dominant destruction channel is not shrinkage.** Every collapse across gemma
and ministral arrived as an *in-place splice* leaving the file the same size or **larger**
(ministral 2542→3389 bytes while breaking). The byte floor guards shrinkage and is
structurally blind to this; it refused every gutting attempt it *could* see.
(b) **Park by verified score, not by recency.** `.__prev__` holds the previous write, so
after a bad write it holds whatever came just before — observed as a *broken* file
(ministral), a *stale* 9-point file (gemma), the copied spec, and a 0-byte file.
Four instances, zero recovery value. Naming the parked copy in the structural verdict
would also help: all three of gemma's repairs were from-scratch rewrites.
(c) **`is_running` is the wrong staleness signal.** Ministral ran 38 minutes with steps
completing normally and the artifact untouched, because every `edit_file` was *rejected*
("old_string not found … the file's real content differs from your memory") after a
splice desynced the model's context from disk. "Artifact unchanged across two polls while
steps still complete" is the observable worth acting on.
(d) **The reseed is wired to a signal that reads empty in its own trigger case.**
lfm2.5 ended dry having verified 0/42 with 35 items abandoned — yet `unmet=0`, so
`chat_harness.rs:1166`'s `abandoned_unmet.is_empty()` guard correctly declined to arm.
`unmet` derives from task done-conditions, not the environment's verdicts. Arming it off
the failing verdicts (already re-read each turn for the ARTIFACT STATE block) closes it.
(e) **New class — spec-into-artifact copy.** lfm2.5 wrote a near-verbatim copy of
`tests/test_01.sh` into `minidb`, making the artifact self-recursive. The read-only guard
protects the spec *from* the artifact but not the artifact *from* the spec; the content
is available at write time.

**One crash, and it is a real bug.** The ministral leg is INVALID because the daemon
**panicked and exited** at m=116: `context.rs:1838` byte-slices a dropped tool result with
`&content[..80]`, and byte 80 landed inside an em dash **in nanna's own `edit_file` error
text**. Self-inflicted; `context.rs:1827` carries the same defect at `..100`. Same class as
the 2026-08-10 `&text[..200]` distillation panic, recurring in a new file — and the
codebase uses em dashes freely in tool errors, so the input is not exotic. The P22 Tier 4
**exit-reason file named the cause instantly**, which is the only reason this is a
one-line diagnosis instead of an unexplained disappearance.

Levers observed working: MissionEnd (one cumulative line, honest counts —
`items_completed=0 items_abandoned=35`, no pretence of success); repeat-done escalation
(fired on both lfm2.5 repeats, counter 1 then 2, each stated in the transcript);
structural shrink holds and byte-floor refusals (4 blocked destructive writes on ornith
alone); truthful tool acks (**zero** phantom python-registry saves all series, against 201
in the post-P22 gemma leg); read-only ladder held in every leg.

Ledgers, per-poll history, per-leg summaries: `D:/Development/nanna-bench/p23-run-*/`.

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
