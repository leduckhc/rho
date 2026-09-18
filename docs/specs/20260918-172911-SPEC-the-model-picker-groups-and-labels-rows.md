# SPEC-the-model-picker-groups-and-labels-rows — group and label the picker rows

Status: delivered.
Owning crate: `rho-tui`.

Feature: `F-model-picker`. This spec extends that feature. It does not add a new one.

## The problem

A user wants the starred models in a real section at the top, a dim vendor name on each
row, a fuzzy filter that matches the vendor, and Tab to change the reasoning effort.

## The sides

One crate owns every side here: `rho-tui`. The sides that must agree are:

- The state in `crates/rho-tui/src/state.rs`. It builds the rows and filters them.
- The renderer in `crates/rho-tui/src/render.rs`. It draws the rows and the headers.
- The panel budget in `crates/rho-tui/src/render.rs`. It reserves the header lines.
- The crate boundary in `crates/rho-tui/src/lib.rs`. It re-exports the public items.

The crate boundary is a real side. The tests import from the crate root only. So the
`pub use state::{...}` block at `crates/rho-tui/src/lib.rs` must add `PickerSections`,
`model_label`, and `vendor_label`. The `pub use render::{...}` block must add
`picker_viewport_rows`, so the budget test can call it. Without these lines the named tests
do not resolve.

The contract kinds this change touches are the data model, the public API, and the
behaviour rules. It touches no wire format, no persisted format, and no config key.

## The scope decision

This spec ships the reasoning effort as the only per-row option. Tab cycles it. The spec
adds section headers, a vendor label, and vendor fuzzy search. It builds no other option
dimension. See `D-effort-is-the-only-row-option`.

## 1. Contract

The new and changed items are below, as compilable Rust. The existing `PickerRow` and
`ModelPicker` fields do not change.

`display_rows` and `sections` now share one private constructor, `build_display`. One
build produces both the rows and the section counts, so the two can never drift. This
answers the drift risk that a second counting pass would carry.

