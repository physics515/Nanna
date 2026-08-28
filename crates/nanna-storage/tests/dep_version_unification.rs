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
    /// How to restore unification once it has split.
    remedy: Remedy,
}

/// How a split is repaired.
///
/// The distinction is not cosmetic: one form can be turned into an exact
/// command from what the lockfile actually holds, and the other cannot.
enum Remedy {
    /// A permissive transitive requirement drifted upward, and the fix is a
    /// lockfile pin back onto the version the rest of the graph needs. The
    /// command is *derived* from the strays actually observed — a hardcoded
    /// one goes stale the moment upstream publishes again, and then prints a
    /// `cargo update -p name@<gone>` that errors out instead of fixing
    /// anything. Not hypothetical: this entry named `@0.10.0` while the graph
    /// had already drifted to `0.11.0`.
    PinBackTo(&'static str),
    /// No mechanical command — a manifest requirement has to change.
    /// Carries the instruction verbatim.
    Manual(&'static str),
}

impl Remedy {
    /// Render the fix for a crate that resolved to `versions` (len >= 2).
    fn describe(&self, name: &str, versions: &[&str]) -> String {
        debug_assert!(versions.len() >= 2, "only called on an actual split");
        debug_assert!(!name.is_empty(), "crate name must be non-empty");
        match self {
            Remedy::Manual(text) => (*text).to_string(),
            Remedy::PinBackTo(keep) => {
                assert!(
                    versions.contains(keep),
                    "`{name}` must keep {keep}, but the graph holds {versions:?} — the pin \
                     target left the graph, so this entry needs re-deciding, not re-pinning"
                );
                let commands: Vec<String> = versions
                    .iter()
                    .filter(|version| *version != keep)
                    .map(|stray| format!("cargo update -p {name}@{stray} --precise {keep}"))
                    .collect();
                assert!(!commands.is_empty(), "a split must have at least one stray");
                commands.join("\n       ")
            }
        }
    }
}

/// Crates that MUST appear exactly once in the resolved graph.
///
/// Every entry is here because a real build failure proved it, not because a
/// duplicate looked untidy. Adding an entry speculatively would make this guard
/// fail on graphs that are actually fine.
const UNIFIED_CRATES: &[UnifiedCrate] = &[
    UnifiedCrate {
        name: "malachite-bigint",
        reason: "`pymath` requires `malachite-bigint = \"0\"` — any 0.x, so a \
                 bare `cargo update` always takes the newest — while `rustpython-common` \
                 resolves 0.9, and `rustpython-stdlib`, which depends on both, then fails to \
                 compile with 17 E0277/E0308 errors about `malachite_bigint::{BigInt, \
                 BigUint}`. The `=0.9.2` req in crates/nanna-scripting/Cargo.toml pins only \
                 OUR edge and does not constrain `pymath`, so the split recurs every sweep",
        remedy: Remedy::PinBackTo("0.9.2"),
    },
    UnifiedCrate {
        name: "rten",
        reason: "`ocrs` takes `rten::Model` in `OcrEngineParams { detection_model, \
                 recognition_model }`; a direct `rten` req ahead of what `ocrs` pins hands it a \
                 model from the other copy (E0308)",
        remedy: Remedy::Manual(
            "keep `rten` in crates/nanna-tools/Cargo.toml at whatever version `ocrs` \
             requires (`cargo tree -p ocrs -e normal --depth 1`)",
        ),
    },
    UnifiedCrate {
        name: "rten-tensor",
        reason: "the tensor types `rten` and `ocrs` exchange live here, so it splits for the same \
                 reason `rten` does and is the half that reports the mismatch",
        remedy: Remedy::Manual("same as `rten` — track `ocrs`'s requirement"),
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
            guarded.remedy.describe(guarded.name, &versions),
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

#[test]
fn pin_back_remedy_names_the_versions_actually_present() {
    // The whole point of deriving the command: the stray version is whatever the
    // lockfile drifted to, not whatever was written down when the entry was added.
    let remedy = Remedy::PinBackTo("0.9.2");
    let text = remedy.describe("malachite-bigint", &["0.9.2", "0.11.0"]);
    assert!(
        text == "cargo update -p malachite-bigint@0.11.0 --precise 0.9.2",
        "got {text:?}"
    );

    // Two strays at once still produce one runnable command each.
    let text = remedy.describe("malachite-bigint", &["0.9.2", "0.10.0", "0.11.0"]);
    assert!(
        text.lines().count() == 2,
        "expected one command per stray, got {text:?}"
    );
    assert!(
        text.contains("@0.10.0 --precise 0.9.2") && text.contains("@0.11.0 --precise 0.9.2"),
        "both strays must be named: {text:?}"
    );
}

#[test]
fn manual_remedy_is_passed_through_unchanged() {
    let remedy = Remedy::Manual("track `ocrs`'s requirement");
    let text = remedy.describe("rten", &["0.24.0", "0.25.0"]);
    assert!(text == "track `ocrs`'s requirement", "got {text:?}");
}
