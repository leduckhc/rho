# SPEC-the-interactive-session-records-itself — the TUI writes a session file

Status: delivered.
Owning crate: `rho-tui`, with sides in `rho-cli` and `rho-core`.

Feature: `F-append-only-session-log`. This spec wires that feature to the interactive
path. It also closes a gap in `F-session-resume` and `F-session-list`. It reaches
`F-session-cancel-without-close` too, because the cancel path now has a caller.

This spec delivers item `C11b` from `.rho-work/ledger-model-catalog.md`. That item was
parked. The user has now asked for it.

## The problem

The user wants this: switch the model mid-session, resume tomorrow, and land on the same
model. The interactive TUI writes no session file today, so nothing is there to resume.

## Why it matters

A separate spec, `SPEC-switch-the-provider-mid-session`, designs the resume half. That half
restores what the file recorded. The TUI records nothing, so the restore does nothing. This
spec makes the TUI record. It also makes an interactive session visible to
`rho sessions list` and to `--continue`, because it creates the file.

## The one seam, end to end

A reviewer found the earlier draft designed a seam and never wired it. The lifecycle sat
across `rho-cli`, four loop arms, and `App::run`, with nothing driving it end to end. This
revision fixes both halves. Section 0 states the `run_interactive` handoff as compilable
Rust. Section 10 names the tests. Section 14 names the drive that a cargo test cannot reach.

The headless path proved the pattern. Its seam, `record_and_print`, owns the whole lifecycle
in one function, because a grep cannot prove reachability, order, or an argument. This spec
follows the same rule. See `D-the-interactive-recorder-lives-on-the-app`.

## The sides

Four sides must agree. Each has one owner.

| Side | Owner | What it agrees to |
| --- | --- | --- |
| The recording seam | `rho-tui` | one function folds every session record |
| The app that owns the recorder | `rho-tui` | the app holds the recorder and calls the seam |
| The recorder handoff and the lock | `rho-cli` | open the file, give the recorder, keep the lock |
| The append-only recorder | `rho-core` | fold a prompt, an event, a change, a cancel, a close |

The contract kinds this change touches are the public API and the persisted format. It adds
no new record type. It touches no wire format and no new config key.

The persisted format is unchanged. The recorder writes the records the headless path writes.
So an old reader parses a new file with no error.

## One recording mechanism, not two

The app owns the one recorder. The headless path owns its recorder inside `Recording`, and
this path owns its recorder inside `App`. See `D-the-interactive-recorder-lives-on-the-app`.

The `rho-core` session-level recorder is the dead second mechanism. `Session::with_recorder`
and the `record_model_change` branch in `Session::set_selection` have no production caller.
Only `crates/rho-core/tests/selection.rs` reaches them. Shipping the app recorder as well
would leave that `rho-core` surface dead. Dead surface is this project's named defect class.
See `D-dead-surface-is-a-defect-class`.

**This change deletes the dead `rho-core` surface.** It deletes `Session::with_recorder`, the
`recorder` field on `Session`, and the recorder branch in `Session::set_selection`. The app
records a model change through the seam instead. Section 15 lists the exact edits for the
controller, because a spec writes no Rust.

`SPEC-switch-the-provider-mid-session` assumed session-level recording of a switch. It must
follow this choice. A live switch records its `ModelChange` through the app seam, after a
successful `apply_selection`, not inside `rho-core`. The controller amends that draft. This
spec does not edit it beyond the one line already there.

## 0. The `run_interactive` handoff

`run_interactive` builds the session and the app today, and it opens no recording. This is
the missing call site. Every block below is compilable Rust. The new lines open the
recording, hold it for the lock, hand the recorder to the app, replay the messages, and emit
the session-id notice.

