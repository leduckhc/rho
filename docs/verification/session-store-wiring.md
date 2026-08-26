# The session store, wired: what was really run

This page holds real commands and their real output. Nothing here is a summary of a report.

## 1. The mutation proofs of the core slice

`AGENTS.md` step 7 asks for a deliberate break, a watched failure, and a restore. The good
file was copied to `/tmp/mod.rs.good` first. **`git checkout` was never used**, because it
throws away every uncommitted change in a file. See `D-jcode-bash-lessons`.

Command:

```sh
python3 /tmp/mutate.py    # copies /tmp/mod.rs.good back before and after each break
```

Each row is one deliberate break of `crates/rho-core/src/session/mod.rs`, and the tests that
caught it.

| Break | Tests that failed |
| --- | --- |
| `mint_id` returns an id even when the file holds it | `two_dropped_records_do_not_mint_a_duplicate_id`, `a_fork_of_a_branch_does_not_mint_a_duplicate_id`, `a_fork_of_a_fork_keeps_every_id_unique`, `a_resume_of_a_resume_keeps_every_id_unique`, `a_reopen_of_a_reopened_file_writes_one_more_reopened_record`, `every_record_id_in_a_file_is_unique` |
| the integrity check runs and its result is dropped | `an_orphan_refuses_the_file`, `an_unknown_chain_record_is_caught_by_its_children`, `a_hand_built_leaf_parent_is_refused`, `a_duplicate_id_in_a_file_is_refused` |
| the duplicate-id branch is deleted | `a_duplicate_id_in_a_file_is_refused` |
| `Name` is classed as a chain record, not a leaf | `a_hand_built_leaf_parent_is_refused` |
| `create` calls `File::create`, which truncates | `create_twice_with_one_id_never_truncates`, `create_minted_remints_after_a_collision`, `create_minted_gives_up_after_mint_attempts` |
| `create` sets no file mode | `a_session_file_is_0o600_on_unix` |
| `create_private_dir` sets no directory mode | `a_store_directory_is_0o700_on_unix` |
| a sidecar spill sets no mode | `a_sidecar_spill_file_is_0o600_on_unix` |
| `create` writes no `ModelChange` record | `a_created_file_states_its_model_on_the_second_line`, `a_fork_of_a_branch_does_not_mint_a_duplicate_id` |
| the header states no session id | `a_header_states_its_own_session_id`, `the_new_header_fields_survive_a_write_and_a_read` |
| a fork keeps the old parent on its first copied record | `a_fork_re_parents_its_first_copied_record` |
| the chain walk breaks on a hole, as it used to | `a_branch_walk_with_a_hole_is_an_error` |
| `create_minted` tries once and never re-mints | `create_minted_remints_after_a_collision` |
| a fork mints without seeing the copied ids | `a_fork_of_a_branch_does_not_mint_a_duplicate_id`, `a_fork_of_a_fork_keeps_every_id_unique` |

**One break passed at first, and that is the point of this step.** The header-id break
changed nothing a test could see, because `a_header_states_its_own_session_id` read the file
through `SessionReader::read`, which falls back to the file stem when the header states no id.
So the test passed against a header with no id at all. Both tests now read through
`read_from`, which has no path and therefore no fallback. After the change the same break
fails both tests.

The good file was copied back at the end. The suite:

```sh
cargo test --workspace --all-features
```

```
1536 passed, 0 failed
```