```rust
// crates/rho-tui/src/state.rs

/// The three sections of the picker, in display order.
///
/// `build_display` orders the rows as current, then starred, then catalog. These counts
/// name the boundary between the three. The renderer draws a header from them, so a header
/// needs no row of its own and can never take the selection. See
/// `D-the-picker-draws-a-starred-section`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PickerSections {
    /// The leading display rows that are the current model. It is 0 or 1.
    pub current: usize,
    /// The display rows of the starred section, after the current model.
    pub starred: usize,
    /// The display rows of the catalog section, after the starred section.
    pub catalog: usize,
}

impl ModelPicker {
    /// Build the display rows once, and count each section in the same pass.
    ///
    /// This is the one source of the display order. `display_rows` and `sections` both
    /// read it, so a change to the order updates both. The counting matches the pushes,
    /// row for row.
    fn build_display(&self, starred: &[String]) -> (Vec<PickerRow>, PickerSections) {
        let mut display = Vec::new();
        let starred_set: std::collections::HashSet<&str> =
            starred.iter().map(|id| id.as_str()).collect();
        let row_by_id: std::collections::HashMap<&str, &PickerRow> =
            self.rows.iter().map(|row| (row.id.as_str(), row)).collect();
        let current = self.rows.iter().find(|row| row.is_current).cloned();
        let current_count = usize::from(current.is_some());
        if let Some(current) = &current {
            display.push(current.clone());
        }
        let mut starred_seen = std::collections::HashSet::new();
        let mut starred_count = 0usize;
        for id in starred {
            if current.as_ref().map(|row| &row.id) == Some(id) {
                continue;
            }
            if !starred_seen.insert(id.clone()) {
                continue;
            }
            if let Some(row) = row_by_id.get(id.as_str()) {
                let mut dup = (*row).clone();
                dup.starred = true;
                dup.is_current = false;
                display.push(dup);
            } else {
                display.push(PickerRow {
                    id: id.clone(),
                    is_current: false,
                    starred: true,
                    effort: None,
                    stale: false,
                    catalog: false,
                });
            }
            starred_count += 1;
        }
        let mut catalog_count = 0usize;
        for row in &self.rows {
            if row.is_current || !row.catalog {
                continue;
            }
            let mut shown = row.clone();
            shown.starred = starred_set.contains(row.id.as_str());
            shown.is_current = false;
            display.push(shown);
            catalog_count += 1;
        }
        let sections = PickerSections {
            current: current_count,
            starred: starred_count,
            catalog: catalog_count,
        };
        (display, sections)
    }

    /// The rows as they appear on screen: current, then starred, then catalog.
    pub fn display_rows(&self, starred: &[String]) -> Vec<PickerRow> {
        self.build_display(starred).0
    }

    /// The row counts of the three display sections, in display order.
    ///
    /// The sum equals `self.display_rows(starred).len()`, because both come from one build.
    pub fn sections(&self, starred: &[String]) -> PickerSections {
        self.build_display(starred).1
    }

    /// The number of section header lines the picker draws for the current filter.
    ///
    /// It is 0, 1, or 2. The starred section and the catalog section each add one header
    /// when the filter keeps at least one of that section's rows. The current model row
    /// never draws a header. Every budget site reserves this count.
    pub fn header_rows(&self, starred: &[String]) -> usize {
        let sec = self.sections(starred);
        let starred_end = sec.current + sec.starred;
        let mut starred_has = false;
        let mut catalog_has = false;
        for display_index in self.filtered_indices(starred) {
            if display_index >= sec.current && display_index < starred_end {
                starred_has = true;
            } else if display_index >= starred_end {
                catalog_has = true;
            }
        }
        usize::from(starred_has) + usize::from(catalog_has)
    }

    /// The indices of the displayed rows that pass the fuzzy filter.
    ///
    /// An empty query returns every display index. A non-empty query keeps a row when the
    /// query matches the shown model name or the vendor label. It no longer matches the
    /// full id, because the vendor part is now shown and matched on its own. See
    /// `D-a-picker-row-labels-its-vendor`.
    pub fn filtered_indices(&self, starred: &[String]) -> Vec<usize> {
        let display = self.display_rows(starred);
        if self.query.is_empty() {
            return (0..display.len()).collect();
        }
        display
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                fuzzy_match(&self.query, model_label(&row.id))
                    || fuzzy_match(&self.query, vendor_label(&row.id))
            })
            .map(|(index, _)| index)
            .collect()
    }
}

/// The vendor prefix of a model id, for a dim label and for the fuzzy filter.
///
/// The result borrows from `model_id`, so it allocates nothing. A model id is untrusted,
/// so the renderer passes the result through `sanitize_line` before it draws. An empty
/// return means the id has no vendor prefix. See `D-a-picker-row-labels-its-vendor`.
pub fn vendor_label(model_id: &str) -> &str {
    let id = model_id.trim();
    if id.is_empty() {
        return "";
    }
    if let Some(index) = id.find('/') {
        return &id[..index];
    }
    if id.contains('.') {
        let first_dot = id.find('.').unwrap();
        let first = &id[..first_dot];
        if is_region_prefix(first) {
            let rest = &id[first_dot + 1..];
            let end = rest.find('.').unwrap_or(rest.len());
            return &rest[..end];
        }
        return first;
    }
    ""
}

/// The model name of a model id, with the vendor prefix and any region prefix removed.
///
/// This is the name the row shows and the fuzzy filter matches. The full id is what
/// `Enter` applies, so this transform never reaches the provider. The id is untrusted, so
/// the renderer passes this result through `sanitize_line` too. The result borrows from
/// `model_id`. See `D-a-picker-row-labels-its-vendor`.
pub fn model_label(model_id: &str) -> &str {
    let id = model_id.trim();
    if id.is_empty() {
        return "";
    }
    if let Some(index) = id.find('/') {
        return &id[index + 1..];
    }
    if id.contains('.') {
        let first_dot = id.find('.').unwrap();
        let first = &id[..first_dot];
        if is_region_prefix(first) {
            if let Some(second_dot) = id[first_dot + 1..].find('.') {
                return &id[first_dot + 1 + second_dot + 1..];
            }
            return "";
        }
        return &id[first_dot + 1..];
    }
    id
}

/// True when a dot segment is a Bedrock cross-region prefix, such as `us` in
/// `us.anthropic.claude-3-5-sonnet`. This is a display heuristic. It is never a wire value.
fn is_region_prefix(segment: &str) -> bool {
    matches!(
        segment,
        "us" | "eu" | "apac" | "ap" | "ca" | "sa" | "me" | "af" | "us-gov"
    )
}
```

