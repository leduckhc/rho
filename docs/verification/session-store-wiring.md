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

**Re-measured after the review changed the row builder**, because a number in a doc must match the
tree it documents. Three runs: 21.88 ms, 19.43 ms, 20.13 ms. `docs/benchmarks.md` states all three,
so a reader expects a spread and not one exact value. A documentation reviewer raised this: it ran
the command and got 21.37 ms against a doc that named 20.68 ms.

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

## 9. The mutation proofs of the review fixes

A reviewer that did not write the code read the whole diff, with the same defect history. Section
15c of the spec lists its six findings. Each fix has a proof.

| Break | Tests that failed |
| --- | --- |
| a lock refusal degrades to ephemeral again | `a_filesystem_that_cannot_lock_stops_a_new_run` |
| only `Busy` stops the run, and `LockUnsupported` does not | the same |
| the resume mode check compares the wrong way | `a_forged_header_cannot_widen_a_run`, `a_resume_that_would_widen_is_refused_on_the_command_line`, `an_unknown_mode_name_still_parses_to_the_strictest_mode` |
| a delete takes no lock | `delete_refuses_a_live_session` |
| a fork mints once and gives up | `a_fork_mints_a_free_id_through_the_store` |
| an unknown record in a walk is not named | `a_fork_at_a_record_the_file_does_not_hold_is_refused` |

The reviewer named `a_forged_header_cannot_widen_a_run` as theatre, and the row above is the
answer: the rewritten test drives a table of eight stored-and-live mode pairs, so reversing the
comparison breaks it. The first version asserted only that a narrower resume succeeded, and that
passes whether or not the header is trusted.

An attempt to break `StoredApproval::parse` into a fail-open default did not compile, because the
match has no wildcard. That is the shape `D-plugin-does-not-classify-itself` asks for: a new mode
name breaks the build instead of becoming the most permissive one.

## 10. The public items no test names

`AGENTS.md` step 8 asks for this list, and step 9 asks a reviewer for it too.

**Removed rather than tested.** `SessionLock::path` and the `path` field behind it. Nothing read
either. A field no reader wants is dead surface, and this project has a defect class for it.
`sessions_command::mint_free_id` went the same way, replaced by `SessionStore::fork_minted`.

**Covered only through a caller, and named by no test:**

| Item | Where | What reaches it |
| --- | --- | --- |
| `SessionStore::create_file` | `session/mod.rs` | `the_session_file_key_overrides_the_store` drives it through `recording::open`. |
| `SessionSelector::resumes` | `rho-cli/src/recording.rs` | `ephemeral_and_continue_together_are_refused` and every resume test. |
| `SessionsAction` and its five variants | `rho-cli/src/cli.rs` | `session_binary.rs` drives `list`, `show`, `fork`, `name`, and `delete` through the real binary. |
| `sessions_command::run` | `rho-cli/src/sessions_command.rs` | the same four tests in `session_binary.rs`. |

Each of those has a test that fails when it breaks. None has a test that names it, and that is
stated here rather than left for a reader to discover.

## 11. The review fixes, driven for real again

The whole sweep ran once more on the release binary, after the review fixes.

```
### 1 a run
rho: session 20260826-213448-0fbc at /tmp/rho-final/home/.rho/sessions/root-c3f84ecf/20260826-213448-0fbc.jsonl
The bug is: the parser reads a number but forgets to preserve the sign (positive or negative).
### 2 a resume
rho: continuing session 20260826-213448-0fbc in /tmp/rho-final/root (4 messages)
Sign.
### 3 twice
rho: continuing session 20260826-213448-0fbc in /tmp/rho-final/root (6 messages)
Sign.
### 4 the crash continue
last record: closed
rho: continuing session 20260826-213503-a557 in /tmp/rho-final/root (4 messages)
RECOVERED.
### 5 name a session, then resume it
named session 20260826-213503-a557 "the essay session".
STILL-READABLE.
ID                   LAST ACTIVE TITLE              MODEL           TOKENS  COST
20260826-213503-a557 just now    the essay session  us.anthropic.c…   3.1k     -
20260826-213448-0fbc just now    Read sample.txt a… us.anthropic.c…   2.1k     -
### 6 a delete while the session is live is refused
rho: session 20260826-213503-a557 is open in another process. Use another session, or close that one.
### 7 and once it is free, the delete works
deleted session 20260826-213503-a557. A session forked from it is its own file, so it stays. ...
```

A note on the lock, learned the hard way in this run. An earlier attempt reported `Busy` for a
minute, and the cause was **not** rho: a background run from an earlier shell was still streaming
and still held the lock. `flock` is per open file, and a probe confirmed the lock was free the
moment that process ended. So the refusal was correct every time, and the surprise was the shell.

