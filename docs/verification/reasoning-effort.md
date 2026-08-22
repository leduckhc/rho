# Verification — the thinking request and the effort level

Date: 20260821. Spec: `docs/specs/20260819-134615-SPEC-reasoning-across-providers.md`,
section 9. This is `AGENTS.md` step 11, so every command and its real output is here.

Build:

```sh
cargo build --release -p rho-cli
```

Account 980637428984, region `us-east-1`. The thinking model is
`us.anthropic.claude-haiku-4-5-20251001-v1:0`. Output is trimmed to the lines that matter.
The skills notice on stderr is filtered out of each paste.

## 1. The reported defect, before and after

Before this change rho never asked Bedrock for extended thinking, so Claude wrote
`<thinking>` tags into the answer. After it, the reasoning arrives as a structured block.

```sh
rho run "Which is larger, 9.11 or 9.9? Answer in one sentence." \
  --provider bedrock --model "$M" --reasoning-effort medium --reasoning full
```

stdout, the answer alone:

```text
9.9 is larger than 9.11.
```

stderr, the reasoning:

```text
The user is asking me to compare two numbers: 9.11 and 9.9, and answer in one sentence.
...
When comparing decimals, 9.90 > 9.11 (since 0.90 > 0.11)
```

The split matters. stdout carries the answer, so a pipe stays clean, and the reasoning goes
to stderr. No `<thinking>` tag reached either stream.

## 2. A two-call tool loop, with thinking on

Sprint 1 found that Bedrock worked for one tool call and answered 400 for two. Anthropic also
documents that a thinking block must be replayed inside a tool loop, and rho does not replay
one yet. So this run tested the risk rather than assuming it.

```sh
printf 'zx9-quibble\n' > a.txt; printf 'kt4-marlow\n' > b.txt
rho run "Use your tools. Read a.txt, then read b.txt, then print the two exact strings you \
found, separated by a comma." --provider bedrock --model "$M" \
  --reasoning-effort high --reasoning full
```

```text
zx9-quibble, kt4-marlow
```

stderr showed reasoning **inside** the tool-calling turn:

```text
I'll use the read tool to read both files. Since these reads are independent, I can make
both calls at once.
```

Result: exit 0, two files read, and no 400. Bedrock's Converse API accepted the next request
without the thinking block. That is measured now, not assumed. It stays a risk for a provider
that enforces the replay, and section 4 of the spec holds the design for it.

## 3. An unsupported model asks for nothing

Rule 1 of section 9 fails closed. A field the endpoint does not know is a 400 for the whole
turn, so this is the rule that keeps a non-thinking model working.

```sh
rho run "Say the word ok." --provider bedrock --model "amazon.nova-lite-v1:0" \
  --reasoning-effort high
```

```text
ok
```

Nova has no thinking field. rho asked for nothing, and the turn worked.

`anthropic.claude-3-5-haiku-20241022-v1:0` answers 500 in this account, with **and without**
the flag, so it proves nothing either way. That was checked before it was blamed.

## 4. `off` on a thinking model

```sh
rho run "Say the word ok." --provider bedrock --model "$M" --reasoning-effort off
```

```text
ok
```

## 5. A bad level at each of the three sources

Each error names its own source, per `D-the-merge-cannot-name-a-values-source`.

```sh
rho run "hi" --provider bedrock --model "$M" --reasoning-effort ludicrous
# rho: the --reasoning-effort flag is wrong: unknown reasoning effort "ludicrous":
#      the valid levels are off, low, medium, high, and xhigh    (exit 1)

RHO_REASONING_EFFORT=ludicrous rho run "hi" --provider bedrock --model "$M"
# rho: the RHO_REASONING_EFFORT variable is wrong: unknown reasoning effort "ludicrous": ...

printf 'reasoning-effort = "ludicrous"\n' > .rho/config.toml
rho run "hi" --provider bedrock --model "$M"
# rho: the reasoning-effort value "ludicrous" is not valid: unknown reasoning effort ...
```

## 6. A defect this run found, and its fix

The file refusal first printed a sentence that is not a sentence:

```text
rho: cannot parse the config file the merged configuration: the reasoning-effort key value
"ludicrous" is not valid: ...
```

`ConfigError::Parse` needs a path, and four merged-layer parsers passed the literal "the
merged configuration" as one. The fix is `ConfigError::Value`, which names a key and a value
and no file. After it:

```text
rho: the reasoning-effort value "ludicrous" is not valid: unknown reasoning effort ...
rho: the sandbox value "loose" is not valid: unknown sandbox mode "loose". Use off, confined,
     or strict.
```

See `D-a-merged-value-error-names-no-file`. A live run had reported this once before, on
`tui-reasoning`, and it rode along unfixed. Two sightings of one wrong shape is enough.

## 7. The file leg, twice

"Twice" has caught two defects on this branch, so every path runs twice.

```sh
printf 'reasoning-effort = "medium"\n' > .rho/config.toml
rho run "Which is larger, 9.11 or 9.9? One sentence." --provider bedrock --model "$M" \
  --reasoning full     # run twice
```

```text
run 1: 9.9 is larger than 9.11 because 0.9 is greater than 0.11.   exit 0, 368 bytes of reasoning
run 2: 9.9 is larger than 9.11.                                    exit 0, 296 bytes of reasoning
```

A config file alone turns thinking on. The two runs differ in wording, because a model is not
a function, and both carried reasoning.

## What is not verified here

- **OpenRouter and Azure.** This change touches the Bedrock request only. The effort level
  reaches every provider through `CompletionRequest`, and no other crate reads it yet.
- **The replay.** It was unbuilt when these runs happened, and it landed straight after. See
  `docs/verification/reasoning-replay.md` for the live proof, where a corrupted signature
  makes Bedrock answer 400.
- **The budget numbers.** The ladder is a starting point. Section 7 of the spec says a
  measurement gets its own bench.
