//! The advisory lock that stops two processes writing one session.
//!
//! See `SPEC-session-store-wiring` section 7d, `D-a-live-session-holds-a-lock`, and
//! `D-a-session-lock-is-flock-through-libc`.
//!
//! Two worktrees share one project key, so `--continue` in both can open one file. Both would
//! seed their record ids from the same read, both would mint the same ids, and the lines would
//! interleave. fx is the only tool read for this project that solved the same race, and it
//! solved it with a per-session advisory lock. rho copies that shape.

use std::fs::{File, OpenOptions};
use std::path::Path;

use crate::session::SessionError;

/// A held advisory lock on one session.
///
/// It is released on drop, and by the operating system when the process dies. So a crash never
/// leaves a session locked for ever, which a plain lock file with a pid inside would.
pub struct SessionLock {
    /// The open descriptor that holds the lock. Dropping it releases the lock.
    ///
    /// The path is **not** kept. Nothing read it, and a field no reader wants is dead surface.
    /// See `D-dead-surface-is-a-defect-class`. A refusal names the path from the error instead.
    file: File,
}

impl Drop for SessionLock {
    fn drop(&mut self) {
        // `flock` is released when the last descriptor for the open file closes, which the
        // `File` drop does. The explicit unlock makes the release visible in this file, so a
        // reader does not have to know that rule.
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            // Safety: the descriptor is owned by `self.file` and is open for as long as this
            // call runs.
            unsafe {
                libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
            }
        }
    }
}

/// Take a non-blocking exclusive advisory lock on `path`.
///
/// The lock sits on a separate `<id>.lock` file, and never on the session file itself. A lock
/// on the session file would race with the writer's own handle, because two descriptors in one
/// process do not share `flock` state on every platform.
///
/// `EWOULDBLOCK` is `SessionError::Busy`. Every other failure is
/// `SessionError::LockUnsupported`, so a filesystem that cannot lock **refuses** the run. A
/// warning that continued would fail open, and that is the shape of
/// `D-plugin-does-not-classify-itself`.
pub(crate) fn take_lock(path: &Path, session_id: &str) -> Result<SessionLock, SessionError> {
    // The mode goes on the open call, so the lock file is never briefly world readable. It holds no
    // content, and it sits beside a private conversation, so it gets the same care.
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(|_| SessionError::LockUnsupported {
            path: path.to_path_buf(),
        })?;
    // A lock file can already exist, from a run that crashed, and `OpenOptions::mode` applies at
    // creation only. So this one keeps the explicit call, and it is not redundant here.
    crate::session::set_owner_only(path)?;

    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        // Safety: the descriptor is owned by `file` and is open for as long as this call runs.
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result != 0 {
            let code = std::io::Error::last_os_error().raw_os_error();
            return Err(classify_lock_failure(code, path, session_id));
        }
        Ok(SessionLock { file })
    }
    // A target with no `flock` cannot promise the invariant, so it refuses rather than
    // pretending. See section 7d.
    #[cfg(not(unix))]
    {
        let _ = (file, session_id);
        Err(SessionError::LockUnsupported {
            path: path.to_path_buf(),
        })
    }
}

/// Turn a `flock` failure into the error the contract names.
///
/// It is a separate function because it cannot be reached from a test otherwise. A filesystem
/// that refuses to lock is hard to arrange on a developer machine, and the branch that handles
/// it is the fail-open risk: a warning that continued would be the shape of
/// `D-plugin-does-not-classify-itself`. So the classification is pure, and a test drives it with
/// each error code.
///
/// `EWOULDBLOCK` and `EAGAIN` mean another process holds the lock. **Everything else is a
/// refusal**, including an unknown code, so a filesystem rho does not understand stops the run.
pub fn classify_lock_failure(code: Option<i32>, path: &Path, session_id: &str) -> SessionError {
    #[cfg(unix)]
    if let Some(code) = code
        && (code == libc::EWOULDBLOCK || code == libc::EAGAIN)
    {
        return SessionError::Busy {
            id: session_id.to_string(),
        };
    }
    #[cfg(not(unix))]
    let _ = (code, session_id);
    SessionError::LockUnsupported {
        path: path.to_path_buf(),
    }
}

/// Is this session locked by another process right now?
///
/// It takes the lock and drops it at once. A success means nobody held it at that moment.
///
/// **This is a probe, and not a reservation.** A review named the race, and it is real: a session
/// that was free here can be taken before the caller takes the lock for its own write. The caller
/// then gets `SessionError::Busy` and stops, which names the session and tells the user what to do.
///
/// The invariant that matters still holds. The real lock is taken **before** any write, in
/// `SessionStore::lock`, so two processes never write one file. The probe only chooses which
/// candidate to try first, and losing that race costs one error message and never a corrupt file.
///
/// A reservation would need `newest_resumable` to return a held lock. That changes the return type
/// of a read-only query into a resource, so a caller that only wanted to know the id would take a
/// lock it must remember to drop. The cost is worse than the race.
pub(crate) fn is_locked_elsewhere(path: &Path, session_id: &str) -> bool {
    matches!(take_lock(path, session_id), Err(SessionError::Busy { .. }))
}
