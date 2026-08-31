# Configuration

This page describes every config key, environment variable, profile, and credential
source for rho 0.1.0.

## Config files

rho reads two files.

**Global file:** `$XDG_CONFIG_HOME/rho/config.toml`.
When `XDG_CONFIG_HOME` is unset or empty, rho uses `$HOME/.config/rho/config.toml`.

**Project file:** `<root>/.rho/config.toml`.
The root is `--root`, then `RHO_SESSION_ROOT`, then the current directory.

A missing file is not an error.
An unreadable file, a malformed file, or an unknown key stops the run.

## Precedence

rho merges six sources.
The list runs weakest first, strongest last.

1. Built-in defaults
2. Global file
3. Project file
4. Named profile (selected with `--profile NAME`)
5. Environment variables (`RHO_*`)
6. CLI flags

A CLI flag beats every file value.
An environment variable beats every file value and every profile value.

Five keys are powerful: `base-url`, `skill-paths`, `mcp-config`, `session-root`, and
`session-file`. rho drops these five from the environment when the project is untrusted and one
of two signals is present. A notice then names each key it dropped. Pass `--trust-project` to use
them in a project you trust.

The first signal is a project config file that rho really read. The second signal is a file that
injects environment variables: `.envrc`, `.env`, or `.devcontainer/devcontainer.json`. A clone can
ship any of these, and each can set a powerful `RHO_*` variable. rho tests only whether the file
exists. rho never reads it and never runs it.

rho looks for those files from the project directory up to the git root. direnv also loads a
parent `.envrc`, so a clone injects the environment in every subdirectory. The git root is the top
of the clone, so the search stops there. rho never searches your home directory. With no git
repository, rho searches the project directory alone.

In a plain directory with none of those files, every environment variable works.

`RHO_SESSION_ROOT` is stricter. It needs `--trust-project` in every directory, because it selects
which project config file rho reads. rho makes that choice before it reads any file, so neither
signal above is available yet.

Three cases stay open. A directory shipped without a `.git`, with `.envrc` in a parent, does not
gate, because there is no clone boundary to find. Running rho at that directory's own root still
gates. A `.envrc` below your home directory gates, though `~/.envrc` itself does not. A shell rc
file, a `Makefile`, and a `docker-compose.yml` are not detected.

## Keys

All keys are kebab-case.
An unknown key is an error, not a warning.

| Key | Type | Default | Environment variable |
|---|---|---|---|
| `provider` | string | unset | `RHO_PROVIDER` |
| `model` | string | unset | `RHO_MODEL` |
| `session-root` | path | unset, so rho uses the current directory | `RHO_SESSION_ROOT` |
| `session-file` | path | unset | `RHO_SESSION_FILE` |
| `ephemeral` | bool | `false` | `RHO_EPHEMERAL` |
| `sandbox` | string | `off` | `RHO_SANDBOX` |
| `approval` | string | unset | `RHO_APPROVAL` |
| `skill-paths` | list of paths | empty | `RHO_SKILL_PATHS` |
| `no-skills` | bool | `false` | `RHO_NO_SKILLS` |
| `tui-mouse` | bool | `true` | `RHO_TUI_MOUSE` |
| `tui-reasoning` | string | `summary` | `RHO_TUI_REASONING` |
| `reasoning-effort` | string | unset | `RHO_REASONING_EFFORT` |
| `mcp-config` | path | unset | `RHO_MCP_CONFIG` |
| `base-url` | string | unset | `RHO_BASE_URL` |
| `tui-motion` | bool | `true` | `RHO_TUI_MOTION`, and `RHO_REDUCE_MOTION=1` |
| `no-agents` | bool | `false` | `RHO_NO_AGENTS` |

`RHO_SKILL_PATHS` uses the OS path separator (`:` on Unix, `;` on Windows).

`RHO_LOG` controls log verbosity.
It has no config-file key.

### `sandbox`

Valid values: `off`, `confined`, `strict`.
The default is `off`.
See [permissions](permissions.md) for what each value restricts.

### `approval`

Valid values: `read-only`, `ask`, `allow-all`.
When unset, the frontend resolves the mode.

`ask` is refused at runtime with:

```
approval = "ask" needs an interactive frontend, which this build does not have here. Use read-only or allow-all, or pass --read-only.
```

Use `--read-only` as the flag equivalent of `approval = "read-only"`.
See [permissions](permissions.md) for the effect of each mode.

### `tui-reasoning`

Valid values: `off`, `summary`, `full`, `live`.
The default is `summary`.

### `reasoning-effort`

Valid values: `off`, `low`, `medium`, `high`, `xhigh`.
When unset, rho sends no effort field and the provider uses its own default.

### Boolean environment variables

