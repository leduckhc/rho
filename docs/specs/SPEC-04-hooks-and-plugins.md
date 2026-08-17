# SPEC-04 — Hooks and plugins

Status: draft for sprint 1.
Owning crates: `rho-core` (the `Hook` trait and chain), `rho-plugin` (the
out-of-process host).

rho extends in two tiers. Tier 1 is a compiled Rust `Hook`. Tier 2 is an
out-of-process plugin over stdio JSON-RPC. `ADR-001` records why these two tiers,
and why no WASM in sprint 1.

Features covered: F-40 (hook trait), F-41 (lifecycle points), F-42 (out-of-process
plugin), F-43 (plugin schema cache), F-61 (full tool list at turn one).

Scope note: F-40 and F-41 are `planned` in `docs/features.md`. The `Hook` trait
and two hook points ship in sprint 1 because the agent loop and the S3 tests need
them. The full seven-point lifecycle is planned. F-42 ships in sprint 1. F-43 is
`planned`; sprint 1 defines the cache shape but a live connection may still lag.

## 1. The `Hook` trait (Tier 1)

A hook is a compiled struct. It fires at defined points in the agent loop. It has
zero process overhead. Sprint 1 defines two points. The loop calls them in
registration order.

```rust
use crate::ToolOutput;
use async_trait::async_trait;

/// A mutable view of a pending tool call, passed to `before_tool_call`.
pub struct ToolCallView<'a> {
    pub name: &'a str,
    /// The parsed arguments. A hook may edit them in place before execution.
    pub arguments: &'a mut serde_json::Value,
}

/// The outcome of a `before_tool_call` hook.
#[derive(Clone, Debug, PartialEq)]
pub enum HookOutcome {
    /// Let the call proceed to the next hook, then the tool.
    Continue,
    /// Stop the call. The loop makes an error tool result with this reason.
    Block { reason: String },
}

#[async_trait]
pub trait Hook: Send + Sync {
    /// A stable name for logs.
    fn name(&self) -> &str;

    /// Fires before a tool runs, after argument parsing. The hook may edit the
    /// arguments in place. The first `Block` stops the call.
    async fn before_tool_call(&self, _call: &mut ToolCallView<'_>) -> HookOutcome {
        HookOutcome::Continue
    }

    /// Fires after a tool finishes, before the result is appended. The hook may
    /// edit the output in place.
    async fn after_tool_result(&self, _name: &str, _output: &mut ToolOutput) {}
}
```

## 2. The hook chain and its ordering guarantee

```rust
use std::sync::Arc;

pub struct HookChain {
    hooks: Vec<Arc<dyn Hook>>,
}

impl HookChain {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }
    /// Add a hook. Registration order is the run order.
    pub fn push(&mut self, hook: Arc<dyn Hook>) {
        self.hooks.push(hook);
    }
    pub fn hooks(&self) -> &[Arc<dyn Hook>] {
        &self.hooks
    }
}

impl Default for HookChain {
    fn default() -> Self {
        Self::new()
    }
}
```

Ordering guarantee:
- Hooks run in registration order for every point.
- For `before_tool_call`, the chain stops at the first `Block`. Later hooks do not
  run for that call.
- Each hook sees the argument edits made by an earlier hook.
- `after_tool_result` runs every hook, in order. There is no block at this point.

## 3. Planned lifecycle points (F-41)

The full set is planned, not sprint 1: session start, before provider request,
after provider response, before tool call, after tool result, turn end, session
end. Sprint 1 ships `before_tool_call` and `after_tool_result` only. Adding a
point is an interface change, so the trait uses default methods to keep old hooks
source-compatible when a point is added.

## 4. Tier-2 plugin protocol

A plugin is a subprocess in any language. It speaks JSON-RPC 2.0 over stdio, one
JSON object per line, LF framing. The host is `rho-plugin`. The host launches the
subprocess, performs the handshake, lists the plugin tools, and serves each call.
A plugin tool implements the `Tool` trait through a host-side proxy, so the agent
loop treats a plugin tool the same as a built-in tool.

### 4.1 Framing

- One JSON object per line. The delimiter is `\n`.
- The host strips a trailing `\r`.
- A message is a JSON-RPC 2.0 request, response, or notification.

### 4.2 Handshake

The host sends `initialize`. The plugin replies with its protocol version and its
tool list. The host advertises the tools at once.

Request (host to plugin):

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"host":{"name":"rho","version":"0.1.0"}}}
```

Response (plugin to host):

```json
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"plugin":{"name":"my-plugin","version":"0.1.0"},"tools":[{"name":"search_docs","description":"Search the docs","kind":"search","inputSchema":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}}]}}
```

Each tool entry has `name`, `description`, `kind`, and `inputSchema`. The `kind`
maps to `ToolKind`. The `inputSchema` is a JSON Schema object.

### 4.3 Tool call

The host sends `call_tool` when the model calls a plugin tool.

Request:

```json
{"jsonrpc":"2.0","id":2,"method":"call_tool","params":{"callId":"call_7","name":"search_docs","arguments":{"query":"retry"}}}
```

Streamed update (plugin to host, a notification, no id):

```json
{"jsonrpc":"2.0","method":"tool_update","params":{"callId":"call_7","output":"scanning 12 files"}}
```

Final response:

```json
{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"found 3 matches"}],"isError":false}}
```

The `content` array uses the same content-block JSON as `SPEC-01`. The host maps
it to `ToolOutput`.

### 4.4 Cancel

The host sends a `cancel` notification when the turn is cancelled. The plugin
stops the call and returns an error result or no result. The host stops waiting.

```json
{"jsonrpc":"2.0","method":"cancel","params":{"callId":"call_7"}}
```

### 4.5 Shutdown

The host sends `shutdown`, then closes stdin. The plugin flushes and exits. The
host waits a short grace period, then kills the process.

```json
{"jsonrpc":"2.0","id":99,"method":"shutdown","params":{}}
```

### 4.6 Crash isolation

A plugin runs in its own process. A crash must not take down the session.

Rules:
- A plugin call that fails, times out, or hits a closed pipe returns a
  `ToolError` and an error `ToolResult`. The session continues.
- The host never calls `unwrap` on plugin output. A malformed line is logged and
  dropped, not fatal.
- A dead plugin is marked unavailable. Its tools return an error result that tells
  the model the tool is unavailable. The tool list shape does not change
  mid-session, so the prompt prefix stays stable. See section 5.
- The default per-call timeout is 120000 ms.

### 4.7 Host types

```rust
use std::sync::Arc;
use crate::{Tool, ToolKind};

