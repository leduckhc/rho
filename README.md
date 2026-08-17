# rho

A composable coding agent harness, written in Rust.

rho is small on purpose. The runtime, the providers, the tools, and the
frontends are separate crates. You pick the parts you need. You can replace
any part with your own crate.

Status: sprint 1 delivered. The repository is private until the first release; see
[docs/release-checklist.md](docs/release-checklist.md).

Sprint 1 delivered. 224 tests. See [workflow.yaml](workflow.yaml) for the
stages and [docs/verification/sprint-1.md](docs/verification/sprint-1.md) for what
was actually run against real services.

`rho` streams answers and runs tools today, against OpenRouter and AWS Bedrock.
Azure OpenAI is implemented and unit-tested, but nobody has yet run it live.

## Why

Agent harnesses are heavy. A single session can cost more than 100 MB of resident
memory. That cost makes many parallel sessions impractical. rho treats memory and
start time as features, so it measures them.

**101 idle sessions fit in 10.7 MiB.** One more session costs about 25 KB. A whole
process holding one session peaks at 8.3 MiB. Every number, and the command that
produced it, is in [docs/benchmarks.md](docs/benchmarks.md). Reproduce them with
`bash bench/footprint.sh`.

Those figures are not the same measurement as the published per-session numbers for
process-per-session harnesses. `docs/benchmarks.md` says so plainly, and shows the
comparison that favours the others.

## Layout

| Crate | Role |
| --- | --- |
| `rho-core` | Session, agent loop, event stream, provider and tool traits, hooks. No HTTP. |
| `rho-config` | Layered configuration and profiles. |
| `rho-tools` | Built-in file and shell tools. |
| `rho-plugin` | Out-of-process plugin host over stdio JSON-RPC. |
| `rho-provider-openrouter` | OpenRouter and OpenAI-compatible endpoints. |
| `rho-provider-bedrock` | AWS Bedrock Converse. |
| `rho-provider-azure` | Azure OpenAI Responses. |
| `rho-tui` | Minimal terminal interface. |
| `rho-acp` | Agent Client Protocol server frontend. |
| `rho-provider-testkit` | A reusable conformance suite. Run it against your own `Provider`. |
| `rho-cli` | The `rho` binary. Frontends and providers are cargo features. |

## Build

```sh
cargo build --workspace --all-features
cargo test --workspace --all-features
```

Smallest useful build:

```sh
cargo build -p rho-cli --no-default-features --features minimal
```

## Use

```sh
export OPENROUTER_API_KEY=...
rho run "Read note.txt and tell me what it says" \
  --provider openrouter --model anthropic/claude-haiku-4.5

rho                      # the interactive terminal interface
rho --read-only ...      # deny every tool that can change state
```

Tools are confined to the session root. A path outside it is refused, including
one reached through a symlink. `--read-only` denies every mutating tool, so you can
point rho at a repository you do not trust.

## Extending

rho has three tiers, named for what each contributes.

| Tier | Name | Contributes |
| --- | --- | --- |
| 0 | Core tools | The irreducible set: `read`, `write`, `edit`, `list`, `glob`, `grep`, `bash`, `task`, `task_cancel`. No network. |
| 1 | Capability loaders | Nothing of their own. Skills, MCP servers, and subprocess plugins load somebody else's capability. |
| 2 | Extensions | New tools and new hooks, in any crate, behind a cargo feature. |

A hook sees a tool call before it runs. It may observe it, rewrite its arguments, or
refuse it with a reason the model reads. That is how a guardrail works.

See [docs/extending.md](docs/extending.md).

## Contributing

Read [AGENTS.md](AGENTS.md) first. It holds the development flow as a checklist, from
brainstorm to spec, then test-first implementation, then live verification, then the doc
update. Every step in it comes from a defect this project shipped.

## Licence

MIT. See [LICENSE](LICENSE).
