#![warn(clippy::pedantic, clippy::nursery, clippy::all)]
//! Every service a bundled skill declares must be one the daemon can register.
//!
//! A scripted skill declares its daemon-side dependencies as
//! `requires: ["memory.list", ...]`, and the registry withholds a skill whose
//! services are absent rather than advertising a tool that can only fail (the
//! 2026-07-26 run that spent its tail retrying `reflect` and `list_reminders`
//! is why). That behaviour is right, and it is also *quiet*: the skill simply
//! is not there, logged once at `info` among a few hundred boot lines.
//!
//! So the failure mode this test exists for is not a crash, it is an absence.
//! Measured 2026-09-15: **14 of the 23 declared services were registered
//! nowhere**, so 16 of the 44 bundled skills were withheld at every boot —
//! including all three tool-authoring skills, which this run then wired
//! (`tools.create`/`update`/`list`), then `vision.analyze`, `audio.*` and the
//! four `browser.*`, leaving **4 missing** — the three `schedule.*`, blocked on
//! delivery rather than on a bridge, and `screenshot.capture`.
//! Each remaining gap has a roadmap item; what did not exist was one place that
//! says so, or anything stopping the next skill from joining them unnoticed.
//!
//! This test is that place. It compares what the skills ask for against what
//! the daemon's two service builders can ever insert, and requires every gap to
//! be named in [`KNOWN_MISSING_SERVICES`] with its reason. A new skill that
//! declares a service nobody implements fails here, at compile-and-test time,
//! instead of being silently withheld from the model forever.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Services declared by a bundled skill that the daemon registers nowhere.
///
/// Each entry is a live roadmap item, not an accepted state: the skill that
/// needs it is withheld at every boot. Removing an entry once its service is
/// implemented is the point — a stale entry is caught by
/// [`no_known_missing_entry_is_stale`].
const KNOWN_MISSING_SERVICES: &[(&str, &str)] = &[];

/// Fewest skills the tree can plausibly hold; below this the walk found nothing
/// and every assertion would pass for free. There were 44 on 2026-09-15.
const MIN_BUNDLED_SKILLS: usize = 40;

/// Fewest services the builders can plausibly insert; same guard, other side.
/// There were 24 on 2026-09-15.
const MIN_REGISTERED_SERVICES: usize = 20;

fn workspace_crates_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR is `crates/nanna-daemon`.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("nanna-daemon sits under crates/")
        .to_path_buf()
}

/// Service names each bundled skill declares, keyed by skill directory name.
///
/// Parsed with the loader's own `extract_manifest`, not a bespoke regex, so a
/// declaration this test cannot see is one the daemon cannot see either.
fn required_services_by_skill() -> BTreeMap<String, Vec<String>> {
    let skills_dir = workspace_crates_dir()
        .join("nanna-tools")
        .join("default-skills");
    let entries = std::fs::read_dir(&skills_dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", skills_dir.display()));

    let mut by_skill = BTreeMap::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        // Same tool.ts/tool.js precedence the loader uses.
        let ts = dir.join("tool.ts");
        let source_path = if ts.exists() { ts } else { dir.join("tool.js") };
        let Ok(source) = std::fs::read_to_string(&source_path) else {
            continue;
        };
        let Some(manifest) = nanna_scripting::extract_manifest(&source) else {
            continue;
        };
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        by_skill.insert(name, manifest.requires);
    }
    by_skill
}

/// Every service name the daemon can ever insert, from anywhere in its source.
///
/// Read from source rather than by calling the builders, because most inserts
/// are conditional on an optional dependency (memory, storage, a scheduler, a
/// tools directory) and a map built inside a test would report a subset that
/// changes with the test's own fixtures. The question here is "does anything,
/// anywhere, implement this name" — which is a source fact.
///
/// It walks the **whole** `src/` tree rather than a named list of files. An
/// earlier version scanned only `server.rs` and `tasks.rs`, and the very next
/// service to be added lived in a new module, so the scan reported it missing
/// and the ledger would have been wrong in the safe-looking direction. A test
/// whose coverage has to be maintained by hand is a test that silently stops
/// covering things.
fn registered_service_names() -> BTreeSet<String> {
    /// A service key: `family.action`, both lowercase snake segments. Tight
    /// enough that an unrelated `map.insert("some.other")` cannot pass for one.
    fn looks_like_a_service(key: &str) -> bool {
        let Some((family, action)) = key.split_once('.') else {
            return false;
        };
        let segment_ok = |segment: &str| {
            !segment.is_empty()
                && segment.starts_with(|c: char| c.is_ascii_lowercase())
                && segment
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        };
        segment_ok(family) && segment_ok(action)
    }

    fn walk(dir: &Path, names: &mut BTreeSet<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, names);
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            // Any `<map>.insert("<key>"`, so a new service module is covered
            // without this test being told about it.
            let mut rest = text.as_str();
            while let Some(at) = rest.find(".insert(") {
                rest = &rest[at + ".insert(".len()..];
                // The key is the next string literal, which may sit on the
                // following line: `.insert(\n    "pdf.read".to_string(),`.
                let Some(open) = rest.find('"') else { break };
                // ...but only if nothing but whitespace precedes it, or a
                // `map.insert(other_variable)` would swallow the next literal
                // on the line.
                if !rest[..open].trim().is_empty() {
                    continue;
                }
                let after_open = &rest[open + 1..];
                let Some(close) = after_open.find('"') else {
                    break;
                };
                let key = &after_open[..close];
                if looks_like_a_service(key) {
                    names.insert(key.to_string());
                }
            }
        }
    }

    let mut names = BTreeSet::new();
    walk(
        &workspace_crates_dir().join("nanna-daemon").join("src"),
        &mut names,
    );
    names
}

