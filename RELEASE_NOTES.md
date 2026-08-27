# Nanna v0.3.11-beta.20 — True in the Code, False in the World

Five fixes, two of which no test could have caught because the code was self-consistent and wrong
about the outside world. The other three turn standing "remember to do X" notes into checks that
fail in milliseconds.

## What's New

### Scheduled work reached a model that no longer exists (this release)

Every install that had never configured `llm.model_priority` sent its scheduled heartbeat to
**`claude-sonnet-4-20250514`**, a retired model snapshot, and got a `404 not_found_error` back. The
daemon reported itself healthy throughout — the failure was logged at `WARN` and swallowed, so the
only visible symptom was that scheduled work silently never happened.

The shipped default is now **`claude-sonnet-5`**, the live same-tier alias. Six defaults carried the
dead id (`nanna-config`, `nanna-core` ×2, `nanna-agent`, `nanna-llm`, `nanna-server`) and all six
moved together.

The general lesson is in the fix, not the id. Anthropic publishes an **undated family alias** that
tracks the live model and a **dated snapshot** pinned to one release and retired on a schedule. A
dated *default* ships with an expiry date built into it. Pinning a snapshot is a perfectly good
choice for a user to make in their own config — it is not a good default for the product to ship, and
a new test refuses to let one become the default again.

Verified on the real binary, not argued: booting the release daemon against a scratch config went
from `3 ×` `API error: 404` and `2 ×` "All models exhausted" to **zero of each**, with a completed
run — `model=claude-sonnet-5, duration_s=5, tool_calls=2, faults_healed=0`.

**If you have a model pinned in your own config, nothing changes for you.** This only affects installs
running on the built-in default.

### Steering no longer looks like breakage after a reload

When the zero-information breaker answers a repeated tool call itself, the timeline renders it as
*steering* — the tool never ran, so nothing failed. That was true only while the run was live. A
timeline **rebuilt after navigating away and back** showed the same calls as a wall of red tool
errors, because the run journal recorded the outcome (`success: false`) without recording *why*.

The journal now carries the replay marker beside the outcome, so a restored timeline reads the same
as the live one. The marker is additive on the wire: journals written before this release load
unchanged.

Also fixed one layer down — a trimmed replay in a crash-recovery checkpoint used to be labelled
"the call failed". It now says the tool never ran.

### A panic under the journal lock could take the rest of the run with it

The two writers to a run's journal disagreed about what a poisoned mutex means. The chat path had
always treated poisoning as survivable, with a stated reason: a panicking thread must not erase the
run's record. The **harness** path — writing to the very same journal — panicked instead, at five
places.

So one panic anywhere under that lock turned every later text delta, tool call and step of that run
into another panic, inside a spawned turn where a panic is invisible and the run simply stops. That
is the shape of a bug that once read as a mysterious wedge for a day. Both writers now share one
policy, and a test poisons the lock from a real panicking thread to prove the record survives and
later writes still land.

### Memory: the forgetting curve's decay constant was off by one slot

The FSRS decay exponent was `0.0658`, believed to be FSRS-6's published default. It is
FSRS-6's **`w[19]`**. The decay is `w[20]` = **`0.1542`** — confirmed against the reference
implementation, which also clamps that parameter to `0.1..=0.8`, a range the old value sat *below*.
No fitted parameter set could have contained it.

The correction is deliberately gated so it cannot re-break what the previous fix bought: the
retention harness now measures aged recall at **all three** exponents the default has held and
asserts the corrected value recalls exactly as much as the misread one — not merely "enough". At the
practical extreme (800-day-old memories) both clear the recall gate comfortably; what actually
differs is which consolidation band an aged memory lands in, which is why matching the published
curve is the whole point.

### Dependency freshness

`uuid 1.26`, `which 8.0.6`, the tiptap suite at `3.30.5`, `vue 3.5.42`, `happy-dom 20.11.8`, and the
Rust toolchain moved to `nightly-2026-08-27`. TypeScript 7 was attempted and reverted for the third
time — `vue-tsc` cannot run on it until TypeScript 7.1 exposes a programmatic compiler API, which is
an upstream constraint affecting Vue, Angular and ESLint alike.

Two dependency pins that previously lived only as a note ("remember to redo this after every
update") are now enforced by a test that runs in **0.00 seconds** and prints the exact fix command.
It was verified against the real regression rather than assumed.

## Fixed

- Scheduled heartbeats failing with `404` on every unconfigured install.
- Breaker replays rendering as tool failures in any timeline restored after a reload.
- A single panic under the run-journal lock cascading into a stalled run.
- FSRS forgetting-curve decay using `w[19]` where `w[20]` was meant.
- A crash-recovery checkpoint describing a trimmed replay as a failed call.

## Known / still open

- **A heartbeat that fails on every attempt is still only a `WARN`**, and the daemon still reports
  itself healthy while it happens. A model that fails *every* time is a configuration fault, not a
  transient one, and deserves to surface. Where operator-visible faults belong is not yet decided.
- The `malachite-bigint` and `rten` version pins remain, both waiting on upstream
  (`rustpython-codegen` accepting malachite 0.10; `ocrs` moving to `rten 0.25`).
- TypeScript 7 stays deferred until 7.1 or a tsgo-backed `vue-tsc`.

## Verification

1691 workspace tests green · clippy clean with no new warnings in any changed file · release build
green on the new toolchain · frontend typecheck clean with 237 tests · release daemon booted against
a scratch config with a before/after log diff proving the model fix.
