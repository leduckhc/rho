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

## D-018 — The provider extension point is verified from outside the workspace

`README.md` and decision D-010 both claim that a third party can implement
`rho_core::Provider` and prove it conforms, without forking rho. That claim was
never tested. An untested claim about an extension point is a slogan.

**What the controller did.** Built a scratch crate at `/tmp/outsider`, outside the
rho workspace, depending on `rho-core` and, as a dev dependency,
`rho-provider-testkit`. Implemented a toy `Provider`, wrote one `ProviderHarness`
bridge, and called `run_all(&ToyHarness).await`.

**Result: the claim holds.** One call runs every conformance check. The suite then
correctly **rejected** a deliberately broken provider that omitted its first event:

```
the first event must be MessageStart, but it was TextStart { index: 0 }
```

So the suite discriminates, and its message names the violation.

**One real finding, about discoverability rather than design.** Writing the outside
crate needed three guesses at the API, and all three were wrong. The trait method is
`id`, not `name`. The usage event is `StreamEvent::Usage(Usage)`, not a struct
variant. The `HarnessRun.guard` field needs a `Box<dyn Any + Send>` even when a
provider has nothing to guard.

An outsider hits every one of those. So the crate now carries a `README.md` with a
working bridge, a table saying what each `Script` must produce, and the list of
checks. The types were always right. Only the front door was missing.

## D-019 — `bash` scrubs credential variables, and the spec stops overclaiming

A security audit listed three items as "documented but confirmed, minor". The
controller re-read them and disagreed on one, then demonstrated it live.

**The demonstration.** A real model, asked through `rho run`, printed the child's
environment. It saw `OPENROUTER_API_KEY` and `AWS_SECRET_ACCESS_KEY`. A single
network call would exfiltrate them. The model's tool arguments are
attacker-controlled input, because a prompt injection lives in any file the agent
reads. So this is not minor.

**Decision.** `bash` removes every variable whose **name** looks like a credential
before it starts the child. The filter reads the name, not the value, because a
value cannot be recognised reliably. It removes the name as well, so the presence of
a key leaks nothing.

The filter is a denylist, and that needs a reason. An allowlist is safer in
principle, but a command legitimately needs a wide and open-ended set of variables.
An allowlist would break ordinary work, and users would switch it off. A denylist
that catches the recognisable shapes is the useful trade. A false positive costs one
variable. A false negative costs a key.

**The more important half of this decision is the wording.** `SPEC-03` section 7
said that "path confinement and the approval policy" bound `bash`. The first half
was false, and a false claim about a boundary is worse than no claim. Path
confinement does not apply to `bash`, because `cd` and an absolute path both leave
the session root.

So the spec now states the truth:

- The **approval policy is the only real boundary** for `bash`. It declares
  `ToolKind::Execute`, so a read-only policy denies it outright. `--read-only` is the
  way to point rho at a repository you do not trust.
- Credential scrubbing is **defence in depth, not a boundary**. A shell can still
  read `~/.aws/credentials` from disk. Anything that runs commands can read files the
  user can read.
- Sprint 1 adds no container and no namespace sandbox. That is the honest limit.

**Verified live after the fix.** The same injected probe now reports zero matching
variables, and `PATH` still works.

**The two remaining audit items stay open, and are recorded rather than hidden.**
`bash` has no path confinement, which is now documented as intended. And
`PluginHost::launch` does no path validation, so a user who configures a plugin from
a writable directory trusts that directory. Both need a design decision, not a patch.

## D-020 — The plugin host states a trust policy, and refuses a plugin in the session root

A security audit reported that `PluginHost::launch` did no path validation. The
controller had recorded it as open. This closes it.

**The risk is concrete.** If a plugin may live inside the session root, then a
repository hands executable code to the agent that reads it. A checked-in script
becomes a tool as soon as somebody points rho at that repository. Worse, the model can
write such a script itself, with `write` or `bash`, so a later launch runs code the
model authored.

**Decision.** `PluginHost::new` takes a `PluginPolicy`. The policy refuses:

- a path that does not resolve, is not a file, or is not executable;
- a plugin under `untrusted_root`, which a caller sets to the session root;
- a world-writable plugin, since another local user could replace the file first.

