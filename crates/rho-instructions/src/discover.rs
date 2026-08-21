//! Gather every applicable instruction file.
//!
//! See `docs/specs/20260821-131725-SPEC-project-instructions.md` section 2.
//!
//! Every refusal in this module is fail-closed. A candidate rho cannot prove safe is
//! dropped with a reason, and never read.

use crate::types::{
    Instruction, InstructionConfig, InstructionOrigin, InstructionSet, MAX_OMISSION_RECORDS,
    Omission, OmissionReason, home_dir,
};
use std::path::{Component, Path, PathBuf};

/// What an ancestor-walk refusal names as its source.
///
/// A walk refusal never means the session root's own file was dropped, so it must not name
/// a path. See the live finding in `docs/verification/project-instructions.md`.
const ANCESTOR_WALK: &str = "the search of the directories above the session root";

/// Gather every applicable instruction file.
///
/// This never fails. An unreadable or unsafe candidate becomes an `Omission`, because a
/// missing project file must not stop a session.
pub async fn gather(config: &InstructionConfig) -> InstructionSet {
    let mut set = InstructionSet::default();
    let mut used_bytes = 0usize;

    // The home directory bounds the ancestor walk. `None` reads the environment. When that
    // fails, the walk covers zero directories. The ancestor cap never stands in for this
    // boundary, because a count bounds work and only home bounds reach.
    let home = config
        .home
        .clone()
        .or_else(home_dir)
        .and_then(|path| std::fs::canonicalize(path).ok());
    if home.is_none() {
        push_omission(
            &mut set,
            "HOME".to_string(),
            OmissionReason::HomeUnavailable,
        );
    }

    // Source one: the user file. `user_dir: None` resolves under home, and never means
    // "skip this source".
    let user_dir = config
        .user_dir
        .clone()
        .or_else(|| home.as_ref().map(|h| h.join(".config").join("rho")));
    if let Some(dir) = user_dir {
        load_from_dir(
            &dir,
            config,
            InstructionOrigin::User,
            &mut set,
            &mut used_bytes,
        )
        .await;
    }

    if !config.discover {
        return set;
    }

    let Some(session_root) = config.session_root.as_ref() else {
        push_omission(
            &mut set,
            "session root".to_string(),
            OmissionReason::NoSessionRoot,
        );
        return set;
    };
    let Ok(root) = std::fs::canonicalize(session_root) else {
        push_omission(
            &mut set,
            session_root.display().to_string(),
            OmissionReason::Unreadable,
        );
        return set;
    };

    // Sources two and three: the ancestors, broad first, then the session root last. The
    // root is not an ancestor, so it needs no boundary and always loads.
    for dir in ancestors(&root, home.as_deref(), config, &mut set) {
        load_from_dir(
            &dir,
            config,
            InstructionOrigin::Project,
            &mut set,
            &mut used_bytes,
        )
        .await;
    }
    load_from_dir(
        &root,
        config,
        InstructionOrigin::Project,
        &mut set,
        &mut used_bytes,
    )
    .await;

    set
}

/// The ancestor directories to read, ordered broad to narrow.
///
/// A directory qualifies when it lies above the session root and strictly below the home
/// directory. The home directory itself is never included, because its instruction file is
/// the user file and one file must not arrive under two origins.
fn ancestors(
    root: &Path,
    home: Option<&Path>,
    config: &InstructionConfig,
    set: &mut InstructionSet,
) -> Vec<PathBuf> {
    let Some(home) = home else {
        // The boundary is unknown, so no ancestor is in reach. `HomeUnavailable` is already
        // recorded by the caller.
        return Vec::new();
    };
    if !root.starts_with(home) || root == home {
        // Only the walk is refused. The session root's own file is still read by the
        // caller, so the record names the walk and never the file. A record that named the
        // file would tell the user their `AGENTS.md` was ignored when rho read it.
        push_omission(
            set,
            ANCESTOR_WALK.to_string(),
            OmissionReason::RootOutsideHome,
        );
        return Vec::new();
    }

    // Collect narrow to broad, so the cap keeps the directories nearest the root.
    let mut nearest_first = Vec::new();
    let mut current = root.parent();
    while let Some(dir) = current {
        if dir == home || !dir.starts_with(home) {
            break;
        }
        nearest_first.push(dir.to_path_buf());
        current = dir.parent();
    }

    if nearest_first.len() > config.limits.instruction_ancestor_cap {
        // The cap drops the broadest directories, never the root file.
        push_omission(set, ANCESTOR_WALK.to_string(), OmissionReason::AncestorCap);
        nearest_first.truncate(config.limits.instruction_ancestor_cap);
    }

    nearest_first.reverse();
    nearest_first
}

