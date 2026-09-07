# Nanna v0.3.13-beta.22 — It Never Built Here

The project moved to Linux in September. This release is the discovery that it had never actually
compiled there — and the two-line class of mistake that hid it.

## What's New

### The workspace did not build on Linux at all

`origin/master` was red on Linux before this branch touched anything. Not "had warnings", not "failed
a test" — `cargo build --workspace` did not produce a binary. Two independent causes, both invisible
from Windows, and neither one reachable by any amount of testing on the platform CI runs:

**1. A runtime `cfg!` where a compile-time `#[cfg]` was meant.** The `exec` tool picks its shell with

```rust
let mut cmd = if cfg!(windows) { /* Git Bash / PowerShell routing */ } else { /* sh */ };
```

`cfg!(windows)` is a *boolean expression*, not conditional compilation. It selects the right branch at
runtime, so the behaviour was always correct — but **both arms are still type-checked on every
platform**, and the four helpers the Windows arm calls (`strip_outer_quotes`, `classify_windows_command`,
`git_bash_path`, `WinShell`) are `#[cfg(windows)]` and simply do not exist on Linux. Five
`E0425`/`E0433` errors, and `nanna-scripting` — hence the `nanna` binary — could not compile.

Split into `#[cfg(windows)]` / `#[cfg(not(windows))]` bindings so the Windows arm is compiled out.
**Nothing changes on Windows:** the same branch is chosen, just earlier. Net −7/+5 lines.

**2. `libc 0.2.187` broke the vendored Python runtime.** libc corrected `POSIX_SPAWN_SETSID` from
`c_int` to `c_short` on linux-gnu — correctly, since glibc really does store spawn flags in a
`short`. But `rustpython-vm 0.5.0` hands that constant straight to
`nix::spawn::PosixSpawnFlags::from_bits_retain`, which `nix` types as `c_int`. E0308, and the `python`
feature stops building.

The fix is already upstream — RustPython PR #8343, merged 2026-07-22 — and has never been released;
0.5.0 is still the newest crates.io version. So `libc` is held at **0.2.186**. That window is narrower
than it looks: `rustpython-stdlib 0.5.0` itself requires `libc ^0.2.183`, leaving exactly four usable
releases, which is why a plain `cargo update` lands outside it every single time.

### A pin that can no longer be forgotten

The `libc` ceiling is the third dependency constraint in this repo that existed only as a note saying
"remember to redo this after `cargo update`". The other two became a test in August; this one joins
them now. `held_back_crates_stay_below_their_ceiling` asserts a version **ceiling** — the mirror of the
existing single-version guard — and its failure message carries the remedy command *and* the condition
that retires the pin, so the ceiling gets removed on purpose rather than renewed forever. It runs in
0.00s and was verified against the real regression, not just written.

### Dependency freshness

`ocrs 0.13.0` finally shipped against `rten 0.26`, unblocking a pin that three previous runs recorded
as "not ours to fix" — `rten 0.24 → 0.26` with zero source changes. Plus the routine sweep: `wide 1.7`,
`playwright-rs 0.17`, `deno_core 0.411`, and on the frontend `@tiptap/* 3.31.3`, `vitest 5`,
`@lucide/vue 1.42`, `vue-router 5.3.1`, `@playwright/test 1.63` and the Tauri plugins.

## Verified

On Linux, after the fixes: `cargo build --workspace --exclude nanna-gui` green ·
`cargo test --workspace --exclude nanna-gui` **1683 passed / 0 failed / 12 ignored** across 47 test
binaries · `cargo clippy --workspace --all-targets --exclude nanna-gui` **0 errors**, no new warnings
in the changed regions · frontend `vue-tsc --noEmit` clean, **238/238** vitest, `pnpm build` green.

The built Linux daemon was booted against a scratch config (never the operator's): it reaches
`Daemon ready`, serves IPC and health, answers `GET /health` with
`{"status":"ok","version":"0.3.11","uptime_secs":1}`, handles SIGTERM cleanly, and logs **zero
panics**. The only errors are Ollama being unreachable — it is not installed on this host — which the
readiness-wait path handles as designed instead of burning retry budget.

## Known limits

- **The Tauri GUI on Linux is still unverified.** `cargo build` and `cargo test` are green; the desktop
  app is not part of that claim.
- **CI has no Linux job**, so nothing yet stops the next Windows-only assumption from landing the same
  way. That is filed, not fixed.
