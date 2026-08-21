# D-an-orphan-is-capped-not-deleted — never delete a leftover, and never let leftovers grow

**Question (step-9 security review):** A crash leaves a worktree and a branch, and the spec says
a later run never deletes an orphan. What bounds the disk?

**Nothing did.** Every crash left a tree and a branch for ever. That is the unbounded-growth
family that already cost this project 805 MB, in a slower and more expensive form.

**Decision:** rho counts the orphans under `.rho/worktrees/` before it creates a new tree. Past
`MAX_ISOLATION_ORPHANS`, which is 32, it refuses with `TooManyOrphans` and names the directory to
clean. It still never deletes an orphan.

**Reason:** The two failures are not equal. Deleting an orphan can destroy work a human has not
recovered, and this project already destroyed a parallel worker's files once. Filling a disk is
recoverable and visible. So rho keeps every leftover, and stops adding more.

**Removal always goes through git.** `git worktree remove` refuses a tree git does not know, and
refuses a dirty or locked tree. A recursive delete would follow a symlink the child created. So
reclaim never deletes a path git does not list.

**Rules out:** Automatic cleanup of an orphan. An unbounded orphan directory. A bare recursive
delete anywhere in the reclaim path.
