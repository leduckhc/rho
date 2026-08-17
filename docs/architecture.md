# rho architecture

This document describes the crate dependency graph, the request/response data flow, and the session lifecycle.

---

## Crate dependency graph

```
rho-cli  ──────────────────────────────────────────────┐
  │                                                     │
  ├── rho-tui (optional feature: tui)                   │
  │     └── rho-core                                    │
  ├── rho-acp (optional feature: acp)                   │
  │     └── rho-core                                    │
  ├── rho-provider-openrouter (feature: openrouter)     │
  │     └── rho-core                                    │
  ├── rho-provider-bedrock (feature: bedrock)           │
  │     └── rho-core                                    │
  ├── rho-provider-azure (feature: azure)               │
  │     └── rho-core                                    │
  ├── rho-tools                                         │
  │     └── rho-core                                    │
  ├── rho-plugin                                        │
  │     └── rho-core                                    │
  └── rho-config                                        │
        (no rho-* dependencies)                         │
                                                        │
rho-core: no dependency on rho-tui, rho-acp, or rho-cli
```

**Dependency rules:**

- `rho-core` is a pure library. It has no HTTP server, no terminal, and no CLI dependency.
- `rho-config` has no dependency on any other rho crate.
- `rho-tools` and `rho-plugin` depend only on `rho-core`.
- Each provider crate depends only on `rho-core`.
- `rho-tui`, `rho-acp`, and `rho-cli` are the only crates that depend on `rho-core` from the top level.
- Frontends and providers are cargo features on `rho-cli`. A build with `--no-default-features` compiles neither.

---

## Crate responsibilities

| Crate | Single reason to change |
|-------|------------------------|
| `rho-core` | The agent loop, event stream, message model, `Provider` trait, `Tool` trait, `Hook` trait, and session state. |
| `rho-config` | Reading, merging, and resolving layered configuration and credentials. |
| `rho-tools` | The built-in file and shell tools: read, write, edit, list, glob, grep, bash. |
| `rho-plugin` | The out-of-process plugin host: subprocess lifecycle, JSON-RPC handshake, tool listing, tool dispatch. |
| `rho-provider-openrouter` | The HTTP client for `POST /api/v1/chat/completions` with SSE streaming and tool call assembly. |
| `rho-provider-bedrock` | The `ConverseStream` client with SigV4 signing and the standard AWS credential chain. |
| `rho-provider-azure` | The Azure OpenAI `/responses` client with API key and Entra token auth. |
| `rho-tui` | The terminal renderer: transcript, streaming answer, thinking, tool rows, input editor, status line. |
| `rho-acp` | The stdio JSON-RPC server that allows any process to drive the agent. |
| `rho-cli` | The `rho` binary entry point. Parses arguments, wires crates together, starts one of: TUI, ACP, or print mode. |

---

## Request/response data flow

The flow below traces one user prompt from entry to first token to tool call to turn end.

```
User input
    │
    ▼
rho-cli (argument parse, mode selection)
    │
    ▼
rho-core: AgentLoop
    │
    ├─1─► rho-config: resolve credentials, select model
    │
    ├─2─► rho-core: build Context
    │       • system prompt (stable, set once)
    │       • full tool list (stable, set once)
    │       • message history
    │
    ├─3─► Hook::before_request (optional, planned F-41)
    │
    ├─4─► Provider::stream(&Context) ──► HTTP SSE ──► provider API
    │                                                    │
    │       token events ◄──────────────────────────────┘
    │       tool_call events ◄─────────────────────────────
    │
    ├─5─► Event channel (rho-core::EventSender)
    │       │
    │       ├──► rho-tui: render token/thinking/tool row
    │       └──► rho-acp: forward as JSON-RPC event
    │
    ├─6─► if tool_call event:
    │       │
    │       ├─► Hook::before_tool_call (optional, planned F-41)
    │       │
    │       ├─► ApprovalGate (optional, planned F-29)
    │       │
    │       ├─► ToolSet::dispatch(tool_name, input)
    │       │     │
    │       │     ├── rho-tools built-in
    │       │     └── rho-plugin out-of-process (stdio JSON-RPC)
    │       │
    │       ├─► Hook::after_tool_result (optional, planned F-41)
    │       │
    │       └─► append tool result to Context, loop to step 3
    │
    └─7─► stop_reason == "stop" or "error"
            │
            └─► TurnEnd event, append messages to session log
```

