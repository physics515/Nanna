# Nanna v0.3.23-beta.32 — Measuring Before Building

This release is mostly about the memory system, and mostly about deciding things with numbers
instead of with arguments. Three long-standing questions in the roadmap were each waiting on a
measurement that nobody had taken. All three are now answered, and two of them turned out to have
an answer nobody expected.

Recall got faster. Dreaming got faster. Neither is a rewrite — they are the cheap wins, taken
deliberately so that the expensive one can be judged on evidence rather than intuition.

## What's Faster

**Recall stopped ranking your whole memory store to return ten results.** Every time Nanna recalls
something — and she recalls on every ingest, not just when you ask — she scored every memory and
then **sorted all of them** before keeping the top handful. At fifty thousand memories that is
sorting fifty thousand things to keep ten. It now selects rather than sorts, which is **5-18% off
the entire recall path**, cosine included.

That change came with a quieter correctness fix. The ranking relied on sort stability to break
ties, and ties are ordinary — every memory that cannot be compared scores identically. It now
orders ties explicitly, so repeated recalls return the same memories in the same order. And a
memory with a degenerate embedding used to produce a score that was *not a number*, which the old
comparator treated as "equal to everything" and could seat anywhere in your results, including
first. It now ranks last, where it belongs.

**Dreaming got about a third cheaper on a long-lived store.** Two passes, both of which leave the
result bit-for-bit identical:

- The similarity kernel was recomputing each memory's magnitude for every pair it looked at, inside
  a loop that looks at every pair. Those are now computed once. Worth 11-18%.
- Pairs that **cannot possibly** be close enough to merge are now rejected before Nanna reads
  their embeddings at all. On a store spread over months this skips **21.9% of all comparisons**,
  for another 16-22%.

The second one is exact rather than clever: it computes the best score a pair could achieve with a
perfect match, and skips only when even that is not good enough. Nothing that would have merged is
missed, and the test suite proves it by dreaming the same store twice, with and without the
shortcut, and requiring identical results.

## What We Learned, Including Where We Were Wrong

**Keeping memories in RAM beats querying them from the database, by 9-14x.** Nanna can search
memories with an exact database query that uses almost no memory, and it was an open question
whether that should become the normal path. It should not: the in-memory scan is between nine and
fourteen times faster, and the cost of loading memories in the first place pays for itself after
**two or three searches**. The database path stays for stores too large to hold in memory, and we
now know the size where that starts to matter — around 350,000 memories, roughly seven times
today's practical ceiling.

**Making the arithmetic cheaper does not fix a quadratic, and we measured how little it helps.**
Removing two thirds of the similarity kernel's arithmetic was expected to be worth two to three
times. It was worth thirteen percent, because the work is limited by memory bandwidth, not by
arithmetic. Recorded plainly, because it closes off a whole direction: for a store of half a
million memories, every cheap optimization available — a third of the arithmetic, a fifth of the
comparisons — moves a dream cycle from about 73 minutes to about 47. The next step has to be
comparing fundamentally fewer pairs, not comparing them faster.

**Thirteen of Nanna's twenty-one memory-decay parameters do not do anything.** They are public,
they are saved to your config, several are non-zero, and they look exactly like tuning knobs.
Eight of them are wired to a formula; the rest are a table inherited from a published algorithm
whose update rules Nanna does not use. This is now proven by a test that perturbs each one and
checks whether anything moves, so the documentation cannot quietly go stale — and the module no
longer claims to be an implementation of an algorithm it only borrows one curve from.

## Under the Hood

- **Toolchain moved to nightly-2026-09-20**, with the full gate re-run under it.
- **Clippy warnings halved** (21 → 11) by taking a newer lint's rewrites across 22 sites. Two of
  its suggestions were rejected for being less readable than the code they replaced.
- **Dependencies refreshed**: 8 Rust crates, 2 GUI packages. TypeScript 7 remains blocked upstream.
- **2,178 tests pass**, up from 2,167.
- The dreaming quality baseline — 0.90 compression at 1.000 recall — is **unchanged**, which is how
  we know the speedups above cost nothing in what Nanna actually remembers.
