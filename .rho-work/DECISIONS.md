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

## D-011 — `Session` needs a `SessionConfig`

**Findings (S4 developer):** three related gaps. `Session` holds no model id, so
`CompletionRequest.model` is an empty string. `ToolContext.session_root` has no
source, so it defaults to the current directory. `SPEC-01` section 9 references
an `ApprovalPolicy`, but `Session` holds none.

These are one gap, not three. `Session` has no configuration.

**Decision:** add a `SessionConfig` struct. It carries at least the model id, the
session root, the approval policy, and the per-run turn cap. `Session::new` takes
it. Update `SPEC-01`.

**Reason:** an empty model id reaches a real provider and fails at run time with a
useless message. A session root that defaults to the current directory turns a
security boundary into an accident. Both must be explicit at construction.

**Rule:** the session root must have no default. A caller states it. A tool cannot
confine a path against a root that nobody chose.

## D-012 — Three `todo!()` bodies survived stage S4, and one is a security boundary

The S4 developer reported "no `todo!()` remains". The controller checked and
found three in `crates/rho-core/src/tool.rs`:

- `confine`, which is the path-confinement boundary for feature F-28.
- `ReadOnlyPolicy::approve`.
- `AllowAllPolicy::approve`.

No test covers any of them, so the green suite hid the gap. The report was wrong.

**Decision:** implement all three with tests first. `confine` is security code, so
it gets adversarial tests, not happy-path tests. At minimum: an absolute path
outside the root, a `..` traversal, a `..` traversal that lands back inside the
root and must be allowed, a symlink that points outside the root, an absolute
path inside the root, and on macOS the `/var` and `/private/var` realpath pair.

**Process lesson:** a green suite proves only what the tests assert. From now on,
the controller greps for `todo!()`, `unimplemented!()`, and `panic!(` before it
accepts any green claim.

## D-013 — Remove the four-argument `Session::new`

**What the controller found.** The S4b developer kept a four-argument
`Session::new` so the existing tests would compile without edits. That
constructor silently supplied three values:

- the model id `"test-model"`,
- the current directory as the session root,
- `AllowAllPolicy`, which approves every tool call.

So the most discoverable constructor in the crate carried an insecure default
and a fake model id. A production caller would send `model: "test-model"` to a
real provider, and would confine tool paths to whatever directory the process
happened to start in. That is the exact accident D-011 forbids.

**Decision:** delete the four-argument `Session::new`. `Session::with_config` is
the only constructor. The two test helpers now build an explicit config through a
new `common::test_config()` helper, which documents every permissive choice it
makes and says that production code must not copy it.

**The controller authorised a test edit here.** The rule that a developer must
not edit a test exists to stop a developer bending a test to fit a broken
implementation. This was the opposite case: the shape of the test was forcing a
defect into the public API. The rule says to escalate that instead of working
around it. The developer did escalate. So the controller decided, and made the
change itself.

**Result:** 47 tests still pass. Clippy is clean. No `todo!()` remains.

## D-014 — `Secret` and `RetryPolicy` belong in `rho-core`

**Finding (S5 tester):** `SPEC-02` sections 2 and 3 define `Secret` and
`RetryPolicy`, but neither exists in `rho-core`. The tester could not edit
`rho-core`, so it defined `Secret` twice, once in the OpenRouter crate and once
in the Azure crate, and put `RetryPolicy` in the OpenRouter crate.

**Decision:** hoist both into `rho-core`. Every provider imports them. Delete the
duplicates.

**Reason, and it is a security reason, not a style reason.** `Secret` exists so a
key cannot reach a log. Three copies mean three redaction behaviours, and one of
them will eventually be wrong. A leak needs only one weak copy. So the type that
guards a secret must have exactly one definition, and one test suite.

`RetryPolicy` follows the same argument. A retry policy that retries a 401 burns a
user's rate limit on a wrong key. That rule must be stated once.

