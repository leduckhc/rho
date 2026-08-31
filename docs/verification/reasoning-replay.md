# Verification — the reasoning replay

Date: 20260821. Spec: `docs/specs/20260819-134615-SPEC-reasoning-across-providers.md`,
section 4 and rules 8 to 11. This is `AGENTS.md` step 11.

Build:

```sh
cargo build --release -p rho-cli
```

Account 980637428984, region `us-east-1`, model
`us.anthropic.claude-haiku-4-5-20251001-v1:0`. The files `a.txt` and `b.txt` hold
`zx9-quibble` and `kt4-marlow`, so the model cannot guess their contents.

## 1. A two-call tool loop with thinking on

```sh
rho run "Use your tools. Read a.txt, then read b.txt, then print the two exact strings you \
found, separated by a comma." --provider bedrock --model "$M" \
  --reasoning-effort high --reasoning full
```

```text
zx9-quibble, kt4-marlow          exit 0
```

stderr carried the reasoning from inside the tool-calling turn.

**This run alone proves nothing about the replay.** The same command passed before the replay
existed, when rho sent no reasoning at all. A green result is not evidence that a field
travelled.

## 2. The proof: break the signature and watch Bedrock reject it

The signature was replaced with a literal in `replay_block`, the release binary was rebuilt,
and the same command was run again.

```rust
// deliberate break, reverted after the run
ReasoningTextBlock::builder()
    .text(text)
    .signature("deliberately-wrong-signature")
```

```text
rho: client error: status 400: Bedrock rejected the request as invalid. Check the model id
     and the request shape.                                               exit 1
```

A wrong signature can only break a request that carries it. So the signed reasoning block is
really in the request, and the real signature is what Bedrock accepts.

The file was restored from a copy in `/tmp`, never with `git checkout`, per
`D-jcode-bash-lessons`. The restore was checked:

```sh
grep -c "deliberately-wrong" crates/rho-provider-bedrock/src/lib.rs   # 0
```

## 3. The good path again, twice

"Twice" has caught two defects on this branch.

```text
run 1 exit=0 -> zx9-quibble, kt4-marlow
run 2 exit=0 -> zx9-quibble, kt4-marlow
```

## 4. The old path, with no thinking

```sh
rho run "Use your tools. Read a.txt then print what you found." --provider bedrock --model "$M"
```

```text
exit 0 -> I'll read a.txt for you.
```

A turn with no thinking sends no reasoning block, and the tool loop is unchanged.

## 5. After the second review

A Codex review found six things, and four of them changed the code. The live runs were then
repeated, because the request path had moved.

```text
run 1 exit=0 -> zx9-quibble, kt4-marlow
run 2 exit=0 -> zx9-quibble, kt4-marlow
```

The new report for an unreadable model id was driven too, with an application inference
profile ARN, which hides the model behind an opaque id:

```sh
rho run "Say ok." --provider bedrock \
  --model "arn:aws:bedrock:us-east-1:...:application-inference-profile/none" \
  --reasoning-effort high --log info
```

```text
WARN rho_provider_bedrock: this model is not known to support extended thinking, so rho asked
     for none model=arn:aws:bedrock:us-east-1:...:application-inference-profile/none
rho: client error: status 400 ...
```

The 400 is the profile itself, which does not exist in this account. The warning is the point:
rho still fails closed, and it now says why rather than dropping the request in silence.

## 6. After the reviewer team

Six reviewers read the change, each with one aim. Four findings changed the request path, so
the live runs were repeated.

The replay now covers the current tool loop only, per `D-replay-only-the-current-loop`. A
three-call loop and a two-call loop both still finish:

```text
run 1 exit=0 -> zx9-quibble, kt4-marlow
run 2 exit=0 -> zx9-quibble, kt4-marlow
three tool calls, effort medium: exit 0
```

The effort level now reaches OpenRouter as well, and that was driven with a real key:

```sh
rho run "Which is larger, 9.11 or 9.9? One sentence." --provider openrouter \
  --model anthropic/claude-haiku-4.5 --reasoning-effort high --reasoning full
```

| run | reasoning on stderr |
| --- | --- |
| `--reasoning-effort high` | 398 bytes |
| no effort flag | 0 bytes |
| `--reasoning-effort off` | 0 bytes |
| `--reasoning-effort xhigh` | accepted, exit 0 |

The middle two rows are the control. Without them the first row proves only that the model
sometimes thinks, and rho's field could have been ignored.

## 7. The question a review could not answer, answered live

A performance review found that the first replay scope still grew inside one tool loop, and
said its fix needed live Bedrock to confirm: does Anthropic accept a request that omits the
thinking of the **earlier** turns of a loop, and keeps only the pending call's turn?

It does. The narrower request was built and driven twice, through a three-call loop:

```sh
rho run "Use your tools one at a time. First read a.txt. Then read b.txt. Then read c.txt. \
Then print the three exact strings separated by commas." \
  --provider bedrock --model "$M" --reasoning-effort high
```

```text
run 1 exit=0 -> **zx9-quibble, kt4-marlow, qp7-tundra**
run 2 exit=0 -> **zx9-quibble, kt4-marlow, qp7-tundra**
```

A plain thinking turn still works, with 286 bytes of reasoning on stderr, and OpenRouter is
unaffected.

### After the trailing-run rule

A review found a shape the rule mishandled, so the rule changed again and the runs were repeated.
A reviewer also said parallel tool calls in one turn had never been driven, so that ran too.

