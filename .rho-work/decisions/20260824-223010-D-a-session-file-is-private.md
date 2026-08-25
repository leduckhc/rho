# D-a-session-file-is-private — the store carries the transcript's mode bits

**Question:** rho is about to write a full conversation to disk on every run. What
permissions does the store get?

## The decision

- A session file is `0o600` on unix.
- Every directory rho creates under the store root is `0o700` on unix.
- A sidecar spill file gets the same mode as the session file.

rho already solved this once. `crates/rho-core/src/transcript.rs` sets a file to `0o600` at
line 135, and it walks the ancestors to `0o700` at line 126. Its test is
`permissions_are_0o600_on_unix`, at line 226. The store follows that precedent exactly.

## Why

A security review rated this high. A default umask makes a directory `0o755` and a file
`0o644`. Then any local user reads every conversation. A synced backup folder does too.

The first draft of the spec said nothing about mode bits. So the default would have been
the umask, and nobody would have noticed until a real machine leaked.

## Rules that hold

- The mode is set by `create`, not by a caller. A caller cannot forget it.
- A test asserts each mode, and it mirrors the transcript test by name.
- The session id is not an access control. Four hex characters stop a collision, and the
  directory mode stops a reader.

## Rules out

**Trusting the umask.** It is a user setting, and this is a security boundary.

**Encryption at rest in this lane.** It needs a key, and a key needs a home. The spec
states the gap instead of half solving it.

## Cost

Two `set_permissions` calls on unix, and three tests.
