# SPEC-select-text-and-find-the-bottom — an owned selection, and a way back to the newest row

Status: draft. No code yet. This spec is the contract for review before any side starts.
Owning crate: `rho-tui`
Features: F-transcript-selection, F-scroll-to-bottom
Decisions this spec implements: D-selection-is-logical-not-display,
D-copy-uses-osc52-fire-and-forget, D-app-selection-is-primary-native-is-fallback,
D-select-mode-keys

## 1. The problem

The interface runs in the alternate screen. It captures the mouse by default. See
`crates/rho-tui/src/app.rs:75`. So the terminal's own click-drag-copy does not work. A
user cannot select transcript text with the mouse today. A user without a mouse has no
way to copy at all. A user who scrolls far up also has no fast way back to the newest row.

The user picked the full-depth option. rho owns the selection. rho holds its own copy of
the selected text. rho does not merely release the mouse to the terminal.

## 2. The sides, and who owns each

- The **data model**: `Selection` and `TextPos` in `rho-tui`. Stored in `TuiState`.
- The **reducer**: `TuiState::apply` in `rho-tui`. It never touches the selection.
- The **renderer**: `render(state, frame)` in `rho-tui`. It paints the selection and the
  affordance from state alone. It stays a pure function of state.
- The **frontend loop**: `App` in `rho-tui`. It maps mouse and key events to selection
  calls, and it emits the clipboard write.

Contract kinds this change touches: the data model, the error taxonomy, the behaviour
rules, the configuration (`tui-mouse`), and the key bindings.

## 3. Why a selection lives in the state

The renderer is a pure function of state. The frame tests assert exact frames, in
`crates/rho-tui/tests/frames.rs`. So the selection must live in the state as data. If it
did not, the renderer could not draw it, and no test could assert it.

## 4. The units, and why

A selection endpoint is a **byte offset into a row's sanitized logical text**, on a
**grapheme-cluster boundary**. It is not a screen cell. It is not a character.

- A screen cell is wrong. The transcript is wrapped for the terminal width. A wide CJK
  glyph takes two cells. A resize re-wraps the text. A cell offset would move under the
  same text after a resize.
- A raw character or a raw byte is wrong at an edge. A ZWJ emoji family is one grapheme
  but many bytes and many chars. A cut inside it would corrupt the copy.
- A byte offset is a valid Rust slice index. It is cheap and stable. rho constrains it to
  a grapheme boundary, so a cut never splits a glyph.

The transcript is append-only. rho pushes a row and never removes or reorders one. See
`crates/rho-tui/src/state.rs`, where `self.rows.push` is the only structural write. So a
logical row index is stable for the whole session. The selection uses that index.

## 5. The data model, in `rho-tui`

```rust
/// A position in the transcript, in logical units. Owned by `rho-tui`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPos {
    /// The logical row index. The transcript is append-only, so this stays valid.
    pub row: usize,
    /// A byte offset into that row's sanitized text. It lands on a grapheme boundary.
    pub byte: usize,
}

/// A live text selection. Owned by `rho-tui`, stored in `TuiState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    /// Where the selection began. It is fixed until the selection ends.
    pub anchor: TextPos,
    /// Where the selection reaches now. It moves as the user drags or extends.
    pub cursor: TextPos,
}

impl Selection {
    /// The ordered pair, low then high, whatever the drag direction.
    pub fn ends(&self) -> (TextPos, TextPos) {
        if self.anchor <= self.cursor {
            (self.anchor, self.cursor)
        } else {
            (self.cursor, self.anchor)
        }
    }

    /// True when the selection covers no text.
    pub fn is_empty(&self) -> bool {
        self.anchor == self.cursor
    }
}
```

`TuiState` gains one field and four methods, in `rho-tui`:

```rust
impl TuiState {
    /// The current selection, or `None` when nothing is selected.
    pub fn selection(&self) -> Option<Selection>;

    /// Begin a selection. The anchor and the cursor start at the same position.
    pub fn selection_begin(&mut self, at: TextPos);

    /// Move the cursor. The anchor does not move. A no-op when no selection exists.
    pub fn selection_extend_to(&mut self, at: TextPos);

