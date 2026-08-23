# D-fan-out-is-one-tool-call — A fan-out is one tool call, and general dispatch stays sequential


A live sweep showed that three of the four subagent limits cannot trigger. See
`docs/verification/subagents-bedrock.md`. `AgentLoop::dispatch` runs tool calls one at a
time, so one session never holds two live children, and `ChildSlot` frees between two
sequential spawns. rho advertised a fan-out and could not fan out.

**The question.** How does rho run several children at once?

## The option we rejected

**Make `AgentLoop::dispatch` concurrent.** Every harness that fans out this way relies on the
model emitting several tool calls in one assistant message, and the loop running them
together. It is the smallest diff, and it is wrong here.

Concurrency in `dispatch` applies to **every** tool, not to subagents. The built-in set holds
`edit` and `write` at `ToolKind::Edit`, and `bash` and `task` at `ToolKind::Execute`. Two
concurrent `edit` calls on one file race. Two concurrent `bash` calls interleave their output.
Worse, the approval policy asks a human, and `D-approval-default-ask` makes that the default,
so concurrent dispatch would race two prompts onto one terminal. The session log is
append-only by `D-append-only-jsonl`, and a concurrent dispatch makes its order
non-deterministic.

So the change would buy a fan-out and pay with a data race, an unreadable transcript, and a
broken approval prompt. That trade is not worth making.

## The decision

**A fan-out is one tool call.** `rho-tools` gains a second tool, `spawn_agents`, that takes a
list of tasks and runs them together inside its own call. `AgentLoop::dispatch` stays
sequential and unchanged.

Four rules come with it.

1. **Concurrency is confined to subagents.** A child is already isolated: a fresh
   conversation, its own `Session`, and its own derived `CancelToken`. Nothing else in the
   tool set gains concurrency.
2. **A per-task refusal is a per-task result.** When the fifth child exceeds
   `max_children_per_parent`, that task reports the refusal and the others still run. A limit
   must not fail the whole call.
3. **Results are reported in request order.** Execution is concurrent and the report is
   deterministic. A stable order keeps the provider cache warm, which is the rule in
   `D-measured-cost-and-cache`, and it makes a test possible.
4. **Two tools, not one widened tool.** `spawn_agent` keeps its two-field schema for the
   common case of one child. `SPEC-background-tasks` set this precedent when it split `task`
   from `task_cancel`. A small schema per tool costs less context than one schema with a mode
   flag, and the model chooses correctly more often.

## What this rules out

It rules out concurrent general tool dispatch, until somebody writes a separate spec that
answers the file race, the approval race, and the log order. It rules out a fan-out that
fails whole when one child is refused. It rules out a non-deterministic result order.

## What it makes true

`max_children_per_parent` and `max_live_total` now bite, and a test can reach them. The
limits stop being decoration. That was the point.
