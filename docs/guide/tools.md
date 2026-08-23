# Tools

rho 0.1.0. This page tells you what the model can do on your machine, every limit,
and why a large result came back cut.

## Tool reference

| Name | What it does | When available | Notable limit |
|---|---|---|---|
| `read` | Read a text file by line range | Always | 100 000 bytes; cut and noted |
| `list` | List one directory | Always | None |
| `glob` | Match files by a glob pattern | Always | Skips `.gitignore` matches |
| `grep` | Search file contents by a regex | Always | Skips files over 5 MB |
| `write` | Write a whole file | Always | Creates parent directories; overwrites |
| `edit` | Replace one exact span in a file | Always | Refuses an ambiguous match |
| `bash` | Run a shell command | Always | 100 000 bytes output; 120 s timeout |
| `task` | Inspect background tasks | When a task registry exists | Read-only; cannot cancel |
| `task_cancel` | Cancel a background task | Always in the `rho` binary | Denied by `--read-only` |
| `read_tool_result` | Read a stored large result | When a result store exists | 64 KiB per read; 20 matches per search |
| `spawn_agent` | Start one subagent | When agent definitions exist | See [subagents](subagents.md) |
| `spawn_agents` | Start several subagents at once | When agent definitions exist | See [subagents](subagents.md) |
| `agent_status` | Poll one subagent, or list the running ones | When agent definitions exist | Read-only; call it with no id to list |
| `steer_agent` | Send a new instruction to a running subagent | When agent definitions exist | Denied by `--read-only` |
| `cancel_agent` | Stop one running subagent | When agent definitions exist | Denied by `--read-only`; siblings keep running |
| `mcp__<server>__<tool>` | Call an MCP server tool | When MCP servers are configured | Varies by server |

## Path confinement

Every file and directory tool routes its path through `confine`. The function
resolves symlinks component by component. It compares canonical forms. A path
that escapes the session root returns an error. The tool never reads or writes
the file.

That means `read` with `path: "/etc/passwd"` fails. So does `path:
"../../secrets"`. The error names the path and says it escapes the root.

`bash` is different. It runs a full shell and has no path confinement. A `cd`
or an absolute path in the command reaches any file you can reach. The approval
policy is the boundary for `bash`. Pass `--read-only` to deny it outright.

## `--read-only`

`--read-only` allows `read`, `list`, `glob`, `grep`, `task`, and
`read_tool_result`. It denies `write`, `edit`, `bash`, `task_cancel`,
`spawn_agent`, and `spawn_agents`. The model can inspect the tree and read
results but cannot change anything.

## Sandbox modes

Pass `--sandbox confined` or `--sandbox strict` to confine `bash` at the OS
level.

`confined` restricts file writes to the session root and a scratch directory.
The command can still reach the network.

`strict` does the same and also blocks network access.

On macOS, rho uses `sandbox-exec`. On Linux it uses `bwrap`. When you request
confinement and neither tool is on `PATH`, rho refuses the command. It never
runs unconfined. The error message names the mode and says to install `bwrap`
or pass `--sandbox off`.

See [permissions](permissions.md) for the full trust and sandbox rules.

## `bash`

Parameters: `command` (required), `timeout_ms` (optional, default 120 000,
maximum 600 000), `run_in_background` (optional boolean).

`bash` runs `sh -c <command>` in the session root. It merges stdout and stderr.
Output over 100 000 bytes is cut and the result says so.

A single line over 64 KiB is split into pieces. No piece is dropped.

The default timeout is 120 seconds. The ceiling is 600 seconds. If `timeout_ms`
is 5 000 or less, the result notes that the unit is milliseconds, not seconds.

**Background tasks.** A command goes to the background automatically when
`timeout_ms` is above 30 000, when the command matches a known long-running shape
(for example `cargo build`, `npm test`, `sleep`, `--watch`), or when
`run_in_background` is `true`. A background call returns at once with a task id.
Use the `task` tool to probe it. If a foreground command exceeds its timeout and a
task registry exists, rho adopts it into the background instead of killing it.

> **Partly built.** The model can probe a task with the `task` tool, and you cannot watch one.
> rho computes a progress summary for a background task and stores it on the task row, and the
> renderer never draws that field. So a long build shows no percentage and no step count. Ask
> the model to call `task`, or read the command's own output when it finishes.

**Environment scrubbing.** Before the command starts, rho removes every
environment variable whose name looks like a secret. The variable name and value
both disappear. A command cannot see `OPENROUTER_API_KEY`, `AWS_SECRET_ACCESS_KEY`,
or any name the redactor recognises. This is defence in depth. A command can still
read a credential file from disk, for example `~/.aws/credentials`. The approval
policy is the real boundary.

**Sandbox.** Under `--sandbox confined` or `--sandbox strict`, rho wraps the
command in `sandbox-exec` (macOS) or `bwrap` (Linux). A missing backend refuses
the command.

**Kill.** On timeout or cancel, rho kills the whole process group. No grandchild
survives.

## `edit`

Parameters: `path` (required), `old_text` (required), `new_text` (required),
`replace_all` (optional boolean, default false).

`edit` replaces an exact text span in a file. `old_text` must match verbatim,
including indentation.

By default `old_text` must appear exactly once. If it appears more than once,
the call fails and the error says how many matches it found. Pass `replace_all:
true` to change every match on purpose.

If `old_text` is absent, the error says so and names the near miss it found. Two
near misses it checks: the span with whitespace trimmed, and the same lines with
different indentation.

`old_text` cannot be empty. An empty pattern matches at every character boundary.
The call fails with an explanation.

`old_text` and `new_text` must differ. An identical replacement always fails.

## `read_tool_result`

Parameters: `handle` (required), `start_byte` (optional), `byte_count`
(optional), `query` (optional).

When a result is 16 KiB or larger and a result store exists, rho stores the whole
result and gives the model a 4 KiB preview and a handle. Use
`read_tool_result` to read the rest.

Pass `query` to search for a literal string. The tool returns at most 20 matches.
Each matching line is cut at 512 bytes.

Pass `start_byte` and `byte_count` to read a byte range. The default read is
8 KiB. The maximum per call is 64 KiB.

All slices are cut on character boundaries. A read never returns half a
multi-byte character.

The store lives in a private temporary directory. It is removed when the session
ends. A handle from one run is not valid in another run.

**Without a store.** If rho has no result store, a result over 64 KiB is cut at
64 KiB and the tail is gone. The result says how many bytes were lost. There is no
handle and no way to read the tail.

## Result store: when it exists

The result store opens when the session starts. rho 0.1.0 opens one by default.
If the session has no store, `read_tool_result` is not registered and the model
does not see it.

## What does not work yet

> **Not built yet.** There is no `todo` tool. The model cannot add or read a
> task list. No code registers a `todo` tool in this session.

> **Not built yet.** There is no `ask_user` tool. The model cannot ask you a
> question mid-turn. If it needs a choice, it must make one or state what it
> assumed.
