# SPEC-tui-inline-and-composer — An inline band, a transcript the terminal keeps, and a real composer

Status: draft. Sprint 4, stages S1 to S3.
Owning crate: `rho-tui`. Consumed by `rho-cli` and `rho-config`.

This spec replaces `SPEC-tui-scroll-copy-composer`, which built in-app scrolling on a false
premise. It extends `SPEC-tui` and `SPEC-tui-experience`.

It answers one user report: the transcript cannot scroll, the mouse cannot select, and the
composer in the running product is a `String`.

The architecture answers the first two. rho draws an inline band and gives every finished
row to the terminal. See `D-inline-viewport-not-alternate-screen`, and the evidence in
`docs/verification/inline-viewport-spike.md`.

Features covered: F-inline-band, F-freeze-upward, F-optional-mouse, F-composer-editing,
F-draft-history, and F-external-editor.

## 0. The rules this spec obeys

1. The state is a value, and the reducer is pure. No IO enters `state.rs`.
2. The renderer is pure, and it reads no clock.
3. The key handler returns an action. The event loop is the only place that does IO.
4. Every promise on screen answers a key. See `D-a-panel-nobody-can-open`.
5. A row is immutable once the terminal owns it. See `D-a-frozen-row-never-repaints`.

## 1. The shape

```
  shell line: cargo test                     ← the user's own scrollback, untouched
  ρ rho  ~/Work/Vibe/rho · main · sonnet-4.5 · openrouter        ← the banner, frozen once
  ❯ add a duration to every tool row                             ← frozen rows, in the
  Done. Every tool row now carries a duration.                     terminal's scrollback
  ✓ edit  crates/rho-tui/src/render.rs · +18 −4           0.2s
┌─────────────────────────────────────────────────────────────┐
│ ✓ bash  cargo test --workspace                       31.7s  │  the band, which rho owns
│ ∴ thinking                                                  │  live rows
│ ╭─────────────────────────────────────────────────────────╮ │
│ │ ❯ █                                                     │ │  composer
│ ╰─────────────────────────────────────────────────────────╯ │
│ working 12s · 48.2k in, 3.1k out · enter send · ? help      │  footer
└─────────────────────────────────────────────────────────────┘
```

The outer box is not drawn. It marks what `Viewport::Inline` owns.

Three consequences, and each one is a rule:

- The terminal scrolls, selects, and searches the transcript. rho writes no code for that.
- A row above the band is immutable, so it must be final before it goes there.
- The live area is bounded, so a long turn shows its tail until its rows freeze.

## 2. The band

```rust
/// The band height rho asks for, in rows.
///
/// One footer, a composer of up to ten rows, and at least three live rows.
pub const BAND_ROWS: u16 = 14;

/// The band height for a terminal of `height` rows.
///
/// The band never takes the whole terminal, because the shell prompt returns below it. A
/// terminal shorter than `BAND_ROWS + 1` gets `height - 1`. A one-row terminal gets one.
pub fn band_rows(height: u16) -> u16;
```

`rho-tui` opens the terminal with `Viewport::Inline(band_rows(height))`. It never enters the
alternate screen.

The height is fixed for the life of the `Terminal`, because `Viewport::Inline` carries it
and no setter exists. The spike proved that this is enough: the layout inside the band
moves, so the draft grows from one row to four inside one band.

The regions inside the band, top to bottom:

| Rows | Region | Fixed or grows |
| --- | --- | --- |
| rest | Live rows of the current turn | grows into the space the composer leaves |
| 3 to 10 | Composer box | grows with the draft, to `COMPOSER_MAX_TEXT_ROWS` plus two |
| 0 to 7 | A transient panel | absent by default, and it borrows from the live rows |
| 1 | Footer | fixed |

The degradation order inside the band, as the band shrinks: the live rows go first, then
the footer, then the composer border. The composer input row is the last survivor.

