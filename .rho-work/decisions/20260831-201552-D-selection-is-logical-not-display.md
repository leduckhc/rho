# D-selection-is-logical-not-display

Status: accepted
Date: 20260831-201552
Spec: SPEC-select-text-and-find-the-bottom

## The question

In what unit does rho store a text selection? A screen cell, a byte, a character, or a
grapheme cluster? And who mutates the selection when a row changes?

## The decision

A selection endpoint is a byte offset into a row's sanitised logical text. The offset lands
on a grapheme-cluster boundary. The row is a logical row index, not a display row.

The reducer never mutates the selection. rho clamps an endpoint on read. rho re-reads the
covered text at copy time.

## Why

The renderer wraps text for the terminal width. A resize re-wraps it. A screen cell would
move under the same text after a resize. A raw byte or a raw character can split a wide CJK
glyph or a ZWJ emoji family. A byte offset on a grapheme boundary is a valid slice index and
never splits a glyph.

The transcript is append-only. `crates/rho-tui/src/state.rs` only pushes a row. So a logical
row index is stable for the whole session.

Clamp-on-read keeps the reducer a pure fold. The reducer does not learn about the selection.
A streaming delta stays simple.

## What this rules out

- A cell-based selection that survives a resize.
- A character or byte offset that may split a grapheme at an edge.
- A reducer that mutates the selection on every streamed delta.

## The risk

The logical row index depends on the append-only invariant. If a future change removes or
reorders a row, an endpoint would point at the wrong row. A test pins the invariant.
