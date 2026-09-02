# D-model-picker-allows-fuzzy-search-and-typed-fallback

Date: 20260902-000309

## The question

`D-the-model-picker-is-a-panel` said the picker has no text field: no filter, no query.
The first user then asked how to reach a starred id in a list of twenty. Typing is the
answer, and a typed id also needs to reach the wire even when the list omits it, per
`D-a-listing-failure-never-stops-a-session`.

What matches, what does `Enter` do, and which keys change?

## The decision

The panel gains one string, `query`, shown at the top as `> <query>`. Rows filter as the
user types.

- **Match rule.** Case-insensitive subsequence: every character of `query` appears in the
  row's id in order, not necessarily contiguous. `sn45` matches `claude-sonnet-4-5`. This
  is the fzf shape a user expects when they read the word "fuzzy".
- **Order.** The filtered rows keep the picker's original order: the current model first,
  starred rows after. rho does not rank matches by score. A ranked list would move the
  current row on every keystroke, and the current row is the one anchor.
- **Selection.** `selected` indexes into the **filtered** rows. `↑ ↓` clamp to that range.
- **Typed fallback.** `Enter` on a query that matches no row applies the query verbatim
  as the model id, with a `model set to <query>` notice. `Enter` on a query with matches
  applies the highlighted match. A typed id always reaches the provider, per
  `D-a-listing-failure-never-stops-a-session`.
- **Keys that change.** `j`, `k`, `e`, and `*` are legitimate characters in model ids
  (`gpt-4o-mini`, `claude-sonnet-4-5`, `openai/gpt-oss`). They stop being panel keys and
  become query characters. The panel keeps `↑ ↓ Enter Esc Backspace`, and takes over:
  - `Tab` cycles the highlighted row's preview effort. (Was `e`.)
  - `Shift+Tab` (`BackTab`) toggles the highlighted row's star. (Was `*`.)
  - `Backspace` removes the last query character. When the query is empty, `Backspace`
    is ignored; the panel is closed with `Esc`.

## What this rules out

- A ranked list that reorders on every keystroke. The anchor row is the current model,
  and reordering it would be worse than no filter.
- A separate "search" mode that the user enters explicitly. Every typed character
  filters at once, so a first user who types a query never wonders "am I in a text
  field?".
- A key handler that swallows `j` because `j` used to move down. A model id with `j` in
  it (openrouter has several) would be unreachable.
- `Enter` that refuses a query with no match. rho already accepts a typed id the list
  omits, and this must not change.
- Sorting matches by score. The picker is short, and the anchor at row zero is more
  useful than a score column that the user learns to ignore.

## Why

The picker draws few rows and its rows carry no other action, so a text field costs one
row. Subsequence match reads as "fuzzy" to a user who has used fzf, ripgrep with `--fuzzy`,
or VS Code's Ctrl-P. The typed-fallback rule is the guarantee
`D-a-listing-failure-never-stops-a-session` already made, moved into the picker.
