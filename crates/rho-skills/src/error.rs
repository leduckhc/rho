//! Errors for skill loading.

use std::path::PathBuf;

/// An error while reading a skill body on demand.
#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    /// The skill file could not be read. The file may have been removed.
    #[error("cannot read the skill file {path}: {source}. Check that the file still exists.")]
    Io {
        /// The path that failed.
        path: PathBuf,
        /// The underlying input or output error.
        #[source]
        source: std::io::Error,
    },
}
