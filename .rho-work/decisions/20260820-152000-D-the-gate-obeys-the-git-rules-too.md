# D-the-gate-obeys-the-git-rules-too — one hardening rule, two enforcement points

**Question (second-pass security review):** The four git rules bind reclaim. What binds the gate?

**Nothing did, and that re-opened the probed attack.** The gate runs **before** reclaim, and an
`ArtifactSpec::Command` check runs through `SandboxedRunner` with the child's root as its working
directory. A check that runs git, directly or through a build tool or a lint, reads the worktree
`.git` file. The child may have rewritten that file, and the probe in
`docs/verification/worktree-git-probe.md` shows the result: the child's hook runs as rho, with
rho's environment, before reclaim ever starts.

**The trusted-author rule does not cover this.** `D-an-acceptance-check-has-a-trusted-author`
makes the command **string** trusted. The attack needs no untrusted string. A benign check that
runs `git diff --exit-code` is enough, because the child rewrote a pointer at run time.

**Decision:** For an isolated child the gate runs every check with `GIT_DIR` set to the recorded
administrative directory, `GIT_WORK_TREE` set to the worktree, `core.hooksPath` neutralised, and
`GIT_CONFIG_NOSYSTEM=1`. So a check cannot reach a pointer or a hook the child controls, whatever
it shells out to.

**One rule, two points.** The hardening is a property of every git call rho causes, not a property
of reclaim. A rule written for one caller is a rule the next caller forgets.

**Rules out:** Hardening reclaim alone. Trusting the author of a check to defend against the
child's runtime writes. Any git call in an isolated child that reads the worktree pointer.
