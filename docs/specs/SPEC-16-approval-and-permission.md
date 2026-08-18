# SPEC-16 — Approval and permission

Status: draft for sprint 2.
Owning crates: `rho-core` for the policy, `rho-cli` and `rho-acp` and `rho-tui` for the
mode resolution and the frontends.
Features: F-32, F-33, F-34, F-130.

## 1. The decision

The owner chose the approval model. This spec quotes it, then specifies it.

> rho gains an interactive Ask policy. It defaults to Ask wherever a human or a client can
> answer. It defaults to read-only where nobody can answer.

So Ask is the new default where an answer is possible. Read-only is the default where no
answer is possible. Allow-all is never a default. A user reaches allow-all only with an
explicit value.

### What exists today, and what does not

The tree ships three policies, and none is interactive.

- `crates/rho-core/src/tool.rs` ships `ApprovalPolicy`, `ApprovalDecision`,
  `ReadOnlyPolicy`, and `AllowAllPolicy`. There is no interactive policy.
- `crates/rho-cli/src/cli.rs` picks `AllowAllPolicy` unless `--read-only` is passed.
- `crates/rho-tui/src/` holds no approval code at all.
- `docs/specs/SPEC-06-acp.md` covers the ACP permission path. ACP has
  `session/request_permission`.
- Sprint 2 ships subagents with `BothPolicies`, in `docs/specs/SPEC-11-subagents.md`. A
  child may only be more restrictive. This spec keeps that rule.

The Ask policy and the mode resolution are new work. This spec names the public surface,
so a later stage builds it against a stated contract.

## 2. The Ask policy public API

The Ask policy implements the existing `ApprovalPolicy` trait. It does not change the
trait, so every existing caller keeps working. It asks a frontend and awaits an answer.

```rust
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

use crate::{ApprovalDecision, ApprovalPolicy, ToolKind};

/// The default time a frontend has to answer one request.
pub const DEFAULT_APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);

/// One approval question sent to a frontend.
///
/// The frontend answers on `respond`. A dropped `respond` sender is a denial.
pub struct ApprovalRequest {
    /// The tool name the model wants to call.
    pub tool: String,
    /// The typed category of the tool.
    pub kind: ToolKind,
    /// The tool arguments, redacted for display.
    pub arguments: serde_json::Value,
    /// The frontend sends its answer on this channel.
    pub respond: oneshot::Sender<ApprovalAnswer>,
}

/// A frontend's answer to one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalAnswer {
    /// Allow this one call.
    AllowOnce,
    /// Allow this call, and remember the choice for its scope. See section 5.
    AllowAlways,
    /// Deny this one call.
    RejectOnce,
    /// Deny this call, and remember the choice for its scope.
    RejectAlways,
}

/// Asks a frontend before a mutating tool call, and awaits the answer.
///
/// It holds the sending half of a channel to the frontend, and an answer timeout.
/// It implements the existing `ApprovalPolicy` trait, so it changes no caller.
pub struct AskPolicy { /* private */ }

impl AskPolicy {
    /// Build a policy over a channel to the frontend, with an answer timeout.
    pub fn new(to_frontend: mpsc::Sender<ApprovalRequest>, timeout: Duration) -> Self;
}

#[async_trait]
impl ApprovalPolicy for AskPolicy {
    /// A read-only kind is allowed without a question. A mutating kind sends an
    /// `ApprovalRequest` and awaits the answer. See section 3 for the fail-closed
    /// rules.
    async fn approve(
        &self,
        tool: &str,
        kind: ToolKind,
        args: &serde_json::Value,
    ) -> ApprovalDecision;
}
```

The channel type to the frontend is `tokio::sync::mpsc::Sender<ApprovalRequest>`. The
answer channel is a `tokio::sync::oneshot` inside each request. The request type is
`ApprovalRequest`. The answer type is `ApprovalAnswer`. The timeout is a `Duration`, and
`DEFAULT_APPROVAL_TIMEOUT` is five minutes.

A read-only kind never reaches the frontend. `AskPolicy::approve` allows a read-only kind
at once, so a read never blocks on a human. Only a mutating kind, which includes the
undeclared `Other` kind, asks. This matches the fail-closed rule in `ReadOnlyPolicy`.

## 3. The fail-closed rules

Each rule is a denial, and each stands alone.

- A timeout is a denial. The frontend did not answer inside the timeout, so the call is
  denied.
- A closed channel is a denial. The `mpsc` send failed, so the call is denied.
- A dropped answer sender is a denial. The frontend took the request and never answered,
  so the call is denied.
- A client that does not declare the permission capability cannot be asked. It gets
  read-only, not allow-all.

So every path that cannot produce an explicit allow ends in a denial. A denial is never
an error that ends the run. See section 8.

## 4. The mode resolution table

The frontend and the runtime decide the resolved policy per case. The table states the
resolved policy for each case.

