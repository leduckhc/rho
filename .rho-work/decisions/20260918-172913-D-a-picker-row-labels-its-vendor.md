# D-a-picker-row-labels-its-vendor

Date: 20260918-172913

## The question

The user asked for a dim provider name next to each model name, and for the fuzzy search to
match that provider name as well as the model. A session has one provider, drawn once in the
panel header. So what provider does a row show, and how does the filter use it?

## The decision

A row derives its vendor from the model id, with the pure function `vendor_label`. A row
shows the model name from `model_label`, which is the id with the vendor prefix and any
region prefix removed. The row draws the model name, then the dim vendor label next to it.

The fuzzy filter matches the query against the model name or the vendor label. It no longer
matches the full id. The full id is still what `Enter` applies, so the wire value never
changes.

`vendor_label` reads the id left to right. A `/` id gives the text before the first `/`. A
`.` id gives the first dot segment, unless that segment is a Bedrock region prefix, in which
case it gives the next segment. An id with no separator gives an empty label.

An empty vendor label draws nothing, and its separator draws nothing. Both text columns
pass through `sanitize_line` before they draw, because a model id is untrusted. The model
name is untrusted too, so the name column is sanitized, not only the vendor column.

The fuzzy filter lives in one place, `ModelPicker::filtered_indices`. The draw function
`model_picker_panel` reads that function. It keeps no second copy of the predicate, so the
drawn set and the reserved budget agree.

This decision amends the match rule of `D-model-picker-allows-fuzzy-search-and-typed-fallback`.
That decision matched the full id. This decision matches the shown model name or the vendor.
The full id stays the value that `Enter` applies.

## What this rules out

- A new `PickerRow` field for the vendor. A row is built in three places, so a field would
  go stale. The pure function reads the id every time, so nothing can drift.
- A change to `ModelDescriptor`. The vendor comes from the id, so the descriptor keeps `id`
  and `display_name` only. See `D-a-model-descriptor-carries-no-capability-claim`.
- Repeating the session provider on every row. The session provider is constant and already
  shows in the panel header, so it carries no per-row information.
- A filter that matches the full id and also the vendor. The vendor is a substring of the
  id, so a substring match is always an id match. A vendor clause on top of a full-id clause
  would be dead code, and no test could prove it. This is why the filter matches the model
  name, not the full id.
- A row that shows the full id and appends the vendor. The vendor would repeat the id prefix
  and add no information, and the vendor filter clause would still be dead.
- A raw vendor label or a raw model name on the terminal. An escape sequence in an id could
  move the cursor. The width clip does not strip an escape, so both columns are sanitized on
  draw.
- A second filter in the draw function. `model_picker_panel` reads `filtered_indices`, so
  one filter feeds both the budget and the draw.
- A match rule that stays split from the older decision. This decision amends the match rule
  of `D-model-picker-allows-fuzzy-search-and-typed-fallback`.

## Why

The user wants to group models by vendor and to type a vendor to filter. A derived vendor
needs no new state and no descriptor change. Splitting the vendor out of the shown name is
what makes both the label and the vendor filter carry real information. It is also the only
form that a test can prove, because a full-id match would already cover the vendor.
