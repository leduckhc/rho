# SPEC-09 — Model Context Protocol client

Status: draft for sprint 2.
Owning crate: `rho-mcp`.
Features: new. MCP client, shared server pool, schema cache.

## 1. Why MCP, and why the pool matters more here than anywhere else

The Model Context Protocol is how the rest of the world ships agent tools. Supporting
it means rho reaches a large existing ecosystem without rho writing an integration for
each service.

rho already has `rho-plugin`, its own stdio JSON-RPC tool protocol. MCP does not
replace it. They answer different questions. `rho-plugin` is for a tool written **for
rho**, with rho's `ToolKind` and approval model. MCP is for a tool written for
**everyone**. Both are hosted, and both feed the same `ToolRegistry`.

**The pool is the part that matters for rho specifically.** rho exists to run many
sessions at once, and the measured cost is about 25 KB per extra session. An MCP server
is a whole process, often a Node or Python program costing tens of megabytes. If every
session spawned its own copy of every configured server, fifty sessions with three
servers each would spawn 150 processes and undo the entire footprint argument.

So a server is shared by default. Sessions reference-count it. The last session to
release it stops it. A server that must not be shared says so in its config.

## 2. Design rules

- **Never block start-up on a handshake.** A session must be usable before an MCP
  server has connected.
- **Never break the prompt cache.** The tool list in the first request must already
  contain the MCP tools, or the model pays to re-read the conversation when they
  appear. See section 4.
- **A server is shared unless it says otherwise.** See section 1.
- **A slow or broken server degrades one tool, never the session.** A server that never
  answers must not hang a turn.
- **An MCP tool is untrusted.** It is code somebody else wrote, reached over a pipe. Its
  name, its schema, and its output are all untrusted input. See section 6.

## 3. Transport and protocol

JSON-RPC 2.0. Two transports in sprint 2.

- **stdio**: rho spawns the server and speaks over its stdin and stdout. This is the
  common case.
- **HTTP with SSE**: rho connects to a URL. `rho-mcp` must not link an HTTP client
  itself, because `rho-core` stays HTTP-free and `rho-mcp` sits beside it. So the HTTP
  transport is a trait the caller supplies, and the `rho-cli` wiring passes an
  implementation backed by the `reqwest` already present in the provider crates.

Handshake, then use:

1. `initialize` with the protocol version, client capabilities, and client info.
2. `notifications/initialized`.
3. `tools/list`, which may page with a cursor.
4. `tools/call` for each invocation.

Sprint 2 implements tools only. Resources and prompts are section 8.

## 4. The schema cache, which is the whole trick

An MCP server only reports its tools after `initialize` and `tools/list`. That is a
round trip against a process rho has just spawned. Two naive designs both lose.

- Block start-up until every server answers. The session takes seconds to become
  usable.
- Connect lazily and register the tools when they arrive. The tool list changes
  mid-conversation, which rewrites the stable prefix and throws away the provider's
  prompt cache. `SPEC-01` section 1 forbids exactly this.

So rho keeps an on-disk cache of the tool schemas each server reported last time, at
`~/.rho/mcp-schema-cache.json`. At start-up rho **advertises from the cache**, so the
first request already carries the tools. The real connection happens on a background
task. A `tools/call` that arrives before the handshake finishes waits for it, which is
connect-on-first-call. After a real connection, the live schemas replace the cache
entry.

`rho-plugin` already uses this pattern for its own tools, in `crates/rho-plugin/src/cache.rs`.
Follow it, and share the shape where it is genuinely the same.

**Two correctness guards, and they are not optional.**

- **The cache entry is keyed by a fingerprint of the server config**: the command, the
  arguments, the environment, the transport, the URL, and the shared flag. Change the
  config and the fingerprint changes, so a stale entry is ignored. Without this, rho
  would advertise a tool that a reconfigured server no longer has.
- **The cache is a hint, never truth.** A `tools/call` goes to the live server. If the
  live list differs from the cache, the registry reconciles and the cache is rewritten.
  A cached tool that no longer exists must produce a clear error, not a panic.

## 5. Naming

An MCP tool is namespaced, because two servers may both expose `search`.

```
mcp__<server>__<tool>
```

A hyphen becomes an underscore, since some providers reject a hyphen in a tool name.

Collisions must be impossible, not unlikely. Two servers whose names differ only by a
hyphen and an underscore would collide after the replacement, so a duplicate final name
is an error at registration, and it names both servers.

## 6. Security

An MCP server is semi-trusted: the user configured it. It is still somebody else's
code, and it may be compromised or simply careless.

- **A server does not classify its own tools.** MCP has no `ToolKind`. So every MCP tool
  reports `ToolKind::Other`, which `ToolKind::is_read_only` treats as mutating, so a
  read-only policy denies it. This follows decision D-017, where a plugin's own claim
  about its kind was refused for the same reason. A later feature may let the **user's**
  configuration grant a kind to a named MCP tool. The server's opinion never counts.
- **Approval runs before every call**, as for any tool.
- **A tool name from a server is untrusted.** Validate it against a strict pattern
  before it becomes a registry key. Reject a name with a path separator, a control
  character, or a leading dot.
- **A schema from a server is untrusted.** Cap its size. A vast schema in the system
  prompt is a denial of service against the context window, and it is charged to the
  user.
- **Output is untrusted.** Cap it, and sanitise it before it reaches the TUI, exactly
  like tool output.
- **A line is capped**, as in decision D-016, where an uncapped line reader reached
  805 MB of resident memory.
