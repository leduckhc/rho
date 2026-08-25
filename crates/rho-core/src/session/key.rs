//! The project key and the session id.
//!
//! See `SPEC-session-store-wiring` sections 3, 3a, and 4, and the decisions
//! `D-session-store-layout`, `D-a-project-key-cannot-escape-the-store`, and
//! `D-a-session-id-sorts-by-time`.
//!
//! Stage `core-red` transcribed this surface from the reviewed contract, with an
//! unimplemented body per function. Stage `core-green` made each body real.
//!
//! This note names no stub macro on purpose. The ship gate greps every crate source for
//! one, so a comment that spelled it would fail a gate that is meant to catch real stubs.
#![allow(dead_code)]

use std::ffi::OsStr;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::BufRead;
use std::path::{Path, PathBuf};

use crate::session::SessionError;

/// The most of a `.git` entry that `ProjectKey::resolve` reads.
///
/// A one-line file of ten megabytes is a denial of service, not a git directory.
pub const GIT_ENTRY_MAX_BYTES: usize = 4096;

/// The name for a key whose directory name sanitizes to nothing.
const NAME_PLACEHOLDER: &str = "project";

/// The `gitdir:` prefix of a `.git` file that points at a worktree entry.
const GITDIR_PREFIX: &str = "gitdir:";

/// A byte belongs in a directory name only inside this set.
fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')
}

/// Reduce a raw name to exactly one safe path segment.
///
/// It drops every byte outside `[A-Za-z0-9._-]`, then collapses any run of two or more
/// dots to one. So the result never holds `/`, `\`, or `..`. An empty result becomes the
/// placeholder, never an empty segment.
fn sanitize_name(raw: &str) -> String {
    let kept: String = raw
        .bytes()
        .filter(|&b| is_name_byte(b))
        .map(char::from)
        .collect();

    let mut out = String::with_capacity(kept.len());
    let mut last_was_dot = false;
    for ch in kept.chars() {
        if ch == '.' {
            if last_was_dot {
                continue;
            }
            last_was_dot = true;
        } else {
            last_was_dot = false;
        }
        out.push(ch);
    }

    if out.is_empty() || out == "." {
        NAME_PLACEHOLDER.to_string()
    } else {
        out
    }
}

/// Eight lowercase hex digits, a digest of the full identity path.
fn digest_hex(identity: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    identity.hash(&mut hasher);
    format!("{:08x}", hasher.finish() as u32)
}

/// Build the key from the resolved identity path.
///
/// The name is the last path segment, sanitized. The digest keeps two same-named
/// identities apart.
fn key_from_identity(identity: &Path) -> ProjectKey {
    let raw = identity
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = sanitize_name(&raw);
    ProjectKey(format!("{name}-{}", digest_hex(identity)))
}

/// Walk a `gitdir:` path up to the repository root.
///
/// A worktree `.git` file points at `<repo>/.git/worktrees/<name>`. The repository root is
/// the parent of the `.git` directory, so every worktree of one repository shares it. A
/// path with no `.git` component is returned as is, and the sanitizer keeps its name safe.
fn identity_from_gitdir(gitdir: &Path) -> PathBuf {
    for ancestor in gitdir.ancestors() {
        if ancestor.file_name() == Some(OsStr::new(".git"))
            && let Some(parent) = ancestor.parent()
        {
            return parent.to_path_buf();
        }
    }
    gitdir.to_path_buf()
}

/// The identity of one project. Every worktree of one repository shares it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectKey(String);

impl ProjectKey {
    /// Resolve the key for a project root.
    ///
    /// It reads the `.git` entry in `root`. A `.git` file holds one line, such as
    /// `gitdir: /path/to/main/.git/worktrees/name`. The function parses that line and
    /// walks up to the repository. So every worktree of one repository returns one key.
    ///
    /// It spawns no process, and it never fails. An unreadable `.git` falls back to the
    /// physical path of `root`.
    ///
    /// **A `.git` file is untrusted input.** So the read is bounded, and the key is
    /// sanitized. See section 3a.
    pub fn resolve(root: &Path) -> Self {
        let git = root.join(".git");
        match std::fs::metadata(&git) {
            Ok(meta) if meta.is_file() => match std::fs::File::open(&git) {
                Ok(file) => Self::resolve_from(std::io::BufReader::new(file), root),
                Err(_) => key_from_identity(root),
            },
            // A `.git` directory, or a missing or unreadable entry, keys on the root.
            _ => key_from_identity(root),
        }
    }

