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

## 2. The mutation proofs of the row slice

Same method. `crates/rho-core/src/session/row.rs`, `lock.rs`, and `mod.rs` were copied to
`/tmp` first, and copied back before and after every break.

| Break | Tests that failed |
| --- | --- |
| the head read has no line bound, so a row decodes the whole file | `a_row_never_decodes_the_whole_file`, `a_first_prompt_beyond_the_head_lines_leaves_the_title_empty`, `a_list_of_five_hundred_sessions_reads_only_the_head_and_the_tail` |
| the tail read starts at byte zero, so the window is the whole file | `a_row_never_decodes_the_whole_file`, `a_list_of_five_hundred_sessions_reads_only_the_head_and_the_tail` |
| an explicit `Name` record never wins over the first prompt | `a_tail_read_drops_a_partial_first_line` |
| the first usage record wins, not the last | `a_row_reports_the_cumulative_usage` |
| a decoded record no longer decides the close flag | `a_row_marks_a_closed_session`, `a_closed_session_is_never_offered` |
| the title is not cut at one line | `a_row_falls_back_to_the_first_prompt` |
| the title is not capped at 60 bytes | `a_row_falls_back_to_the_first_prompt` |
| `rows` sorts oldest first | `rows_come_back_newest_first` |
| an unreadable file fails the whole list | `one_unreadable_file_is_one_row` |
| a prefix picks the newest of several matches | `an_ambiguous_prefix_lists_every_match` |
| `newest_open` offers a closed session | `a_closed_session_is_never_offered` |
| `newest_open` does not skip a locked session | `newest_open_skips_a_locked_session` |
| the lock is taken and never held | `a_second_process_cannot_open_a_live_session`, `a_lock_is_released_when_the_process_ends`, `newest_open_skips_a_locked_session` |
| a `flock` failure warns and continues | the same three |
| every `flock` failure counts as busy, so a filesystem that cannot lock looks live | `only_a_would_block_error_means_busy` |
| a lock file that cannot open is not a refusal | `a_lock_file_that_cannot_open_is_refused` |

**Five breaks passed at first.** Each one was a test that could not fail, and each is fixed:

1. **The title one-line rule.** The test used a 90 character first line, so the 60 byte cap
   cut the newline away and hid the missing rule. It now uses two prompts: a short first line
   proves the one-line rule, and a long one proves the cap.
2. **An unreadable file.** Both bad files in the test still **opened**; only their content was
   bad. So the open-failure branch had no test. The test now also writes a file with mode
   `0o000`.
3. **The `flock` refusal.** The only test of it passed a store root that is a file, so
   `create_private_dir` refused first and `take_lock` was never reached. A new test makes the
   lock path a directory, so the open fails inside `take_lock`.
4. **The classification of a `flock` error code.** A filesystem that refuses to lock cannot be
   arranged on a developer machine, so the branch was unreachable from a test. The
   classification is now a pure function, `classify_lock_failure`, and
   `only_a_would_block_error_means_busy` drives it with `EWOULDBLOCK`, `EAGAIN`, `ENOLCK`,
   `EOPNOTSUPP`, `EBADF`, and an unknown code.
5. **The partial first line of a tail read.** Deleting the explicit skip changed nothing a test
   could see, because a partial line cannot decode into an `Entry` and the loop drops it
   already. Two guards where each masks the other is the trap this project met in `record_fits`
   and in `ProviderState::for_owner`. **So the redundant guard is gone**, and the comment says
   why. `a_tail_read_drops_a_partial_first_line` pins the outcome.

## 3. The list budget, measured

```sh
cargo test -p rho-core --test session_rows -- --nocapture a_list_of_five
```

```
500 rows in 20.683875ms
```

The budget in the spec is 100 milliseconds for 500 sessions. The number asserts nothing, per
`D-a-budget-is-measured-not-asserted`. The assertion that proves the bound is the sentinel row:
one of the 500 files carries a `Name` record past the head window and more than
`ROW_TAIL_BYTES` before the end, so a full decode reports an explicit title and a bounded read
does not.

The cache in `D-no-list-cache-until-a-budget-fails` therefore does not ship.

## 4. The mutation proofs of the recorder slice

`crates/rho-core/src/session/mod.rs` was copied to `/tmp/mod3.rs.good` first, and copied back
before and after every break.

| Break | Tests that failed |
| --- | --- |
| no `TurnEnd` arm, exactly as the defect was | `a_run_records_the_assistant_text_of_a_turn`, `a_run_records_a_tool_call_before_its_result`, `every_tool_call_on_disk_has_a_result_on_disk`, `a_recorded_run_replays_as_a_valid_message_list`, `a_reasoning_payload_survives_the_recorder_verbatim`, `a_recorded_secret_named_argument_is_masked` |
| text deltas are not accumulated | `a_run_records_the_assistant_text_of_a_turn`, `a_recorded_run_replays_as_a_valid_message_list` |
| a tool call is not folded | five tests, including `every_tool_call_on_disk_has_a_result_on_disk` |
| the parsed arguments are replaced by an empty object | `a_run_records_a_tool_call_before_its_result`, `a_cancel_records_the_real_tool_arguments`, `a_recorded_secret_named_argument_is_masked` |
| a tool result is not wrapped in a `ToolResult` block | `a_run_records_a_tool_call_before_its_result`, `every_tool_call_on_disk_has_a_result_on_disk` |
| an empty turn writes a blank message | `an_empty_turn_writes_no_assistant_record` |
| the reasoning payload is dropped | `a_reasoning_payload_survives_the_recorder_verbatim` |
| the reasoning payload is rewritten | `a_reasoning_payload_survives_the_recorder_verbatim` |
| a cancel does not flush the turn | `a_cancel_records_the_real_tool_arguments` |
| a cancel writes no synthetic result | `cancel_leaves_no_half_written_tool_pairing` |
| redaction is skipped on the folded turn | `a_recorded_secret_named_argument_is_masked` |
| an empty title is accepted | `an_empty_name_is_refused_by_the_recorder` |

Every break was caught on the first attempt in this slice. Twelve of twelve.
