# D-a-resume-into-the-tui-replays-but-does-not-repaint — the TUI replays into context only

Date: 20260918

## The question

A `session-file` config key can reopen an existing session in the TUI. `open_recording` then
returns rebuilt messages. Does the TUI redraw those messages as transcript rows, or not?

## The decision

**The TUI replays the messages into the session context, and it does not repaint them.** It
calls `Session::replay`, the same call the headless path uses. So the model keeps the earlier
conversation. The transcript stays empty of the old turns. The user sees one notice instead,
`continuing the session file <path> (<n> messages)`, which `open_recording` already returns.

The replay runs in `run_interactive`, before the app starts. It runs after `build_session`
and before `App::with_recorder`. So the recorder folds only new records, and the old messages
never fold again. The `Recording` stays bound for the run, so the lock stays held. See
`SPEC-the-interactive-session-records-itself` section 0 and section 12.

## The reason

Replay into the context is cheap and correct. It gives the model its history, which is the
point of a resume. It reuses `Session::replay`, so it adds no new path.

Repainting the history as transcript rows is a larger feature. It must turn stored records
back into rows, wrap each one, and scroll them. That work belongs to its own spec. A notice is
honest and small, and it tells the user the resume worked.

## What this rules out

- A silent resume. The notice names the file and the message count.
- A repaint of stored records in this spec. That is a separate feature.
- A second replay path. The TUI reuses `Session::replay`.
- A replay that folds into the file again. The replay runs before the recorder attaches.
