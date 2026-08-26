# Sessions

This page describes what rho keeps after a run and what it throws away.

**`rho run` writes a session file by default.** You can list your sessions, read one, continue
one, name one, fork one, and delete one. The terminal does not record yet; only `rho run` does.

## Where a session lives

```
~/.rho/sessions/<project-key>/<session-id>.jsonl
```

The project key is the directory name plus eight hex characters. The hex comes from the git
common directory, so **every worktree of one repository shares one pool of sessions**. rho reads
the `.git` entry itself and spawns no git process.

The session id is `<YYYYMMDD-HHMMSS>-<4 hex characters>`, so the file names sort newest last.

The store is private. A session file is `0o600`, and every directory rho creates under the store
root is `0o700`. rho does not trust your umask for this, because the file holds a whole
conversation.

## The commands

```sh
rho sessions list                       # every session, newest first
rho sessions list --long                # also the directory and the fork origin
rho sessions show <id-prefix>           # one line per record, with its record id
rho sessions show <id-prefix> --full    # the whole text of each record
rho sessions name <id-prefix> "a title" # an explicit title
rho sessions fork <id-prefix> --at r5   # copy the branch ending at r5 into a new session
rho sessions delete <id-prefix>         # remove one session file
```

A prefix is enough while it is unique. An ambiguous prefix is refused, and the message lists
every match. rho never picks one for you.

`show` and `list` send nothing to a model. Looking at yesterday's session costs nothing.

A real listing:

```
ID                   LAST ACTIVE TITLE              MODEL           TOKENS  COST
20260826-204754-f8c9 just now    the sign bug       us.anthropic.c…   2.1k     -
```

The cost is a dash when the provider reports none. rho shows no number it did not read. There is
no turn count, because counting turns needs the whole file.

## Continue a conversation

```sh
rho run "and now fix it" --continue                 # the newest session of this project
rho run "and now fix it" --continue=20260826-2047   # that one
rho run "and now fix it" --resume=20260826-2047     # --resume is the same flag
```

**The value needs an equals sign.** `--resume 20260826-2047` puts the id where the prompt goes,
so rho refuses it and tells you to write `--resume=20260826-2047`.

A resume replays the earlier conversation to the model. So the model remembers what it read and
what it said, and it does not read your files again.

A session that ended cleanly is still resumable. rho writes a reopen record, so the file always
says what happened.

### After a crash

A run that dies leaves a session with no close record. Run `rho run "..." --continue` and rho
opens it. Nothing needs cleaning by hand: the lock dies with the process.

### Two rho processes

A live session holds an advisory lock. A second rho that tries the same session says:

```
rho: session 20260826-204754-f8c9 is open in another process. Use another session, or close that one.
```

A bare `--continue` moves past a live session and takes the next one. `list` and `show` always
work, because a read takes no lock.

## Fork a session

Two commands you can type:

```sh
rho sessions show 20260826-2047
rho sessions fork 20260826-2047 --at r4
```

`show` prints the record id in the first column, so you copy it into `--at`. The original file
stays byte-identical, and the new session names its origin.

A tool result in `show` reports the tool and a byte count, never the body. So a secret inside a
result does not reach your terminal by accident.

## Write no file at all

```sh
rho run "a throwaway question" --ephemeral
```

The `ephemeral` config key does the same. `--ephemeral` with `--continue` is refused, because
there would be nothing to continue.

The `session-file` config key names one exact file, and it overrides the store.

## Keeping a plain text record too

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
The path sits under the system temporary directory:

```
<temp>/rho-transcripts-<uid>/<pid>/tasks/<task-id>.jsonl
```

On Linux `<temp>` is usually `/tmp`. On macOS it is your private `$TMPDIR`, which looks like
`/var/folders/lx/.../T`. Run `echo $TMPDIR` to see yours.

A real run produced this file, with these permissions:

```
/var/folders/lx/.../T/rho-transcripts-501/87155/tasks/agent-1.jsonl
-rw-------  531 bytes, 4 lines
```

The directory and file are private to the owning user (`0o700` and `0o600`).
The file streams one event per line as the subagent runs.
You can read it with `tail -f` while rho is running.
Each line carries a timestamp in epoch milliseconds and the agent name.
Event types are `TurnStart`, `Text`, `ToolStart`, `ToolUpdate`, `ToolEnd`, `Usage`, `Delivered`, and `End`.

The transcript directory is not removed by rho when the session ends. It is under the system temp folder, so the OS may clear it on reboot.

## A resume cannot widen a permission

A session file records the approval mode and the sandbox mode it ran under. A resume may
**tighten** either one, and it may never widen one:

```
rho: a resume would widen approval from read-only to allow-all; pass --allow-widen to allow it
```

Pass `--allow-widen` when you mean it. The flag is an error on its own.

A session file is untrusted input. So the stored mode can only take a permission away, never
grant one, and a forged file cannot stand in for the flag. The working directory the run uses
comes from your config, never from the file. A fork origin is shown and never followed.

## What a session file does not protect

**rho does not encrypt a session file, and it removes no old session.** The store grows until
you delete something.

**A resume reads the whole file.** `rho sessions list` does not, so a big session slows no listing.
A resume holds about 640 bytes per record, which is roughly three times the file size. A session of
a thousand turns costs under a megabyte, and one of a hundred thousand turns costs about 61 MiB. The
numbers and their command are in [benchmarks](../benchmarks.md). No cap bounds the record count, so
a session you never end keeps growing.

**A `session-file` outside the store is your directory, not rho's.** rho makes any directory it
creates itself private, `0o700`, and it sets the file to `0o600`. It does **not** change the mode of
a directory you already have. So a key that points into a shared or a synced folder puts the file
there, private to you, in a directory whose mode you chose.

Redaction masks a **credential-shaped argument key**, such as `api_key`, inside a tool call. It
matches a key name, and nothing else. So a session file holds these verbatim:

- a secret you pasted into a prompt,
- a secret inside a tool **result**, such as the output of `cat .env`,
- a secret on a `bash` command line, because the key there is `command`,
- a token inside a URL, such as a git remote.

If you record a secret, delete that session.

```sh
rho sessions delete <id-prefix>
```

Delete removes the session file, every sidecar beside it, and its lock file. It does not
overwrite the bytes, so a recovery tool may still find them. It does not remove a session forked
from this one, because a fork is its own file.

The four hex characters in an id are not a secret. They stop two sessions in the same second from
colliding. The `0o700` on the store is what keeps other users out.

## What does not work yet

> **Not built yet.** The `/sessions` slash command appears in the [terminal](terminal.md)
> command list, and it does nothing. **A terminal session records no file either.** Use
> `rho run` when you want a session, and `rho sessions list` as the picker.

> **Not built yet.** `/name` does not exist. Use `rho sessions name`.

> **Not built yet.** There is no rewind and no replay. `rho sessions fork` is the way to go back
> to an earlier point.

See [status.md](status.md) for the full list of what is not yet wired.

## If you embed rho as a library

`SessionStore` is the seam. A caller injects the store root and the project key, so a frontend
gets the same behaviour the command line has. `SessionStore::rows` builds a picker, `row_from`
is the bounded row builder, `SessionStore::newest_resumable` answers "continue", and
`Session::replay` puts a rebuilt conversation back into a context.

`SessionStore` is a struct and not a trait. A different storage backend is a fork, and
`SPEC-session-store-wiring` section 10 says why.
See [architecture](../architecture.md) for where each piece sits.
