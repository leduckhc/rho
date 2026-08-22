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
