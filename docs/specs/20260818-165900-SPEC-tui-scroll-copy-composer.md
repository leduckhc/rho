# SPEC-tui-scroll-copy-composer — The transcript scrolls, the user copies, and the composer is real

Status: **superseded on 2026-08-18 by `SPEC-tui-inline-and-composer`.** Read that spec
instead. This one stays because its commit message and this note explain why.

Its premise is false. It assumed rho must keep the alternate screen, so it specified an
in-app scroll, a scroll rail, a keyboard selection, and an OSC 52 copy. A spike proved that
`Viewport::Inline` plus `Terminal::insert_before` give a pinned composer and leave the
transcript in the terminal's own scrollback. See
`docs/verification/inline-viewport-spike.md` and
`D-inline-viewport-not-alternate-screen`. The terminal then scrolls, selects, and searches,
so sections 2 and 4 below describe work rho must not do.

Sections 3, 5, and 6 survive in the new spec: the mouse stays uncaptured, the composer gets
a cursor, and the editor never reaches a shell.

Status: draft. Sprint 4, stages S1 to S3.
Owning crate: `rho-tui`. Consumed by `rho-cli` and `rho-config`.

This spec extends `SPEC-tui` and `SPEC-tui-experience`. It changes no rule from either.
It answers one user report: the transcript cannot scroll, the mouse cannot select, and the
composer in the running product is a `String`.

The scope is the three stages of `D-first-tui-spec-covers-three-stages`. S1 is scrolling and
the follow rule. S2 is copy and the mouse. S3 wires the `Composer` that already exists.

Features covered, before this spec was superseded: F-optional-mouse, F-composer-editing,
F-draft-history, and F-external-editor. Four feature rows died with the premise. They named
an in-app scroll, a scroll rail, a keyboard copy, and a scrollback dump. F-inline-band and
F-freeze-upward replace them, because the terminal now does that work.

## 0. The rules this spec obeys

1. The state is a value, and the reducer is pure. No IO enters `state.rs`.
2. The renderer is pure, and it reads no clock.
3. The key handler returns an action. The event loop is the only place that does IO.
4. Every promise on screen answers a key. See `D-a-panel-nobody-can-open`.
5. One fact lives in one field. See `D-follow-is-derived-not-stored`.

Three defects in this crate came from a public item that no key reached. So every public
item below names its test in section 9.

## 1. What is wrong today

`transcript_window` walks the rows from the newest backwards, then drops the overflow:

```rust
// Drop any overflow from the top, so the newest row stays visible.
if lines.len() > rows {
    lines.drain(0..lines.len() - rows);
}
```

The transcript is not scrolled. It is truncated. `TuiState` holds no scroll position at all.
The app also enters the alternate screen, so the terminal scrollback holds nothing.

`setup_terminal` enables mouse capture for the whole session. With capture on, the terminal
gives drag events to rho, and rho does nothing with them. So the user lost drag-select and
gained one clickable list. `D-native-selection-is-the-default` reverses that trade.

`Composer`, `route_burst`, and `attach_image` are public, tested, and called by nothing.
`state.input` is a `String`.

## 2. The scroll model

### 2.1 One number

```rust
/// Rendered transcript lines between the bottom of the view and the newest line.
///
/// Zero means the view follows the newest line. A value above zero means the user
/// scrolled up, so new output must not move the view. There is no follow flag,
/// because two fields for one fact can disagree. See `D-follow-is-derived-not-stored`.
pub scroll_rows: usize,

/// The transcript window height, measured at the last clamp. The key handler reads it,
/// so a page key scrolls a half screen without asking the renderer.
pub view_rows: usize,

/// The rendered transcript line count, measured at the last clamp. The clamp compares
/// it with the current count, so growth below a scrolled view does not move the view.
pub content_rows: usize,

/// The frame width at the last clamp. A line count is width-dependent, because text
/// wraps. So the clamp compares widths before it trusts a change in the count.
pub content_width: u16,
```

```rust
impl TuiState {
    /// True when the view follows the newest line.
    pub fn follows(&self) -> bool;

    /// Move the view up by `lines`. The next clamp bounds it at the oldest line.
    pub fn scroll_up(&mut self, lines: usize);

    /// Move the view down by `lines`. It stops at the newest line, which follows again.
    pub fn scroll_down(&mut self, lines: usize);

    /// Jump to the newest line, and follow again.
    pub fn scroll_to_latest(&mut self);

    /// Jump to the oldest line. The next clamp resolves the exact position.
    pub fn scroll_to_oldest(&mut self);
}
```

