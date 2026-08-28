# D-a-recorder-writes-the-assistant-turn — the recorder must fold the stream, or a resume is invalid

**Question:** `SessionRecorder` is the only thing that turns a live run into records. What
does it write for an assistant turn?

## The defect

Nothing. `SessionRecorder::observe` writes a tool result, a usage record, and a stop
record. Its own doc comment says it writes "an assistant message at a turn end", and its
match has no `TurnEnd` arm. It also never reads `StreamEvent::TextDelta`,
`StreamEvent::ToolCallStart`, or `StreamEvent::ToolCallEnd`.

So a recorded session holds the user prompts and the tool results, and nothing else. A
resume then rebuilds a message list with a `ToolResult` that matches no `ToolCall`. Every
provider refuses that request. The answer text is lost as well.

No test caught it, because every test that writes a `Message` record builds the record by
hand and appends it through the writer. `D-cancel-keeps-the-session-open` states that a
cancel writes "the assistant message so far", and the cancel path invents empty arguments
for an open call because it never held the real ones.

`SPEC-session-store-wiring` did not name this. So it is an amendment, recorded here.

## The decision

The recorder folds the stream into one assistant message per turn.

- `MessageStart` and `TurnStart` open a turn and clear the parts.
- `TextDelta` appends to the current text block. `TextEnd` closes it.
- `ThinkingDelta` and `ThinkingEnd` build a `ReasoningReplay` block when the provider sent
  a replay payload, and a `ReasoningTrace` block when it sent none.
- `ToolCallStart` opens a call. `ToolCallEnd` completes it with the parsed arguments and
  the provider payload.
- `TurnEnd` writes one `Message` record with role `Assistant`, holding those blocks in
  block-index order. An empty turn writes nothing.
- A `ToolEnd` result record is written after the assistant message that names the call, so
  the file order is always call, then result.
- A cancel writes the partial assistant message, with the real arguments it already holds.

## The second half of the same defect

`ToolEnd` wrote the raw output blocks as the content of the tool message. So the record
carried no `tool_call_id`, while `Agent::finish_tool` wraps the same output in a
`ContentBlock::ToolResult`. The recorded conversation therefore had a different shape from the
one the model saw.

A resume then sent a tool message no provider can match to a call. Worse,
`branch_messages` looks for a `ToolResult` block to pair a call, so it found none and invented
a synthetic error result **beside** the real result. A model would have read "the tool call did
not finish" next to the output it produced.

So `ToolEnd` writes a `ToolResult` block that names its call. The shape on disk is now the
shape in the context, and `every_tool_call_on_disk_has_a_result_on_disk` pins it.

## Rules that hold

- The pairing invariant is now provable on the run path. For any run, every `ToolCall` on
  disk has a `ToolResult` on disk, or a synthetic error result.
- The block order follows the provider `index`, so a replay sends the blocks back in the
  order the provider produced them.
- A reasoning payload is kept verbatim, because a rewritten payload cannot replay. That is
  rule 9 of `SPEC-reasoning-across-providers`, and it already governs the cap path.
- Redaction still runs, through `redact_block`, before anything reaches the file.

## Rules out

**Rebuilding the assistant message inside the agent loop and putting it on `TurnEnd`.**
That widens a core event that four frontends already match on, for one consumer.

**Recording the raw stream events.** The file would hold thousands of delta lines per
turn, and a resume would have to fold them all again. `D-recorder-consumes-events` already
settled that the recorder folds, and the file holds messages.

**Leaving the tool call out and repairing on read.** `branch_messages` can invent a missing
result. It cannot invent a missing call, because the arguments are gone.

## Cost

One accumulator in the recorder, five more match arms, and six tests.
