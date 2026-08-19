# Verification — subagents, driven for real on Bedrock

Date: 2026-08-19. This is AGENTS.md step 11, "drive it for real". Every command and every
output below is real. The provider is AWS Bedrock. The model is the latest haiku, which is
cheap enough to run a full sweep.

```sh
cargo build --release -p rho-cli
unset AWS_PROFILE   # this machine had AWS_PROFILE=alquist, which does not exist
MODEL=us.anthropic.claude-haiku-4-5-20251001-v1:0
```

The model id came from the live catalogue, not from memory:

```sh
aws bedrock list-inference-profiles \
  --query "inferenceProfileSummaries[?contains(inferenceProfileId,'haiku-4-5')].inferenceProfileId" \
  --output text
# us.anthropic.claude-haiku-4-5-20251001-v1:0
# global.anthropic.claude-haiku-4-5-20251001-v1:0
```

## The test bed

The session root is `/tmp/rho-e2e/root`, so no test touches the repository. It holds
`sample.txt` and three definitions in `.rho/agents/`, which makes them **project**
definitions and therefore subject to the trust gate.

| Definition | `tools` | Purpose in the sweep |
| --- | --- | --- |
| `scout` | `read, grep, list` | The read-only child. |
| `greedy` | `read, bash, write, edit, spawn_agent, nonexistent_tool` | Asks for more than the parent holds. |
| `nodesc` | `read` | Has no `description`, so it must not load. |

## Provider smoke test

```sh
./target/release/rho run "Reply with exactly the word READY and nothing else." \
  --provider bedrock --model $MODEL --root /tmp/rho-e2e/root
# READY
# real 0m1.463s
```

## What passed

**The project trust gate holds.** Without `--trust-project`:

```
rho: 2 project agent definition(s) were found and not loaded: greedy, scout. A definition
carries a tool list and a model, and it runs unattended, so one from this repository stays
off until you trust it. Pass --trust-project.
```

Two, not three. So `nodesc` did not load at all, and
`a_definition_without_a_description_does_not_load` holds in the shipped binary.

**Delegation works.** With `--trust-project`, a `spawn_agent` call for `scout` returned the
file contents to the parent in about 7 seconds.

**The tool set really is intersected.** `greedy` asked for six tools and reported four:

```
- read
- write
- edit
- bash

[note: these requested tools were dropped because the parent does not hold them:
spawn_agent, nonexistent_tool]
```

Both `spawn_agent` and the unknown name were dropped, and **the drop was reported to the
caller**. That is F-tool-set-intersection, live.

**The intersection is enforced, not advertised.** `scout` was told to create `breach.txt`
with any tool it had. It answered that it cannot write, and `ls` confirmed no file appeared.

**A read-only parent refuses to delegate.** With `--read-only`, the `spawn_agent` call was
denied by the approval policy, and the session continued and answered. This confirms the
claim in `SPEC-subagents` section 10.

**An unknown agent name is a result, not a fault.** `agent="ghost"` produced a normal reply
and the session stayed open.

**An absent file is a result, not a fault.** `scout` was told to read a file that does not
exist. It reported the failure, and the parent continued and printed `CONTINUED`.

**Twice works.** AGENTS.md says "twice" has caught two defects here, so the sweep spawned
two children in one session. They took ids `agent-1` and `agent-2`, both answered, and the
slot freed between them.

**Context isolation holds.** This was the decisive test. `scout` read a file holding
`MARKER-TOKEN-ZQX7419` and was told not to repeat the token. The parent was then asked for
the token. It answered:

```
TOKEN NOT VISIBLE TO ME.
```

So the child transcript did not reach the parent context.

**Credentials do not cross into a child.** The parent process held a real
`AWS_SECRET_ACCESS_KEY`. A child with `bash` ran `printenv AWS_SECRET_ACCESS_KEY || echo
ABSENT_FROM_CHILD_ENV` and returned:

```
ABSENT_FROM_CHILD_ENV
```

So D-bash-scrubs-credentials holds across the subagent boundary.

## Two defects the sweep found, both now fixed

### Defect one: the model could not learn which agents exist

Asked to list the agents it could spawn, the model named three that do not exist, taken from
unrelated skill text. It never named `scout` or `greedy`.

The cause was in `rho-tools`. The schema took a free string:

```rust
"agent": { "type": "string", "description": "The agent definition name." }
```

And `description()` returned a fixed sentence. So the definition's own `description` was
parsed, required, and then **never sent to the model**. `SPEC-subagents` section 5 says the
field exists because "the model reads this to choose". It could not.

The fix adds a JSON Schema `enum` of the loaded names, and lists each agent with its purpose
in the tool description. After the fix, the same prompt named `scout` and `greedy` with
their real purposes.

Guarded by `the_schema_offers_only_the_agents_that_loaded` and
`the_description_carries_each_agent_purpose`.

### Defect two: no transcript was ever written

Every spawn logged this:

```
WARN rho_core::subagent: cannot write the child transcript: No such file or directory
(os error 2) path=/tmp/rho-e2e/root/.rho/agent-transcripts/agent-1.log
```

Nothing created the directory. `AgentReport.transcript` was therefore always `None` on a
real run, and section 6 of the spec advertises the transcript as an in-process advantage.

**The existing test passed against this bug**, which is the failure mode AGENTS.md step 7
warns about. `a_child_transcript_never_enters_the_parent_context` writes into a `tempdir`
that already exists, so it never met the missing directory.

The fix creates the parent directory before the write. After the fix:

```sh
ls -la /tmp/rho-e2e/root/.rho/agent-transcripts/
# -rw-r--r--  1587  agent-1.log
grep -oE "ToolStart|ToolEnd|TurnStart" .../agent-1.log | sort | uniq -c
#    1 ToolEnd
#    1 ToolStart
#    2 TurnStart
```

Guarded by `a_transcript_writes_into_a_directory_that_does_not_exist_yet`, which asserts the
parent directory is absent first, so it cannot pass for the wrong reason.

## Both guards were proved

Each fix was broken on purpose, and each test failed. The good files were copied to `/tmp`
first and copied back, never restored with `git checkout`. See D-jcode-bash-lessons.

| Break | Result |
| --- | --- |
| Remove `"enum": self.names` | `the_schema_offers_only_the_agents_that_loaded` FAILED |
| Remove the purpose from the description | `the_description_carries_each_agent_purpose` FAILED |
| Remove `create_dir_all` | `a_transcript_writes_into_a_directory_that_does_not_exist_yet` FAILED |

## Round two: fixing the limits found three more defects

The first round left the limits unexamined. A second pass drove them, and each one taught
something.

### Defect three: every limit refusal named a flag that does not exist

`rho-core` told the user to raise `--max-agent-depth`, `--max-children-per-parent`, or
`--max-live-agents`. None of the three existed, and `rho-cli` hardcoded
`SubagentLimits::default()`. A refusal must teach, and these taught something impossible.

A unit test even asserted the text `--max-agent-depth`, so the suite pinned the wrong
message. That assertion was changed, and the reason is recorded in the test itself.

The fix adds `--max-children-per-parent`, `--max-live-agents`, and `--child-timeout-secs`.
The depth refusal now names no flag, because no flag can help. See D-cli-depth-is-zero.

### Defect four: my own first fix broke the feature completely

The first attempt set the command-line depth to 0, on the reasoning that a command-line child
cannot spawn. Three unit tests passed. The product then refused **every** spawn:

```
the depth limit is 0 and this would be depth 1. Do the work here.
```

The root session is itself depth 0, so a cap of 0 refuses the first child. The spec says as
much: "A depth of 0 forbids spawning." The honest value is 1, which lets the root delegate
once and stops a grandchild.

