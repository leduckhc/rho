# SPEC-tui-markdown — colour the markup, and keep the row a grid

Status: **phase 1, phase 2, and tables all shipped.** Syntax highlighting stays out of scope.

The contract went to review before either side was written. It came back `REVISE`, and this
spec now carries the revisions. Phase 1, the line-level subset, is built and tested, because it
needs no contract change. Phase 2, inline emphasis, needs the run-level contract in section 3
and must not start until the items in section 3a are settled. See
`D-markdown-line-level-first`.

## 1. Why

rho draws a model answer as plain text, so `**bold**`, `` `code` ``, and a `# heading` reach
the screen as their own punctuation. The owner reads code all day and asked for colour
instead of markup. jcode and pi both do this, and both were read from source.

`D-block-text-keeps-its-shape` already made a newline and an indent survive. This is the next
step, and it is a smaller one than it looks.

## 2. The prior art, read from source

**pi** (`pi-tui/dist/components/markdown.js`, `theme/theme.js:1028`) parses with `marked`
and maps tokens onto ten named roles: `mdHeading`, `mdLink`, `mdLinkUrl`, `mdCode`,
`mdCodeBlock`, `mdCodeBlockBorder`, `mdQuote`, `mdQuoteBorder`, `mdHr`, `mdListBullet`. Bold,
italic, underline, and strikethrough stay modifiers, not colours. A level 1 heading is bold
and underlined. A fence keeps its ``` markers, drawn in a border colour.

**pi refuses to guess a language.** `getMarkdownTheme` calls `supportsLanguage(lang)` first,
and its comment says auto-detection "can misidentify prose as AppleScript, LiveCodeServer,
etc., coloring random English words as keywords." We take that lesson whole.

**jcode** (`crates/jcode-tui-markdown`, 7140 lines) uses `pulldown-cmark` for parsing and
`syntect` for highlighting, and emits ratatui `Line` values. It also carries latex, mermaid,
and an incremental renderer. That is far more than this spec wants.

## 2a. The two phases, and why the line falls there

**Phase 1, shipped.** A heading, a fence, a code line, a quote, a list marker, and a rule.
Each styles a whole row, so its markup can be stripped **before** the text is wrapped and one
style covers every row the line produces. No contract change.

**Phase 2, shipped.** `**bold**`, `*italic*`, `***both***`, and `` `code` ``. Each styles part
of a row, which needed the row type to carry runs.

The split is where the renderer stops being able to express the answer, not a guess at effort.

## 3. The contract, shipped

**The sides.** The row producers in `rho-tui::render` own one side. The frame writer, `put`,
owns the other. Today they agree on one style for a whole row, so inline colour is not
expressible at all. That is the contract that has to change first.

```rust
/// One drawn row: a sequence of styled runs, left to right.
///
/// A row was `(String, Style)`, so a style covered a whole row and `**bold**` could not be
/// bold. A run is the smallest unit that carries its own style.
pub type StyledLine = Vec<(String, Style)>;

/// One run with one style, for a row that needs no inline change.
pub fn plain(text: String, style: Style) -> StyledLine;

