# Verification: the interactive session records itself

This records a real drive of `SPEC-the-interactive-session-records-itself`. It is not a test
report. Every command below was run, and the output is quoted as it appeared.

## Why a drive, and not a test

The five recording calls live in the `tokio::select!` arms of the interactive event loop, in
`crates/rho-tui/src/app.rs`. The loop owns the terminal, so no `cargo test` can reach an arm.
The spec says so, and names this document as the guard.

The risk is real and measured. The model picker drew `the catalog load ended unexpectedly`
over a good list of 447 models, on every `/model` press, while 2242 tests passed. No test saw
it, because the defect lived in a `select!` arm. A live drive found it in one run.

## The harness

`bench/tui_session_recording_drive.py` starts the real binary under a pseudo-terminal. It
feeds the master file descriptor into a `pyte` screen, so every assertion reads rendered
cells. It sets `HOME` to a temporary directory, so the drive never touches the real `~/.rho`.

`pyte` is not in the system python on this machine. A virtual environment holds it:

```sh
python3 -m venv /tmp/rho-pty-venv
/tmp/rho-pty-venv/bin/pip install pyte
```

## The run

```sh
cargo build -p rho-cli
/tmp/rho-pty-venv/bin/python bench/tui_session_recording_drive.py
```

Real output:

```
--- FINDINGS
   PASS · 1a. transcript shows a `session <id> at <path>` notice
   PASS · 1b. the path from the notice exists on disk
   PASS · 4a. the picker shows model rows from the catalog
   PASS · 4b. no `catalog load ended unexpectedly` line drew
   PASS · 2a. the file holds a session header record
   PASS · 2b. a model_change record carries a provider and a model
   PASS · 3. the file ends with a `closed` record
   PASS · 5a. the first rho started and holds the named file
   PASS · 5b. a second rho on the same file refuses with the busy error
--- OK · 9 checks passed
```

Exit code 0.

## What each check proves

| Check | What it proves |
| --- | --- |
| 1a | The session id reaches the user. The alternate screen hides stderr, so the notice goes to the transcript. |
| 1b | The path in the notice is a real file. A notice naming nothing would be worse than silence. |
| 2a | The recorder wrote the header. |
| 2b | A `model_change` record carries a provider and a model, as one pair. |
| 3 | A clean exit closes the file, so a crash offer can tell a clean exit from a crash. |
| 4a | The picker still lists models. |
| 4b | The catalog fix holds. This is the regression guard for the defect below. |
| 5a | The first session holds its named file. |
| 5b | A second rho on the same file refuses. The lock is held for the whole run. |

## The defect this drive exists to catch

The `maybe_catalog` arm of the event loop cleared its receiver only on the error path. The
loading task sends one event and drops the sender, so the next poll returned `None`. That
`None` reached the panic path, and the picker drew an error over a good list.

Both the `Models` branch and the `Error` branch now clear the receiver.

## Proof that the drive catches it

A drive that passes against broken code is worth nothing, exactly like a test that does. So
the fix was removed on purpose, and the drive was run again.

The break, in `crates/rho-tui/src/app.rs`, deleted one line from the `Models` branch:

```rust
*catalog_events = None;
```

Real output with the line removed:

```
   PASS · 4a. the picker shows model rows from the catalog
   ...
--- FAILURES
   FAIL · 4b. no `catalog load ended unexpectedly` line drew
--- FAILED · 1 of 9 checks
```

Exit code 1. One check failed, and no other check failed with it. So the check is specific,
and it is not theatre.

The good file was then copied back from `/tmp/app.rs.predrive`. **No `git checkout` was used.**
That command throws away every uncommitted change in a file, and this tree held a large
uncommitted change. The drive was run again and reported `OK · 9 checks passed`, exit code 0.

## What this drive does not cover

- A fatal signal, and a panic that unwinds. Neither guarantees a close. The spec states this,
  and an open file still resumes, so only the crash hint is lost.
- Redaction. A plain `Text` block inside a tool result is written unredacted. The spec records
  this as a known defect with its reason. A `bash` result that prints a secret reaches the
  session file. This drive does not test it, and does not claim to.