`scroll_up` and `scroll_to_oldest` may name a line that does not exist yet, because the
handler knows no width. The clamp in section 2.2 fixes that before the frame draws.

### 2.2 The clamp, and why growth does not move the view

```rust
/// Bound the scroll position to the content this frame size can show.
///
/// The event loop calls this before every draw, and after a resize. It asks the same
/// `plan_layout` the renderer asks, so the geometry has one source of truth. It does
/// no IO and it reads no clock.
///
/// It also compensates for growth. While the view is scrolled, every new line below it
/// increases `scroll_rows` by one, so the user keeps reading the same lines.
pub fn clamp_scroll(state: &mut TuiState, width: u16, height: u16);
```

Order of work inside the clamp:

1. Measure `window` from `plan_layout`, and `content` from `transcript_line_count`.
2. When `scroll_rows > 0` and `width == state.content_width`, move the view by the change
   in the count. Growth adds `content - state.content_rows`. A shrink subtracts it. Both
   saturate at zero.
3. When `scroll_rows > 0` and the width changed, scale the position instead:
   `scroll_rows = scroll_rows * content / state.content_rows`. A rewrap changes every
   line, so no exact anchor survives it. A bound is what the test asserts.
4. Move the selection of section 4.1 by the same amount that step 2 or step 3 moved the
   view. A selection that stayed put would name other lines after one new row.
5. Store `view_rows = window`, `content_rows = content`, and `content_width = width`.
6. Clamp `scroll_rows` to `content - window`, or to zero when the content fits. Clamp the
   selection to the content too.

Step 2 is the rule the user feels. Without it, five new lines push the view five lines
down, and the paragraph the user is reading walks off the screen.

Step 3 exists because a line count answers a different question at each width. An earlier
draft of this spec compared the counts alone. A resize then read the rewrap as new output
and shoved the view. A widen compensated by nothing at all. So the width guard is not a
detail. It is the difference between a rule and a drift.

Step 4 is the same rule for the selection. A review found it stated in section 4.1 and
missing from this list, and an implementer follows this list.

### 2.3 The window, and the cost

```rust
/// The rendered transcript line count for this state at this width.
pub fn transcript_line_count(state: &TuiState, width: u16) -> usize;

/// The visible transcript text, oldest line first, one entry per drawn row.
///
/// This is what the renderer draws, with no style. A test asserts scrolling with it,
/// and the copy path in section 4 selects from it.
pub fn visible_transcript(state: &TuiState, width: u16, height: u16) -> Vec<String>;
```

The walk still starts at the newest row. It now skips `scroll_rows` lines before it fills
the window, so one frame costs `O(window + scroll_rows)`. A frame at the bottom costs what
it costs today. A frame at the top of a long transcript costs one walk of the transcript,
and only while the user stays there. `transcript_line_count` is the one function that
always walks every row, so it counts lines and builds no string.

### 2.4 The rail

```rust
/// One cell of the scroll rail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailCell {
    /// The unfilled track, drawn in the `muted` role with `┃`.
    Track,
    /// The thumb, drawn in the `text` role with `█`.
    Thumb,
}

/// The rail column for one window, top row first.
///
/// `None` when the content fits the window, because a transcript that fits needs no
/// rail. See `D-a-rail-only-when-it-overflows`. The thumb covers at least one row. It
/// touches the bottom row when `scroll_rows` is zero, and the top row at the oldest line.
pub fn scroll_rail(
    content_rows: usize,
    window_rows: usize,
    scroll_rows: usize,
) -> Option<Vec<RailCell>>;
```

`Track` is not decoration. It states how much of the transcript the window covers. So a
test asserts a track cell above the thumb and below it.

The rail draws in the last column of each transcript row. It replaces no text, because the
transcript measure already leaves ten columns clear. See `TEXT_MEASURE_MARGIN`.

### 2.5 The follow banner

```rust
/// The banner that replaces the header rule while the view is scrolled.
///
/// `hidden_rows` counts rendered lines below the view. `None` while the view follows, so
/// the resting frame keeps the plain rule. The text names the key that returns.
pub fn follow_banner(hidden_rows: usize, width: usize) -> Option<String>;
```

