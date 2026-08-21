# Verification — the slot queue, driven for real on Bedrock

Date: 2026-08-20. This is AGENTS.md step 11. Every command and every output below is real. The
provider is AWS Bedrock and the model is the latest haiku.

## What the change must do

A fan-out wider than the per-parent cap used to lose the extra tasks. Each one came back as a
refusal, and the model had to notice and retry by hand. rho now queues them. The model gets an id
at once, and the child starts when a slot frees.

## 1. Setup

```sh
cargo build --release -p rho-cli
unset AWS_PROFILE
export HOME=/tmp/rho-queue-e2e/home      # a fake home, so the real ~/.rho is never read
MODEL=us.anthropic.claude-haiku-4-5-20251001-v1:0
cd /tmp/rho-queue-e2e/root               # four files: a.txt, b.txt, c.txt, d.txt
```

One definition in `$HOME/.rho/agents/scout.md`:

```markdown
---
name: scout
description: Reads one named file and reports one sentence about it.
tools: read, list
max_turns: 3
---
You read exactly the one file the prompt names. Answer in one sentence.
```

## 2. A fan-out of four under a cap of one runs every task

```sh
rho run "Use spawn_agents once, with four tasks, to send the scout agent at a.txt, b.txt, \
c.txt and d.txt, one file per task. Then say in one line what each child reported." \
  --provider bedrock --model $MODEL --max-children-per-parent 1
```

```text
rho: 1 agent definition(s) available to spawn_agent: scout.
a.txt contains "alpha file"; b.txt contains "beta file"; c.txt contains "gamma file";
d.txt contains "delta file".
```

Four answers under a cap of one, in 20.7 seconds. Three of the four waited. Before this change,
three of the four returned "the per-parent child limit is 1".

## 3. The same command a second time

The "twice" rule has caught two defects in this project, so every path runs twice.

```text
rho: 1 agent definition(s) available to spawn_agent: scout.
a.txt contains "alpha file", b.txt contains "beta file", c.txt contains "gamma file",
and d.txt contains "delta file".
```

## 4. The process-wide cap still refuses, and it still teaches

```sh
rho run "Use spawn_agents once with three tasks: scout on a.txt, scout on b.txt, scout on \
c.txt. Report exactly what came back for each, including any refusal text." \
  --provider bedrock --model $MODEL --max-live-agents 1
```

```text
**Scout on a.txt:** "I read the file a.txt as requested and found it contains the text
'alpha file'."

**Scout on b.txt:** "the process-wide agent limit is 1 and 1 agents are live. Wait for an
agent to finish, or ask the user to raise --max-live-agents."

**Scout on c.txt:** "the process-wide agent limit is 1 and 1 agents are live. Wait for an
agent to finish, or ask the user to raise --max-live-agents."
```

One cap queues and one refuses, exactly as section 2.7 of the spec states. The refusal names the
flag, and the task that fitted still reported its work.

## 5. A queued child is pollable by the id the model was given

```sh
rho run "Do exactly this: call spawn_agent twice with background true, first for scout on \
a.txt, then for scout on b.txt. Straight after the second call, call agent_status on the id \
the second call gave you. Then quote the agent_status text verbatim." \
  --provider bedrock --model $MODEL --max-children-per-parent 1
```

The model quoted rho's own answer:

```text
scout (id 2) is queued, at place 1 in its parent's line. It has not started, and it will start
when a sibling finishes. Cancel it with cancel_agent, or steer it now and it reads the message
on its first turn.
```

The id the model holds is the id that waits, and the place is computed on read.

## 6. The defect this step found

The same run, with a cancel between the spawn and the poll:

```sh
rho run "Do exactly this: call spawn_agent with background true for scout on a.txt. Then call \
spawn_agent with background true for scout on b.txt. Then call cancel_agent on the second id, \
then call agent_status on that same second id. Quote both results verbatim." \
  --provider bedrock --model $MODEL --max-children-per-parent 1
```

Before the fix, rho answered:

```text
subagent 2 was asked to stop. Its siblings keep running.
scout (id 2) is queued, at place 1 in its parent's line. It has not started, and it will start
when a sibling finishes. Cancel it with cancel_agent, or steer it now and it reads the message
on its first turn.
```

Three false statements to a model that had just cancelled the child: it holds a place, it will
start, and a steer will reach it. A cancel wakes the waiting task, and the entry leaves the map
when that task drops it, so there is a window where the entry is present and the token is already
cancelled. No test covered the window. See decision D-a-cancelled-waiter-says-so.

After the fix, the same run:

```text
**Result from cancel_agent:**
> subagent 2 was asked to stop. Its siblings keep running.

**Result from agent_status:**
> scout (id 2) finished: cancelled. 0 turn(s), 0 token(s).
```

