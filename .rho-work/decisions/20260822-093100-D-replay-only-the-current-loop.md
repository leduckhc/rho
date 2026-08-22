# Only the current tool loop replays its reasoning

Date: 20260822. Reference: `D-replay-only-the-current-loop`.
Found by a performance review of the reasoning work.

## The question

The prompt is append-only, so every message goes out again on every later turn. The first
version of the replay sent every stored reasoning block, with its full text and signature, on
every request for the rest of the session.

A review worked the cost out: turn one's trace is re-sent on turns two to twenty, so the extra
bytes are B multiplied by the sum of one to nineteen, which is 190 times B. With a large
`xhigh` trace that is tens of megabytes over a session, and the growth is O(turns squared).

## The decision

A reasoning block replays only when it sits at or after the last user message. Everything
older is history: the transcript keeps it, the reader can see it, and the wire never carries
it again.

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
- `a_whole_tool_loop_keeps_its_reasoning`: a tool result does not end a loop, so both assistant
  turns inside it still replay.
- `an_out_of_scope_block_leaves_no_text_behind`: the dropped text never appears as prose.
- Live: the two-call tool loop still passes on Bedrock after the change. See
  `docs/verification/reasoning-replay.md`.

## The limit

The rule reads roles, so it assumes a user message opens a loop and a tool result does not. If
a future frontend injects a synthetic user message mid-loop, the scope would cut early and a
turn would lose its chain. A test would catch it, because the loop would fail with a 400.