This is the AGENTS.md step 11 lesson repeating inside one sitting. Unit tests passed while
the product was unusable, and only a real run showed it. The replacement test now drives a
real `AgentRegistry`, spawns a child, and asserts the grandchild is refused. It fails if the
depth returns to 0.

### Defect five, the worst: a child timeout killed the parent session

A child given a three second timeout and a thirty second `sleep` produced this:

```
$ rho run '...' --child-timeout-secs 3
I'll spawn the slowpoke agent with that command.
$ echo $?
0
```

No answer, no error, exit 0. Two causes, both in `rho-tools`:

1. **The child shared the parent's cancel token.** `collect_report` cancels the token when a
   child passes its timeout, so the cancel reached the parent and ended the whole run.
   `SPEC-subagents` section 8 wants one direction: a cancelled parent stops every descendant,
   and a child must not stop its parent.
2. **`report.outcome` was never read.** The tool returned only `report.summary`, and a timed
   out child has an empty summary. So the tool returned an empty string, and the model had
   nothing to act on. Section 6 of the spec says the parent receives the outcome and the
   usage.

The fix adds `CancelToken::child`, a token that follows its parent one way, and the tool now
states the outcome whenever it is not plain success. After the fix:

```
The slowpoke subagent was cancelled after hitting its 3-second timeout because the
30-second sleep operation was too long for the child agent's execution window.
```

The parent survived, reported, and answered.

### The first version of that guard was weak, and the break test caught it

The first timeout test used an empty provider script. That makes the child fail fast, which
is a different branch, so the test passed while the cancel bug was still live. Breaking the
implementation on purpose is what exposed it. The test now uses a provider that never yields
and a fifty millisecond timeout, and it fails on either half of the bug:

| Break | Result |
| --- | --- |
| Share the parent token again | FAILED: "a child ending must never cancel the parent session" |
| Drop `report.outcome` again | FAILED: "a failed child must return something the model can act on" |

See decision D-two-weak-tests, which names this family.

## Which limits can actually trigger

The agent loop runs tool calls **one at a time, in call order**. See
`AgentLoop::dispatch` in `crates/rho-core/src/agent.rs`. So one session never holds two live
children, and `ChildSlot` frees on drop between sequential spawns. A live run with
`--max-children-per-parent 1` therefore ran two children in a row without a refusal, which is
correct: the cap counts children running **at once**.

| Limit | Triggers from one CLI session? | Why |
| --- | --- | --- |
| `child_timeout` | **Yes**, verified live | A slow child is cancelled and reported. |
| `max_children_per_parent` | No | Children are sequential, so one is live at a time. |
| `max_live_total` | No | It needs several sessions in one process. |
| `max_depth` | No | A command-line child holds no spawn tool. |

That is not a bug, and it is worth stating plainly: **rho does not fan out within one
session.** The four limits are for a host that runs many sessions in one process, which is
rho's density claim, and for a library caller that spawns children concurrently. The spec's
own words, "a fan-out of fifty is ordinary", describe sessions in a process, not children in
a turn.

## Three things this sweep could not reach

These are not defects. They are limits of the shipped wiring, and they are recorded so
nobody claims coverage that does not exist.

**A grandchild is impossible through the CLI.** `rho-cli` captures the parent tool set
before `spawn_agent` joins it, so no child ever receives `spawn_agent`. That is deliberate
defence in depth, and `crates/rho-cli/src/cli.rs` says so. The effect is that
`SubagentLimits::max_depth`, which defaults to 2, cannot be exercised from the CLI. The
depth cap is covered by unit tests only.

**The restrictive half of `BothPolicies` is unreachable through the CLI.** The composed
policy only bites when the parent is more restrictive than the child. The only restrictive
CLI parent is `--read-only`, and that denies `spawn_agent` outright. So the composition is
present and unit-tested, but a live run cannot show it refusing a child's write.

