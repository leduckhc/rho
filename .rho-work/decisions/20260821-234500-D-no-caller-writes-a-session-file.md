# No production caller writes a session file, and the persisted format has no reader

Date: 20260821. Reference: `D-no-caller-writes-a-session-file`.
Found while building the reasoning replay. It is outside that diff.

## The finding

`SessionRecorder` is complete, tested, and **never constructed outside a test**.

```sh
grep -rn "SessionRecorder\|SessionLog::" crates/rho-cli crates/rho-tui crates/rho-acp
# no matches
```

`rho run` and the TUI never record a prompt, a turn, or a stop. There is no `--resume` flag
either. So:

- No session file exists in production, whatever `session-root` and `session-file` say.
- The whole persisted format is a contract with a future rho, and nothing exercises it end
  to end today.
- `check_resume_permission`, the resume repair of a trailing tool call, and the fork and list
  operations all have tests and no caller.

This is the same family as the credential gate that guarded nothing until a call site
existed. Code that exists but is never connected is not a feature.

## What this decision does not do

It does not fix it. Wiring the recorder needs the resume permission check, a session
identifier in the CLI, a `--resume` flag, and a decision about what the TUI shows on a
resume. That is a feature with its own spec, and bolting it onto the reasoning work would
ship an unreviewed session lifecycle.

`AGENTS.md` says to fix a verified bug even outside the diff, or to say plainly that the fix
is too large now. This is the second case, and this file is the saying.

## What the reasoning work does about it

The persisted format of a reasoning block is proved at the unit level, in both directions:

- `a_state_round_trips_through_the_session_file`
- `an_old_thinking_block_imports_as_a_trace`
- `a_replay_key_with_no_state_reads_as_a_trace`
- `a_new_block_is_readable_by_an_old_rho`
- `a_value_over_the_record_cap_is_dropped_and_reported`

Those cover the format. They cannot cover a lifecycle that has no caller, and the
verification file says so rather than implying a live resume was tested.

## What the reader now guarantees, with no caller

Two security reviews hardened this path while it still has no production writer, because the
reader is a contract with every file rho will ever write. The reader now:

- bounds every field of a record it reads, with the same caps the write path applies,
- bounds the whole record, because the write path does,
- skips a bad record in the middle, counts it, and stops at a ceiling,
- ignores an unknown field at every level, so a file from a later rho still loads.

Neither `truncated_tail` nor `dropped_records` has a consumer, because no resume path exists.
A reviewer named that, and it is the same absence this decision records.

## The next step

One spec for the session lifecycle: when rho creates a file, when it resumes, what a resume
may not widen, and what the frontends draw. Then wire it, and drive it for real. Until then,
a reasoning payload replays only inside one process, where the transcript lives in memory.
