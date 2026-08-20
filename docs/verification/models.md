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

## The Bedrock default moved to the latest Haiku

Date: 20260819. The owner asked for the latest Haiku from Bedrock. Everything below was run, not
read.

`aws bedrock list-inference-profiles` reports three Haiku profiles in `us-east-1`, all `ACTIVE`:

```
us.anthropic.claude-3-haiku-20240307-v1:0        ACTIVE
us.anthropic.claude-haiku-4-5-20251001-v1:0      ACTIVE
global.anthropic.claude-haiku-4-5-20251001-v1:0  ACTIVE
```

`ACTIVE` is not the same as callable. Three ids were run through `rho run`:

| id | us-east-1 | eu-west-1 |
| --- | --- | --- |
| `anthropic.claude-haiku-4-5-20251001-v1:0` | 400 | 400 |
| `us.anthropic.claude-haiku-4-5-20251001-v1:0` | works | 400 |
| `global.anthropic.claude-haiku-4-5-20251001-v1:0` | works | works |

The commands, verbatim:

```sh
./target/release/rho run "Reply with exactly: OK" \
  --provider bedrock --model anthropic.claude-haiku-4-5-20251001-v1:0
# rho: client error: status 400: Bedrock rejected the request as invalid.

AWS_REGION=eu-west-1 ./target/release/rho run "Reply with exactly: OK" \
  --provider bedrock --model us.anthropic.claude-haiku-4-5-20251001-v1:0
# rho: client error: status 400: Bedrock rejected the request as invalid.

AWS_REGION=eu-west-1 ./target/release/rho run "Reply with exactly: OK" \
  --provider bedrock --model global.anthropic.claude-haiku-4-5-20251001-v1:0
# OK
```

So a Claude 4.5 model needs an inference profile, and the profile must be the global one. The new
default is `global.anthropic.claude-haiku-4-5-20251001-v1:0`. See
`D-bedrock-default-is-the-global-haiku`.

### What the new default was driven through

```
rho run --provider bedrock, no --model:
  rho: no model given, so using the default for bedrock:
       global.anthropic.claude-haiku-4-5-20251001-v1:0. Set --model or RHO_MODEL to choose another.
  DEFAULT OK

one tool call:   "How many files are in the crates directory?"  -> 15
two tool calls:  "Run ls on crates, then ls on docs."           -> crates: 15, docs: 15
```

**Two tool calls in one turn is the sprint-1 Bedrock defect**, where rho answered one tool call
and returned 400 for two. It passes.

The interface was then driven on Bedrock, streaming, with markdown and a table. The banner reads
`global.anthropic.claude-haiku-4-5-20251001-v1:0 · bedrock`, the heading and emphasis style, and
the table's right-aligned column aligns. The screenshot is `shots/13-bedrock-haiku45.png`.

### The old default

`amazon.nova-micro-v1:0` is still `ACTIVE` and was not replaced because it broke. It was chosen
when the question was the smallest model that calls a tool four times out of four. The rows above
in this file still record that sweep, and they stay, because they were true when measured.

### Two guards were added

`the_bedrock_default_is_the_latest_haiku_on_a_global_profile` asserts the default names a
`haiku-4-5` and starts with `global.`, so a later edit cannot quietly pin it to one region.

`no_default_names_a_bare_claude_45_model` asserts no provider's default is a bare Claude 4.5 id,
because Bedrock rejects those with a 400 on the first prompt.
