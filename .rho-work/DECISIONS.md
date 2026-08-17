# Controller decisions

Decisions the controller made during the sprint. These are binding. They answer
questions that subagents raised. Do not re-open them.

## D-001 — Session file format is rho's own, not pi's

**Question (S1 documenter):** should the session log be compatible with pi's
session format, so `makit` can switch tools without a session migration?

**Decision:** No. rho defines its own append-only JSONL session format, with a
version field in the first record.

**Reason:** pi's format encodes pi's own message and content model. If rho
adopts it, pi's decisions leak into `rho-core`, and the core stops being free to
normalise providers its own way. That breaks the reason the project exists.

**Mitigation:** a one-way converter is a `planned` feature in a separate crate,
`rho-session-import-pi`. It is out of scope for sprint 1. Record it in
`docs/features.md`.

## D-002 — The headless frontend is ACP, and it is the real ACP

**Question (S1 documenter):** should public docs say "ACP" or "RPC"?

**Decision:** Say **ACP**, and mean the real Agent Client Protocol. `rho-acp`
implements the same protocol that `pi-acp` and Zed speak. Do not invent a
private RPC dialect for sprint 1.

**Reason:** `makit` already drives `pi` over ACP. If `rho-acp` speaks real ACP,
`makit` can use rho as a drop-in session backend with no change in `makit`. That
is the shortest path to the owner's actual goal, which is 50 cheap concurrent
sessions.

**Reference:** the ACP protocol documents and JSON schema are already on disk at
`~/Work/Vibe/acp-docs/`. Use `~/Work/Vibe/acp-docs/schema/` as the source of
truth. Do not guess a method name.

**Scope:** `rho-acp` is `planned` for delivery after the TUI in sprint 1. Its
spec is still written in S2, because the spec constrains `rho-core`'s event
model.

## D-003 — `docs/benchmarks.md` is created by S11 and nothing may pre-empt it

Any document that cites a rho performance number must write `to be measured` and
link to `docs/benchmarks.md`. The website must not publish a rho number until
S11 fills that file with a real measurement and the command that produced it.

## D-004 — Website direction is mockup C, with A's command-prompt labels

**Question (S10a designer):** which of the three mockups wins? The designer
picked A, the all-monospace terminal-native one.

**Decision:** Mockup **C** (`mockup-c-parts.html`) wins. Take two things from A.

**Reason:** C reads better and sells the idea faster.

- C's crate picklist, with `[x]` and `[ ]` boxes, states "you pick the parts" in
  one glance. That is the single most important message on the page. A only
  lists the crates.
- C mixes a proportional face for headings with mono for code. That gives a real
  type hierarchy. A sets body text in mono at full width, which tires the reader.
- A's `$ cat crates.txt` framing is clever, but it is a costume. The content is
  not a terminal session, so the frame fights the content.

**Take from A:** the `$ <command>` form for the small eyebrow label above each
section. It is terminal-native without dressing the whole page as a shell.

**Must fix in C:** the `[x]` and `[ ]` column is decoration to a screen reader.
Give it a real accessible label, or make it a real form control, or mark it
`aria-hidden` and put the meaning in text.

## D-005 — The Hook trait and the approval gate are sprint-1 interfaces

**Question (S2 architect):** `docs/features.md` marks F-40 (Hook) and F-29
(approval gate) as `planned`. But the S3 definition of done tests hook order,
and `SPEC-06-acp.md` needs an async approval path for the ACP
`session/request_permission` request.

**Decision:** The **traits** ship in sprint 1. Flip F-29 and F-40 to `sprint-1`.
The rich behaviour stays `planned`.

- Sprint 1 delivers: the `Hook` trait, its ordering guarantee, and one or two
  real hook points. Plus the approval callback type and its wiring in the tool
  dispatch path.
- Sprint 1 does not deliver: a full hook point at every lifecycle stage, a
  policy language, or an approval user interface beyond a TUI prompt.

**Reason:** an interface added later changes every caller. An interface added now
costs almost nothing. `rho-acp` cannot conform to ACP without an approval path,
so the type must exist in the core from the start.

## D-006 — The architect may widen the core event model to fit ACP

