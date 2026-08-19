# SPEC-tui-alternate-screen — rho owns the screen, and owns the scroll

Status: draft, for review before any implementation.
Decision: `D-alternate-screen-after-all`.
Evidence: `docs/verification/alt-screen-spike.md`.

rho enters the alternate screen at startup and draws the whole terminal. The transcript
scrolls inside rho. The terminal's own search still reaches the screen, so rho writes no dump.

## 1. The sides, and who owns each

A side is any two places that must agree. This change has five.

| Side | Owner | What it must agree on |
| --- | --- | --- |
| The screen | `rho-tui` | enter, restore, and the layout |
| The terminal | the terminal emulator | the escape sequences, and the mouse protocol |
| The scroll state | `rho-tui` | the offset rules, and the pin rule |
| The keys | `rho-tui`, through `bindings.rs` | every scroll key answers, and the help states it |
| The command line | `rho-cli` | the mouse default |

The contract kinds this change touches: the public API, the data model, the error set, the
wire format to the terminal, the configuration, and the behaviour rules.

## 2. The screen guard

Restoration is structural. A statement at the end of `run` does not survive a panic or a
signal, and a measured `SIGTERM` left mouse reporting on, so the shell printed
`35;111;18M` on every mouse move. `SIGHUP` is what closing a window sends.

```rust
/// Owns the terminal modes for the lifetime of the UI.
///
/// `Drop` restores every mode, so a panic that unwinds through `run` still leaves the
/// terminal usable. Construction is the only way to enter the alternate screen.
pub struct ScreenGuard {
    mouse: bool,
    restored: bool,
}

impl ScreenGuard {
    /// Enter raw mode, the alternate screen, and mouse reporting when `mouse` is true.
    ///
    /// It also installs a panic hook that restores the terminal before the panic message
    /// prints. A panic message printed into raw mode is unreadable.
    pub fn enter(mouse: bool) -> Result<Self, TuiError>;

    /// Restore every mode now, and make `Drop` a no-op.
    ///
    /// The external editor needs the normal buffer, so it restores and enters again.
    pub fn restore(&mut self) -> Result<(), TuiError>;

    /// Enter the alternate screen again after `restore`.
    pub fn reenter(&mut self) -> Result<(), TuiError>;
}

impl Drop for ScreenGuard {
    fn drop(&mut self);
}

/// The exact sequences the guard writes, exposed so a test can pin them.
///
/// A future edit must not drop one silently, so `bench/check-sequences.py` greps for each.
pub fn enter_sequences(mouse: bool) -> String;
pub fn restore_sequences(mouse: bool) -> String;
```

`restore_sequences` must contain `?1049l`, and when `mouse` is true it must also contain
`?1006l`, `?1003l`, `?1002l`, and `?1000l`.

## 3. The scroll state

The newest row is at the bottom, directly above the composer. The offset counts rows from
the oldest row.

**`total` and `visible` are display rows, after wrapping, and never logical transcript
rows.** A transcript row that wraps to three screen rows counts as three. Mixing the two
units puts the offset in one unit and the screen in the other, which is how the banking
defect returns.

```rust
/// Where the transcript view sits, and whether it follows new output.
///
/// There is no `Default` derive. A derived default gives `pinned: false`, which parks the
/// view at the oldest row and stops it following output. `TuiState` derives `Default`, so
/// an embedded `Scroll` would inherit that silently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scroll {
    /// The first visible display row, counted from the oldest. It is advisory while the
    /// view is pinned, because `first_visible` then derives the position.
    offset: usize,
    /// True while the view follows the newest row.
    pinned: bool,
}

impl Default for Scroll {
    /// The default view follows output.
    fn default() -> Self;
}

/// The rows a wheel event moves. One, because trackpad momentum sends hundreds of events
/// for one gesture. A measured session sent 2789 of them.
pub const WHEEL_ROWS: usize = 1;
/// The rows a page key leaves overlapping, so the reader keeps their place.
pub const PAGE_ROWS_MARGIN: usize = 2;

impl Scroll {
    /// A view that follows the newest row. This is the state at startup.
    pub fn pinned() -> Self;

    /// The first display row to draw.
    ///
    /// A pinned view derives the newest position here, so it can never hold a stale
    /// offset. Returning the raw offset made a pinned view show the **oldest** row on a
    /// resumed session, while `is_pinned` still answered true.
    pub fn first_visible(&self, total: usize, visible: usize) -> usize;

    /// Move toward the oldest row, and release the pin.
    ///
    /// The offset is clamped **here**, where it changes. Clamping only where it draws let
    /// the offset climb past the last row, so a scroll back paid one event for every event
    /// overshot, and the wheel read as dead and then jumped.
    pub fn up(&mut self, rows: usize, total: usize, visible: usize);

    /// Move toward the newest row. Reaching the newest row restores the pin.
    pub fn down(&mut self, rows: usize, total: usize, visible: usize);

    /// Jump to the oldest row, and release the pin.
    pub fn to_oldest(&mut self);

    /// Jump to the newest row, and restore the pin.
    pub fn to_newest(&mut self, total: usize, visible: usize);

    /// Answer new output. A pinned view needs no work, because `first_visible` derives the
    /// new newest row. An unpinned view is clamped and never moved.
    pub fn on_new_rows(&mut self, total: usize, visible: usize);

    /// Answer a resize, because `visible` changed.
    pub fn on_resize(&mut self, total: usize, visible: usize);

    /// True while the view follows the newest row.
    pub fn is_pinned(&self) -> bool;

    /// The display rows hidden above and below, for the rail and a position readout.
    pub fn hidden(&self, total: usize, visible: usize) -> (usize, usize);
}
```