**A child failure caused by a dead child was not reproduced.** The sweep covered an absent
file, a denied write, a denied spawn, and an unknown agent. It did not kill a child mid-run,
so salvage and the retry cap remain unit-tested only.

## Round three: the fan-out, and two more defects

Round two showed that only the timeout could trigger. Round three made the caps reachable.

### Defect six: the retry cap had no caller

`RetryLedger` was exported, documented, and unit-tested, and **nothing used it**. So a
poisoned task could be re-delegated forever. `F-salvage-and-retry-cap` claims the cap holds,
and the first half of that row worked while the second half was decoration.

This is the gap the module comment in `crates/rho-cli/src/subagents.rs` warns about in its
own words: a tool that no session registers is not shipped. The same is true of a guard.

The ledger is now wired into the spawn path, keyed by the agent **and** the work, so two
different tasks for one agent do not share a count. Guarded by
`a_repeatedly_dying_child_is_refused_at_the_retry_cap`, which fails when the wiring is removed.

### The fan-out, and why it is one tool call

`AgentLoop::dispatch` runs tool calls one at a time. Making it concurrent would apply
concurrency to `edit`, `write`, and `bash`, race two approval prompts onto one terminal, and
make the append-only log non-deterministic. So concurrency is confined to subagents, in a new
`spawn_agents` tool that runs a list of tasks together inside one call. See
`D-fan-out-is-one-tool-call`.

Three children in one call, live:

```sh
rho run 'Use spawn_agents once with three tasks: scout on a.txt, scout on b.txt,
         scout on c.txt. Report each result.' --child-timeout-secs 600
```

```
1. a.txt: Contains "alpha"
2. b.txt: Contains "beta"
3. c.txt: Contains "gamma"
```

Eight seconds for three children, and three transcripts on disk.

### The per-parent cap now bites, and its message is now true

The same call with four tasks and a cap of two:

```sh
rho run '...four tasks...' --max-children-per-parent 2
```

```
Succeeded (2):
1. Task 1 (scout on a.txt): "The file contains the single word 'alpha'."
2. Task 2 (scout on b.txt): "b.txt contains the single word 'beta'."

Failed (2):
3. Refusal: "the per-parent child limit is 2 and this parent already runs 2. Wait for a
   child to finish, or ask the user to raise --max-children-per-parent."
```

Two things to note. The refusal names a flag that now exists. And a refused task is a
per-task result, so the two tasks that fit still returned their work.

### Defect seven: my deep-chain cancel test was weak, and the break test caught it again

A security review flagged the untested path: a wake that travels up a token chain on a
multi-thread runtime. The test written for it **passed with the ancestor wake deleted**.

The cause was the fast path. The test cancelled the root immediately after spawning the
waiter, so `cancelled()` found the flag already set and returned without ever registering a
notify. The test never reached the code it was written for.

The replacement parks the waiter first, with `yield_now` on a single-thread runtime and no
`sleep`, and asserts the token is still live before it cancels. It now fails when the
ancestor wake is removed, while the contention version still passes. Both are kept, and the
comment says which one is load-bearing.

**That is the second time in this sitting that breaking the code exposed a weak test.** Step
7 of the development flow paid for itself twice in one day.

## Round four: three more defects, from making a child addressable

The registry counted children and did not know them, and the spawn tool built a child session
as a local and dropped it. So nothing could cancel one child, watch one child, or steer one.
Three missing features had one cause.

**Defect eight: the three agent events had no sender.** `AgentSpawned`, `AgentProgressed`, and
`AgentFinished` were defined in `rho-core` and rendered by `rho-tui`, and **nothing emitted
them**. The TUI held a renderer for events that never arrived, which is
D-a-panel-nobody-can-open in a new place. `SPEC-subagents` section 9 named the missing
driver-level hook, and it stayed missing.

