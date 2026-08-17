//! Shared helpers for the skill tests.

#![allow(dead_code)]

use rho_skills::SkillConfig;
use std::fs;
use std::path::{Path, PathBuf};

/// Write a `SKILL.md` in `dir` with the given frontmatter body and instructions.
///
/// `frontmatter` is the YAML between the fences, without the fences. `body` is
/// the text after the closing fence.
pub fn write_skill(dir: &Path, frontmatter: &str, body: &str) {
    fs::create_dir_all(dir).unwrap();
    let content = format!("---\n{frontmatter}\n---\n{body}");
    fs::write(dir.join("SKILL.md"), content).unwrap();
}

/// Write a raw file, creating parent directories.
pub fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

/// A config that scans only the given user directories and session root.
pub fn config(user_dirs: Vec<PathBuf>, session_root: Option<&Path>) -> SkillConfig {
    SkillConfig {
        user_dirs,
        session_root: session_root.map(Path::to_path_buf),
        explicit: Vec::new(),
        project_trusted: false,
        discover: true,
    }
}