### The invariants

1. `first_visible(total, visible) <= total.saturating_sub(visible.max(1))` after **every**
   method. The clamp is at the mutation, never only at the draw.
2. A pinned view always reports the newest position from `first_visible`, whatever the
   offset holds.
3. `up` releases the pin **when rows exist above the view**. When the whole transcript
   fits, the view keeps following, so later output is not missed.
4. `down` restores the pin only when the view reaches the newest row.
5. `on_new_rows` moves a pinned view, and never moves an unpinned view.
6. `on_resize` restores the pin when the view lands on the newest row. Without it a view
   sitting visually at the bottom stays unpinned, and output piles up unseen.
7. `visible` is treated as at least one. A zero-height view must not let `first_visible`
   run past the end.
8. `WHEEL_ROWS` is 1. A wheel event moves one display row.
9. A horizontal wheel event is ignored by name. A measured session sent 537 of them, from
   trackpad drift during a vertical gesture.
10. rho offers no setting to invert the wheel. See `D-alternate-screen-after-all`.
11. The transcript is append-only, so `total` never shrinks except on a resize, which
    changes wrapping. No method may assume `total` only grows.

### The direction, stated visually

`ScrollUp` reveals **older** rows, so a row already on screen moves **toward the bottom**.
`ScrollDown` reveals newer rows, so a row on screen moves toward the top. The words "up"
and "down" name the intent and not the pixels, and that ambiguity is why this paragraph
exists.

### There is no extension point, and none is needed

`Scroll` and `ScreenGuard` are concrete types inside `rho-tui`, and no crate may depend on
`rho-tui`. A third party extends rho through `rho_core::Tool` and through a provider crate,
never through the screen.

A new scroll **key** needs no edit to `Scroll`: it calls `up` or `down` from the binding
table. A new terminal **mode** does force an edit to `enter`, `restore`, `enter_sequences`,
and `restore_sequences`, and that is accepted: a mode is a change to the contract with the
terminal, so it belongs in this spec first.

## 4. There is no transcript dump

An earlier draft of this spec gave `ctrl-p` a dump. It left the alternate screen, wrote every
transcript row into the terminal's own scrollback, and entered the alternate screen again. A
spike proved it works, and `docs/verification/alt-screen-spike.md` records that it recovered
40 of 40 rows.

**It is not built, because it answers a question nobody asked.** The dump existed to give
back the terminal's search and copy. The owner tested iTerm2 and Ghostty, and the terminal's
own search reaches the alternate screen in both. So nothing is lost, and a feature that
serves no need is a promise rho must keep for no gain.

The guard still needs `restore` and `reenter`, and the reason is the external editor. rho
leaves the terminal for `$EDITOR` and comes back. That path exists today in `app.rs`.

## 5. The keys

A key that already carries a meaning never scrolls. An earlier draft of this section listed
`ctrl-u`, `ctrl-d`, `↑` and `↓` as scroll keys, and all four were already taken. See
`D-scroll-keys-yield-to-an-empty-draft` and its supersede note.

| Keys | Scrolls? | Meaning |
| --- | --- | --- |
| wheel up, wheel down | yes | one display row |
| `pageup`, `pagedown` | yes | one screen, less `PAGE_ROWS_MARGIN` |
| `home`, `end` | while the draft is empty | the oldest row, and the newest |
| `ctrl-d` | never | quit while the draft is empty |
| `ctrl-u` | never | cut to the line start |
| `↑ ↓` | never | recall the history, move a selection |