The header of `SPEC-tui-experience` section 1 leaves the band. It becomes a banner, frozen
once at startup, because the working directory and the model do not change during a
session. The live counters move to the footer.

```rust
/// The one-line banner, frozen above the band when the session starts.
pub fn banner_line(state: &TuiState, width: usize) -> String;
```

## 3. Freezing a row upward

### 3.1 Finality

```rust
/// True when nothing can change this row again.
///
/// One arm per `Row` variant, and no wildcard arm, so a new variant fails the build. See
/// `D-row-finality-is-explicit` for the table of rules and the reducer path behind each.
pub fn row_is_final(state: &TuiState, index: usize) -> bool;
```

The rules, restated here because a spec must stand alone. "The turn ended" means
`state.activity == ActivityState::Idle`, which `on_agent_end` and `end_run` both set.
`last_stop.is_some()` is not the test, because it is `None` before the first turn.

| Row | Final when |
| --- | --- |
| `User` | always |
| `Assistant` | a newer `Assistant` row exists, or `activity` is `Idle` |
| `Thinking` | a newer `Thinking` row exists, or `activity` is `Idle` |
| `Tool` | the status is `Ok` or `Failed` |
| `Agent` | `finished` is true |
| `Task` | `finished` is true |
| `Error` | always |

The `Assistant` and `Thinking` rules hold on two invariants, and a test pins each. The row
list is append-only, so no row moves. Both delta paths search from the end, so only the
newest row of a kind can grow. Break either invariant and finality breaks silently.

Finality also requires the row's metadata. A `Tool` row carries a duration, and a line
frozen without it would state a wrong fact for ever. Section 3.5 makes the reducer write
that metadata in the same call that makes the row final, so the two can never disagree.

### 3.2 The batch

```rust
/// One batch of final rows, ready for `Terminal::insert_before`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreezeBatch {
    /// The rendered lines, oldest first. They carry no style, and `sanitize_line` has
    /// already run, because the terminal owns these cells after the insert.
    pub lines: Vec<String>,
    /// The count of transcript rows the lines cover.
    pub rows: usize,
}

/// The next batch to freeze, or `None` when the oldest unfrozen row is still live.
///
/// It takes the longest final prefix, so the scrollback keeps the session order. A row
/// behind a live row waits, even when the row itself is final.
pub fn next_freeze(state: &TuiState, width: u16) -> Option<FreezeBatch>;
```

```rust
impl TuiState {
    /// The count of rows the terminal already owns. The band draws no row below it.
    pub frozen_rows: usize,

    /// Record that `rows` more rows left the band. A count past the end saturates.
    pub fn mark_frozen(&mut self, rows: usize);

    /// The rows the band still owns, oldest first.
    pub fn live_rows(&self) -> &[Row];
}
```

### 3.3 The order the loop must keep

Freeze first, then draw. A draw that ran first would show a row the scrollback already
holds, and the user would read it twice.

```rust
// In the event loop, after every state change:
let width = terminal.size()?.width;   // the width this frame will draw at
while let Some(batch) = next_freeze(state, width) {
    let height = u16::try_from(batch.lines.len()).unwrap_or(u16::MAX);
    terminal.insert_before(height, |buf| write_lines(buf, &batch.lines))?;
    state.mark_frozen(batch.rows);
}
draw(terminal, state)?;
```

Three rules bind that loop:

- The freeze width is the width this frame draws at. The loop reads the width once per
  frame, after any resize event. A line frozen at the wrong width wraps wrongly for ever.
- `mark_frozen` runs after the insert that wrote those rows, never before. An insert that
  fails returns the error and ends the session, because a partial write cannot be undone.
  The rows stay unfrozen in the state, so nothing is lost from the session file.
- The blank line that separates turns belongs to the row below it, not above. So a batch
  never ends with a blank line, and the band draws the separator for its first live row.
  Otherwise the seam between the scrollback and the band gains or loses a blank row.

### 3.4 A late event for a frozen row

