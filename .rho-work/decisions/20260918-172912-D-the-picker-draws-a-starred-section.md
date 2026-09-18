# D-the-picker-draws-a-starred-section

Date: 20260918-172912

## The question

`display_rows` already puts the starred models first, after the current model. But nothing
on screen shows where the starred block ends and the catalog begins. How does the picker
draw a real, visible starred section without letting a user select a header?

## The decision

The renderer draws two dim header lines. It draws a `starred` header before the first
starred row. It draws a `models` header before the first catalog row. The current model row
draws no header, because its `(current)` tag already names it.

A header is render-only. It is never a row in `display_rows`. The state exposes the section
boundary through `ModelPicker::sections`, which returns the row count of the current,
starred, and catalog sections. The state also exposes `ModelPicker::header_rows`, which
returns how many headers the current filter needs.

A header draws only when its section keeps at least one row after the filter. Three budget
sites reserve `header_rows` lines: `picker_viewport_rows`, `panel_demand`, and
`model_picker_panel`. So the headers never push a model row past the panel edge.

`sections` does not run a second counting pass. `display_rows` and `sections` both read one
private constructor, `build_display`, which builds the rows and counts them in the same
pass. So the boundary and the order can never drift.

## What this rules out

- A header inside `display_rows`. It would shift every index. `selected` and
  `scroll_offset` index the filtered rows, so a header in the list would become selectable.
- A header for the current model. One row with a `(current)` tag needs no divider above it.
- A header drawn when its section is empty after the filter. An empty section shows nothing.
- A budget that ignores the headers. The old fixed line count would hide the last model row
  or overrun the panel. All three sites change: `picker_viewport_rows`, `panel_demand`, and
  the draw function `model_picker_panel`.
- A `sections` that re-counts the rows on its own. It reads `build_display`, so a change to
  the display order updates the counts too.
- A draw function that filters the rows on its own. `model_picker_panel` reads
  `filtered_indices`, so the drawn set and the reserved budget match.
- A boundary that the renderer guesses from row flags. A starred catalog row carries
  `starred = true` in both the starred section and the catalog section, so a flag cannot
  tell the two apart. The counts from `sections` are the boundary.

## Why

The user asked for a real, visible starred section. A dim header is the smallest change
that shows the boundary. Keeping the header render-only protects the selection model, which
already indexes rows and not headers. Exposing the counts, not a new list, keeps the one
source of truth in `display_rows`.