The S2 architect added `ToolKind`, `Tool::kind()`, `ToolSpec.kind`, and
`AgentStopReason` on `AgentEnd`, mapped one to one onto the ACP `StopReason`.
This is approved. The remaining ACP concepts, which are plan, locations,
structured diff, and cost, stay on the `rho-acp` side and do not enter
`rho-core`.

## D-007 — The ACP cancelled stop reason needs an explicit serde rename

The S2 reviewer found a real interoperability defect. The controller confirmed it
against `~/Work/Vibe/acp-docs/schema/v1/schema.json`, where `StopReason` is
`["end_turn","max_tokens","max_turn_requests","refusal","cancelled"]`.

ACP spells the value `cancelled`, with two letters `l`. The Rust variant is
`AgentStopReason::Canceled`, with one `l`. So `serde(rename_all = "snake_case")`
alone emits `canceled`, which no ACP client accepts.

**Decision:** keep the Rust variant name `Canceled`, and carry
`#[serde(rename = "cancelled")]` on it. A named test guards the attribute:
`agent_stop_reason_canceled_serialises_as_cancelled`.

**Reason:** the Rust name follows Rust convention. The wire name follows the
protocol. An attribute plus a test is cheaper than a name that reads wrong in
either place.

## D-008 — `Session` gets a read-only context accessor

**Question (S3 tester):** `SPEC-01` gives `Session` no way to read its `Context`.
So `agent_loop_appends_assistant_and_tool_messages` cannot assert on the context.
The tester asserted the equivalent ordering through the event stream instead.

**Decision:** Add a read-only accessor to `Session`. Name it `messages`. It
returns a borrowed slice, or a cheap snapshot if a lock forces that. Add it to
`SPEC-01`.

**Reason:** the append-only rule is a core invariant. An invariant that a test
cannot observe is an invariant that will break in silence. A read-only accessor
does not weaken encapsulation, because it grants no mutation. Keep the
event-stream assertion as well. Two views of one invariant are better than one.

**Constraint:** read only. No public API may mutate the context from outside a
turn.

## D-009 — Fix the `CancelToken::cancelled` wake race

**Finding (S3 tester):** `cancelled()` checks the flag, and only then awaits
`notify.notified()`. `Notify::notify_waiters` stores no permit. So on a
multi-thread runtime a `cancel()` between the flag check and the registration is
lost, and the waiter hangs. The tester's test passes only because it runs on the
current-thread runtime, where the interleaving cannot happen.

This is a confirmed bug. The rules forbid leaving one unfixed.

**Decision:** S4 fixes it. Create the `Notified` future first. Then check the
flag. Then await. The `Notified` future registers on creation, so no wake is
lost.

```rust
pub async fn cancelled(&self) {
    let notified = self.inner.notify.notified();
    if self.is_cancelled() {
        return;
    }
    notified.await;
}
```

**Test:** add `cancel_token_cancelled_wakes_on_multi_thread_runtime`, marked
`#[tokio::test(flavor = "multi_thread", worker_threads = 2)]`, with a bounded
timeout so a lost wake fails the test instead of hanging the suite. S4 may add
this test, because it guards a bug the spec did not describe.

## D-010 — The provider contract suite is a real crate, not a private test file

`workflow.yaml` put the shared provider contract suite in
`crates/rho-core/tests/provider_contract.rs`. The controller changed this.

**Decision:** the suite lives in a new crate, `rho-provider-testkit`. It exports
reusable functions that assert the contract against any `Provider`
implementation. Each provider crate calls those functions from its own tests.

**Reason, and it is a product reason, not a convenience.** rho's thesis is that a
third party writes a provider without forking rho. A third party cannot run a
test file that is private to `rho-core`. A testkit crate lets any author prove
conformance with the same assertions we use. The extension point stops being a
claim and becomes a tool.

**Second reason:** it removes a file collision. Stage S4 must prove it did not
touch `crates/rho-core/tests/`, and a new file there would break that proof.

**Constraint:** `rho-provider-testkit` is a normal crate, not a dev-dependency
hack. It depends only on `rho-core`, `tokio`, `serde`, `serde_json`, `futures`,
and `async-trait`. It never depends on a concrete provider.
