# Nanna v0.3.17-beta.26 — Your Data, Readable

This release lets you take a conversation — or everything Nanna remembers — out as a document you can
read, and shows you what each edit did while you were away. Underneath, it fixes a transport limit that
silently dropped large replies to the CLI, and it finishes Linux: the desktop app now builds there.

## What's New

### `nanna export` — conversations and memories as documents you own

```bash
nanna export <session-id>                    # readable Markdown transcript
nanna export <session-id> --format json      # the complete stored session, lossless
nanna export --memories [--scope global]     # what Nanna remembers, with provenance
```

The daemon renders the document — it owns the data — so every client gets the same one. The Markdown
transcript follows the chat page's own layout: thinking, tool calls with their input and output, each
edit's before/after view, and the reply, which is neither dropped nor printed twice. Code fences are
sized so a tool output that quotes Markdown cannot break out of its block. JSON is the stored record in
a versioned envelope, proven to load back.

A memory export carries each memory's text, where it came from (`unknown` when that was never recorded —
never a guessed "you said so"), its workspace and its full FSRS state, and **no embedding vectors**:
they are derived data, recomputed by a re-embed, and most of the bytes. The daemon must be running;
`nanna sessions` lists the ids.

### See what each edit did

Every `edit_file` call now records a bounded before/after view of what it changed — the lines, where
they start, and whether the view was cut. It shows in the run timeline, and it is **kept with the
session**, so it is still there when you open the session after an unattended run, or after a restart.
Only an edit that succeeded and actually ran carries one.

### The desktop app builds on Linux

`nanna-gui` did not build on Linux at all: `tauri-build 2.6.3` walks a fixed three directories up from
its build output, and current cargo nests that output one level deeper, so the sidecar copy landed on a
directory and the build panicked. This release carries the upstream fix (tauri#15831, due in
`tauri-build` 2.7.0) as a vendored patch with a test that retires it with the next Tauri release, and
CI now compiles the GUI on Linux too.

## What's Fixed

- **The CLI dropped any daemon reply over 16 MiB.** A WebSocket read limit protects only the side that
  sets it. The daemon had raised its own to 128 MiB, but `nanna-client` still read with the 16 MiB
  default, so a long session's history — or a large export — arrived as a dropped connection.
  Reproduced with a 20 MiB reply before fixing; one shared limit now, applied at both ends, with a
  guard test.
- **`nanna server` ignored the port you chose.** `--port` defaulted to 3000 regardless of
  `[server].port`, so the onboarding answer, the documented config key and the `PORT` variable were
  all discarded. The flag still wins; otherwise your configured port is used.
- **`[server].host` is gone.** Nothing ever read it — `nanna server` binds `--host`, loopback by
  default — so it looked like a security setting while controlling nothing. Existing config files that
  still carry it load unchanged.
- **The long-run journal disagreed with itself.** Unattended task runs stored tool output uncapped in
  their run record and could overwrite an earlier call's result when a model reused a call id; they now
  share the chat path's journal writer.
- **Webhook text no longer reaches the agent in your voice.** A generic webhook authenticates a caller,
  not you; its payload is now framed as external data the agent must not take as your instructions.
- **Database migrations are split by a lexer that knows comments and quotes.** They used to be split on
  every `;`, so a semicolon inside a SQL comment would have cut a statement in half. That would not show
  on an existing install, but would fail on a fresh one. Only a test kept that from happening. Every
  existing migration is proven to run exactly as before.
- **`nanna export <id> | head` no longer crashes** when the reader stops early. A message that leaves a
  code block open no longer swallows the rest of a Markdown export either.
- **Cost estimates now use current Claude prices.** Several were out of date:
  - Sonnet 5 was reported 50% too high.
  - Opus 4 and 4.1 were reported at a third of their price.
  - Fable 5.1 cache reads were 4× too high.
  - Mythos 5 was priced as Sonnet.

  The table now matches Anthropic's published rates, checked 2026-09-11.

## Also In This Release

- **Recall reports its two stages separately.** Embedding the query (a model call) and searching (an
  in-memory scan) are timed on their own, logged once per recall and returned by `memory.search`; a
  slow embed no longer hides behind a fast search.
- **How long new memories stay unfindable** is now measured: the time from a memory being written
  without a vector to receiving one, reported as p50/p95 in `memory.stats`.
- **`nanna doctor`** now warns when chat and summarization point at two different Ollama servers — a
  split that is easy to create by setting only one of the two keys — and **`nanna doctor --online`**
  asks each Ollama server in use whether it is answering and has the models you configured, naming
  any that need an `ollama pull`. It never reads the keyring or tests a provider key.
- **1-hour prompt caching** for Anthropic (`[llm].prompt_cache_ttl = "1h"`), priced correctly at the
  API's own 2x write rate.
- **Session events reach every client**: a rename or delete from one window now updates the others,
  and a client that falls behind is told how many events it missed instead of silently losing them.
- **Typed `nanna-client` APIs** for the scheduler, workspaces and channels.
- **GUI type-check at zero errors** (22 were left, five of them hiding real bugs), now gated in CI.
- **Dependencies** swept to latest; the `libc` and `malachite-bigint` ceilings still stand, enforced by
  tests. The exact `aegis` pin is retired — `turso` already builds it pure-Rust.

## Known Issues

- **No in-app GUI verification on Linux yet.** The app builds, but the WebDriver harness needs
  `WebKitWebDriver` (`webkit2gtk-4.1`), which is not installed on the build host. The new edit-diff view
  is covered by unit tests only.
- **Exporting from the app** is not there yet — `nanna export` is the way today.
- **Two config keys name the Ollama server** (`[memory].ollama_host` for chat and embeddings,
  `[llm].ollama_url` for summarization). `nanna doctor` flags a mismatch; which key should win is an
  open decision.
- **`[server].enabled` and the Agent tab's personality selector are read by nothing.** Both are open
  decisions rather than silent fixes.
