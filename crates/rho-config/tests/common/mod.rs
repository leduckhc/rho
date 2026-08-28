//! Shared helpers for the `rho-config` tests.
//!
//! No test reads a real config path. Every file lives under a `tempfile::TempDir`.
//! Every environment lookup is an in-memory map, so no test mutates the process
//! environment.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use rho_config::{ConfigPaths, Sources};
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
    project_sources(path)
}

/// A `Sources` built from one project path, through the one constructor.
///
/// The fields are `pub(crate)`, so a test builds a `Sources` the same way the product
/// does. See `SPEC-config-call-site` section 2.
pub fn project_sources(path: PathBuf) -> Sources {
    Sources::from_paths(ConfigPaths {
        global: None,
        project: Some(path),
        ..Default::default()
    })
}

/// A `Sources` that reads no file at all, for an environment-only or flag-only test.
pub fn empty_sources() -> Sources {
    Sources::from_paths(ConfigPaths::default())
}
