# Nanna v0.3.12-beta.21 — Signals That Mean Something

A thumbs-up in Slack has been feeding Nanna's long-term memory for a while. So has a 🍕, a party hat,
and a 💔 — all of them as *praise*. A PDF tool the README has been advertising turns out to have
failed at every call. And the nightly routine's own test gate was, for at least one run, reporting
green about a build it had not actually made.

**This release also carries everything prepared for v0.3.11-beta.20, which was never published.**
That version was cut into `master` and its build never dispatched, so its content ships here and its
notes are folded in below rather than lost.

## What's New

### A reaction that isn't feedback no longer counts as praise

Slack `reaction_added` events already reached the memory system — that path has been live in
`nanna serve` all along. What it did with them was the problem: the classifier matched **substrings**
and **fell through to "positive"** for anything it did not recognise.

So any unrecognised emoji applied a `Helpful` signal (+0.3 to the FSRS weight) to **every** memory the
session had stored in the last ten minutes. A 🍕 on an answer promoted the whole batch. And because
matching was by substring, `broken_heart` contains `heart` — so 💔 **promoted** too, the exact
opposite of what the person meant.

Reactions are now classified into three states rather than two: approval, correction, or **not a
feedback signal at all**. Nothing happens for the third, which is the honest answer to an emoji
nobody assigned a meaning to. Matching is exact, after Slack's `::skin-tone-N` modifier is stripped —
`thumbsup::skin-tone-6` previously matched nothing and fell through to the default.

### The bookkeeping behind that feedback was growing without a ceiling

Two maps sit between a reaction and the memories it credits, and both were unbounded in practice.

The memory map had two writers: one capped at 50 entries per session, and an inline copy with no cap
at all — and the uncapped one is the one that actually runs. Its ten-minute recency filter only ran
when somebody reacted, which on the common path nobody does, so a long-lived daemon grew it for the
lifetime of the process.

The message-link map was worse in a quieter way: its cap discarded **five thousand arbitrary entries
at once** in hash order, so the message a person was about to react to was exactly as likely to be
dropped as one from an hour earlier — and nothing ever expired, so it grew with uptime rather than
with traffic.

Both now prune by the same ten-minute attribution window, on write. That window is not a tidiness
rule: an entry past it can only ever resolve to memories that have already expired, so dropping it
loses nothing anything could have used. The caps survive as burst backstops, and the link map now
evicts the oldest entry rather than a random handful.

### `read_pdf` works

The bundled `read_pdf` tool declared a `pdf.read` service that was never registered, so every call
returned *"PDF reading service not available"* — while the Rust extractor behind it sat complete and
tested, and the README listed PDF among the shipped tools. It is registered now.

Registration alone would have shipped it broken in a quieter way. The tool has always been sent a
page range (`"1-5"`, `"3"`) and has only ever read an integer page *count*, so asking for page 3 of a
forty-page contract would have returned all forty with nothing saying the request had been dropped —
a wrong answer that looks like a right one, which is a worse failure than a tool that plainly does
not work. `pages` is now honoured, and refuses what it cannot read rather than guessing: `"5-2"` is
an error, not a silently reversed range.

It also gained the 10 MB ceiling `read_file` already applies, so reading a PDF is no longer a way in
for a file the plain file reader turns away, and it reports `page_count` / `pages_read` beside the
text so a caller can tell it did not get the whole document.

### The nightly routine was grading its own homework on the wrong desk

Every project on this machine shares one Cargo target directory. A neighbouring project had built a
dependency there under a *different* Rust nightly, and the test run reported **1697 passed, 0
failed** — then died compiling doctests with `found crate compiled by an incompatible version of
rustc`. Nothing in Nanna was wrong. The gate was.

That matters more than the wasted minutes: a run that cannot tell a genuine failure from a
contaminated one has not earned any of the numbers it reports. The routine now pins its own target
directory before the first build, and every figure below was re-earned from a cold one.

### Dependency freshness

`wide 1.7`, `deno_core 0.411`, and six compatible bumps. On the frontend, `@lucide/vue 1.35`,
`vue-router 5.3`, `@vue/test-utils 2.5`, `happy-dom 20.11.12`.

A guard added last release, to stop two version pins from being a per-run habit, did its job — it
caught the `malachite-bigint` split in 0.00 seconds instead of twenty minutes into a release build.
But its printed fix command named a version no longer in the graph, so running it would have failed.
The command is now derived from what the lockfile actually holds.

## Also in this release (prepared for v0.3.11-beta.20, never published)

- **Scheduled work reached a model that no longer exists.** Every install that had never configured
  `llm.model_priority` sent its heartbeat to a retired snapshot and got a 404 back, logged at `WARN`
  and swallowed — so scheduled work silently never happened. The default is now the live family
  alias. If you have a model pinned in your own config, nothing changes for you.
- **Steering no longer looks like breakage after a reload.** A timeline rebuilt after navigating away
  showed breaker replays as a wall of red tool errors, because the journal recorded the outcome
  without recording why. Journals written before this release load unchanged.
- **A panic under the run-journal lock could take the rest of the run with it.** The two writers to
  the same journal disagreed about what a poisoned mutex means; they now share one policy.
- **The FSRS forgetting curve's decay constant was off by one slot** — `w[19]` where `w[20]` was
  meant, a value that sat below the reference implementation's own clamp.

## Fixed

- Unrecognised Slack reactions promoting every recent memory; `broken_heart` promoting rather than
  demoting; skin-toned reactions matching nothing.
- Two unbounded maps behind reaction attribution, one of which also evicted at random.
- `read_pdf` failing at every call, and silently ignoring the page range it was sent.
- `read_pdf` having no input size ceiling where `read_file` has one.
- A dependency guard printing a fix command that no longer applies.
- (from 0.3.11) Scheduled heartbeats 404-ing on unconfigured installs; breaker replays rendering as
  failures in a restored timeline; a journal-lock panic cascading into a stalled run; the FSRS decay
  exponent.

## Known / still open

- **Reactions are the only feedback signal wired.** Corrections and tool success/failure — the
  `UsedSuccessfully` / `CausedError` half of the model — are still unfed.
- **Telegram and Discord cannot deliver reactions today.** Telegram needs `message_reaction` named in
  `allowed_updates` plus admin rights in the chat; Discord needs the Gateway, which this daemon does
  not run.
- **`read_pdf` has no OCR fallback yet.** The hook exists and is tested, but the OCR pipeline it
  would call is itself unregistered. Image-only pages come back empty and say so, rather than
  pretending. Two other built-in tools are complete, exported and unreachable in the same way.
- A heartbeat that fails on *every* attempt is still only a `WARN`.
- The `malachite-bigint` and `rten` pins remain, both waiting on upstream.
- TypeScript 7 stays deferred until 7.1 or a tsgo-backed `vue-tsc`.

## Verification

1714 workspace tests green with doctests included, from a cold, uncontaminated target directory ·
release build of the daemon green on `nightly-2026-08-27` · clippy with no new warnings · frontend
typecheck clean, 238 tests, production build green with four routes prerendered.