The check resolves the path before comparing, so `..` and a symlink cannot dodge the
root test.

**No `Default`, and no policy-free constructor.** A default would have to choose a
policy, and the only context-free choice is the permissive one. That is the shape
decision D-013 removed. `PluginPolicy::trust_any_path` exists for a caller that already
controls the path, and it is named so the call site admits what it does.

**Sequencing note.** The CLI does not wire plugins yet, so no production caller had to
change. The policy therefore lands before the caller exists, which is the right order.
`SPEC-04` section 5a records that the CLI must pass
`PluginPolicy::confined_to_outside(session_root)` when it does wire them.

**Verification.** Six tests cover the refusals and one covers ordinary use. The
controller disabled the policy check and confirmed all six fail, then restored it.

## D-021 — `bash` path confinement stays out, and the reason is written down

The same audit listed a second open item: `bash` has no path confinement, because `cd`
and an absolute path both leave the session root.

**Decision. This stays as it is, and it is now documented rather than open.**

Confining a shell command is not a path check. It needs a container, a namespace, or a
`chroot`, because a shell can reach any path the user can reach, and it can do so
through a hundred routes. A partial check would be worse than none, since it would read
as a boundary while a single `cd ..` walked through it.

So `SPEC-03` section 7 now states the truth plainly: the approval policy is the only
real boundary for `bash`, credential scrubbing is defence in depth, and sprint 1 ships
no sandbox. `--read-only` denies `bash` outright, and that is the supported way to point
rho at a repository you do not trust.

A real sandbox is a design task with its own spec, not a patch. It is recorded in
`docs/non-goals.md` scope, not left as a silent gap.

## D-022 — A project skill is not loaded until the project is trusted

A skill is instructions the model will follow, and it may carry scripts the model will
run. So a skill inside the repository under edit is a prompt injection with a filename.

This is the same threat as decision D-020, where the plugin host refuses a plugin under
the session root. A skill is worse in one way: nobody reads a Markdown file as code.

**Decision.** A project skill is withheld until the user trusts that session root. The
default is untrusted, and nothing infers trust. A withheld skill is still **listed**,
because a user who cannot see a skill cannot decide about it.

`--read-only` does not grant skill trust. The two are independent. A read-only session
still follows instructions, and instructions can exfiltrate through a read.

## D-023 — `allowed-tools` in a skill is parsed, warned about, and ignored

The Agent Skills frontmatter allows a skill to pre-approve tools for itself.

**Decision.** rho reads the field, warns that it has no effect, and ignores it.

**Reason.** A skill granting itself approval inverts the boundary that decisions D-012
and D-017 worked to make fail closed. The approval policy stays the single authority.
Honouring the field needs a design that asks the **user**, not one that believes the
skill.

## D-024 — An MCP server does not classify its own tools

MCP has no equivalent of `ToolKind`, and a server's own opinion would not be
trustworthy if it had one.

**Decision.** Every MCP tool reports `ToolKind::Other`, which `is_read_only` treats as
mutating. So a read-only session denies an MCP tool, and any session needs an explicit
approval for one.

This is decision D-017 applied to a second source. A plugin's self-declared kind was
refused for the same reason. A later feature may let the user's configuration grant a
kind to a named MCP tool. Trust then comes from the user.

## D-025 — An MCP server is shared between sessions by default

An MCP server is a whole process, often a Node or Python program costing tens of
megabytes.

rho's measured cost is about 25 KB per extra session. If each session spawned its own
copy of every configured server, fifty sessions with three servers each would spawn 150
processes, and the footprint argument that justifies this project would collapse.

**Decision.** A server is shared by default, keyed by a fingerprint of its config, and
reference counted. The last session to release it stops it. A server that must not be
shared sets `shared: false`.

**Consequence to watch.** A shared server means one session's misbehaviour can affect
another. So every limit in `SPEC-09` section 6 applies per call, not per session, and a
call timeout protects a session from a server that another session has wedged.

## D-026 — Redaction has one home, because the copies had already drifted

The S14 developer reported that four pieces of code now existed in two or more crates.
The controller checked, and two of them were security code that had **already diverged**.