The `WARN a record read from a file carried oversize content` line comes from the read-side cap
bounding the 900 word essay the crashed run recorded. That is `cap_entry_state` doing its job.

## 12. The review phase, and its mutation proofs

The suite was green, the step 7 proofs were recorded, and the work was committed. Then a review
phase ran, in this shape:

1. A code-graph brief, built with the `cbm` tools on **this** worktree. `cbm_index`, then
   `cbm_changes` for the blast radius, `cbm_architecture` for the boundaries, `cbm_trace` on
   `row_from`, and a Cypher query for exported items with no inbound call. The brief is what every
   reviewer read. **One caveat is written into it:** the graph's `CALLS` resolution for Rust is
   weak, and it lists `parse_header`, `walk_chain` and `check_integrity` as having no caller, which
   is false. So "no inbound calls" from the graph is not evidence.
2. Four subagents in parallel, one lens each: correctness and architecture, security, test quality,
   and documentation claims. Each got the same brief, the defect history, and the instruction to
   assume another defect of the same family exists.
3. `codex review`, from outside this harness, with **no sight of the subagent findings**, so its
   lens stayed independent.

### The proofs

| Break | Tests that failed |
| --- | --- |
| the reader does not check for a cycle | `a_cyclic_parent_chain_is_refused_rather_than_looping` |
| the walker does not check for a cycle | `a_hand_built_cyclic_walk_is_refused_rather_than_looping` |
| `depths` walks to the root for every record | `show_of_a_long_session_is_not_quadratic` |
| the session file mode is not on the open call | `a_session_file_is_0o600_on_unix`, `a_session_file_is_never_briefly_world_readable` |
| the sidecar mode is not on the open call | `a_sidecar_spill_file_is_0o600_on_unix` |
| a fork leaves a leaf as the head | `a_fork_at_a_leaf_record_leaves_a_usable_head` |
| a tool result is written without its call | `a_tool_result_is_never_written_without_its_call` |
| the tail window is unbounded | `a_row_stays_inside_the_tail_window_when_the_file_grew` |
| the `show` header cuts from the end | `show_keeps_the_state_when_the_model_is_long`, `show_keeps_the_model_and_the_state_when_the_title_is_long` |
| the model does not give way | `show_keeps_the_state_when_the_model_is_long` |
| the tool-result rule keys on the block shape | `show_never_prints_a_tool_message_body`, `show_full_never_prints_a_tool_message_body_either` |
| only a `Text` block is expired | `a_handle_hidden_in_a_reasoning_block_expires_too` |
| one preview per block is expired | the same |
| the promise line is kept | the same |
| a nested tag survives inside the kept head | the same |
| the lifecycle records no prompt | `the_whole_lifecycle_runs_in_order`, `the_prompt_is_recorded_before_the_answer_even_when_the_run_fails` |
| the lifecycle never closes | the same two |
| the printer gets no recording | `the_whole_lifecycle_runs_in_order` |
| a named session file takes no lock | `a_named_session_file_holds_a_lock` |
| a named session file skips the permission check | `a_named_session_file_cannot_widen_a_run` |
| a named session file does not replay | `a_named_session_file_replays_what_it_holds` |

`4000 records in 16.25ms`, from
`cargo test -p rho-cli --test sessions_print -- --nocapture show_of_a_long`. The same case at
O(N squared) is eight million map lookups and clones.

### Three breaks passed at first, and each one changed something

1. **The mode on the `open` call.** A `set_permissions` after it made the final mode the same, so
   deleting either changed nothing a test could see. That is the redundant-guard trap of
   `record_fits`, met for the fourth time on this branch. **The chmod is gone from the create paths**
   and the mode on `open` is the single mechanism, so the existing `0o600` tests now observe it. The
   lock file keeps its chmod, because a lock file can already exist and `OpenOptions::mode` applies
   at creation only.
2. **The tail bound.** `a_row_never_decodes_the_whole_file` passes a `size_bytes` equal to the real
   length, so the window end and the file end coincide and the bound is invisible. A new test hands
   the builder a **stale** `size_bytes`, which is exactly what a file that grew after the metadata
   read gives.
3. **The nested-tag scrub.** The first version replaced the opening marker only, so
   `handle="r-live"` survived inside the kept head. The test did not cover nesting, and a probe in a
   scratch crate outside the repository showed the leftover. Now every tag goes, and the test drives
   a nested preview.

### One break still passes, and it is stated rather than hidden

Wrapping the `record_and_print` call in `run_headless` inside `if false` passes the whole suite. The
lifecycle itself is now behaviour, and that last hop is a grep. This crate cannot inject a stub
provider, because `crates/rho-cli/src/provider.rs` belongs to another lane. Section 6 of this
document is the only guard on that hop, and section 16 of the spec says so.

