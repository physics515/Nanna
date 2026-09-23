#![warn(clippy::pedantic, clippy::nursery, clippy::all)]
//! Source-tree guard: every `.rs` file under a crate's `src/` must be reachable
//! from that crate's root module.
//!
//! A `.rs` file that no `mod` declaration names is not "unused code" — it is
//! **uncompiled** code. `cargo check`, `clippy`, `cargo fmt` and the test suite
//! never see it, so it can drift arbitrarily far from compiling while still
//! reading, in an editor or a review diff, like shipped behaviour. That is the
//! expensive part: the file looks like an answer to whoever finds it next.
//!
//! This is not hypothetical. The four files this guard was written against
//! (`src/daemon_launcher.rs`, `src/updater.rs`, `src/webview2.rs`,
//! `crates/nanna-gpu/src/batch_processor.rs`) had accumulated **12 compile
//! errors** between them — an unresolved `tauri_plugin_updater` import in a
//! crate that does not depend on it, a `std::process::Output` assigned to an
//! `Option<Child>`, a call to a `regex_pattern` function that does not exist,
//! and a `parking_lot` path in a crate without that dependency. None of it had
//! ever been compiled.
//!
//! Lives beside `dep_guard.rs` / `dep_version_unification.rs` for the same
//! reason they do: `nanna-storage` is cheap to build and already inside CI's
//! `cargo test` scope, so a plain `cargo test` enforces the invariant with no
//! extra job. It reads the source tree, not this crate.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Deepest module nesting the walker will follow. Rust module trees in this
/// workspace are 3 levels at most (`lib.rs` → `control/mod.rs` → `handlers.rs`);
/// 32 is far above anything real, and bounds the walk against a `#[path]` cycle
/// that points a module at one of its own ancestors.
const MAX_MODULE_DEPTH: usize = 32;

/// Lower bound on the reachable-file count, so a parser that silently stopped
/// matching `mod` declarations fails loudly instead of reporting a clean tree.
/// The workspace had 300+ reachable source files when this was written.
const MIN_REACHABLE_FILES: usize = 200;

/// Workspace root (`CARGO_MANIFEST_DIR` = `<root>/crates/nanna-storage`).
fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("crate should live two levels below the workspace root")
        .to_path_buf()
}

/// Every workspace package directory: the `members` list plus the root package
/// itself, which is a package but is not listed as a member.
fn package_dirs(root: &Path) -> Vec<PathBuf> {
    let manifest = std::fs::read_to_string(root.join("Cargo.toml"))
        .expect("workspace Cargo.toml must be readable");
    let members_block = manifest
        .split_once("members = [")
        .map(|(_, rest)| rest)
        .and_then(|rest| rest.split_once(']'))
        .map(|(block, _)| block)
        .expect("workspace Cargo.toml must declare `members = [ .. ]`");

    let mut dirs = vec![root.to_path_buf()];
    for line in members_block.lines() {
        let trimmed = line.trim().trim_end_matches(',').trim_matches('"');
        if trimmed.is_empty() {
            continue;
        }
        assert!(
            !trimmed.contains('*'),
            "member glob `{trimmed}` is not supported by this guard — expand it \
             or teach the guard to walk globs",
        );
        dirs.push(root.join(trimmed));
    }
    dirs
}

/// Strip comments so a `mod` inside one is never mistaken for a declaration.
/// Block comments nest in Rust, so the depth is counted rather than toggled.
fn strip_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut depth = 0_usize;
    let mut idx = 0_usize;
    while idx < bytes.len() {
        let two = bytes.get(idx..idx + 2);
        if two == Some(b"/*") {
            depth += 1;
            idx += 2;
        } else if two == Some(b"*/") && depth > 0 {
            depth -= 1;
            idx += 2;
        } else if depth > 0 {
            // Keep newlines so line-oriented matching below stays aligned.
            if bytes[idx] == b'\n' {
                out.push('\n');
            }
            idx += 1;
        } else if two == Some(b"//") {
            while idx < bytes.len() && bytes[idx] != b'\n' {
                idx += 1;
            }
        } else {
            out.push(char::from(bytes[idx]));
            idx += 1;
        }
    }
    out
}

