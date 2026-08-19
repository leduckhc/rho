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

## An operator error worth recording

The first credential probe used double quotes, so the **outer shell** expanded
`$AWS_SECRET_ACCESS_KEY` and put the real secret into the prompt. The child's environment
was clean, and `env | grep -c` proved it, but the value still reached the provider through
the prompt text. The probe was redone with single quotes.

The lesson is for the operator, not for rho: quote a credential probe so the local shell
cannot expand it. The key used during this sweep must be rotated.