Each key above is in `bindings()`, so the help screen states it and cannot drift. A scroll
key answers only while no panel is open and the transcript overflows.

The reducer reads no screen, so the event loop writes `transcript_total` and
`transcript_visible` into the state each frame. It already writes `composer_width` the same
way.

## 6. The layout

The whole terminal, top to bottom:

| Rows | Region |
| --- | --- |
| rest | the transcript, scrolled by `Scroll` |
| 0 to 1 | the rail, drawn only when the transcript overflows |
| 0 or more | a panel, sized to its content |
| 3 to 12 | the composer, two rules and the draft |
| 1 | the footer |

The composer keeps `D-ledger-wins-the-band`: two rules, open sides, ten draft rows at most,
which is twelve with the rules. A tool row keeps its status glyph first and its two-column
indent. An approval keeps its session root and yields no row.

`BAND_ROWS` stops being a layout budget. The help panel draws the whole table, because the
screen has room, so the counted header and the window are no longer needed. The window code
stays until the panel is rewritten, and the rewrite removes it.

## 6b. The freeze machinery is removed, because it is now wrong

The inline band pushed a finished row into the terminal's scrollback with
`Terminal::insert_before`, and rho could never repaint that row again. So the reducer drops
a late event that names an already-frozen row.

In the alternate screen rho owns every row and can repaint any of them. **That drop is now
a defect, not an optimisation: it discards output that rho is able to show.** So this is a
removal and not a deletion of dead code.

Removed: `next_freeze`, `freeze_all`, `banner_freeze`, `FreezeBatch`, and every
`insert_before` call. `frozen_rows` becomes zero and then goes, and the late-event drop path
goes with it. `BAND_ROWS` stops being a layout budget, and `plan_band` goes with the band.

Nothing inherits the freeze path's one useful capability, which was rendering a row as plain
text with no style. There is no dump, so that code goes too.

A test must pin the repair: `a_late_event_reaches_an_old_row`.

## 6c. A startup notice reaches the screen, because it did not before

rho printed its startup notices to the terminal, and then opened the alternate screen over
them. Measured on the release binary: the notices wrote at byte 5 and byte 320, and the
alternate screen opened at byte 535. So each one was visible for a few milliseconds, on a
buffer the user never looks at again.

One hidden line says a project skill stays unloaded until the user trusts it. That is a
security notice.

**No test could catch this**, because the fault was an ordering rule between two crates.
`rho-cli` owned the notices, and `rho-tui` owned the screen. Neither side was wrong alone.

The contract that joins them:

```rust
/// One rendered transcript row.
pub enum Row {
    // ... the existing variants ...
    /// A startup notice. Not an error: a default model and an unloaded skill both
    /// deserve a line, and neither one failed.
    Notice { message: String },
}

impl TuiState {
    /// Push a one-line notice row. The text is sanitised, and it wraps when drawn.
    pub fn push_notice(&mut self, message: impl Into<String>);
}

impl App {
    /// Seed the startup notices, in the order the caller gives them.
    pub fn with_notices<I, S>(self, notices: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>;

    /// The transcript rows the user sees. A frontend or a test reads what rho drew.
    pub fn live_rows(&self) -> &[Row];
}
```

On the `rho-cli` side, `build_config_with_notices` collects a notice instead of printing it.
`build_config` keeps its old signature and prints, because the non-interactive paths have no
screen and a print is right there.

**The rules.**

- A notice draws with the `!` glyph and the `Warn` role. It never draws as an error.
- A notice wraps. The skill notice ends with `Pass --trust-project to load them`, and a
  padded single line clipped exactly that.
- The label keeps a text column of at least `NOTICE_MIN_TEXT`, which is 12. Below it the
  label takes its own row and the text takes the whole measure. Without that floor the wrap
  width reached zero at width 24 or less, and the whole message vanished.
- The splash draws while **every** row is a notice, because a notice is chrome and not
  conversation. The notices draw under the starters.
- When the notices do not fit under the splash, rho falls back to the scrollable transcript.
  Truncating them inside the splash block would rebuild the defect in a new place.
- An error raised before the screen opens still goes to stderr. There is no screen yet.

See `D-a-notice-reaches-the-transcript`.

## 7. The error set

