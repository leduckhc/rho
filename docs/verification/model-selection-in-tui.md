# Verification: model selection in the TUI

The four surfaces `SPEC-model-selection-in-tui` ships:

- `/model` opens the picker.
- `/model <id>` applies without a picker.
- `/effort <level>` sets the effort.
- `/effort` alone reports the current level.

The picker shows the current model plus every starred id, and `*` toggles a star to
`~/.rho/starred-models.toml`. See `D-the-model-picker-is-a-panel`.

## What the drive ran

`bench/tui_model_picker_drive.py` runs the real release binary inside a pty. It reads
the terminal with `pyte`, so every check is on rendered cells. It seeds `HOME` to a
temp directory with a starred file that names two ids.

Every check reads a `pyte` screen line and asserts on its content. See
`bench/tui_model_picker_drive.py` for the exact byte sequences and asserts.

## The branch binary passes seven checks

```
--- FINDINGS
   picker opened with current + 2 starred rows
   Enter applied starred-a; banner updated
   /model direct-id applied without a picker
   /effort high produced a notice
   /effort alone reported the current level
   /effort loud pushed an error naming the valid levels
   * removed starred-a from ~/.rho/starred-models.toml
--- OK · 7 checks passed
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
| Keep duplicate current in starred rows | `crates/rho-tui/src/state.rs` | The picker drops duplicates of the current | `the_picker_shows_the_current_model_first_and_then_the_starred` |
| Save writes an empty array | `crates/rho-tui/src/starred.rs` | The file mirrors the in-memory list | `starred_read_and_write_round_trip` |

Every mutation was watched to fail its named test, and every restored file was
`diff -q` against its `/tmp` backup and reported no drift.

## Test count

- `crates/rho-core/tests/selection.rs`: 3 tests.
- `crates/rho-tui/tests/model_picker.rs`: 13 tests.
- `crates/rho-tui/tests/starred.rs`: 3 tests.
- One updated golden fixture: `docs/design/tui-frames/100-slash-list.txt` (adds the
  `/effort` row and re-widens `/model`'s summary).