The shape is `──── 42 new lines below · end jumps to the latest ────…`, padded with `─` to
the full width. One hidden line reads `1 new line below`.

### 2.6 The scroll keys

| Key | Action |
| --- | --- |
| `⇞` `PageUp` | up a half window, which is `view_rows / 2`, and at least one line |
| `⇟` `PageDown` | down a half window |
| `ctrl-home` | the oldest line |
| `ctrl-end` | the newest line, and follow again |
| `end` while scrolled | the newest line, and follow again |
| `end` while following | the cursor to the end of the draft row |
| wheel up, wheel down | three lines a notch, and only when the mouse is ours |

`end` carries two meanings, and the scroll position decides which. A scrolled view is a
mode, and the banner states the key. So the meaning is on screen when it applies.

`end` while a selection lives jumps to the newest line and drops the selection. A selection
the user cannot see is worse than none. A printable key does the same, and then types. So
no key traps the user inside a selection.

```rust
/// Lines one wheel notch scrolls.
pub const WHEEL_LINES: usize = 3;
```

## 3. The mouse

Mouse capture is off by default. `setup_terminal` enters the alternate screen alone. See
`D-native-selection-is-the-default`.

The sequences become data, so a test can read them. `setup_terminal` writes what these
functions return, and nothing else.

```rust
/// The escape sequences that start the interface. `mouse` adds mouse capture.
pub fn setup_sequences(mouse: bool) -> String;

/// The escape sequences that give the terminal back. It disables capture only when the
/// setup enabled it, so rho never turns off a mode it did not turn on.
pub fn restore_sequences(mouse: bool) -> String;
```

```rust
impl App {
    /// Turn mouse capture on. Off by default, so the terminal keeps drag-select and
    /// its own wheel. On, rho gets the wheel and the clickable slash list.
    pub fn with_mouse(self, enabled: bool) -> Self;
}
```

The config carries the value as one flat key, because `ConfigLayer` denies an unknown key
and holds no nested table for the interface.

```rust
// rho-config: one new optional field on ConfigLayer, and one on Config.
pub tui_mouse: Option<bool>,   // file key `tui-mouse`, env `RHO_TUI_MOUSE`
pub tui_mouse: bool,           // resolved, false by default
```

`rho-cli` passes it: `rho_tui::App::new(session, model).with_mouse(config.tui_mouse)`.

## 4. Copy, and the ways out

### 4.1 The selection

```rust
/// A transcript selection, counted in rendered lines above the newest line.
///
/// `anchor` is the line where `v` started. `head` moves with the arrows. The clamp
/// shifts both by the growth below them, so a selection keeps its text while a turn runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

/// The live selection, or `None` in every other mode.
pub selection: Option<Selection>,
```

```rust
impl TuiState {
    /// Start a one-line selection at the newest visible line.
    pub fn start_selection(&mut self);

    /// Extend the selection up by one line. It stops at the oldest line.
    pub fn extend_selection_up(&mut self);

    /// Extend the selection down by one line. It stops at the newest line.
    pub fn extend_selection_down(&mut self);

    /// Drop the selection and keep the view where it is.
    pub fn cancel_selection(&mut self);

    /// The count of selected lines. Zero with no selection.
    pub fn selected_rows(&self) -> usize;
}

/// The selected text, one line per row, joined with a newline and no trailing newline.
pub fn selected_text(state: &TuiState, width: u16, height: u16) -> String;
```

The keys are `v` to start, `↑` and `↓` to extend, `y` to copy, `o` to open, and `esc` to
cancel. While a selection lives, the footer reads `select · 2 rows` on the left, and
`↑ ↓ extend · y copy · o open in $EDITOR · esc cancel` on the right.

### 4.2 The clipboard

```rust
/// The OSC 52 sequence that puts `text` on the terminal clipboard.
///
/// The sequence travels with the terminal stream, so it works over ssh and inside tmux.
/// rho links no clipboard crate and starts no process. See `D-copy-goes-through-osc-52`.
pub fn osc52_clipboard(text: &str) -> String;
```

The shape is `ESC ] 52 ; c ; <base64 of text> BEL`. The base64 encoder is private to the
crate, and section 9 pins it against the RFC 4648 vectors.

### 4.3 The dump