### 1a. The renderer body changes

The private signatures do not change. Their bodies change. Three sites must agree, and one
filter feeds all three.

`filtered_indices` is the one filter. `model_picker_panel` must build its visible set from
`picker.filtered_indices(starred)`. It must not keep its own copy of the predicate. Today
it filters inline with `fuzzy_match(&picker.query, &row.id)`. That line is deleted. The old
full-id filter is a superset of the new one, so a stale copy would draw more rows than the
budget reserved, and the scroll footer total would disagree with the budget.

```rust
// crates/rho-tui/src/render.rs

// (1) model_picker_panel builds the visible set from the single filter.
//   delete:  picker.query.is_empty() || fuzzy_match(&picker.query, &row.id)
//   use:     let filtered = picker.filtered_indices(starred);

// (2) model_picker_panel reserves the header lines in its own budget too.
//   let header_rows = picker.header_rows(starred);
//   let fixed_lines = lines.len() + 1 + header_rows; // footer + section headers

// (3) model_picker_panel draws the section headers, dim, from PickerSections.
//   Before the first visible row of the starred section, draw "  starred".
//   Before the first visible row of the catalog section, draw "  models".

// (4) model_picker_panel sanitizes both text columns before it draws.
//   name:   sanitize_line(model_label(&row.id))
//   vendor: sanitize_line(vendor_label(&row.id))

// (5) picker_viewport_rows subtracts the header lines.
//   old: layout.panel_rows.saturating_sub(3 + status_rows)
//   new: layout.panel_rows.saturating_sub(3 + status_rows + header_rows)
//     where header_rows = picker.header_rows(&state.starred_models)

// (6) panel_demand adds the header lines to the picker demand.
//   old: (visible_rows + 2 + footer_rows + status_rows, 0)
//   new: (visible_rows + 2 + footer_rows + status_rows + header_rows, 0)
//     where header_rows = picker.header_rows(&state.starred_models)
```

## 2. The vendor derivation rule

`vendor_label` reads the id from left to right:

1. Trim the id. An empty id returns an empty label.
2. When the id has a `/`, the vendor is the text before the first `/`.
3. When the id has no `/` but has a `.`, split on the dots.
4. When the first dot segment is a region prefix, the vendor is the next segment.
5. When the first dot segment is not a region prefix, the vendor is that first segment.
6. When the id has no `/` and no `.`, the label is empty.

`model_label` follows the same shape. It returns the text after the vendor separator. For
a region id it drops the region and the vendor both. For a vendorless id it returns the
whole id.

| input | vendor_label | model_label | why |
| --- | --- | --- | --- |
| `anthropic/claude-3.5-sonnet` | `anthropic` | `claude-3.5-sonnet` | slash id |
| `openai/gpt-4o-mini` | `openai` | `gpt-4o-mini` | slash id |
| `anthropic.claude-3-5-sonnet-20241022-v2:0` | `anthropic` | `claude-3-5-sonnet-20241022-v2:0` | dot id |
| `us.anthropic.claude-3-5-sonnet` | `anthropic` | `claude-3-5-sonnet` | region prefix skipped |
| `gpt-4o` | *(empty)* | `gpt-4o` | no vendor prefix |
| *(empty)* | *(empty)* | *(empty)* | empty id |
| `/` | *(empty)* | *(empty)* | only a separator |
| `.` | *(empty)* | *(empty)* | only a separator |
| `foo.bar.baz` | `foo` | `bar.baz` | several separators |
| `anthropic\u{1b}[31m/claude` | `anthropic\u{1b}[31m` | `claude` | unsafe vendor, sanitized on draw |

The last row returns the raw bytes. The renderer drops the escape sequence with
`sanitize_line`, so the drawn vendor is `anthropic`. See section 4.

## 3. The section headers

`build_display` orders the rows as current, then starred, then catalog. The headers make
that order visible. The renderer draws two header lines:

- A `starred` header, before the first filtered row of the starred section.
- A `models` header, before the first filtered row of the catalog section.

The current model row draws no header. Its `(current)` tag names it. A header draws only
when its section keeps at least one row after the filter. Both headers draw dim.

A header is render-only. `selected` and `scroll_offset` index the filtered rows, never a
header. So a user can never highlight a header. See `D-the-picker-draws-a-starred-section`.