The reducer finds a tool, an agent, and a task row by id, and those searches walk the whole
row list. A provider that repeats a `ToolEnd`, or sends a `ToolStart` for an id that already
finished, would then mutate a row the terminal owns. The screen cannot change, so the state
and the screen would disagree for ever. Sprint 1 proved that a provider does surprising
things, so this is not a theoretical case.

Rules:

- A lookup for an update returns a **live** row only. An event whose row is frozen changes
  nothing.
- `on_tool_start` pushes a new row only when the id matches **no** row, live or frozen. A
  late start for a frozen id is dropped, and it never pushes a duplicate.
- Dropping is explicit and counted, so a defect is visible rather than silent. See
  `D-a-late-event-for-a-frozen-row-is-dropped`.

```rust
impl TuiState {
    /// The count of events dropped because their row was already frozen. The footer never
    /// shows it. A test asserts it, and a live run prints it at `debug` level.
    pub late_events: usize,
}
```

### 3.5 The reducer owns the row metadata

`row_durations` and `row_bodies` are public fields that **no production code writes**. Only
`crates/rho-tui/tests/frames.rs` and `crates/rho-tui/tests/render.rs` write them. So the
duration ladder of `SPEC-tui-experience` renders for fixtures and never for a user, and
`docs/features.md` claims F-duration-ladder and F-duration-slot ship.

That is a defect this spec must fix, because a freeze writes a line that can never be
repaired. A row must carry its final duration before it leaves the band.

The reducer takes the clock as data, so it stays pure and reads no clock itself. See
`D-the-reducer-owns-the-row-metadata`.

```rust
impl TuiState {
    /// Fold one agent event into the state, at `now_millis` on the caller's clock.
    ///
    /// The clock arrives as data, so the reducer stays pure and a test drives time. The
    /// same call writes the row metadata, so a row and its duration can never disagree.
    pub fn apply(&mut self, event: &AgentEvent, now_millis: i64);
}
```

The metadata the reducer now writes:

| Event | What it writes |
| --- | --- |
| `ToolStart` | the start instant of that tool row |
| `ToolEnd` | `row_durations[index]`, from the start instant |
| `Stream(ThinkingStart)` | the start instant of that thinking row |
| `Stream(ThinkingEnd)` | `row_durations[index]` for the thinking row |
| `TurnStart` | the turn start, for the footer clock |
| `AgentEnd` | `turn_millis`, so the footer states the finished turn |

`row_bodies` stays empty, because the fold keys are out of scope. A row with no body freezes
as its one header line, which is what the band draws today.

Every existing reducer test gains the `now_millis` argument. That is a mechanical change,
and no assertion changes. One entry point takes the clock, so no caller can forget it.

### 3.6 Exit

Exit is reachable in the middle of a run. `/quit` opens through the slash panel while a turn
streams, and `ctrl-d` leaves on an empty draft. So the claim that every row is final at exit
is false, and the loop must make it true.

The exit sequence, in order:

1. Cancel the run, when one is active, with the token the loop holds.
2. Call `end_run`, which sets `activity` to `Idle`. Every row is then final by section 3.1.
3. Freeze what is left, with the loop of section 3.3.
4. Restore the terminal, and leave the cursor below the band.

A row that never finished still freezes, and it freezes as it stands: a running tool row
reads as running. That is the truth of a cancelled session, and the session file holds the
same thing.

## 4. The live area, and the renderer

```rust
/// The live rows as text, newest-anchored, one entry per drawn row.
///
/// It keeps the newest lines when the live rows do not fit, exactly as the transcript did
/// before. A line that scrolls out of the live area is not lost: it reaches the scrollback
/// when its row freezes.
pub fn live_window(state: &TuiState, width: u16, rows: u16) -> Vec<String>;
```

The renderer keeps its entry point and loses its transcript walk:

```rust
/// Draw the band. Pure. No IO. Safe to call every frame.
pub fn render(state: &TuiState, frame: &mut Frame<'_>);

/// The band layout for one frame area. One source of geometry, for the renderer and for
/// the click mapping, exactly as `plan_layout` was.
pub fn plan_band(height: u16, input_rows: usize, panel_rows: usize) -> Band;

/// The slash-command index drawn at screen row `row`, if any. It asks `plan_band`.
pub fn slash_row_index(state: &TuiState, width: u16, height: u16, row: u16) -> Option<usize>;
```

`transcript_window` and `plan_layout` go. `Band` names the same regions as the old layout
struct, minus the header and the rule, plus the live area.

### 4.1 The frame fixtures change, and that is a design change

`crates/rho-tui/tests/frames.rs` compares rendered frames against
`docs/design/tui-frames/*.txt`. Those fixtures start with a header row and a rule row, and
they are 24 rows tall. This spec moves the header into a frozen banner and gives the
renderer a band of about 14 rows. So the renderer cannot produce those frames at all.

These are **assertion** changes, not renames. The spec states that plainly, because the rule
in `AGENTS.md` is to stop and say so. The design changed, on the owner's decision, so the
fixtures follow the design.

The work this implies:

- Regenerate every fixture in `docs/design/tui-frames/` for the band shape.
- Add one fixture for the banner line, which the old set has no equivalent for.
- Keep the width tests and the completeness test, `the_frame_set_is_complete`.
- Update the layout table in `docs/tui-design.md`, so the design and the code agree.

An assertion may only change where the fixture describes the old architecture. A fixture
that fails for any other reason is a defect report.

## 5. The mouse, and the terminal's habits

Mouse capture stays off by default, per `D-native-selection-is-the-default`. With capture
off the wheel, a drag, and the terminal search all work on the transcript, because it is
ordinary output.

```rust
/// The escape sequences that start the interface. `mouse` adds mouse capture.
/// The sequences are data, so a test reads them with no terminal.
pub fn setup_sequences(mouse: bool) -> String;

/// The sequences that give the terminal back. It disables only what the setup enabled.
pub fn restore_sequences(mouse: bool) -> String;

impl App {
    /// Turn mouse capture on. Off by default. On, rho gets the wheel and the clickable
    /// slash list, and the user loses drag-select.
    pub fn with_mouse(self, enabled: bool) -> Self;
}
```

Neither sequence may contain `?1049h`. A test asserts that, because the whole architecture
rests on it.

The config carries the switch as one flat key, because `ConfigLayer` denies an unknown key
and holds no table for the interface.

```rust
// rho-config: one new optional field on ConfigLayer, and one on Config.
pub tui_mouse: Option<bool>,   // file key `tui-mouse`, env `RHO_TUI_MOUSE`
pub tui_mouse: bool,           // resolved, false by default
```

## 6. The composer

### 6.1 The draft replaces the string

```rust
// TuiState: `input: String` is gone.
/// The draft, with its paste chips and its cursor.
pub draft: Composer,

impl TuiState {
    /// The draft as the user sees it, with a chip label for each held paste.
    pub fn draft_text(&self) -> String;
    /// True when the draft holds nothing.
    pub fn draft_is_empty(&self) -> bool;
}
```

Every test that reads `state.input` reads `draft_text()` instead, which is a rename.

Six places **write** the field, and a write is not a rename. `crates/rho-tui/tests/frames.rs`
writes it at lines 329, 478, 534, and 562. `crates/rho-tui/tests/render.rs` writes it at
lines 63 and 106. Each becomes `state.draft.set_text(…)`. Line 478 of `frames.rs` writes
`"/"` to build the slash panel by hand, and it must reach that state through the key handler
instead. A fixture that builds a state no key can build proves nothing, which is
`D-a-panel-nobody-can-open`.

A test whose assertion changes is a defect report, not a rename. Stop and say so.

### 6.2 The cursor moves over units