/// Read the first configured filename that exists in `dir`, and append it to the set.
///
/// One directory contributes at most one file.
async fn load_from_dir(
    dir: &Path,
    config: &InstructionConfig,
    origin: InstructionOrigin,
    set: &mut InstructionSet,
    used_bytes: &mut usize,
) {
    for filename in &config.filenames {
        // A filename is a file name. A separator or a parent reference is an escape, and a
        // config file is one place such a value could arrive from.
        if !is_plain_filename(filename) {
            push_omission(
                set,
                dir.join(filename).display().to_string(),
                OmissionReason::UnsafePath,
            );
            return;
        }
        let path = dir.join(filename);

        match read_candidate(&path).await {
            Candidate::Absent => continue,
            Candidate::Refused(reason) => {
                push_omission(set, path.display().to_string(), reason);
                return;
            }
            Candidate::Text(text) => {
                let observed_bytes = text.len();
                let (body, truncated) =
                    truncate_on_boundary(text, config.limits.instruction_file_bytes);

                if *used_bytes + body.len() > config.limits.instructions_total_bytes {
                    push_omission(set, path.display().to_string(), OmissionReason::TotalBudget);
                    return;
                }
                *used_bytes += body.len();

                set.delivered.push(Instruction {
                    path,
                    body,
                    origin,
                    observed_bytes,
                    truncated,
                });
                return;
            }
        }
    }
}

/// What one candidate path turned out to be.
enum Candidate {
    /// No such file. Try the next configured filename.
    Absent,
    /// The file exists and rho refuses it. The reason is reported to the user.
    Refused(OmissionReason),
    /// The whole file, as text.
    Text(String),
}

/// Read one candidate, refusing anything rho cannot prove safe.
///
/// The open is the enforcement, not the check before it. `O_NOFOLLOW` makes the kernel
/// refuse a symlink, so no attacker can swap a regular file for a link between a check and
/// a read. `O_NONBLOCK` stops a fifo from blocking the open forever, and the file type is
/// then read from the open descriptor rather than from the path. Both together close the
/// race that a `symlink_metadata` check alone would leave open.
async fn read_candidate(path: &Path) -> Candidate {
    use std::os::unix::fs::OpenOptionsExt;

    let opened = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path);

    let file = match opened {
        Ok(file) => file,
        Err(error) => {
            return match error.raw_os_error() {
                // `O_NOFOLLOW` reports a symlink this way.
                Some(libc::ELOOP) => Candidate::Refused(OmissionReason::Symlink),
                _ => match error.kind() {
                    // Only a genuinely absent file moves on quietly. Every other error is a
                    // real failure the user must hear about, because a file rho cannot open
                    // is not the same as a project with no rules.
                    std::io::ErrorKind::NotFound => Candidate::Absent,
                    _ => Candidate::Refused(OmissionReason::Unreadable),
                },
            };
        }
    };

    // The type comes from the descriptor rho already holds, so it cannot change under us.
    let Ok(meta) = file.metadata() else {
        return Candidate::Refused(OmissionReason::Unreadable);
    };
    if meta.file_type().is_symlink() {
        return Candidate::Refused(OmissionReason::Symlink);
    }
    if !meta.is_file() {
        return Candidate::Refused(OmissionReason::NonRegular);
    }

    let mut bytes = Vec::new();
    let read = tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut file = file;
        file.read_to_end(&mut bytes).map(|_| bytes)
    })
    .await;

    let bytes = match read {
        Ok(Ok(bytes)) => bytes,
        _ => return Candidate::Refused(OmissionReason::Unreadable),
    };

    // A non-UTF-8 file is refused, never delivered lossily. A lossy read would put
    // replacement characters into the model's contract and call it complete.
    match String::from_utf8(bytes) {
        Ok(text) => Candidate::Text(text),
        Err(_) => Candidate::Refused(OmissionReason::Unreadable),
    }
}

/// True when `name` is one plain file name, with no separator and no parent reference.
fn is_plain_filename(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    )
}

/// Keep at most `budget` bytes, cutting on a character boundary.
///
/// A cut through a multi-byte character would produce invalid UTF-8 and a provider error.
fn truncate_on_boundary(text: String, budget: usize) -> (String, bool) {
    if text.len() <= budget {
        return (text, false);
    }
    let mut end = budget;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut body = text;
    body.truncate(end);
    (body, true)
}

/// Record one omission, up to the record cap.
///
/// The cap bounds memory. It never turns a refusal into a pass, because the refusal has
/// already happened by the time this runs.
fn push_omission(set: &mut InstructionSet, source: String, reason: OmissionReason) {
    if set.omissions.len() >= MAX_OMISSION_RECORDS {
        return;
    }
    set.omissions.push(Omission { source, reason });
}
