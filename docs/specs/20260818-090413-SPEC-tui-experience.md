# SPEC-tui-experience — The TUI experience

Status: draft for sprint 3, stage U2.
Owning crate: `rho-tui`. Consumed by `rho-cli`.

This spec extends `SPEC-tui`. It grows the minimal TUI into the interface that
`docs/tui-design.md` approved. It changes no rule from `SPEC-tui`. Three rules bind
this work. State is a pure function of events. A render is pure and testable. Input
never blocks on model work.

The design is the contract. This spec turns each design decision into a testable
rule and a compilable signature. It reinterprets nothing. The frame mocks in
`docs/design/tui-frames/` are the layout acceptance criteria.

Features covered: F-themes (the theme surface), F-slash-commands (the slash list), and
F-keybindings (the shortcuts and help). It also designs durations, concise mode, paste
collapsing, attachments, motion, and the guide, which the design lists for this stage.

## 1. Layout

The layout stacks fixed regions around one region that grows. The reference frame is
`100-idle.txt`. Every region matches the design layout table.

| Rows | Region | Fixed or grows |
| --- | --- | --- |
| 1 | Header content | fixed |
| 1 | Header rule, full width | fixed |
| rest | Transcript | grows and scrolls |
| 0 to 7 | A transient panel | absent by default |
| 3 to 10 | Composer box | grows with the draft |
| 1 | Footer | fixed |

The fixed cost is six rows. They are the header, the rule, a three-row composer, and
the footer. A 24-row terminal gives the transcript 18 rows. Only the transcript
scrolls. A long transcript drops rows from the top.

A transient panel is the slash list, the help screen, or the approval prompt. Only one
panel shows at a time. The panel borrows its rows from the transcript. It never takes a
row from the header or the footer. The composer also borrows from the transcript when
the draft grows.

Rules:
- The header, the rule, and the footer keep their rows at every width.
- The composer grows down to a cap. See section 5.
- The transcript keeps the newest row visible.

## 2. The duration ladder

The format is Makit's ladder, taken exactly. It arrives with a proven defect class,
so this spec copies the carry tests too.

| Span | Output |
| --- | --- |
| under 9.95 seconds | one decimal, trailing `.0` stripped: `2.4s`, `2s` |
| 10 to 59 seconds | whole seconds: `13s`, `59s` |
| 1 to 59 minutes | zero-padded seconds: `2m 41s`, `18m 04s` |
| 1 to 23 hours | zero-padded minutes: `4h 12m`, `1h 00m` |
| a day or more | days and hours: `3d 4h` |

The rule that matters most is where the rounding happens. Round exactly once, at the
top. Then use integer arithmetic for every tier below. Rounding inside a tier lets a
carry escape it. That defect shipped in Makit's own mockup. It turned `59.5s` into
`60s`, `119.7s` into `1m 60s`, and `3599.7s` into `59m 60s`.

An unrepresentable span renders an empty slot. It never renders a zero. A negative span
is unrepresentable, because a clock can step back, so `end < start` is reachable. An
open span is unrepresentable, because no end event has arrived.

The signature is verbatim below. It returns `Option<String>`. The caller passes `None`
for an open span. `None` comes back for both the open span and the negative span.

```rust
/// Format a span on Makit's duration ladder.
///
/// `span_millis` is the time between a start event and its end event.
/// `None` means the span is open, because no end event has arrived yet.
/// A `Some` value below zero means the clock stepped back, so `end < start`.
/// Round exactly once here at the top, then use integer arithmetic for every tier.
/// Return `None` for an unrepresentable span, so the caller draws an empty slot.
pub fn format_duration(span_millis: Option<i64>) -> Option<String>;
```

## 3. The duration slot

Every duration sits in a fixed slot. The slot is seven columns wide. The value is right
aligned inside it. The slot is seven columns everywhere, at every width.

Seven columns is the width of the widest rung, `18m 04s`. A fixed slot means a live
tick never reflows the text beside it. A value that grows from `9.1s` to `2m 41s` moves
no character after the slot. That is the whole point of reserving the column.