```rust
pub enum TuiError {
    /// The terminal refused a mode change or a write.
    Io(String),
    /// The terminal cannot hold the composer and the footer at startup.
    ///
    /// `need` is `COMPOSER_MIN_ROWS + FOOTER_ROWS`, which is four. This is fatal, and only
    /// at startup.
    TooSmall { rows: u16, need: u16 },
}
```

`TooSmall` is fatal **only at startup**. A resize that makes the terminal too small must
never end the session, because dragging a window narrow and wide again is ordinary. While
the terminal is too small to draw, rho draws what fits, in this order: the composer's draft
row, then the footer, then the transcript. It draws no panel. The guard still restores on
every path.

## 8. Configuration

| Key | Default | Reason |
| --- | --- | --- |
| `tui.mouse` | **true** | the wheel is the only way to scroll in the alternate screen |
| `--mouse`, `--no-mouse` | override | a user whose terminal loses selection needs the way out |

An old config file has no `tui.mouse` key. It reads as the new default, which is true, so
an existing user gains the wheel and does not have to edit a file. A user who wants the old
behaviour writes `tui.mouse = false` or passes `--no-mouse`.

The default flips from off to on. `D-alternate-screen-after-all` records why, and the cost:
mouse capture can take native selection away, so the footer must name the modifier that
restores it. A user who wants the mouse out of the way passes `--no-mouse`.

## 9. Test cases

Each test names the assertion it proves.

### The guard

- `the_guard_enters_the_alternate_screen` — `enter_sequences(false)` contains `?1049h`.
- `the_guard_restores_the_alternate_screen` — `restore_sequences(false)` contains `?1049l`.
- `the_guard_restores_every_mouse_mode` — `restore_sequences(true)` contains `?1000l`,
  `?1002l`, `?1003l`, and `?1006l`.
- `a_dropped_guard_restores_the_terminal` — a guard dropped inside a scope writes the
  restore sequences to its sink.
- `a_panic_restores_the_terminal` — a panic unwinding through a scope that holds a guard
  still writes the restore sequences. `catch_unwind` observes it.
- `a_restored_guard_does_not_restore_twice` — `restore` then `drop` writes the sequences
  once, because a double reset clears a line the shell has already drawn.

### The scroll

- `a_new_view_is_pinned_to_the_newest_row` — `Scroll::pinned().is_pinned()`.
- `scrolling_up_clamps_at_the_oldest_row` — `up` past the top leaves `offset` at zero.
- `scrolling_down_clamps_at_the_newest_row` — `up` then `down` past the end leaves
  `first_visible` at `total - visible`, and no further.
- `scrolling_up_releases_the_pin` — one `up` sets `is_pinned` false.
- `reaching_the_newest_row_restores_the_pin` — `down` to the end sets `is_pinned` true.
- `output_moves_a_pinned_view` — `on_new_rows` on a pinned view shows the newest row.
- `output_never_moves_an_unpinned_view` — `up`, then `on_new_rows`, leaves `first_visible`
  unchanged. This is the rule that stops output dragging the view away from a reader.
- `an_edge_event_is_absorbed_and_not_banked` — 500 `down` calls at the end, then one `up`,
  moves the view by one row at once. A measured session sent 960 edge events, and the old
  code required every one to be paid back.
- `a_resize_clamps_the_offset` — a shorter view clamps `first_visible`.
- `a_row_on_screen_moves_toward_the_bottom_when_scrolling_up` — the observable form of the
  direction rule. It names a row, scrolls up, and asserts the row's screen position grew.
  An offset assertion cannot catch an inverted convention, and this one can.
- `a_pinned_view_shows_the_newest_row` — `Scroll::pinned().first_visible(160, 20)` is 140,
  not 0. A review found this, and the first draft of the contract showed row 0 while
  `is_pinned` answered true.
- `the_default_view_follows_output` — `Scroll::default().is_pinned()`.
- `a_resize_to_the_bottom_restores_the_pin` — release the pin, resize so everything fits,
  and the pin returns.
- `to_oldest_shows_the_first_row_and_releases_the_pin`.
- `to_newest_shows_the_last_row_and_restores_the_pin`.
- `hidden_counts_the_rows_above_and_below` — the pair sums with `visible` to `total`.
- `a_zero_height_view_does_not_run_past_the_end` — `visible` of 0 is treated as 1.
- `a_wheel_event_moves_exactly_one_row` — pins `WHEEL_ROWS`.
- `a_page_key_leaves_an_overlap` — a page moves `visible - PAGE_ROWS_MARGIN` rows, so the
  reader keeps their place.
