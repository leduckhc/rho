# Verification — the queue bounds, driven for real

Date: 2026-08-26. This is AGENTS.md step 11. Every command and every output below is real.

Two providers ran: AWS Bedrock with `us.anthropic.claude-haiku-4-5-20251001-v1:0`, and
OpenRouter with `anthropic/claude-haiku-4.5`. Every path ran twice, because "twice" has caught
two defects in this project.

## What the change must do

A steering message had a count cap and no byte cap, so 32 messages could hold any amount of
memory. A queued child had no wait deadline, so one blocking `spawn_agents` call could hold a
parent's turn for the wait line depth times the child timeout. Both are recorded in
`.rho-work/progress.md`.

## 1. Setup

```sh
cargo build --release -p rho-cli
unset AWS_PROFILE
export HOME=/tmp/rho-bounds-e2e/home      # a fake home, so the real ~/.rho is never read
cd /tmp/rho-bounds-e2e/root               # a.txt, b.txt, c.txt, d.txt, and locked.txt
```

Two definitions live in `$HOME/.rho/agents`: `scout`, which reads one named file, and
`sleeper`, which waits for a steer.

## 2. The new flags exist

```sh
./target/release/rho --help | grep -E "queue-wait|agent-steer"
```

```text
      --queue-wait-secs <SECONDS>
      --max-agent-steer-bytes <BYTES>
```

## 3. The deadline breaks no happy path

A fan-out of four under a cap of one. Three tasks wait, and the default deadline is 600
seconds, so all four run.

```sh
rho run "Use spawn_agents once, with four tasks, to send the scout agent at a.txt, b.txt, \
c.txt and d.txt, one file per task. Then say in one line what each child reported." \
  --provider bedrock --model $MODEL --max-children-per-parent 1
```

```text
rho: 2 agent definition(s) available to spawn_agent: scout, sleeper.
**Results:** scout-a found "alpha file", scout-b found "beta file", scout-c found "gamma
file", and scout-d found "delta file".

real	0m18.931s
```

The second run:

```text
scout-a reports a.txt contains "alpha file"; scout-b reports b.txt contains "beta file";
scout-c reports c.txt contains "gamma file"; scout-d reports d.txt contains "delta file".

real	0m17.853s
```

## 4. A deadline of zero refuses every waiter, and the refusal teaches

```sh
rho run "Use spawn_agents once, with four tasks, ... Then report exactly which tasks \
succeeded and quote any refusal text you got, verbatim." \
  --provider bedrock --model $MODEL --max-children-per-parent 1 --queue-wait-secs 0
```

```text
- **scout-a (a.txt)**: Succeeded. Report: "I read the file a.txt and found that it
  contains the text 'alpha file'."
- **scout-b (b.txt)**: Failed. Refusal: "the child waited 0 seconds for a slot and none
  freed, so the work was not started. Run it again later, spawn it with background: true,
  or ask the user to raise --queue-wait-secs."
- **scout-c (c.txt)**: the same refusal.
- **scout-d (d.txt)**: the same refusal.

real	0m8.142s
```

The second run quoted the same text, three times, and took 8.042 seconds. One task ran, and
the call returned in eight seconds rather than holding the turn.

## 5. The byte cap refuses a model's own steer

```sh
rho run "Spawn the sleeper agent with spawn_agent and background true. Call steer_agent \
once with a message of exactly this text: Please read a.txt and then tell me in one \
sentence what it holds. If that call is refused, report the refusal text verbatim and \
stop. Do not retry." --provider bedrock --model $MODEL --max-agent-steer-bytes 32
```

```text
The call is refused with this refusal text:
"the message is 65 bytes and the limit is 32 bytes. Send a shorter message, or write the
detail to a file and name the file."
```

The second run reported the same text.

The count charges 64 bytes for the block as well as its payload, so the binary was rebuilt and
this path ran twice again, with a cap of 100 bytes:

```text
"the message is 129 bytes and the limit is 100 bytes. Send a shorter message, or write the
detail to a file and name the file."
```

A steer with no flag still works, so the default cap refuses nothing a model normally writes:

```text
"queued for sleeper (id 1), at position 1. The child reads it after its current tool calls
finish."
```

An earlier pair of runs let the model retry. It read the refusal, wrote the detail to a file,
and steered with the file name. So the advice in the message is advice a model can act on.

## 6. A background waiter that ran out still owes a report

```sh
rho run "Spawn the scout agent twice with spawn_agent and background true, one at a.txt \
and one at b.txt. Then call agent_status with no argument, and then call agent_status for \
each id you were given. Report each answer verbatim." \
  --provider bedrock --model $MODEL --max-children-per-parent 1 --queue-wait-secs 0
```