| Case | Resolved policy |
| --- | --- |
| Interactive TUI | Ask |
| `rho run` with a terminal | Ask |
| `rho run` with no terminal | read-only |
| `rho acp`, client declares the capability | Ask |
| `rho acp`, client does not declare it | read-only |
| Subagent child | the parent's policy, composed |

So five cases resolve, and one composes. A terminal or a capable client resolves to Ask.
No terminal, or a client with no capability, resolves to read-only. No case resolves to
allow-all. A subagent child composes the parent's policy with `BothPolicies`, per section
9.

A user reaches allow-all only with an explicit config value or an explicit flag. See
section 5.

## 5. The config keys, and how they interact with SPEC-13

`SPEC-13` owns the `approval` key. The value set is `read-only`, `ask`, and `allow-all`.
`ApprovalMode::from_str` parses the value and fails closed on an unknown name.

- A config value may narrow the resolved default. A resolved `ask` narrows to `read-only`
  with a config `approval` of `read-only`.
- A config value widens the resolved default only when the user states it. A resolved
  `read-only` widens to `allow-all` only with an explicit `approval` of `allow-all`.
- `Config::defaults` leaves `approval` unset, so the resolution table sets the default.
  See `SPEC-13` section 4.

### The remembered answer

An `AllowAlways` answer applies to more than one call. It is scoped to the tool name for
the current session. So a later call of the same tool skips the prompt. A `RejectAlways`
answer works the same way for a denial.

An unbounded memory of approvals is a fail-open risk. So the memory has a bound.

- The scope is the tool name, not the arguments. So the memory holds at most one entry
  per tool.
- The memory lives for the session only. It never reaches disk.
- A new session starts with an empty memory. A remembered allow never crosses a session.

So the memory cannot grow without a bound, and a remembered allow cannot outlive the
session that made it.

### An execute tool cannot be remembered

A remembered allow is scoped to a tool name. For `bash` that scope is far too wide,
because one `bash` allow covers every later command in the session. A `git log` answer
would then approve an `rm -rf`. So the memory refuses a kind that runs a program.

- `AskPolicy` rejects an `AllowAlways` answer for `ToolKind::Execute`. It treats the
  answer as `AllowOnce` instead, and it says so in the event stream.
- The same rule holds for any later kind that runs a program.
- A `RejectAlways` answer is remembered for every kind, because a remembered denial
  cannot fail open.

So a user who wants every command approved must set `approval = allow-all` on purpose.
A prompt cannot become blanket command approval by one careless click. This is the
`ToolKind::Other` lesson from decision D-017, applied to the memory.

Tests: `allow_always_for_an_execute_kind_degrades_to_allow_once`, and
`reject_always_is_remembered_for_every_kind`.

## 6. The ACP mapping

`rho-acp` maps the Ask to `session/request_permission`, the client method the agent
calls. The field names come from `~/Work/Vibe/acp-docs/schema/v1/schema.json`.

- The request carries `sessionId`, `toolCall`, and `options`. The `toolCall` is a
  `ToolCallUpdate`. The `options` is an array of `PermissionOption`.
- Each `PermissionOption` carries `optionId`, `name`, and `kind`. The `kind` is one of
  `allow_once`, `allow_always`, `reject_once`, and `reject_always`.
- The client replies with a `RequestPermissionResponse`. Its `outcome` is a
  `RequestPermissionOutcome`.

rho maps each ACP outcome to an action.

- A `selected` outcome with an `allow_once` option maps to `ApprovalDecision::Allow`.
- A `selected` outcome with an `allow_always` option maps to `Allow`, and rho remembers
  it for the tool. See section 5.
- A `selected` outcome with a `reject_once` or a `reject_always` option maps to
  `ApprovalDecision::Deny`.
- A `cancelled` outcome maps to `Deny`, and the run stops with the stop reason
  `cancelled`. `SPEC-06` section 5 owns the spelling.

A client that does not declare the permission capability is never asked. It resolves to
read-only, per section 4.

## 7. The TUI behaviour

The TUI prompt must not block the input editor, and it must not block the event stream.
The frontend receives an `ApprovalRequest` on the channel, and renders a prompt. The
agent loop awaits the answer on the oneshot channel, not on the UI thread.

- The event stream keeps flowing while the prompt is open. A streamed token still
  renders.
- The input editor stays usable. The user can still type or scroll.
- Ctrl-C during a prompt is a denial. It denies the call, and it does not exit rho.

So a prompt is a non-blocking overlay, not a modal stop. A denial from Ctrl-C returns to
the model as a tool error, per section 8.

## 8. The interaction with the session log

An approval decision is not its own session record in sprint 2. The result of the
decision is recorded through the normal path. An allowed call records its `ToolCall` and
`ToolResult`. A denied call records a `ToolResult` that is an error.

A denial reaches the model as a tool error. It does not end the run. The model reads the
error and chooses again. Defect 9 in `.rho-work/progress.md` was a tool error that killed
a whole run, and this rule prevents that shape.

## 9. The interaction with BothPolicies from SPEC-11