The fix gives `ToolContext` a typed event channel beside its string one, wired like the
existing `ToolUpdate` forwarding, with a drain after the tool returns so the finish event is
not lost.

**And a bug inside that fix.** The child's own event stream and the new outbound sender were
both named `events`, so the parameter was shadowed and `AgentFinished` was silently never
sent. The test caught it because it asserts a **pairing**: every spawn has exactly one finish.
An assertion on one expected event would have passed. That is AGENTS.md step 12, and it earned
its place.

**Defect nine: a limit nobody could set, again.** `SubagentLimits::max_tool_calls` arrived
hardcoded, with no flag, exactly like the three limits fixed in round two. It is now
`--max-agent-tool-calls`.

**Defect ten: the first fix for the CLI depth broke the feature completely.** Setting the
command-line depth to 0 passed three unit tests and then refused **every** spawn, because the
root session is itself depth 0. The honest value is 1. The replacement test drives a real
`AgentRegistry`, spawns a child, and asserts the grandchild is refused, so it fails if the
depth returns to 0. See D-cli-depth-is-zero.

## Round five: the tool-call budget

A turn cap counts provider round trips. It cannot bound a turn that asks for forty tool calls,
so a budget counts the work instead. The check runs **before** each call, so the cap is never
exceeded rather than noticed afterwards.

`the_tool_call_budget_stops_a_turn_that_asks_for_too_many_tools` scripts one turn with five
tool calls against a budget of three. A turn cap of 32 would have let all five run.

## Round six: the gate got a caller, and a lying agent was caught

The gate shipped in `rho-core` with nothing calling it, which is the same "an unreachable guard
is not shipped" trap as `RetryLedger` and `confine` before it. `spawn_agent` now takes an
`artifacts` list.

Two live runs, and this is the clearest evidence in this file.

An agent definition named `liar`, whose body tells it to always claim success and never use a
tool, was given `artifacts=["report.md"]`:

```
The tool result shows that the liar subagent failed its acceptance gate — it didn't
actually create report.md. The liar agent claimed success but didn't do the work, so
the task was rejected.
```

An agent named `writer`, given the same task, wrote the file and passed. `cat report.md`
returned `findings ok`. So the gate accepts real work and refuses a claim.

**Defect eleven, found by a new guard rather than by a run.** A spec's test list is a promise,
and no gate could check it, because a test name is not an id. `bench/check-spec-tests.py` now
checks that every test a **delivered** spec names really exists. It found fifteen phantom names
in `SPEC-agent-tasks`. Eight were the implementation choosing another name with no
reconciliation, four came from a decision that listed tests nobody wrote, and three were
promises of behaviour that was never built. Those three are now built:

- A rejected task sets `is_error`. It did not, so a model reading only the flag would have read
  a rejection as a success.
- A task with no goal is refused before anything runs. `AgentTask::validate` existed and the
  tool never reached it.
- `AgentOutcome::Rejected` round-trips through serde, because it crosses the persisted boundary.

**The gate obeys the parent's sandbox, and now that is proved.** `a_gate_command_obeys_the_parent_sandbox`
runs a confined check that writes inside the root and then tries to write outside it. The
second write fails and no file appears. The test skips when the host has no sandbox backend,
like every other sandbox test.

## The security review, and what it found

A security review of `CancelToken::child` ran after the fix. It found **no defect in the
primitive**, and it proved that with stress tests rather than by reading: sibling to sibling
is unreachable, a sibling self-cancel never wakes a parked parent, and every parked sibling
wakes on a parent cancel. It also proved the snapshot-at-creation reasoning in
`D-cancel-wake-race` holds for every ancestor in the chain, not only the nearest one.

It found two weak tests, and it was right about both.