```text
agent_status(id=1): scout (id 1) is running: 2 turn(s), 798 token(s).
agent_status(id=2): scout (id 2) finished: failed: the child waited 0 seconds for a slot
and none freed, so the work was not started. Run it again later, spawn it with
background: true, or ask the user to raise --queue-wait-secs.. 0 turn(s), 0 token(s).
```

**This run found a defect, and the fix is in this branch.** The reason ends with a full stop,
and `agent_status` added a second one: `--queue-wait-secs.. 0 turn(s)`. `AgentOutcome::label` builds a phrase, and the
caller builds the sentence, so the phrase must not end one. It is fixed, and
`a_failed_label_is_a_phrase_and_not_a_sentence` pins it. The same fix covers every failed
outcome, not only this one. After the fix, both runs printed one stop:

```text
scout (id 2) finished: failed: the child waited 0 seconds for a slot and none freed, so the
work was not started. Run it again later, spawn it with background: true, or ask the user to
raise --queue-wait-secs. 0 turn(s), 0 token(s).
```

## 7. OpenRouter, the same two paths

```sh
rho run "Use spawn_agents once, with three tasks: scout at a.txt, scout at nope.txt, and \
scout at b.txt. Then report for each task whether it succeeded, and quote any refusal text \
verbatim." --provider openrouter --model anthropic/claude-haiku-4.5 \
  --max-children-per-parent 1 --queue-wait-secs 0
```

```text
1. **a-scout (a.txt)**: Succeeded.
2. **nope-scout (nope.txt)**: Failed. Refusal text: "the child waited 0 seconds for a slot
   and none freed, so the work was not started. Run it again later, spawn it with
   background: true, or ask the user to raise --queue-wait-secs."
3. **b-scout (b.txt)**: the same refusal.
```

Both runs read the same. So the refusal is not one provider's rendering.

## 8. The failure paths still belong to the child

With the default deadline, every queued child starts and reports its own error. An absent
file, a file with mode `000`, and a readable file, all over a cap of one:

```sh
rho run "Use spawn_agents once, with three tasks: scout at nope.txt, scout at locked.txt, \
and scout at a.txt. Then say for each task in one line what happened." \
  --provider openrouter --model anthropic/claude-haiku-4.5 --max-children-per-parent 1
```

```text
1. **nope.txt**: File does not exist at that location.
2. **locked.txt**: File exists but cannot be read due to permission denied (error 13).
3. **a.txt**: Successfully read and contains the text "alpha file".
```

The second run said the same. So the deadline hides no child error, and a waiter that starts
keeps its whole budget.

## 9. The deliberate breaks

Twenty-nine mutations ran, each with the file copied to `/tmp` first and copied back after.
Never `git checkout`. Every one was caught by the named tests, and the restored file passed
again.

| The break | The tests that failed |
| --- | --- |
| `push` measures the message and ignores the result | 5 byte-cap tests |
| the count skips a `ReasoningReplay` payload | `the_counted_size_covers_every_block_kind` |
| the count skips a tool call's replay payload | `the_counted_size_covers_every_block_kind` |
| `with_limits` raises a small cap to the default | 3 byte-cap tests |
| the deadline timer reads `child_timeout` | 3 deadline tests |
| a wait that ran out reports `Cancelled` | 3 deadline tests |
| the deadline arm comes before the cancel arm | `a_cancel_beats_the_deadline` |
| a timed-out waiter is marked handed out | 2 drop-guard tests |
| a queued child's queue ignores the limits | `a_child_queue_carries_the_byte_cap_from_the_limits` |
| a started child's queue ignores the limits | `a_started_child_queue_carries_the_byte_cap_from_the_limits` |
| one byte cap for both kinds of queue | `the_child_byte_cap_is_smaller_than_the_session_one` |
| the deadline default ignores `--child-timeout-secs` | `an_unset_queue_wait_follows_the_child_timeout` |
| `--max-agent-steer-bytes` is parsed and ignored | `the_agent_steer_byte_flag_reaches_the_limits` |
| a timed-out waiter is a cancel for the parent | `a_waiter_that_ran_out_of_patience_is_a_failure_that_names_the_wait` |
| a failed label keeps its full stop | `a_failed_label_is_a_phrase_and_not_a_sentence` |
| a block is free to hold, so only payload counts | `a_block_is_never_free_to_hold` |
| the count walks a value of any depth | `a_value_too_deep_to_count_is_refused` |
| an object key is free | `no_json_node_is_free_to_hold` |
| a null node is free | `no_json_node_is_free_to_hold` |
| an object loses its own floor | `no_json_node_is_free_to_hold` |
| an object entry costs nothing of its own | `no_json_node_is_free_to_hold` |
| an array loses its own floor | `no_json_node_is_free_to_hold` |
| a string value is free | `no_json_node_is_free_to_hold`, `the_counted_size_covers_every_block_kind` |
| the block walk has no depth guard | `a_nested_tool_result_too_deep_to_count_is_refused` |
| the nested walk forgets the depth | `a_nested_tool_result_too_deep_to_count_is_refused` |
| the queue keeps the caller's allocation | `the_queue_keeps_no_capacity_the_message_did_not_need` |
| a nested block list keeps its room | `the_queue_keeps_no_capacity_the_message_did_not_need` |
| the shrink walk has no guard of its own | `the_shrink_walk_stops_where_the_count_stops` |
| every trailing stop is eaten | `a_failed_label_is_a_phrase_and_not_a_sentence` |
| a full queue answers before an oversized message | `an_oversized_message_is_too_large_and_not_merely_full` |