#[test]
fn every_service_a_skill_requires_is_registered_or_knowingly_missing() {
    let by_skill = required_services_by_skill();
    assert!(
        by_skill.len() >= MIN_BUNDLED_SKILLS,
        "found only {} bundled skills; the walk is broken and every assertion \
         below would pass for free",
        by_skill.len(),
    );

    let registered = registered_service_names();
    assert!(
        registered.len() >= MIN_REGISTERED_SERVICES,
        "found only {} registered services; the source scan is broken",
        registered.len(),
    );
    // Positive space: the service the `read_pdf` gap was fixed by must be seen
    // by this scan, or a green result means nothing.
    assert!(
        registered.contains("pdf.read"),
        "the scan missed pdf.read, which is registered",
    );

    let known: BTreeSet<&str> = KNOWN_MISSING_SERVICES
        .iter()
        .map(|(name, _)| *name)
        .collect();
    let mut unexplained: Vec<String> = Vec::new();
    for (skill, requires) in &by_skill {
        for service in requires {
            if registered.contains(service) || known.contains(service.as_str()) {
                continue;
            }
            unexplained.push(format!("{skill} requires {service}"));
        }
    }

    assert!(
        unexplained.is_empty(),
        "these skills declare services the daemon registers nowhere, so they \
         are withheld from the model at every boot and nothing says why:\n  {}\n\
         Implement the service, or add it to KNOWN_MISSING_SERVICES with the \
         roadmap item that tracks it.",
        unexplained.join("\n  "),
    );
}

#[test]
fn no_known_missing_entry_is_stale() {
    let registered = registered_service_names();
    let stale: Vec<&str> = KNOWN_MISSING_SERVICES
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| registered.contains(*name))
        .collect();
    assert!(
        stale.is_empty(),
        "these services are registered now and must leave \
         KNOWN_MISSING_SERVICES, or the ledger starts lying about what is \
         broken: {stale:?}",
    );

    let required: BTreeSet<String> = required_services_by_skill()
        .into_values()
        .flatten()
        .collect();
    let unrequested: Vec<&str> = KNOWN_MISSING_SERVICES
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| !required.contains(*name))
        .collect();
    assert!(
        unrequested.is_empty(),
        "these services are in KNOWN_MISSING_SERVICES but no skill asks for \
         them any more, so the entry tracks nothing: {unrequested:?}",
    );

    for (name, reason) in KNOWN_MISSING_SERVICES {
        assert!(
            !name.is_empty(),
            "a known-missing entry has no service name"
        );
        assert!(
            !reason.is_empty(),
            "{name} is listed as knowingly missing with no reason, which makes \
             it indistinguishable from an oversight",
        );
    }
}

/// The count is not the assertion — the ledger above is. This one exists so the
/// scale of the gap is visible in the test output rather than having to be
/// recounted by hand next time somebody asks how much of the tool surface is
/// actually reachable.
///
/// **It counts services with no implementation anywhere, which is a lower bound
/// on what a given daemon withholds.** Some services are registered only when
/// something is configured — `vision.analyze` needs `[memory]
/// ocr_model_priority` to name a model the router can serve — so on a default
/// install the live count is higher than this one. That is the intended
/// difference: "nobody wrote it" and "you have not configured it" are different
/// problems with different fixes, and the daemon's own boot warning reports the
/// second.
#[test]
fn report_how_many_bundled_skills_are_withheld() {
    let by_skill = required_services_by_skill();
    let registered = registered_service_names();

    let withheld: Vec<&String> = by_skill
        .iter()
        .filter(|(_, requires)| requires.iter().any(|s| !registered.contains(s)))
        .map(|(skill, _)| skill)
        .collect();

    assert!(
        withheld.len() < by_skill.len(),
        "every bundled skill is withheld, which means the service scan failed \
         rather than that the daemon ships nothing",
    );
    println!(
        "{} of {} bundled skills have a service nobody implements: {:?} \
         (a live daemon withholds at least these, plus any whose service is \
         implemented but unconfigured)",
        withheld.len(),
        by_skill.len(),
        withheld,
    );
}