```rust
/// Every transcript line at this width, oldest first, for the scrollback dump.
///
/// Each line passes through `sanitize_line`, exactly as the renderer does. A dumped line
/// therefore carries no escape sequence. This matters more here than on screen: the dump
/// prints outside the alternate screen, so an escape sequence would reach the user's
/// shell, and the text is model-authored.
pub fn transcript_dump(state: &TuiState, width: u16) -> Vec<String>;

/// The exact bytes the dump writes, in order: leave the alternate screen, every line, then
/// enter it again. A test asserts the order, because a dump inside the alternate screen
/// writes to a buffer the user never sees.
pub fn dump_script(lines: &[String]) -> String;
```

> **The dump is not built. See `D-alternate-screen-after-all`.** The paragraph below states
> the design as it stood. `ctrl-p` and `dump_script` exist in no code. The dump gave back the
> terminal's search, and the terminal's own search reaches the alternate screen in iTerm2 and
> in Ghostty, so the need does not exist. A spike proved the dump works, and that record
> stays in `docs/verification/alt-screen-spike.md` for a future reader.

`ctrl-p` returns `KeyAction::DumpTranscript`. The event loop writes `dump_script`. It enters
the alternate screen again even when the write fails, because a terminal left outside it
loses the interface, and a terminal left inside it loses the user's shell.

The other two copy paths are safe by construction, and the spec states why. A copy carries
base64 text inside one sequence, so the payload cannot end the sequence. An editor view
writes a temporary file, so no transcript byte reaches the terminal at all.

### 4.4 The editor

```rust
/// The editor command, from `$VISUAL`, then `$EDITOR`, then `vi`.
///
/// The caller passes the values, so no test reads the real environment.
pub fn editor_command(visual: Option<&str>, editor: Option<&str>) -> String;

/// The program and its arguments, split from an editor value on whitespace.
///
/// The first token is the program. The rest are arguments. The loop appends the temporary
/// file path as the last argument, and it runs the program with `std::process::Command`.
/// It never passes the value to a shell, and it never uses `sh -c`. So a value such as
/// `vi; rm -rf ~` gives `vi` three inert arguments, and the second command never runs.
pub fn editor_argv(command: &str) -> Vec<String>;
```

`o` returns `KeyAction::ViewInEditor(String)` with the selected text. `ctrl-x ctrl-e` and
`ctrl-g` return `KeyAction::EditDraft(String)` with the draft. The loop writes a temporary
file, runs the editor, and reads the file back. On `EditDraft` it replaces the draft with
what came back. On `ViewInEditor` it changes no state, because a view is read-only.

A failed editor run pushes one error row. It never discards the draft.

### 4.5 The new actions

```rust
pub enum KeyAction {
    None,
    Submit(String),
    Cancel,
    Exit,
    /// Put this text on the terminal clipboard, with OSC 52.
    Copy(String),
    /// Show this text in the editor. Read-only, so the draft is untouched.
    ViewInEditor(String),
    /// Edit this text in the editor, then replace the draft with the result.
    EditDraft(String),
    /// Write every transcript line to the terminal scrollback.
    DumpTranscript,
}
```

Every new variant does IO in the loop, and none does IO in the handler.

## 5. The composer

### 5.1 The draft replaces the string

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

Every test that reads `state.input` reads `draft_text()` instead. That is a rename, and no
assertion changes.

Four places **write** the field, and a write is not a rename. `crates/rho-tui/tests/frames.rs`
sets `state.input` at lines 329, 478, 534, and 562. Each becomes
`state.draft.set_text(…)`. Line 478 writes `"/"` to set up the slash panel by hand. It must
reach the panel state through the key handler instead, because a fixture that builds a state
no key can build proves nothing. That is the defect class of `D-a-panel-nobody-can-open`.

A test whose assertion changes is a defect report, not a rename. Stop and say so.

### 5.2 The cursor moves over units