The scroll window applies to model rows only. A header draws inside the window, before its
section's first row, when that row is visible. Three sites reserve `header_rows` lines:
`picker_viewport_rows`, `panel_demand`, and `model_picker_panel`. The reservation is the
count of non-empty sections, even when a scroll hides a header. This reservation never
overruns the panel. It can leave one spare line when the list is scrolled.

## 4. The row layout

A row draws its parts left to right, in this order:

1. Two spaces of indent.
2. The star column, `★` when starred, `☆` when not.
3. One space, then the sanitized model name from `model_label`.
4. Two spaces, then the sanitized vendor label from `vendor_label`, drawn dim.
5. The effort suffix ` [effort=<level>]`, only when the row has a preview effort.
6. The ` (current)` tag, only for the current model.
7. The ` (stale)` tag, only for a stale cached row.

The vendor label is the first part to yield when the width runs out. The renderer measures
the whole row. When it does not fit, the renderer omits the vendor label and its two
spaces. The model name and the state tags stay. When the row still does not fit, the cell
clip in `put` truncates the right end, so a tag yields before the name.

An empty vendor label draws nothing. The two spaces before it draw nothing either. A
stray separator after a vendorless name is a defect. See
`D-a-picker-row-labels-its-vendor`.

Both text columns pass through `sanitize_line` before they draw. A model id is untrusted,
and `model_label` can hold an escape too, such as in `foo/\u{1b}[31mbar` or in a vendorless
`\u{1b}[31mgpt-4o`. The width clip in `put` does not strip an escape, because an escape has
zero display width. So the sanitize call, not the clip, is the escape guard. No untrusted
byte reaches the terminal raw from either column.

## 5. The effort option

Tab cycles the highlighted row's preview effort. The order is unchanged:
`None → Off → Low → Medium → High → XHigh → None`, through `next_effort_in_cycle`. The row
draws the effort as ` [effort=<level>]`, after the name and the vendor. A row with no
effort draws no suffix. Effort is the only per-row option. See
`D-effort-is-the-only-row-option`.

## 6. What the contract forbids

- A header that sits in `display_rows`. It would shift every index and become selectable.
- A `PickerRow` field for the vendor. A field would go stale in the places a row is built.
  The pure function reads the id every time.
- A second filter in `model_picker_panel`. The renderer must call `filtered_indices`, so
  the budget and the draw agree.
- A fuzzy clause that matches the full id. The vendor part would then match twice, and the
  vendor clause would be dead. See `D-a-picker-row-labels-its-vendor`.
- A raw model name or a raw vendor on the terminal. Both columns are sanitized.
- A second option dimension, such as a speed switch. See
  `D-effort-is-the-only-row-option`.
- A change to `ModelDescriptor`. The vendor comes from the id. See
  `D-a-model-descriptor-carries-no-capability-claim`.

## 7. Errors and known limits

- An id with an escape sequence in either column: `sanitize_line` drops the sequence whole.
- A degenerate id such as `/` or `.`: the vendor label and the model name are both empty.
  The row then falls back to the full id. So `/` draws `☆ /`, and a row never draws a bare
  star. `Enter` applies the same full id. See `a_region_only_id_never_draws_an_empty_name`.
- **A crafted id can forge a state tag. This is not fixed.** rho draws ` (current)` and
  ` (stale)` after the name. `sanitize_line` keeps a space and a bracket, so an id may end
  with the same text. Measured: the id `gpt-4o (current)` draws `☆ gpt-4o (current)`, and the
  real current row draws `☆ seed  openai (current)`. Text alone cannot tell them apart.
  The harm is confusion, not access, because `Enter` applies the id the row names. A fix
  needs the tag in a reserved right-aligned slot, where untrusted text cannot reach. That
  slot changes the row arithmetic this spec settled, so it needs its own decision. See
  `F-duration-slot` for the reserved-slot pattern this project already uses.
- A full-id query no longer matches its own row. A user who types a full slash id gets no
  match, so `Enter` takes the typed-fallback path at `handle_model_picker_key`. That path
  applies `self.reasoning_effort`, not a row's previewed effort. A Tab preview is then lost.
  This spec keeps that path as is. A user sets the effort with `/effort` or with Tab on a
  matched row. A change to carry a preview onto the typed-fallback path is out of scope, and
  it would need a new decision, because it adds state the picker does not hold today.