```text
three-call loop, run 1: zx9-quibble,kt4-marlow,qp7-tundra          exit 0
three-call loop, run 2: **zx9-quibble, kt4-marlow, qp7-tundra**    exit 0
parallel calls in one turn: **b.txt:** `kt4-marlow`                exit 0
a plain thinking turn: 9.9 is larger than 9.11.                    exit 0
```

The parallel-call run finished and printed one of the two files, which is the model choosing what
to say rather than a transport failure. The request was accepted, which is what this run tests.

**What that proves, exactly.** One shape was driven: sequential turns, one tool per turn, each
separated by a tool result, at a loop length of three. It proves that a provider accepts a request
which omits the thinking of earlier separated turns. It does **not** prove the flat count at
length 20 or 100, which is a unit test, and it does not cover a merged run of consecutive
assistant turns, which a review found and which is now covered by a test rather than by a live
run. A reviewer asked for this paragraph, because the first version of it claimed the general
case from one measurement.

## What is proved, and what is not

Proved live:

- rho captures the signature Bedrock sends, and replays it on the next request.
- Bedrock accepts the replayed block, and rejects a corrupted one, so the block travels.
- A turn without thinking is unaffected.

Proved by test only, and stated here so nobody reads more into the runs above:

- **The owner check.** A payload from another provider or another model is dropped. One
  process holds one model today, so a live mismatch needs a model switch inside a session,
  and rho has no such command yet. Four unit tests cover both halves of rule 8.
- **The persisted format.** No production caller writes a session file, so a replay across a
  resume cannot be driven at all. See `D-no-caller-writes-a-session-file`. The format is
  proved in both directions at the unit level.
- **OpenRouter and Azure.** Neither replays yet. Both name every content block in an arm of
  its own, and both send nothing. The hosts behind OpenRouter disagree, and rho has no live
  proof for either direction, so it guesses at neither.
- **Encrypted reasoning.** `RedactedContent` is carried as base64 and replayed as a blob.
  Bedrock did not send one during these runs, so only the translation tests cover it.

## 8. Re-driven on 20260829, when the drop report became data

The drop report moved from a log line to returned data. See
`D-a-drop-report-is-data-not-a-log-line`. The request path is untouched, so these runs check
that claim rather than trust it. Same account, same region, same model, same two files.

```sh
cargo build --release -p rho-cli
rho run "Use your tools. Read a.txt, then read b.txt, then print the two exact strings you \
found, separated by a comma." --provider bedrock --model "$M" --reasoning-effort high
```

```text
run 1 exit=0 -> zx9-quibble, kt4-marlow      439 bytes on stderr
run 2 exit=0 -> zx9-quibble, kt4-marlow      439 bytes on stderr
```

A sequential three-step prompt with `--reasoning full` printed the reasoning and finished at
exit 0, so a thinking turn inside a tool loop still works.

### The corrupted-signature proof no longer reproduces, and the reason is a real defect

Section 2 above broke the signature on 20260821 and Bedrock answered 400. The same break was
applied again today, on top of this change:

```rust
// deliberate break, restored from /tmp after the run
.signature("deliberately-wrong-signature")
```

```text
exit=0 -> zx9-quibble, kt4-marlow
```

**A wrong signature was accepted.** So no signature travels any more. A unit probe on the
exact shape rho sends after a tool result confirms it, and counts the blocks:

```text
PROBE: reasoning blocks in the request = 0
PROBE: drops reported = []
```

The cause is the replay scope, not this change. `D-replay-only-the-current-loop` landed on
20260822, one day **after** the 400 was measured. The scope is the trailing run of assistant
turns, and a tool result ends that run. In a sequential loop the request always ends with a
tool result, so the assistant turn that holds the pending call sits outside the scope, and its
thinking is dropped as history.

`a_separated_turn_in_a_loop_does_not_replay` pins the current rule with an assistant turn
**after** the tool result. rho never builds that shape in a sequential loop, so no test covered
the shape that ships.

This change does not fix it, and it must not: the brief for this lane forbids changing what
the provider sends on the wire. The fix moves the wire, so it needs its own decision, its own
spec amendment, and its own live re-verification. `docs/features.md` now reports
`F-bedrock-reasoning-replay` as `partial` and names what is not built.

The break was restored from a copy in `/tmp`, never with `git checkout`, per
`D-jcode-bash-lessons`. The restore was checked twice:

```sh
grep -c "deliberately-wrong" crates/rho-provider-bedrock/src/lib.rs   # 0
diff -q /tmp/lib.rs.good crates/rho-provider-bedrock/src/lib.rs       # identical
```

### The failure path, driven twice

A session was continued with a **different** model, so the stored payload's owner no longer
matches the request:

```sh
rho run "Use your tools. Read a.txt, then tell me the exact string." --provider bedrock \
  --model us.anthropic.claude-haiku-4-5-20251001-v1:0 --reasoning-effort high
rho run --continue "Now say the word ok." --provider bedrock \
  --model amazon.nova-micro-v1:0 --log info
```

```text
exit=0 -> Ok.        no drop report on stderr
```

**No report, and that is correct.** The new prompt is a user message, so the stored payload is
out of the current loop. Rule 12 drops history by design and reports nothing, which is what
`an_out_of_loop_payload_is_not_reported_as_a_drop` asserts.

So the owner refusal still cannot be driven live, for the same reason section 2's list already
gave, plus the scope defect above. Four unit tests cover it, and they now assert data instead
of a log line. This page claims no live proof of the refusal.
