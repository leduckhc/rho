# SPEC-the-tool-row-has-three-levels

Status: draft

Owner crate: `rho-tui`. Touches `rho-tui` only. No change to `rho-core` or `rho-tools`.

Covers three features: three levels of tool-call detail, a colour-coded diff for the
`edit` tool, and better rows for the other tools.

## Problem

A tool row shows one fixed shape. The user wants three shapes: a "super one-liner", a
"short-form", and a "full length" view. The user also wants the `edit` tool to show a
colour-coded diff, like git. Other tool rows need the same three levels.

Today `crates/rho-tui/src/concise.rs` holds two levels, `RowFold::Collapsed` and
`RowFold::Expanded`. It is scaffolding. No production path reaches it. `render.rs` draws
rows itself and never calls `tool_row_lines`. The key help advertises `ctrl-o`, and no
key folds a row.

## The sides

- The reducer in `crates/rho-tui/src/state.rs`. It owns the row metadata.
- The renderer in `crates/rho-tui/src/render.rs`. It reads the metadata and draws.
- The theme in `crates/rho-tui/src/theme.rs`. It owns the colour roles.
- The level model in `crates/rho-tui/src/concise.rs`. It owns the types.

All four sides live in `rho-tui`. The contract they share is the level enum, the body
line model, and the theme roles below.

Contract kinds this change touches: the data model (`RowDetail`, `BodyLine`, `LineKind`),
the configuration (a default level), the extension surface (a new tool renders with no
renderer edit), and the behaviour rules (which key changes a level, and what a failed
row does).

## Contract: the three levels

Replace `RowFold` with a three-level enum in `crates/rho-tui/src/concise.rs`.

```rust
/// How much of a tool row shows. Three levels, coarse to fine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowDetail {
    /// One line: the header alone. The "super one-liner".
    Line,
    /// The header plus a bounded body preview. The "short-form".
    Short,
    /// The header plus the whole body. The "full length".
    Full,
}

/// The level a fresh tool row starts at, when nothing has set one.
pub const ROW_DETAIL_DEFAULT: RowDetail = RowDetail::Short;

/// How many body lines the `Short` level keeps.
pub const SHORT_BODY_LINES: usize = 3;

/// The level a fresh tool row takes, from the default and whether it failed.
///
/// A failed row starts at `Full`, because its output is the point. So a level that
/// hides a body can never hide an error.
pub fn initial_tool_detail(default: RowDetail, failed: bool) -> RowDetail {
    if failed { RowDetail::Full } else { default }
}

/// The next level. `ctrl-o` runs this on the newest tool row. The cycle wraps.
pub fn cycle_detail(detail: RowDetail) -> RowDetail {
    match detail {
        RowDetail::Line => RowDetail::Short,
        RowDetail::Short => RowDetail::Full,
        RowDetail::Full => RowDetail::Line,
    }
}

/// The caret glyph for a level. `Line` and `Short` can open further, so both show `▸`.
/// `Full` is fully open, so it shows `▾`.
pub fn detail_caret(detail: RowDetail) -> &'static str {
    match detail {
        RowDetail::Line | RowDetail::Short => "▸",
        RowDetail::Full => "▾",
    }
}
```

### What each level shows, per tool

The renderer treats every tool the same way. A row is a header line plus body lines. The
header is the glyph, the verb, the payload, and the duration slot. The body is the tool
output, one `BodyLine` per line. So a new tool needs no renderer code (see Extension
point).

| Level | Header | Body | When space runs out |
|-------|--------|------|---------------------|
| `Line` | one line, caret `▸` | none | the payload is cut with a marked ellipsis by `fit_to_width` |
| `Short` | one line, caret `▸` | first `SHORT_BODY_LINES` body lines, then a `… +N more` line when longer | each body line is cut to the measure |
| `Full` | one line, caret `▾` | every body line | each body line is cut to the measure |

The `edit` tool differs only in what its body holds: a unified diff. The three levels are
the same. `Line` shows `✓ edit  path.rs`. `Short` shows the summary and the first diff
lines. `Full` shows the whole diff, colour-coded.

`read`, `list`, `glob`, `grep`, `write`, `bash`, `task`, `task_cancel`, and
`read_tool_result` each show their own output lines in the body, cut the same way.

## Contract: how a level is chosen and changed

- Per row. Each tool row carries its own level. A global toggle is out of scope.
- The reducer stores one level per row, parallel to `rows`, and replaces
  `row_folds: Vec<RowFold>` in `crates/rho-tui/src/state.rs` with:

```rust
/// The detail level of each tool row, parallel to `rows`. A missing entry is
/// `ROW_DETAIL_DEFAULT`.
pub row_detail: Vec<RowDetail>,
```