```rust
/// One unit of the draft, as the cursor moves over it. A chip is one unit, so one
/// left key steps over a whole paste.
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

    /// Move the cursor one display row. It returns `false` when no such row exists, so
    /// the caller can give the key to the history instead.
    pub fn move_row_up(&mut self, width: usize) -> bool;
    pub fn move_row_down(&mut self, width: usize) -> bool;

    /// Insert a newline at the cursor.
    pub fn insert_newline(&mut self);

    /// Delete the unit after the cursor. A chip deletes whole.
    pub fn delete_forward(&mut self);

    /// Cut to the end of the display row, into the kill buffer.
    pub fn kill_to_line_end(&mut self);
    /// Cut to the start of the display row, into the kill buffer.
    pub fn kill_to_line_start(&mut self);
    /// Cut the word before the cursor, into the kill buffer.
    pub fn kill_word_left(&mut self);
    /// Paste the kill buffer at the cursor.
    pub fn yank(&mut self);

    /// The display rows at this width, wrapped, and capped by `COMPOSER_MAX_TEXT_ROWS`.
    pub fn display_lines(&self, width: usize) -> Vec<String>;

    /// The cursor cell as a row and a column into `display_lines`.
    pub fn cursor_cell(&self, width: usize) -> (usize, usize);

    /// True when the draft holds no unit.
    pub fn is_empty(&self) -> bool;

    /// Replace the whole draft, and put the cursor at the end. Used by the history and
    /// by the editor. It drops every held paste, because it replaces the whole draft, so
    /// `take` then returns exactly this text. It keeps the kill buffer.
    pub fn set_text(&mut self, text: &str);

    /// Take the model text and empty the draft. The cursor returns to zero.
    pub fn take(&mut self) -> String;
}
```

These items keep their names, their signatures, and their behaviour. They stay public,
because `crates/rho-tui/tests/paste.rs` calls every one of them.

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

`insert` and `backspace` now act at the cursor. `backspace` also moves the cursor one unit
left, because the unit it deleted was there. `delete_forward` leaves the cursor where it is,
and it does nothing at the end of the draft. A fresh draft holds the cursor at the end, so
every existing paste test passes unchanged. That is the acceptance rule for this refactor.

### 5.3 The newline keys

| Key | Meaning |
| --- | --- |
| `enter` | send the draft |
| `ctrl-j` | insert a newline |
| `alt+enter` | insert a newline |
| `shift+enter` | insert a newline, where the terminal reports the modifier |

`shift+enter` needs the kitty keyboard protocol. A terminal without it reports plain
`enter`, and the help says so. rho requests no protocol upgrade in this spec.

### 5.4 The history

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
history, so the text is recoverable with `↑`. A single `esc` arms that, and any other key
disarms it. This mirrors the Ctrl-C gate, which the footer already explains.

### 5.5 Reverse search

```rust
/// The reverse-search panel state.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct HistorySearch {
    /// The typed query.
    pub query: String,
    /// The selected row of the filtered list.
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
///
/// The match is a case-insensitive substring. An empty query matches every entry.
pub fn filter_history(history: &[String], query: &str) -> Vec<usize>;
```

`ctrl-r` opens it. Typing filters. The arrows move. `enter` puts the entry in the draft and
closes. `esc` closes and keeps the draft. The panel owns the keyboard while it is open.

## 6. The frame

The layout table of `SPEC-tui-experience` section 1 stands. Three rows change their content.

| Region | Change |
| --- | --- |
| Header rule | the follow banner replaces it while `scroll_rows > 0` |
| Transcript | the last column carries the rail when the content overflows |
| Composer | it draws `display_lines`, and the cursor sits at `cursor_cell` |
| Footer | it carries the selection hints while a selection lives |

The degradation order is unchanged. The rail is part of a transcript row, so it survives
wherever the transcript survives. The banner is part of the rule, so it drops with the rule.

## 7. What this spec does not change

The reducer stays pure. The renderer stays pure. `plan_layout` stays the one source of
geometry, and `clamp_scroll` and `slash_row_index` both ask it.

The alternate screen stays, per `D-keep-the-alternate-screen`.

The context percentage and the money rule of `D-context-in-the-footer` belong to a later
stage. This spec keeps the footer content it has today, plus the selection hints.

## 8. Cost

The frame budget of `docs/benchmarks.md` holds. Two new costs enter, and both are bounded.

| Path | Cost |
| --- | --- |
| a frame at the newest line | as today |
| a frame scrolled up by `n` lines | one walk of `window + n` lines |
| `clamp_scroll` | one line count over every row, with no string built |
| `transcript_dump` | one walk of every row, on one key press |

`clamp_scroll` runs once per frame, so the line count must allocate nothing. A benchmark
records the frame cost at the bottom and at the top of a ten thousand row transcript.

## 9. Test cases