```rust
// crates/rho-cli/src/cli.rs, inside run_interactive, after provider_name is resolved

// Open the session file before the provider runs, exactly as the headless path does. The
// TUI has no --continue flag, so the selector is New. A `session-file` config key still
// reopens a named file inside `open_recording`. See section 6.
let request = RunRequest {
    prompt: String::new(),
    selector: SessionSelector::New,
    ephemeral: loaded.ephemeral,
    allow_widen: false,
};
let mut recording = match open_recording(&loaded, &config, &provider_name, &request, &home_dir())
{
    Ok(recording) => recording,
    Err(error) => return fail(error),
};
// The notices ride the transcript, never stderr. The id and the path ride it too, so the
// user can resume this file. See ruling 5 and section 5.
notices.extend(recording.notices.iter().cloned());
if let (Some(id), Some(path)) = (&recording.id, &recording.path) {
    notices.push(format!("session {} at {}", id.as_str(), path.display()));
}

// build_session takes `config` by value, so open the recording first.
let (session, tasks, extras) = match build_session(cli, &loaded, config).await {
    Ok(triple) => triple,
    Err(error) => return fail(error),
};
notices.extend(extras.notices.iter().cloned());

// Replay the rebuilt conversation into the context before the app starts. The TUI does not
// repaint the old rows. See `D-a-resume-into-the-tui-replays-but-does-not-repaint`.
if !recording.messages.is_empty() {
    session
        .replay(std::mem::take(&mut recording.messages))
        .await;
}

// Move the recorder to the app, and keep `recording` bound. `take_recorder` leaves the lock,
// the id, and the path inside `recording`, so the file stays locked for the run. See ruling 6
// and section 12.
let recorder = recording.take_recorder();

let mut app = rho_tui::App::new(session, model)
    .with_mouse(mouse)
    .with_motion(motion)
    .with_reasoning(reasoning)
    .with_reasoning_effort(initial_effort)
    .with_starred_models(starred)
    .with_suggested_models(suggestions)
    .with_task_events(tasks.session_events())
    .with_context(cwd, branch, provider_name)
    .with_notices(notices)
    .with_recorder(recorder);
if let Some(catalog) = catalog {
    app = app.with_catalog(catalog);
}

let code = match app.run().await {
    Ok(()) => 0,
    Err(error) => fail(anyhow::anyhow!(error)),
};
drain_mcp(&extras).await;
// Drop the recording after the run, so the lock outlives `app.run()`. A one-line
// `open_recording(...).take_recorder()` would drop the lock at once. See ruling 6.
drop(recording);
code
```

The `drop(recording)` line is deliberate. It binds the lock for the life of `app.run()`, and
it makes the lifetime visible to a reader. The type system does not enforce it, so a live
drive is the guard. See section 12 and section 14.

## The seam

The seam is one function, `record_turn`, in `rho-tui`. It folds one loop transition into the
session file. The event loop calls it, and never inlines a recorder call. So the recording
lifecycle lives in one place a test drives, not across five match arms.

### The seam and its steps, in `rho-tui`

```rust
// crates/rho-tui/src/app.rs

use rho_core::{AgentEvent, ContentBlock, SessionRecorder};

/// One transition of the event loop that the session file must reflect.
///
/// The loop passes one of these to `record_turn`. The loop never calls the recorder itself,
/// so the whole recording lifecycle lives in one function a test drives. See
/// `D-the-interactive-recorder-lives-on-the-app`.
pub enum TurnRecord<'a> {
    /// A turn began. The submit arm carries the submitted input.
    Prompt(&'a [ContentBlock]),
    /// One agent event arrived. The agent-event arm folds it.
    Event(&'a AgentEvent),
    /// The model changed. `provider` is the session's running provider, fixed for the run
    /// today, so it comes from the session and not from the selection. See ruling 4.
    Selection { provider: &'a str, model: &'a str },
    /// The turn was cancelled. The cancel arm carries this. It flushes the partial turn,
    /// completes any open tool pairing, and writes one stop. The session stays open. See
    /// ruling 2 and `D-cancel-keeps-the-session-open`.
    Cancel,
    /// The run ended on its own. `App::run` calls this after the loop returns. A cancel never
    /// reaches here. See `D-cancel-keeps-the-session-open`.
    Close,
}

/// Fold one loop transition into the session file. This is the recording seam.
///
/// A `None` recorder records nothing. A write failure degrades the log to ephemeral inside
/// `rho-core`, and the run continues. The alternate screen hides a tracing warning, so a
/// fresh degrade pushes one transcript notice here. See ruling 5, section 7, and
/// `D-write-failure-degrades`.
pub fn record_turn(
    recorder: &mut Option<SessionRecorder>,
    state: &mut TuiState,
    step: TurnRecord<'_>,
) {
    let Some(recorder) = recorder.as_mut() else {
        return;
    };
    let was_live = !recorder.is_ephemeral();
    // Read whether this is a close before the match moves `step`.
    let is_close = matches!(&step, TurnRecord::Close);
    match step {
        TurnRecord::Prompt(input) => {
            recorder.record_prompt(input);
        }
        TurnRecord::Event(event) => {
            recorder.observe(event);
        }
        TurnRecord::Selection { provider, model } => {
            recorder.record_model_change(provider, model);
        }
        TurnRecord::Cancel => {
            recorder.record_cancel();
        }
        TurnRecord::Close => {
            if let Err(error) = recorder.close() {
                // The screen is closing here, so stderr is visible again. See section 4.
                tracing::warn!(%error, "the session file could not state its close");
            }
        }
    }
    // A record that just degraded the log to ephemeral is a silent drop unless the user is
    // told. The alternate screen hides tracing, so the notice rides the transcript. Close is
    // exempt, because it does not degrade the log and the screen is already closing.
    if was_live && !is_close && recorder.is_ephemeral() {
        state.push_error(
            "the session file could not be written. rho keeps running, and this session is \
             now ephemeral."
                .to_string(),
        );
    }
}
```

