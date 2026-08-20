# D-jcode-bash-lessons — What reading jcode's `bash` tool changed


Four adoptions, one deliberate omission, and one defect that only a live run found.

**Adopted.** A disk-backed scratch directory through `TMPDIR` and `RHO_SCRATCH_DIR`,
because `/tmp` is a tmpfs on most Linux systems and a build there spends the memory this
project exists to save. A timeout message that names the millisecond unit. A byte-ratio
progress shape, `1.5/3.0 GiB`. A phase-line progress shape, `Compiling ...`.

**The live finding.** The timeout message was nearly unreachable. rho adopts a command
that outruns its foreground timeout, and it backgrounds a command matching a long-running
shape before the timeout applies. So `ToolError::Timeout` almost never fires in a real
session, and a `timeout_ms` of 1000 silently became a background task. The model
concluded it had asked for one.

The fix moved the hint from the error to **any background start**, keyed on the value
rather than the reason. Two rounds of live testing were needed, because the first fix
covered only the adoption path and the shape heuristic fired first.

**Not adopted, and why.** jcode wraps `cargo` through its own repository script, which
encodes its build policy and does not generalise.

jcode also detects when a child is waiting on standard input, then asks the user for a
line. rho gives the child a null stdin, so an interactive command hangs instead. That is
a real gap and the feature is genuinely good. It is also about 180 lines of per-platform
unsafe code, reading `/proc` on Linux and libproc through FFI on macOS. So it needs its
own spec and its own test plan. It is recorded here as a known gap rather than half-built.

**A process lesson, and it is mine.** While verifying one of these tests, the controller
ran `git checkout` on a file with uncommitted work, and destroyed the change it had just
written. A copy of the file happened to exist in `/tmp`, so only one edit was lost. This
is decision D-shared-working-tree, one commit after writing it. **Verify a guard by copying the file,
never by asking git to restore it.**
