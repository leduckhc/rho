# D-a-workspace-hardens-every-git-call — hooks off, an explicit git directory, a scrubbed environment

**Question (step-9 security review, then a live probe):** A worktree's `.git` is a file inside
the child's own root, and `confine` allows the child to write it. What can a prompt-injected
child do to rho?

**What the probe showed.** It repointed its own `.git` at a repository it created, planted a
`post-commit` hook there, and waited. rho's ordinary `git commit` ran that hook as the rho
process. The hook saw four `AWS_` variables. The commit landed in the child's repository, and
the real branch stayed empty, so a report claiming a branch would have been false. The commands
and the real output are in `docs/verification/worktree-git-probe.md`.

**Decision:** Every git call a `Workspace` makes obeys four rules.

1. Hooks are off: `-c core.hooksPath=/dev/null`, and `--no-verify` where accepted.
2. The administrative directory is recorded at create time and passed as `--git-dir`, with
   `--work-tree` for the worktree. The `.git` file in the worktree is never read again.
3. The environment is scrubbed with the same scrub `bash` uses, at
   `crates/rho-tools/src/bash.rs:621`.
4. The committer identity is explicit, so a machine with no configured identity still keeps the
   work.

`GIT_TERMINAL_PROMPT=0` is set, so no git call waits for a human.

**Rule 2 is the load-bearing one.** Rule 1 stops the hook that exists today. Rule 2 makes the
whole class harmless, because rho never follows a pointer the child can write.

**A fifth rule, in depth.** The child's write path denies any path holding `.git`. It is not the
boundary. It is a second lock on a door that rule 2 already welded.

**Rules out:** Trusting `confine` alone for this. Reading the worktree `.git` file after create
time. A `Workspace` that inherits rho's environment. A commit that depends on the user's git
configuration. Any severity rating for this that stays a rating rather than a probe.
