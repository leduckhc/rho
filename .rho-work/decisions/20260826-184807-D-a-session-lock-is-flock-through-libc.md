# D-a-session-lock-is-flock-through-libc — one syscall, no new lock crate

**Question:** `D-a-live-session-holds-a-lock` says a live session holds an advisory lock.
`SPEC-session-store-wiring` section 7d states the type and the two errors. What holds the
lock?

## The decision

`flock(fd, LOCK_EX | LOCK_NB)` on unix, called through the `libc` crate.

`libc` joins `rho-core` as a dependency. It was added with `cargo add libc -p rho-core`,
and it was already in `Cargo.lock` through other crates, so the tree gains no new
compilation unit.

On a target that is not unix, `SessionStore::lock` returns
`SessionError::LockUnsupported`. That refuses the run, which is what section 7d asks for.

## Why flock, and not a crate

`fs2` is unmaintained. `fs4` works, and it would add a crate plus a version to keep for one
syscall. `libc` is already in the tree and it is the crate every alternative wraps.

`flock` is the same call fx uses, and fx is the prior art `D-a-live-session-holds-a-lock`
copied. So rho matches the tool it read.

## Rules that hold

- The lock is taken on a separate file, `<id>.lock`, and never on the session file. A
  lock on the session file would make the writer's own handle race with the lock handle,
  because two handles in one process share no `flock` state on every platform.
- The lock file is created with mode `0o600`, like every other file in the store.
- The lock is non-blocking. `EWOULDBLOCK` is `SessionError::Busy`. Every other error is
  `SessionError::LockUnsupported`, so a filesystem that cannot lock refuses.
- `SessionLock` releases the lock in `Drop`, and the operating system releases it when the
  process dies. So a crash never leaves a session locked.
- A read-only path takes no lock, so `list` and `show` always work.

## Rules out

**A lock file holding a pid.** A crash leaves it, and a user must then delete a file.

**`fcntl` record locking.** It is released by any `close` of any descriptor to the file in
the process, so an unrelated open and close would drop the lock in silence.

**Blocking with a timeout.** A user would wait with no message. A refusal that names the
other session is more useful, and `D-a-live-session-holds-a-lock` already ruled the queue
out.

## Cost

One dependency already in the lock file, one `unsafe` call, and one `Drop` impl.