## 8. The amended decision

This spec narrows the match rule. `D-model-picker-allows-fuzzy-search-and-typed-fallback`
says the query matches the row's id, with the example `sn45` matches `claude-sonnet-4-5`.
The new rule matches the model name or the vendor, not the full id.

This spec and `D-a-picker-row-labels-its-vendor` amend that older decision's match rule.
The controller owns the older decision file. The proposed new wording for its `Match rule`
bullet is: "Case-insensitive subsequence against the shown model name or the vendor label.
The full id is what `Enter` applies. See `D-a-picker-row-labels-its-vendor`." The example
`sn45` still matches, because `sn45` is a subsequence of the model name `sonnet-4-5`.

## 9. Test cases

Each test lives in this repo when the status flips to `delivered`. Each row names the file
and the assertion.

### `rho-tui` — the vendor and model labels

- `vendor_label_reads_the_slash_prefix` — `crates/rho-tui/tests/model_picker.rs`.
  `anthropic/claude-3.5-sonnet` gives `anthropic`.
- `vendor_label_reads_the_dot_prefix` — `crates/rho-tui/tests/model_picker.rs`.
  `anthropic.claude-3-5-sonnet-20241022-v2:0` gives `anthropic`.
- `vendor_label_skips_a_bedrock_region_prefix` — `crates/rho-tui/tests/model_picker.rs`.
  `us.anthropic.claude-3-5-sonnet` gives `anthropic`.
- `vendor_label_is_empty_without_a_vendor` — `crates/rho-tui/tests/model_picker.rs`.
  `gpt-4o`, an empty id, `/`, and `.` all give an empty label.
- `vendor_label_returns_the_raw_unsafe_segment` — `crates/rho-tui/tests/model_picker.rs`.
  An id with an escape sequence returns the raw bytes, so the renderer sanitizes them.
- `model_label_strips_the_vendor_and_region` — `crates/rho-tui/tests/model_picker.rs`.
  The model name drops the vendor and any region prefix.

### `rho-tui` — the fuzzy filter

- `filtered_indices_matches_the_vendor_label` — `crates/rho-tui/tests/model_picker.rs`.
  The query `anthropic` keeps an anthropic row whose model name lacks the query.
- `filtered_indices_matches_the_model_name` — `crates/rho-tui/tests/model_picker.rs`.
  The query `sonnet` keeps the sonnet row and drops the others.
- `filtered_indices_drops_a_full_id_only_match` — `crates/rho-tui/tests/model_picker.rs`.
  A query that is a subsequence of the full id, but of neither the model name nor the
  vendor, returns an empty result. A revert to a full-id match makes this test fail.

### `rho-tui` — the sections and the budget

- `sections_pins_each_section_boundary` — `crates/rho-tui/tests/model_picker.rs`.
  Build the display rows. Assert the first `current` rows are the current model. Assert the
  next `starred` rows are the starred ids in order. Assert the rest are the catalog rows.
  A row that crosses the boundary while the sum holds still fails this test.
- `header_rows_counts_only_non_empty_sections` — `crates/rho-tui/tests/model_picker.rs`.
  A filter that empties the catalog drops its header.

### `rho-tui` — the renderer

- `a_header_is_never_the_highlighted_row` — `crates/rho-tui/tests/render.rs`.
  Drive the arrow keys through a picker with both sections. Render each step. Assert the
  reversed line is a model row, never the `starred` or `models` header.
- `the_drawn_rows_match_filtered_indices_for_a_divergent_query` —
  `crates/rho-tui/tests/render.rs`. Type a query the full id matches but the model name and
  the vendor do not. Assert the drawn rows and the highlight match `filtered_indices`.
- `a_starred_header_draws_above_the_starred_rows` — `crates/rho-tui/tests/render.rs`.
  The `starred` header draws before the first starred row.
- `a_models_header_draws_above_the_catalog_rows` — `crates/rho-tui/tests/render.rs`.
  The `models` header draws before the first catalog row.
- `a_row_draws_the_name_then_a_dim_vendor` — `crates/rho-tui/tests/render.rs`.
  The vendor draws dim, after the model name.
- `a_row_draws_the_effort_suffix_and_none_omits_it` — `crates/rho-tui/tests/render.rs`.
  A row with a preview effort draws ` [effort=<level>]` after the vendor. A row with no
  effort draws no suffix.