### The builder, in `rho-tui`

The `App` gains one field, `recorder`, and one builder. `None` means the app records nothing,
which keeps every current test green.

```rust
// crates/rho-tui/src/app.rs, inside impl App

/// Give the app the recorder that folds the session file.
///
/// The app owns the recorder for the life of the run. It folds the prompt, every event, each
/// model change, the cancel, and the close through `record_turn`. Without this call the app
/// records nothing, exactly as before. See `D-the-interactive-recorder-lives-on-the-app`.
pub fn with_recorder(mut self, recorder: SessionRecorder) -> Self {
    self.recorder = Some(recorder);
    self
}
```

### The prompt classifier, in `rho-tui`

One function decides that a key starts a turn and records a prompt. The loop records only
what it returns. So a steer can never record a prompt, and a test drives the decision.

```rust
// crates/rho-tui/src/app.rs

/// The prompt a key submits, or `None` for a key that starts no turn.
///
/// Only `Submit` starts a turn and records a prompt. `Steer` joins the running turn, so it
/// returns `None`. This is the one place that decides a prompt record, so a test drives it.
/// See section 2 and `a_steered_message_records_no_prompt`.
fn submit_prompt(action: &KeyAction) -> Option<Vec<ContentBlock>> {
    match action {
        KeyAction::Submit(text) => Some(vec![ContentBlock::Text { text: text.clone() }]),
        _ => None,
    }
}
```

### The model-change guard, in `rho-tui`

One function decides that an applied selection records a model change. It records only when
the model id changed. So an effort-only change writes no redundant record, and the file shape
matches the headless file. See ruling 3.

```rust
// crates/rho-tui/src/app.rs

/// The record an applied selection produces, or `None` when nothing changed.
///
/// It records only when the model id changed. An effort-only change reselects the same model,
/// so it writes no `ModelChange`, exactly like the headless guard. `provider` is the running
/// provider, fixed for the run today. See rulings 3 and 4, and
/// `D-model-selection-is-mutable-behind-a-mutex`.
fn selection_record<'a>(
    previous_model: &str,
    current: &'a ModelSelection,
    provider: &'a str,
) -> Option<TurnRecord<'a>> {
    if current.model == previous_model {
        return None;
    }
    Some(TurnRecord::Selection {
        provider,
        model: &current.model,
    })
}
```

### The recorder handoff, in `rho-cli`

The interactive path opens a `Recording`, the same way the headless path does. It gives the
recorder to the app, and it keeps the lock. So the file stays locked for the whole run.

```rust
// crates/rho-cli/src/recording.rs, inside impl Recording

/// Take the recorder, and leave an inert one behind.
///
/// The app folds the session file, so the recorder moves to the app. The lock, the id, and
/// the path stay here, so the file stays locked and named for the run. The inert recorder
/// left behind writes nothing, so a later `close` on this value is safe. The caller must
/// keep the `Recording` bound for the life of the run. See ruling 6 and section 12.
pub fn take_recorder(&mut self) -> SessionRecorder {
    std::mem::replace(&mut self.recorder, SessionRecorder::new(SessionLog::Off))
}
```

