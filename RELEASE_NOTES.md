# Nanna v0.3.19-beta.28 — Things That Said They Were Working

This release is about a single failure shape, found five times in one night: something reports that
it is fine, and it is not. A skill catalogue nothing ever parsed. A validator that validated nothing.
A diagnostic that said `ok` for a configuration its own daemon had already given up on. An error
message that named everything except the cause. Each one passed every test in the repository.

It also folds in three nightly runs that had been waiting on review (#318, #319, #320), so one merge
carries all of them.

## What's Fixed

**8,848 lines of shipped tool code that nothing ever parsed.** Nanna's 44 bundled skills are
JavaScript. The only thing that read them was a manifest extractor that scans the source as *text* —
it never parsed the code. Planting `var lines = [;;; this is not javascript at all (((` into a
skill's body left **295 tests passing**. That skill would have shipped, been offered to the model
with a perfectly valid input schema, and failed for the first time inside your session. Every
bundled skill is now parsed in the engine that runs it, including the module rewrite and wrapper the
runtime applies.

**"Validate the source compiles" compiled nothing.** When Nanna writes a tool for itself, the
validation step was a constructor call whose result was thrown away on the next line. A tool with
unparseable garbage in it was written to disk, registered, and advertised — and the first sign of
trouble was a failed call several turns later. Creating or editing a tool now actually parses it and
refuses with the engine's own line and column, leaving nothing behind.

**A release build could ship zero tools and compile clean.** The build script that embeds the skill
catalogue fell through to an empty array if it could not find the directory. That is the build users
get. It now fails the build instead, and a test checks the shipped catalogue against the source tree
in both directions.

**An expired login blamed your model name.** When your Anthropic credential expires, every request
failed with `No provider for model: claude-sonnet-5 (detected: Anthropic, available: [Ollama])` —
which reads as "you typed the model wrong" or "Anthropic support was dropped". The real cause was
logged once, at startup, and thrown away. It is kept now, so the failure says
`— a stored OAuth login exists but could not be used (...)`, and `nanna doctor --online` reports the
expiry directly with the fix attached.

**`nanna doctor` said `ok` for an embedder the daemon skips.** It checked that a provider was
*named*, not that it could be *reached* — so the shipped defaults passed on any machine without an
OpenAI key, while the daemon was already saying memory would run without vectors and recall would be
unavailable. The doctor now reproduces the daemon's own resolution and splits the verdict: a missing
key is a warning you can fix, a typo is a failure no key will help.

## What's New

**`find_files`** — find a file by name when you know what it is called but not where it lives.
`*.rs` finds every Rust file at any depth; `src/**/*.rs` anchors to a path. Sizes included, and if
the tree is too large to walk completely it says so plainly rather than reporting "no matches" for a
search that never finished.

## Also

- The desktop app **builds and runs on Linux** for the first time on the maintainer's host — window,
  tray, daemon sidecar, clean shutdown.
- Dependency sweep: 5 Rust bumps, 2 GUI bumps, both guarded pins held.
- Benchmarks re-run: no measurable cost, every budget holds with 5x or more headroom.

## Still Open

Anthropic OAuth **token refresh** is broken — the request shape is wrong and the endpoint answers
400. The wire format is undocumented and a test attempt would spend your refresh token, so this
release makes the failure loud and legible rather than guessing at a fix. If your login expires,
re-mint it with `claude setup-token`; `nanna doctor --online` will tell you when that is the problem.

GUI WebDriver testing on Arch Linux remains unavailable — the only packaged `WebKitWebDriver` is the
wrong WebKitGTK generation for the app. Notes in `ROADMAP.md`.