    /// Resolve the key from an open `.git` entry source and the project root.
    ///
    /// This is the `BufRead` seam. `resolve` opens `root/.git` and calls this. A test
    /// drives it with a source that counts the bytes it hands out, and asserts the count
    /// stays at or under `GIT_ENTRY_MAX_BYTES`. So the test fails against an
    /// implementation that reads the whole file and then truncates the string.
    pub fn resolve_from<R: BufRead>(git_entry: R, root: &Path) -> Self {
        // Read at most one line, and at most `GIT_ENTRY_MAX_BYTES` of it. A `.git` file is
        // untrusted input, so a ten megabyte line must never be read whole.
        let mut limited = git_entry.take(GIT_ENTRY_MAX_BYTES as u64);
        let mut buffer = Vec::new();
        let _ = limited.read_until(b'\n', &mut buffer);

        let line = String::from_utf8_lossy(&buffer);
        match line.trim().strip_prefix(GITDIR_PREFIX) {
            Some(path) => {
                let identity = identity_from_gitdir(Path::new(path.trim()));
                key_from_identity(&identity)
            }
            // A line that is not a gitdir entry falls back to the physical root.
            None => key_from_identity(root),
        }
    }

    /// The directory name for this key, `<directory-name>-<8 hex characters>`.
    ///
    /// It is always exactly one path segment. See section 3a.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The default store root, `<home>/.rho/sessions`.
///
/// The caller passes `home`, so a test never reads the real home directory.
pub fn default_store_root(home: &Path) -> PathBuf {
    home.join(".rho").join("sessions")
}

/// A session id, `<YYYYMMDD-HHMMSS>-<4 hex characters>`.
///
/// A sort of the file names gives newest first, so an ordered list reads no file.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SessionId(String);

impl SessionId {
    /// Mint an id from a time and four random hex characters.
    ///
    /// Both inputs are parameters, so a test mints a known id and never sleeps.
    pub fn mint(epoch_millis: u64, suffix: u16) -> Self {
        let (year, month, day, hour, minute, second) = civil_from_millis(epoch_millis);
        Self(format!(
            "{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}-{suffix:04x}"
        ))
    }

    /// Parse a whole id. A malformed id is a `SessionError::Decode`.
    pub fn parse(text: &str) -> Result<Self, SessionError> {
        if is_id_shape(text) {
            Ok(Self(text.to_string()))
        } else {
            Err(SessionError::Decode(format!("not a session id: {text:?}")))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Test the exact id shape `<YYYYMMDD-HHMMSS>-<4 lowercase hex>`.
fn is_id_shape(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() != 20 || bytes[8] != b'-' || bytes[15] != b'-' {
        return false;
    }
    let all_digits = |range: &[u8]| range.iter().all(u8::is_ascii_digit);
    let all_lower_hex = |range: &[u8]| {
        range
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
    };
    all_digits(&bytes[0..8]) && all_digits(&bytes[9..15]) && all_lower_hex(&bytes[16..20])
}

/// Convert epoch milliseconds to a UTC civil date and time.
///
/// rho has no date dependency, so this uses integer arithmetic. The day-to-date step is
/// Howard Hinnant's `civil_from_days` algorithm.
fn civil_from_millis(epoch_millis: u64) -> (i64, u32, u32, u32, u32, u32) {
    const SECONDS_PER_DAY: i64 = 86_400;
    let total_seconds = (epoch_millis / 1000) as i64;
    let days = total_seconds.div_euclid(SECONDS_PER_DAY);
    let seconds_of_day = total_seconds.rem_euclid(SECONDS_PER_DAY);

    let hour = (seconds_of_day / 3600) as u32;
    let minute = ((seconds_of_day % 3600) / 60) as u32;
    let second = (seconds_of_day % 60) as u32;

    // civil_from_days: days since 1970-01-01 to a proleptic Gregorian date.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365; // [0, 399]
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100); // [0, 365]
    let month_pivot = (5 * day_of_year + 2) / 153; // [0, 11]
    let day = (day_of_year - (153 * month_pivot + 2) / 5 + 1) as u32; // [1, 31]
    let month = if month_pivot < 10 {
        month_pivot + 3
    } else {
        month_pivot - 9
    } as u32; // [1, 12]
    let year = year + i64::from(month <= 2);

    (year, month, day, hour, minute, second)
}

/// What a prefix resolved to.
///
/// `Many` carries every match, so the error can list them. A prefix never picks one.
#[derive(Clone, Debug, PartialEq)]
pub enum PrefixMatch {
    One(SessionId),
    None,
    Many(Vec<SessionId>),
}
