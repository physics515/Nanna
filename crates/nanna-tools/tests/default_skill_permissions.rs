//! Every bundled skill must declare its own `permissions.json`.
//!
//! A missing file does not mean "no permissions" — it means the widest ones.
//! `ScriptedTool::from_file` starts from `ToolPermissions::default()` (empty
//! read/write/net, `run: false`, `env: false`), but the daemon calls
//! `ensure_permissions` *before* loading skills, and that writes
//! `DEFAULT_PERMISSIONS_JSON` — `read: ["*"], write: ["*"], run: true,
//! net: ["*"], env: true` — into any skill directory lacking one. So the safe
//! default is never what a shipped skill actually gets: a forgotten file fails
//! **open**, silently, with nothing in the tree left to review.
//!
//! Not hypothetical. `create_tool` and `edit_tool` were added in one commit.
//! `create_tool` shipped `read: ["~"], write: ["~"]`; `edit_tool` shipped no
//! file at all — so the tool that rewrites other tools' source ran with
//! whole-filesystem read and write while its own sibling was scoped to home.
//!
//! The assertion is deliberately "the file exists and parses", not "the scopes
//! are narrow": 27 bundled skills are `~`-scoped and 13 legitimately need `*`,
//! so the reviewable property is that somebody chose, not which way they chose.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Fewest skills the tree can plausibly hold; below this the walk found
/// nothing and every assertion would pass for free. There were 43.
const MIN_BUNDLED_SKILLS: usize = 40;

/// `CARGO_MANIFEST_DIR` is `nanna-tools`, which owns `default-skills/`.
fn default_skills_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("default-skills")
}

/// Read a skill's permissions, or `None` if the file is absent or malformed —
/// the loader ignores a file it cannot deserialize, which leaves whatever
/// `ensure_permissions` wrote, so "malformed" and "missing" fail the same way.
fn read_permissions(dir: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(dir.join("permissions.json")).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let obj = value.as_object()?;
    // The five fields ToolPermissions deserializes; a file missing one of them
    // silently takes serde's default for it.
    for key in ["net", "read", "write"] {
        obj.get(key)?.as_array()?;
    }
    for key in ["env", "run"] {
        obj.get(key)?.as_bool()?;
    }
    Some(value)
}

/// Every directory under `default-skills/` that holds a `tool.ts`.
fn bundled_skills() -> Vec<PathBuf> {
    let dir = default_skills_dir();
    assert!(dir.is_dir(), "{dir:?} is not a directory");
    let mut skills: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {dir:?}: {e}"))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("tool.ts").is_file())
        .collect();
    skills.sort();
    skills
}

#[test]
fn every_bundled_skill_declares_its_permissions() {
    let skills = bundled_skills();
    assert!(
        skills.len() >= MIN_BUNDLED_SKILLS,
        "only {} bundled skills found — did the layout change?",
        skills.len(),
    );

    let offenders: Vec<String> = skills
        .iter()
        .filter(|dir| read_permissions(dir).is_none())
        .map(|dir| {
            dir.file_name()
                .map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into())
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "bundled skill(s) ship no usable permissions.json, so `ensure_permissions` \
         hands them read:[\"*\"] write:[\"*\"] run:true net:[\"*\"] env:true on \
         first run — the widest set, chosen by nobody: {offenders:?}",
    );
}

/// The two halves of tool authoring must not disagree about their reach.
/// `edit_tool` rewrites an existing tool's source in the same directory
/// `create_tool` writes a new one into, so a wider scope on either is an
/// accident rather than a decision — and it was one.
#[test]
fn the_tool_authoring_pair_share_one_scope() {
    let dir = default_skills_dir();
    let perms = |skill: &str| {
        read_permissions(&dir.join(skill))
            .unwrap_or_else(|| panic!("{skill} has no usable permissions.json"))
    };
    let create = perms("create_tool");
    let edit = perms("edit_tool");
    for field in ["read", "write"] {
        assert_eq!(
            create[field], edit[field],
            "create_tool and edit_tool disagree about {field} scope",
        );
    }
}
