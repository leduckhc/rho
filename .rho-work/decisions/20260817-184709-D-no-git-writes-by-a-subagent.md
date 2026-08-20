# D-no-git-writes-by-a-subagent — A subagent must never run a git command that changes the working tree


**What happened.** The S6 developer ran `git stash` to isolate its own crates from
a parallel agent's uncommitted work. The stash displaced the S7 agent's live
files. The S6 agent noticed, restored the files and the unstaged state, and
dropped the stash. Nothing was lost, and it reported the mistake plainly. But the
S7 agent could have failed for a reason it could never diagnose.

**Decision.** A subagent runs no git command that writes. Read-only commands stay
allowed: `git diff`, `git status`, `git log`, `git show`. Forbidden, without
exception: `stash`, `checkout`, `restore`, `reset`, `clean`, `commit`, `add`,
`rebase`, `merge`. The controller owns the index and the working tree.

**The deeper cause, and the real fix.** Parallel agents share one checkout, so
`cargo test --workspace` fails whenever a sibling's work in progress does not
compile. That failure has nothing to do with the stage being gated. Two rules
follow.

1. **Gate a stage on its own crates**, not on the workspace, while a sibling is
   mid-flight. Use `cargo test -p <crate> ...`. The controller runs the full
   workspace gate once, after the parallel wave lands.
2. For a future sprint, run each parallel implementation stage in its own git
   worktree. The controller then merges. That removes the race, at the cost of a
   rebuild per worktree.

**Brief change.** Every future stage brief states the per-crate gate commands, and
states that a workspace gate is the controller's job.
