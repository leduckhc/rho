# Verification: the reasoning token field, and Azure driven for real

Date 20260831. This page records a live probe of three provider surfaces. It answers the open
question in `SPEC-usage-carries-reasoning`, and it tests a contract decision in
`SPEC-choose-a-model-and-configure-a-run`.

Every number here came from a real request. No value is copied from memory.

## The reasoning token count sits at one nested path

The spec first guessed a top-level `reasoning_tokens` field on `usage`. That guess was wrong.
Both providers carry the count in the same nested place, and neither carries it at the top
level:

```
usage.completion_tokens_details.reasoning_tokens
```

| surface | model or deployment | count seen |
| --- | --- | --- |
| OpenRouter | `openai/gpt-5` | 64 |
| Azure, real endpoint | deployment `gpt-5.5` | 13 |
| Azure, through the xdent proxy | deployment `gpt-5.5` | 5 |
| Bedrock | `us.anthropic.claude-haiku-4-5` | the field is absent |

So one parser serves both providers, and Bedrock's `None` is a measured absence.

Two more fields appeared. They are out of scope, and they are recorded so nobody probes twice.
OpenRouter sends `usage.cost_details` beside the `cost` that rho already reads. Azure sends
`usage.latency_checkpoint`, which holds a server-side time to first token.

## Azure lists 456 models, and none of them can be called

`SPEC-choose-a-model-and-configure-a-run` decided that Azure answers `None` when asked for a
model catalogue, because Azure names deployments and not models. The probe confirms it.

```sh
GET  {endpoint}/openai/models?api-version=2025-04-01-preview       -> 200, 456 models
GET  {endpoint}/openai/deployments?api-version=2025-04-01-preview  -> 404
POST {endpoint}/openai/deployments/o3-mini-2025-01-31/chat/completions -> 404
```

Eight listed model ids were each tried as a deployment name. All eight returned 404. A
deployment that the operator had really created answered at once.

So an Azure catalogue would be worse than no catalogue. It would offer 456 names, and every one
would fail. That is the argument for keeping the catalogue optional in the type system.

## rho drives Azure, and it takes two tool calls in one turn

Sprint 1 recorded that Azure rejected every tool call. That is fixed, and this is the drive
that proves it.

rho reads three variables for Azure, and it refuses a base url, so the endpoint is the base:

```sh
export AZURE_OPENAI_ENDPOINT=...        # the resource, or the proxy
export AZURE_OPENAI_DEPLOYMENT=gpt-5.5
export AZURE_OPENAI_API_KEY=...
rho run "read fact.txt and reply with only the pass phrase" --provider azure --model gpt-5.5
```

| path | result |
| --- | --- |
| through the xdent proxy, one tool call | answered `quartz` |
| through the xdent proxy, two tool calls | answered `alpha line, beta line` |
| the real Azure resource, one tool call | answered `quartz` |

The two-tool turn is the important row. The session file holds both calls in **one** assistant
message:

```
assistant message tool_calls: ['read', 'read']
```

## The xdent proxy needs a tunnel

pi's own `azure-openai` and `azure-claude` entries point at `127.0.0.1:58788`, and they carry a
placeholder key, because the proxy holds the real credential. The tunnel is an SSH forward:

```sh
~/bin/xdent-tunnel-opencode      # 127.0.0.1:58788 -> xdent-dev:8787
```

rho does not need it. rho reached the real Azure resource directly with the same result. The
proxy path works too, because rho appends `/openai/v1/responses` to the endpoint, and the proxy
serves that path.
