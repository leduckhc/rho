//! Shared helpers for the `rho-config` tests.
//!
//! No test reads a real config path. Every file lives under a `tempfile::TempDir`.
//! Every environment lookup is an in-memory map, so no test mutates the process
//! environment.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use rho_config::Sources;
use tempfile::TempDir;

/// Make a temporary directory. It is removed when the returned handle drops.
pub fn temp_dir() -> TempDir {
    tempfile::tempdir().expect("a temp dir")
}

/// Write a file under `dir` and return its path.
pub fn write_file(dir: &TempDir, name: &str, contents: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, contents).expect("write the temp config file");
    path
}

/// Build an in-memory environment from string pairs.
pub fn env_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

/// Build the `RHO_*` variable list a `Sources` carries.
pub fn env_vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

/// A `Sources` that reads only one project file.
pub fn sources_with_project_file(path: PathBuf) -> Sources {
    Sources {
        project_file: Some(path),
        ..Sources::default()
    }
}
