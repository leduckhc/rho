# Task — finish SPEC-reasoning-across-providers

Working notes for branch `feat/reasoning-across-providers`. Spec:
`docs/specs/20260819-134615-SPEC-reasoning-across-providers.md`.

## The task, in one sentence

Make rho show a model's reasoning correctly, and replay it correctly, on every provider.

## Why the work is open

Another agent wrote 1668 lines, then stopped. Commit `e08b190` saved that work and vouched
for none of it. This file states what is done, what is not, and what must be decided first.

The gate now passes on the saved work: `fmt` clean, `clippy` clean, 855 tests pass, the
minimal build links, `check-ids.py` reports 0, `check-prose.py` reports 0. No `todo!()`
anywhere in `crates/*/src`.

**A green gate is not a finished feature here.** The saved work is the display half. The
contract half is missing, and three defects sit inside the part that looks done.

## Lane

Step 0 of `AGENTS.md`: this is **a feature**, so every step applies. It is also a
**contract** change, so step 3 comes before any code, and step 9 reviews the contract on
its own before either side implements it.

Two of the requirements below are **bug fixes** on already-committed code. They take the bug
fix lane: the reproducing test lands first, and step 7 is the whole point.

## What is done

| Part | Where | State |
| --- | --- | --- |
| Three-name reasoning reader | `rho-provider-openrouter/src/lib.rs` | done, 4 tests |
| `ThinkingSplitter`, the tag rule | `rho-core/src/thinking.rs` | done, 11 tests |
| `ReasoningDisplay`, four modes | `rho-core/src/reasoning.rs` | done, 3 tests |
| Splitter wired into the reducer | `rho-tui/src/state.rs` | done, 3 tests |
| The four display modes drawn | `rho-tui/src/render.rs` | done, 5 tests |
| Config key `tui-reasoning` | `rho-config/src/lib.rs` | parses, then is discarded — see R7 |
| `--reasoning` flag | `rho-cli/src/cli.rs` | flag and env only — see R7 |
| Named arms, no `_ => {}` | `rho-provider-bedrock/src/lib.rs` | done, 1 test |

## Requirements

Each one names its spec section and its proof. `R1` to `R4` are the contract, and they are
blocked until the contract review passes. `R5` to `R7` are independent, and `R6` and `R7`
are confirmed defects that may be fixed first.

### R1 — Split the data model into a trace and a replay block

Spec section 4. `ContentBlock` in `rho-core/src/content.rs:24` still holds the old
`Thinking { thinking, signature }`. The spec replaces it with two variants, because the old
name never said which blocks travel, and that is why one was dropped in silence.

- `ReasoningTrace { text }` — for the reader, and it never reaches a provider.
- `ReasoningReplay { text, signature: Option<String> }` — echoed back when the wire asks.
- Add `thought_signature: Option<String>` to `ToolCall`, so a Gemini crate needs no edit to
  shared code later.
- `signature` stays `Option<String>`. An earlier draft used `""` for none, and a review
  named that a fail-open default.

Proof: `a_trace_never_reaches_a_provider`, and `every_content_block_has_an_explicit_arm`
must still hold in every provider after the variants change.

### R2 — Add the replay policy, as data

Spec section 3 "One". `ReplayPolicy` and `ReasoningWire` do not exist anywhere in the tree.
This is the cornerstone, and guessing fails in both directions: Kimi and DeepSeek **require**
the replayed field, and Mistral answers 422 `Extra inputs are not permitted` without it.

- `ReasoningWire { read_fields, replay, rejects_unknown_fields, ask_to_enable }`.
- `ReasoningReplay` with `Never`, `SignedWhileSameModel`, `TextOnToolCall`,
  `SignatureOnToolCall`.
- The table governs the OpenAI-compatible family only. Reading stays per-provider code,
  because Gemini marks a part with `thought: true` and no field name selects that.

Proof: `never_replay_sends_nothing`, `a_rejecting_endpoint_receives_no_reasoning_field`,
`a_tool_call_endpoint_receives_the_text`, `a_signature_replays_on_the_matching_tool_call`,
`a_signed_block_replays_for_the_same_model`, `a_signed_block_is_dropped_for_another_model`.

### R3 — Ask Bedrock for thinking

Spec section 0 defect 1, and `ask_to_enable`. This is the defect the owner reported. rho
never asks for extended thinking, so Claude writes `<thinking>` tags into ordinary text.
`rho-provider-bedrock/src/lib.rs` currently drops reasoning in a named arm and says replay
is phase 2.

Proof: `ask_to_enable_adds_the_thinking_request`.

Note: the tag rule in R5 is the safety net for a model that writes tags anyway. It is not a
substitute for asking, and the spec treats them as two separate fixes.

### R4 — Make the persisted format work in both directions

Spec section 4, "The persisted format". Untested today, and a session file binds every other
version of rho.

- A new rho reads `"type": "thinking"` as `ReasoningTrace`, and it never replays a stale
  signature from an older session.
- An old rho must still load a new file, so the new variants serialise under the old
  `"type": "thinking"` tag with an added `replay` key.
- A missing `replay` key reads as `false`, which is the safe direction.
- A truncated final line is dropped, and the session still opens.

Proof: `an_old_thinking_block_imports_as_a_trace`, `a_new_block_is_readable_by_an_old_rho`,
`a_missing_replay_key_reads_as_false`, `a_truncated_final_line_does_not_stop_the_load`,
`the_reasoning_text_is_stored_in_every_mode`.