```rust
/// The reserved width of every duration slot, in columns.
pub const DURATION_SLOT_COLUMNS: usize = 7;
```

## 4. Where durations appear

Each duration comes from the tick count the state carries. No render reads a clock.

| Duration | Where | Live |
| --- | --- | --- |
| Session | the header, rightmost | ticks while a turn runs |
| Turn | the footer, after the working word | live, then kept when done |
| Tool call | its own row slot | live while running, final when done |
| Thinking | its `∴` row | live while streaming, final when done |

A live duration turns `warn` amber once it passes one minute. It returns to its normal
role when it completes. The amber cue is Makit's escalation.

## 5. Paste collapsing

A paste at or above the threshold collapses to one chip. The threshold is 1000
characters. The chip label is `[paste N chars]`, where `N` is the character count. A
second paste of the same size reads `[paste N chars #2]`. The suffix is codex's repeat
suffix, because two same-size pastes must stay distinct.

The full pasted text is held aside. It still reaches the model on send. The chip is one
unit. A backspace deletes the whole chip.

The composer height is bounded. It grows one row per draft line, to eight text rows, ten
rows with the box borders. Past the cap the draft scrolls inside the box. The cursor row
stays visible. The top row shows a `muted` `…` marker.

A paste burst is a paste that arrives as a stream of key events, on a terminal with no
bracketed paste. A burst detector buffers the keys. It flushes them through the paste
path. So a pasted `?` never opens the help.