```rust
/// One unit of the draft, as the cursor moves over it. A chip is one unit, so one left
/// key steps over a whole paste.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// One character of typed text.
    Char(char),
    /// One newline, which starts a display row.
    Newline,
    /// One collapsed paste, shown as its chip label.
    Paste,
}

impl Composer {
    /// The units of the draft, in send order.
    pub fn units(&self) -> Vec<Unit>;
    /// The cursor, as a unit index. Zero sits before the first unit.
    pub fn cursor(&self) -> usize;

    /// Move the cursor. Each one saturates at its end of the draft.
    pub fn move_left(&mut self);
    pub fn move_right(&mut self);
    pub fn move_word_left(&mut self);
    pub fn move_word_right(&mut self);
    pub fn move_line_start(&mut self);
    pub fn move_line_end(&mut self);

    /// Move the cursor one display row. It returns `false` when no such row exists, so the
    /// caller gives the key to the history instead.
    pub fn move_row_up(&mut self, width: usize) -> bool;
    pub fn move_row_down(&mut self, width: usize) -> bool;

    /// Insert a newline at the cursor.
    pub fn insert_newline(&mut self);
    /// Delete the unit after the cursor. A chip deletes whole. At the end it does nothing.
    pub fn delete_forward(&mut self);

    /// Cut into the kill buffer.
    pub fn kill_to_line_end(&mut self);
    pub fn kill_to_line_start(&mut self);
    pub fn kill_word_left(&mut self);
    /// Paste the kill buffer at the cursor.
    pub fn yank(&mut self);

    /// The display rows at this width, wrapped, capped by `COMPOSER_MAX_TEXT_ROWS`.
    pub fn display_lines(&self, width: usize) -> Vec<String>;
    /// The cursor cell, as a row and a column into `display_lines`.
    pub fn cursor_cell(&self, width: usize) -> (usize, usize);
    /// True when the draft holds no unit.
    pub fn is_empty(&self) -> bool;

    /// Replace the whole draft, and put the cursor at the end. It drops every held paste,
    /// because it replaces the whole draft, so `take` then returns exactly this text. It
    /// keeps the kill buffer.
    pub fn set_text(&mut self, text: &str);
    /// Take the model text and empty the draft. The cursor returns to zero.
    pub fn take(&mut self) -> String;
}
```

These items keep their names, their signatures, and their behaviour, because
`crates/rho-tui/tests/paste.rs` calls every one of them:

```rust
impl Composer {
    pub fn new() -> Self;
    pub fn paste(&mut self, text: &str) -> Option<PasteChip>;
    pub fn insert(&mut self, text: &str);
    pub fn backspace(&mut self) -> bool;
    pub fn model_text(&self) -> String;
    pub fn chip_count(&self) -> usize;
    pub fn height_rows(&self) -> usize;
}

pub const COMPOSER_MAX_TEXT_ROWS: usize = 8;
pub const LARGE_PASTE_CHARS: usize = 1000;
```

`insert` and `backspace` now act at the cursor. `backspace` moves the cursor one unit left,
because the unit it deleted was there. A fresh draft holds the cursor at the end, so every
existing paste test passes unchanged. That is the acceptance rule for this refactor.

### 6.3 The newline keys

| Key | Meaning |
| --- | --- |
| `enter` | send the draft |
| `ctrl-j` | insert a newline |
| `alt+enter` | insert a newline |
| `shift+enter` | insert a newline, where the terminal reports the modifier |

`shift+enter` needs the kitty keyboard protocol. A terminal without it reports plain
`enter`, and the help says so. rho requests no protocol upgrade in this spec.

### 6.4 The history

```rust
/// The drafts submitted in this session, oldest first.
pub history: Vec<String>,

impl TuiState {
    /// Recall the previous draft. Return `false` at the oldest entry.
    pub fn recall_previous(&mut self) -> bool;
    /// Recall the next draft, and then the live draft. Return `false` past the end.
    pub fn recall_next(&mut self) -> bool;
}
```

