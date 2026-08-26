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

## 6. Driven for real, on Bedrock

This is `AGENTS.md` step 11. Every command and every output below is real. The provider is AWS
Bedrock and the model is the latest haiku, which is cheap enough for a full sweep.

```sh
cargo build --release -p rho-cli
unset AWS_PROFILE
export HOME=/tmp/rho-sess-e2e/home    # so no drive touches the real ~/.rho
export MODEL=us.anthropic.claude-haiku-4-5-20251001-v1:0
mkdir -p /tmp/rho-sess-e2e/root && cd /tmp/rho-sess-e2e/root && git init -q .
echo "the parser reads a number, and it forgets the sign" > sample.txt
```

### A run writes a session

```sh
./target/release/rho run "Read sample.txt and say in one short sentence what the bug is." \
  --provider bedrock --model $MODEL --root /tmp/rho-sess-e2e/root
```

```
rho: session 20260826-204754-f8c9 at /tmp/rho-sess-e2e/home/.rho/sessions/root-e3764aec/20260826-204754-f8c9.jsonl
I'll read the sample.txt file for you.
The bug is: the parser reads a number but forgets to include the sign.
```

The store is private, and the mode is not the umask:

```
drwx------  /tmp/rho-sess-e2e/home/.rho/sessions
drwx------  /tmp/rho-sess-e2e/home/.rho/sessions/root-e3764aec
-rw-------  20260826-204754-f8c9.jsonl
```

```sh
./target/release/rho sessions list --root /tmp/rho-sess-e2e/root
```

```
ID                   LAST ACTIVE TITLE              MODEL           TOKENS  COST
20260826-204754-f8c9 just now    Read sample.txt a… us.anthropic.c…   2.1k     -
```

The cost column is a dash because Bedrock reports no cost. rho shows no number it did not read.

### A resume, and then the same thing twice

```sh
./target/release/rho run "Which word in the file names the thing it forgets? One word." \
  --continue --provider bedrock --model $MODEL --root /tmp/rho-sess-e2e/root
```

```
rho: continuing session 20260826-204754-f8c9 in /tmp/rho-sess-e2e/root (4 messages)
The word is "sign".
```

```sh
./target/release/rho run "Say the same word again." --continue ...
```

```
rho: continuing session 20260826-204754-f8c9 in /tmp/rho-sess-e2e/root (6 messages)
sign
```

**The model never re-read the file.** It answered from the conversation the resume replayed, and
the message count grew from four to six. That is the proof the replay works, and no fixture
could give it.

### A crash continue, after a real SIGKILL

```sh
./target/release/rho run "Write a 900 word essay about integer parsing, one sentence per line." ... &
sleep 1.5 && kill -9 $!
```

```
SIGKILL sent to 58086
the crashed session is 20260826-204830-5c0a
--- its last record, which must not be a close ---
message
--- the lock file is left behind, and the operating system released the lock ---
/tmp/.../20260826-204830-5c0a.lock
--- the crash continue ---
rho: continuing session 20260826-204830-5c0a in /tmp/rho-sess-e2e/root (1 messages)
RECOVERED
```

The killed process left no `Closed` record, so the crash offer finds it. The operating system
released the `flock`, so the next run could take it with no cleanup by hand.

### Two processes, and the lock

```
--- the second process, on the same session ---
rho: session 20260826-202353-16b1 is open in another process. Use another session, or close that one.
--- and a bare --continue while it is live ---
rho: continuing session 20260826-202432-ddca ...     # it moved to the next one
--- a read-only command on a live session ---
ID                   LAST ACTIVE TITLE              MODEL           TOKENS  COST
20260826-202432-ddca just now    Read sample.txt a… us.anthropic.c…   2.1k     -
```

### The fork flow, from the two printed commands

```sh
./target/release/rho sessions show 20260826-2047 --root /tmp/rho-sess-e2e/root
```

```
session  20260826-204754-f8c9  "Read sample.txt and say in one short sentence w…
  r1    20:47:54  model        bedrock us.anthropic.claude-haiku-4-5-20251001-v…
  r2    20:47:54  user         Read sample.txt and say in one short sentence wh…
  r3    20:47:56  usage        2.1k
  r4    20:47:56  tool_call    read  path=sample.txt
  r5    20:47:56  tool_result  read  50 B
  r6    20:47:57  usage        2.1k
  r7    20:47:57  assistant    The bug is that the parser reads a number but fo…
  r8    20:47:57  stop         EndTurn
  r9    20:47:57  closed
```

The tool result shows the tool and 50 bytes, and never the body. A secret inside a result
cannot reach the terminal by accident.