## 13. The review fixes, driven for real

The release binary was rebuilt and driven again. Two of these cases came **out of** the drive, and
they are the reason step 11 runs after a review and not only before one.

### A widen refusal was becoming a warning

```sh
RHO_SESSION_FILE=/tmp/rho-dbg/n/ro.jsonl ./target/release/rho run "Say only A." --read-only ...
head -1 /tmp/rho-dbg/n/ro.jsonl   # approval: read-only  sandbox: off
RHO_SESSION_FILE=/tmp/rho-dbg/n/ro.jsonl ./target/release/rho run "Say only B." ...
```

Before the fix:

```
rho: cannot open a session file: a resume would widen approval from read-only to allow-all; pass --allow-widen to allow it. This run is ephemeral.
B.
```

The refusal became a warning and the run answered. `open_recording` degraded **every** failure on a
new session, and a named session file is opened with a new selector.

After the fix:

```
rho: a resume would widen approval from read-only to allow-all; pass --allow-widen to allow it
exit 1
```

**The rule is inverted now: only a write failure degrades.** A list of errors that stop the run is a
fail-open shape, because the next variant joins the degrade by default. The rule names what
degrades instead. Tests: `a_write_failure_on_a_new_session_degrades_the_run`,
`a_widen_refusal_on_a_new_session_stops_the_run`, `a_busy_session_stops_the_run`, all driving the
real `open_recording`.

### One crafted file blocked `--continue` for ever

A row comes from two bounded reads, and it never walks a parent link. So a file with a cyclic chain
still builds a **readable-looking row**. `--continue` chose it, and then failed on the full
read:

```
rho: the parent links of record a form a cycle, so this file cannot be read
```

Every later `--continue` failed the same way, and a user had to find and delete the file with no
hint. `newest_matching` now reads the candidate it is about to name, which costs one read that the
resume does anyway. After the fix, in the same poisoned store:

```
rho: continuing session 20260826-224828-8bef in /tmp/rho-rev/root (4 messages)
MOVED-PAST.
```

Test: `newest_resumable_skips_a_session_it_cannot_read`. It asserts first that the cyclic file
really does build a row, or the test would be vacuous.

### The rest of the sweep, after the fixes

```
### a run, a resume, and the same again
rho: session 20260826-224822-e79e at /tmp/rho-rev/home/.rho/sessions/root-63873f5c/20260826-224822-e79e.jsonl
The parser reads a number but forgets to handle the sign (positive or negative).
rho: continuing session 20260826-224822-e79e in /tmp/rho-rev/root (4 messages)
Sign.
rho: continuing session 20260826-224822-e79e in /tmp/rho-rev/root (6 messages)
Sign.

### the session-file key
rho: session 20260826-224829-b8e4 at /tmp/rho-rev/notes/mine.jsonl
FIRST.
rho: continuing the session file /tmp/rho-rev/notes/mine.jsonl (2 messages)   # it replays now
FIRST.
--- with --allow-widen ---
WIDENED.

### a cyclic file, with a 20 second deadline on each
rho: the parent links of record a form a cycle, so this file cannot be read     # sessions show
rho: the parent links of record a form a cycle, so this file cannot be read     # a named resume
20991231-235959-cafe just now    one                                     -     -   # the list still works
```

`sessions show` and a named resume both refuse a cyclic file in well under the deadline. Before the
guard they never returned.

### The lock on a named file, probed rather than timed

A first attempt looked like a failure and was a timing artefact: the background run had already
finished. So the lock was probed directly.

```
first run alive? yes
-rw-------  mine.jsonl
-rw-------  mine.lock
--- is the lock really held, probed with flock from python ---
flock: held by rho (EWOULDBLOCK)
--- a second run on the same named file ---
rho: session mine is open in another process. Use another session, or close that one.
exit 1
```

**A timing test is not a proof.** The probe is, and it is why this section shows the flock result
beside the refusal.

## 14. A test the controller destroyed, and the guard that caught it

`bench/check-claimed-tests.py` failed the first attempt to commit this review phase:

```
a commit message names `a_handle_hidden_in_a_reasoning_block_expires_too`, and no test of that name exists
VIOLATIONS 1 (range origin/main..HEAD)
```

The test was real, it had failed against the defect, and then a scripted edit deleted it. The edit
replaced a region of `crates/rho-cli/tests/session_cli.rs` using a slice from one index to another,
and the new test sat between the two anchors. The suite stayed green, because the deletion removed
the only thing that could fail.

**This is the second time in this lane that a slice-based edit destroyed committed work.** The first
took the tail of `crates/rho-cli/src/sessions.rs`, and a compile error caught that one within a
minute. A test has no compile error to catch it.

Two lessons, both written down rather than remembered:

