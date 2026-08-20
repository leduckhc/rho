# Verification — the slot queue, driven for real on Bedrock

Date: 2026-08-20. This is AGENTS.md step 11. Every command and every output below is real. The
provider is AWS Bedrock and the model is the latest haiku.

## What the change must do

A fan-out wider than the per-parent cap used to lose the extra tasks. Each one came back as a
refusal, and the model had to notice and retry by hand. rho now queues them. The model gets an id
at once, and the child starts when a slot frees.

## Setup

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

## 1. A fan-out of four under a cap of one runs every task

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

## 2. The same command a second time

The "twice" rule has caught two defects in this project, so every path runs twice.

```text
rho: 1 agent definition(s) available to spawn_agent: scout.
a.txt contains "alpha file", b.txt contains "beta file", c.txt contains "gamma file",
and d.txt contains "delta file".
```

## 3. The process-wide cap still refuses, and it still teaches

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

## 4. A queued child is pollable by the id the model was given

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

## 5. The defect this step found

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

## 6. One more defect, outside this change, not fixed here

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