- **The environment is scrubbed** for a stdio server, as in decision D-019. A server
  gets no variable whose name looks like a credential, unless its own config sets one
  explicitly. A configured server often needs a token, so the config may add one back
  by name; inheriting the whole environment is what we refuse.
- **A timeout on every call.** A server that never answers fails one call.
- **No orphan process.** A stdio server runs in its own process group. Releasing the
  last reference kills the group.

## 7. Public API

```rust
/// How to reach one server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub transport: McpTransport,
    /// Extra environment for a stdio server. Added after the credential scrub, so a
    /// server that needs a token names it here.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// False keeps this server private to one session. Default true.
    #[serde(default = "default_true")]
    pub shared: bool,
    #[serde(default)]
    pub call_timeout_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpTransport {
    Stdio { command: String, #[serde(default)] args: Vec<String> },
    Http { url: String, #[serde(default)] headers: BTreeMap<String, String> },
}

/// A tool as a server described it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct McpToolDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

/// The client for one server.
pub struct McpClient { /* private */ }

impl McpClient {
    /// Connect and handshake. Returns the tool list.
    pub async fn connect(config: &McpServerConfig, limits: McpLimits)
        -> Result<(Self, Vec<McpToolDef>), McpError>;
    pub async fn call(&self, tool: &str, args: serde_json::Value)
        -> Result<ToolOutput, McpError>;
    pub async fn shutdown(&self);
}

/// The shared pool. One process per config fingerprint, reference counted.
pub struct McpPool { /* private */ }

impl McpPool {
    pub fn new(limits: McpLimits) -> Arc<Self>;
    /// Take a reference to a server, connecting in the background if needed.
    pub async fn acquire(self: &Arc<Self>, config: &McpServerConfig)
        -> Result<McpHandle, McpError>;
    /// How many live servers the pool holds. For a test and for a status line.
    pub async fn live_count(&self) -> usize;
}

/// A session's reference to a pooled server. Dropping it releases the reference.
pub struct McpHandle { /* private */ }

/// Build `rho_core::Tool` objects for a set of servers, advertising from the cache.
///
/// Returns at once. It does not wait for a handshake.
pub async fn tools_for(
    pool: &Arc<McpPool>,
    configs: &[McpServerConfig],
    cache: &McpSchemaCache,
) -> Result<Vec<Arc<dyn Tool>>, McpError>;

#[derive(Clone, Copy, Debug)]
pub struct McpLimits {
    pub call_timeout_ms: u64,
    pub connect_timeout_ms: u64,
    pub max_line_bytes: usize,
    pub max_output_bytes: usize,
    pub max_schema_bytes: usize,
    pub max_tools_per_server: usize,
}
```

## 8. Test cases

Protocol, against a stub server binary in the crate:
- `connect_performs_initialize_then_lists_tools`
- `call_returns_the_tool_output`
- `a_server_error_becomes_a_tool_error_not_a_panic`
- `a_paged_tools_list_is_fully_collected`
- `an_unknown_protocol_version_is_reported_clearly`

Robustness:
- `a_server_that_never_answers_times_out_and_fails_one_call`
- `a_server_that_dies_mid_call_returns_an_error_and_the_session_survives`
- `a_garbage_line_does_not_panic_the_client`
- `a_line_longer_than_the_cap_is_refused`
- `a_schema_larger_than_the_cap_is_refused`
- `a_server_advertising_too_many_tools_is_capped`

Naming, all offline:
- `dispatch_name_prefixes_the_server_and_replaces_hyphens`
- `two_servers_that_collide_after_replacement_are_an_error`
- `a_tool_name_with_a_path_separator_is_refused`
- `a_tool_name_with_a_control_character_is_refused`

The cache:
- `tools_are_advertised_from_the_cache_before_any_connection`
- `a_config_change_invalidates_the_cached_entry`
- `a_live_connection_replaces_the_cached_entry`
- `calling_a_cached_tool_that_no_longer_exists_is_a_clear_error`
- `tools_for_returns_without_waiting_for_a_handshake` — bound it with a timeout, so a
  blocking implementation fails instead of hanging.

The pool, which is the footprint argument:
- `two_sessions_sharing_a_config_start_one_process`
- `a_server_marked_not_shared_starts_one_process_per_session`
- `releasing_the_last_handle_stops_the_server`
- `releasing_one_of_two_handles_keeps_the_server_running`
- `a_different_config_starts_a_different_process`
- `dropping_the_pool_kills_every_server`

Security:
- `every_mcp_tool_reports_tool_kind_other` — the D-017 rule. A server never classifies
  itself.
- `a_read_only_policy_denies_an_mcp_tool`
- `approval_runs_before_the_call_reaches_the_server`
- `a_stdio_server_does_not_inherit_a_credential_variable`
- `a_configured_env_entry_reaches_the_server` — the scrub must not break a server that
  legitimately needs a token.
- `output_with_an_escape_sequence_is_sanitised`

## 9. Out of scope for sprint 2

- MCP resources and prompts. Tools first, because tools are what a coding agent needs.
- rho **as** an MCP server, exposing its own tools to another client. Worth doing, and
  it needs its own spec. The `ToolRegistry` already holds everything such a server
  would advertise, so the work is a frontend, not a redesign.
- OAuth for an HTTP server. Sprint 2 sends configured headers only.
- Sampling, where a server asks the client to run a model call.
- Server-initiated notifications, including `tools/list_changed`. Handling that means
  changing the tool list mid-session, and the prompt cache forbids it. It needs a
  design that reconciles at a turn boundary.