`↑` gives the key to the composer first. When `move_row_up` returns `false`, the key
recalls the previous draft. `↓` mirrors it. A submit appends the text, and it appends
nothing when the text repeats the last entry. The history lives for the session only.

`esc` on an open panel closes the panel. `esc` twice with no panel clears the draft into the
history, so `↑` brings it back. A single `esc` arms that, and any other key disarms it.

### 6.5 Reverse search

```rust
/// The reverse-search panel state.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct HistorySearch {
    pub query: String,
    pub selected: usize,
}

// Panel gains one variant.
pub enum Panel {
    None,
    Approval(Approval),
    SlashList(SlashList),
    Help,
    /// The reverse history search, opened by `ctrl-r`.
    HistorySearch(HistorySearch),
}

/// The history entries that hold `query`, newest first, as indexes into the history.
/// The match is a case-insensitive substring. An empty query matches every entry.
pub fn filter_history(history: &[String], query: &str) -> Vec<usize>;
```

### 6.6 The external editor

```rust
/// The editor command, from `$VISUAL`, then `$EDITOR`, then `vi`.
/// The caller passes the values, so no test reads the real environment.
pub fn editor_command(visual: Option<&str>, editor: Option<&str>) -> String;

/// The program and its arguments, split from an editor value on whitespace.
///
/// The first token is the program, and the rest are arguments. The loop appends the
/// temporary file path last, and runs the program with `std::process::Command`. It never
/// passes the value to a shell, and it never uses `sh -c`. So `vi; rm -rf ~` gives `vi`
/// three inert arguments, and the second command never runs.
pub fn editor_argv(command: &str) -> Vec<String>;
```

`ctrl-x ctrl-e` and `ctrl-g` return `KeyAction::EditDraft(String)` with the draft text. The
loop writes a temporary file, runs the editor, and reads the file back. A failed run pushes
one error row and keeps the draft.

```rust
pub enum KeyAction {
    None,
    Submit(String),
    Cancel,
    Exit,
    /// Edit this text in the editor, then replace the draft with the result.
    EditDraft(String),
}
```

Three variants of the superseded spec are gone: `Copy`, `ViewInEditor`, and
`DumpTranscript`. The terminal owns copying now.

## 7. Cost

| Path | Cost |
| --- | --- |
| a frame | the band only, which is at most `BAND_ROWS` rows |
| a freeze | one `insert_before` per batch, and the batch is the final prefix |
| the write cost of thirty inserts | 3425 bytes, measured, with `scrolling-regions` |

`rho-tui` enables the `ratatui` feature `scrolling-regions`, added with `cargo add`. See
`D-scrolling-regions-is-required` for the measurement.

A frame no longer costs a walk of the transcript, because the band holds the live rows
only. The old renderer walked the whole row list per frame. So this design is cheaper than
what ships today, and `docs/benchmarks.md` gets the new number.

## 8. Test cases

Every public item above appears here. A test lands red first, and step 7 of `AGENTS.md`
proves it fails for the right reason.

### S1, the band and the freeze

