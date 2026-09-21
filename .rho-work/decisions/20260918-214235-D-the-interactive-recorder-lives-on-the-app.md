# D-the-interactive-recorder-lives-on-the-app — the TUI app owns the recorder and folds one seam

Date: 20260918

## The question

The interactive TUI must record its session. The recorder can live on the `Session`, or on
the TUI app. Where does it live, and what drives the prompt, the events, the model change, the
cancel, and the close?

## The decision

**The app owns the one recorder.** `App` holds one `Option<SessionRecorder>`, and `rho-cli`
attaches it with `App::with_recorder`. One function, `record_turn`, folds every record: the
prompt, each event, each model change, the cancel, and the close. The event loop calls
`record_turn`, and it never calls the recorder itself.

The app holds the `rho-core` `SessionRecorder`, not the `rho-cli` `Recording`. So `rho-tui`
gains no dependency on `rho-cli`.

**One recording mechanism, not two.** The `rho-core` session-level recorder is the dead second
mechanism. `Session::with_recorder` and the `record_model_change` branch in
`Session::set_selection` have no production caller. This change deletes them, and it deletes
the `recorder` field on `Session`. The TUI records a model change through the seam, not through
the session. See `D-dead-surface-is-a-defect-class`.

`SPEC-switch-the-provider-mid-session` assumed session-level recording of a switch. It must
follow this choice. A live switch records its `ModelChange` through the app seam, after a
successful `apply_selection`, not inside `rho-core`.

## The reason

The prompt and the events arrive at TUI loop points. The submit arm holds the prompt, and the
agent-event arm holds each event. A recorder must be reachable there. `rho-tui` cannot depend
on `rho-cli`, so it cannot hold `Recording`. It can hold `SessionRecorder`, because that type
is in `rho-core`.

A single recorder value has one owner. The app owns it, so the session cannot. So the
session-level recorder is dead on every production path, and dead surface is a defect here.

A grep cannot prove a call is live, in order, or has the right argument. The headless path
learned this and put its whole lifecycle in one function, `record_and_print`. One function a
test drives is the proof. `record_turn` is that function for the TUI.

Two small functions keep the seam honest. `submit_prompt` decides that a key starts a turn, so
a steer records no prompt. `selection_record` decides that a change records a `ModelChange`, so
an effort-only change records nothing. Each is a pure function a test drives.

## What this rules out

- A second recorder on the `Session` in the TUI. One recorder exists, and the app owns it.
- A session-level recorder anywhere. The `rho-core` recorder surface is deleted.
- A recording lifecycle spread across the match arms. Every record goes through one seam.
- `rho-tui` depending on `rho-cli`. The app holds the `rho-core` recorder alone.
- A `close` driven from inside a match arm. `App::run` records the close once, after the loop
  returns, so a cancel never closes the file. See `D-cancel-keeps-the-session-open`.
- A cancel that leaves an open tool call unpaired. The cancel arm folds `Cancel`, which calls
  `SessionRecorder::record_cancel`. See `D-cancel-keeps-the-session-open`.
- A silent mid-run write failure. The seam pushes one transcript notice on a fresh degrade.
  See `D-write-failure-degrades` and `D-a-notice-reaches-the-transcript`.
