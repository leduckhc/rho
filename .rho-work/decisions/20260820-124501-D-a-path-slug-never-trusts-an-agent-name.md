# D-a-path-slug-never-trusts-an-agent-name — a name is a label, and the id is the identity

**Question (step-9 security review):** The worktree path and the branch name both embed the
agent name. What validates that name?

**Nothing rejects it.** `name_warnings` only warns, and the loader states that a bad name still
loads. `sanitize` strips control characters and keeps `/`, `..`, a space, and a leading dash. A
user-scope definition in `~/.agents/agents` loads with no `--trust-project`. So a name of
`../../../../tmp/x` would place a worktree outside its directory, and reclaim would later remove
that path. A leading dash would arrive as an option to `git worktree add`.

**Decision:** The raw name never reaches a path or a git ref. rho builds a slug: keep
`[a-z0-9-]`, lowercase the rest, drop every other character, trim a leading and a trailing dash,
and truncate to 32 characters. An empty result becomes `agent`. The path and the branch then
carry the slug, the agent id, and a UTC timestamp.

**The id carries the identity, and the slug is only a label.** So a collision after slugging is
harmless, and a hostile name becomes an ugly label instead of a traversal.

**Rules out:** Any use of the raw name in a path, a ref, or a command argument. A design that
depends on the loader rejecting a bad name, when the loader deliberately keeps loading. A slug
that is the sole identity.
