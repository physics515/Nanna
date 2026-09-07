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

/// A crate that must NOT be resolved past a known-good version, and why.
///
/// This is the mirror image of `UnifiedCrate`: there the danger is two copies,
/// here it is one copy that is too new. A dependency of ours can require a
/// version range that includes a release it does not itself compile against —
/// and cargo will happily pick the newest member of that range.
struct CeilingCrate {
    /// Crate name exactly as it appears in `Cargo.lock`.
    name: &'static str,
    /// Highest version known to build this workspace, inclusive.
    version_max: &'static str,
    /// What breaks above the ceiling.
    reason: &'static str,
    /// The concrete command that restores a buildable version.
    remedy: &'static str,
    /// The condition under which this entry should be deleted, not renewed.
    lift_when: &'static str,
}

/// Crates held below their latest release because a *dependency* cannot build
/// against the newer one.
///
/// As with `UNIFIED_CRATES`, an entry earns its place by having broken a real
/// build. A ceiling is a liability — it holds back security fixes — so each one
/// carries the condition that retires it.
const CEILING_CRATES: &[CeilingCrate] = &[CeilingCrate {
    name: "libc",
    version_max: "0.2.186",
    reason: "libc 0.2.187 corrected `POSIX_SPAWN_SETSID` from `c_int` to `c_short` on linux-gnu \
             (glibc really does store spawn flags in a `short`). `rustpython-vm 0.5.0` passes that \
             constant straight into `nix::spawn::PosixSpawnFlags::from_bits_retain`, which nix \
             types as `c_int` — E0308 at rustpython-vm-0.5.0/src/stdlib/posix.rs:1812. That kills \
             the `python` feature, and with it the `nanna` binary, on Linux only. Note \
             `rustpython-stdlib 0.5.0` requires `libc ^0.2.183`, so the buildable window is the \
             four releases 0.2.183..=0.2.186 — narrow enough that a bare `cargo update` always \
             lands outside it",
    remedy: "cargo update -p libc --precise 0.2.186",
    lift_when: "rustpython publishes any release after 0.5.0 — the fix is upstream already \
                (RustPython PR #8343 `Fix building against new libc`, merged 2026-07-22), it has \
                simply never been released",
}];

/// Compare two dotted numeric versions positionally.
///
/// Deliberately not a semver dependency: these are plain `x.y.z` lockfile
/// versions and this guard stays dependency-free like its sibling parser.
/// Missing components read as zero, so `1.2` and `1.2.0` compare equal.
fn version_is_at_most(version: &str, ceiling: &str) -> bool {
    assert!(!version.is_empty(), "version must be non-empty");
    assert!(!ceiling.is_empty(), "ceiling must be non-empty");

    let mut left = version.split('.');
    let mut right = ceiling.split('.');
    // Bounded: a lockfile version is at most major.minor.patch plus a suffix,
    // so four components is already one more than can appear.
    for _ in 0..4 {
        let (a, b) = (
            numeric_component(left.next()),
            numeric_component(right.next()),
        );
        if a != b {
            return a < b;
        }
    }
    true
}

/// Read one dotted component as a number, ignoring any pre-release suffix.
///
/// A missing component is zero; a non-numeric one (e.g. `0-rc1`) contributes
/// only its leading digits, which is enough to order the releases we pin.
fn numeric_component(part: Option<&str>) -> u64 {
    let Some(text) = part else { return 0 };
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    debug_assert!(
        digits.len() <= text.len(),
        "digit prefix cannot exceed the component"
    );
    digits.parse().unwrap_or(0)
}

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

#[test]
fn held_back_crates_stay_below_their_ceiling() {
    let lockfile = workspace_lockfile();
    let contents = std::fs::read_to_string(&lockfile)
        .unwrap_or_else(|e| panic!("cannot read {lockfile:?}: {e}"));
    let packages = resolved_packages(&contents);
    assert!(
        !packages.is_empty(),
        "parsed zero packages from {lockfile:?} — lockfile format changed?"
    );

    for guarded in CEILING_CRATES {
        let versions: Vec<&str> = packages
            .iter()
            .filter(|(name, _)| *name == guarded.name)
            .map(|(_, version)| *version)
            .collect();

        // Same reasoning as the unification guard: a ceiling on a crate that
        // left the graph can never fire, so make its removal deliberate.
        assert!(
            !versions.is_empty(),
            "`{}` is no longer in the dependency graph — delete its CEILING_CRATES entry \
             in this test rather than leaving a guard that can never fire",
            guarded.name,
        );

        for version in &versions {
            assert!(
                version_is_at_most(version, guarded.version_max),
                "`{}` resolved to {} but must stay at or below {}.\n  why: {}\n  fix: {}\n  \
                 lift this ceiling when: {}",
                guarded.name,
                version,
                guarded.version_max,
                guarded.reason,
                guarded.remedy,
                guarded.lift_when,
            );
        }
    }
}

#[test]
fn version_comparison_orders_releases_and_respects_the_boundary() {
    // Positive space: at or below the ceiling.
    assert!(version_is_at_most("0.2.186", "0.2.186"), "equal is allowed");
    assert!(version_is_at_most("0.2.183", "0.2.186"), "older patch");
    assert!(version_is_at_most("0.1.999", "0.2.186"), "older minor");

    // Negative space: the exact release that broke the build must be rejected,
    // and numeric (not lexicographic) ordering must be used — "0.2.9" would sort
    // ABOVE "0.2.186" as text.
    assert!(
        !version_is_at_most("0.2.187", "0.2.186"),
        "the breaking release must fail the guard"
    );
    assert!(
        !version_is_at_most("0.2.189", "0.2.186"),
        "current latest must fail the guard"
    );
    assert!(
        version_is_at_most("0.2.9", "0.2.186"),
        "components compare numerically, not as text"
    );
    assert!(!version_is_at_most("1.0.0", "0.2.186"), "newer major");

    // Missing components read as zero, so a two-part version is comparable.
    assert!(version_is_at_most("0.2", "0.2.0"), "0.2 == 0.2.0");
}