Here the report landed before the poll. The window itself is now covered by two tests, because a
live run cannot be made to land inside it on purpose:
`status_says_a_cancelled_queued_child_will_not_start` and
`agent_status_says_a_cancelled_queued_child_will_not_start`.

## 7. The end-to-end suite, and what it proves

The checks above are now assertions in `bench/demo-subagents.sh`, so nobody has to trust this
page. The script drives the release binary against Bedrock and exits non-zero on any failure.

```sh
env -u AWS_PROFILE ./bench/demo-subagents.sh
```

```text
7. A fan-out runs children together, and the caps bite
  PASS fan-out child 1 answered
  PASS fan-out child 2 answered
  PASS fan-out child 3 answered
  PASS the per-parent cap queues a task instead of refusing it
  PASS the task that had a slot ran
  PASS the first task that waited still ran
  PASS the second task that waited still ran
  PASS the process-wide cap refuses, and it names the flag that raises it
  PASS a refused task does not lose the work of the task that fitted

9. A queued child holds an id, and the model can act on it
  PASS a spawn over the cap is admitted, not refused
  PASS and the model is told where it sits in the line
  PASS no refusal names the per-parent cap any more
  PASS a queued child is cancellable by the id the model holds
  PASS a cancelled child is never promised a start
  PASS and the answer says it was cancelled
  PASS a steer is accepted for a child that has not started
  PASS and the receipt names the agent, which holds no live handle yet
  PASS a steer for a queued child is never a lost message

Result
  passed: 48
  failed: 0
```

**Three consecutive runs passed, and the suite exits 0.** A live suite that passes once proves
less, because a model paraphrase changes between runs.

**The new checks were proved against the old behaviour.** With `run_one_child` and
`start_background_child` put back to `spawn_child`, the same suite reported `passed: 42,
failed: 6`, and the six were exactly the queue checks. So these checks can fail.

**Two checks failed first, and rho was right both times.** They asked the model to reproduce a
whole tool result verbatim, and the model answered "That's the verbatim result" and then
summarised it. The flag name and the child's answer were lost in the paraphrase. The prompt now
asks two narrow questions, which a model quotes rather than rewrites. A model paraphrase is not a
defect, and a check that cannot tell the difference is a bad check.

## 8. Named handles, driven for real

The last slice of the spec. A model addresses a child by a name instead of a number, and it may
set a name of its own. `bench/demo-subagents.sh` section 10 asserts it against Bedrock.

```text
10. A model can address a child by name
  PASS a background spawn reports a name the model may use
  PASS agent_status with no id lists the running children
  PASS listing with no id is not a refusal
  PASS a steer reaches a child by its caller-set name
  PASS a name is not refused as a malformed id
  PASS a derived handle stops a child
  PASS a derived handle is not refused
  PASS an unknown name is an ordinary miss that teaches

Result
  passed: 55
  failed: 0
```

**Proved against the old behaviour too.** With `resolve` made to refuse every name, the same
suite reported `passed: 53, failed: 2`, and the two were the steer-by-alias and the
cancel-by-handle checks. So these checks can fail.

**What the mutations found that reading did not.** Eight deliberate breaks went into the handle
code. Six were caught at once. Two were not, and both were real:

| Mutation | Why nothing caught it | What it means |
| --- | --- | --- |
| the handout re-derives a name | the handout never called the binder, so the guard was unreachable | the promise "a waiter keeps its name" rested on an accident |
| `resolve` skips the ownership re-check | every handle test used a root caller | a mid-tree caller could reach a cousin's child by name |

Both are fixed, and both now have tests. See decision D-one-place-binds-a-name.

**One guard was fail-open, and it hid this spec for a whole run.** `bench/check-spec-tests.py`
read the entire `Status:` line and exempted any spec that mentioned "draft" or "planned". This
spec says `delivered` and then explains that one line is marked planned, so the guard exempted
it and reported green while checking nothing. It now reads only the first word of the status.
The name count it checks rose from 210 to 318 the moment that was fixed.

## 9. One more defect, outside this change, not fixed here

The first definition file in this session wrote its tool list as a YAML sequence:

```markdown
tools: [read, list]
```

rho reads `tools` as a string, so `serde_yaml` failed, `load_definition` returned `None`, and the
whole definition disappeared. No notice, no warning, and `spawn_agent` was never registered. The
model then answered that it had no such tool, and two runs were wasted before the cause was
found.

The documented spelling is `tools: read, list`, so this is not a broken contract. It is a
failure path that teaches nothing, which AGENTS.md forbids. It belongs to `rho-skills` and it
needs its own contract decision, because `AgentSet` today carries only `loaded` and `withheld`
and a rejected file has nowhere to go. It is recorded in `.rho-work/progress.md` and it is not
fixed in this change.
