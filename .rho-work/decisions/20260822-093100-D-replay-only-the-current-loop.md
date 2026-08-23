# Only the pending run of assistant turns replays its reasoning

Date: 20260822. Reference: `D-replay-only-the-current-loop`.
Found by a performance review of the reasoning work.

## The question

The prompt is append-only, so every message goes out again on every later turn. The first
version of the replay sent every stored reasoning block, with its full text and signature, on
every request for the rest of the session.

A review worked the cost out **from the code path, not from a measurement**: turn one's trace
is re-sent on turns two to twenty, so the extra bytes are B multiplied by the sum of one to
nineteen, which is 190 times B. With a large `xhigh` trace that is tens of megabytes over a
session, and the growth is O(turns squared).

No bench builds a twenty-turn request, so the number above is arithmetic over the append-only
rule. The one measured number in this work is the TUI frame cost in `docs/benchmarks.md`.

## The decision

A reasoning block replays only when it belongs to **the one assistant turn that carries the
pending tool call**. Everything else is history: the transcript keeps it, the reader can see
it, and the wire never carries it again.

The rule took three attempts, and each one was corrected by evidence rather than by opinion.

1. **From the last user message onward.** A performance review found that an autonomous tool
   loop holds no user message at all, so a hundred-iteration loop still re-sent iteration
   one's trace ninety-nine times. The growth returned one level down.
2. **The last assistant turn.** A test then failed: `a_prompt_with_no_answer_replays_nothing`.
   When a user prompt follows that turn, its chain is already closed, and its thinking must not
   travel again.
3. **The last assistant turn, and only when it comes after the last prompt.** A review then
   found the shape that breaks: Bedrock wants alternating roles, so consecutive assistant turns
   merge into one wire message. Keeping only the last turn's thinking left the earlier turn's
   tool call with no thinking in front of it, which Anthropic refuses.
4. **The trailing run of assistant turns.** This is the rule in the code. The run starts after
   the last message that is not from the assistant, so a tool result ends it and a long loop
   still sends one trace. A merged pair keeps both.

The title of this file said "one assistant turn" for two of those attempts, while the code
comment said "turns". A review named the contradiction, and the title now matches the rule.

## Why

Anthropic needs the thinking of the assistant turns that carry the **pending** tool call, so
that the signature chain of the current loop stays whole. A block from before the last prompt
is not part of that chain. Sending it buys nothing and costs its bytes every turn.

## What it rules out

- No unbounded prompt growth from reasoning.
- No conversion of an out-of-scope block into prose. It is dropped whole, because prose would
  read as an answer.
- No per-turn cap or truncation of a replayed payload. A truncated signature is worse than an
  absent one, so the scope shrinks instead of the payload.

## The evidence

- `only_the_current_loop_replays_its_reasoning`: two prompts, and only the second turn's
  reasoning reaches the wire.
- `a_long_tool_loop_sends_one_trace`: twenty iterations send one trace, not twenty.
- `the_replayed_count_does_not_grow_with_the_loop`: the count is flat at one for loops of 1, 5,
  20, and 100 iterations. That pins the invariant rather than an example.
- `a_prompt_with_no_answer_replays_nothing`: a closed chain never travels again. This is the
  test that found attempt 2 wrong.
- `an_out_of_scope_block_leaves_no_text_behind`: the dropped text never appears as prose.
- Live: the two-call tool loop still passes on Bedrock after the change. See
  `docs/verification/reasoning-replay.md`.

## The limit

The rule reads roles, so it assumes a user message opens a loop and a tool result does not. Two
shapes sit at its edges, and both are now pinned by a test.

**A transcript with no user message still replays exactly one turn.** The scope needs no prompt
to anchor it, so the unbounded fallback of the first attempt is gone. See
`a_transcript_with_no_prompt_replays_everything`.

**A synthetic user message mid-loop would cut the scope early**, and the turn would lose its
chain. Nothing injects one today. A live run would fail with a 400 rather than fail quietly,
which is the failure mode to want.