**Key invariants:**

1. Steps 1 and 2 run once per agent run. The system prompt and tool list do not change between turns. This keeps the provider KV cache warm.
2. The event channel is the only coupling between the agent loop and the frontends. `rho-core` does not import `rho-tui` or `rho-acp`.
3. Tool dispatch is synchronous from the loop's perspective. The tool may use async I/O internally.
4. A cancellation token threads through every step. Dropping the token aborts the in-flight HTTP request and any running tool.

---

## Session lifecycle

The lifecycle below covers one process from start to shutdown.

```
process start
    │
    ├─1─► rho-config: load global config, merge project config, resolve env vars
    │
    ├─2─► rho-cli: parse arguments, select frontend (TUI / ACP / print)
    │
    ├─3─► rho-tui or rho-acp: initialize frontend, open display or socket
    │
    ├─4─► rho-plugin: for each configured plugin path, spawn subprocess,
    │       JSON-RPC handshake, load tool schemas (from cache if available)
    │
    ├─5─► rho-core: create Session
    │       • if --session <path>: load JSONL file, replay message tree to leaf
    │       • if --no-session: ephemeral, no file
    │       • if new session: create JSONL file, write header entry
    │
    ├─6─► frontend waits for user prompt
    │       (TUI: keyboard input; ACP: JSON-RPC prompt command)
    │
    ├─7─► AgentLoop.run(prompt)
    │       • see request/response data flow above
    │       • on each turn end: append all new entries to JSONL file (F-50)
    │
    ├─8─► after turn: frontend returns to step 6
    │
    └─9─► Ctrl-C / abort / process signal
            │
            ├─► CancellationToken drop → abort in-flight HTTP and tool
            ├─► rho-plugin: send SIGTERM to each plugin subprocess
            └─► rho-tui or rho-acp: clean shutdown, restore terminal state
```

**State transitions:**

| State | Trigger | Next state |
|-------|---------|------------|
| `Idle` | session loaded | `WaitingForPrompt` |
| `WaitingForPrompt` | user submits prompt | `Running` |
| `Running` | stop reason `stop` | `WaitingForPrompt` |
| `Running` | stop reason `error` (permanent) | `WaitingForPrompt` |
| `Running` | stop reason `error` (transient, retry limit not reached) | `Running` |
| `Running` | cancellation | `WaitingForPrompt` |
| `WaitingForPrompt` | Ctrl-C or signal | `ShuttingDown` |

---

## Open questions (for the architect, stage S2)

1. What is the exact shape of the `Provider` trait? In particular, does it return `impl Stream<Item = Event>` or take a callback? The choice affects cancellation and backpressure.
2. What is the exact shape of the `Tool` trait? Does `execute` take an `AbortSignal` equivalent, and how does streaming output flow back?
3. How does the event channel type the sender and receiver? `tokio::sync::mpsc`, `flume`, or `async-channel`?
4. Settled by decision D-001. The session JSONL format is rho's own. The first record carries a version field. rho does not copy the pi format. A one-way converter is feature F-54, and it is `planned`.
5. Settled by decision D-002. `rho-acp` speaks the real Agent Client Protocol. The authoritative JSON schema is on disk at `~/Work/Vibe/acp-docs/schema/`. The architect reads that schema. The architect does not spike a private wire format. See `docs/specs/SPEC-06-acp.md`.