A boolean variable accepts `1`, `true`, `yes`, `0`, `false`, or `no`.
The match ignores case and trims surrounding space.
Any other value stops the run.

### `[subagents]`

The table accepts four keys. All four reach the agent.

| Key | Type | Effect |
|---|---|---|
| `max-depth` | integer | How deep the tree may go. `0` forbids spawning. |
| `max-children-per-parent` | integer | Children one parent runs at once. |
| `max-live-total` | integer | Agents live in the whole process. |
| `child-timeout-secs` | integer | How long a child may run. |

A flag beats the table. See [cli](cli.md) for the flags.

**`max-depth` cannot go above 1 from the command line.** rho gives a child no spawn tool, so
a grandchild cannot exist, and a higher value is lowered to 1. A lower value is honoured,
because it is stricter.

The six other subagent limits have a flag and no config key yet:
`--max-agent-tool-calls`, `--agent-grace-turns`, `--max-queued-per-parent`,
`--max-queued-total`, `--queue-wait-secs`, and `--max-agent-steer-bytes`.
`--queue-wait-secs` follows `child-timeout-secs` when you do not pass it.

#### A project file may only lower a limit

A limit is a bound. So a project file may make a run stricter and never looser.

rho takes the built-in defaults, then your own global file. That is the ceiling. A project
file, and any profile a project file defines, is then lowered to that ceiling. A project value
above the ceiling is refused, and rho names the limit on stderr:

```
rho: a project file may only lower a subagent limit, so rho kept your own value for
subagents.max-live-total (from /repo/.rho/config.toml).
```

`--trust-project` does **not** lift this. That flag loads a capability, and a limit is not a
capability a file adds. To raise a cap in one repository, pass the flag for that run, or set
the value in your own global file.

### `session-file`

One exact file for the session of this run. It overrides the store under
`~/.rho/sessions/<project-key>/`, so every run with this key set appends to the same file.

`rho run` reads it. The terminal records nothing yet. See
[sessions.md](sessions.md).

### `ephemeral`

`true` writes no session file at all. The default is `false`, so `rho run` records.

`--ephemeral` on the command line does the same thing, and it wins. `--ephemeral` with
`--continue` is refused, because there would be nothing to continue.

### `base-url`

The provider endpoint. Use it for a local model host.

```toml
base-url = "http://localhost:11434/v1"
```

rho appends the standard OpenAI chat path, so a base with or without `/v1` both work. An
`https` url is allowed anywhere. Plain `http` is allowed only to `localhost`, `127.0.0.0/8`,
or `[::1]`, because the credential would otherwise travel in clear text. A loopback endpoint
also bypasses every proxy, so `HTTP_PROXY` cannot capture your key.

A startup notice names the host your key goes to. `base-url` with `bedrock` or `azure` stops
the run, because each names its endpoint its own way.

### `tui-motion`

False stops the animation that sweeps the working word. `--no-motion` does the same, and so
does `RHO_REDUCE_MOTION=1`, which wins over `tui-motion` whichever order they arrive in.

### `no-agents`

True stops the agent-definition search, so rho offers no subagent. It is separate from
`no-skills`, which stops the skill search only.

## Project file trust

A project file is untrusted by default.
An untrusted file loses `session-root`, `session-file`, `skill-paths`, `mcp-config`, and
`base-url`, and it marks any `!command` credential as refused. The same applies inside a
`[profiles.x]` block, and to `RHO_SKILL_PATHS`, `RHO_MCP_CONFIG`, `RHO_BASE_URL`,
`RHO_SESSION_ROOT`, and `RHO_SESSION_FILE` from the environment.

`session-root` is on that list because it is the boundary every tool is confined to. A probe
proved a cloned repository could move it and read a file outside itself.

rho names what it dropped:

```
rho: this project is not trusted, so rho ignored session-root (from ./.rho/config.toml),
base-url (from the environment). Pass --trust-project to use them.
```

A display key such as `model` needs no trust, because it grants nothing.

Pass `--trust-project` to restore those keys.

A refused credential stops the run when the provider asks for it, which is before the first
model turn. The command never runs. A live probe confirmed both halves.

## Profiles

A profile is a named block inside a config file.
It can hold every key, including `[credentials]`.

```toml
[profiles.work]
provider = "openrouter"
model = "anthropic/claude-sonnet-4.5"
reasoning-effort = "high"
```

Select a profile with `--profile work`.
An unknown profile name stops the run.

A profile merges at layer 4.
It beats any plain file value, but environment variables and CLI flags beat a profile.

## Credentials

A credential value takes four forms.

rho names each credential after its provider, so the entry for OpenRouter is `openrouter`
and the entry for Azure is `azure`.

