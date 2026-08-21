# D-isolation-granted-only-by-caller — Only a trusted caller grants a child a worktree

**The question.** Who may give a child an isolated working tree? An agent definition, the
model, or only a trusted caller?

**The decision.** Only a trusted caller grants isolation. A caller grants it by installing a
`Workspace`, by passing `--isolate-subagents`, or by setting `subagents.isolate = true`. A
definition may refuse with `isolation: off`. The model may refuse with `isolate: false`.
Neither a definition nor the model may grant.

**Why the caller grants.** Granting moves the child's session root. The root is a security
boundary. Granting also creates a git branch and uses disk. So granting is both a boundary
choice and a resource choice. Only the caller owns both.

**Why a refusal is safe.** A refused child runs in the parent tree. That tree is the parent's
own reach. A child is never more permissive than its parent. So a refused child stays inside
the parent's reach. A refusal removes a worktree. A refusal removes no restriction.

**Why no silent fallback.** A caller asked for isolation. An isolation failure must not put
the child in the shared tree without consent. A silent fallback would widen the blast radius.
So rho reports the failure as a result. The parent chooses again.

**What this rules out.** A definition cannot demand a worktree. The model cannot demand a
worktree. rho cannot fall back to the shared tree on its own. A child cannot use isolation to
reach a tree the caller did not grant.
