# D-one-detail-model-for-every-row — a thinking row folds like a tool row

Date: 20260831. Reference: `D-one-detail-model-for-every-row`.
Specs: `docs/specs/20260831-201549-SPEC-the-tool-row-has-three-levels.md`, and
`docs/specs/20260819-134615-SPEC-reasoning-across-providers.md` section 5.

## The question

The requester wants the reasoning text shown by default, and one key to collapse it. rho already
folds a tool row. Should a thinking row get its own mechanism, or the same one?

## What exists today

The two row kinds are drawn by different rules, and only one of them can be changed after the
fact.

A tool row reads a **per-row** fold. `crates/rho-tui/src/render.rs` line 625 asks
`row_fold(state, index)` and draws the body only when the row is expanded. The state already
carries `row_folds`, parallel to `rows`. So the machinery works, and no key changes it. The key
help advertises `ctrl-o` and says `not built yet`.

A thinking row reads a **global** mode, `state.reasoning_display`, with four values: `Off`,
`Summary`, `Full`, and `Live`. There is no per-row state, so nothing can collapse one block
while another stays open. Changing the mode needs a restart.

## The decision

**One detail model serves every row that has a body.** A thinking row takes the same
`RowDetail` a tool row takes, and the same key changes it.

```rust
/// How much of a row's body shows. One model for every row that has a body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowDetail {
    /// The header only. A thinking row shows `∴ thought for 1.1s`.
    Line,
    /// The header and the first few body lines, then a count of the rest.
    Short,
    /// The header and the whole body.
    Full,
}
```

For a thinking row the three levels read:

| level | what shows |
| --- | --- |
| `Line` | `∴ thought for 1.1s` |
| `Short` | the summary row, the first body lines, then `… +N more` |
| `Full` | the summary row and the whole reasoning text |

So today's `Summary` is the new `Line`, and today's `Full` keeps its meaning and gains a middle
step.

## The default is `Full`

The requester asked for the text by default, and the measurement supports it. A short question
cost eight transcript rows at `Full` and one at `Summary`. That is the price of reading the
model's work, and it is the requester's call to pay it.

`Off` stays, because a user may want no reasoning at all, and that is not a detail level. It
remains a separate switch.

## `Live` is a different axis, and it stays one

`Live` is not a detail level. It says what happens **while** the row is still streaming, and a
measurement shows why the two must not merge: with `Live`, the text streams during the turn and
then disappears, so a settled `Live` row and a settled `Summary` row are identical. The only
difference is what a watcher saw.

So the model has two axes:

- **While streaming**: draw the text, or draw nothing yet.
- **After settling**: `Line`, `Short`, or `Full`.

`Live` therefore becomes "stream the text, then settle to `Line`". It is expressible in the new
model rather than deleted, and no user loses it.

## What it rules out

- No second fold mechanism, and no second key. A user learns one gesture.
- No global-only reasoning display. A key changes one row, and the global setting only chooses
  the level a new row starts at.
- No `RowDetail::Other` or `Unknown`. This project shipped a fail-open `ToolKind::Other` once.
- No colour as the only marker. A streaming thinking row must carry the `∴` glyph as well as
  the muted colour, because a measurement found that colour alone marks it today.

## Test cases

- `a_thinking_row_folds_to_its_summary_line` — the key takes a `Full` thinking row to `Line`.
- `a_thinking_row_cycles_three_levels` — the same key cycles `Full`, `Line`, `Short`.
- `the_default_thinking_level_is_full` — a new thinking row starts at `Full`.
- `a_streaming_thinking_row_carries_the_glyph` — the marker does not depend on colour.
- `a_live_row_settles_to_its_line` — the streaming axis and the settled level stay separate.
- `one_key_folds_a_tool_row_and_a_thinking_row` — one gesture, both row kinds.
