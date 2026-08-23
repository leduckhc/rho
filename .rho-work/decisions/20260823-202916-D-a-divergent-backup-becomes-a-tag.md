# D-a-divergent-backup-becomes-a-tag — never delete a branch that holds unique work

**Question:** `main` is pushed and green, and two `backup/*` branches remain. The user asked
to prune them. Is a delete safe?

**Decision:** check what a branch holds before you delete it, and turn a divergent one into
an `archive/*` tag instead of deleting it.

`backup/main-pre-rebase-20260823-200350` held the two commits that the rebase replayed. Same
content, new hashes. A delete loses nothing, so it was deleted.

`backup/main-pre-reset` held 27 commits that `main` does not contain. It came from an earlier
reset. Twenty-one test names in its `crates/rho-tui/tests/freeze.rs` exist nowhere under
`crates/` today, and they belong to a TUI design that `main` replaced with the alternate
screen. So the work is superseded, and it is not duplicated. It is now
`archive/main-pre-reset-20260823`, and the branch is gone from the branch list.

**How to tell the two apart.** `git merge-base --is-ancestor <branch> main` answers "is this
contained". When it says no, list the unique commits with `git log --oneline main..<branch>`,
and then check whether the content arrived by another route. A squash merge makes a fully
merged branch look divergent, so the commit list alone is not enough. Compare the files.

**Why a tag and not a branch.** A tag stays out of the branch list. So it does not read
as work in flight. It never moves, and it costs one ref. A branch invites a push.

**Rules out:** deleting a branch because a checklist step says prune. Trusting `git branch -d`
to refuse the unsafe case, because a squash merge makes it refuse a safe delete and a reset
makes `-D` accept an unsafe one. Keeping a divergent branch on `origin` to be safe, which
leaves a stale head in everyone's fetch.
