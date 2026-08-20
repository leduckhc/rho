# D-bash-no-path-confinement — `bash` path confinement stays out, and the reason is written down


The same audit listed a second open item: `bash` has no path confinement, because `cd`
and an absolute path both leave the session root.

**Decision. This stays as it is, and it is now documented rather than open.**

Confining a shell command is not a path check. It needs a container, a namespace, or a
`chroot`, because a shell can reach any path the user can reach, and it can do so
through a hundred routes. A partial check would be worse than none, since it would read
as a boundary while a single `cd ..` walked through it.

So `SPEC-tool-interface` section 7 now states the truth plainly: the approval policy is the only
real boundary for `bash`, credential scrubbing is defence in depth, and sprint 1 ships
no sandbox. `--read-only` denies `bash` outright, and that is the supported way to point
rho at a repository you do not trust.

A real sandbox is a design task with its own spec, not a patch. It is recorded in
`docs/non-goals.md` scope, not left as a silent gap.