Every public item above appears here. A test lands red first, and step 7 of `AGENTS.md`
proves it fails for the right reason.

### S1, scrolling

| Test | Assertion |
| --- | --- |
| `a_fresh_state_follows` | `follows()` is true, and `scroll_rows` is zero |
| `scroll_up_stops_auto_follow` | after `scroll_up(1)`, `follows()` is false |
| `new_output_does_not_move_a_scrolled_view` | `visible_transcript` is unchanged after ten rows arrive and the clamp runs |
| `new_output_moves_a_following_view` | the newest row is visible after it arrives |
| `end_returns_to_the_latest_and_follows` | `end` sets `scroll_rows` to zero |
| `end_moves_the_cursor_when_not_scrolled` | `end` moves the cursor and leaves the view |
| `ctrl_home_reaches_the_oldest_line` | the first line of the first row is visible |
| `scroll_clamps_at_both_ends` | `scroll_up(usize::MAX)` then a clamp gives `content - window` |
| `scroll_down_past_the_end_follows_again` | `scroll_down(usize::MAX)` gives zero |
| `a_page_key_moves_a_half_window` | `⇞` moves `view_rows / 2` lines |
| `a_page_key_moves_one_line_in_a_tiny_window` | a two-row window still moves one line |
| `the_wheel_moves_three_lines` | one notch moves `WHEEL_LINES` |
| `the_rail_shows_the_window_position` | the thumb sits at the bottom while following |
| `the_rail_draws_a_track_around_the_thumb` | a track cell exists above the thumb and below it |
| `a_short_transcript_draws_no_rail` | `scroll_rail` returns `None` |
| `the_rail_thumb_is_never_empty` | a ten thousand line transcript still draws one thumb cell |
| `the_banner_counts_the_hidden_lines` | the text holds `42 new lines below` |
| `the_banner_is_singular_for_one_line` | the text holds `1 new line below` |
| `a_following_view_draws_no_banner` | `follow_banner` returns `None` |
| `a_resize_reclamps_the_view` | a shorter frame leaves `scroll_rows` inside the content |
| `a_shrink_below_a_scrolled_view_keeps_the_lines` | a removed line moves the view back by one |
| `a_rewrap_keeps_the_view_within_one_window` | after a width change, the drift is under `view_rows` |
| `scroll_to_latest_follows_again` | `follows()` is true, from any position |
| `scroll_to_oldest_reaches_the_first_line` | the first line is visible after one clamp |
| `the_line_count_matches_the_window_walk` | both agree on a fixture transcript |

### S2, copy and the mouse

| Test | Assertion |
| --- | --- |
| `capture_is_off_by_default` | `setup_sequences(false)` holds no capture sequence |
| `with_mouse_enables_capture` | `setup_sequences(true)` holds the capture sequence |
| `the_restore_matches_the_setup` | `restore_sequences(false)` disables no mode the setup skipped |
| `the_config_key_defaults_to_false` | an empty config resolves `tui_mouse` to false |
| `the_env_var_sets_the_mouse_key` | `RHO_TUI_MOUSE=true` resolves to true |
| `v_starts_a_one_row_selection` | `selected_rows()` is one |
| `a_selection_extends_and_stops_at_the_top` | the head stops at the oldest line |
| `a_selection_extends_down_and_stops_at_the_bottom` | the head stops at the newest line |
| `a_selection_copies_the_rows_it_covers` | `selected_text` holds those lines only |
| `a_copy_returns_the_text_and_does_no_io` | the handler returns `KeyAction::Copy` |
| `o_returns_the_selection_and_changes_no_state` | the state before equals the state after |
| `a_selection_survives_new_output` | the text is unchanged after ten rows arrive |
| `end_drops_a_selection_and_follows` | `selection` is `None`, and `follows()` is true |
| `a_printable_key_drops_a_selection_and_types` | the draft holds the character |
| `esc_cancels_a_selection_and_keeps_the_view` | `selection` is `None`, `scroll_rows` is unchanged |
| `osc52_carries_the_base64_of_the_text` | the sequence matches the expected bytes |
| `base64_matches_the_rfc_vectors` | the six RFC 4648 vectors pass |
| `the_dump_writes_every_row_once` | the dump line count equals the full line count |
| `the_dump_sanitises_every_line` | a row holding `ESC [ 2 J` dumps with no escape byte |
| `the_dump_leaves_the_alternate_screen_first` | `dump_script` starts with the leave sequence and ends with the enter sequence |
| `the_editor_command_prefers_visual` | `$VISUAL` wins over `$EDITOR` |
| `the_editor_command_falls_back_to_vi` | with neither set, the answer is `vi` |
| `the_editor_argv_never_reaches_a_shell` | `vi; rm -rf ~` gives the program `vi` and three arguments |
| `a_failed_editor_keeps_the_draft` | the draft text is unchanged, and one error row exists |

