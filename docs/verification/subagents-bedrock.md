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

## An operator error worth recording

The first credential probe used double quotes, so the **outer shell** expanded
`$AWS_SECRET_ACCESS_KEY` and put the real secret into the prompt. The child's environment
was clean, and `env | grep -c` proved it, but the value still reached the provider through
the prompt text. The probe was redone with single quotes.

The lesson is for the operator, not for rho: quote a credential probe so the local shell
cannot expand it. The key used during this sweep must be rotated.