- `ctrl-o` cycles the newest tool row through `Line`, `Short`, `Full`, and back. The
  binding summary in `crates/rho-tui/src/bindings.rs` becomes
  `"ctrl-o  cycle the newest tool row: one line, short, full"`, and its `built` flag
  becomes `true`.
- A failed row starts at `Full`. `ctrl-o` may still cycle it, because a user who read the
  error may want it small. The start level is the safe default, not a lock.

## Contract: the default, and the fail-open rule

The default is `RowDetail::Short`. A one-liner hides too much for a coding agent, and
`Full` floods the transcript when a `read` returns a whole file. `Short` shows the shape
of the output and stays compact.

`initial_tool_detail` forces `Full` for a failed row. So the default never hides an
error. That is the fail-safe rule this spec requires.

## Contract: the diff for the `edit` tool

The diff is computed in `rho-tools`, and it already is: `edit.rs` builds a
`similar::TextDiff` unified diff and returns it inside `ToolOutput::text`. This spec adds
no diff code to `rho-core` or `rho-tools`.

Reasons for that choice:

- The same unified-diff text must reach the model, so it must travel as text anyway.
- `rho-core` must hold no diff library and no terminal concern.
- The `+`, `-`, and ` ` prefixes of a unified diff are a mark a colour-blind reader can
  read. Colour never carries the meaning alone.

The renderer classifies each body line into a colour role. Classification is a pure
function in `crates/rho-tui/src/concise.rs`, over the sanitised body text:

```rust
/// The semantic class of one tool body line, for colour and for the level cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// Ordinary output, or a diff context line.
    Context,
    /// An added line in a diff. Its text keeps its leading `+`.
    Added,
    /// A removed line in a diff. Its text keeps its leading `-`.
    Removed,
    /// A diff file or hunk header, for orientation.
    Meta,
}

/// One tool body line: its class and its verbatim text.
///
/// The `+`, `-`, or ` ` mark stays in `text`. Colour never carries meaning alone, so a
/// reader who sees no colour still reads the mark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BodyLine {
    pub kind: LineKind,
    pub text: String,
}

/// Classify a sanitised tool body into lines.
///
/// A line is `Added` or `Removed` only inside a diff hunk, which a `@@` line opens. A
/// plain tool never emits `@@`, so its output is never mis-coloured. The `---`, `+++`,
/// `@@`, `before`, and `after` header lines are `Meta`.
pub fn classify_body(body: &str) -> Vec<BodyLine> {
    let mut in_hunk = false;
    body.lines()
        .map(|line| {
            if line.starts_with("@@") {
                in_hunk = true;
                return BodyLine { kind: LineKind::Meta, text: line.to_string() };
            }
            if line.starts_with("---") || line.starts_with("+++") {
                return BodyLine { kind: LineKind::Meta, text: line.to_string() };
            }
            let kind = if in_hunk && line.starts_with('+') {
                LineKind::Added
            } else if in_hunk && line.starts_with('-') {
                LineKind::Removed
            } else {
                LineKind::Context
            };
            BodyLine { kind, text: line.to_string() }
        })
        .collect()
}
```

The reducer must store the body. Today `on_tool_end` in `state.rs` drops
`output.content`, so the diff never reaches a row. `on_tool_end` must take the output
text, run `classify_body`, and store the result. So `row_bodies` in `state.rs` changes
type:

```rust
/// The classified body lines of each tool row, parallel to `rows`.
pub row_bodies: Vec<Vec<BodyLine>>,
```

`classify_body` runs after `sanitize_line`. So an escape sequence is dropped before
classification, and a hostile `+` written by an escape cannot fake a diff colour.

### Theme roles a diff needs

Add two roles to `crates/rho-tui/src/theme.rs`, and extend `Role::ALL` to `[Role; 14]`.
Every role table (`role_256`, `role_bg_256`, `role_16`, `role_none`) is exhaustive, so a
missing mapping fails the build.

```rust
    /// An added diff line. Green in colour modes. Its `+` mark carries the meaning with
    /// no colour.
    DiffAdd,
    /// A removed diff line. Red in colour modes. Its `-` mark carries the meaning with
    /// no colour.
    DiffDel,
```

- `role_256`: `DiffAdd => Some(71)` (a green apart from accent 78), `DiffDel => Some(167)`
  (a red apart from error 203).
- `role_16`: `DiffAdd => Ansi16::Green`, `DiffDel => Ansi16::Red`.
- `role_none`: both resolve to `RoleStyle::plain()`, because the `+` and `-` marks carry
  the meaning with no colour.
- `role_bg_256`: both `None`. Only `UserBand` paints a background.

