# Verification — the `tools` keywords, driven for real on Bedrock

Date: 2026-08-20. This is AGENTS.md step 11. Every command and every output below is real.
The provider is AWS Bedrock and the model is the latest haiku.

## The defect this proves fixed

`tools: all` used to be read as a tool literally named `all`. The intersection dropped it, the
child registry was built from an empty list, and the child ran with no tools. See decision
D-a-tool-keyword-stands-alone.

## Setup

```sh
cargo build --release -p rho-cli
unset AWS_PROFILE                      # this machine has AWS_PROFILE=alquist, which does not exist
export HOME=/tmp/rho-keyword-live/home # a fake home, so the real ~/.rho is never read
MODEL=us.anthropic.claude-haiku-4-5-20251001-v1:0
```

Two definitions in `$HOME/.rho/agents/`:

```markdown
---
name: reader
description: Reads a named file and reports its first line. Use it to read one file.
tools: all
---
Read the file you are asked about. Report its first line verbatim. Do not change any file.
```

```markdown
---
name: talker
description: Answers from its own knowledge only. It holds no tools at all.
tools: none
---
You hold no tools. Answer in one sentence from your own knowledge.
```

One file in the session root: `target.txt`, holding `the secret line is olive-42`.

## 1. `tools: all` gives the child the parent's tools

```sh
rho run "Use spawn_agent with the reader agent. Tell it to read the file target.txt in the \
session root and report its first line." --provider bedrock --model $MODEL
```

```
rho: 2 agent definition(s) available to spawn_agent: reader, talker.
The first line of target.txt is: **the secret line is olive-42**
```

The child read a real file. Before the fix the same definition produced a child with an empty
registry, so this answer was impossible.

## 2. `tools: none` gives the child no tools

```sh
rho run "Use spawn_agent with the talker agent. Tell it to read the file target.txt and report \
the first line. Then tell me exactly what the child reported." --provider bedrock --model $MODEL
```

```
rho: 2 agent definition(s) available to spawn_agent: reader, talker.
The child reported: "I don't have the ability to read files from your system. I can only work
with information you provide directly in our conversation. ..."

The talker agent has no file access tools—it can only answer from its own knowledge.
```

The child held no tool and said so. The parent stayed live and offered the other agent.

## 3. The failure path, and the same thing twice

```sh
rho run "Spawn the reader agent twice, one after the other. First ask it for the first line of \
target.txt. Then ask it for the first line of missing.txt, which does not exist. Report both \
answers plainly." --provider bedrock --model $MODEL
```

```
rho: 2 agent definition(s) available to spawn_agent: reader, talker.
1. **target.txt**: The first line is **the secret line is olive-42**
2. **missing.txt**: The file does not exist at the root of the session.
```

Two spawns in one session. An absent file is a result, and the second child freed its slot, so
the run continued.

## What this run does not cover

A mixed line, such as `tools: all, read`, was proved by test only. Its rule is a loader rule and
it never reaches the provider.
