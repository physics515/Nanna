//! Dependency guard: Turso is the *only* database engine.
//!
//! Nanna committed to a pure-Rust, SQLite-compatible embedded DB (`turso`) and
//! explicitly bans the C-backed / alternative SQL stacks. This test fails the
//! build if any of them ever re-enters the dependency tree — the "CI guard"
//! from the roadmap, implemented without external CI so a plain `cargo test`
//! enforces it.
//!
//! Note: `libsqlite3-sys` is intentionally *not* banned here — it arrives
//! transitively through RustPython in `nanna-scripting` and is a separate
//! concern from the database engine choice.

use std::path::PathBuf;

/// Crate names that must never appear in the resolved dependency graph.
const BANNED_CRATES: &[&str] = &["rusqlite", "libsql", "sqlx"];

/// Locate the workspace `Cargo.lock` starting from this crate's manifest dir.
fn workspace_lockfile() -> PathBuf {
    // CARGO_MANIFEST_DIR = <root>/crates/nanna-storage → up two levels.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("crate should live two levels below the workspace root");
    root.join("Cargo.lock")
}

#[test]
fn no_banned_database_crates_in_lockfile() {
    let lockfile = workspace_lockfile();
    let contents = std::fs::read_to_string(&lockfile)
        .unwrap_or_else(|e| panic!("cannot read {lockfile:?}: {e}"));

    // Collect every resolved crate name (`name = "..."` lines in Cargo.lock).
    let names: Vec<&str> = contents
        .lines()
        .filter_map(|line| line.strip_prefix("name = \""))
        .filter_map(|rest| rest.strip_suffix('"'))
        .collect();
    assert!(
        !names.is_empty(),
        "parsed zero crate names from {lockfile:?} — lockfile format changed?"
    );

    let offenders: Vec<&str> = BANNED_CRATES
        .iter()
        .copied()
        .filter(|banned| names.contains(banned))
        .collect();
    assert!(
        offenders.is_empty(),
        "banned database crate(s) entered the dependency tree: {offenders:?} — \
         Nanna is Turso-only (see nanna-storage). Remove the offending dependency.",
    );
}