/// The display width of a row, summed over its runs.
pub fn line_width(line: &StyledLine) -> usize;
```

`put` takes a `&StyledLine` and writes each run in order. Every producer that has no inline
styling calls `plain`, so the change is mechanical for 29 of the 30 sites.

## 3a. What review required, and how each item was answered

Item 6 is new, and it comes from measuring pi and jcode side by side rather than reading them.
See `docs/verification/notices-live.md`.

The reviewer rated the first four `High` or `Medium-High`. None may be skipped.

1. **Done.** The pipeline is scan, then wrap over runs, then pad in `put`. The first draft of this spec
   had the order wrong: wrapping ran on raw text, so removing `**` afterwards left a row four
   columns short of the wrap's own measure. The claim "markup never changes width by accident"
   was deleted, because markup removal **always** changes width.
2. **Answered by placing the invariant at the consumer.** `StyledLine` stays a list of runs,
   and `put` is the single place that clips a run at the right edge and pads a short row. One
   place enforces it in code, for every producer including a future one. A struct would spread
   the rule across 30 call sites and still depend on each of them calling it.
3. **Done, and rho goes further than CommonMark in two places.** The flanking rule is in, plus
   backslash escapes, multi-backtick spans, and `***triple***`. Beyond CommonMark: **an
   underscore never carries emphasis**, so `__init__` and `snake_case` survive, and **an
   intraword star never opens**, so `2*3*4` survives. Both are deliberate: in a coding agent's
   prose those are identifiers and arithmetic, not emphasis.
4. **Fence state spans the whole message**, never the visible window. Phase 1 already does
   this, and phase 2 must keep it.
5. **A table and a nested list degrade to verbatim.** Phase 1 does this, with a test.
6. **Deferred, and stated.** Inline code takes a foreground colour only, as pi does. jcode draws inline code as
   RGB 140,180,255 on RGB 45,45,45, and measured beside a foreground-only change it reads more
   clearly. `RoleStyle` has `color`, `dim`, `bold`, and `reversed`, and **no background field**.
   So an inline code background needs the role table extended, and every role must then state a
   background or the exhaustive match will not compile. Decide that before phase 2 starts, not
   during it.

**The parser is not a dependency.** rho reads a small, fixed subset with its own scanner. No
`pulldown-cmark` and no `syntect`. Reasons: rho ships a binary whose size and start time are
features, the subset below is a few hundred lines, and a general parser invites the scope
jcode took. This is a decision, not a preference, and `D-markdown-line-level-first` states
what it rules out.

### The subset, and what each part becomes

| Markup | Drawn as | Role |
| --- | --- | --- |
| `# h1` to `###### h6` | the text, no hashes, bold | `MdHeading` |
| a table, a nested list | verbatim, never partly styled | `Text` |
| `**bold**`, `__bold__` | the text, no markers, bold | current role, bold |
| `*italic*`, `_italic_` | the text, no markers, italic | current role, italic |
| `` `code` `` | the text, no backticks | `MdCode` |
| a fenced block | each line, verbatim, never re-wrapped | `MdCodeBlock` |
| the fence line itself | the fence and its language | `Muted` |
| `- item`, `* item`, `+ item` | `•` then the text | `Text` |
| `1. item` | the number kept, then the text | `Text` |
| `> quote` | `┃` then the text | `Muted` |
| `---`, `***`, `___` | a full-width rule | `Muted` |

**Two roles are new, not six.** The first draft minted six `Md*` roles, and review called that
vocabulary pollution: a role couples one feature to a shared enum, and every role forces edits
to `Role`, `Role::ALL`, and three mapping functions.

Shipped: `MdHeading`, because no role carries a colour with the bold weight, and `MdCodeBlock`,
because no role reads as "not prose". A fence and a quote reuse `Muted`. A list item reuses `Text`,
matching pi, which leaves item text at the body colour. Phase 2 adds `MdCode` only if `MdCodeBlock` proves wrong for an inline span.

`style_for` also stopped naming a role. It read `if role == Role::Caution`, so every bold role
needed an edit to shared code. It now reads the weight from `role_16`, which the exhaustive
match already forces every role to state.

### The rules

- **Inside a fence, nothing is markup.** A `*` in code is a star. The scanner tracks fence
  state per row, so a code block cannot be reinterpreted.
- **An unclosed marker is text.** `2 * 3 * 4` is arithmetic, and `a_b_c` is an identifier. A
  marker matches only when its pair is on the same row, and `_` inside a word never matches.
- **Markup never changes the text's width by accident.** A row's runs sum to the same columns
  the text occupies, so `line_width` stays the source of truth for wrapping and padding.
- **No syntax highlighting.** No language is guessed and none is coloured. pi's comment is
  the reason, and rho has no `syntect`.