    /// Drop the selection.
    pub fn selection_clear(&mut self);
}
```

The reducer never calls these. A streaming delta mutates a row in place and never touches
the selection. rho clamps an endpoint on read, so a shrunk row cannot hold a stale offset.
See section 9.

## 6. The mapping seam

The mouse gives a cell. The selection needs a `TextPos`. rho maps one to the other with a
pure function in `rho-tui`:

```rust
/// Map a screen cell to a transcript position. Return `None` when the cell holds no
/// transcript text, for example the composer or an empty row. Owned by `rho-tui`.
pub fn cell_to_pos(
    state: &TuiState,
    width: u16,
    height: u16,
    cell_row: u16,
    cell_col: u16,
) -> Option<TextPos>;
```

`cell_to_pos` wraps text with the same helper the renderer uses. It reads
`TuiState::scroll_first_visible`, which is public but has no caller today. See
`bench/check-dead-surface.py`. So this spec gives that dead function its first caller.

## 7. The gestures

Every gesture works only while no panel is open. A mouse gesture works only while rho
captures the mouse. See section 10.

### Mouse

- `Down(Left)` on a transcript cell begins a selection there. A click on a slash row keeps
  its current meaning. See `crates/rho-tui/src/app.rs:291`.
- `Drag(Left)` extends the cursor to the pointer cell.
- `Up(Left)` ends the drag and copies. See section 8.
- A `Down(Left)` that starts and releases on the same cell clears the selection.

### Keyboard

A user without a mouse needs a keyboard path. rho adds a select mode. It works only while
the draft is empty, like the scroll keys. See `D-scroll-keys-yield-to-an-empty-draft`.

- `alt-v` begins a selection at the newest visible cell. The mnemonic is "visual".
- The arrows, `pageup`, `pagedown`, `home`, and `end` move the cursor and extend the
  selection. The view follows the cursor.
- `y` copies and leaves select mode.
- `esc` cancels the selection and leaves select mode.

`alt-v` does not collide with any binding in `crates/rho-tui/src/bindings.rs`. `y` is safe,
because select mode captures the key before the draft sees it. `esc` already closes a
panel, so select mode is one more panel it closes.

## 8. Autoscroll while selecting

A drag past the top edge scrolls up. A drag past the bottom edge scrolls down. rho scrolls
`WHEEL_ROWS` rows for each drag event it receives. See `crates/rho-tui/src/scroll.rs`.

The rate is bounded by two facts. rho scrolls at most one row per drag event. The terminal
sends one drag event per pointer move, so a still pointer scrolls nothing. `Scroll::up` and
`Scroll::down` clamp the offset at each end. So the scroll cannot run away.

## 9. Re-wrap and streaming

A resize re-wraps the transcript. The selection endpoints are logical, so they do not move.
The renderer paints the same text at new cells. The selected text is unchanged.

A streaming delta lengthens or replaces a row while the selection exists. rho does not
mutate the selection when this happens. rho clamps each endpoint to the row's current byte
length on read. rho re-reads the covered text at copy time. So the copy reflects the row's
content at the moment of the copy.

## 10. The copy path

rho copies with OSC 52. The sequence is `\x1b]52;c;<base64>\x07`. The base64 payload is the
selected text. rho adds `base64` to `rho-tui` with `cargo add base64`.

```rust
/// The largest selection rho copies, in bytes.
pub const COPY_BYTE_CAP: usize = 100 * 1024;

/// Why a copy did not happen. Owned by `rho-tui`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CopyError {
    /// No selection existed when the copy was asked for.
    NothingSelected,
    /// The selection held more bytes than `COPY_BYTE_CAP`. The field is that cap.
    TooLarge { limit: usize },
}

