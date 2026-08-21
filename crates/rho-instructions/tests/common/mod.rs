//! Shared helpers for the project instruction tests.
//!
//! Every test builds its own tree under `tempfile`, so no test reads the real home
//! directory. See AGENTS.md step 5.

#![allow(dead_code)]

use rho_instructions::{InstructionConfig, InstructionLimits};
use std::fs;
use std::path::{Path, PathBuf};

/// Write a file, creating parent directories.
pub fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

/// Write an `AGENTS.md` in `dir`.
pub fn write_instructions(dir: &Path, content: &str) {
    write_file(&dir.join("AGENTS.md"), content);
}

/// A config with an explicit home, so the test never reads the real environment.
pub fn config(home: &Path, session_root: &Path) -> InstructionConfig {
    InstructionConfig {
        filenames: vec!["AGENTS.md".to_string()],
        user_dir: None,
        session_root: Some(session_root.to_path_buf()),
        home: Some(home.to_path_buf()),
        discover: true,
        limits: InstructionLimits::default(),
    }
}

/// The delivered bodies, in order. Handy for an order assertion.
pub fn bodies(set: &rho_instructions::InstructionSet) -> Vec<String> {
    set.delivered.iter().map(|i| i.body.clone()).collect()
}

/// The delivered paths, in order.
pub fn paths(set: &rho_instructions::InstructionSet) -> Vec<PathBuf> {
    set.delivered.iter().map(|i| i.path.clone()).collect()
}

/// True when the set records the given reason.
pub fn has_reason(
    set: &rho_instructions::InstructionSet,
    reason: rho_instructions::OmissionReason,
) -> bool {
    set.omissions.iter().any(|o| o.reason == reason)
}

/// Canonicalise a temporary directory, so a comparison survives a symlinked `/tmp`.
pub fn real(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap()
}
