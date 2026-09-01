//! The starred-models file: read and write the user's picker favourites.
//!
//! The file lives at `~/.rho/starred-models.toml`. Format is a single top-level array:
//!
//! ```toml
//! starred = [
//!   "anthropic/claude-sonnet-4-6",
//!   "amazon.nova-micro-v1:0",
//! ]
//! ```
//!
//! rho owns every write. See `D-starred-models-live-in-their-own-file`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The default path, resolved as `$HOME/.rho/starred-models.toml`. `None` when the
/// environment names no home. The caller checks and pushes a notice.
///
/// It reads only `HOME`, so a test overrides it. rho keeps every user-scoped file under
/// `~/.rho/`, so this joins the same root that `config.toml` lives in.
pub fn default_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".rho/starred-models.toml"))
}

/// Read the starred list from `path`. A missing file returns an empty list, never an
/// error. A parse error returns a message the caller pushes as a notice. rho never
/// overwrites a file it could not read.
pub fn load(path: &Path) -> Result<Vec<String>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "starred-models: cannot read {}: {error}",
                path.display()
            ));
        }
    };
    let text = std::str::from_utf8(&bytes).map_err(|error| {
        format!(
            "starred-models: {} is not valid utf8: {error}",
            path.display()
        )
    })?;
    let file: StarredFile = toml::from_str(text)
        .map_err(|error| format!("starred-models: {} does not parse: {error}", path.display()))?;
    Ok(file.starred)
}

/// Write the starred list to `path`, atomically. Writes to a sibling temp file first, then
/// renames it into place. A rename on the same filesystem is atomic, so a crash never
/// leaves a half-written file.
pub fn save(path: &Path, starred: &[String]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = StarredFile {
        starred: starred.to_vec(),
    };
    // The unwrap holds because `StarredFile` has one field, both `Serialize`.
    let body = toml::to_string_pretty(&file).expect("a plain list of strings serialises");
    let mut temp = path.to_path_buf();
    let file_name = path
        .file_name()
        .map(|name| name.to_owned())
        .unwrap_or_else(|| std::ffi::OsString::from("starred-models.toml"));
    let mut temp_name = std::ffi::OsString::from(".");
    temp_name.push(&file_name);
    temp_name.push(".tmp");
    temp.set_file_name(&temp_name);
    {
        let mut out = fs::File::create(&temp)?;
        out.write_all(body.as_bytes())?;
        out.sync_all()?;
    }
    fs::rename(&temp, path)?;
    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
struct StarredFile {
    #[serde(default)]
    starred: Vec<String>,
}
