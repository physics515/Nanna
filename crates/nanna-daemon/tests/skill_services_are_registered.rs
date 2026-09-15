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
//! Measured 2026-09-15: **14 of the 23 declared services are registered
//! nowhere**, so 16 of the 44 bundled skills are withheld at every boot —
//! including all three tool-authoring skills. Each of those has a roadmap item;
//! what did not exist was one place that says so, or anything stopping the next
//! skill from joining them unnoticed.
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
const KNOWN_MISSING_SERVICES: &[(&str, &str)] = &[
    (
        "audio.transcribe",
        "P18: Whisper client written, never wired",
    ),
    (
        "audio.tts",
        "P18: only OpenAI TTS exists, and it is unwired",
    ),
    (
        "browser.action",
        "P18: nanna-browser is real but registered nowhere",
    ),
    (
        "browser.evaluate",
        "P18: nanna-browser is real but registered nowhere",
    ),
    (
        "browser.extract",
        "P18: nanna-browser is real but registered nowhere",
    ),
    (
        "browser.screenshot",
        "P18: nanna-browser is real but registered nowhere",
    ),
    (
        "schedule.add",
        "P18: scheduler exists, the skill bridge does not",
    ),
    (
        "schedule.cancel",
        "P18: scheduler exists, the skill bridge does not",
    ),
    (
        "schedule.list",
        "P18: scheduler exists, the skill bridge does not",
    ),
    (
        "screenshot.capture",
        "P18: skill exists, service missing, Rust tool is a stub",
    ),
    (
        "tools.create",
        "P18: UserToolManager exists, no service exposes it",
    ),
    (
        "tools.list",
        "P18: UserToolManager exists, no service exposes it",
    ),
    (
        "tools.update",
        "P18: UserToolManager exists, no service exposes it",
    ),
    (
        "vision.analyze",
        "P18: create_vision_tool and OcrTool are complete and unreachable",
    ),
];

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

/// Every service name the daemon's builders can ever insert.
///
/// Read from the source of the two `HashMap<String, ServiceFn>` builders rather
/// than by calling them, because most inserts are conditional on an optional
/// dependency (memory, storage, a scheduler) and a map built in a test would
/// report a subset that changes with the test's own fixtures. The question here
/// is "does anything, anywhere, implement this name" — which is a source fact.
fn registered_service_names() -> BTreeSet<String> {
    let src = workspace_crates_dir().join("nanna-daemon").join("src");
    let mut names = BTreeSet::new();
    for file in ["server.rs", "tasks.rs"] {
        let text = std::fs::read_to_string(src.join(file))
            .unwrap_or_else(|e| panic!("cannot read {file}: {e}"));
        let mut rest = text.as_str();
        while let Some(at) = rest.find("services.insert(") {
            rest = &rest[at + "services.insert(".len()..];
            // The key is the next string literal, which may sit on the
            // following line: `services.insert(\n    "pdf.read".to_string(),`.
            let Some(open) = rest.find('"') else { break };
            let after_open = &rest[open + 1..];
            let Some(close) = after_open.find('"') else {
                break;
            };
            let key = &after_open[..close];
            if key.contains('.') {
                names.insert(key.to_string());
            }
        }
    }
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
        "{} of {} bundled skills are withheld for missing services: {:?}",
        withheld.len(),
        by_skill.len(),
        withheld,
    );
}
