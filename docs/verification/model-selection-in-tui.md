# Verification: model selection in the TUI

The five surfaces `SPEC-model-selection-in-tui` ships:

- `/model` opens the picker.
- `/model <id>` applies without a picker.
- `/effort <level>` sets the effort.
- `/effort` alone reports the current level.
- The picker filters as the user types (fuzzy subsequence), applies the highlighted
  filtered row on `Enter`, and applies the query verbatim on `Enter` when no row matches.

The picker shows the current model plus every starred id. `Tab` cycles the preview
effort, `Shift+Tab` toggles a star to `~/.rho/starred-models.toml`, `Esc` closes. See
`D-the-model-picker-is-a-panel` and
`D-model-picker-allows-fuzzy-search-and-typed-fallback`.

## What the drive ran

`bench/tui_model_picker_drive.py` runs the real release binary inside a pty. It reads
the terminal with `pyte`, so every check is on rendered cells. It seeds `HOME` to a
temp directory with a starred file that names two ids.

Every check reads a `pyte` screen line and asserts on its content. See
`bench/tui_model_picker_drive.py` for the exact byte sequences and asserts.

## The branch binary passes nine checks

```
--- FINDINGS
   picker opened with current + 2 starred rows
   Enter applied starred-a; banner updated
   fuzzy typing `b` filtered to starred-b and Enter applied it
   a query with no match applied verbatim as the model id
   /model direct-id applied without a picker
   /effort high produced a notice
   /effort alone reported the current level
   /effort loud pushed an error naming the valid levels
   Shift+Tab on a filtered row removed starred-b from the file
--- OK · 9 checks passed
```

The command that produced this output:

```sh
cargo build --release -p rho-cli
python3 bench/tui_model_picker_drive.py
```

## The base binary fails at the first check

The same script run against the tree at commit `1d19140` (the tip of `main` before
this feature) fails before it reaches check two:

```
=== BASE BINARY (main, pre-feature) ===
AssertionError: picker header shows the current row: ['ρ rho  ...',
    ...  '! notice · model: seed-model on openrouter',
    '────',  '❯ Type a prompt. / for commands. ? for help.',  '────', ...]
```

The base binary answers `/model` with the notice
`model: seed-model on openrouter` and nothing else. There is no picker, no header row,
and no starred list. The command that produced this output:

```sh
git worktree add /tmp/rho-base-wt HEAD
( cd /tmp/rho-base-wt && cargo build --release -p rho-cli )
cp /tmp/rho-base-wt/target/release/rho /tmp/rho-base
RHO_BINARY=/tmp/rho-base python3 bench/tui_model_picker_drive.py
```

The failure is the evidence: the branch adds every surface the feature promises, and
the base has none of them.

## The picker without fuzzy also loses ground

The first cut of the picker (commit `3073600`) had no query field. The same script
run against that binary passes the first two checks and fails at the fuzzy assertion:

```
=== PARENT (3073600, picker without fuzzy) ===
AssertionError: the query prompt draws `> b`:
  ['ρ rho  ...',
   ...  'model:',
   '  ★ starred-a (current)',
   '  ★ starred-b',
   '❯ █',  '  ready  ↑ ↓ choose · enter apply · e effort · * star · esc close']
```

The parent picker draws the `model:` header and no `> b` query prompt, because typing
`b` had no meaning there. On this branch the same key stroke filters the list. The
failing assertion is the evidence.

## Mutation proofs

Every rule in the spec is pinned by a test whose failure is watched, not assumed.
`AGENTS.md` step 7 names this "prove the test catches the bug". For each file below,
the file was copied to `/tmp` first, one rule was broken, the failing test was named,
and the file was restored by copying back. See
`.rho-work/decisions/20260901-232014-D-model-selection-is-mutable-behind-a-mutex.md`
and the sibling decisions.

| Mutation | File | Rule | Test that fails |
| --- | --- | --- | --- |
| Read model from `SessionConfig` inside `build_request` | `crates/rho-core/src/agent.rs` | The mutex is the one source of truth for the running model | `set_selection_takes_effect_on_the_next_request` |
| Make `set_selection` a no-op | `crates/rho-core/src/agent.rs` | The write reaches the mutex | `set_selection_takes_effect_on_the_next_request` |
| Send `None` reasoning even when the mutex holds one | `crates/rho-core/src/agent.rs` | The effort on the wire follows the mutex | `set_selection_takes_effect_on_the_next_request` |
| Drop the effort on `/model <id>` | `crates/rho-tui/src/state.rs` | A typed id keeps the current effort | `slash_model_with_an_id_arg_applies_and_closes` |
| Enter returns `KeyAction::None` from the picker | `crates/rho-tui/src/state.rs` | Enter carries the `ApplySelection` action | `enter_applies_the_highlighted_row_and_closes` |
| Star toggle returns `KeyAction::None` | `crates/rho-tui/src/state.rs` | A star toggle emits `PersistStarred` | `star_toggles_and_returns_a_persistence_action` |
| Down wraps at the last row | `crates/rho-tui/src/state.rs` | The selection never wraps | `arrow_keys_move_the_picker_selection_and_never_wrap` |
| Substring match replaces subsequence match | `crates/rho-tui/src/state.rs` | `fuzzy_match` is a subsequence, not a substring | `fuzzy_match_matches_a_scattered_subsequence_case_insensitively` |
| Enter on an empty filter returns `None` | `crates/rho-tui/src/state.rs` | Enter applies the query verbatim | `enter_on_an_empty_filter_applies_the_query_verbatim` |
| Printable char routes as ignored | `crates/rho-tui/src/state.rs` | Every printable key appends to the query | `typing_filters_the_picker_by_fuzzy_subsequence` |
| Backspace is a no-op | `crates/rho-tui/src/state.rs` | Backspace pops a query char and resets the selection | `backspace_removes_a_query_char_and_resets_the_selection` |
| openrouter suggestions empty | `crates/rho-cli/src/provider.rs` | A first-time picker has seed rows | `openrouter_suggestions_are_non_empty_and_unique` |
| openrouter suggestions duplicate | `crates/rho-cli/src/provider.rs` | The list has no duplicates | `openrouter_suggestions_are_non_empty_and_unique` |
| Long footer hint collides with `ready` | `crates/rho-tui/src/render.rs` | The picker hint is short enough to leave a space | `the_model_picker_footer_hits_a_space_between_status_and_hint` |
| Keep duplicate current in starred rows | `crates/rho-tui/src/state.rs` | The picker drops duplicates of the current | `the_picker_shows_the_current_model_first_and_then_the_starred` |
| Save writes an empty array | `crates/rho-tui/src/starred.rs` | The file mirrors the in-memory list | `starred_read_and_write_round_trip` |

Every mutation was watched to fail its named test, and every restored file was
`diff -q` against its `/tmp` backup and reported no drift.

## Test count

- `crates/rho-core/tests/selection.rs`: 3 tests.
- `crates/rho-tui/tests/model_picker.rs`: 21 tests (6 for the picker keys, 6 for the
  fuzzy filter, 6 for the slash commands, 3 for the panel layout).
- `crates/rho-tui/tests/starred.rs`: 3 tests.
- `crates/rho-cli/src/provider.rs`: 2 tests (gated behind `feature = "tui"`) for the
  provider suggestion list.
- `crates/rho-tui/tests/render.rs`: 1 regression test for the footer overflow.
- One updated golden fixture: `docs/design/tui-frames/100-slash-list.txt` (adds the
  `/effort` row and re-widens `/model`'s summary).
