# D-an-agent-symlink-cannot-smuggle-trust — one loader, one classification

**Question:** the skill loader resolves a path before it classifies it, so a symlink in
`~/.rho/skills` that points inside the session root is treated as a project skill. The agent
loader classified a file by the directory that found it. Does an agent definition need the
same rule?

## The probe

Thirty seconds, with the release binary:

```sh
ln -s /repo/evil.md ~/.rho/agents/evil.md
rho run "Say: probed." --root /repo          # no --trust-project
```

```text
rho: 1 agent definition(s) available to spawn_agent: evil.
```

A definition that lives in the repository loaded as trusted. It carries a tool list, a model
choice, and instructions, and it runs unattended. So this is worse than the skill hole that
`D-project-skill-needs-trust` closed, and the agent loader was the half that stayed open.

## The decision

`discover_agents` resolves a path before it classifies it, exactly as `discover` does. A file
found through a user directory that resolves inside the session root is a **project**
definition, so it is withheld until the user passes `--trust-project`.

`is_inside` in `crate::discover` becomes `pub(crate)`, and both loaders call it. **One
implementation**, because the two loaders already drifted once and a copy would let them drift
again.

One more thing follows. Trust is now decided in one function, `admit`, for the user pass and
the project pass together. Before, each pass had its own branch, and only the project branch
dropped a rejection detail.

## Rules out

**A second copy of the rule inside `agent.rs`.** That is how this hole appeared. The skill
loader was fixed and the agent loader was not, because the rule lived in one place and the
knowledge lived in a person.

**Refusing a symlink outright.** A user may link a definition on purpose, and a link into a
directory the user trusts is fine. The rule is about where the file **is**, not about how it
was found.

**Trusting the file because the link sits in a trusted directory.** A repository can arrive
with a `.rho/agents` full of anything. The link is the smuggling route, and the target decides.

## Cost

One call to a function that already existed, one `admit` function, and one test.
`a_symlink_from_a_user_dir_into_the_session_root_is_treated_as_a_project_agent`.