| Form | Example | Effect |
|---|---|---|
| Literal string | `"sk-live-abc123"` | Used as-is |
| `env:VAR` | `"env:ANTHROPIC_API_KEY"` | Reads that variable at runtime |
| `${VAR}` interpolation | `"Bearer ${TOKEN}"` | Fills each span from the environment |
| `!command args` | `"!pass show rho/key"` | Runs the command; stdout is the value |

**With no entry, rho reads the provider's own variable.** OpenRouter reads
`OPENROUTER_API_KEY` and Azure reads `AZURE_OPENAI_API_KEY`, so nothing changes if you already
set one. An absent key stops the run with a sentence naming what to set:

```
rho: cannot resolve the credential "openrouter": no [credentials] entry names it, and the
environment variable "OPENROUTER_API_KEY" is not set. Set that variable, or add a
[credentials] entry named "openrouter".
```

An empty key stops the run the same way. An empty key reaches the provider and returns 401,
which reads as a broken account rather than a missing key.

**Bedrock takes no entry.** The AWS SDK reads its own chain: environment variables, a profile,
the SSO cache, and IMDS. rho reads only `AWS_REGION` for it, and a region is not a secret.

**A project file's whole `credentials` table needs `--trust-project`.** Every form is refused,
not only `!command`. A project file arrives with a clone, and a clone chooses `provider` too,
so it chooses which credential name resolves. Three attacks follow from that: `env:VAR` reads
a variable you never meant to send, `${VAR}` does the same, and a literal sends your whole
conversation to an account somebody else reads. A refused entry fails when it resolves, and
the message names the flag.

Your own global file is never gated, in any form. A home directory is not a clone.

A command helper runs with a minimal environment, inheriting only `PATH` and `HOME`. Its
stderr is dropped, so a chatty helper cannot print a key onto rho's stderr. A command that
exits non-zero, writes bad UTF-8, or runs over 30 seconds fails the resolve.

rho expands no `~` in any path. Write an absolute path, or rho creates a directory named `~`.

## Example `config.toml`

```toml
# rho 0.1.0 — paste this file and edit what you need.
# Path: $HOME/.config/rho/config.toml

# Which provider to use: openrouter, bedrock, or azure.
provider = "openrouter"

# Model id passed to the provider.
model = "anthropic/claude-haiku-4.5"

# The boundary for every tool. rho refuses a path outside it. No ~ expansion.
session-root = "/home/you/code/my-project"

# One exact session file. It overrides the store under ~/.rho/sessions.
# session-file = "/home/you/notes/my-session.jsonl"

# Write no session file at all. The default is false, so `rho run` records.
# ephemeral = false

# Sandbox mode: off, confined, or strict.
sandbox = "off"

# Approval mode: read-only, allow-all. (ask is refused in this build.)
approval = "allow-all"

# Extra directories rho searches for skills. Absolute paths only.
skill-paths = ["/home/you/.rho/skills", "/opt/shared-skills"]

# Set true to skip all skill discovery.
no-skills = false

# Let the terminal interface capture the mouse (on by default).
tui-mouse = true

# How the terminal interface draws reasoning: off, summary, full, live.
tui-reasoning = "summary"

# How hard the model thinks: off, low, medium, high, xhigh. Unset = provider default.
reasoning-effort = "medium"

# Path to an MCP server file. No ~ expansion, so write it out in full.
mcp-config = "/home/you/.rho/mcp.json"

# Each entry is named after its provider: openrouter, azure. Bedrock takes none.
[credentials]
# Literal value (avoid in a file you share).
openrouter = "sk-live-abc123"

# Read from an environment variable.
azure = "env:MY_AZURE_KEY"

# Or fill a template from environment variables.
# azure = "Bearer ${MY_TOKEN}"

# Or run a command; its stdout is the value.
# openrouter = "!pass show rho/openrouter"

# Subagent limits. A project file may lower one of these and never raise one.
# max-depth above 1 is lowered to 1, because a child holds no spawn tool.
[subagents]
max-depth = 1
max-children-per-parent = 5
max-live-total = 10
child-timeout-secs = 300

[profiles.fast]
# Override the model and the effort for quick runs.
model = "anthropic/claude-haiku-4.5"
reasoning-effort = "low"

[profiles.strict]
# Deny every write, and confine the shell.
sandbox = "strict"
approval = "read-only"
```

## What does not work yet

| Key | Symptom |
|---|---|
| `session-root` in a global file | It confines tools, and it does not move which project file rho reads. |
| six subagent limits | `max-tool-calls`, `grace-turns`, `max-queued-per-parent`, `max-queued-total`, `queue-wait-secs`, and `max-steer-message-bytes` have a flag and no config key. |
| a `[subagents]` limit from `RHO_*` | No environment variable sets a subagent limit. Use a file or a flag. |