| Test | Assertion |
| --- | --- |
| `the_setup_never_enters_the_alternate_screen` | `setup_sequences(false)` holds no `?1049h` |
| `the_restore_matches_the_setup` | it disables no mode the setup skipped |
| `the_band_leaves_a_row_for_the_shell` | `band_rows(10)` is 9, and `band_rows(40)` is `BAND_ROWS` |
| `a_one_row_terminal_gets_one_row` | `band_rows(1)` is 1 |
| `a_user_row_is_final_at_once` | `row_is_final` is true |
| `an_error_row_is_final_at_once` | `row_is_final` is true |
| `a_running_tool_row_is_never_final` | `Pending` and `Running` both answer false |
| `a_finished_tool_row_is_final` | `Ok` and `Failed` both answer true |
| `the_newest_assistant_row_is_not_final_mid_turn` | it answers false while `Running` |
| `an_older_assistant_row_is_final` | a newer assistant row makes it true |
| `every_row_is_final_when_the_turn_ends` | `row_is_final` is true for all |
| `an_unfinished_agent_row_is_never_final` | `finished` false answers false |
| `an_unfinished_task_row_is_never_final` | `finished` false answers false |
| `a_late_tool_event_for_a_frozen_row_changes_nothing` | the row's lines are unchanged, and `late_events` is one |
| `a_late_tool_start_pushes_no_duplicate_row` | the row count is unchanged |
| `a_late_agent_progress_after_finish_changes_nothing` | the frozen row is untouched |
| `a_frozen_line_never_changes_again` | replaying every event kind leaves the frozen lines equal |
| `the_reducer_writes_a_tool_duration` | `row_durations` holds the span after `ToolEnd` |
| `the_reducer_writes_a_thinking_duration` | the same for a thinking block |
| `the_reducer_writes_the_turn_clock` | `turn_millis` is set at `AgentEnd` |
| `a_tool_row_freezes_with_its_duration` | the frozen line holds the duration text |
| `a_final_row_behind_a_live_row_waits` | `next_freeze` returns `None` |
| `the_freeze_takes_the_longest_final_prefix` | `rows` equals the prefix length |
| `nothing_freezes_twice` | two calls with no new row give `None` the second time |
| `a_frozen_row_leaves_the_band` | `live_rows` no longer holds it |
| `the_freeze_sanitises_every_line` | a row holding `ESC [ 2 J` freezes with no escape byte |
| `mark_frozen_saturates` | a count past the end leaves `frozen_rows` at the row count |
| `the_live_window_keeps_the_newest_lines` | the newest line is present, the oldest is not |
| `a_batch_never_ends_with_a_blank_line` | the last line of a batch holds text |
| `the_banner_holds_the_cwd_and_the_model` | both appear in `banner_line` |
| `the_banner_freezes_once` | a second startup step inserts no second banner |
| `exit_mid_run_freezes_every_row_once` | a streaming turn plus `ctrl-d` leaves each row once |
| `exit_freezes_a_running_tool_row_as_running` | the frozen line reads `running` |
| `the_freeze_width_is_the_draw_width` | a resize before the freeze wraps at the new width |
| `a_resize_redraws_the_band_whole` | the composer border spans the new width |
| `the_band_degrades_in_the_stated_order` | a five-row band keeps the composer input row |
| `the_band_value_is_fourteen_rows` | `BAND_ROWS` is 14, which the layout table needs |
| `the_frame_set_is_complete` | every fixture in `docs/design/tui-frames/` has a test |

### S2, the mouse

| Test | Assertion |
| --- | --- |
| `capture_is_off_by_default` | `setup_sequences(false)` holds no capture sequence |
| `with_mouse_enables_capture` | `setup_sequences(true)` holds it |
| `the_config_key_defaults_to_false` | an empty config resolves `tui_mouse` to false |
| `the_env_var_sets_the_mouse_key` | `RHO_TUI_MOUSE=true` resolves to true |
| `with_mouse_reaches_the_setup` | the app built with the flag writes the capture sequence |

### S3, the composer

