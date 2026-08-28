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

> **Partly built.** rho reads the `[subagents]` table from a config file and no
> code applies it. The run is silent. Pass `--max-children-per-parent`,
> `--max-live-agents`, and `--child-timeout-secs` as flags instead. There is no
> `--max-depth` flag, and a subagent cannot spawn one of its own.
> See [cli](cli.md) for those flags.

The table accepts four keys:

| Key | Type |
|---|---|
| `max-depth` | integer |
| `max-children-per-parent` | integer |
| `max-live-total` | integer |
| `child-timeout-secs` | integer |

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

A refused credential does not stop the run. rho marks it, and the error would appear only
when something resolves it. Nothing resolves a credential today, so the run continues and the
command never runs. A live probe confirmed both halves.

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

> **Partly built.** rho parses `[credentials]` and nothing resolves an entry, so the whole
> table changes nothing today. Give a provider its key through the environment instead, as
> [providers](providers.md) shows. The four forms below describe what the library does when a
> caller resolves one, which no part of the `rho` command does yet.

| Form | Example | Effect |
|---|---|---|
| Literal string | `"sk-live-abc123"` | Used as-is |
| `env:VAR` | `"env:ANTHROPIC_API_KEY"` | Reads that variable at runtime |
| `${VAR}` interpolation | `"Bearer ${TOKEN}"` | Fills each span from the environment |
| `!command args` | `"!pass show rho/key"` | Runs the command; stdout is the value |

The `!command` form is blocked in an untrusted project file, and it never runs there.
A command helper would run with a minimal environment, inheriting only `PATH` and `HOME`.
A command that exits non-zero, writes bad UTF-8, or runs over 30 seconds would fail the
resolve.

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

# [credentials] is partly built. Nothing resolves an entry, so this block does nothing.
[credentials]
# Literal value (avoid in shared files).
my-key = "sk-live-abc123"

# Read from an environment variable.
anthropic-key = "env:ANTHROPIC_API_KEY"

# Fill a template from environment variables.
bearer-token = "Bearer ${MY_TOKEN}"

# Run a command; its stdout is the value. Blocked in untrusted project files.
vault-key = "!pass show rho/anthropic"

# [subagents] is partly built — these keys parse but have no effect today.
# Use --max-children-per-parent, --max-live-agents, --child-timeout-secs instead.
# [subagents]
# max-depth = 3
# max-children-per-parent = 5
# max-live-total = 10
# child-timeout-secs = 300

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
| `[credentials]` table | Parses, and nothing resolves an entry. No provider asks for one. Use an environment variable. |
| `[subagents]` table | Parses silently, no effect. Use CLI flags. |

