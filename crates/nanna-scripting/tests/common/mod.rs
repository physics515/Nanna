//! Shared scaffolding for the default-skill integration tests.
//!
//! Not a test target itself — `tests/common/` is a directory, so cargo compiles
//! it into each test binary that says `mod common;` rather than running it.

/// Deadline the skill fixtures in these tests run under.
///
/// **These tests measure edit and search *semantics* — the bytes a skill leaves
/// on disk and the shape of the value it returns — not latency.** Every fixture
/// is a handful of small files in a temp directory: single-digit milliseconds of
/// real work.
///
/// They previously inherited [`nanna_scripting::ScriptedTool`]'s production
/// default of 30 s. That number is right in production, where it bounds a real
/// tool call that may shell out or fetch. It is wrong here, for one reason: the
/// engine's deadline is **wall-clock and absolute**, so it measures elapsed time
/// rather than time the script was actually scheduled. Under a full
/// `cargo test --workspace` — sixteen test binaries plus a compile on the same
/// box — a correct edit that needs 3 ms of CPU can spend 30 s of wall-clock
/// waiting for one, and the deadline fires on a passing test.
///
/// Observed 2026-08-24: 6 of `edit_file_skill.rs`'s 17 tests failed a full
/// workspace run with `"Timeout after 30000ms"`; the same file run alone passed
/// 17/17 in **3.49 s**.
///
/// So the deadline here is **not an assertion** — it is the backstop that keeps
/// a runaway script from wedging the suite forever, and nothing else. Five
/// minutes is ~100 000× the real work: unreachable by scheduling delay on any
/// machine that can run this suite at all, while still bounding an infinite loop
/// to something CI notices rather than hanging until the job is killed.
///
/// Raising it further would not be safer, and lowering it back toward the real
/// work would re-introduce the flake. If a fixture ever genuinely approaches
/// this, the fixture is wrong, not the number.
pub const FIXTURE_TIMEOUT_MS: u64 = 300_000;