- `a_late_event_reaches_an_old_row` — the repair from section 6b. An event that names a row
  already scrolled out of view still updates that row, because rho can repaint it now.

### The external editor

- `the_guard_leaves_and_reenters_for_the_editor` — `restore` then `reenter` writes `?1049l`
  and then `?1049h`. This is the only caller of the pair, now that there is no dump.

### The keys and the help

- `every_scroll_key_has_a_binding_row` — each key in section 5 appears in `bindings()`.
- `the_help_states_every_scroll_key` — `help_rows()` contains each of them.
- `every_scroll_key_moves_the_view` — presses each of `pageup`, `pagedown`, `ctrl-u`,
  `ctrl-d`, `home`, `end`, `↑`, `↓` and asserts `first_visible` changed. A key in the
  binding table that moves nothing is the `help_panel` defect again: the table promised the
  key and the code answered nothing.
- `a_horizontal_wheel_event_moves_nothing` — `ScrollLeft` and `ScrollRight` leave `Scroll`
  unchanged.

### The layout

These live in `crates/rho-tui/tests/layout.rs`. `plan_screen` is public and had no direct
test before, and neither did `STARTUP_MIN_ROWS` or `TooSmall`.

- `the_transcript_takes_the_rows_the_composer_leaves` — a sweep over every height from the
  minimum to 60, four draft heights, and four panel shapes. The regions must sum to the
  height exactly. A row unaccounted for draws twice or not at all.
- `the_composer_keeps_its_ten_row_cap` — unchanged from `D-ledger-wins-the-band`. A 500 row
  draft still takes ten rows, and a draft under ten takes what it asks.
- `an_approval_states_its_session_root` — unchanged, and it must stay passing.
- `a_panel_floor_survives_a_tall_draft` — the floor holds in the tight band, heights 8 to 16.
  A ten-row draft would otherwise squeeze the panel out.
- `the_transcript_shrinks_when_the_composer_grows` — the direction of the trade. The footer
  never yields.
- `the_banner_yields_before_the_transcript_starves`.
- `a_terminal_too_short_reports_and_does_not_draw` — a two-row terminal returns `TooSmall`.
  It draws no panel, no banner, and no rules, and it keeps the draft row.
- `the_too_small_boundary_is_exactly_the_startup_minimum` — every height below the minimum
  reports, and every height at or above it draws.
- `a_small_screen_never_hides_the_draft_row`.
- `a_zero_row_terminal_plans_nothing_and_does_not_panic` — a resize storm reaches zero.
- `the_too_small_error_states_both_numbers` — the message says the size and the requirement.

### The notices

- `a_notice_becomes_a_transcript_row`.
- `a_notice_is_not_an_error` — the row is not `Row::Error`.
- `a_notice_row_is_sanitised` — an escape and a bell do not survive.
- `every_notice_reaches_the_transcript_in_order` — the pairing is complete, and ordered.
- `the_app_seeds_its_notices_into_the_transcript` — the wiring that was missing.
- `an_app_with_no_notice_shows_no_notice_row` — a quiet startup stays quiet.
- `a_notice_row_draws_its_text_and_says_notice`.
- `a_long_notice_keeps_its_tail` — the notice wraps, and no word is lost.
- `a_notice_survives_a_narrow_screen` — widths 20 to 120, and no word is lost at any of them.
- `the_splash_survives_a_few_notices`.
- `many_notices_stay_reachable_instead_of_truncated` — 40 notices report more rows than
  fit, so the wheel reaches them.
- `the_default_model_notice_is_data_and_not_a_print`, in `rho-cli`.
- `an_explicit_model_raises_no_notice`, in `rho-cli`.

## 10. Out of scope

- **The scroll rail's appearance.** One muted column, no arrows. Its shape is not designed
  here, and it draws only when the transcript overflows.
- **Copy and selection.** OSC 52 already has `D-copy-goes-through-osc-52`. The interaction
  between mouse capture and native selection is named in section 8 and not solved here.
- **The keyboard enhancement protocol.** `shift+enter` still cannot reach rho, and the
  alternate screen does not change that. It needs its own spec.
- **The tool row payload.** A tool row states no payload today, which
  `docs/verification/ledger-band-live.md` records. It is a separate defect.
- **Search inside rho.** The terminal's own search reaches the alternate screen in iTerm2
  and in Ghostty, and it is better than any search rho would write this year.
- **Windows and Linux measurement.** Every number here came from macOS, ghostty, and tmux.