## 1. Where the seam is called

The event loop calls `record_turn` at five points. `App::run` calls it once more. Each call
forwards one value, so no arm holds logic a grep cannot see.

- The submit arm passes `Prompt`. It records the input the classifier built.
- The agent-event arm passes `Event`. It folds each event before the state applies it.
- The apply-selection arm passes `Selection`, but only when `selection_record` returns one.
- The cancel arm passes `Cancel`. It records the stop and keeps the session open.
- `App::run` passes `Close` after the loop returns, on success and on error alike.

The five loop-arm calls own the terminal. A cargo test cannot reach them. Section 14 names
the drive that does.

## 2. When a turn starts and closes

A turn starts on `KeyAction::Submit`. The submit arm records the input with `Prompt`, then
starts the stream. So the file states the prompt before the answer arrives.

A turn does not start on `KeyAction::Steer`. `submit_prompt` returns `None` for a steer, so
the seam records no prompt. The steered text reaches the file inside the running turn's
events, folded by the agent-event arm.

The close is a session event, not a turn event. `App::run` records `Close` once, after the
loop returns. So one clean run writes one close.

## 3. What happens on cancel

`D-cancel-keeps-the-session-open` governs this. A cancel stops the turn and keeps the session
open. The cancel arm cancels the token, then passes `Cancel` to the seam. It never records
`Close`.

The seam calls `SessionRecorder::record_cancel`. That call flushes the turn, writes any bare
tool call, and synthesises a result for every open tool call. Then it writes one stop record.
So a cancelled turn never leaves a `ToolCall` on disk with no matching `ToolResult`. The
session file stays open, and the user can prompt again.

`record_cancel` sat in `bench/allowed-uncalled.txt` because no frontend had an interrupt
path. The cancel key is that path, so this change gives `record_cancel` a production caller.
Section 15 says to retire the allowlist line.

## 4. What happens on each exit path

The TUI can end in five ways. The table states what the file holds in each case.

| Exit path | Does close run? | What the file holds |
| --- | --- | --- |
| `KeyAction::Exit` | Yes | a closed session |
| The input stream ends | Yes | a closed session |
| An error returns from the loop | Yes | a closed session |
| A fatal signal | No | an open session, still resumable |
| A panic that unwinds | No | an open session, still resumable |

The first three paths return control to `App::run`. `App::run` records `Close` after the
loop returns, on the success path and the error path alike. So all three guarantee a close.

A fatal signal cannot guarantee a close. `spawn_signal_restore` restores the terminal, and
the process then ends before any close runs. A panic cannot guarantee a close either. The
stack unwinds through the screen guard, and the close call never runs.

An open session is not a broken session. `SessionStore::newest_resumable` resumes an open
file, so `--continue` still works. The close record only helps the crash offer tell a clean
exit from a crash. So an un-closed file loses that hint, and nothing more.

## 5. Where the session id reaches the user

The id and the path reach the user through the transcript, never through stderr. The TUI
opens the alternate screen, so a line printed to stderr first is lost.
`D-a-notice-reaches-the-transcript` settled this. Section 0 pushes one notice,
`session <id> at <path>`, into the list `App::with_notices` already draws.

## 6. What a resume into the TUI does

The interactive path has no `--continue` flag today, so a fresh run creates a new file. A
`session-file` config key can still name an existing file. Then `open_recording` reopens it
and returns the rebuilt messages.

The handoff replays those messages into the session context, the same way the headless path
does, with `Session::replay`. So the model keeps the earlier conversation. The TUI does not
repaint the old messages as transcript rows, because rendering resumed history is a separate
feature. The user sees a notice instead, which `open_recording` already returns. See
`D-a-resume-into-the-tui-replays-but-does-not-repaint`.

## 7. What a write failure does

`D-write-failure-degrades` governs this. `SessionLog::record` degrades the log to ephemeral
on a write failure, and the run continues. The degrade happens inside `rho-core`, so the seam
sees no error and the loop keeps running.

