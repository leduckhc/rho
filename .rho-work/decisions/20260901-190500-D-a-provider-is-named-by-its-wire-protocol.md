# D-a-provider-is-named-by-its-wire-protocol — anthropic and openai-chat

Date: 20260901. Reference: `D-a-provider-is-named-by-its-wire-protocol`.
Related: `SPEC-provider-interface`, `D-a-provider-names-its-own-credential`.

## The question

Bedrock is broken for the user, and the xdent proxy is the working path. The proxy speaks two
wire protocols on many routes: **Anthropic Messages** on `/*/anthropic/v1/messages`, and
**OpenAI Chat Completions** on `/*/openai/v1/chat/completions`. pi has five entries for the
same proxy, differing only by the base URL prefix.

Should rho add one provider called `xdent`, or two providers named by the wire protocol?

## The evidence

Three providers exist today: `bedrock`, `azure`, `openrouter`. Each is a crate. Each name says
what the code **connects to**, not what it **speaks**. So a new deployment writes a new crate.

pi's `models.json` proves the shape. It carries five `azure-*` entries, but every route uses one
of two APIs. The proxy is one deployment of a protocol, not a protocol of its own.

A live probe confirmed both surfaces answer with dummy auth, so the proxy holds the credential.
Anthropic Messages returned the standard `content` array and streamed a standard SSE event
stream. OpenAI Chat returned a standard completion with `usage.completion_tokens_details`.

## The decision

**A provider is named by its wire protocol. Two new crates ship:**

- `rho-provider-anthropic` — speaks Anthropic Messages against any base URL.
- `rho-provider-openai-chat` — speaks OpenAI Chat Completions against any base URL.

Each provider takes a base URL, an authorization header, and a set of headers, and it never
carries a deployment name in its identifier. So the xdent scenario is a **configuration**,
not a **codebase change**:

```toml
[credentials]
xdent-key = "value"                          # or "env:XDENT_API_KEY", or a helper

[[providers]]
id       = "xdent-claude"
protocol = "anthropic"
base-url = "http://127.0.0.1:58788/dev1/anthropic"
credential = "xdent-key"

[[providers]]
id       = "xdent-gpt"
protocol = "openai-chat"
base-url = "http://127.0.0.1:58788/chat/openai/v1"
credential = "xdent-key"
```

The user then runs `rho --provider xdent-claude --model claude-sonnet-4-6` or
`rho --provider xdent-gpt --model gpt-5.5`. A fourth route needs a config entry, not a crate.

The same crates reach the real services. `rho --provider anthropic --model claude-sonnet-4-5`
uses `https://api.anthropic.com` as the default base URL, and the credential comes from
`ANTHROPIC_API_KEY`. `rho --provider openai-chat --model gpt-4o-mini` uses
`https://api.openai.com/v1` and reads `OPENAI_API_KEY`.

## What this rules out

- No provider crate whose identifier names a deployment.
- No hidden route table in code. A route lives in the user's config file.
- No default that fails open. A crate without a base URL and a credential must not run.
- No provider that speaks two protocols. One crate, one wire format, one parser.

## What this does not decide

- The credential mechanism stays as it is today. This decision uses it, not extends it.
- `rho-provider-azure` keeps its identifier, because it speaks a distinct protocol
  (`openai-responses`) that neither new crate covers. A future refactor may collapse it into
  an `openai-responses` crate, and this decision leaves that door open.
- The default `--base-url` refusal for `azure` stays, because the Azure endpoint is not the
  same shape as an OpenAI-chat base URL. This decision applies only to the two new crates.

## The prompt that made this necessary

The user asked for a provider that is "basically anthropic-like and openai-likes". A crate per
wire family answers that literally, and rho gains real Anthropic and real OpenAI as a side
effect.
