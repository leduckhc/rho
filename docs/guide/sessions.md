# Sessions

This page describes what rho keeps after a run and what it throws away.
rho 0.1.0 writes no session file. Every run starts fresh. Nothing carries over.

## What you can do today

Redirect stdout to keep a record of a run.

```sh
rho run "refactor the auth module" 2>&1 | tee run.log
```

`tee` writes to `run.log` and still prints to the terminal.
The file captures the agent's text output. It does not capture tool calls, because
`rho run` prints assistant text to stdout and reasoning to stderr, and nothing else. Add
`--log info` when you want the tool calls too.
It does not capture the raw provider stream or internal tool results.
It captures nothing from subagent runs; those go to separate files (see below).

Interactive `rho` runs in the alternate screen, so the transcript is gone when it exits.
`rho run` prints to stdout like any other command, so a redirect keeps everything.

See [terminal.md](terminal.md) for the alternate-screen behaviour.

## The result store

When a tool returns a large result, rho stores the full text in a private temporary directory.
The context keeps a short preview and a handle.

```
<tool_result_preview handle="r-0001" stored_bytes="142300" preview_bytes="4096">
... first lines of the output ...
</tool_result_preview>
The full result is stored outside the context. Use read_tool_result with this
exact handle to read a byte range, or to search it for a literal string.
```

The agent can call `read_tool_result` to read any byte range or search for a literal string.
The store lives in a `rho-results-*` directory under the system temp folder.
When the session ends, that directory is deleted. The handles stop working.

See [tools.md](tools.md) for `read_tool_result`.

## Subagent transcripts

rho writes one JSONL transcript file for every subagent it spawns.
The path on Unix is:

```
/tmp/rho-transcripts-<uid>/<pid>/tasks/<task-id>.jsonl
```

The directory and file are private to the owning user (`0o700` / `0o600`).
The file streams one event per line as the subagent runs.
You can read it with `tail -f` while rho is running.
Each line carries a timestamp in epoch milliseconds and the agent name.
Event types are `TurnStart`, `Text`, `ToolStart`, `ToolUpdate`, `ToolEnd`, `Usage`, `Delivered`, and `End`.

The transcript directory is not removed by rho when the session ends. It is under the system temp folder, so the OS may clear it on reboot.

## What does not work yet

> **Not built yet.** The `/sessions` slash command appears in the [terminal](terminal.md)
> command list, and it does nothing. rho answers `/sessions is not built yet. See
> F-slash-commands in docs/features.md.` No session file is written today.

> **Not built yet.** The `session-file` config key parses without error, and nothing reads it. Setting it has no effect today.

> **Not built yet.** The `ephemeral` config key parses without error, and nothing reads it. Setting it has no effect today.

Because no session file exists, there is no resume command and no way to continue a past conversation.
The library holds a resume rule: it refuses a resume that would widen the approval or sandbox mode. For example, a session saved under `ask` approval cannot resume under `allow-all`. The flag `--allow-widen` would bypass that check. None of this runs today, because the CLI does not write a session file in the first place.

See [status.md](status.md) for the full list of what is not yet wired.

## If you embed rho as a library

The library holds what the binary does not: a session record format, resume, a permission
check on resume, and conversation branching. None of it is wired into the `rho` command.
See [architecture](../architecture.md) for where each piece sits.
