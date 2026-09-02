# SPEC-model-selection-in-tui — pick a model mid-session

Status: delivered.
Owning crates: `rho-core` (the mutable slice and the API), `rho-tui` (the picker, the
slash commands, the starred file), `rho-cli` (the wiring).

Delivered by feature `F-model-picker`. Live drive in
`docs/verification/model-selection-in-tui.md`. Test count: `rho-core` selection tests (3),
`rho-tui` model_picker tests (13), `rho-tui` starred tests (3). Nine mutation proofs; see
the verification page for the table.

This spec closes the gap D-a-panel-nobody-can-open first named. `/model <arg>` used to
push a notice that said "not built yet", and rho had no way to change the model without a
restart. This spec builds:

1. `Session::selection` and `Session::set_selection`, so the model and the effort change
   mid-session and take effect on the next turn.
2. A `Panel::ModelPicker`, opened by `/model` alone, listing the current model and the
   starred list.
3. `/model <id>`, `/effort`, and `/effort <level>`, which apply directly and never open
   the picker.
4. A `~/.rho/starred-models.toml` file, owned by the TUI, that a user's stars persist to.

The decisions this spec obeys, in order:

- D-model-selection-is-mutable-behind-a-mutex
- D-starred-models-live-in-their-own-file
- D-the-model-picker-is-a-panel
- D-model-arg-bypasses-the-picker
- D-model-picker-allows-fuzzy-search-and-typed-fallback
- D-a-model-descriptor-carries-no-capability-claim
- D-a-listing-failure-never-stops-a-session

## 1. Contract

The public API, verbatim.

```rust
// crates/rho-core/src/agent.rs

/// The mutable slice of the session config.
///
/// A `ModelSelection` is what `Session::selection` reads and `set_selection` writes. The
/// running turn is not affected: `Driver::build_request` reads the pair at the start of
/// every turn, so the change takes effect at the next turn. See
/// `D-model-selection-is-mutable-behind-a-mutex`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelSelection {
    /// The provider-specific model id, exactly as `--model` accepts it.
    pub model: String,
    /// How hard the model should think. `None` means the provider's own default.
    pub reasoning_effort: Option<crate::ReasoningEffort>,
}

impl Session {
    /// Read the current model and effort. Cheap; the lock is uncontended.
    pub fn selection(&self) -> ModelSelection;

    /// Replace the current model and effort. Takes effect at the next turn boundary. The
    /// running turn is unaffected. Safe from any thread while a run is in flight.
    pub fn set_selection(&self, selection: ModelSelection);
}
```

```rust
// crates/rho-tui/src/state.rs

/// The model picker panel, opened by `/model` alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelPicker {
    /// The rows the picker draws, current first, starred after.
    pub rows: Vec<PickerRow>,
    /// The highlighted row index into the **filtered** rows.
    pub selected: usize,
    /// The fuzzy query. Empty means no filter. See
    /// `D-model-picker-allows-fuzzy-search-and-typed-fallback`.
    pub query: String,
}

impl ModelPicker {
    /// The indices of `rows` that pass the fuzzy filter, in original order.
    pub fn filtered_indices(&self) -> Vec<usize>;
}

/// Case-insensitive subsequence match: every character of `query` appears in `target`
/// in order, not necessarily contiguous. `sn45` matches `claude-sonnet-4-5`.
pub fn fuzzy_match(query: &str, target: &str) -> bool;

/// One row of the model picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerRow {
    /// The provider-scoped model id.
    pub id: String,
    /// True when this row is the current model of the session.
    pub is_current: bool,
    /// True when this row is in the starred file.
    pub starred: bool,
    /// The preview effort. `None` keeps the session's current effort.
    pub effort: Option<rho_core::ReasoningEffort>,
}

impl TuiState {
    /// Seed the starred list from the file. The frontend calls it at startup and on every
    /// picker open. See `D-starred-models-live-in-their-own-file`.
    pub fn set_starred_models(&mut self, starred: Vec<String>);

    /// Open the picker with the current model plus the starred list, dropping any star
    /// that equals the current model.
    pub fn open_model_picker(&mut self);
}

/// A file-owned starred list. The TUI owns every write.
pub mod starred {
    use std::path::Path;

    /// Read the starred list from `path`. A missing file returns an empty list, never an
    /// error. A parse error returns the reason so the caller can push one notice.
    pub fn load(path: &Path) -> Result<Vec<String>, String>;

    /// Write the starred list to `path`, atomically. Goes through a temp file plus rename.
    pub fn save(path: &Path, starred: &[String]) -> std::io::Result<()>;
}

/// Additions to the key-action enum, so the loop performs IO while the state stays pure.
#[non_exhaustive]
pub enum KeyAction {
    // ... existing variants ...
    /// Apply this model and effort to the session. Reaches `Session::set_selection`.
    ApplySelection(rho_core::ModelSelection),
    /// Persist this starred list to `~/.rho/starred-models.toml`.
    PersistStarred(Vec<String>),
}
```