- `an_empty_vendor_draws_no_separator` — `crates/rho-tui/tests/render.rs`.
  A vendorless id draws no trailing separator.
- `a_narrow_row_drops_the_vendor_first` — `crates/rho-tui/tests/render.rs`.
  A narrow width omits the vendor and keeps the model name.
- `the_vendor_is_sanitized_before_it_draws` — `crates/rho-tui/tests/render.rs`.
  An escape sequence in the vendor is dropped whole.
- `the_model_name_is_sanitized_before_it_draws` — `crates/rho-tui/tests/render.rs`.
  An escape in the model name, in a vendorless id and after a separator, is dropped whole.
- `the_headers_reduce_the_model_row_budget` — `crates/rho-tui/tests/render.rs`.
  The public `picker_viewport_rows` returns fewer rows when a header shows.
- `two_headers_and_a_full_catalog_fit_the_panel_budget` —
  `crates/rho-tui/tests/render.rs`. Open a picker with a current model, a starred row, and
  enough catalog rows to fill the cap, at a height where both headers plus the cap fit.
  Assert both headers draw. Assert the model rows drawn equal the `picker_viewport_rows`
  count. Assert no line falls beyond the panel band.

### `rho-tui` — the renderer, tests added after the first review

These guard defects a four-lens review and a mutation pass found after the first build.
Each one was proven to fail against the defect it names.

- `the_picker_keeps_a_header_and_fills_its_grant_at_every_scroll_offset` —
  `crates/rho-tui/tests/render.rs`. Drive the picker down through both sections. At every
  offset assert the drawn row count equals `picker_viewport_rows`, the footer row does not
  move, and a section header still shows. Without sticky headers the panel returned fewer
  lines than its grant, so the composer and the footer jumped up by up to two rows.
- `a_control_sequence_in_a_picker_error_cannot_reach_the_screen` —
  `crates/rho-tui/tests/render.rs`. A provider failure message is untrusted. Assert the
  escape body is absent, not only the escape byte.
- `a_region_only_id_never_draws_an_empty_name` — `crates/rho-tui/tests/render.rs`.
  The id `us.anthropic` strips to an empty model name. The row falls back to the full id,
  because the full id is what `Enter` applies.
- `the_vendor_separator_is_exactly_two_spaces` — `crates/rho-tui/tests/render.rs`.
  The drawn gap between the name and the vendor is exactly two spaces.
- `the_vendor_separator_width_is_measured_as_it_is_drawn` —
  `crates/rho-tui/tests/render.rs`. The row measures the separator as `2 + vendor.width()`
  and draws two spaces. Those are two constants that must agree. Assert the boundary from
  both sides: at 21 columns the vendor fits and draws, and at 20 columns the row is exactly
  `☆ claude`. A measurement of `3` drops the vendor early. A measurement of `1` overruns the
  frame, and the backend clips the last character, so an assertion on the vendor word alone
  does not catch it.

**On the two sanitisation tests.** `the_vendor_is_sanitized_before_it_draws` and
`the_model_name_is_sanitized_before_it_draws` first asserted only that no escape byte
reached the buffer. `TestBackend` drops a bare escape byte on its own, so both tests passed
with `sanitize_line` removed. A mutation pass proved it. Both now assert the escape body is
absent, and both were re-proven against the removal.

The pty drive `bench/tui_model_picker_drive.py` gained assertions. It checks the drawn
`starred` header and a dim vendor on a real suggestion row. The drive is a script, not a
Rust test, so it is not named here as a test function.

## 10. Out of scope

- **The multi-dimension option UI.** No arrow-key option selector, no speed switch, and no
  context-size switch. Arrow keys keep moving the row selection. See
  `D-effort-is-the-only-row-option`.
- **Cross-provider listing.** The picker still shows one provider's models. A provider
  switch stays out. See `SPEC-model-selection-in-tui`.
- **A change to `ModelDescriptor`.** The vendor comes from the id. The descriptor keeps
  `id` and `display_name` only. See `D-a-model-descriptor-carries-no-capability-claim`.
- **Ranking matches by score.** The row order stays current, starred, catalog. See
  `D-model-picker-allows-fuzzy-search-and-typed-fallback`.
- **Carrying a Tab preview onto the typed-fallback path.** See section 7.
