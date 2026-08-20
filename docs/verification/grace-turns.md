# Verification — grace turns, driven for real on Bedrock

Date: 2026-08-20. This is AGENTS.md step 11. Every command and every output below is real. The
provider is AWS Bedrock and the model is the latest haiku.

## What the feature must change

A child that runs out of turns has nobody to ask for more. Before this change it was cut off
mid-thought, and the parent received whatever text existed. rho now warns the child a stated
number of turns early, so the child spends a turn writing its summary.

## Setup

```sh
cargo build --release -p rho-cli
unset AWS_PROFILE
export HOME=/tmp/rho-grace-live/home     # a fake home, so the real ~/.rho is never read
MODEL=us.anthropic.claude-haiku-4-5-20251001-v1:0
```

One definition in `$HOME/.rho/agents/counter.md`, with a four-turn cap:

```markdown
---
name: counter
description: Counts files in the session root, one directory per turn, and never stops early.
tools: all
max_turns: 4
---
You explore the session root. Use one tool call per turn, and keep exploring.
Do not stop until you are told to stop. Report what you found.
```

Six files in the session root, one of them in a subdirectory.

## 1. With a two-turn warning, the child finishes cleanly

```sh
rho run "Spawn the counter agent and tell it to list every file under the session root, \
exploring one directory per turn. Then report exactly what the child said." \
  --provider bedrock --model $MODEL --agent-grace-turns 2
```

The child's transcript, read back from the JSONL:

```text
TurnStart {'turn': 1}
Text: I'll start by listing the root directory to see what's there.
ToolStart {'name': 'list'}   ToolEnd {'error': False}
TurnStart {'turn': 2}
Text: Found 5 files in the root and a `sub/` directory. Let me explore the `sub`...
ToolStart {'name': 'list'}   ToolEnd {'error': False}
Delivered {'count': 1}
TurnStart {'turn': 3}
Text: ## Final Summary  **What I Did:** - Listed the root directory and foun...
End {'outcome': 'done'}
```

`Delivered {'count': 1}` is the warning, landing at the boundary after turn two, which is where
two turns remain of four. The child then wrote a summary and the run ended `done`. It never
reached its cap.

The parent's answer carried the child's own structure, including the part a gate cannot check:

```text
- Did not use glob or grep to search for additional files that might exist elsewhere
**Open Questions:**
- Are there any hidden files or directories (starting with `.`) that weren't listed?
```

## 2. With the warning off, the same child is cut off

The same prompt, the same definition, `--agent-grace-turns 0`:

```text
TurnStart {'turn': 1}
TurnStart {'turn': 2}
TurnStart {'turn': 3}
TurnStart {'turn': 4}
End {'outcome': 'out_of_turns'}
```

The parent was told:

```text
The agent used all 4 of its available turns exploring the directory structure. It started
examining the session root and then focused on the `sub` directory, finding 1 file within it
before running out of turns.
```

No summary, no open questions, and the outcome is `out_of_turns`. That is the behaviour before
this change, and the flag reproduces it exactly.

## What this run proves that a test cannot

A test proves the message is pushed, delivered once, and never displaces a user message. It
cannot prove a real model **acts** on the text. This run does: the same child, with the same cap,
ended `done` with a structured summary rather than `out_of_turns` with a fragment.

## What this run does not cover

A full steering queue at grace time, and the retry that follows, are proved by test only. Filling
a real queue mid-run needs a tool that pushes 32 messages, which is not a realistic live scenario.