- An edit that spans from one anchor to another deletes everything between them. Prefer a single
  anchored replacement, and read the diff.
- `check-claimed-tests.py` is the only guard that can see a lost test, and it earned its place here.
  The commit named the test, the tree did not hold it, and the gate refused. See the note on that
  checker in `AGENTS.md`.

The test was restored, and both of its mutation proofs were re-run:

| Break | Test that failed |
| --- | --- |
| only a `Text` block is expired | `a_handle_hidden_in_a_reasoning_block_expires_too` |
| a nested tag survives in the kept head | the same |

## 15. The printed examples in the spec are real output now

A documentation reviewer found the spec's own examples drifting from the code, and one of them
contradicted the contract: the `sessions show` header in section 8a printed `2 turns`, which section
5 forbids because a turn count needs a whole file. A drafted example in a delivered spec is the
stale-spec defect this project already has a decision about.

Both examples were replaced with real output, from the built binary over a seeded store:

```sh
HOME=/tmp/rho-spec/home ./target/release/rho sessions show 20260825-09 --root /tmp/rho-spec/root
HOME=/tmp/rho-spec/home ./target/release/rho sessions list --root /tmp/rho-spec/root
HOME=/tmp/rho-spec/home ./target/release/rho sessions list --long --root /tmp/rho-spec/root
```

```
session  20260825-094512-a3f9  "fix the parser"  claude-sonnet-4  closed
  r1    09:25:12  model        bedrock claude-sonnet-4
  r2    09:25:12  user         fix the parser
  r3    09:25:12  assistant    I will read the file first.
  r4    09:25:12  tool_call    read  path=src/parse.rs
  r5    09:25:12  tool_result  read  1.2 KiB
  r6    09:25:12  assistant    The bug is on line 42. Shall I fix it?
  r7    09:25:12  user         yes
  r8    09:25:12  assistant    Done. I changed one line.
  r9    09:25:12  stop         EndTurn
  r10   09:25:12  closed

ID                   LAST ACTIVE TITLE              MODEL           TOKENS  COST
20260825-094512-a3f9 just now    fix the parser     claude-sonnet-4      -     -
20260824-171003-77b2 just now    add the retry test claude-haiku-4    3.1k $0.01
20260823-092211-0c41 -           * unreadable: cannot decode a rec…      -     -

ID                   LAST ACTIVE TITLE              MODEL           TOKENS  COST  CWD  FORK
20260825-094512-a3f9 just now    fix the parser     claude-sonnet-4      -     -  /work/rho  -
20260824-171003-77b2 just now    add the retry test claude-haiku-4    3.1k $0.01  /work/rho  -
20260823-092211-0c41 -           * unreadable: cannot decode a rec…      -     -
```

Three things a reader can now check by eye. The first row shows a dash for its tokens, because that
session recorded no `Usage` record. The `tool_result` line names the tool and 1.2 KiB and never the
body. `--long` runs past 80 columns on purpose, and the default list does not.

## 16. What the security lens asked, and what it cost to answer

The security lens attacked six claims and found the cycle, the mode window, the tool-message body,
and the handle expiry. Sections 12 and 13 hold those fixes. It also asked one question this document
could only answer with a measurement: **what bounds a whole-file read?**

`rho sessions list` never reads a whole file. A resume does, because `branch_messages` walks parent
links and a walk needs the set. Two caps already bound that read, `MAX_LINE_BYTES` and
`MAX_DROPPED_RECORDS`, and neither bounds the record count.

So it was measured, with a scratch binary depending on `rho-core`:

```
records=20000   file=4.2 MiB   read=27.76ms   rss delta=12.9 MiB  per record=675 bytes
records=100000  file=21.2 MiB  read=102.93ms  rss delta=60.8 MiB  per record=637 bytes
```

**About 640 bytes per record, and about three times the file size, linear in both.** A session of a
thousand turns costs under a megabyte. `docs/benchmarks.md` holds the table, the harness, and the
commands, and `docs/guide/sessions.md` tells a user the same thing in their own terms.

The cap is **not** added, and the reason is written down rather than left as an omission: capping the
record count means deciding which end of a conversation to lose, and losing the front breaks its
beginning. That belongs with `F-context-compaction`, which section 10 of the spec keeps out of this
lane.

Two more of its findings were answered in prose rather than in code, and both are now stated where a
reader meets them:

- A project key has no length cap. A four kilobyte `gitdir:` line gives `ENAMETOOLONG`, which is a
  degrade and not a traversal. The sanitizer still holds: no separator and no `..` survives it.
- A `session-file` in a directory the user already owns keeps that directory's mode. rho sets `0o700`
  on a directory it creates and `0o600` on the file, and it does not chmod a directory it found. The
  guide says so, next to the rest of the privacy notes.