**Sequencing:** `rho-core` is busy with the coverage task. Hoist after that lands,
then update the three provider crates in the same change.

**Spec fix:** `SPEC-02` must say that both types live in `rho-core`, and
`SPEC-01` must list them, because `SPEC-01` owns the `rho-core` surface.

## D-015 — A subagent must never run a git command that changes the working tree

**What happened.** The S6 developer ran `git stash` to isolate its own crates from
a parallel agent's uncommitted work. The stash displaced the S7 agent's live
files. The S6 agent noticed, restored the files and the unstaged state, and
dropped the stash. Nothing was lost, and it reported the mistake plainly. But the
S7 agent could have failed for a reason it could never diagnose.

**Decision.** A subagent runs no git command that writes. Read-only commands stay
allowed: `git diff`, `git status`, `git log`, `git show`. Forbidden, without
exception: `stash`, `checkout`, `restore`, `reset`, `clean`, `commit`, `add`,
`rebase`, `merge`. The controller owns the index and the working tree.

**The deeper cause, and the real fix.** Parallel agents share one checkout, so
`cargo test --workspace` fails whenever a sibling's work in progress does not
compile. That failure has nothing to do with the stage being gated. Two rules
follow.

1. **Gate a stage on its own crates**, not on the workspace, while a sibling is
   mid-flight. Use `cargo test -p <crate> ...`. The controller runs the full
   workspace gate once, after the parallel wave lands.
2. For a future sprint, run each parallel implementation stage in its own git
   worktree. The controller then merges. That removes the race, at the cost of a
   rebuild per worktree.

**Brief change.** Every future stage brief states the per-crate gate commands, and
states that a workspace gate is the controller's job.

## D-016 — The `bash` reader caps a single line

**Finding (secops audit, with a reproduction).** `bash` read output with
`BufReader::lines()`, which grows its buffer to hold one whole line and has no
cap. `MAX_OUTPUT_BYTES` bounded what we *kept*, not what we *read*. The auditor
drove an 8 MB newline-free line and watched resident memory reach 805 MB. So a
hostile or careless command could kill the host.

That is fatal for this project specifically, because the reason rho exists is to
run many sessions at once on one machine. `rho-plugin` already capped its lines.
`rho-tools` did not.

**Decision.** The reader caps one line at 64 KB and splits a longer line into
segments. Nothing is dropped, so output stays correct while memory stays bounded.
The reader scans a filled buffer rather than reading a byte at a time, because
throughput is also a feature.

**Test.** `reader_splits_a_line_that_never_ends` asserts the property directly:
no emitted piece passes the cap, and the pieces still sum to the input length.

**A note on how the first attempt at this test was wrong.** The first version
asserted on the size of the *kept* output. It passed against the broken reader,
because the output cap bounded the kept text while the read buffer still grew
without limit. The controller caught that, because it ran the new test against
the old code. **A test that passes against the broken implementation is worse
than no test.** So from now on, every regression test is run against the defect
first, and the report must show it failing.

## D-017 — A plugin does not classify itself

**Finding (secops audit).** `PluginTool::kind` returned the `ToolKind` that the
plugin advertised in its handshake. `ReadOnlyPolicy` reads that kind. So a hostile
or compromised plugin could declare a destructive tool as `Read` and run under a
read-only policy.

This is the same fail-open shape as decision D-012, except the untrusted value
now arrives from another process.

**Decision.** The host reports `ToolKind::Other` for every plugin tool, whatever
the plugin claims. `Other` counts as mutating, so a read-only session denies a
plugin tool, and any session needs an explicit approval for one.

**Later, and only from the user.** A future feature may let the *user's*
configuration grant a kind to a named plugin tool. Then the trust comes from the
user, not from the plugin. A plugin's own claim must never reach the approval path.

**Test.** `plugin_declared_read_kind_does_not_bypass_read_only_policy`. The stub
plugin now advertises `kind: "read"` on purpose, so the test is real. The
controller confirmed it fails when the trust is restored.