**The credential denylist** existed in `rho-tools/src/bash.rs` and again in
`rho-mcp/src/transport.rs`. The two lists were identical, which is exactly what decision
D-014 saw for `Secret` shortly before those copies diverged.

**The terminal sanitiser existed three times, with three different behaviours.**

- `rho-tui` replaced each unsafe character. Safe, but it left visible rubbish:
  `red\x1b[31mtext` rendered as `red\u{fffd}[31mtext`.
- `rho-mcp` parsed and dropped a whole escape sequence, so the same input rendered as
  `redtext`.
- `rho-tools` used a regex over the message.

So the same hostile output rendered three ways, depending on which path carried it.

**Decision.** A new crate, `rho-redact`, holds `looks_like_a_secret`, `sanitize_text`,
and `sanitize_line`. Every other crate calls it. The behaviour kept is the best of the
three: an escape sequence is dropped whole, and any other control character is replaced,
so nothing invisible survives and nothing visible is left behind.

**Reason, quoting D-014 because the argument is the same.** A leak needs only one weak
copy. So a value that guards a secret gets one definition and one test suite. A filter
that guards a terminal is the same kind of thing.

**Verification.** The controller broke the escape filter in `rho-redact` and confirmed
that all four crates fail: `rho-redact`, `rho-tui`, `rho-mcp`, and `rho-tools`. Before
the consolidation, breaking one copy left the others silent.

**Two duplicates stay, on purpose.** The bounded line reader and the stdio JSON-RPC
engine appear in `rho-plugin` and `rho-mcp`. They are close but not the same: one speaks
rho's protocol and one speaks MCP, and the framing differs. Merging them now would
invent an abstraction to fit two callers. They are recorded here so a third caller
triggers the extraction instead of a third copy.

## D-027 — What reading jcode's `edit` tool changed

Recorded because `AGENTS.md` step 1 cites this as the reason to read prior art, and a
reviewer could not substantiate the claim. A war story with no record teaches nothing.

The owner asked for `replace_all` on the `edit` tool, and pointed at jcode's
`crates/jcode-app-core/src/tool/edit.rs`. Reading it produced four changes.

**Three features rho lacked.**

1. `replace_all`, plus an ambiguity error that names the escape hatch. Strictness alone
   was the trap: a rename touching three identical spans forced the model to add
   surrounding context three times, or to give up.
2. Near-miss diagnostics. A bare "not found" is a dead end, because the model cannot
   tell an absent span from one that differs in whitespace. A failed match now names the
   near miss it found, with a line number.
3. Familiar argument aliases. Models are trained on `file_path`, `old_string`, and
   `new_string`. rho keeps its own consistent names and accepts those too, because a
   schema error costs a whole turn.

**One bug in jcode, which rho now guards against.** jcode accepts an empty
`old_string`. An empty pattern matches at every character boundary, so with
`replace_all` the file is rewritten: `"abc"` becomes `"XaXbXcX"`. rho refuses the call.

`SPEC-03` section 6a records the outcome. The lesson for step 1 stands: reading the real
source found three gaps and one defect, and guessing would have found neither.

## D-028 — Another agent shares this working tree, so read the diff before you commit

**What happened.** A separate agent keeps this repository private before the first
release. It edits files without committing: it added the private rule to `AGENTS.md`, a
row to `docs/index.md`, and `docs/release-checklist.md`, and it flipped the GitHub
visibility to private through the API.

The controller did not notice. It committed those edits inside its own commits, under its
own messages. So `git log` says the controller wrote a rule it did not write. Nothing was
lost, and that was luck: the controller replaces whole sections with a script, and a
different ordering would have destroyed the other agent's work.

The controller also repeated "public, MIT" in several reports, hours after the repository
became private. It read its own memory instead of the current state.

**Decision.** Three rules.

1. **Read the diff before you commit.** `git status` and `git diff` first. A change you
   do not recognise belongs to somebody else. Commit it separately, and say whose it is,
   or leave it alone.
2. **Prefer a targeted edit over a section rewrite.** A script that replaces a whole
   section destroys a concurrent edit inside that section without a conflict. A small
   anchored replacement fails loudly instead.
3. **Re-read the state you are about to assert.** Visibility, a version, a test count,
   and a benchmark all change under you. Check, then claim.