| Test | Assertion |
| --- | --- |
| `every_paste_test_passes_unchanged` | the existing `paste.rs` suite is green with no edit |
| `typing_inserts_at_the_cursor` | a left key then a character inserts before the last one |
| `backspace_deletes_at_the_cursor` | the unit before the cursor goes, and the cursor follows |
| `a_chip_is_one_unit_for_motion` | one left key steps over a whole paste chip |
| `delete_forward_removes_the_next_unit` | the unit after the cursor goes |
| `delete_forward_at_the_end_does_nothing` | the draft is unchanged |
| `ctrl_j_inserts_a_newline` | `display_lines` returns two rows |
| `alt_enter_inserts_a_newline` | the same, with the alt modifier |
| `enter_sends_the_whole_draft` | the submitted text holds both rows and the held paste |
| `word_motion_crosses_one_word` | `move_word_left` stops at the word start |
| `word_motion_crosses_one_word_forward` | `move_word_right` stops after the word |
| `move_right_stops_at_the_end` | the cursor does not pass the last unit |
| `line_start_and_line_end_bound_one_row` | both land on the row the cursor is on |
| `down_moves_the_cursor_then_gives_up_the_key` | `move_row_down` returns false on the last row |
| `kill_to_line_end_fills_the_kill_buffer` | `yank` restores the cut text |
| `kill_to_line_start_keeps_the_tail` | the text after the cursor stays |
| `kill_word_left_cuts_one_word` | one word goes, and the kill buffer holds it |
| `the_composer_height_is_capped` | twenty draft rows render `COMPOSER_MAX_TEXT_ROWS` |
| `the_cursor_cell_follows_the_wrap` | a wrapped row puts the cursor on the second row |
| `the_draft_grows_and_the_live_area_shrinks` | the band height is unchanged |
| `set_text_replaces_the_held_pastes` | `take` returns the new text alone |
| `an_empty_draft_reports_empty` | `is_empty` and `draft_is_empty` agree |
| `history_recalls_the_previous_prompt` | `↑` on a one-row draft loads the last submit |
| `history_recalls_forward_to_the_live_draft` | `↓` past the newest restores the draft |
| `history_does_not_duplicate_a_repeat` | two identical submits give one entry |
| `up_moves_the_cursor_in_a_tall_draft` | `↑` on row two moves the cursor, not the history |
| `esc_esc_clears_the_draft_into_the_history` | the draft is empty, and `↑` restores it |
| `one_esc_does_not_clear_the_draft` | the draft survives a single `esc` |
| `reverse_search_filters_and_accepts` | `enter` puts the matched entry in the draft |
| `reverse_search_esc_keeps_the_draft` | the draft is the text typed before `ctrl-r` |
| `reverse_search_matches_without_case` | `TEST` matches `test` |
| `the_panel_owns_the_keyboard` | a character typed into the search never reaches the draft |
| `the_editor_command_prefers_visual` | `$VISUAL` wins over `$EDITOR` |
| `the_editor_command_falls_back_to_vi` | with neither set, the answer is `vi` |
| `the_editor_argv_never_reaches_a_shell` | `vi; rm -rf ~` gives the program `vi` and three arguments |
| `a_failed_editor_keeps_the_draft` | the draft is unchanged, and one error row exists |
| `ctrl_x_ctrl_e_returns_edit_draft` | the handler returns `KeyAction::EditDraft` with the draft |
| `ctrl_g_returns_edit_draft` | the same for the second binding |
| `the_units_report_a_chip_as_one_unit` | `units` holds one `Unit::Paste` for a chip |
| `shift_enter_falls_back_to_enter` | with no kitty protocol, the draft sends |
| `the_help_lists_every_new_key` | each key of section 6.3 to 6.6 has a help row |

The last test is the guard for `D-a-panel-nobody-can-open`. The help comes from the binding
table, so a new key with no binding row fails it.

### Driving it for real

A test cannot see the terminal's scrollback, so step 11 covers it. The commands and their
output go in `docs/verification/`. At minimum: a turn with two tool calls, a cancel, a
resize during a turn, and a quit. Then read the scrollback and confirm every row is there,
once each, in order.

## 9. Out of scope

The fold keys `ctrl-o` and `ctrl-e`. They can only work on the live band now, and that
needs its own decision. See `D-a-frozen-row-never-repaints`.

Fuzzy filtering for the slash list, `@` file mentions, and `!` shell mode. They are S4.

Queued messages, the wired approval panel, and the task list. They are S5.

The context percentage, the money figure, `ctrl-l`, and `ctrl-z`. They are S6.

A vim mode, per `D-vim-mode-waits`. A history file on disk. Markdown rendering, syntax
highlighting, and any image protocol. The image chip, because `attach_image` still answers
no key. A second renderer for the alternate screen.
