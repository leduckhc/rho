# rho

A composable coding agent harness, written in Rust.

rho is small on purpose. The runtime, the providers, the tools, and the
frontends are separate crates. You pick the parts you need. You can replace
any part with your own crate.

Status: sprint 1, in development. See [workflow.yaml](workflow.yaml).

## Why

Agent harnesses are heavy. A single session can cost more than 100 MB of
resident memory. That cost makes many parallel sessions impractical. rho
treats memory and start time as features.

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

## Licence

MIT. See [LICENSE](LICENSE).
