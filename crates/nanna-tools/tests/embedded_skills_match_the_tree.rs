//! The release binary must ship every skill the source tree has.
//!
//! `build.rs` walks `default-skills/` and emits `DEFAULT_SKILLS`, which is what
//! a RELEASE build extracts on first run — the source tree is only read in
//! debug builds. So a skill that the build script fails to pick up is invisible
//! exactly where it matters most, and nothing downstream says so: the generated
//! array compiles whether it holds forty-four entries or none.
//!
//! That is not hypothetical in this repo. A debug build off Windows once loaded
//! **none** of its 43 JS/TS skills because one path was assembled with a
//! Windows separator (fixed 2026-09-13). This is the same failure on the
//! release path, and it had no gate until now.
//!
//! `build.rs` now asserts rather than falling through to an empty array; this
//! test is the other half — it checks the emitted set against the tree itself,
//! so a skill dropped for any reason is a test failure and not a quiet
//! omission.

use nanna_tools::skills::defaults::DEFAULT_SKILLS;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn skills_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("default-skills")
}

/// Skill directories in the source tree, by name.
fn skills_on_disk() -> BTreeSet<String> {
    std::fs::read_dir(skills_dir())
        .expect("read default-skills")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

/// Embedded files grouped by the skill they belong to.
fn embedded_by_skill() -> BTreeMap<String, BTreeSet<String>> {
    let mut map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for file in DEFAULT_SKILLS {
        map.entry(file.skill_name.to_string())
            .or_default()
            .insert(file.file_name.to_string());
    }
    map
}

#[test]
fn every_skill_in_the_tree_is_embedded_in_the_binary() {
    let on_disk = skills_on_disk();
    let embedded = embedded_by_skill();

    assert!(
        !on_disk.is_empty(),
        "no skill directories found — the gate cannot be skipped into passing"
    );

    let embedded_names: BTreeSet<String> = embedded.keys().cloned().collect();
    let missing: Vec<&String> = on_disk.difference(&embedded_names).collect();
    assert!(
        missing.is_empty(),
        "skills exist on disk but are NOT in the release binary: {missing:?}"
    );

    // The other direction matters too: an embedded skill with no directory
    // means the generated array is stale relative to the tree.
    let orphaned: Vec<&String> = embedded_names.difference(&on_disk).collect();
    assert!(
        orphaned.is_empty(),
        "embedded skills with no directory on disk: {orphaned:?}"
    );
}

#[test]
fn every_embedded_skill_carries_its_tool_and_its_permissions() {
    for (skill, files) in embedded_by_skill() {
        assert!(
            files.contains("tool.ts"),
            "embedded skill `{skill}` has no tool.ts: {files:?}"
        );
        assert!(
            files.contains("permissions.json"),
            "embedded skill `{skill}` has no permissions.json: {files:?}"
        );
    }
}

#[test]
fn no_embedded_file_is_empty() {
    // `include_str!` of a truncated file succeeds and yields "". A skill that
    // extracts as an empty tool.ts is registered and then fails at call time.
    for file in DEFAULT_SKILLS {
        assert!(
            !file.content.trim().is_empty(),
            "embedded {}/{} is empty",
            file.skill_name,
            file.file_name
        );
    }
}

/// The count is the assertion. A loop over a nearly empty array passes
/// everything inside it for free — the lesson `default_skill_permissions.rs`
/// wrote down, applied to the set that actually ships.
#[test]
fn the_binary_ships_the_whole_catalogue() {
    let embedded = embedded_by_skill();
    assert!(
        embedded.len() >= 44,
        "only {} skills are embedded; the tree shipped 44 on 2026-09-14, so a smaller number \
         means skills went missing rather than that the gate got easier",
        embedded.len()
    );
    assert_eq!(
        embedded.len(),
        skills_on_disk().len(),
        "the embedded catalogue and the source tree must be the same size"
    );
}