/// The module name in `[pub[(..)]] mod NAME;`, or `None` for anything else
/// (including inline `mod NAME { .. }`, which declares no file).
fn file_module_name(line: &str) -> Option<&str> {
    let mut rest = line.trim();
    if let Some(after) = rest.strip_prefix("pub") {
        rest = after.trim_start();
        if rest.starts_with('(') {
            rest = rest.split_once(')')?.1.trim_start();
        }
    }
    let name = rest.strip_prefix("mod ")?.trim();
    let name = name.strip_suffix(';')?.trim();
    let is_ident = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    is_ident.then_some(name)
}

/// The path in a `#[path = "..."]` attribute on this line, if any.
fn path_attribute(line: &str) -> Option<&str> {
    let rest = line.trim().strip_prefix("#[path")?.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    rest.split_once('"').map(|(value, _)| value)
}

/// Follow `mod` declarations from `entry`, inserting every file reached.
fn walk(entry: &Path, depth: usize, reached: &mut BTreeSet<PathBuf>) {
    assert!(
        depth < MAX_MODULE_DEPTH,
        "module nesting exceeded {MAX_MODULE_DEPTH} at {} — a `#[path]` cycle?",
        entry.display(),
    );
    if !entry.is_file() || !reached.insert(entry.to_path_buf()) {
        return;
    }
    let Ok(source) = std::fs::read_to_string(entry) else {
        return;
    };
    let dir = entry.parent().unwrap_or_else(|| Path::new("."));
    let stem = entry.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    // `lib.rs`/`main.rs`/`mod.rs` own their own directory; any other file owns
    // a subdirectory named after it.
    let child_dir = if matches!(stem, "lib" | "main" | "mod") {
        dir.to_path_buf()
    } else {
        dir.join(stem)
    };

    let cleaned = strip_comments(&source);
    let mut pending_path: Option<String> = None;
    for line in cleaned.lines() {
        if let Some(value) = path_attribute(line) {
            pending_path = Some(value.to_string());
            continue;
        }
        let Some(name) = file_module_name(line) else {
            // Only an attribute line carries a `#[path]` forward; anything else
            // between it and its `mod` would mean we mis-parsed.
            if !line.trim().is_empty() && !line.trim().starts_with("#[") {
                pending_path = None;
            }
            continue;
        };
        if let Some(rel) = pending_path.take() {
            walk(&dir.join(rel), depth + 1, reached);
            continue;
        }
        for candidate in [
            child_dir.join(format!("{name}.rs")),
            child_dir.join(name).join("mod.rs"),
        ] {
            if candidate.is_file() {
                walk(&candidate, depth + 1, reached);
                break;
            }
        }
    }
}

/// Every `.rs` file under `dir`, recursively.
fn rust_files_under(dir: &Path) -> BTreeSet<PathBuf> {
    let mut found = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(rust_files_under(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.insert(path);
        }
    }
    found
}

/// Compilation entry points for a package: `src/lib.rs`, `src/main.rs`, and
/// each extra binary under `src/bin/`.
fn entry_points(src: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = ["lib.rs", "main.rs"]
        .iter()
        .map(|name| src.join(name))
        .filter(|p| p.is_file())
        .collect();
    entries.extend(rust_files_under(&src.join("bin")));
    entries
}

