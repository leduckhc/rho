# SPEC-06 — ACP frontend mapping

Status: draft for sprint 1. Delivery is `planned`, after the TUI. The spec is
written now because it constrains the `rho-core` event model (decision D-002).

Owning crate: `rho-acp`. It depends on `rho-core`. It links no provider crate.

Decision D-002: `rho-acp` speaks the real Agent Client Protocol, the same protocol
`pi-acp` and Zed speak. It is not a private dialect. The owner's app `makit`
already drives `pi` over ACP, so a conforming `rho-acp` is a drop-in backend.

Source of truth: `~/Work/Vibe/acp-docs/schema/v1/schema.json` and its `meta.json`.
This spec does not restate the protocol. It states the mapping from the
`rho-core` event model to ACP notifications, and the gaps for sprint 1.

Features covered: F-90 (ACP frontend), F-91 (prompt), F-92 (steer), F-93 (abort),
F-94 (session commands), F-95 (permission and extension UI).

## 1. Role and transport

`rho-acp` is the ACP **Agent**. The client is `makit`, Zed, or any conforming
peer. The transport is stdio JSON-RPC 2.0, one message per line, LF framing, the
same framing as the plugin protocol in `SPEC-04`.

`rho-acp` handles the ACP agent methods and emits the client-bound notifications.
It owns one `rho_core::Session` per ACP session id.

## 2. Method mapping

The agent methods from `meta.json`:

| ACP method | rho-acp action |
| --- | --- |
| `initialize` | Reply with `protocolVersion: 1` and the agent capabilities in section 6. |
| `authenticate` | No-op in sprint 1. Credentials come from the environment. |
| `session/new` | Create a `Session` with a fresh id and the built-in tools. |
| `session/load` | Planned. Needs the session format (F-50). |
| `session/prompt` | Map `prompt` content blocks to `Vec<ContentBlock>`; call `Session::prompt`; stream updates; reply with a `stopReason`. F-91. |
| `session/cancel` | Call `CancelToken::cancel` on the running turn. F-93. |
| `session/set_mode` | Planned. Sprint 1 has one mode. |
| `session/list`, `session/delete`, `session/resume`, `session/close` | Planned. |
| `logout` | No-op in sprint 1. |

Steering (F-92) maps to a second `session/prompt` while a run is active. Sprint 1
delivers a steer message after the current tool calls finish, before the next
provider turn. Full steer queueing is planned.

## 3. Content block mapping

The `rho-core` `ContentBlock` maps to the ACP `ContentBlock` both ways.

| rho `ContentBlock` | ACP `ContentBlock` |
| --- | --- |
| `Text { text }` | `{"type":"text","text":...}` |
| `Image { source }` | `{"type":"image","data":...,"mimeType":...}` |
| `Thinking` | Not sent as prompt content. Streamed as `agent_thought_chunk`. |
| `ToolCall` | Reported through `tool_call`, not as content. |
| `ToolResult` | Reported through `tool_call_update` content, not as prompt content. |

An inbound ACP `resource_link` or `resource` maps to a `Text` block that names
the resource in sprint 1. Full resource handling is planned.

## 4. Event mapping

`rho-acp` consumes the `AgentEvents` stream from `Session::prompt`. It turns each
`AgentEvent` into a `session/update` notification, or into the final
`session/prompt` response.

| `AgentEvent` | ACP `session/update` |
| --- | --- |
| `Stream(TextStart\|TextDelta)` | `agent_message_chunk` with a text `ContentBlock` |
| `Stream(ThinkingStart\|ThinkingDelta)` | `agent_thought_chunk` with a text `ContentBlock` |
| `Stream(ToolCallEnd { id, arguments })` | `tool_call` with `toolCallId`, a synthesised `title`, `kind`, `status: "pending"`, and `rawInput: arguments` |
| `ToolStart { id }` | `tool_call_update` with `status: "in_progress"` |
| `ToolUpdate { id, output }` | `tool_call_update` with a text `content` item |
| `ToolEnd { id, output }` | `tool_call_update` with `status: "completed"` or `"failed"` and the output as `content`, plus `rawOutput` |
| `TurnStart`, `TurnEnd` | No notification. Internal loop markers. |
| `AgentEnd { stop_reason }` | The `session/prompt` response `stopReason`; see section 5 |

The tool `kind` is the `ToolKind` from `ToolCallEnd` context and the tool spec.
The values match the ACP `ToolKind` set one-to-one, so no remap is needed.

The `messageId` on a chunk is one id per assistant turn. `rho-acp` mints it at
`TurnStart` and reuses it for every chunk in that turn.

## 5. Stop reason mapping

`AgentStopReason` maps one-to-one to the ACP `StopReason`. The set was chosen to
match. This is why `SPEC-01` uses these exact names.

| `AgentStopReason` | ACP `StopReason` |
| --- | --- |
| `EndTurn` | `end_turn` |
| `MaxTokens` | `max_tokens` |
| `MaxTurnRequests` | `max_turn_requests` |
| `Refusal` | `refusal` |
| `Canceled` | `cancelled` |

