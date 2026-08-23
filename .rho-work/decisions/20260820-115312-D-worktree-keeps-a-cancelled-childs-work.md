# D-worktree-keeps-a-cancelled-childs-work — A cancelled or timed-out child keeps its changes

**The question.** A cancelled child and a timed-out child both leave a worktree. It may hold
uncommitted work. What happens to that work?

**The decision.** Reclaim always keeps the child's changes. On cancel, reclaim commits the
work to a branch. On timeout, reclaim commits the work to a branch. The report names the
branch. The outcome stays `Canceled` or `OutOfTurns`.

**When a commit is not safe.** rho leaves the tree on disk. It sets `isolation_root` to the
worktree path. It sets `branch` to `None`. The summary names the leftover tree. A human
recovers it by hand.

**Why.** A cancelled child may have done useful work. A silent delete would waste that work.
A silent delete would also hide it. Losing a child's work with no trace is the worst outcome
here. A named branch or a named tree lets the parent inspect the result.

**How this fits rho's rule.** A child failure is a result, not the end of the run. See
`SPEC-subagents` section 8. A cancelled child is one more result the parent can act on.

**What this rules out.** Cancel cannot discard a child's changes. A timeout cannot discard a
child's changes. rho cannot delete a worktree that still holds unkept work.
