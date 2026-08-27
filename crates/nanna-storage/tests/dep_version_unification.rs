//! Dependency guard: crates whose **types cross a crate boundary** must resolve
//! to exactly ONE version in the workspace graph.
//!
//! Cargo is happy to resolve two semver-incompatible copies of the same crate.
//! That is usually harmless — two `bitflags`, two `windows-sys`, two `syn` all
//! coexist in this lockfile today and nothing notices. It is *not* harmless when
//! crate `A` hands one of `C`'s types to crate `B`: then the two copies are two
//! distinct types and the build dies with `there are multiple different versions
//! of crate C in the dependency graph`.
//!
//! This repo has hit that exact failure twice, and both times the only thing
//! holding the fix was a note in `ROADMAP.md` saying "remember to redo the pin
//! after every `cargo update`" — which is a habit, not a gate. Worse, the
//! `malachite-bigint` form of it surfaces in practice only in the **release**
//! build, i.e. ~20 minutes after the mistake was made.
//!
//! So the invariant gets asserted where it can be read in milliseconds. This
//! lives beside `dep_guard.rs` because that is already the workspace's
//! lockfile-guard home (cheap crate, already in CI's `cargo test` scope) — it is
//! a dependency-graph invariant, not a storage one.

use std::path::PathBuf;

/// A crate that must resolve to exactly one version, and what to do about it.
struct UnifiedCrate {
    /// Crate name exactly as it appears in `Cargo.lock`.
    name: &'static str,
    /// Why a second copy breaks the build — the type that crosses the boundary.
    reason: &'static str,
    /// The concrete command that restores unification.
    remedy: &'static str,
}

/// Crates that MUST appear exactly once in the resolved graph.
///
/// Every entry is here because a real build failure proved it, not because a
/// duplicate looked untidy. Adding an entry speculatively would make this guard
/// fail on graphs that are actually fine.
const UNIFIED_CRATES: &[UnifiedCrate] = &[
    UnifiedCrate {
        name: "malachite-bigint",
        reason: "`pymath` accepts 0.10 while `rustpython-codegen` requires 0.9, so a bare \
                 `cargo update` resolves both and `rustpython-stdlib` fails to compile with 17 \
                 E0277/E0308 errors about `malachite_bigint::{BigInt, BigUint}`",
        remedy: "cargo update -p malachite-bigint@0.10.0 --precise 0.9.2",
    },
    UnifiedCrate {
        name: "rten",
        reason: "`ocrs` takes `rten::Model` in `OcrEngineParams { detection_model, \
                 recognition_model }`; a direct `rten` req ahead of what `ocrs` pins hands it a \
                 model from the other copy (E0308)",
        remedy: "keep `rten` in crates/nanna-tools/Cargo.toml at whatever version `ocrs` requires \
                 (`cargo tree -p ocrs -e normal --depth 1`)",
    },
    UnifiedCrate {
        name: "rten-tensor",
        reason: "the tensor types `rten` and `ocrs` exchange live here, so it splits for the same \
                 reason `rten` does and is the half that reports the mismatch",
        remedy: "same as `rten` — track `ocrs`'s requirement",
    },
];

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

/// Collect `(name, version)` for every `[[package]]` entry in a lockfile.
///
/// Parses positionally rather than with a TOML dependency: a `[[package]]`
/// block always states `name` before `version`, so a pending-name state machine
/// is enough and keeps this guard free of its own dependencies.
fn resolved_packages(contents: &str) -> Vec<(&str, &str)> {
    assert!(!contents.is_empty(), "lockfile contents are empty");

    let mut packages: Vec<(&str, &str)> = Vec::new();
    let mut pending_name: Option<&str> = None;
    for line in contents.lines() {
        let trimmed = line.trim();
        if let Some(name) = quoted_value(trimmed, "name") {
            pending_name = Some(name);
        } else if let Some(version) = quoted_value(trimmed, "version") {
            if let Some(name) = pending_name.take() {
                packages.push((name, version));
            }
        }
    }

    assert!(
        pending_name.is_none(),
        "lockfile ended with a package name that had no version — format changed?"
    );
    packages
}

/// Extract `value` from a `key = "value"` line, if the line is exactly that.
fn quoted_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    debug_assert!(!key.is_empty(), "key must be non-empty");
    let rest = line.strip_prefix(key)?.trim_start();
    let value = rest.strip_prefix("= \"")?.strip_suffix('"')?;
    debug_assert!(
        !value.contains('"'),
        "quoted value should not contain a quote"
    );
    Some(value)
}

#[test]
fn type_crossing_crates_resolve_to_one_version() {
    let lockfile = workspace_lockfile();
    let contents = std::fs::read_to_string(&lockfile)
        .unwrap_or_else(|e| panic!("cannot read {lockfile:?}: {e}"));
    let packages = resolved_packages(&contents);
    assert!(
        !packages.is_empty(),
        "parsed zero packages from {lockfile:?} — lockfile format changed?"
    );

    for guarded in UNIFIED_CRATES {
        let versions: Vec<&str> = packages
            .iter()
            .filter(|(name, _)| *name == guarded.name)
            .map(|(_, version)| *version)
            .collect();

        // A guard for a crate that left the graph is a dead guard. Fail loudly so
        // the entry gets removed deliberately instead of passing forever.
        assert!(
            !versions.is_empty(),
            "`{}` is no longer in the dependency graph — delete its UNIFIED_CRATES entry \
             in this test rather than leaving a guard that can never fire",
            guarded.name,
        );
        assert!(
            versions.len() == 1,
            "`{}` resolved to {} versions {:?}, but it must be unified.\n  why: {}\n  fix: {}",
            guarded.name,
            versions.len(),
            versions,
            guarded.reason,
            guarded.remedy,
        );
    }
}

#[test]
fn lockfile_parser_pairs_names_with_versions() {
    let sample = "\
[[package]]
name = \"alpha\"
version = \"1.2.3\"
source = \"registry+https://github.com/rust-lang/crates.io-index\"

[[package]]
name = \"beta\"
version = \"0.1.0\"
dependencies = [
 \"alpha\",
]
";
    let packages = resolved_packages(sample);
    assert!(
        packages == vec![("alpha", "1.2.3"), ("beta", "0.1.0")],
        "parsed {packages:?}"
    );

    // Negative space: a dependency list mentioning a name must not be mistaken
    // for a package, and a `version` with no preceding `name` must be dropped.
    let orphan = "version = \"9.9.9\"\n";
    assert!(
        resolved_packages(orphan).is_empty(),
        "orphan version must not pair"
    );
}

#[test]
fn duplicate_versions_are_detected() {
    let sample = "\
[[package]]
name = \"malachite-bigint\"
version = \"0.9.2\"

[[package]]
name = \"malachite-bigint\"
version = \"0.10.0\"
";
    let packages = resolved_packages(sample);
    let versions: Vec<&str> = packages
        .iter()
        .filter(|(name, _)| *name == "malachite-bigint")
        .map(|(_, version)| *version)
        .collect();
    assert!(
        versions.len() == 2,
        "expected both copies, got {versions:?}"
    );
    assert!(
        versions.contains(&"0.10.0"),
        "the offending copy must be visible: {versions:?}"
    );
}