- **No link rewriting.** A URL stays visible. No OSC 8 hyperlink, because a link that hides
  its target is a phishing surface in a terminal.
- **The escape filter is untouched.** `sanitize_block` still runs first, so this styles text
  that is already safe. Markup styling never re-admits an escape.
- **A user row is not markdown.** rho draws what the user typed, verbatim.

## 3b. Tables

A model emits a table in most answers, so this is not an edge case. rho follows jcode: aligned
columns, a bold header, a rule row, and a box-drawing divider. pi draws full box borders, which
is heavier than the rest of this interface.

A table is the one construct that spans several lines, so it is recognised in `scan_markdown`,
which already reads a whole message.

**The rules.**

- A table needs a header row **and** an alignment rule under it. Two pipe lines alone are not a
  table and stay verbatim. Half a table drawn is worse than none.
- The outer pipes are optional, because a model often leaves them off.
- A column's width is the widest cell, header included.
- `:---` is left, `---:` is right, `:---:` is centred. A number column reads wrong left aligned.
- **A cell's inline markers come off before its width is measured.** Measuring `**bold**` and
  removing the stars later would shift every column to its right. The cost is that emphasis in a
  cell is dropped rather than styled, because a row here is one string and cannot carry runs.
- A ragged row is padded, never dropped, or data disappears.
- A table row is never wrapped. It is already aligned, and wrapping would stack the columns into
  nonsense. `put` cuts it at the screen edge.
- A table inside a fence is code.

Three kinds carry it: `TableHead`, `TableRule`, and `TableRow`. The header takes `MdBold`, the
rule takes `Muted`, and a row takes `Text`.

## 4. Out of scope

Nested lists deeper than one level. Footnotes. Reference links. HTML blocks. Latex.
Mermaid. Syntax highlighting. Images. Strikethrough. Emphasis inside a table cell. A per-language theme. Incremental
reparsing: the row is re-scanned each frame, and section 5 bounds that cost.

## 5. The cost budget

The scanner runs per visible row, per frame, over at most `TEXT_MEASURE_CAP` columns. It must
add no allocation for a row with no markup: the common case returns one run borrowing the
whole line. `bench/tui_frame.py` must show no regression beyond noise, and the number goes in
`docs/benchmarks.md` with its command.

## 6. Test cases

### The scanner

- `a_row_with_no_markup_returns_one_run`
- `a_heading_drops_its_hashes_and_takes_the_heading_role`
- `bold_markers_are_removed_and_the_run_is_bold`
- `italic_markers_are_removed_and_the_run_is_italic`
- `inline_code_takes_the_code_role_without_backticks`
- `a_fence_line_takes_the_fence_role`
- `a_line_inside_a_fence_takes_the_code_block_role`
- `a_star_inside_a_fence_is_not_italic`
- `an_unclosed_marker_stays_text`
- `arithmetic_is_not_italic` — `2 * 3 * 4`
- `an_underscore_inside_a_word_is_not_italic` — `wrap_block`
- `a_bullet_keeps_its_text_and_colours_only_the_glyph`
- `a_quote_takes_the_quote_role`
- `a_rule_draws_full_width`

### The contract

- `a_styled_row_sums_to_the_text_width` — for every case above, `line_width` equals the
  visible width, so wrapping cannot drift.
- `every_role_has_all_three_mappings` — unchanged, and it must stay passing with the new
  roles.
- `an_escape_never_survives_markdown_styling` — the security guard, on one hostile string.
- `the_scanner_only_deletes_and_inserts_known_glyphs` — the same guard as a property, which
  the review asked for. Over a hostile corpus, every character the scanner emits is either in
  its input or one of two fixed glyphs. So the scanner cannot synthesise an escape at all,
  which is a stronger statement than any single example.
- `a_user_row_is_drawn_verbatim`

### The frames

- The ten fixtures in `docs/design/tui-frames/` compare symbols, not styles, so they must
  pass unchanged. If one changes, the scanner altered text it should only have coloured.