```rust
/// A paste at or above this character count collapses to one chip.
pub const LARGE_PASTE_CHARS: usize = 1000;

/// The bounded composer height, in text rows. Ten rows with the box borders.
pub const COMPOSER_MAX_TEXT_ROWS: usize = 8;

/// The chip that stands in for one large paste in the composer.
pub struct PasteChip {
    /// The character count of the held text.
    pub chars: usize,
    /// The repeat index. The first paste of a size is 1, the second is 2.
    pub repeat: u32,
}

/// The chip label, for example `[paste 12431 chars]` or `[paste 12431 chars #2]`.
pub fn paste_chip_label(chip: &PasteChip) -> String;
```

## 6. Attachments

An image paste or path becomes one chip. The chip label is `[image #N size]`, where `N`
is the attachment index. The size is the image size, for example `1.2MB`. The chip has
the same behaviour as a paste chip.

The provider caps the image size. The cap is five megabytes. An oversize attachment is
refused in place. The footer states `image too large: 12MB, the limit is 5MB` in `warn`.
The refused attachment never reaches the model.

An attachment path stays inside the session root. A path outside the session root is
refused, so a draft cannot read a file the session may not read.

```rust
/// The provider size cap for one image attachment, in bytes.
pub const IMAGE_MAX_BYTES: u64 = 5 * 1024 * 1024;

/// The chip that stands in for one image attachment.
pub struct ImageChip {
    /// The attachment index. The first is 1.
    pub index: u32,
    /// The image size in bytes.
    pub bytes: u64,
}

/// The chip label, for example `[image #1 1.2MB]`.
pub fn image_chip_label(chip: &ImageChip) -> String;
```

## 7. Concise mode

Concise mode collapses a tool row to its header. The header keeps the verb, the payload,
the duration slot, the status glyph, and the caret. The body is hidden. The body holds
the last twelve output lines. A failed row expands itself, because the output is the
point.

The caret shows the fold state. A collapsed row shows `▸`. An expanded row shows `▾`.
The ASCII carets are `>` and `v`.

The expand keys are three. `ctrl-o` toggles the newest row. `enter` toggles the selected
row. `ctrl-e` expands everything for a review pass.

Concise mode is opt-in. The default is off, so a tool row shows its header only when the
user turns concise mode on. The setting is `tui.concise`.

```rust
/// True when concise mode collapses a tool row to its header. Off by default.
pub const CONCISE_MODE_DEFAULT: bool = false;

/// Whether one transcript row is collapsed or expanded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowFold {
    Collapsed,
    Expanded,
}
```

## 8. Motion

There is one motion. A highlight band sweeps the working word in the footer, left to
right. The word is `working`, `thinking`, or `waiting`. Nothing else animates.

The period is two seconds. The state carries a tick count. The event loop advances the
tick every 100 milliseconds, only while a turn runs. The period is 20 ticks. The frame
is a pure function of `tick % 20`. A test asserts any frame by picking a tick.

The band is a raised cosine. Its half width is five columns. Ten columns pad each end,
so the sweep enters and leaves cleanly. The band shape is codex's shape.

A render must never read a clock. The tick is the only time source. That keeps the
render pure and testable.

The sweep stops, and the word renders plain, under any of these conditions. The user
sets `tui.motion = false`. The user passes `--no-motion`. Stdout is not a terminal. A
reduced-motion preference is set by `tui.reduce_motion = true`. The variable
`RHO_REDUCE_MOTION=1` is set.

With motion off the interface loses no information. The word names the state. The
durations still count.

```rust
/// The sweep period, in ticks. Each tick is 100 milliseconds, so the period is 2 s.
pub const SWEEP_PERIOD_TICKS: u64 = 20;

/// The raised-cosine weight of one column at one tick, from 0.0 to 1.0.
///
/// The frame is a pure function of `tick`, so a test asserts a frame by its tick.
/// The band half width is five columns, with ten columns of padding at each end.
pub fn sweep_weight(tick: u64, column: usize) -> f32;

/// The no-true-colour rendering of one swept cell, by weight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionCell {
    Dim,
    Plain,
    Bold,
}

/// Map a sweep weight to its 256-colour or no-colour tier.
pub fn motion_cell(weight: f32) -> MotionCell;
```

## 9. Slash commands, shortcuts, help, and the guide

The footer always shows the current three moves. Idle, it reads `enter send · /
commands · ? help`. A first-time user finds everything from there.

Typing `/` in an empty draft opens the slash list above the composer. Typing filters
it. `↑ ↓` choose a row. `enter` runs it. `esc` closes it. The selected row carries the
`❯` marker and renders reversed. `100-slash-list.txt` shows it open.

Typing `?` in an empty draft opens the help screen in the same framed panel.
`100-help.txt` shows it.

The binding table is the single source of truth. The help screen is generated from the
binding table. So the help can never drift from the keys. A test asserts that the help
rows match the binding table.

`/guide` runs a two-minute tour in the transcript. It prints example rows and names each
part. The empty state and the help screen both name `/guide`.

```rust
/// One key binding. The binding table is the single source of truth.
pub struct Binding {
    /// The key or key pair, for example `alt+enter`.
    pub keys: &'static str,
    /// The one-line summary shown on the help screen.
    pub summary: &'static str,
}

/// The whole binding table.
pub fn bindings() -> &'static [Binding];

/// The help rows, generated from the binding table, so the two cannot drift.
pub fn help_rows() -> Vec<String>;
```

## 10. The theme

A theme is a role table. A role list is testable. A colour list is taste. The role enum
is below. The dark theme is the default. `tui.theme = "light"` switches tables.

| Role | 256-colour | 16-colour | No colour |
| --- | --- | --- | --- |
| `text` | terminal default | default | plain |
| `muted` | 245 | default, dim | dim |
| `accent` | 78 | green | bold |
| `error` | 203 | red | bold |
| `warn` | 179 | yellow | bold |
| `caution` | 173 | yellow, bold | bold, reversed glyph |

In 16-colour mode `warn` and `caution` share yellow. The glyph and the bold weight keep
them apart. With no colour every state still reads, because every state carries a glyph.

```rust
/// A colour role. A theme is a role table, never a colour list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Text,
    Muted,
    Accent,
    Error,
    Warn,
    Caution,
}

/// A 16-colour terminal colour, before any modifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ansi16 {
    Default,
    Green,
    Red,
    Yellow,
}

/// One resolved role style, colour plus modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoleStyle {
    pub color: Ansi16,
    pub dim: bool,
    pub bold: bool,
    pub reversed: bool,
}

