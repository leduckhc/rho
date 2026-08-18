//! One-way import of a pi session file into rho's record set.
//!
//! Feature F-54. See `SPEC-14` section 9. The conversion is one-way: it reads a pi
//! JSONL file and returns rho `Entry` records. It never changes the pi file.
//!
//! Stage T4 defines the public surface with a `todo!()` body. A later stage makes it
//! real.

use std::path::Path;

use rho_core::{Entry, SessionError};

/// Convert a pi session file to rho records.
///
/// It reads the JSONL file at `pi_path` and maps each pi record to a rho `Record`,
/// per the mapping in `SPEC-14` section 9. It keeps every `id`, every `parentId`, and
/// every `timestamp`, so the tree shape survives. It drops only the known set:
/// `thinking_level_change`, `session_info`, and `custom`. It never changes the pi file.
pub fn import_pi_session(pi_path: &Path) -> Result<Vec<Entry>, SessionError> {
    let _ = pi_path;
    todo!("F-54: SPEC-14 section 9 pi import is not implemented yet")
}