A silent degrade is a defect. A warning through `tracing` is silent here, because the
alternate screen hides it. So `record_turn` watches for the moment the log turns ephemeral
and pushes one transcript notice. A disk that fills at turn 40 tells the user at turn 40,
rather than costing hours in silence. `D-a-notice-reaches-the-transcript` is the precedent.

An open failure degrades too. `open_recording` maps a new-session I/O failure to
`Recording::degraded`, which returns an inert recorder and one notice. The notice reaches the
transcript through section 0. So a run with no writable session directory still starts, and
the user learns the session is ephemeral.

## 8. What the recorder costs

The recorder appends one JSON line per record, and it flushes each line. `SessionWriter::append`
writes one line and flushes once. A plain turn appends the prompt and the assistant message. A
turn that calls tools also appends one record per tool call and per result. A model change
appends one record. A clean close appends one record. So the per-turn write count is small and
bounded by the turn's own content.

rho treats memory and speed as features. This spec claims no throughput number, because it
measured none.

### The blocking-I/O trade, accepted in writing

`SessionWriter::append` writes and flushes per record, on the event-loop thread. The
agent-event arm folds every streamed event. A slow or full disk can therefore stall key
input, redraw, and cancel. No guard crosses an await on this path, so there is no deadlock.
The responsiveness cost is real, and this spec accepts it for now. The reasons are these:

- The per-turn write count is small and bounded, so the common case is cheap.
- A persistently failing disk degrades to ephemeral after one record, so a broken disk stops
  costing writes at once and the user learns why.
- An off-thread writer is a real feature, not a tweak. It needs a bounded channel,
  back-pressure, and a drain on close, because the close record must be the last write. A
  dropped queue on exit would lose the close.
- rho claims no latency number here, because it measured none.

The follow-up, if a measurement later shows a real stall: move the writer to its own task,
behind a bounded channel, and drain it before the close. That is out of scope for this spec.

## 9. Redaction, and the widened exposure

`redact_block` redacts `ToolCall.arguments` and recurses into a `ToolResult`. A plain `Text`
block inside a result falls to a wildcard arm, `other => other.clone()`, and reaches disk
unredacted. A `bash` result that prints a secret is written verbatim.

This defect exists today on the headless path, so this change does not cause it. But this
change writes interactive shell output to disk for the first time. So it widens the exposure
a lot. The spec is not silent on it.

### The small fix this change makes

The wildcard is a fail-open shape. A new content block joins it by default, exactly as
`ToolKind::Other` once approved any tool. See `D-plugin-does-not-classify-itself`. This change
replaces the wildcard with named arms for `Text` and `Image`. The behaviour is unchanged, and
the compiler now forces the next block type to state its redaction. Section 15 lists the edit.

### The larger fix, recorded as a known defect

A full fix redacts a secret inside free text, by construction. It is too large for this spec,
for real reasons:

- `rho_redact::redact_json_secrets` reads a JSON key name. It cannot see a bare `KEY=value`
  in free text. So a new text scrubber is needed.
- A text scrubber risks a false positive that corrupts a replayable tool result.
- `D-bash-scrubs-credentials` already removes credential-shaped variables from the child
  environment. So the common environment leak is closed at the source. The residual is a
  secret the model reads from a file, which no harness redacts reliably today.

So this stays a known defect. The follow-up needs its own decision and spec: a text secret
scrubber over `ToolResult` text, with a measured false-positive rate. `D-redact-tool-arguments`
is the precedent for redacting by construction. rho never leaves a verified bug hidden, so this
records the bug and its reason plainly.

## 10. What the contract forbids

- A recorder call inlined in a match arm. Every record goes through `record_turn`.
- A `close` on cancel. The cancel arm keeps the session open.
- A prompt record on steer. `submit_prompt` returns `None` for a steer.
- A model change record for an effort-only change. `selection_record` returns `None`.
- A second recorder on the session in the TUI. The app owns the one recorder.
- A session-level recorder anywhere. The `rho-core` recorder surface is deleted.
- The app depending on `rho-cli`. The app holds the `rho-core` recorder, not `Recording`.
- A dropped lock during the run. The caller keeps the `Recording` alive for the lock.
- A session id printed to stderr before the screen opens. The id rides the transcript.
- A run ended by a write failure. A failure degrades the log, and the run continues.
- A silent mid-run degrade. A degrade pushes one transcript notice.
- A wildcard arm in `redact_block`. Every block type states its redaction.

