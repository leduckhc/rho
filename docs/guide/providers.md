# Providers

rho 0.1.0 ships three model providers: `openrouter`, `bedrock`, and `azure`.
Pass the name with `--provider` or set `RHO_PROVIDER`.

## OpenRouter

```sh
export OPENROUTER_API_KEY=sk-or-...
rho run "say hello" --provider openrouter --model anthropic/claude-haiku-4.5
```

OpenRouter uses one credential: `OPENROUTER_API_KEY`.
An empty value counts as missing and stops the run with a named error.

**Reasoning.** Pass `--reasoning-effort low|medium|high|xhigh`.
rho sends it as `{"reasoning": {"effort": "low"}}` in the request body.
`xhigh` maps to `"high"` on the wire.
OpenRouter reads the reasoning text from three field names some hosts use
(`reasoning`, `reasoning_content`, `reasoning_text`) and routes each to stderr.

**Live proof.** Driven on 2026-08-17 and 2026-08-21.
Streamed answers, single and parallel tool calls, `--read-only`, path confinement,
and reasoning at `high` and `xhigh` all passed.
The model used was `anthropic/claude-haiku-4.5`.

## Bedrock

```sh
export AWS_REGION=us-east-1
rho run "say hello" --provider bedrock
```

Bedrock reads your region from `AWS_REGION` and then uses the standard AWS credential
chain (env vars, `~/.aws/credentials`, instance profile, and so on).
rho signs every request with SigV4.
Set `AWS_PROFILE`, `AWS_ACCESS_KEY_ID`, or any other chain variable as usual.

The default model is `global.anthropic.claude-haiku-4-5-20251001-v1:0`.
A bare `anthropic.claude-haiku-4-5-...` id returns 400; a `us.` profile fails outside
`us-east-1`; the `global.` profile works in every region.

**Reasoning.** Pass `--reasoning-effort low|medium|high|xhigh`.
rho sends a `thinking` object holding `budget_tokens`, inside an
`additionalModelRequestFields` block.
rho captures the `signature` Bedrock returns with the reasoning block and
replays it on the next request in the same tool loop.
A corrupted signature returns HTTP 400, which is how the replay was proved live.
A model that does not support extended thinking receives no reasoning field.
Asking a Nova or other non-thinking model to think has no effect and no error.

**Live proof.** Driven on 2026-08-17, 2026-08-19, and 2026-08-21.
Streamed answers, single and parallel tool calls, `--read-only`, path confinement,
reasoning in a two- and three-call tool loop, subagent delegation, and replay
all passed.
The models used were `us.anthropic.claude-haiku-4-5-20251001-v1:0` and
`global.anthropic.claude-haiku-4-5-20251001-v1:0`.

## Azure OpenAI

```sh
export AZURE_OPENAI_ENDPOINT=https://my-resource.openai.azure.com
export AZURE_OPENAI_DEPLOYMENT=my-deployment
export AZURE_OPENAI_API_KEY=...
rho run "say hello" --provider azure --model my-deployment
```

Azure needs three variables.
rho checks them in this order: endpoint, deployment, key.
A missing or empty variable stops the run and names the variable to set.
Azure names a deployment, not a model id, so `--model` is optional
and there is no built-in default.

rho targets the `/openai/v1/responses` API, not the chat-completions path.
Tool results go as `function_call_output` items, not as `role: tool` messages.
The assistant's tool calls are replayed before their outputs, with `call_id` values
matching.

**Reasoning.** Azure does not receive an effort level today.
Setting `--reasoning-effort` prints one warning per effort level and has no effect.
The warning reads: `this provider does not send a reasoning effort yet`.

> **Not verified.** Azure has unit tests behind it and no live run. A later attempt reached a
> real endpoint and got `DeploymentNotFound` for every guessed deployment name, so the check
> did not finish. Treat Azure as untested until you run the commands above against it
> yourself.

## Comparison table

| Provider | Credential variables | Cargo feature | Default model | Tool calls | Reasoning | How far proven |
| --- | --- | --- | --- | --- | --- | --- |
| `openrouter` | `OPENROUTER_API_KEY` | `openrouter` (on by default) | `anthropic/claude-haiku-4.5` | Yes | Effort word on the wire | Live, 2026-08-21 |
| `bedrock` | `AWS_REGION` + AWS chain | `bedrock` (on by default) | `global.anthropic.claude-haiku-4-5-20251001-v1:0` | Yes | Signed block replay | Live, 2026-08-21 |
| `azure` | `AZURE_OPENAI_API_KEY`, `AZURE_OPENAI_ENDPOINT`, `AZURE_OPENAI_DEPLOYMENT` | `azure` (on by default) | None | Yes | Not sent | Unit tests only |

All three features are in the `default` feature set.
A provider left out of the build gives a different error than an unknown name:

```
# Provider compiled out:
rho: the provider "azure" is not in this build.
Rebuild rho with the feature: cargo build --features azure.

# Provider name unknown:
rho: the provider "mycloud" is not known.
Choose one of: openrouter, bedrock, azure.
```

## Choosing a model

If you give no `--model`, rho picks a default and tells you.

```
rho: no model given, so using the default for bedrock:
     global.anthropic.claude-haiku-4-5-20251001-v1:0.
     Set --model or RHO_MODEL to choose another.
```

Pass the deployment name your account holds.
For Bedrock, use an inference profile id (`us.` or `global.`) and not a bare
foundation model id.
A bare Claude 4.5 id returns 400 from Bedrock.
See `docs/verification/models.md` for a sweep of Bedrock model ids and their
tool-calling results.

## Adding a provider

`crates/rho-provider-testkit` is a conformance suite for the `Provider` trait.
It drives a mock server through every rule in the provider contract and asserts
the normalised event stream.
An outside author implements `ProviderHarness` for their provider and calls `run_all`.
See [extending.md](../extending.md) for how to wire a new provider into the build.

## A local model host

```sh
rho --base-url http://localhost:11434 --model qwen2.5
```

Any host that speaks the OpenAI format works: Ollama, vLLM, LiteLLM, LM Studio. rho appends the
standard chat path, so a base with or without `/v1` both work. The provider stays `openrouter`,
because that crate is the OpenAI-compatible client.

Your key still goes with the request, so rho names the host it is going to in a startup notice.
An `https` url is allowed anywhere. Plain `http` is allowed only to `localhost`, `127.0.0.0/8`,
or `[::1]`, and a loopback endpoint also bypasses every proxy, so `HTTP_PROXY` cannot capture
the key. rho follows no redirect, because a redirect could carry the key to another scheme.

`--base-url` with `bedrock` or `azure` stops the run, since each names its endpoint its own way.
See [configuration](configuration.md) for the config key and the variable.

## Storing credentials

Give every provider its key through the environment. That is the only route that works.

> **Partly built.** A `[credentials]` table in `config.toml` parses, and nothing resolves an
> entry, and no provider asks for one. So the table changes nothing today, in silence. A live
> probe confirmed it. See [configuration](configuration.md) for the format, and treat it as
> unfinished.

For a first run, see [quickstart](quickstart.md).

## What does not work yet

Azure reasoning is not sent.
`--reasoning-effort` logs a warning for Azure and has no effect on the request.

No provider reads a credential from a config file. Use the environment.
