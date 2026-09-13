# Nanna v0.3.18-beta.27 — Code That Was Never Compiled

Four `.rs` files in this repository had never been compiled. Not "unused" — *uncompiled*: no `mod`
declaration named them, so `cargo check`, clippy, `cargo fmt` and the test suite all walked past
them. And separately, on Linux and macOS, a development build of Nanna loaded **none** of its 43
JS/TS skills, because one path was assembled with a Windows separator.

Both are the same failure mode. Something reads like shipped behaviour and is not, and nothing in
the build says so. This release fixes both and makes each class fail loudly next time.

## What's Fixed

### 43 skills were missing from every debug build off Windows

Nanna's tools *are* JS/TS skills on disk. Release builds extract an embedded copy; debug builds read
the source tree. The constant naming that source tree was:

```rust
concat!(env!("CARGO_MANIFEST_DIR"), "\\default-skills")
```

which is a path only on Windows. Everywhere else a backslash is an ordinary filename character, so
the constant named a file that has never existed. Measured before the fix:

```
DEV_TOOLS_DIR       = Some(".../crates/nanna-tools\default-skills")
is_dir()            = false
resolve_tools_dir() = None
```

A developer daemon on Linux with no `NANNA_TOOLS_DIR` and no `[tools].tools_dir` therefore ran with
the Rust built-ins and nothing else, while the same commit on Windows loaded all 43 skills. And it
was worse than merely missing: the same resolution feeds `register_discover_tools`, which skipped
silently — so the system prompt went on telling the model it has `discover_tools`, one of the four
tools it names, while the tool was not registered.

The fix is to *join* rather than concatenate a separator, which is correct by construction on every
platform. The branch that used to return `None` in silence now warns and names the two ways out.
Two tests cover it, both verified to fail on the old line and pass on the new one, and both written
against `is_dir()` rather than the spelling of the path — so they are the same test on Windows,
Linux and macOS.

### Four files that had never compiled, and a guard so it cannot happen quietly again

`src/daemon_launcher.rs`, `src/updater.rs`, `src/webview2.rs` and
`crates/nanna-gpu/src/batch_processor.rs` — 32 KB — were reachable from no crate root. Declaring the
modules and building produces **12 compile errors**: an unresolved `tauri_plugin_updater` import in
a crate that has never depended on it, a `std::process::Output` assigned to an `Option<Child>` (with
`Command::output()` used to launch a daemon, which would block until the daemon exited), a call to a
`regex_pattern` function that does not exist, and `parking_lot` in a crate without it.

The cost was never the bytes. It is that each file reads like an answer: `daemon_launcher.rs` looks
like the sidecar launcher, `updater.rs` looks like the updater, `batch_processor.rs` re-proposes
batch sizing the compiled `BatchedSearch` already does. All four are deleted; git history is the
record.

The durable half is a guard. A new test walks `mod` declarations from every workspace package's
roots and fails if any `.rs` under that package's `src/` is unreachable, naming the files. It is
verified in both directions — it named exactly those four before the deletion, and it fires on a
freshly planted orphan — and it ships with three parser tests, because a guard whose matcher quietly
stops matching is a test that always passes.

## Dependencies

`playwright-rs 0.17 → 0.18` (compiled unchanged) plus 30 lockfile bumps, including `ocrs 0.13.1`
published the same day. GUI: `@lucide/vue 1.45.0`, `marked 18.0.13`, `tailwind-merge 3.7.0`,
`happy-dom 20.14.5`.

One process correction came out of the sweep: the standing rule was "re-apply the held pins after
every `cargo update`", and that is not sufficient — `cargo upgrade --incompatible` re-resolves the
lockfile too, and silently walked `libc` back above its ceiling after the pin had been applied. The
pin-backs have to be the last lockfile operation of a sweep. `rustpython` has now published nothing
since 0.5.0 in March, so both holds it forces remain, enforced by a test rather than by memory.

## Still Open

- **GUI verification on Linux remains unavailable.** The WebDriver harness needs `WebKitWebDriver`,
  which on current Arch ships in **`webkitgtk-6.0`** — not `webkit2gtk-4.1`, which is what Tauri's
  docs, the WebdriverIO Tauri service and the harness itself all advise, and which is already
  installed here without the binary. The install is owner-gated, so nothing in this release was
  verified through the real app.
- The `UsedSuccessfully` / `CausedError` memory-feedback signals still have no producer anywhere in
  the product. The blocker is plumbing rather than design, and the intended attribution is now
  written down.