#[test]
fn every_source_file_is_reachable_from_its_crate_root() {
    let root = workspace_root();
    let packages = package_dirs(&root);
    assert!(
        packages.len() > 1,
        "parsed {} package dirs from the workspace manifest — parser broken?",
        packages.len(),
    );

    let mut orphans: Vec<PathBuf> = Vec::new();
    let mut reachable_total = 0_usize;

    for package in &packages {
        let src = package.join("src");
        if !src.is_dir() {
            continue;
        }
        let entries = entry_points(&src);
        assert!(
            !entries.is_empty(),
            "{} has a src/ directory but neither lib.rs nor main.rs",
            package.display(),
        );

        let mut reached = BTreeSet::new();
        for entry in &entries {
            walk(entry, 0, &mut reached);
        }
        reachable_total += reached.len();
        orphans.extend(rust_files_under(&src).difference(&reached).cloned());
    }

    assert!(
        reachable_total >= MIN_REACHABLE_FILES,
        "only {reachable_total} reachable source files found (expected at least \
         {MIN_REACHABLE_FILES}) — the `mod` parser stopped matching, so this \
         guard would pass vacuously",
    );

    let listed: Vec<String> = orphans
        .iter()
        .map(|p| {
            p.strip_prefix(&root)
                .unwrap_or(p)
                .display()
                .to_string()
                .replace('\\', "/")
        })
        .collect();
    assert!(
        listed.is_empty(),
        "these source files are not reachable from any crate root, so nothing \
         compiles them — not `cargo check`, not clippy, not `cargo fmt`, not the \
         test suite:\n  {}\n\nEither declare the module (`mod <name>;` in the \
         parent) or delete the file. An uncompiled .rs file reads like shipped \
         behaviour and is not.",
        listed.join("\n  "),
    );
}

// ---------------------------------------------------------------------------
// The guard is only as good as its parser, and a parser that quietly stops
// matching turns this file into a test that always passes. These pin the three
// cases that would do that.
// ---------------------------------------------------------------------------

#[test]
fn mod_declarations_are_recognised_in_every_visibility_form() {
    assert_eq!(file_module_name("mod plain;"), Some("plain"));
    assert_eq!(file_module_name("pub mod public;"), Some("public"));
    assert_eq!(
        file_module_name("    pub(crate) mod scoped;"),
        Some("scoped")
    );
    assert_eq!(file_module_name("pub(super) mod up;"), Some("up"));
    assert_eq!(
        file_module_name("mod with_underscore_9;"),
        Some("with_underscore_9")
    );

    // An inline module declares no file, so it must not be treated as one.
    assert_eq!(file_module_name("mod inline {"), None);
    // Nor may anything that merely mentions the word.
    assert_eq!(file_module_name("use crate::mod_utils;"), None);
    assert_eq!(file_module_name("let modern = 1;"), None);
    assert_eq!(file_module_name(""), None);
}

#[test]
fn comments_cannot_fake_a_mod_declaration() {
    // A commented-out `mod` is the dangerous direction: if it counted, the
    // orphan it names would be reported as reachable and slip through.
    assert_eq!(
        strip_comments("// mod ghost;\nmod real;").trim(),
        "mod real;"
    );
    assert_eq!(
        strip_comments("/* mod ghost; */\nmod real;").trim(),
        "mod real;"
    );
    // Rust block comments nest — a naive toggle would reopen here and swallow
    // the declaration that follows.
    let nested = strip_comments("/* outer /* inner */ still */\nmod real;");
    assert!(
        nested.contains("mod real;"),
        "nested comment ate the declaration: {nested:?}"
    );
    assert!(
        !nested.contains("outer"),
        "nested comment leaked: {nested:?}"
    );
    // Newlines inside a stripped comment are preserved, so the line the
    // declaration after it lands on is still the line it was on.
    let spanning = strip_comments("/* a\nb\nc */\nmod real;");
    assert_eq!(
        spanning.matches('\n').count(),
        3,
        "line alignment lost: {spanning:?}"
    );
    assert_eq!(spanning.lines().nth(3), Some("mod real;"));
}

#[test]
fn path_attributes_are_read_off_their_own_line() {
    assert_eq!(
        path_attribute(r#"#[path = "elsewhere.rs"]"#),
        Some("elsewhere.rs")
    );
    assert_eq!(
        path_attribute(r#"  #[path="nested/file.rs"]  "#),
        Some("nested/file.rs")
    );
    assert_eq!(path_attribute("#[cfg(unix)]"), None);
    assert_eq!(path_attribute("mod plain;"), None);
}