### R5 — Keep an orphaned tool call whole

Spec section 6 rule 5. An orphaned tool call gets a synthetic error result, so the signature
chain stays whole. pi does this, and its comment says it "satisfies API requirements".

Proof: `an_orphaned_tool_call_gets_a_synthetic_result`.

### R6 — Stop the CLI swallowing a bad reasoning mode

**A confirmed defect, and the two paths disagree with each other.**

- `rho --reasoning loud` silently becomes `summary`. See
  `resolve_reasoning` at `rho-cli/src/cli.rs:428`, where `.ok()` discards the error.
- `RHO_TUI_REASONING=loud` silently becomes `summary`, by the same `.ok()`.
- `tui-reasoning = "loud"` in a config file is a hard error. See
  `a_bad_reasoning_value_fails_closed` in `rho-config/tests/reasoning.rs:51`.

The spec requires `an_unknown_reasoning_mode_is_refused`, and says "it does not fall back in
silence". The committed code and its test `a_bad_flag_value_falls_back_to_summary` assert the
opposite, and the doc comment defends the fallback.

So this needs a **decision before a fix**: refuse everywhere, as the spec says, or fall back
everywhere with a warning on stderr. Either answer is defensible, and the current split is
not. Whichever wins, the spec, the code, and one of the two tests must change together.

### R7 — Make the config file reach the screen

**A confirmed defect.** `rho-config` parses `tui-reasoning`, validates it, and stores it in
`Config::reasoning` at `rho-config/src/lib.rs:175`. Nothing ever reads that field.
`resolve_reasoning` re-reads the environment and never consults the loaded config, so a
config-file setting is parsed and then thrown away.

The spec test is named `the_flag_beats_the_config_and_the_environment`, and the config leg of
that precedence chain does not exist.

Proof: `the_flag_beats_the_config_and_the_environment`, plus a test that a config file alone
changes the drawn mode.

### R8 — Reconcile the spec's test names with the tree

Step 13. The spec names 38 tests. 14 match the tree by name, and 9 more exist under a
different name, so the spec and the code disagree about what is proven.

| Spec name | Name in the tree |
| --- | --- |
| `a_mixed_case_tag_is_stripped` | `a_mixed_case_tag_opens_a_block` |
| `an_opening_tag_split_across_three_deltas_matches` | `an_opening_tag_split_across_three_deltas` |
| `a_self_closing_tag_opens_no_region` | `a_self_closing_tag_is_an_empty_block_and_keeps_the_rest` |
| `an_unclosed_tag_stops_at_a_tool_call` | `an_unclosed_tag_before_a_tool_call_keeps_only_the_reasoning` |
| `a_reasoning_delta_splits_on_a_character_boundary` | `a_multibyte_reasoning_body_is_not_corrupted` |
| `the_reasoning_mode_parses_every_value` | `every_name_round_trips` |
| `an_unknown_reasoning_mode_is_refused` | `an_unknown_name_is_an_error`, and see R6 |
| `the_flag_beats_the_config_and_the_environment` | `the_flag_wins_over_the_env_var`, and see R7 |
| `a_stripped_tag_does_not_change_the_request` | absent, and `no_text_is_emitted_before_the_decision` too |

Pick one name per test, and make the spec and the tree agree. Add the two absent tests.

### R9 — Prove the saved work catches its own defects

Step 7, and it was never run on any of this. The commit says so. For each of the 14 landed
tests, break the implementation on purpose and watch the test fail.

Copy the file to `/tmp` first. **Never restore with `git checkout`**, because it destroys
every uncommitted change in that file. See `D-jcode-bash-lessons`.

The suspect ones are the display tests, because a renderer test can assert a row exists
while the mode logic is inverted.

### R10 — Drive it for real, then write it down

Step 11, and there is no `docs/verification/` file for reasoning. No fixture can catch this
class of defect, because a fixture describes a response and these defects are in the
**request**. Sprint 1 lost a week to exactly that.

- Build `cargo build --release -p rho-cli`, then run a real prompt on Bedrock with a Claude
  model, and confirm no `<thinking>` tag reaches the screen as the answer.
- Run a **two-call tool loop**, because Bedrock worked for one tool call and returned 400 for
  two.
- Exercise every provider the change touches. OpenRouter and Bedrock are both in the diff.
- Ask the model to print a literal `<thinking>` tag, and confirm it prints.
- Write the commands and the real output into `docs/verification/`.

### R11 — Update the docs last

Step 13. `docs/features.md` has one reasoning mention, on the OpenRouter row, and it claims
"Reasoning tokens pass through". Add the rows this work creates, with the owning crate and
the extension point. Record R6's decision in `.rho-work/decisions/`. Delete any claim the
tree cannot prove.

## Order of work

1. R6 and R7 first. They are confirmed defects on committed code, they are small, and R6
   needs a decision that nothing else waits on.
2. R9 next, on the 14 landed tests. Do this before building on them.
3. Write the contract for R1 and R2 in the spec, as compilable Rust. Review it on its own,
   with the one question from step 9: does a new case need an edit to shared code?
4. Then R1, R2, R3, R4, R5 behind the reviewed contract.
5. R8, R10, R11 to close.

## Open question for the owner

R6 needs a ruling: does an unknown reasoning mode refuse, or warn and fall back? The spec
says refuse. The committed CLI falls back and defends it in a comment. I will not pick for
you, because the spec and the code each have a test asserting the opposite of the other.