The harness is `/tmp/rho-mutations/prove.py`. It reports `ESCAPED MUTATIONS 0`.

## 10. The review round, and what four lenses plus an outside tool found

Four reviewers read the change in parallel, one lens each, and `codex review` read it from
outside this harness. Three findings were real, and each is fixed with a test and a proved
mutation.

- **An object key was counted, and no test said so.** A test lens deleted the key charge, and
  the whole suite of 1532 tests stayed green. A message of one 60 KiB key would then count ten
  bytes and pass a 64 KiB cap. The count now has one named floor, `JSON_NODE_MIN_BYTES`, and
  `no_json_node_is_free_to_hold` pins a key, a string, an empty container, and a node.
- **The block walk had no depth guard while the value walk did.** Two lenses and codex found it
  independently. A probe outside the repository proved that an unguarded walk aborts the
  process: `thread 'main' has overflowed its stack`. `block_bytes` now carries the same guard.
- **A count reads a length, so a caller's spare room was invisible.** codex found it. A probe
  showed a `Vec` of capacity 100000 holding one block, and a `String` of capacity 1000000
  holding two bytes. `push` now drops that room before it stores the message.

One charge turned out to be redundant, and a deliberate break proved no test could see it. The
array element charge is deleted, per AGENTS.md step 6, because every element costs the node
floor already.

A second charge looked redundant and was not. A re-review of the fix showed that deleting the
object entry charge let a map of short keys and empty values count four bytes an entry, while a
real map entry costs tens. It is restored, and the assertion that pins it uses an **empty** key.
With any key at all the key charge alone makes an object dearer than an array, so the first
version of that assertion could not see the entry charge, and a break proved it.

**One process note.** `codex review` ran while the mutation harness was running, so it read a
file in a mutated state and reported an unused variable that no commit ever held. Its other two
findings were real. Do not run an outside reviewer and a mutation harness over one working tree
at the same time.

## 11. The pull request review

A reviewer read the branch on pull request 6. CodeRabbit had hit its free-plan rate limit, so it
reported nothing, and the human read was the first outside read of the whole branch. It confirmed
the gate, the arithmetic, and one deliberate break of its own. It raised two nits, and both were
real:

- **The depth guard was indirect, and one setting removed it.** The walk that drops spare room
  had no guard of its own. It was safe only because a too-deep message counts `usize::MAX` and
  `push` then refuses it. A host that sets the cap to `usize::MAX`, through `with_limits` or
  `--max-agent-steer-bytes`, stops that refusal working, and the walk would then recurse without
  a bound. Both walks now carry the guard, so no caller can arrange the failure.
  `the_shrink_walk_stops_where_the_count_stops` proves it without a crash: what the walk reaches
  is shrunk, and what lies past the depth keeps its room.
- **`AgentOutcome::label` stripped every trailing stop, not one.** A reason that ended in an
  ellipsis lost all three dots, so it stopped saying that it trailed off. It now removes one
  stop. The test covers an ellipsis, one stop, and no stop.

Both fixes were proved by a break: `the-shrink-walk-has-no-guard` and
`every-trailing-stop-is-eaten`, each caught by its named test.

**One caveat the reviewer raised and I did not close.** `live_permits` is per `AgentRegistry`,
so the 80 MiB ceiling is per registry. A host that opens many registries in one process holds
that many ceilings. The code says so already, and no test drives many registries.

The last two rows of the table above come from a second review, which asked what the count
misses. A message of
ten thousand empty blocks counted nothing and held ten thousand allocations. And a value
nested deeper than the walk would have ended the process on the stack. Both are closed, and
both have a test.

Two of those breaks were found by this step, not by the first draft of the tests. The timer
that read `child_timeout` escaped, because the test asserted the reason and not the moment.
The tests now assert when the wait ended, and both defaults differ in every deadline test.