**One of my tests was misnamed and could not see the bug it implied.**
`a_leaf_cancelling_never_cancels_an_ancestor_under_contention` awaited the cancel before
asserting, so it did not contend, and it checked flags only. A flag check cannot catch a
**wake-only upward leak**: a bug that pulses the parent's `Notify` without setting its flag.
The test is renamed to `a_leaf_cancelling_never_sets_an_ancestor_flag`, which is what it
actually proves, and the gap is now covered by
`a_sibling_self_cancel_never_wakes_a_parked_parent`.

**No test noticed that the broadcast is load-bearing.** Swapping `notify_waiters` for
`notify_one` woke only one sibling, and the whole suite stayed green. That matters now,
because `spawn_agents` parks several siblings on one parent token at once.
`a_parent_cancel_wakes_every_parked_sibling` parks six siblings, proves each one parked, then
cancels the parent. It fails on `notify_one`.

Both new tests were proved by breaking the code:

| Break | Result |
| --- | --- |
| Pulse the parent's notify on a child cancel | FAILED: "a sibling self-cancel must not wake a parked parent" |
| `notify_waiters` becomes `notify_one` | FAILED: "sibling 1 never woke, so the parent cancel did not broadcast" |

The review used a wall-clock timeout to prove a parked parent stayed parked. The versions in
the repository use `yield_now` and `is_finished` instead, so they are deterministic and use no
`sleep`. Each one also cancels the parent at the end, to prove the parked assertion is not
passing for the wrong reason.

**Cost.** `is_cancelled` now walks the parent chain. The chain is bounded by
`SubagentLimits.max_depth`, and `rho-cli` pins that to 1, so the walk is at most two links on
the hot path.

## Run it yourself: `bench/demo-subagents.sh`

Do not trust this page. The script asserts every claim on it, and it exits non-zero when any
check fails.

```sh
./bench/demo-subagents.sh
```

It builds the binary, makes a temporary session root, writes four agent definitions, and runs
thirty checks against the live provider. It removes the directory when it finishes. Thirty
passed on three consecutive runs.

**One lesson from writing it: never assert on how the model words an answer.** The first
version grepped for phrases the model happened to use, and three checks failed while the
product was correct. `timed out` does not contain the substring `timeout`. A paraphrase is not
a defect. So the script now asks for a verbatim quote when it needs rho's own text, and every
pattern accepts each reasonable wording.

**One check had to change because a fix made it unreachable.** The bad-agent-name check used to
drive the model into calling `spawn_agent` with a name that does not exist. The schema `enum`
now stops the model inventing one, so it declines and lists the real agents instead. That is
defect one's fix working. The tool-level path is still covered, by
`an_unknown_agent_name_is_a_result_not_a_fault` in `rho-tools`.

## Steering, and the mode it cannot reach

Steering a running child works, and it is proved by a runnable example rather than by a
claim:

```sh
unset AWS_PROFILE
cargo run --release -p rho-cli --example steer_subagent
```

Real output, identical on three consecutive runs:

```
1. the child is registered and addressable
   live: id 1 agent scout depth 1
2. waiting for the child to start a turn, then steering it
   the child is on turn 1
   steered at queue position 1
3. the child read the steering message
   delivered 1 message(s) at turn 1
4. the child changed course
   final answer: "I'll start by reading a.txt.STEERED"
   it ran 2 turn(s), so it stopped early rather than reading all three files
5. the handle leaves the live list when the child finishes
   live children now: 0
```

The child was told to read three files, one per turn. It read the first, received the
steering message at the turn boundary, and stopped. So the message reached the model and
changed its course.

### `steer_agent` cannot reach a live child from `rho run`, and that is a mode limit

This is the honest part. `AgentLoop::dispatch` runs tool calls one at a time, and the spawn
tool blocks until the child finishes. So inside a single `rho run` turn a child never
outlives the parent's tool call. By the time the model could call `steer_agent`, there is no
live child.

A live probe shows exactly that:

```sh
rho run 'Use steer_agent with id=42 and message="hello". Quote the tool result verbatim.'
```

