# D-bedrock-default-is-the-global-haiku — the latest Haiku, on a global profile

Date: 20260819

## The question

rho's Bedrock default was `amazon.nova-micro-v1:0`. The owner asked for the latest Haiku. Which
id, exactly?

## The decision

**`global.anthropic.claude-haiku-4-5-20251001-v1:0`.**

Not the bare foundation id, and not the `us.` profile.

## The reason, measured

Three ids were run through `rho run` against live Bedrock. The result decided the choice, and no
part of it was read from a table.

| id | us-east-1 | eu-west-1 |
| --- | --- | --- |
| `anthropic.claude-haiku-4-5-20251001-v1:0` | **400** | **400** |
| `us.anthropic.claude-haiku-4-5-20251001-v1:0` | works | **400** |
| `global.anthropic.claude-haiku-4-5-20251001-v1:0` | works | works |

Two facts come out of that.

**A Claude 4.5 model needs an inference profile.** The bare foundation id has no on-demand
throughput, so it returns 400 on the first prompt. `aws bedrock list-foundation-models` reports
it `ACTIVE`, which is true and not the same thing as callable. A default taken from that listing
would have shipped broken.

**The profile has to be the global one.** The `us.` profile is region scoped. A `us.` default
works on the machine that chose it and fails for every caller outside a US region, which is the
worst shape a default can have: correct for its author, broken for a stranger.

## Why the old default was replaced

`amazon.nova-micro-v1:0` is still `ACTIVE`, and it was not replaced because it broke. It was
chosen when the question was "the smallest model that calls a tool four times out of four". A
coding agent needs a model that handles tools and long context well, and Haiku 4.5 is that.

## What was verified before the change shipped

Against live Bedrock, with the new default and no `--model`:

- A plain answer.
- One tool call, which returned the right count.
- **Two tool calls in one turn.** That is the sprint-1 Bedrock defect, where rho worked for one
  tool call and returned 400 for two. It passes now.
- The full interface, streaming, with markdown and a table.

## What this rules out

- **No default read from a model listing.** `ACTIVE` in `list-foundation-models` does not mean
  callable. A default is verified by calling it, through rho, or it is not a default.
- **No region-scoped default.** A `us.` or `eu.` prefix in a built-in default is a defect, because
  the author's region is not the user's. A caller who wants one passes `--model`.
- **No fallback chain.** jcode offers a one-key fallback, and its Bedrock default is end of life,
  so the fallback offered a second end-of-life model and ping-ponged between the two. A fallback
  list without a liveness check trades one dead model for another. rho names one live default and
  reports it.
- **This does not enable extended thinking.** Haiku 4.5 supports it, and rho still never asks for
  it. `rho-provider-bedrock` also still drops a thinking block when building a request, at
  `crates/rho-provider-bedrock/src/lib.rs:613`. That stays open, and this decision does not touch
  it.