Every method takes `&self` on `Session`, so a shared `Arc<Session>` (or a session held
inside `App`) can write the mutex from the app loop. See the decision.

## 2. Behaviour

- The picker opens only through `/model` with no argument. It never opens on any other
  key.
- The top row draws `> <query>`, or an instructive hint when the query is empty. Every
  filtered row shows the id, a star column (`★` starred, `☆` not), an optional
  `[effort=<name>]` when the row has a preview effort, and `(current)` at the end when it
  is the current model.
- **Fuzzy filter.** As the user types, rows filter by
  case-insensitive subsequence. `selected` indexes into the filtered rows only. See
  `D-model-picker-allows-fuzzy-search-and-typed-fallback`.
- `Enter` on a filtered row calls `Session::set_selection` with
  `ModelSelection { id, effort_override.or(session.selection.reasoning_effort) }` and
  closes the panel.
- **Typed fallback.** `Enter` when the filter matches nothing applies the query verbatim
  as the model id, and pushes a `model set to <query>` notice. A typed id always reaches
  the provider, per `D-a-listing-failure-never-stops-a-session`.
- `Tab` cycles the highlighted row's `effort` field. It never calls
  `Session::set_selection` on its own. The order is `None → Off → Low → Medium → High →
  XHigh → None`.
- `Shift+Tab` toggles the row's `starred` flag, updates the state's starred list, and
  returns `KeyAction::PersistStarred(list)`.
- `Backspace` removes the last query character and resets `selected` to zero. When the
  query is empty it is a no-op; `Esc` is what closes the panel.
- Any other printable character appends to the query and resets `selected` to zero. `j`,
  `k`, `e`, `*`, and every other letter type into the query.
- `esc` closes the panel and leaves the session's selection untouched, even if `Tab` was
  pressed.
- The panel never wraps: `↑` at row 0 stays at 0; `↓` at the last filtered row stays
  there.
- `/model <id>` sets the model to `<id>`, keeps the current effort, and pushes one notice
  `model set to <id>`. No picker.
- `/effort` alone pushes a notice `effort: <name>`, where `<name>` is one of `unset`,
  `off`, `low`, `medium`, `high`, `xhigh`.
- `/effort <level>` applies the level and pushes `effort: <level>`. `unset` clears it. An
  unknown level pushes an error row that names the valid levels; the state is not
  changed.
- A model change never rewrites the session file. The change lives in the session's
  in-memory selection only. See section 6.

## 3. Extension points

- New effort levels: add a variant to `ReasoningEffort`, and the cycle in `e` picks it up
  through the exhaustive match. No picker code changes.
- New picker sources (a `/model` that lists from a provider): a caller adds rows to
  `ModelPicker::rows` before opening the panel. `PickerRow::starred` and `is_current`
  survive. Listing is a later spec, per D-a-listing-failure-never-stops-a-session.
- Star sharing: the file is a plain TOML list of ids, so a user shares it by copying the
  file.

## 4. What the contract forbids

- A `Session::set_selection` that changes the provider. The provider is fixed for the life
  of the session.