/// Build the OSC 52 clipboard write, or say why it cannot. Owned by `rho-tui`.
pub fn clipboard_write(selected: Option<&str>) -> Result<String, CopyError> {
    let text = selected.ok_or(CopyError::NothingSelected)?;
    if text.len() > COPY_BYTE_CAP {
        return Err(CopyError::TooLarge {
            limit: COPY_BYTE_CAP,
        });
    }
    Ok(osc52_encode(text))
}
```

`osc52_encode` is a private helper in `rho-tui`. It base64-encodes the bytes and wraps them
in the OSC 52 frame.

**What rho sanitises.** The text comes from each row's already-sanitised logical content.
See `crates/rho-tui/src/sanitize.rs`. It holds no escape sequence. So the OSC 52 payload
holds no escape sequence, and a malicious tool output cannot inject a control code into the
clipboard write.

**What the user sees.** On success rho shows a notice: `sent NN bytes to the clipboard`. The
word is "sent", not "copied", because OSC 52 gives no acknowledgement. rho cannot confirm
the write. On `NothingSelected` rho does nothing visible. On `TooLarge` rho shows an error:
`selection too large to copy · limit 100 KB`.

**A terminal that refuses OSC 52.** Some terminals disable or restrict it. rho cannot detect
this, because the sequence returns no reply. The `/help` copy documents the limit. rho keeps
the external-editor path as a fallback, so a user can open the selection in `$EDITOR` and
copy from there. See F-external-editor.

## 11. The scroll-to-bottom affordance

```rust
/// The hidden-below count at which rho shows the jump-to-newest affordance.
pub const JUMP_HINT_ROWS: usize = 1;
```

**When it appears.** rho shows the affordance while `TuiState::is_scroll_pinned` returns
false and the hidden-below count is at least `JUMP_HINT_ROWS`. `is_scroll_pinned` is public
with no caller today. See `bench/check-dead-surface.py`. So this spec gives it its first
caller. "Scrolled too much" is that number, not a feeling. One hidden row below already
hides the newest output.

**What it shows.** A one-line indicator reads `↓ NN new · end`. `NN` is the hidden-below
count. It comes from `TuiState::scroll_hidden().1`, the existing rail source. See
`crates/rho-tui/src/scroll.rs`, `Scroll::hidden`.

**Where it sits.** rho draws it on the bottom transcript row, right-aligned, above the
composer. It hides nothing, because it replaces trailing blank cells on a row the reader has
already scrolled past.

**How it is activated.** The `end` key jumps to the newest row. It already does. A left
click on the indicator cell calls `TuiState::scroll_down` to the newest row. rho maps the
click by the indicator's known cells.

## 12. Mouse capture and `tui-mouse`

rho keeps mouse capture on by default. The app-managed selection is the primary way to
select. rho does not rely on a native modifier bypass. Most terminals still give native
selection when the user holds a modifier, for example Shift on xterm or Option on iTerm2.
The `/help` copy names this as a fallback. rho does not depend on it.

`tui-mouse` stays the config key. `--mouse` and `--no-mouse` stay the flags. When capture is
on, rho owns the wheel, the clickable lists, the app selection, and the affordance click.
When `--no-mouse` releases capture, the terminal owns selection and scrollback natively. rho
then draws no selection and answers no affordance click. The `end` key still works, because
it is a keyboard path.

## 13. What the contract forbids

- `CopyError::NothingSelected` names a copy with no selection.
- `CopyError::TooLarge` names a selection over `COPY_BYTE_CAP`.
- A `TextPos::byte` off a grapheme boundary is forbidden. `selection_begin` and
  `selection_extend_to` snap the offset to the nearest boundary.
- A `TextPos::row` past the last row is forbidden. rho clamps it on read.
- An autoscroll faster than one row per drag event is forbidden.
- An OSC 52 payload with a raw escape is forbidden. The sanitised source prevents it.

## Test cases

- `selection_begin_sets_anchor_and_cursor_together` proves a new selection is empty.
- `ends_orders_low_to_high_whatever_the_drag_direction` proves `Selection::ends` sorts a
  backward drag.
- `extend_to_moves_the_cursor_and_keeps_the_anchor` proves the anchor is fixed.
- `snap_forbids_a_byte_off_a_grapheme_boundary` proves a mid-glyph offset snaps.
- `selected_text_reads_the_current_row_content` proves a copy re-reads a streamed row.
- `resize_during_a_live_selection_keeps_the_selected_text` proves a re-wrap does not move a
  logical endpoint.
- `a_zwj_emoji_at_a_selection_edge_is_never_split` proves a grapheme edge holds a ZWJ family
  whole.
- `a_streaming_delta_that_shrinks_a_row_clamps_the_endpoint` proves clamp-on-read.
- `clipboard_write_of_none_is_nothing_selected` proves the empty case.
- `clipboard_write_over_the_bound_is_too_large` proves the cap trips.
- `clipboard_write_wraps_the_text_in_osc52` proves the frame shape.
- `clipboard_payload_holds_no_escape_after_a_malicious_row` proves the sanitise rule.
- `autoscroll_moves_at_most_one_row_per_drag_event` proves the rate bound.
- `autoscroll_clamps_at_the_oldest_and_newest_row` proves the range bound.
- `affordance_hidden_while_pinned` proves it stays hidden at the newest row.
- `affordance_shows_the_hidden_below_count_from_the_rail` proves the count source.
- `affordance_click_jumps_to_the_newest_row` proves the mouse activation.
- `end_key_jumps_to_the_newest_row` proves the keyboard activation.
- `alt_v_begins_select_mode_only_while_the_draft_is_empty` proves the guard.
- `y_copies_and_leaves_select_mode` proves the keyboard copy.
- `esc_cancels_the_selection` proves the cancel path.

## Out of scope

- Rectangular or block selection.
- Multiple clipboard registers, and the primary-versus-clipboard distinction.
- App selection while `--no-mouse` releases capture.
- Horizontal scrolling of a wide row.
- Search inside the transcript.
- A read of the OSC 52 clipboard back into rho.