/// A launched plugin subprocess.
pub struct PluginProcess {
    // child handle, framed stdio, pending-call table
}

pub struct PluginHost {
    plugins: Vec<Arc<PluginProcess>>,
}

impl PluginHost {
    pub fn new() -> Self {
        Self { plugins: Vec::new() }
    }
    /// Launch a plugin from a command, then handshake and read its tool list.
    pub async fn launch(&mut self, command: &str, args: &[String]) -> Result<Arc<PluginProcess>, PluginError>;
    /// The proxied tools from every live plugin, as `Tool` trait objects.
    pub fn tools(&self) -> Vec<Arc<dyn Tool>>;
    /// Shut down every plugin.
    pub async fn shutdown(&mut self);
}

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("failed to launch plugin: {0}")]
    Launch(String),
    #[error("handshake failed: {0}")]
    Handshake(String),
    #[error("plugin call timed out")]
    Timeout,
    #[error("plugin process is not available")]
    Unavailable,
    #[error("protocol error: {0}")]
    Protocol(String),
}
```

## 5. Schema cache (F-43, planned)

jcode idea: a plugin's tool schemas come from an on-disk cache. So tools are
advertised in the very first provider request, and a late plugin connection never
invalidates the prompt cache.

Sprint 1 defines the cache file shape and reads it when present. The full
background-connect flow is planned.

Cache file, one per plugin, under the rho state directory:

```json
{"version":1,"plugin":"my-plugin","tools":[{"name":"search_docs","description":"Search the docs","kind":"search","inputSchema":{"type":"object","properties":{"query":{"type":"string"}}}}]}
```

Rules:
- At session start, the host reads each cache file and advertises those tools in
  the first request. This keeps the prefix stable per `SPEC-01` section 7 and F-60.
- When the live plugin connects, the host compares the live schema to the cache.
  A match means no change. A mismatch updates the cache for the next session but
  does not change the current session prefix.
- A tool call to a plugin that has not yet connected waits up to the call timeout,
  then returns an error result.

## 6. Test cases

Hooks, in `crates/rho-core/tests/`:
- `hook_chain_runs_in_registration_order` — two hooks record their order; the
  first registered runs first.
- `hook_before_tool_call_first_block_wins` — when the first hook blocks, the
  second hook does not run and the tool does not execute.
- `hook_before_tool_call_edits_arguments` — a hook edits an argument and the tool
  sees the edited value.
- `hook_after_tool_result_edits_output` — a hook edits the output and the loop
  appends the edited result.

Plugin host, in `crates/rho-plugin/tests/`:
- `plugin_host_launches_and_handshakes` — the host launches a stub plugin and
  reads its protocol version.
- `plugin_host_lists_tools_from_handshake` — the tool list from `initialize`
  appears in `PluginHost::tools`.
- `plugin_host_calls_tool_and_gets_result` — a `call_tool` returns a `ToolOutput`
  with the expected content.
- `plugin_host_forwards_tool_updates` — a `tool_update` notification reaches the
  tool context update channel.
- `plugin_host_crash_returns_error_not_panic` — a plugin that exits mid-call
  yields a `ToolError`, the host does not panic, and the session continues.
- `plugin_host_malformed_line_is_dropped` — a non-JSON line is ignored, not fatal.
- `plugin_host_cancel_stops_call` — a cancel notification ends a pending call.
- `plugin_schema_cache_roundtrips` — a cache file parses to the same tool specs
  it was written from.
- `plugin_tool_advertised_from_cache_before_connect` — tools from the cache
  appear in `PluginHost::tools` before the plugin process connects.
- `plugin_host_call_times_out` — a plugin that never answers a call returns
  `PluginError::Timeout` after the per-call timeout, so one bad plugin cannot
  hang the agent.
- `plugin_host_enormous_line_does_not_panic` — a plugin that writes a 20 MB line
  before its result does not panic the host; the host caps the line and still
  reads the result.
- `plugin_host_clean_shutdown_leaves_no_orphan` — after `shutdown` the plugin is
  marked unavailable and reaped, so no orphan process is left behind.

The stub plugin is a second binary in this crate, `rho_stub_plugin`. Its path
comes from `CARGO_BIN_EXE_rho_stub_plugin`. One binary drives every case; the
first command-line argument selects the behaviour (`normal`, `crash`, `hang`,
`garbage`, `bigline`). No test reaches the network.

## 7. Out of scope for sprint 1

- The five extra hook points in F-41. Sprint 1 ships two points.
- The background connect flow for the schema cache. Sprint 1 reads the cache and
  serves calls once the plugin connects.
- Plugin discovery from a config directory. A plugin is launched by an explicit
  command in sprint 1.
- A hook that rewrites the provider request body or messages (F-65). That needs
  the before-request point, which is planned.
- WASM plugins. See `ADR-001`.
- Slash commands (F-44) and skills (F-45). Those are planned.
