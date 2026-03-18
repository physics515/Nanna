//! Build script for nanna-tools: generates embedded default skills for release builds.
//!
//! Scans `default-skills/` and emits a Rust source file containing all skill files
//! as compile-time embedded constants. This allows the daemon to bootstrap the tools
//! directory on first run without needing the source tree.

use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("embedded_skills.rs");
    let skills_dir = Path::new("default-skills");

    println!("cargo:rerun-if-changed=default-skills");

    let mut out = fs::File::create(&dest_path).expect("failed to create embedded_skills.rs");

    // Collect all skills (directories containing tool.ts)
    let mut entries: Vec<_> = Vec::new();

    if skills_dir.is_dir() {
        let mut dirs: Vec<_> = fs::read_dir(skills_dir)
            .expect("cannot read default-skills/")
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .collect();
        dirs.sort_by_key(|e| e.file_name());

        for entry in &dirs {
            let skill_name = entry.file_name();
            let skill_name = skill_name.to_string_lossy();
            let skill_path = entry.path();

            // Each file in the skill directory gets embedded
            let mut files: Vec<_> = fs::read_dir(&skill_path)
                .unwrap()
                .filter_map(|f| f.ok())
                .filter(|f| f.path().is_file())
                .collect();
            files.sort_by_key(|f| f.file_name());

            for file in &files {
                let file_name = file.file_name();
                let file_name = file_name.to_string_lossy();
                let rel_path = format!("default-skills/{}/{}", skill_name, file_name);

                entries.push((skill_name.to_string(), file_name.to_string(), rel_path));
            }

            // Also rerun if any individual skill file changes
            println!("cargo:rerun-if-changed=default-skills/{}", skill_name);
        }
    }

    // Write the embedded skills array
    writeln!(out, "/// A single embedded skill file.").unwrap();
    writeln!(out, "pub struct EmbeddedSkillFile {{").unwrap();
    writeln!(out, "    pub skill_name: &'static str,").unwrap();
    writeln!(out, "    pub file_name: &'static str,").unwrap();
    writeln!(out, "    pub content: &'static str,").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "pub const DEFAULT_SKILLS: &[EmbeddedSkillFile] = &["
    )
    .unwrap();

    for (skill_name, file_name, rel_path) in &entries {
        writeln!(out, "    EmbeddedSkillFile {{").unwrap();
        writeln!(out, "        skill_name: \"{}\",", skill_name).unwrap();
        writeln!(out, "        file_name: \"{}\",", file_name).unwrap();
        writeln!(
            out,
            "        content: include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/{}\")),",
            rel_path
        )
        .unwrap();
        writeln!(out, "    }},").unwrap();
    }

    writeln!(out, "];").unwrap();
}