- A `Session::set_selection` that mutates a running turn. `Driver::build_request` reads
  the mutex once, at the start of a turn, and the request that came before uses the value
  read then.
- A `PickerRow` with no id. An empty id would reach the provider and fail there.
- A `Panel::ModelPicker` that filters as the user types. The picker has no query field,
  and the state carries none.
- A star that carries an effort or a provider. The id is the star. See
  D-a-model-descriptor-carries-no-capability-claim.
- Reading the model from `SessionConfig::model` inside `build_request`. The one source of
  truth for the running model is the mutex.

## 5. Errors

- A starred file that will not parse: one notice row on the first read, then the picker
  opens with an empty starred list. rho does not overwrite the file.
- A starred file that cannot be written: one error row. The in-memory state keeps the new
  star, so a retry may work, but the persistence action tells the truth.
- A slash argument that fails to parse as `ReasoningEffort`: one error row, the valid
  levels named. The selection is not changed.

## 6. Out of scope

- **Provider switching mid-session.** A different provider needs new credentials, a new
  tool set, and a new system prompt. The provider stays fixed.
- **Listing from the provider.** rho does not add `Provider::list_models` in this spec.
  The picker's rows are, in order and deduped: the current model, the starred list, and
  a small per-provider suggestion list hard-coded in `rho-cli`. When listing exists, it
  appends to the rows. See
  `D-the-picker-seeds-from-a-per-provider-suggestion-list`.
- **`/speed fast|normal`.** The alias is spec'd in
  D-a-model-descriptor-carries-no-capability-claim but is a separate command; it is not
  built here. Effort is the concrete dimension this spec covers.
- **A capability badge on a picker row.** rho cannot prove tool support from a listing,
  and the descriptor carries no capability claim.
- **Writing the model change back to the session file.** The session file records the
  turns; the selection is not a turn. A later spec may add a `ModelChange` record.
- **Ranking matches by score.** The picker keeps the original row order, so the current
  row stays first. A score column would move the anchor on every keystroke, and the
  anchor is more useful than the score. See
  `D-model-picker-allows-fuzzy-search-and-typed-fallback`.
- **A `*` key that stars something the picker did not show.** The starred file is edited
  in place for that case.

## 7. Test cases

Every test named here lives in this repo when the spec status flips to `delivered`. Each
row lists the file the test lives in and the assertion the test proves.

### `rho-core` — the mutable slice

- `a_selection_reads_back_the_seed_from_the_config` — `crates/rho-core/tests/selection.rs`.
  Build a session with a config that carries `model = "seed-model"` and
  `reasoning_effort = Some(Medium)`. `session.selection()` returns exactly that pair.
- `set_selection_takes_effect_on_the_next_request` — `crates/rho-core/tests/selection.rs`.
  Set a new selection, run one prompt through a recording provider, assert the provider
  received the new model.
- `set_selection_never_changes_the_running_turn` — `crates/rho-core/tests/selection.rs`.
  Start a run whose provider blocks on a channel. Call `set_selection` with a new model.
  Release the provider. Assert the received request carries the **old** model, and the
  next prompt uses the new one.

### `rho-tui` — the slash commands

- `slash_model_with_no_arg_opens_the_picker` — `crates/rho-tui/tests/model_picker.rs`.
- `slash_model_with_an_id_arg_applies_and_closes` —
  `crates/rho-tui/tests/model_picker.rs`. Returns `KeyAction::ApplySelection` and pushes
  one notice.
- `slash_effort_with_no_arg_shows_the_current_level_as_a_notice` —
  `crates/rho-tui/tests/model_picker.rs`.
- `slash_effort_with_a_level_arg_applies_and_notices` —
  `crates/rho-tui/tests/model_picker.rs`. Returns `KeyAction::ApplySelection`.
- `slash_effort_with_an_unknown_level_pushes_an_error` —
  `crates/rho-tui/tests/model_picker.rs`. Names the valid levels.