## 11. Test cases

Each test lives in this repo when the status flips to `delivered`. Each row names the file
and the assertion. Every test here is a cargo test that runs with no terminal.

### `rho-tui` — the seam `record_turn`

- `the_turn_seam_records_prompt_events_and_close_in_order` —
  `crates/rho-tui/tests/recording.rs`. A file-backed recorder folds a prompt, a scripted
  turn, and a close. `SessionReader` then shows a user message, an assistant message, and a
  close, in that order. A seam that skips the prompt fails this test.
- `a_selection_step_records_a_model_change` — `crates/rho-tui/tests/recording.rs`.
  `record_turn` with `Selection` writes one `ModelChange` whose provider and model match the
  step.
- `an_absent_recorder_records_nothing` — `crates/rho-tui/tests/recording.rs`. `record_turn`
  with a `None` recorder is a no-op, and it never panics.
- `a_degraded_recorder_routes_no_further_record_to_its_writer` —
  `crates/rho-tui/tests/recording.rs`. A recorder over a counting sink that fails its first
  write degrades on the first step. Every later step adds zero writes to the sink. This
  replaces a vacuous off-recorder test, and it pins that no record reaches the writer after
  the degrade.
- `a_mid_run_write_failure_reaches_the_transcript` — `crates/rho-tui/tests/recording.rs`. A
  recorder over a failing sink, folded through `record_turn`, turns ephemeral and pushes
  exactly one transcript error row. A seam that only warns through `tracing` fails this test.
- `a_cancel_records_a_stop_and_completes_open_tool_calls` —
  `crates/rho-tui/tests/recording.rs`. A scripted turn with an open tool call, then a
  `Cancel`, writes one stop and a synthetic result for the open call. The file holds no
  `ToolCall` without a `ToolResult`, and it holds no close. A later `Prompt` still appends,
  so the session stayed open.

### `rho-tui` — the classifier and the guard

- `a_steered_message_records_no_prompt` — `crates/rho-tui/src/app.rs`. `submit_prompt` of a
  `Steer` returns `None`, and `submit_prompt` of a `Submit` returns the input. So a future
  edit that folds a steer into a prompt fails this test.
- `an_effort_only_change_records_no_model_change` — `crates/rho-tui/src/app.rs`.
  `selection_record` with the same model id and a different effort returns `None`.
- `a_model_change_yields_a_selection_record` — `crates/rho-tui/src/app.rs`.
  `selection_record` with a new model id returns a `Selection` whose model is the new id and
  whose provider is the running provider.

### `rho-tui` — the builder

- `with_recorder_sets_the_recorder_field` — `crates/rho-tui/src/app.rs`. `App::with_recorder`
  sets the private `recorder` field to `Some`, and `App::new` leaves it `None`. This is a unit
  test, because the field is private and the loop needs a terminal. An earlier draft named a
  test that drove the private field from outside the crate. That name is retired, because no
  external test can reach a private field. See bench/deleted-tests.txt for deleted tests.

### `rho-cli` — the handoff

- `take_recorder_leaves_an_inert_recorder` — `crates/rho-cli/src/recording.rs`.
  `take_recorder` returns the live recorder, and the value it leaves behind is off. The id and
  the path stay set, so the file stays named.
- `take_recorder_keeps_the_lock` — `crates/rho-cli/src/recording.rs`. The `Recording` still
  holds its lock after `take_recorder`, so a second open of the same file returns
  `SessionError::Busy`. This proves the mechanism. Section 14 proves the lock over a live run.

## 12. The lock over a live run

`take_recorder` moves the recorder out and leaves the lock inside `Recording`. So the lock
lives only as long as the `Recording` value. Section 0 binds `recording` for the whole
function and drops it after `app.run()`. So the lock outlives the run.