The renderer maps a `LineKind` to a role. This arm lives in `render.rs`, and it is closed
over `LineKind`, not over the tool name:

```rust
fn line_role(kind: LineKind) -> Role {
    match kind {
        LineKind::Added => Role::DiffAdd,
        LineKind::Removed => Role::DiffDel,
        LineKind::Context | LineKind::Meta => Role::Muted,
    }
}
```

## Extension point

A new tool renders at all three levels with no renderer edit. The renderer never matches
on the tool name. It reads the header fields the reducer already sets, and the body lines
the reducer already classified. A tool changes its row only by the text it emits.

A tool that wants diff colours emits a unified diff, and `classify_body` colours it. A
tool that emits plain output gets plain body lines. Neither path adds a case to shared
code. So the contract is open for extension and closed for modification.

## Long, hostile, and wide payloads

- **A very long payload.** `Line` cuts the payload to the measure with `fit_to_width`,
  which marks the cut with an ellipsis. `Short` keeps `SHORT_BODY_LINES` lines and adds a
  `… +N more` line. `Full` keeps every line, each cut to the measure.
- **An escape sequence.** `sanitize_line` drops the whole sequence before the row stores
  the text, and before `classify_body`. So no output can move the cursor or fake a
  prompt, and no escape can fake a diff mark.
- **A wide or combining grapheme.** `fit_to_width` counts a wide glyph as two columns, so
  a row never overflows. A combining cluster is a known defect: `fit_to_width` cuts per
  character, not per grapheme cluster, so it can split a cluster. This spec pins the
  current behaviour with a test and does not fix it. See the unresolved question.

## Test cases

- `row_detail_line_is_header_only` — a `Line` row renders exactly one line.
- `row_detail_short_keeps_three_body_lines` — a `Short` row with six body lines renders
  the header plus three body lines plus a `… +3 more` line.
- `row_detail_full_keeps_every_body_line` — a `Full` row with six body lines renders the
  header plus six body lines.
- `cycle_detail_wraps_line_short_full` — `cycle_detail` maps `Line`→`Short`, `Short`→
  `Full`, `Full`→`Line`.
- `initial_tool_detail_uses_default_for_a_passing_row` — with default `Short` and
  `failed=false`, the level is `Short`.
- `initial_tool_detail_forces_full_for_a_failed_row` — with any default and `failed=true`,
  the level is `Full`.
- `ctrl_o_cycles_the_newest_tool_row` — one `ctrl-o` moves the newest tool row from
  `Short` to `Full`, and a second moves it to `Line`.
- `ctrl_o_help_is_marked_built` — the `ctrl-o` binding reports `built=true`.
- `classify_body_marks_added_and_removed_inside_a_hunk` — a diff with `@@`, a `+` line,
  and a `-` line yields `Meta`, `Added`, `Removed` in order.
- `classify_body_leaves_plain_output_as_context` — output with a leading `-` and no `@@`
  yields only `Context` lines.
- `classify_body_keeps_the_diff_mark_in_the_text` — an `Added` line's text still starts
  with `+`.
- `on_tool_end_stores_the_edit_diff_as_the_body` — after a `ToolEnd` with an edit diff,
  the row body holds the classified diff lines.
- `diff_add_and_del_have_all_three_theme_mappings` — `role_256`, `role_16`, and
  `role_none` each resolve `DiffAdd` and `DiffDel`.
- `diff_roles_have_no_colour_in_no_colour_mode` — `role_none(DiffAdd)` and
  `role_none(DiffDel)` equal `RoleStyle::plain()`.
- `an_edit_diff_line_reads_without_colour` — a rendered removed line still begins with
  `-`, so a no-colour reader can read it.
- `a_tool_body_escape_sequence_is_dropped_before_classify` — a body line with an escape
  sequence is sanitised, so it never becomes a false diff mark.
- `fit_to_width_splits_a_combining_cluster_today` — pins the known grapheme defect, so a
  later fix has a failing test to flip.

## Out of scope

- A global level, for the whole transcript at once. Only the per-row cycle ships.
- A level for a thinking row, an assistant row, or an agent row. Only tool rows change.
- A syntax-highlighted diff. Only add, remove, context, and meta get a colour.
- A config key or a CLI flag for the default level. The default is `Short`, in code.
- Fixing `fit_to_width` to cut per grapheme cluster. That is a separate bug fix.
- Persisting a row's level across a session restart.

## Unresolved question

- Should this spec also fix the `fit_to_width` per-cluster defect, or leave it to a
  bug-fix lane? AGENTS.md says never leave a verified bug unfixed. The test
  `fit_to_width_splits_a_combining_cluster_today` records it either way.
