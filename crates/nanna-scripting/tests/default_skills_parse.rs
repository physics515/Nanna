//! Real-data guard: every bundled default skill must **parse** in the engine that
//! runs it.
//!
//! Why this exists. `extract_manifest` reads `name`/`version`/`description`/
//! `parameters` out of a skill by scanning the source as text — it never parses the
//! JavaScript. `default_skills_params.rs` therefore proves the manifest block is
//! well formed and says nothing at all about the body. Measured on this tree before
//! this file existed: planting `var lines = [;;; this is not javascript at all (((`
//! into the body of `wonder/tool.ts` left **295 tests across `nanna-scripting` and
//! `nanna-tools` passing**. The skill would have shipped, been advertised to the
//! model with a valid schema, and failed for the first time inside a user's session.
//!
//! The gate parses the same text the runtime evaluates — `check_syntax` shares
//! `wrap_for_boa` with `execute_sync`, so the `export default` rewrite and the IIFE
//! wrapper are covered too. Either of those can introduce a syntax error the skill
//! file does not itself contain.
//!
//! Parse, not execute: executing 43 skills would need a live bridge, real inputs and
//! a filesystem, and would test their behaviour rather than their existence. Every
//! syntax fault is caught at parse time, and syntax is the class that was unguarded.

use std::path::{Path, PathBuf};

fn skills_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nanna-tools/default-skills")
}

/// Skill directories, sorted, so the count below is stable and failures name a file.
fn skill_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("read default-skills")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    dirs
}

#[test]
fn every_bundled_skill_parses_in_the_engine_that_runs_it() {
    let dir = skills_dir();
    assert!(
        dir.is_dir(),
        "default-skills tree missing at {} — this gate cannot be skipped into passing",
        dir.display()
    );

    let dirs = skill_dirs(&dir);
    assert!(
        !dirs.is_empty(),
        "no skill directories found in {}",
        dir.display()
    );

    let mut parsed = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for skill in &dirs {
        let tool_ts = skill.join("tool.ts");
        assert!(
            tool_ts.is_file(),
            "skill directory ships no tool.ts: {}",
            skill.display()
        );

        let source = std::fs::read_to_string(&tool_ts)
            .unwrap_or_else(|e| panic!("read {}: {e}", tool_ts.display()));
        assert!(
            !source.trim().is_empty(),
            "skill ships an empty tool.ts: {}",
            tool_ts.display()
        );

        match nanna_scripting::check_syntax(&source) {
            Ok(()) => parsed += 1,
            Err(e) => failures.push(format!("{}: {e}", tool_ts.display())),
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} bundled skills do not parse:\n{}",
        failures.len(),
        dirs.len(),
        failures.join("\n")
    );
    assert_eq!(
        parsed,
        dirs.len(),
        "every skill directory must contribute exactly one parsed tool.ts"
    );

    // The count is asserted, not merely reported. `default_skill_permissions.rs`
    // records the same lesson: a loop over an empty list passes every assertion
    // inside it for free, so the number of things checked is itself the assertion.
    assert!(
        parsed >= 44,
        "only {parsed} skills were parsed; the tree shipped 44 on 2026-09-14, so a \
         smaller number means skills went missing rather than that the gate got easier"
    );

    eprintln!("parsed {parsed} bundled skills");
}

/// The planted canary: proof the checker has eyes.
///
/// Without this, a `check_syntax` that always returned `Ok` would make the test
/// above pass forever while guarding nothing — the exact failure mode it was
/// written to catch.
#[test]
fn the_checker_rejects_source_that_does_not_parse() {
    let broken = r#"
export default {
  name: "canary",
  execute: function (input) {
    var lines = [;;; this is not javascript at all (((
    return lines;
  }
}
"#;
    let err = nanna_scripting::check_syntax(broken)
        .expect_err("a deliberately broken skill must not parse");

    let text = err.to_string();
    assert!(
        text.contains("parse failed"),
        "the rejection must say it was a parse failure, got: {text}"
    );
}

/// A real skill body is accepted — the canary above proves the checker can say no,
/// this proves it can still say yes, so neither verdict is a constant.
#[test]
fn a_well_formed_skill_body_is_accepted() {
    let good = r#"
export default {
  name: "canary_ok",
  version: "0.0.1",
  execute: function (input) {
    var lines = [];
    for (var i = 0; i < 3; i++) {
      lines.push(i + ": " + input.topic);
    }
    return { content: lines.join("\n"), success: true };
  }
}
"#;
    nanna_scripting::check_syntax(good).expect("a well-formed skill must parse");
}
