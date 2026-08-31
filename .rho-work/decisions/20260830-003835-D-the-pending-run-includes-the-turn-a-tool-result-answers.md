# D-the-pending-run-includes-the-turn-a-tool-result-answers — the replay scope excluded every real request

**Question (controller, live drive):** a deliberately corrupted signature used to make Bedrock
answer 400. On 20260829 the same break was **accepted**, at exit 0. Does any reasoning block
still reach the wire?

## The defect

No. Zero reasoning blocks travel in the shape rho actually sends.

`D-replay-only-the-current-loop` set the scope to "the trailing run of assistant turns". Its
own words name the hole:

> The run starts after the last message that is not from the assistant, **so a tool result ends
> it** and a long loop still sends one trace.

Ending the run at a tool result was meant to **bound** it. It also **empties** it. rho builds a
request right after it appends the tool results, so the last message is always a tool result,
and the trailing run of assistant turns is always empty.

So the turn that carries the pending tool call is treated as history, and its thinking is
dropped. That is the one turn Anthropic requires.

## Why no test saw it

**Not one scope test ends its message list with a tool result.** All nine end with an assistant
turn, which is the transcript *after* a model answers, and never the request rho *builds*.

A probe on the real shape counts the blocks:

```text
messages: [user, assistant(reasoning + tool_use), tool(result)]
reasoning blocks in the request = 0
```

## Why no live run saw it

`D-replay-only-the-current-loop` recorded this as its live evidence:

> Live: the two-call tool loop still passes on Bedrock after the change.

**A passing loop proves nothing here.** The same command passed before the replay existed, when
rho sent no reasoning at all. `docs/verification/reasoning-replay.md` section 1 states that in
so many words, and the decision then leaned on exactly the run that section warns about.

The old decision also predicted the failure mode wrongly:

> A live run would fail with a 400 rather than fail quietly.

It fails quietly. Bedrock accepts a request that omits the thinking, so nothing complained for
seven days.

## The decision

**The pending run is the trailing run of assistant turns. When the messages end with a run of
tool results, the pending run is the assistant run immediately before that tool run.**

Stated as an algorithm:

1. Take the trailing run of `Role::Tool` messages. It may be empty.
2. The pending run ends where that tool run starts.
3. The pending run is the maximal run of `Role::Assistant` messages ending there.

A tool result no longer **ends** the scope. It **anchors** it: the assistant turns it answers
are exactly the turns whose signature chain must stay whole.

## Rules that hold

**The growth stays flat.** The scope is one contiguous assistant run, adjacent to the trailing
tool run. It never reaches back over an earlier tool result, so a loop of any length still
sends one trace. `the_replayed_count_does_not_grow_with_the_loop` keeps pinning that, and it now
runs on both shapes.

**A closed chain still travels nowhere.** A trailing **user** message is not a tool result, so
the tool run is empty and the assistant run before it does not qualify.
`a_prompt_with_no_answer_replays_nothing` is unchanged.

**A merged pair still keeps both traces.** Step 3 takes the maximal run, so two consecutive
assistant turns both replay, whether a tool result follows them or not. Anthropic refuses a
`tool_use` with no thinking in front of it.

**Parallel calls work.** Step 1 takes a run of tool results, not one, so two results for one
turn still anchor that turn.

## The guard for the class, not the case

A review made the plain point: eight of the nine scope tests model no request rho can build, and
that is how the defect hid. Adding a tenth hand-written shape would repeat the mistake, because
the author of a hand-written shape is the same person who misread the caller.

So the guard **asks the caller for the shape.**
`the_request_the_agent_loop_builds_carries_its_reasoning` runs a real `Session` with a fake
provider. Turn one mints a reasoning payload and a tool call. Turn two records the request it is
handed, which is the message list `rho-core` assembled after it ran the tool. That list, not a
literal, goes into `build_messages_for_model`.

Two properties make it a class guard:

- It asserts its own harness first. The recorded list must end with a tool result, or the test
  says so and fails, rather than proving nothing quietly.
- A later change to how the loop orders or merges the messages of a loop arrives in this test
  with no edit.

It fails against the rule that shipped broken, with `left: []`.

## The limit, stated rather than left implicit

**A user message between an assistant turn and its tool result would break this rule.** The
trailing tool run would start at that user message, the pending run would be empty, and rho
would ship a tool result with no thinking in front of it. Bedrock answers 400.

That shape is **not reachable today.** A review checked the steering path: a steer is delivered
at a turn boundary, after the tool results, so it produces `[..., tool, user]` and never
`[..., assistant, user, tool]`. The reachable steer shape resolves correctly, because the
trailing tool run still touches the assistant turn that made the pending call.

No branch is added for the unreachable shape. A branch no test reaches is surface this project
deletes. If steering ever injects a message before the tool results, this rule needs a test
first, and the failure mode is a loud 400 rather than a quiet drop.

## Rules out

**Replaying every assistant turn.** That is the O(turns squared) growth
`D-replay-only-the-current-loop` removed, and it stays removed.

**Reaching back past an earlier tool result.** Those calls are already answered, so their chains
are closed.

**Trusting a green tool loop as evidence again.** The proof that a block travels is a corrupted
signature and a 400, and nothing else. See `docs/verification/reasoning-replay.md` section 9.

## Supersedes, in part

`D-replay-only-the-current-loop` keeps its cost analysis, its three corrected attempts, and
every rule it ruled out. Only its step 4 changes: a tool result anchors the run instead of
ending it.
