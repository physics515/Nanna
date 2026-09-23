# Nanna v0.3.28-beta.37 — Strictest Lints, No Exceptions

Every part of Nanna now builds under Clippy's strictest lint groups with no suppressions
anywhere, and every number conversion that used to lean on a lossy cast now goes through one
small crate whose results are proven exact.

## What's Changed

**No lint exceptions remain.** Every crate, test, benchmark and build script is checked with
`clippy::pedantic`, `clippy::nursery` and `clippy::all`, and the twenty places that used to
carry an `allow` or `expect` have been rewritten instead of silenced. Nothing you see changes;
what changes is that every future warning is a real finding rather than noise beside an
exception.

**Number conversions are exact by construction.** Scores, ratios, averages and size estimates
used to be converted with `as` casts hidden behind lint expectations. They now go through
`nanna-numeric`, which produces the same value for every input but is built from lossless
pieces and pinned by an exhaustive oracle, so a rounding or saturation rule is stated once and
tested rather than assumed at each call site.

**Process cleanup keeps its contract on every platform.** Killing a tool's process tree still
takes one call on Windows, Linux and macOS; the Unix path no longer pretends to be
asynchronous when it is a single signal.

---

Updating from 0.3.26 or earlier? The
[0.3.27 release notes](https://github.com/physics515/Nanna/releases/tag/v0.3.27-beta.36) cover
keys from your environment never being copied into your secure store, and link to 0.3.26's.
