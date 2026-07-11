//! Real-data guard: every shipped default skill's `parameters` block must
//! normalize into a valid JSON-Schema object via `extract_manifest`, so the
//! LLM-facing tool definitions carry real input schemas (not empty lists).
//!
//! Tolerant by design: if the sibling `nanna-tools/default-skills` tree isn't
//! present (e.g. a packaging layout that strips it), the test no-ops instead of
//! failing.

use std::path::PathBuf;

fn skills_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nanna-tools/default-skills")
}

#[test]
fn every_default_skill_parameters_block_parses() {
    let dir = skills_dir();
    if !dir.is_dir() {
        eprintln!("skipping: {} not present", dir.display());
        return;
    }

    let mut checked = 0usize;
    let mut with_params = 0usize;

    for entry in std::fs::read_dir(&dir).expect("read default-skills") {
        let entry = entry.expect("dir entry");
        let tool_ts = entry.path().join("tool.ts");
        if !tool_ts.is_file() {
            continue;
        }
        let source = std::fs::read_to_string(&tool_ts).expect("read tool.ts");
        let manifest = nanna_scripting::extract_manifest(&source)
            .unwrap_or_else(|| panic!("manifest failed to extract: {}", tool_ts.display()));
        checked += 1;

        // Only assert on skills that actually declare a `parameters:` block.
        if source.contains("parameters:") {
            let params = manifest.parameters.unwrap_or_else(|| {
                panic!(
                    "parameters declared but did not normalize to JSON: {}",
                    tool_ts.display()
                )
            });
            assert!(
                params
                    .get("properties")
                    .map(|p| p.is_object())
                    .unwrap_or(false)
                    || params.get("type").is_some(),
                "parameters schema missing properties/type: {}",
                tool_ts.display()
            );
            with_params += 1;
        }
    }

    assert!(checked > 0, "no default skills were checked");
    eprintln!("checked {checked} skills, {with_params} with parameter schemas");
}
