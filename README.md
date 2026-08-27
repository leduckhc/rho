# rho

A composable coding agent harness, written in Rust.

rho is small on purpose. The runtime, the providers, the tools, and the
frontends are separate crates. You pick the parts you need. You can replace
any part with your own crate.

Status: pre-release, version 0.1.0. The repository is private until the first release; see
[docs/release-checklist.md](docs/release-checklist.md).

**New to rho? Read the [user guide](docs/guide/index.md).** It covers install, the terminal
interface, configuration, providers, permissions, and
[what is not built yet](docs/guide/status.md).

1464 tests pass in the workspace. See
[docs/verification/](docs/verification/) for what was actually run against real services.
[agentic-workflow.yaml](agentic-workflow.yaml) holds the workflow that agents follow to
build a feature.

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
| `rho-plugin` | Out-of-process plugin host over stdio JSON-RPC. A library today; the `rho` binary loads no plugin. |
| `rho-provider-openrouter` | OpenRouter and OpenAI-compatible endpoints. |
| `rho-provider-bedrock` | AWS Bedrock Converse. |
| `rho-provider-azure` | Azure OpenAI Responses. |
| `rho-tui` | Minimal terminal interface. |
| `rho-acp` | Agent Client Protocol server frontend. A library today; the `rho` binary exposes no ACP command. |
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

### Subagents

An agent is a markdown file. Put one in `~/.rho/agents/`, or in `.rho/agents/` in a
repository you trust, and rho offers it to the model.

```markdown
---
name: scout
description: Fast recon. Locates code and reports where things are.
tools: read, grep, list
---
You locate code and report where things are. You do not change files.
```

Write the tool list on one line, or as a YAML sequence: `tools: [read, grep, list]`. When a
file does not load, rho says which file, what is wrong, and what to change.

```sh
rho run "Have the scout find where the parser lives" --trust-project
```

A child runs in a fresh conversation, so its reading never fills the parent's
context. Only a summary comes back. Four rules make delegation safe:

- A child's tool set is the parent's set, filtered by the child's list. It can never
  gain a tool the parent lacks.
- A child's approval policy is the parent's policy composed with its own, so a child
  is never more permissive than its parent.
- The session root is inherited and never overridable, and the sandbox may only narrow.
- A child holds no credentials from the parent's environment.

rho can also check the work. Declare the files a child must deliver, and rho verifies
them after the child stops rather than trusting the child's word:

```sh
rho run 'Have the writer produce report.md, and require artifacts=["report.md"]'
```

A child that claims success without delivering is reported as rejected, and the
refusal names the missing file.

| Flag | Bounds |
| --- | --- |
| `--max-children-per-parent` | How many children run at once. Default 4. |
| `--max-live-agents` | How many agents live in the process. Default 32. |
| `--max-agent-tool-calls` | How many tool calls one child may make. Default 64. |
| `--child-timeout-secs` | How long a child may run. Default 600. |

Every contract this feature exposes is in one page:
[docs/contracts-subagents.md](docs/contracts-subagents.md). The reasoning behind each one is in
[docs/specs/20260818-000223-SPEC-subagents.md](docs/specs/20260818-000223-SPEC-subagents.md).

## Extending

rho has three tiers, named for what each contributes.

| Tier | Name | Contributes |
| --- | --- | --- |
| 0 | Core tools | The irreducible set: `read`, `write`, `edit`, `list`, `glob`, `grep`, `bash`, `task`, `task_cancel`. No network. The subagent tools join it when an agent definition loads. |
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
