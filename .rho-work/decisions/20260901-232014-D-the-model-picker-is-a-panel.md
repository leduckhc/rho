# D-the-model-picker-is-a-panel

Date: 20260901-232014

## The question

`/model` opens what? A modal dialog, a full-screen replacement, or a panel over the
composer? The interface already draws four panels: `Approval`, `SlashList`, `Help`, and
`Guide`. Pick one, or add a fifth.

## The decision

The picker is a fifth `Panel::ModelPicker` variant, drawn over the composer the same way
`SlashList` is. It follows the same routing:

- Only one panel shows at a time.
- Esc closes it and keeps the draft.
- Every other panel's key routing is unchanged.
- The picker is opened only by `/model` with no argument. `/model <id>` bypasses it.
- The renderer plans its layout through the same seam as `SlashList`, so a mouse click
  reaches a row through `slash_row_index`. See D-a-panel-nobody-can-open.

Contents, top to bottom:

1. A header row: `model:  <current-id>` with `★` when starred and `[effort=<name>]` at the
   end.
2. One row per starred model that is not the current one, `☆ <id>` on the row so the
   symbol column stays fixed.
3. A single hint row at the bottom: `↑ ↓ choose · enter apply · e effort · * star · esc close`.
4. When the starred list is empty and the current model is the only row, no notice needs
   to say so; the header is proof.

Keys inside the panel are stated in `D-model-picker-allows-fuzzy-search-and-typed-fallback`,
which amended this decision to add fuzzy filtering. The panel takes `↑ ↓ Enter Tab
Shift+Tab Esc Backspace` as controls, and every printable character types into a fuzzy
query. The one-line summary:

- `↑ / ↓` move the selection over the filtered rows. The selection wraps neither end.
- `Enter` applies the highlighted filtered row, or the query verbatim when no row matches.
- `Tab` cycles the highlighted row's effort through `unset → off → low → medium → high →
  xhigh → unset`. The change is preview only. `Enter` is what applies.
- `Shift+Tab` toggles the star on the highlighted row. The write persists at once.
- `Esc` closes with no change.
- `Backspace` removes the last query character.
- Any other printable character appends to the query.

## What this rules out

- A `Panel::ModelPicker` that reuses `SlashList` internals. The two panels have different
  contents and different keys, and squeezing them into one would grow every existing
  arm of the slash key handler.
- A full-screen picker. rho draws a transcript, and the transcript stays on screen while
  a user picks. It is context.
- A picker that lists every model the provider knows. Listing is a later feature, per
  D-a-listing-failure-never-stops-a-session, and v1 has no `Provider::list_models`.

## Why

A panel is the smallest primitive the existing renderer draws well, so a new panel adds no
new render primitive. The keys mirror `SlashList`, so a user who has used the slash list
already knows the arrow keys.
