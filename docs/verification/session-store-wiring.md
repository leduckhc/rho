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

## 5. The mutation proofs of the command-line slice

`crates/rho-cli/src/recording.rs` and `cli.rs` were copied to `/tmp` first, and copied back
before and after every break.

| Break | Tests that failed |
| --- | --- |
| a bare flag starts a new session instead of the newest | `a_bare_flag_takes_the_newest_session`, `the_flag_does_not_swallow_the_prompt`, `a_session_id_as_the_prompt_is_refused_with_a_hint` |
| an id-shaped prompt is not refused | `a_session_id_as_the_prompt_is_refused_with_a_hint` |
| a resume skips the widen check | `a_resume_that_would_widen_is_refused_on_the_command_line`, `an_unknown_mode_name_still_parses_to_the_strictest_mode` |
| `--allow-widen` is ignored | `allow_widen_permits_the_wider_resume_on_the_command_line` |
| a resume replays nothing | `a_resume_replays_the_earlier_conversation` |
| a stale result handle survives a resume | `a_resume_expires_a_stale_result_handle_from_the_file` |
| `--ephemeral` still writes a file | `the_ephemeral_flag_writes_no_file`, `the_ephemeral_config_key_writes_no_file` |
| `--ephemeral` with `--continue` is allowed | `ephemeral_and_continue_together_are_refused` |
| the `session-file` key is ignored | `the_session_file_key_overrides_the_store` |
| an empty store is not an error, so a resume starts blank | `continue_with_no_session_is_an_error_that_says_what_to_do`, `a_closed_session_is_never_continued`, `a_second_process_cannot_continue_a_live_session` |
| the approval name comes from a literal, not the config | `a_forged_header_cannot_widen_a_run` |
| `close` writes nothing | `a_run_that_ends_closes_its_session`, `a_closed_session_is_never_continued`, `the_run_path_records_the_prompt_the_turns_and_the_close` |
| `start` records no prompt | `the_run_path_records_the_prompt_the_turns_and_the_close` |
| `observe` folds nothing | the two tests above |
| the run path records no prompt | `the_headless_run_path_drives_every_lifecycle_call` |
| the run path never closes the session | the same |
| the printer folds no event | the same |
| the run path opens no recording | the same |

**Four breaks passed at first, and all four were in the run path itself.** That is the exact
defect this lane exists to fix: the store had a full test suite and no caller. My first tests
drove `recording::open` directly, so deleting the calls in `run_headless` changed nothing a test
could see.

Two changes fixed it:

1. The lifecycle moved into `Recording::start`, `Recording::observe`, and `Recording::close`.
   A caller now makes three named calls instead of reaching into the recorder, and each one has
   a test on a real file.
2. `the_headless_run_path_drives_every_lifecycle_call` reads `cli.rs` and asserts that
   `run_headless` calls all three, and that the printer folds every event. A grep is weak, and
   it is the only thing that can see a deleted call on a path with no injectable provider.
   `crates/rho-cli/src/provider.rs` belongs to another lane, so this lane cannot add a stub
   provider to drive the path in process. **Section 6 drives it for real instead.**

The stale-handle test had the same shape of hole: it called `expire_stale_result_handles`
directly, so it would have passed while the resume forgot to call it. It now seeds a session
file with a stored preview and resumes it.