The type system does not enforce this. A one-line `open_recording(...).take_recorder()` would
drop the `Recording` at the end of that statement, and the lock would release in silence while
the app still runs. So `take_recorder_keeps_the_lock` proves the mechanism, and the live drive
in section 14 proves the binding over a real run.

## 13. What a second rho does

A second rho that opens the same session file, while the TUI holds the lock, is refused.
`open_recording` runs before the screen opens. For a named file it reaches
`SessionStore::lock_file`, which returns `SessionError::Busy`. `open_recording` treats a
non-I/O error as fatal, so the second rho stops with a clear error before it draws anything.
See `D-a-live-session-holds-a-lock`. A default TUI run mints a fresh file each time, so two
plain runs never clash.

## 14. Verification by driving

The five loop-arm calls and the lock-over-a-run promise own the terminal. A cargo test cannot
reach them. So `docs/verification/` is the named guard, exactly as the headless lane used it
for wiring a cargo test could not see. Build the release binary and drive it in a pty. Record
the commands and the real output.

- **A turn writes prompt, events, and close, in order.** Drive one prompt, let the turn end,
  exit, then read the session file back. Assert a user message, an assistant message, and a
  close, in that order.
- **A cancel keeps the session open.** Drive one prompt, press the cancel key mid-turn, then
  prompt again. Read the file back. Assert one stop, no close, and a second prompt.
- **A steer writes no extra prompt.** Drive one prompt, steer a message mid-turn, let the turn
  end. Assert exactly one user prompt record for the turn.
- **The lock holds while the app runs.** Start the TUI with a `session-file`. Start a second
  rho on the same file. Assert the second rho exits with the busy error before any screen
  opens.

## 15. Edits for the controller

A spec writes no Rust and no `docs/features.md` row. These edits carry this contract, and the
controller owns them.

- **Delete the dead `rho-core` recorder surface.** Remove `Session::with_recorder`, the
  `recorder` field on `Session`, and the `record_model_change` branch in
  `Session::set_selection`. `Session::set_selection` keeps swapping the selection.
- **Delete the two tests that pinned the deleted surface.** Remove
  `model_change_writes_a_model_change_record` and
  `unchanged_selection_does_not_write_a_model_change_record` from
  `crates/rho-core/tests/selection.rs`. Record each in `bench/deleted-tests.txt`, with the
  commit and the reason.
- **Retire two lines from `bench/allowed-uncalled.txt`.** Remove the
  `crates/rho-core/src/agent.rs::with_recorder` line, because the function is deleted. Remove
  the `crates/rho-core/src/session/mod.rs::record_cancel` line, because the cancel arm now
  calls it.
- **Harden `redact_block`.** Replace the `other => other.clone()` wildcard with named arms
  for `ContentBlock::Text` and `ContentBlock::Image`.
- **`docs/features.md` row.** Set `F-append-only-session-log` to name the interactive path.
  Suggested row text: `F-append-only-session-log | The session log records the headless run
  and the interactive TUI | rho-core, rho-cli, rho-tui | delivered | record_turn seam in
  rho-tui`.
- **`SPEC-choose-a-model-and-configure-a-run` (draft).** It names the two deleted tests. Amend
  it to name the app-seam tests instead. It is a draft, so the gate exempts it now.
- **`SPEC-switch-the-provider-mid-session` (draft).** It records a switch inside
  `apply_selection`. Amend it to record through the app seam, after a successful switch. Do
  not touch it in this change.

## 16. Out of scope

- **A `--continue` flag for the interactive TUI.** A fresh run creates a new file. A
  `session-file` config key can still reopen one.
- **Repainting resumed history as transcript rows.** The TUI replays into the context and
  shows a notice. See `D-a-resume-into-the-tui-replays-but-does-not-repaint`.
- **A new session record type.** The recorder writes the records it already writes.
- **Moving conversation recording into the `rho-core` driver.** The headless path keeps its
  external `Recording`, and this spec changes no headless behaviour.
- **An off-thread session writer.** The write stays inline. See section 8.
- **A free-text secret scrubber for a tool result.** It is a known defect with a named
  follow-up. See section 9.
- **A provider switch.** `SPEC-switch-the-provider-mid-session` owns the switch. This spec
  records the `ModelChange` a switch or a model pick produces.
