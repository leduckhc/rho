# Subagents

rho 0.1.0. This page shows you how to define a child agent and watch it run.

A subagent is a child session rho starts on your behalf.
It gets its own prompt, its own tool set, and a turn cap.
When it finishes, it hands a summary back to the parent.

## Define an agent

An agent definition is a markdown file with YAML front matter.
Place it in one of these directories:

| Scope | Directories checked |
|---|---|
| User (always loaded) | `~/.rho/agents/`, `~/.agents/agents/` |
| Project (needs `--trust-project`) | `.rho/agents/`, `.agents/agents/` inside the project root |

Create a file at `~/.rho/agents/summariser.md`:

```markdown
---
name: summariser
description: Reads a file and returns a one-paragraph summary.
tools: read
max_turns: 6
---

You are a file summariser.
Read the file the caller names.
Reply with one paragraph only.
```

The front matter fields are:

| Field | Required | What it does |
|---|---|---|
| `name` | No | How the model names the agent. Lowercase letters, digits, hyphens, 1–64 characters. Leave it out and rho uses the file name, with a warning. |
| `description` | Yes | What the model reads to choose this agent. A definition with no description does not load. |
| `tools` | No | A space- or comma-separated list of tool names to allow. Write `all` or `*` to inherit every parent tool. Write `none` to give the child no tools. Omit the field to inherit. |
| `model` | No | Override the model. Omit to inherit the parent's model. |
| `max_turns` | No | Cap the child's turns. Omit to use the parent's remaining budget. |
| `sandbox` | No | `off`, `confined`, or `strict`. Can only narrow; the parent's mode is the floor. |

A name that holds uppercase letters, spaces, or non-ASCII characters still loads.
rho prints a warning and uses it.
A name longer than 64 characters loads with a warning too.

## Use it

Start a session in your project:

```sh
rho --trust-project
```

rho prints the definitions it found:

```
1 agent definition(s) available to spawn_agent: summariser.
```

Now ask the model to use it:

```
Summarise README.md using the summariser agent.
```

The model calls `spawn_agent` and rho starts the child.
While the child runs, `agent_status` shows its state.
When the child finishes, the parent receives a report.

## Handles and aliases

Every child gets a handle derived from its definition name.
The first child spawned from `summariser` gets the handle `summariser`.
A second one gets `summariser-2`, then `summariser-3`, and so on.

An alias is a name you or the model chooses when spawning.
Use it to address a running child with `steer_agent` or `cancel_agent`.
An alias must be 64 characters or fewer.
It must not contain control characters.
It must not be only digits.
A refused alias never stops the spawn; the child still runs.

Address a running child by its numeric id or its handle:

```
Cancel the summariser-2 child.
```

## What the parent gets back

When a child finishes, the parent receives:

| Field | What it contains |
|---|---|
| `outcome` | `done`, `out_of_turns`, `canceled`, `failed`, or `rejected` |
| `summary` | The child's final answer, capped at 8 000 characters |
| `usage` | Token counts for every turn, summed |
| `turns` | How many turns the child used |
| `transcript` | Path to the full transcript file on disk |

Only `done` is a success.
Every other outcome is a failure the parent can act on.

## Transcripts

rho writes the child's transcript to the system temp directory.
On Linux and macOS the path is `/tmp/rho-transcripts-<uid>/<pid>/tasks/`.
The file is there while the child runs.
Open it in another terminal to read it live.
rho does not write transcripts into your repository.

## What a child inherits

A child can only narrow what its parent holds.
It cannot widen it.

| Resource | Rule |
|---|---|
| Tools | Intersection of the definition's list and the parent's set. A name the parent does not hold is dropped and reported. |
| Session root | Same as the parent. |
| Sandbox | The stricter of the definition's request and the parent's mode. A weaker request is refused at spawn time. |
| Approval policy | Both the parent's policy and the child's policy must allow a call. |

A child never receives `spawn_agent`.
rho captures the parent's tool set before `spawn_agent` joins it.
A child cannot spawn a child.

## Limits

Start rho with flags to change the defaults:

| Flag | Default | What it limits |
|---|---|---|
| `--max-children-per-parent` | 4 | Children one parent runs at once. Over this cap rho queues the child. |
| `--max-live-agents` | 32 | Agents live in the whole process. |
| `--child-timeout-secs` | 600 | Seconds before rho cancels a child. |
| `--max-queued-per-parent` | 16 | Children one parent may queue for a slot. |
| `--max-queued-total` | 128 | Children waiting across the whole process. |
| `--agent-grace-turns` | 5 | Turns of warning before a child's turn cap. |
| `--max-agent-tool-calls` | 64 | Tool calls one child may make in total. |

When rho refuses a spawn, it names the flag to raise.

Grace turns exist because a child cannot ask for more turns.
When a child is this many turns from its cap, rho tells it to write its summary early.

## Watch, redirect, or stop a running child

rho gives the model three more tools beside the two that spawn. They appear under the same
condition: at least one agent definition loaded.

| Tool | What it does |
|---|---|
| `agent_status` | Poll one child by id or handle. Call it with no id to list every child this session runs. |
| `steer_agent` | Send a message to a child that is still running. The child reads it as its next instruction, after its current tool calls finish. |
| `cancel_agent` | Stop one child. Its siblings keep running, and your session keeps running. |

Steering a child is the way to redirect work without throwing it away. Say what you want
changed, and the child picks it up at its next turn boundary.

`--read-only` denies `steer_agent` and `cancel_agent`, because both change what a child does.
It allows `agent_status`, which only reads.

> **Not built yet.** This is steering for a **child**. You cannot steer your own turn. A
> message you type in the terminal interface while a turn runs does not reach that turn.
> See [status](status.md).

## Timeouts, cancellation, and retries

A child that times out gets outcome `canceled`.
A child you cancel also gets outcome `canceled`.
A child that fails may be retried.
rho retries the same task at most three times.
After three failures rho reports the task failed and stops retrying.

## Spawning several at once

Use `spawn_agents` to start more than one child in a single call.
`spawn_agent` handles the common single-child case.

## Require the files a child must deliver

`spawn_agent` takes an `artifacts` list. Name the files the child must leave behind, and rho
checks each one after the child stops. A missing or empty file turns the outcome into a
rejection that names the file. So a child cannot claim success it did not earn.

```sh
rho run 'Have the summariser write summary.md, and require artifacts=["summary.md"]'
```

> **Partly built.** Only `spawn_agent` carries `artifacts`. The fan-out tool `spawn_agents`
> sends an empty list, so a child started that way is never checked.

## Name collisions between definitions

Two definitions with the same name are both loaded.
The project definition overwrites the user definition when names clash.
Only one definition per name reaches the model.
Rename the file to avoid a silent override.

## What does not work yet

> **Partly built.** rho reads the `[subagents]` table in a config file and no code applies it.
> Set limits with flags instead, for example `--max-children-per-parent 8`.

> **Not built yet.** There is no flag to allow a child to spawn a child.
> `max_depth` is fixed at 1 in this version.
> A definition that sets it has no effect.

## See also

- [Tools](tools.md) — the full list of tools a parent or child may hold.
- [Permissions](permissions.md) — how `--trust-project` and approval policies work.
- [CLI reference](cli.md) — every flag in one place.
