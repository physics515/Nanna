# gui-leg — the GUI-path endurance bench driver

Drives ONE 4-hour benchmark leg through the **real chat path** (daemon + IPC, the
same path the GUI uses): bind a model at runtime, open a fresh workspace seeded
with the read-only 42-test ladder, send the minidb mission, and poll for four
hours. The committed version of the scratchpad harness that ran the 2026-08
GUI-path campaign — hardened against every way that campaign lost numbers
(ROADMAP **P22**; evidence: 50h of wall-clock, ~8 trustworthy hours).

## Contract

A leg ends in exactly one **terminal verdict** (`run/verdict`, first writer wins):

| Verdict | Meaning |
| --- | --- |
| `VALID` | full window, daemon live at every recorded poll, preflight held |
| `VALID_WITH_CAVEATS(...)` | scoreable, but named conditions differed (gpu-shared, embedding-degraded, daemon-restarted, tests-modified) |
| `INVALID(reason)` | the number cannot be trusted — **no score is emitted**. Reasons: daemon-unreachable, ctx-gate/ctx-demoted, gpu-not-exclusive, unreachable-polls-N, heartbeat-stale, worker-died, supervisor-timeout |
| `ABORTED(reason)` | the leg never reached its window (preflight/lock/boot failure) |

Only `VALID*` legs may contribute a number to [`../BASELINE.md`](../BASELINE.md),
recorded **with the work denominator** (see AGENT_EVAL "Updating scores").

Mechanisms, mapped to the campaign failure they close:

- **Immutable legs** — `leg.sh` stages itself + helpers + ladder into
  `run/driver/`, records each sha256 in the ledger header, and execs the copy.
  (Leg 4: script edited mid-run, bash re-read it at a byte offset, a completed
  4-hour result died at `unexpected EOF`.)
- **Liveness-gated scoring** — every recorded poll is preceded by an IPC
  `system.status` probe. Probe fail → the poll is `UNREACHABLE`, nothing else is
  recorded. 3 consecutive fails → `INVALID(daemon-unreachable)`. More than
  `GUI_LEG_MAX_UNREACHABLE_POLLS` (default 2) unreachable polls → the summary
  refuses the score. (Ministral: daemon dead 3h58m, 14 ws errors ledgered, an
  official 0/42 published anyway.)
- **Work denominator** — per poll: tool calls, side-effecting calls, tokens
  in/out, queue depth (IPC `session.get_run_state {light:true}`), plus anchored
  log greps for `Executing tool` counts and `stop=` reasons, plus daemon uptime.
  (1121 / 1193 / 992 / 30 / 28 tool executions in nominally identical windows,
  nothing in the record saying so.) The anchored greps are the documented
  fallback until the T4b session-liveness verb lands.
- **Artifact history** — per poll: artifact copy + scored verdict into
  `run/history/<minutes>/`; summary reports **peak, time-of-peak, final**.
  (Qwen's 22/42 peak existed nowhere on disk; only post-collapse wrecks
  survived.)
- **Worker/supervisor** — the worker heartbeats and appends a machine-readable
  status line (`run/status.jsonl`) every tick (60 s); `supervisor.sh` is a
  separate process that fails the leg loudly on stale heartbeat, persistent
  worker-reported unreachability, a dead worker without a verdict, or lifetime
  overrun. (Leg 4's death went unnoticed 19h35m; gemma's hang was caught by a
  human.)
- **Resume contract** — a resume is an **interjection** (never cancel+resend),
  fires **only** when the probe says the run is not live, records whether the
  next poll showed change (`run/resumes.jsonl`), and stops repeating after
  `GUI_LEG_MAX_INEFFECTIVE_RESUMES` no-change resumes. (Cancel+resend regressed
  13→9/42; an interjection took ABSENT→18/42; fourteen resumes were once fired
  into a dead socket.)
- **Preflight, start AND per poll** — exclusive GPU (ollama residents must be
  the leg model), pinned `num_ctx` (hard abort on any demotion — reads only
  past the leg's log anchor), embedding-store health (recorded in the ledger
  header either way; degraded at start aborts).
- **Captured daemon output** — `run/daemon.out|err` via Start-Process
  redirection. (The ministral crash is unexplainable because nothing captured
  the daemon's last words.)

## Usage (bench machine)

```bash
bash bench/gui-leg/leg.sh ornith:9b ornith-9b
```

One leg per invocation; a PID lock (`$BENCH_ROOT/gui-leg.pid`) refuses overlap.
Never run legs in the background of anything (see the benches-never-in-background
rule); the run dir is self-contained under `GUI_LEG_BENCH_ROOT`
(default `D:/Development/nanna-bench/run-<slug>-<stamp>/`):

```
run-<slug>-<stamp>/
  driver/           immutable staged copy (hashes in ledger header)
  ledger.txt        header + timestamped narrative
  workspace/        the mission's cwd (tests/ read-only)
  history/<min>/    per-poll: minidb, verdict.txt, denominator.json, log-counters.txt
  status.jsonl      one machine-readable line per 60s tick
  heartbeat         epoch seconds, watched by the supervisor
  resumes.jsonl     each resume + whether the next poll changed
  daemon.out|err    the daemon's captured stdio
  session.id        the mission's session
  verdict           terminal verdict (exactly one)
  summary.txt       peak / time-of-peak / final / denominator / caveats
  final/            end-of-window artifact
```

Every knob is a bounded-default `GUI_LEG_*` env var — see the header of
[`leg.sh`](leg.sh). The 42-test ladder ([`ladder-42/`](ladder-42/)) is tracked
as data: **do not edit it** — scores across the series are only comparable
against the identical ladder (the ledger header records its combined hash).

## Self-tests (CI-safe: no GPU, no model, no Rust)

```bash
bash bench/gui-leg/self-test/run-self-tests.sh
```

Scorer oracle (a known-good `reference-minidb` must score 42/42; an empty stub
0/42 with every ladder test individually failing it), `bash -n`/shellcheck,
supervisor units (stale heartbeat / dead worker / clean exit), and two full
dry-run legs against `self-test/fake-daemon.mjs` (healthy → `VALID` with
peak+final; daemon-dies-mid-leg → `INVALID(daemon-unreachable)` with the score
refused). Wired into CI as `gui-leg-selftest.yml` (ubuntu + windows). The
ubuntu job also proves the reference implementation and ladder are POSIX-sh
clean (dash), not just Git-Bash clean.

Requires node ≥ 21 (global WebSocket client) — CI pins 22.
