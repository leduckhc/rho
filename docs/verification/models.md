# Model verification, AWS Bedrock

A separate harness produced `~/Work/Vibe/bedrock_results.tsv`, which lists 109 Bedrock
models as `WORKS` or `FAIL`. 54 are marked `WORKS`.

**That file records one capability: a plain completion.** rho is a coding agent, so the
capability that decides whether a model is usable here is **tool calling**. This page
records what happened when the candidates were driven through `rho run` itself.

Every result below is from the real binary against the real service, in
`us-east-1`, on 2026-08-17.

## The headline finding

**"Works" in the source file does not mean "calls tools".** Of six small models that all
pass a plain answer, two cannot use a tool at all or cannot do it reliably.

| Model | Plain answer | Single tool call | Note |
| --- | --- | --- | --- |
| `amazon.nova-micro-v1:0` | pass | **4 of 4** | The chosen default. |
| `mistral.ministral-3-3b-instruct` | pass | **4 of 4** | |
| `nvidia.nemotron-nano-9b-v2` | pass | **4 of 4** | |
| `openai.gpt-oss-20b-1:0` | pass | **4 of 4** | |
| `amazon.nova-lite-v1:0` | pass | **1 of 4** | Flaky. It narrates the intent and often never calls. |
| `google.gemma-3-4b-it` | pass | **0 of 3** | Cannot. See below. |

`google.gemma-3-4b-it` does not use the tool interface. It writes a tool call as prose
instead:

```
```tool_code print(read_file("c.txt")) ```
```

That is the model imitating a tool call in text, so no tool ever runs. The behaviour
repeated on every attempt, so it is a model limitation and not a defect in rho.

`amazon.nova-lite-v1:0` is the more interesting case, because it sometimes works. It emits
a `<thinking>` block describing the plan, then frequently answers without calling the tool.
One run in four completed. **A flaky model is worse than an incapable one**, because a test
suite that runs it once will pass and mislead.

## Parallel tool calls

Two providers had request-shape defects on this exact case, so it is checked separately.
See `docs/verification/sprint-1.md`.

| Model | Two tool calls in one turn |
| --- | --- |
| `amazon.nova-micro-v1:0` | pass |
| `mistral.ministral-3-3b-instruct` | pass |

## The default

`amazon.nova-micro-v1:0`. It is the smallest model that called a tool four times out of
four, and it also handled two calls in one turn.

A default model is a convenience, not a security choice. Decision D-no-four-argument-session-new removed hidden
defaults for the session root and the approval policy, because a wrong value there is a
breach. A wrong model id is a bad answer and a small bill. So a default is safe here, and it
is still **reported** on use, so the choice is never silent:

```
rho: no model given, so using the default for bedrock: amazon.nova-micro-v1:0.
Set --model or RHO_MODEL to choose another.
```

Defaults per provider:

| Provider | Default | Reason |
| --- | --- | --- |
| `bedrock` | `amazon.nova-micro-v1:0` | Verified above. |
| `openrouter` | `anthropic/claude-haiku-4.5` | Small, cheap, widely available, and used throughout this repository's live tests. |
| `azure` | **none** | Azure names a deployment, not a model. Only the account owner knows the deployment names, so no default would be honest. |

## How to reproduce

```sh
cargo build --release -p rho-cli
cd "$(mktemp -d)"
echo "The sweep canary is: copper-otter-12" > c.txt

# A plain answer.
rho run "Reply with exactly the word: pong" \
  --no-skills --provider bedrock --model amazon.nova-micro-v1:0

# A single tool call. The answer must contain the canary.
rho run "Read c.txt and report the canary value." \
  --no-skills --provider bedrock --model amazon.nova-micro-v1:0
```

**Run a tool-call check at least four times before trusting a model.** That is the whole
lesson of the `nova-lite` row.

## What this page does not claim

- **The other 48 working models are unverified for tool calling.** Only six were swept, all
  small ones, because the goal was to choose a default for testing. A large model is more
  likely to call tools well, not less, but that is an expectation and not a measurement.
- No model was checked for a long session, for context compaction, or for a cache hit.
- Only `us-east-1` was used. A model's availability varies by region.