A child stays at most as permissive as its parent. `SPEC-11` section 3 enforces this by
composition, not by comparison. `BothPolicies` allows a call only when the parent and the
child both allow it.

- An Ask parent stays an Ask parent for the child. The child cannot become an allow-all
  child, because `BothPolicies` still consults the parent, and the parent still asks.
- A read-only parent produces a read-only child, whatever the child asks for.
- A child may narrow further, never widen.

So an Ask parent never yields an allow-all child. The composition makes the escalation
unrepresentable, which is the point of `SPEC-11` section 3.

## 10. Test cases

Resolution, one per row:
- `interactive_tui_resolves_to_ask` — an interactive TUI resolves to Ask.
- `run_with_a_terminal_resolves_to_ask` — `rho run` with a terminal resolves to Ask.
- `run_with_no_terminal_resolves_to_read_only` — `rho run` with no terminal resolves to
  read-only.
- `acp_with_the_capability_resolves_to_ask` — a client that declares the permission
  capability resolves to Ask.
- `acp_without_the_capability_resolves_to_read_only` — a client that does not declare it
  resolves to read-only, not allow-all.
- `a_subagent_child_composes_the_parent_policy` — a child resolves to the parent's
  policy, composed with `BothPolicies`.

Fail-closed, one per rule:
- `a_timeout_is_a_denial` — no answer inside the timeout denies the call.
- `a_closed_channel_is_a_denial` — a send on a closed channel denies the call.
- `a_dropped_answer_sender_is_a_denial` — a taken request with no answer denies the call.
- `a_client_without_the_capability_gets_read_only` — a client with no capability resolves
  to read-only.

Remembered answer:
- `allow_always_skips_a_later_prompt_for_the_same_tool` — a remembered allow applies to a
  later call of the same tool.
- `the_remembered_set_is_bounded_to_one_entry_per_tool` — the memory holds at most one
  entry per tool name, not per argument.
- `a_remembered_allow_does_not_cross_a_session` — a new session starts with an empty
  memory.
- `allow_always_for_an_execute_kind_degrades_to_allow_once` — an `AllowAlways` answer for
  `ToolKind::Execute` approves one call only.
- `reject_always_is_remembered_for_every_kind` — a remembered denial holds for every kind,
  including `ToolKind::Execute`.

ACP mapping:
- `acp_allow_once_maps_to_allow` — a `selected` `allow_once` outcome allows the call.
- `acp_reject_once_maps_to_deny` — a `selected` `reject_once` outcome denies the call.
- `acp_cancelled_denies_and_stops_with_cancelled` — a `cancelled` outcome denies the call
  and stops with `cancelled`.

TUI:
- `a_tui_prompt_does_not_block_the_event_stream` — a streamed event renders while a
  prompt is open.
- `a_tui_prompt_does_not_block_the_input_editor` — the input editor stays usable during a
  prompt.
- `ctrl_c_during_a_prompt_denies_and_does_not_exit` — Ctrl-C denies the call and keeps rho
  running.

Subagent composition:
- `an_ask_parent_never_yields_an_allow_all_child` — a child of an Ask parent cannot allow
  a call the parent would ask about without a parent allow.
- `a_read_only_parent_yields_a_read_only_child` — a child of a read-only parent cannot
  write.

The Ask policy:
- `ask_policy_allows_a_read_only_kind_without_asking` — a read-only kind is allowed with
  no request.
- `ask_policy_asks_for_a_mutating_kind` — a mutating kind sends one `ApprovalRequest`.

Every test uses a scripted fake frontend. No test uses the network. No test uses `sleep`.
A test synchronises with a channel or `tokio::time`.

## 11. Out of scope for sprint 2

- A remembered answer that survives a restart. The memory is session-scoped.
- A per-argument or per-path approval scope. The scope is the tool name.
- A policy rule language, or a remote approval service. F-130 owns those.
- A per-tool config override that grants a kind. That is a later user-trust feature.
- An approval decision as its own session record. The tool result carries the outcome.

## 12. Required change to docs/features.md

The controller owns `docs/features.md`. This section states the change, so the controller
can edit it.

Row F-29 once claimed that the basic TUI wires a confirmation prompt to the approval gate.
`crates/rho-tui/src/` holds no approval code, so that claim was false. The current row no
longer carries the claim, and it must stay that way. F-29 should describe only the trait
and the two shipped policies, read-only and allow-all, and it should stay `sprint-1`.

The Ask policy needs these rows, and they already exist and reference this spec.

- F-32, the Ask approval policy, in `rho-core`. It stays `planned` until the policy is
  built.
- F-33, the approval mode resolution, in `rho-core` and `rho-cli`. It stays `planned`.
- F-34, permission over ACP, in `rho-acp`. It stays `planned`.
- F-130, the approval UI extensions, in `rho-tui`. It stays `planned`.

A new row is needed for the TUI Ask prompt itself, because F-130 covers only the richer
extensions. The controller should add a row for the non-blocking TUI prompt, in
`rho-tui`, marked `planned`, and reference this spec.