## 6. Permission mapping

ACP tool approval uses `session/request_permission`, a client method the agent
calls. `rho-acp` provides an `ApprovalPolicy` (from `SPEC-03`) whose `approve`
method issues that request and awaits the client outcome.

Flow for one mutating tool call:
1. `rho-acp` sees `Stream(ToolCallEnd)` and sends a `tool_call` update with
   `status: "pending"`.
2. The agent loop calls `ApprovalPolicy::approve`. The `rho-acp` policy sends
   `session/request_permission` with the `ToolCallUpdate` and four options:
   - `{ optionId, name, kind: "allow_once" }`
   - `{ optionId, name, kind: "allow_always" }`
   - `{ optionId, name, kind: "reject_once" }`
   - `{ optionId, name, kind: "reject_always" }`
3. The client replies with a `RequestPermissionOutcome`.
   - `selected` with an `allow_*` option maps to `ApprovalDecision::Allow`.
   - `selected` with a `reject_*` option maps to `ApprovalDecision::Deny`.
   - `cancelled` maps to `Deny`, and the run stops with `cancelled`.
4. `allow_always` and `reject_always` are remembered by the `rho-acp` policy for
   the session, so later calls to the same tool skip the prompt.

A denied call becomes an error `ToolResult`, per `SPEC-01` and `SPEC-03`. The
model sees the denial.

## 7. Capabilities

`rho-acp` advertises in the `initialize` response:
- `protocolVersion: 1`.
- `agentInfo`: name `rho`, the crate version.
- `promptCapabilities`: `image: true`, `audio: false`, `embeddedContext: false`
  for sprint 1.
- `mcpCapabilities`: none in sprint 1.
- `loadSession: false` in sprint 1.

`rho-acp` reads the client capabilities. It uses `fs/read_text_file` and
`fs/write_text_file` when the client advertises them; otherwise the built-in file
tools read and write directly under the session root.

## 8. Gaps for sprint 1

These ACP concepts have no full carrier yet. `rho-acp` handles each as noted.
`SPEC-01` section 10 lists the same gaps from the core side.

- `plan` updates: no plan or todo feature in sprint 1 (F-30 is planned).
  `rho-acp` sends no `plan` update.
- `tool_call.title`: synthesised from the tool name and arguments, for example
  `read src/main.rs`.
- `tool_call.locations`: not tracked. `rho-acp` omits it. Follow-along is planned.
- Structured `diff` content for `edit`: `rho-acp` sends the edit result as a text
  `content` item. A structured `diff` item is planned.
- `terminal` tool-call content: not used. `bash` output streams as text `content`.
- `UsageUpdate`: `rho-acp` fills `used` from the token counts in `Usage` and
  `size` from the model context window in config. `cost` is omitted in sprint 1.
- `session/set_mode`, `session/list`, `session/resume`, `session/delete`,
  `session/close`, and `session/load`: planned. See F-94.
- The extension UI sub-protocol beyond permission (F-95): planned.

## 9. Test cases

In `crates/rho-acp/tests/`, driving the agent over an in-memory JSON-RPC pipe with
a fake `rho-core` provider. No test reaches the network.

- `acp_initialize_reports_protocol_version_one` — the `initialize` response has
  `protocolVersion: 1`.
- `acp_prompt_streams_agent_message_chunks` — `TextDelta` events become
  `agent_message_chunk` updates in order.
- `acp_thinking_maps_to_agent_thought_chunk` — a `ThinkingDelta` becomes an
  `agent_thought_chunk`.
- `acp_tool_call_reports_pending_then_in_progress_then_completed` — one tool call
  yields a `tool_call` pending, a `tool_call_update` in_progress, then completed.
- `acp_tool_kind_is_forwarded` — a `bash` call reports `kind: "execute"`.
- `acp_tool_error_maps_to_failed_status` — a failing tool yields a
  `tool_call_update` with `status: "failed"`.
- `acp_stop_reason_end_turn_maps_to_end_turn` — an `EndTurn` run replies with
  `stopReason: "end_turn"`.
- `acp_stop_reason_cancelled_after_cancel` — a `session/cancel` yields
  `stopReason: "cancelled"`.
- `acp_turn_cap_maps_to_max_turn_requests` — hitting the loop cap replies with
  `stopReason: "max_turn_requests"`.
- `acp_permission_allow_once_runs_tool` — an `allow_once` outcome runs the tool.
- `acp_permission_reject_once_denies_tool` — a `reject_once` outcome yields an
  error tool result and the tool does not run.
- `acp_permission_cancelled_stops_turn` — a `cancelled` permission outcome stops
  the run with `stopReason: "cancelled"`.

## 10. Out of scope for sprint 1

- Session persistence methods: load, list, resume, delete, close (F-94).
- Session modes and `session/set_mode`.
- MCP server capabilities and terminal embedding.
- The extension UI sub-protocol beyond permission (F-95).
- Full steer and follow-up queue semantics (F-92 is partial in sprint 1).
- `authenticate` and `logout` flows.