```
no subagent with id 42 is running, so it cannot be steered. It may have finished already.
No subagent is running now.
```

The refusal is correct and it teaches. But nobody should read the registered tool as proof
that a model can steer a sibling today.

**The real caller is a host.** A host owns the registry, so it can watch a child and redirect
it while it works. That is what a TUI or an ACP frontend does, and it is what the example
above demonstrates. The model-facing tool becomes useful when a child can outlive a turn,
which needs background children, and that is a separate change.

## Round seven: two harsh reviews, and eleven more findings

Two reviewers read the finished feature. Both were told to assume another defect of each
family was present, and both were told that a review which finds nothing is a failed review.
Between them they found eleven things, and every one was verified before it was fixed.

**The worst was a contract that a wiring accident was holding up.** `AgentRegistry::cancel`
took a bare id and resolved it against the whole process, and the registry is process-wide on
purpose. The reviewer built two trees in one registry and had the second cancel and steer the
first tree's child. The shipped command line was safe only because each run builds its own
registry and no child holds a control tool. The registry now offers `live_under`,
`descendant`, and `cancel_descendant`, and every tool uses them. See
D-a-caller-addresses-only-its-own.

**`MessageQueued` was defined, rendered, and never emitted.** The same family as the three
agent events, in the same feature, found again. The announcement now lives in `MessageQueue`
itself rather than in one caller, so every pusher announces, including a subagent steered
through `LiveAgent::steer`. Putting it in `Session::steer` would have left the subagent path
silent, which is how the first version was wrong.

**`SessionConfig::with_queue` was a silent-drop trap.** It existed with no caller, and
`Session::with_config` built a fresh queue and ignored it. A caller who set the queue there
had every steering message dropped. `with_config` now honours the config.

**Three tests were weak, and mutation proved it.** Each mutation below passed the whole suite
before the fix:

| Mutation | What it showed |
| --- | --- |
| The tool-call budget check `>=` becomes `>` | The test asserted only the final outcome, so a budget of three could run four calls. It now counts the calls. |
| Delete the post-tool drain in `dispatch_one` | No test drove a subagent through a real parent `Session`, so the forwarding of `AgentFinished` was uncovered. One does now. |
| `narrow_sandbox` `>=` becomes `>` | No test asked for the parent's exact mode, so "a child may keep the same mode" was unproven. |

**Two reservations were not atomic, and one comment said it was.** The per-parent slot did a
load, then a check, then a later add. Its comment claimed all three happened under one atomic.
Both reservations now use a compare-and-swap loop, and two racing tests hold them.

A note on writing those tests: a `std::sync::Barrier` was not enough, because it wakes threads
through the operating system and a staggered start never loses the race. A spin gate on an
`AtomicBool` loses it reliably, and the non-atomic version then grants two children against a
cap of one.

**Two smaller findings.** The credential denylist missed a connection string, so
`DATABASE_URL`, `REDIS_URL`, and `MYSQL_PWD` all survived the scrub while every provider key
was caught. And `RetryLedger` kept an unbounded map keyed by the whole prompt, which a model
writes. Both are fixed and both have tests.

**What the reviews found clean, having tried to break it.** The policy composition, the
tool-set intersection under case and whitespace and duplication and an empty list, the sandbox
narrowing in every ordering, `confine` against a symlink and an absolute path and a `..`
segment and a NUL, the model's inability to reach a command check through any field it
controls, and the definition loader's inability to raise any limit. The cancel token was
called genuinely well tested.

## An operator error worth recording

The first credential probe used double quotes, so the **outer shell** expanded
`$AWS_SECRET_ACCESS_KEY` and put the real secret into the prompt. The child's environment
was clean, and `env | grep -c` proved it, but the value still reached the provider through
the prompt text. The probe was redone with single quotes.

The lesson is for the operator, not for rho: quote a credential probe so the local shell
cannot expand it. The key used during this sweep must be rotated.