### S3, the composer

| Test | Assertion |
| --- | --- |
| `every_paste_test_passes_unchanged` | the existing `paste.rs` suite is green with no edit |
| `typing_inserts_at_the_cursor` | a left key then a character inserts before the last one |
| `backspace_deletes_at_the_cursor` | the unit before the cursor goes, not the last one |
| `a_chip_is_one_unit_for_motion` | one left key steps over a whole paste chip |
| `delete_forward_removes_the_next_unit` | the unit after the cursor goes |
| `ctrl_j_inserts_a_newline` | `display_lines` returns two rows |
| `alt_enter_inserts_a_newline` | the same, with the alt modifier |
| `enter_sends_the_whole_draft` | the submitted text holds both rows and the held paste |
| `word_motion_crosses_one_word` | `move_word_left` stops at the word start |
| `word_motion_crosses_one_word_forward` | `move_word_right` stops after the word |
| `move_right_stops_at_the_end` | the cursor does not pass the last unit |
| `line_start_and_line_end_bound_one_row` | both land on the row the cursor is on |
| `down_moves_the_cursor_then_gives_up_the_key` | `move_row_down` returns false on the last row |
| `set_text_replaces_the_held_pastes` | `take` returns the new text alone |
| `an_empty_draft_reports_empty` | `is_empty` and `draft_is_empty` agree |
| `kill_to_line_end_fills_the_kill_buffer` | `yank` restores the cut text |
| `kill_to_line_start_keeps_the_tail` | the text after the cursor stays |
| `kill_word_left_cuts_one_word` | one word goes, and the kill buffer holds it |
| `the_composer_height_is_capped` | twenty rows of draft render `COMPOSER_MAX_TEXT_ROWS` |
| `the_cursor_cell_follows_the_wrap` | a wrapped row puts the cursor on the second row |
| `the_draft_grows_the_box_and_shrinks_the_transcript` | the frame keeps its height |
| `history_recalls_the_previous_prompt` | `↑` on a one-row draft loads the last submit |
| `history_recalls_forward_to_the_live_draft` | `↓` past the newest entry restores the draft |
| `history_does_not_duplicate_a_repeat` | two identical submits give one entry |
| `up_moves_the_cursor_in_a_tall_draft` | `↑` on row two moves the cursor, not the history |
| `esc_esc_clears_the_draft_into_the_history` | the draft is empty, and `↑` restores it |
| `one_esc_does_not_clear_the_draft` | the draft survives a single `esc` |
| `reverse_search_filters_and_accepts` | `enter` puts the matched entry in the draft |
| `reverse_search_esc_keeps_the_draft` | the draft is the text typed before `ctrl-r` |
| `reverse_search_matches_without_case` | `TEST` matches `test` |
| `the_panel_owns_the_keyboard` | a character typed into the search never reaches the draft |
| `the_help_lists_every_new_key` | each key of section 2.6 and 5.3 has a help row |

The last test is the guard for `D-a-panel-nobody-can-open`. The help is generated from the
binding table, so a new key with no binding row fails it.

## 10. Out of scope

Fuzzy filtering for the slash list, `@` file mentions, and `!` shell mode. They are S4.

Queued messages, the fold keys, the wired approval gate, and the task list. They are S5.

The context percentage, the money figure, `ctrl-l`, and `ctrl-z`. They are S6.

A vim mode, per `D-vim-mode-waits`. A history file on disk. A search of the transcript
itself. Markdown rendering, syntax highlighting, and any image protocol. A second renderer
that writes to the scrollback instead of the alternate screen. A mouse-resizable split.

The image chip. `attach_image` stays reachable by no key, so this spec adds no `Unit::Image`
and no image test. An unreachable variant is the defect this project keeps paying for. The
key and the path source belong to the stage that wires them, and `docs/features.md` now
reports F-attachments as `partial` for that reason.