**These files have another owner.** Treat them as shared: `AGENTS.md` product rules,
`docs/release-checklist.md`, `docs/index.md`, and `.github/workflows/`. Read them before
you write them.

**This is decision D-015 with the roles reversed.** There, a subagent ran `git stash` and
displaced a parallel agent's work. Here the controller was the one at risk of clobbering.
The lesson generalises: a shared working tree has no single owner, so no writer may assume
the tree is as it left it.

## D-029 — What reading jcode's `bash` tool changed

Four adoptions, one deliberate omission, and one defect that only a live run found.

**Adopted.** A disk-backed scratch directory through `TMPDIR` and `RHO_SCRATCH_DIR`,
because `/tmp` is a tmpfs on most Linux systems and a build there spends the memory this
project exists to save. A timeout message that names the millisecond unit. A byte-ratio
progress shape, `1.5/3.0 GiB`. A phase-line progress shape, `Compiling ...`.

**The live finding.** The timeout message was nearly unreachable. rho adopts a command
that outruns its foreground timeout, and it backgrounds a command matching a long-running
shape before the timeout applies. So `ToolError::Timeout` almost never fires in a real
session, and a `timeout_ms` of 1000 silently became a background task. The model
concluded it had asked for one.

The fix moved the hint from the error to **any background start**, keyed on the value
rather than the reason. Two rounds of live testing were needed, because the first fix
covered only the adoption path and the shape heuristic fired first.

**Not adopted, and why.** jcode wraps `cargo` through its own repository script, which
encodes its build policy and does not generalise.

jcode also detects when a child is waiting on standard input, then asks the user for a
line. rho gives the child a null stdin, so an interactive command hangs instead. That is
a real gap and the feature is genuinely good. It is also about 180 lines of per-platform
unsafe code, reading `/proc` on Linux and libproc through FFI on macOS. So it needs its
own spec and its own test plan. It is recorded here as a known gap rather than half-built.

**A process lesson, and it is mine.** While verifying one of these tests, the controller
ran `git checkout` on a file with uncommitted work, and destroyed the change it had just
written. A copy of the file happened to exist in `/tmp`, so only one edit was lost. This
is decision D-028, one commit after writing it. **Verify a guard by copying the file,
never by asking git to restore it.**

## D-030 — Three tiers, named for what each contributes

The owner asked for a name for the basic tool set, and for the agent-level and extension
layers around it. `docs/extending.md` now names them.

| Tier | Name | Contributes |
| --- | --- | --- |
| 0 | Core tools | The irreducible set. Read, change, find, run. |
| 1 | Capability loaders | Nothing of their own. They load somebody else's capability. |
| 2 | Extensions | New tools and new hooks. |

**Named for the contribution, not the importance.** That is the distinction which decides
where a new thing belongs. A loader is not a lesser tool; it is a different kind of thing.

**Tier 0 is closed at nine tools**: `read`, `write`, `edit`, `list`, `glob`, `grep`,
`bash`, `task`, `task_cancel`. Three rules define the tier. No network, so the set works
offline and links no HTTP client. Every path goes through `confine`, with `bash` the
documented exception. Every tool declares a real `ToolKind`.

**Why closed.** A tool costs context in every request whether the model uses it or not. A
tenth must earn that permanent cost. Anything answering "some users will want this" is
tier 2.

**Tier 1 loaders all fail closed, and that repetition is the design.** A skill from the
repository is withheld until trusted, D-022. An MCP server never classifies its own tools,
D-024. Nor does a plugin, D-017. A capability from outside is mutating until a human says
otherwise.

**Two defects this documentation work found.**

First, the doc claimed nine tools and nobody had checked. Counting them from the code gave
nine, and every kind matched, but the claim had been an assertion. Two tests now pin the
set and the kinds, so the doc cannot drift.

Second, and worse: writing the guard exposed that **the core tools were listed twice**, in
`builtin_tools` and again in `builtin_tools_with_tasks`. The controller deleted `grep` from
one list and every test stayed green, because the membership test read the other list. So
adding a core tool to one place would have silently omitted it from the other. There is one
list now, and a test compares the two sets rather than trusting either.

That is decision D-026 again, in a third place. Duplicated code drifts, and a test that
reads only one copy proves nothing about the other.