```sh
./target/release/rho sessions fork 20260826-2047 --at r4 --root /tmp/rho-sess-e2e/root
```

```
forked session 20260826-204754-f8c9 at record r4 into 20260826-204813-937c.
--- the original is byte-identical? ---
yes, d827290a7c9bf50b7c3d90741c13b6b6c7d9a53f
```

### The failure paths, including the same thing twice

```
--- an absent session ---            rho: no session in this project starts with 19700101-00   (exit 1)
--- the SAME absent session, twice --- rho: no session in this project starts with 19700101-00 (exit 1)
--- an ambiguous prefix ---          rho: the id prefix 2026 matches 3 sessions: 20260826-204754-f8c9, 20260826-204807-0497, 20260826-204813-937c
--- an id as the prompt, with a space --- rho: the prompt "20260826-2023" looks like a session id. The value needs an equals sign, so write --resume=20260826-2023 and give the prompt after it.
--- --allow-widen with no session flag --- error: the following required arguments were not provided: --continue[=<ID>]
--- --continue with --ephemeral ---  rho: --ephemeral writes no file, so there is nothing to continue. Drop one of the two.
--- a fork at a record that is not there --- rho: the session holds no record r999
--- an empty title ---               rho: a session title cannot be empty
--- a widening resume ---            rho: a resume would widen approval from read-only to allow-all; pass --allow-widen to allow it
--- with --allow-widen ---           WIDE.
--- --ephemeral ---                  rho: this session is ephemeral, so rho writes no session file.
                                     session files before 3, after 3
--- delete, then the fork survives --- deleted session 20260826-204754-f8c9. ... /…/20260826-204813-937c.jsonl
--- deleting the same session twice --- rho: no session in this project starts with 20260826-204754-f8c9
```

## 7. What the live drive found that every test had missed

**Four defects. Every one of them passed the whole suite first.**

### 1. Bare `--continue` could never work

A run that ends on its own writes a `Closed` record, and `newest_open` skips a closed session.
So right after a successful run:

```
rho: no session to continue in root-e3764aec; start one without --continue
```

The most common thing a user wants was impossible. Every unit test passed, because every test
seeded a session that never closed. `SessionStore::newest_resumable` now answers `--continue`,
and `newest_open` stays the crash offer. See
`D-continue-takes-the-newest-session-closed-or-not`. Tests:
`continue_takes_the_newest_session_closed_or_not`, `newest_resumable_skips_a_locked_session`,
`newest_resumable_skips_an_unreadable_file`,
`a_closed_session_is_continued_and_states_its_reopen`.

The old test `a_closed_session_is_never_continued` asserted the defect, so it is replaced and
recorded in `bench/deleted-tests.txt`.

### 2. `rho sessions name` made the session unreadable

```
rho: record r24 names parent r23, which is a leaf record and never a parent
```

A `Name` record is a leaf, and the writer made it the chain head. The next append then named a
leaf as its parent, and the integrity check of section 6a refused the whole file. **The guard
worked, and the writer produced the bad file.** A leaf no longer moves the head, and a reopen
takes the last chain record. Tests: `a_leaf_record_never_becomes_the_chain_head`,
`a_named_session_reopens_and_stays_readable`.

### 3. `show` printed a staircase

Indentation counted the whole chain to the root, so a linear conversation stepped one column
deeper per record and the text ran off the line after six records. Indentation exists to show a
**branch**, so it now counts branch points. Tests:
`show_does_not_indent_a_linear_conversation`, `show_indents_only_below_a_branch_point`.

A long title also pushed the model and the close state off the header. The title gives way now,
because a user can read it again on the first prompt line. Test:
`show_keeps_the_model_and_the_state_when_the_title_is_long`.

### 4. Two smaller ones

- An empty title refused with `cannot decode a record: a session title cannot be empty`, which
  reads like file corruption. `SessionError::EmptyTitle` names it. Test:
  `an_empty_name_is_refused_by_the_recorder`.
- Neither renderer ended with a newline, so the shell prompt ran into the last row. Test:
  `both_renderers_end_with_a_newline`.
- A crash leaves an `<id>.lock` file, and `delete` did not remove it. Test:
  `delete_removes_the_session_and_its_sidecars`.

## 8. What is not verified live

- **The terminal.** `/sessions` is not built, and the TUI records no session. See the note in
  `docs/features.md`. Only `rho run` and `rho sessions` are wired.
- **One provider.** Bedrock only. The change touches no wire format, so a provider cannot see
  it, and `SessionRecorder` folds the same normalised events for every provider. That is a
  reason, not a measurement.