- `slash_effort_command_appears_in_the_list_with_its_argument` —
  `crates/rho-tui/tests/model_picker.rs`. `filter_slash_commands("/effort high")` matches.

### `rho-tui` — the picker keys

- `the_picker_shows_the_current_model_first_and_then_the_starred` —
  `crates/rho-tui/tests/model_picker.rs`. Row 0 is the current, its `is_current` is true,
  the rest are starred, and duplicates of the current are dropped.
- `arrow_keys_move_the_picker_selection_and_never_wrap` —
  `crates/rho-tui/tests/model_picker.rs`.
- `enter_applies_the_highlighted_row_and_closes` —
  `crates/rho-tui/tests/model_picker.rs`. Returns `KeyAction::ApplySelection` and sets
  `Panel::None`.
- `esc_closes_the_picker_with_no_change` — `crates/rho-tui/tests/model_picker.rs`. Returns
  `KeyAction::None`, sets `Panel::None`, and the session's selection would not change (the
  test never calls `set_selection`).
- `tab_cycles_the_highlighted_rows_effort_but_does_not_apply_until_enter` —
  `crates/rho-tui/tests/model_picker.rs`. Six presses walk the whole cycle.
- `shift_tab_toggles_the_star_and_returns_a_persistence_action` —
  `crates/rho-tui/tests/model_picker.rs`. Returns
  `KeyAction::PersistStarred(new_list)`.

### `rho-tui` — the fuzzy filter

- `fuzzy_match_matches_a_scattered_subsequence_case_insensitively` —
  `crates/rho-tui/tests/model_picker.rs`. `sn45` matches `claude-sonnet-4-5`, `NoVa`
  matches `amazon.nova-micro-v1:0`, an empty query matches every string.
- `fuzzy_match_rejects_a_query_not_present` —
  `crates/rho-tui/tests/model_picker.rs`. Order matters; `54` does not match
  `claude-sonnet-4-5`.
- `typing_filters_the_picker_by_fuzzy_subsequence` —
  `crates/rho-tui/tests/model_picker.rs`. Typing `sonnet` drops every row that fails the
  match, and the query field holds `sonnet`.
- `typing_a_letter_that_is_also_a_key_binds_to_the_query_not_the_shortcut` —
  `crates/rho-tui/tests/model_picker.rs`. `j`, `k`, `e`, and `*` are typed into the
  query, not routed as picker shortcuts.
- `enter_on_an_empty_filter_applies_the_query_verbatim` —
  `crates/rho-tui/tests/model_picker.rs`. Returns
  `KeyAction::ApplySelection(ModelSelection { model: query, … })`.
- `backspace_removes_a_query_char_and_resets_the_selection` —
  `crates/rho-tui/tests/model_picker.rs`. Also asserts that a backspace on an empty
  query is a no-op.

### `rho-cli` — provider suggestions

- `openrouter_suggestions_are_non_empty_and_unique` —
  `crates/rho-cli/src/provider.rs`. The `openrouter` suggestion list has at least one id
  and no duplicates.
- `azure_has_no_suggestions_because_it_names_deployments` —
  `crates/rho-cli/src/provider.rs`. `azure` returns an empty suggestion list.

### `rho-tui` — footer regression

- `the_model_picker_footer_hits_a_space_between_status_and_hint` —
  `crates/rho-tui/tests/render.rs`. On an 88-column terminal the footer keeps at least one
  blank cell between `ready` and the picker hint.

### `rho-tui` — the starred file

- `starred_read_missing_file_returns_an_empty_list` —
  `crates/rho-tui/tests/starred.rs`.
- `starred_read_and_write_round_trip` — `crates/rho-tui/tests/starred.rs`.
- `starred_read_of_a_bad_file_returns_a_notice_and_an_empty_list` —
  `crates/rho-tui/tests/starred.rs`. The `Err(String)` names the file path and the parse
  error.

### `rho-tui` — the renderer

- `the_picker_plans_a_row_per_starred_plus_the_current_line` —
  `crates/rho-tui/tests/model_picker.rs`. `plan_layout` for the picker returns the row
  count the panel needs.
