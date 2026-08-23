# Permissions

This page covers what rho may do on your machine and how to narrow it.
Read it before you run rho on a machine you care about.

rho 0.1.0 is permissive by default.
With no flags it approves every tool call without asking.
It will run `bash` commands, edit files, and make network requests.
It does none of this silently—output streams to the terminal—but it does all of it
without a confirmation prompt.

## Narrow what rho may do

### Approval policy

Pass `--read-only` to deny every tool that can change state.

```sh
rho --read-only "summarise the README"
```

This maps to `approval = "read-only"` in a config file.
You can also write `approval = "allow-all"` to state the default.

> **Not built yet.** `approval = "ask"` is accepted in a config file but always refused
> at startup with the message:
> `approval = "ask" needs an interactive frontend, which this build does not have here. Use read-only or allow-all, or pass --read-only.`
> There is no interactive approval gate anywhere in rho today.
> Use `--read-only` as the safe alternative.

### Sandbox

Pass `--sandbox` to confine what `bash` may reach.

```sh
rho --sandbox strict "run the tests"
```

| Value | What it restricts |
|---|---|
| `off` | Nothing. `bash` runs with your full environment. This is the default. |
| `confined` | Writes are limited to the session root and a scratch directory. |
| `strict` | Same as `confined`, plus network access is denied. |

When you ask for confinement and the host OS offers no sandbox support,
`bash` refuses the command rather than running it unconfined.

### A copy-ready safe invocation

```sh
rho --read-only --sandbox strict "explain this codebase"
```

This denies every mutating tool and denies the network.
The model can read and search files.
It cannot write, delete, move, or run a command.

One gap sits inside that promise. `--sandbox strict` denies the network to `bash`, and to
nothing else. An MCP tool that declares itself a fetch tool still runs under `--read-only`,
because rho counts a fetch as a read. So the network claim holds for a default build, and it
stops holding when you add an MCP server that reaches the network.

### Session root

Every file path a tool touches must be inside the session root.
The default root is your current directory.
Pass `--root /path/to/project` to set it explicitly.
A path that escapes the root is rejected with an error.
Symlinks are resolved before the check, so a link that points outside the root is caught.

### Project config

rho reads a project config file if one is present.
By default it loads the `approval` and `sandbox` settings.
It drops `skill-paths`, `mcp-config`, and any `!command` credential.
Pass `--trust-project` to allow those fields.

### Project skills

A skill file in the repository under edit is not loaded by default.
A skill can instruct the model and can carry scripts.
Pass `--trust-project` to load project skills.

### `AGENTS.md`

An `AGENTS.md` file in the project is loaded as model instructions.
It cannot widen the approval policy, the sandbox, or the tool set.
It has no mechanism to grant any permission.
Instructions in the file influence what the model tries, not what rho allows.

### MCP servers

A stdio MCP server inherits a scrubbed copy of your environment.
Variables whose names match credential patterns—`SECRET`, `PASSWORD`, `API_KEY`,
`ACCESS_KEY`, `_TOKEN`, `_KEY`, and others—are removed before the child starts.
The server can add its own variables via the `env` key in its config entry.
Those are added after the scrub, so they are not removed.

### Subagents

A subagent cannot escalate beyond its parent.
The approval policy is the conjunction of the parent's policy and the child's policy.
A child that asks for `allow-all` under a `read-only` parent stays `read-only`.
A child may ask for a stricter sandbox than its parent, never a weaker one.
A child that requests a weaker sandbox is refused, not silently demoted.

## How read-only is enforced

Each tool declares a kind: `Read`, `Search`, `Think`, `Fetch`, `Execute`, `Edit`, and so on.
The read-only policy uses an allowlist of the kinds that cannot change state.
A tool that declares no kind defaults to `Other`.
`Other` is not on the allowlist, so it is denied under read-only.
A new tool is therefore denied until its author declares a safe kind on purpose.

## What rho cleans before you see it

A tool result and an MCP result reach your terminal through a filter. rho strips every
ANSI escape sequence, so tool output cannot repaint your screen or set your window title.
It strips the bidirectional overrides used in a Trojan Source attack. It strips zero-width
and invisible characters, so two names that look identical cannot hide a difference.

This matters because a tool reads files you did not write. A hostile file cannot use rho's
output to forge a line of your terminal.

## What rho does not protect you from

> **Not built yet.** There is no per-call approval prompt.
> `approval = "ask"` is refused at startup (see above).
> rho does not ask before it runs `bash` for the first time.
> `--read-only` is the only way to prevent execution today.

**Credential scrubbing is a denylist, not an allowlist.**
Variables are removed when their name matches known patterns.
A credential stored under an unusual name survives the scrub.
A shell command can still read any variable that does not match the list.

**The `Secret` type masks values in `Debug` output.**
A struct that holds a `Secret` prints `Secret(***)` in logs.
A raw string credential that never enters a `Secret` wrapper is not masked.
There is no blanket redaction at the log transport level.

**`--sandbox off` (the default) is genuinely unconfined.**
With no sandbox flag, `bash` inherits your full environment (after the credential scrub)
and writes anywhere your user account can write.

**`AGENTS.md` and project skills influence model behaviour.**
They cannot widen what rho permits, but they can ask the model to attempt things
that rho then allows or denies based on the active policy.
A hostile `AGENTS.md` can waste turns and issue unwanted tool calls.
Use `--read-only` when you do not trust the repository.
