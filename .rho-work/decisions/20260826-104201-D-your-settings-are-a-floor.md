# D-your-settings-are-a-floor — a repository may narrow, never widen

**Question:** a config file inside a cloned repository beats the user's own global config, so
a repository can set `sandbox = "off"` and `approval = "allow-all"` and win. A typed flag still
beats both. Is that right?

**Decision: no. A user's own settings are a floor.** A project file may make a run stricter,
and never looser. The user chose the answer to this in plain terms: "your settings are a
floor."

## Why it is not folded into the wiring sprint

The trust fix in that sprint is about **loading a capability**: a skill, an MCP server, a
credential command, a provider endpoint. Those are dropped from an untrusted source.

Widening a security mode is a different rule. `sandbox` and `approval` are not capabilities a
file adds; they are limits a file relaxes. The gate for them is a comparison, not a filter, and
it needs its own probe:

- Rank the modes. `off < confined < strict` for the sandbox, and allow-all is weaker than
  read-only for approval. `narrow_sandbox` in `rho-core` already ranks the sandbox for a
  subagent, and the same order applies here.
- Compare the resolved value against the strongest value any **trusted** layer set. Keep the
  stronger one.
- Say so when a project file asked for less, because a silent refusal to obey a file is its own
  confusion.

## Rules out

**Dropping the keys from an untrusted project file.** A repository asking for a *stricter*
sandbox is useful, and refusing it would teach nothing.

**Requiring a flag every time.** A user who wrote `sandbox = "strict"` once should not have to
type it in every checkout.

**Comparing against the flag layer only.** A user's global file is their own choice as much as
a flag is.

## What must be probed before it lands

A live run: a trusted global config with `sandbox = "strict"`, a project file with
`sandbox = "off"`, and evidence of which one the shell tool obeyed. The same for approval, with
a write the read-only policy must refuse. Neither is proved today, and the sprint that lands
this rule writes that record.