/// The 256-colour index for a role. `None` means the terminal default foreground.
pub fn role_256(role: Role) -> Option<u8>;

/// The 16-colour style for a role.
pub fn role_16(role: Role) -> RoleStyle;

/// The no-colour style for a role. Modifiers carry every meaning.
pub fn role_none(role: Role) -> RoleStyle;
```

## 11. The cost budget

The owner will not trade speed for beauty. Each feature states its added rows, its added
allocations per frame, and its added bytes held. Where the answer is zero, the cell reads
zero. Every number here is a rule a test can pin.

| Feature | Added rows | Added allocations per frame | Added bytes held |
| --- | --- | --- | --- |
| Durations | 0 | one 7-byte string per visible duration | 0 |
| Duration slot | 0 | 0 | 0 |
| Paste collapsing | 0 | 0 | the held paste text, N bytes per paste |
| Attachments | 1 chip row per attachment | 0 | the image size, per attachment |
| Concise mode | 0, it removes rows | 0 | one byte per tool row, the fold flag |
| Motion | 0 | 0 | 8 bytes, the tick count |
| Shortcuts and help | 0 in the resting UI | 0 | 0, the binding table is static |
| Theme | 0 | 0 | 0 |
| Guide | 0 in the resting UI | 0 | 0 |

Motion holds to zero allocations per frame. It writes styles into cells that already
exist. It never allocates a string per character. That is the codex defect this spec
refuses.

## Test cases

All render tests use `ratatui::backend::TestBackend`. No test opens a real terminal.
Each test name states the one assertion it proves.

Duration ladder, the documented rungs:
- `duration_one_decimal_under_ten_seconds` — `2400ms` renders `2.4s`.
- `duration_strips_trailing_zero` — `2000ms` renders `2s`, not `2.0s`.
- `duration_whole_seconds_to_59` — `13000ms` renders `13s`, and `59000ms` renders `59s`.
- `duration_zero_padded_seconds_under_an_hour` — `161000ms` renders `2m 41s`.
- `duration_pads_the_seconds_field` — `1084000ms` renders `18m 04s`.
- `duration_zero_padded_minutes_under_a_day` — `15120000ms` renders `4h 12m`.
- `duration_pads_the_minutes_field` — `3600000ms` renders `1h 00m`.
- `duration_days_and_hours` — `273600000ms` renders `3d 4h`.

Duration ladder, the four carry cases:
- `duration_carry_59_5s_is_1m_00s` — `59500ms` renders `1m 00s`, never `60s`.
- `duration_carry_119_7s_is_2m_00s` — `119700ms` renders `2m 00s`, never `1m 60s`.
- `duration_carry_3599_7s_is_1h_00m` — `3599700ms` renders `1h 00m`, never `59m 60s`.
- `duration_carry_9_96s_is_10s` — `9960ms` renders `10s`, never `10.0s` and never `9.9s`.

Duration edges and the slot:
- `duration_empty_slot_for_open_span` — `format_duration(None)` returns `None`.
- `duration_empty_slot_for_negative_span` — a span below zero returns `None`.
- `duration_none_renders_an_empty_slot_not_a_zero` — an open span draws blank columns.
- `duration_slot_is_seven_columns` — every rendered duration occupies seven columns.
- `duration_slot_right_aligns` — a short value sits at the right edge of the slot.
- `duration_tick_growth_does_not_reflow` — text after the slot holds still as the value grows.
- `duration_amber_past_one_minute` — a live duration past `60s` renders in `warn`.

Paste and attachments:
- `paste_over_threshold_collapses_to_chip` — a 1200-char paste renders `[paste 1200 chars]`.
- `paste_repeat_suffix` — a second same-size paste renders `[paste 1200 chars #2]`.
- `large_paste_still_reaches_the_model` — the full pasted text is sent on submit.
- `paste_chip_deletes_as_one_unit` — one backspace removes the whole chip.
- `paste_burst_does_not_open_the_help` — a burst of a `?` key never opens the help.
- `composer_height_is_bounded` — a tall draft caps at ten rows and scrolls inside.
- `image_chip_label` — a 1.2 MB image renders `[image #1 1.2MB]`.
- `oversize_attachment_is_refused` — a 12 MB image is refused and never sent.
- `oversize_attachment_states_the_limit` — the footer names the size and the 5 MB limit.
- `attachment_path_outside_root_is_refused` — a path outside the session root is refused.

Concise mode:
- `concise_mode_default_is_off` — a fresh state shows tool bodies by default.
- `collapsed_row_expands` — `enter` on a collapsed row shows its body.
- `failed_row_expands_itself` — a failed tool row shows its output with no key press.
- `concise_caret_shows_fold_state` — a collapsed row shows `▸`, an expanded row shows `▾`.

Motion:
- `motion_is_a_function_of_a_tick` — the same tick yields the same frame.
- `motion_period_is_twenty_ticks` — `tick` and `tick + 20` yield the same frame.
- `motion_off_when_tui_motion_false` — `tui.motion = false` renders the word plain.
- `motion_off_when_no_motion_flag` — `--no-motion` renders the word plain.
- `motion_off_when_stdout_not_a_terminal` — a non-terminal stdout renders the word plain.
- `motion_off_when_reduce_motion_setting` — `tui.reduce_motion = true` renders the word plain.
- `motion_off_when_reduce_motion_env` — `RHO_REDUCE_MOTION=1` renders the word plain.
- `render_never_reads_a_clock` — the render is a pure function of state, with no time read.

Help and the theme:
- `help_rows_match_the_binding_table` — every help row comes from the binding table.
- `theme_resolves_every_role` — each role maps to a 256-colour, 16-colour, and no-colour style.

The frame fixtures, guarded now, in `crates/rho-tui/tests/frames.rs`:
- `frame_fixture_<name>_is_exact` — one test per frame. Every row is exactly the stated
  width, and the frame holds 24 rows. Width means display width, not byte length, because
  a box character is three bytes and one column.
- `every_frame_fixture_is_grid_safe` — no frame holds a wide or combining character. Either
  one makes a hand-drawn frame disagree with the terminal, and the disagreement is
  invisible in a diff.
- `the_frame_set_is_complete` — the directory holds ten frames. A missing frame would
  quietly reduce the acceptance criteria.

The frame renders, which drive the real renderer and compare it against each fixture. These
need the renderer that stage U4 builds, so they land with it:
- `frame_100_idle_renders_at_100_columns` — `100-idle.txt` matches at 100 columns.
- `frame_100_streaming_renders_at_100_columns` — `100-streaming.txt` matches at 100 columns.
- `frame_100_tool_run_renders_at_100_columns` — `100-tool-run.txt` matches at 100 columns.
- `frame_100_approval_renders_at_100_columns` — `100-approval.txt` matches at 100 columns.
- `frame_100_error_renders_at_100_columns` — `100-error.txt` matches at 100 columns.
- `frame_100_empty_renders_at_100_columns` — `100-empty.txt` matches at 100 columns.
- `frame_100_slash_list_renders_at_100_columns` — `100-slash-list.txt` matches at 100 columns.
- `frame_100_help_renders_at_100_columns` — `100-help.txt` matches at 100 columns.
- `frame_80_streaming_renders_at_80_columns` — `80-streaming.txt` matches at 80 columns.
- `frame_40_streaming_renders_at_40_columns` — `40-streaming.txt` matches at 40 columns.

## Out of scope

The exclusions from `SPEC-tui` still hold. This spec adds no feature beyond the owner's
list. These stay out:

- Markdown rendering and syntax highlighting in the transcript.
- Mouse support and scrollback search.
- A model picker and a session picker in the TUI.
- A second motion, an idle motion, and any 3D animation.
- A remembered execute approval. See `D-no-remembered-execute-allow`.
- A real duration for a span that a clock cannot represent.
- Any theme colour outside the role table. A plugin replaces the role table, not the layout.
