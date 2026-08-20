# The subagent contracts

Every contract the subagent feature exposes, in one page. A contract is any place where two
sides must agree, so this page covers the public API, the data model, the error set, the
persisted format, the configuration, and the extension points. See AGENTS.md step 3.

**This page is a reference, not a design document.** Each contract names the spec that owns it,
and the spec holds the reasoning. If this page and the code disagree, the code wins and this
page is a defect.

| Contract | Owning spec |
| --- | --- |
| The agent definition on disk | `SPEC-subagents` section 5 |
| Confinement | `SPEC-subagents` section 3 |
| The spawn tree and its limits | `SPEC-subagents` section 7 |
| A live child, and polling a background one | `SPEC-subagents` section 7a |
| The task and the gate | `SPEC-agent-tasks` |
| The result | `SPEC-subagents` section 6 |
| Steering | `SPEC-steering` |
| Events | `SPEC-subagents` section 9 |
| The model-facing tools | `SPEC-subagents`, `SPEC-agent-tasks` |

---

## 1. An agent is a file

A definition lives in `~/.rho/agents/*.md`, or in `.rho/agents/*.md` inside a project. A
project definition stays off until the user passes `--trust-project`. A definition carries
instructions, a tool list, and a model choice, and it runs unattended. So it needs more trust
than a skill, not less. See decision D-project-skill-needs-trust.

```markdown
---
name: scout
description: Fast recon. Locates code and reports where things are.
tools: read, grep, list
model: anthropic/claude-haiku-4.5
max_turns: 12
sandbox: strict
---
You locate code and report where things are. You do not change files.
```

`rho-skills` parses it into:

```rust
pub struct AgentDefinition {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub origin: SkillOrigin,
    pub tools: Option<Vec<String>>,
    pub model: Option<String>,
    pub max_turns: Option<u32>,
    pub sandbox: Option<SandboxMode>,
    pub warnings: Vec<String>,
}
```

| Field | Required | Rule |
| --- | --- | --- |
| `name` | yes | The same character rules as a skill. |
| `description` | yes | The model reads it to choose. Without it the definition does not load. |
| `tools` | no | Intersected with the parent's set. `None` inherits. `all` and `*` also inherit. `none` means no tools. An empty list means no tools. |
| `model` | no | Overrides the inherited model. The provider is never overridable. |
| `max_turns` | no | Capped by the parent's. |
| `sandbox` | no | May only narrow. |

**Every optional field can lower a limit and none can raise one.** `warnings` carries a
dropped tool name, so a mistake in a definition is visible rather than silent.

**The three tool keywords.** `tools: all` and `tools: *` inherit the parent's whole set, which is
what an absent field already means. `tools: none` is an empty set, stated on purpose. A keyword
must stand alone: a line that mixes `all` with a real name drops the keyword, keeps the name, and
warns. The narrow reading wins every time, because a wrong widening is an escalation and a wrong
narrowing is a visible failure. The keyword is resolved by the loader in `rho-skills`, never by
`intersect_tools`, so the security core keeps one literal meaning. See decision
D-a-tool-keyword-stands-alone.

## 2. Confinement: a child is never more permissive than its parent

Four rules, and each one is enforced by construction rather than by a check.

```rust
/// Allow a call only when both policies allow it.
pub struct BothPolicies { /* private */ }
impl BothPolicies {
    pub fn new(parent: Arc<dyn ApprovalPolicy>, child: Arc<dyn ApprovalPolicy>) -> Self;
}

/// The child's set is the parent's set, filtered by the child's request.
pub fn intersect_tools(parent: &[String], child_request: Option<&[String]>) -> ToolIntersection;
pub struct ToolIntersection {
    pub allowed: Vec<String>,
    pub dropped: Vec<String>,
}

/// A child may narrow the sandbox. It may never widen it.
pub fn narrow_sandbox(
    parent: SandboxMode,
    child_request: Option<SandboxMode>,
) -> Result<SandboxMode, SubagentError>;

/// A child's tool-call budget is the parent's, or less.
pub fn cap_tool_calls(parent: u32, requested: Option<u32>) -> u32;
```

An `ApprovalPolicy` is a trait object with no ordering, so two policies cannot be compared.
That is why the rule is composition and not comparison. Escalation is unrepresentable rather
than tested. See decision D-child-confined-by-composition.

**What a child inherits.**

| Thing | Inherited | Overridable |
| --- | --- | --- |
| Provider | yes | never |
| Session root | yes | never |
| Approval policy | yes, composed | never widened |
| Sandbox mode | yes | narrower only |
| Tool set | yes, intersected | smaller only |
| Model | yes | yes |
| Turn cap | yes | lower only |
| Tool-call budget | yes | lower only |
| Context | **no** | a fresh conversation is the point |
| Task registry | **no** | a child's background task dies with the child |

## 3. The spawn tree, and who may address whom

```rust
pub struct AgentId(pub u64);

pub struct AgentRegistry { /* private */ }
impl AgentRegistry {
    pub fn new(limits: SubagentLimits) -> Self;
    pub fn new_tree(&self) -> AgentNode;
    pub fn limits(&self) -> &SubagentLimits;
    pub fn live_total(&self) -> usize;

    // Scoped, and there is no unscoped form.
    pub fn live_under(&self, caller: &AgentNode) -> Vec<LiveAgent>;
    pub fn descendant(&self, caller: &AgentNode, id: AgentId) -> Option<LiveAgent>;
    pub fn cancel_descendant(&self, caller: &AgentNode, id: AgentId) -> bool;
    pub fn status(&self, caller: &AgentNode, id: AgentId) -> Option<AgentStatus>;
    pub fn record_report(&self, caller: &AgentNode, id: AgentId, report: AgentReport);
}

pub struct AgentNode { /* private */ }
impl AgentNode {
    pub fn id(&self) -> AgentId;
    pub fn depth(&self) -> u32;
    pub fn registry(&self) -> &AgentRegistry;
    pub fn live_children(&self) -> usize;
    pub fn limits(&self) -> SubagentLimits;
    pub fn spawn_child(
        &self,
        agent: impl Into<String>,
        cancel: CancelToken,
    ) -> Result<ChildSpawn, SubagentError>;
}
```

**The registry is process-wide, so every lookup is scoped.** A bare id resolved against the
whole registry let one session cancel another session's child. The unscoped `live` and `handle`
views are now private, because a doc comment is not a boundary: the `agent_status` tool reached
for the unscoped view first. `new_tree` is a constructor and not an accessor, so two calls give
two unrelated trees in one registry, which is what makes the cross-tree test possible. See
decision D-a-caller-addresses-only-its-own.

```rust
pub struct ChildSpawn {
    pub node: AgentNode,
    pub slot: ChildSlot,   // dropping it frees both counts and deregisters the handle
}
impl ChildSpawn {
    pub fn queue(&self) -> MessageQueue;
    pub fn progress_sender(&self) -> watch::Sender<AgentProgress>;
    pub fn publish(&self, progress: AgentProgress);
}

pub struct LiveAgent {
    pub id: AgentId,
    pub agent: String,
    pub depth: u32,
}
impl LiveAgent {
    pub fn cancel(&self);
    pub fn is_cancelled(&self) -> bool;
    pub fn cancel_token(&self) -> CancelToken;
    pub fn progress(&self) -> AgentProgress;
    pub fn steer(&self, message: Vec<ContentBlock>) -> Result<usize, QueueError>;
    pub fn queued(&self) -> usize;
}

pub struct AgentProgress {
    pub turns: u32,
    pub usage: Usage,
}
```

A `LiveAgent` exists only while its child runs. `spawn_child` registers it and
`ChildSlot::drop` removes it. A handle to a finished child would cancel nothing, so none is
handed out.

### A finished child still answers

```rust
pub enum AgentStatus {
    Queued { agent: String, depth: u32, position: usize, cancelled: bool },
    Running { agent: String, depth: u32, progress: AgentProgress, queued: usize },
    Finished { report: AgentReport },
}
```

A background child usually finishes while its parent is busy, and `ChildSlot::drop` removes the
live handle at once. A lookup that knew only live children would lose every result the parent
asked for. So `record_report` keeps the report and `status` answers from it.

`Queued` answers for a child that holds an id and no slot. The place is computed on read, never
stored, because the children ahead leave and nothing would renumber a stored place. `cancelled`
is read from the entry's own token, so a child that was cancelled while it waited is never told
that it will start. See decision D-a-cancelled-waiter-says-so.

### A full parent queues a child, and never refuses it

```rust
pub enum Admission {
    Started(ChildSpawn),
    Queued(QueuedChild),
}

pub enum Dequeued {
    Cancelled,
    ProcessWideFull { limit: usize },
}

impl AgentNode {
    /// Reserve a slot now, or queue when this parent's cap is full. The tools call this.
    pub fn admit_child(
        &self,
        agent: impl Into<String>,
        cancel: CancelToken,
    ) -> Result<Admission, SubagentError>;

    /// Reserve a slot now, or refuse. A caller that must not wait calls this.
    pub fn spawn_child(
        &self,
        agent: impl Into<String>,
        cancel: CancelToken,
    ) -> Result<ChildSpawn, SubagentError>;
}

impl QueuedChild {
    pub fn id(&self) -> AgentId;
    pub fn agent(&self) -> &str;
    pub fn depth(&self) -> u32;
    pub fn position(&self) -> usize;
    pub fn cancel(&self);
    pub fn is_cancelled(&self) -> bool;
    pub fn queue(&self) -> MessageQueue;
    /// Wait for a per-parent slot, then take the process-wide one without waiting.
    pub async fn started(self) -> Result<ChildSpawn, Dequeued>;
}
```

Only the per-parent cap queues. Waiting on it waits for this parent's own child. Every other cap
refuses at once, because waiting could not fix it: waiting adds no depth, breaks no cycle, and a
wait on the process-wide cap is a wait on another tree. Both wait lines are bounded, and a full
line refuses and names its flag. `spawn_agent` and `spawn_agents` both call `admit_child`, so a
fan-out wider than the cap runs every task. See `SPEC-subagent-slots-handles-grace` sections 2.1
to 2.9.

The registry keeps the **last 64** reports, oldest dropped first. The bound is deliberate,
because a report holds a summary the model wrote, and an unbounded store keyed by model output
is the shape that already cost this project 805 MB once. See decision D-bash-line-cap.

A remembered report is scoped by the ancestor chain kept beside it, not by the id alone.
`AgentReport` carries no id, and a match on ancestors alone would answer with some other
child's report. That is worse than answering nothing.

## 4. The limits

```rust
pub struct SubagentLimits {
    pub max_depth: u32,                  // 0 forbids spawning
    pub max_children_per_parent: usize,
    pub max_live_total: usize,
    pub child_timeout: Duration,
    pub max_tool_calls: u32,
    pub max_queued_per_parent: usize,
    pub max_queued_total: usize,
    pub grace_turns: u32,
}
```

| Limit | Default | Flag | Reachable from `rho run`? |
| --- | --- | --- | --- |
| `max_depth` | 2 in the library, 1 in the CLI | none, on purpose | no, a CLI child holds no spawn tool |
| `max_children_per_parent` | 4 | `--max-children-per-parent` | yes, through `spawn_agents` |
| `max_live_total` | 32 | `--max-live-agents` | across sessions in one process |
| `child_timeout` | 600 s | `--child-timeout-secs` | yes |
| `max_tool_calls` | 64 | `--max-agent-tool-calls` | yes |
| `max_queued_per_parent` | 16 | `--max-queued-per-parent` | yes, through `spawn_agents` |
| `max_queued_total` | 128 | `--max-queued-total` | across sessions in one process |
| `grace_turns` | 5 for a child, 0 for a plain session | `--agent-grace-turns` | yes |

The CLI depth is 1 and has no flag, because a command-line child receives no spawn tool and no
flag could change that. See decision D-cli-depth-is-zero.

**A flag is the only way to change a limit today.** `rho-config` parses a `[subagents]` layer,
and no binary reads the resolved config yet. So a config file changes no limit here. The gap
covers the whole config crate, and decision D-the-layered-config-has-no-caller records it.

Both reservations are semaphore permits, so two racing spawns cannot both pass a cap of one, and
a freed permit grants the next waiter directly. A counter cannot be waited on, and a counter plus
a notify loses a wake. See decision D-permits-not-counters. A turn cap counts provider round
trips, so `max_tool_calls` exists to bound a single turn that asks for forty tools.

**A queued child spends none of its timeout while it waits.** The clock lives in
`collect_report`, which runs only after `started` resolves.

## 5. The task, and the gate that verifies it

A prompt is not a task. A child that says "done" proves nothing.

```rust
pub struct AgentTask {
    pub agent: String,
    pub goal: String,                 // becomes the child's first message
    pub artifacts: Vec<ArtifactSpec>,
    pub acceptance: Vec<Acceptance>,
}
impl AgentTask {
    pub fn new(agent: impl Into<String>, goal: impl Into<String>) -> Self;
    pub fn with_artifacts(self, artifacts: Vec<ArtifactSpec>) -> Self;
    pub fn with_acceptance(self, acceptance: Vec<Acceptance>) -> Self;
    pub fn validate(&self) -> Result<(), SubagentError>;
}

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArtifactSpec {
    File { path: PathBuf },
    Command { run: String },
    Named { name: String, value: String },
}

pub struct Acceptance {
    pub label: String,
    pub check: ArtifactSpec,
}
```

A `File` path goes through `confine`, the same boundary every tool uses, so an artifact cannot
name a file outside the session root. A `Command` runs under the parent's sandbox. A `Named`
kind reaches a registered checker, and an unmatched kind **fails**, because a skipped check
that counts as a pass is the fail-open shape.

**The `Named` field is `name` and not `kind`.** `kind` is the serde tag for the enum, and serde
refuses the collision. The first draft of the spec said `kind` and did not compile.

### The extension points

A third party adds a check kind, a whole gate, or a command runner, and edits nothing in rho.

```rust
#[async_trait]
pub trait Gate: Send + Sync {
    async fn verify(&self, task: &AgentTask, ctx: &GateContext)
        -> Result<GateReport, SubagentError>;
}

#[async_trait]
pub trait ArtifactChecker: Send + Sync {
    fn kind(&self) -> &str;
    async fn check(&self, value: &str, ctx: &GateContext) -> CheckOutcome;
}

#[async_trait]
pub trait CommandRunner: Send + Sync {
    async fn run(&self, command: &str, root: &Path, cancel: &CancelToken)
        -> std::io::Result<i32>;
}

pub struct GateContext {
    pub session_root: PathBuf,
    pub cancel: CancelToken,
    pub runner: Arc<dyn CommandRunner>,
}

pub enum CheckOutcome {
    Pass,
    Fail { detail: String },
}
```

`rho-core` owns the gate and holds no sandbox, so it takes a `CommandRunner`. `rho-tools`
supplies `SandboxedRunner`, which reuses the same command builder as `bash`. One path, so a
check cannot escape through a second path that drifts.

### Who may write a check

**A check comes only from a trusted author: an agent definition, the configuration, or a Rust
caller.** A model may name a file. A model may never write a command, because a command is
executable and a prompt-injected child would choose the command that judges it. See decision
D-an-acceptance-check-has-a-trusted-author.

## 6. The result

```rust
pub struct AgentReport {
    pub agent: String,
    pub outcome: AgentOutcome,
    pub summary: String,               // capped; the only part the model sees
    pub usage: Usage,
    pub turns: u32,
    pub gate: GateReport,              // rho's verified verdict
    pub claims: ChildClaims,           // the child's unverified words
    pub transcript: Option<PathBuf>,   // for a human, never for the model
}

pub enum AgentOutcome {
    Done,
    OutOfTurns,
    Canceled,
    Failed { reason: String },
    Rejected { failed: Vec<String> },
}

pub struct GateReport {
    pub artifacts: Vec<CheckResult>,
    pub acceptance: Vec<CheckResult>,
}
impl GateReport {
    pub fn passed(&self) -> bool;             // an empty report passes
    pub fn failed_labels(&self) -> Vec<String>;
}

pub struct CheckResult {
    pub label: String,
    pub passed: bool,
    pub detail: String,
}

pub struct ChildClaims {
    pub open_questions: Vec<String>,
    pub what_i_did_not_check: Vec<String>,
}
```

**`gate` and `claims` are separate fields, and that separation is the whole point.** A reader
must never mistake a claim for a verified result. No public constructor builds a `CheckResult`,
so a child cannot grade itself. See decision D-a-child-does-not-grade-itself.

`Rejected` is never `Done`, so a reader that trusts only the outcome still sees the failure.

**The persisted format.** `gate` and `claims` both carry `#[serde(default)]`. An older record
without them reads as an empty, passing report, which is the honest reading: a task that
declared no check has nothing to fail.

## 7. Communication

**There is no wire format between a parent and a child.** A child is another `Session` in the
same process. That is the design, and it is why a fan-out is cheap.

### Downward: delegation

`AgentNode::spawn_child` reserves the slot and returns the queue. The caller builds the child
session with that queue.

### Downward: steering

```rust
pub const STEER_QUEUE_CAPACITY: usize = 32;

pub struct MessageQueue { /* a clone shares one queue */ }
impl MessageQueue {
    pub fn new() -> Self;
    pub fn with_capacity(capacity: usize) -> Self;
    pub fn push(&self, message: Vec<ContentBlock>) -> Result<usize, QueueError>;
    pub fn drain(&self) -> Vec<Vec<ContentBlock>>;
    pub fn clear(&self);
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn observe(&self, sender: Sender<AgentEvent>);
    pub fn unobserve(&self);
}

impl Session {
    pub fn steer(&self, message: Vec<ContentBlock>) -> Result<usize, QueueError>;
    pub fn queue(&self) -> MessageQueue;
    pub fn with_queue(self, queue: MessageQueue) -> Self;
}
```

Four rules bind every side:

- **One delivery point.** The driver drains the queue after the current tool calls finish and
  before it builds the next request. A message never lands inside a provider request.
- **Append only.** Delivery adds a user turn through `Context::append` and edits no earlier
  turn, so the stable prefix stays byte-identical and the provider cache stays warm.
- **Bounded, and never a silent drop.** A full queue returns `QueueError::Full` and keeps every
  earlier message. An unbounded queue is a memory defect, and this project shipped one before.
- **A cancel keeps the queue.** Dropping user input as a side effect is the worse failure, so
  only an explicit `clear` empties it.

The announcement lives in the queue, not in one caller, so **every** pusher announces. A
subagent steered through `LiveAgent::steer` is announced the same way a user message is.

### Downward: cancellation, one way only

```rust
impl CancelToken {
    pub fn child(&self) -> CancelToken;
}
```

An ancestor cancelling stops every descendant. A descendant cancelling leaves its ancestors
running. A shared token gave the second behaviour, and a child timeout then ended the whole
session. `cancel` uses `notify_waiters`, so a parent cancel wakes every parked sibling.

### Upward: polling

A background child reports through one call, `agent_status`. It answers for a running child and
for a finished one, and the finished answer is the same `AgentReport` a blocking spawn returns.
An unknown id is a **result**, not a fault.

Polling and events are both offered on purpose. An event needs a frontend that watches a
stream. A poll needs nothing, so the model itself can use it inside one turn.

### Upward: events

```rust
pub enum AgentEvent {
    // ... the turn and tool variants ...
    MessageQueued    { position: usize },
    MessageDelivered { count: usize },
    AgentSpawned     { id: AgentId, agent: String, depth: u32 },
    AgentProgressed  { id: AgentId, turns: u32, usage: Usage },
    AgentFinished    { id: AgentId, report: AgentReport },
}
```

A tool sends these through `ToolContext`:

```rust
pub struct ToolContext {
    pub session_root: PathBuf,
    pub cancel: CancelToken,
    pub updates: Sender<String>,             // streamed output lines
    pub agent_events: Sender<AgentEvent>,    // typed events
}
```

The driver forwards each one into the parent's stream, and it drains the channel again after
the tool returns. That final drain carries `AgentFinished`.

**An agent event never carries a transcript.** Watching a child costs the parent no context.

## 8. The model-facing tools

Five tools. Four are `ToolKind::Execute`, so `--read-only` denies them. `agent_status` is
`ToolKind::Read`, because reading a child's progress changes nothing.

| Tool | Arguments | Kind | Notes |
| --- | --- | --- | --- |
| `spawn_agent` | `agent`, `prompt`, `artifacts`, `background` | Execute | `agent` is a schema `enum` of the loaded names |
| `spawn_agents` | `tasks`, at least one | Execute | one fan-out per call, results in request order |
| `steer_agent` | `id`, `message` | Execute | scoped to the caller's own descendants |
| `agent_status` | `id` | Read | answers for a running child and a finished one |
| `cancel_agent` | `id` | Execute | siblings and the parent keep running |

**`background` is the difference between waiting and polling.** The default is false, and the
call then blocks until the child stops, so the parent can never poll it. With `background: true`
the tool waits for the id only, never for the work, and returns the id with the three calls that
act on it. A limit refused before the child began still comes back as an ordinary tool result.

**A fan-out cannot be backgrounded.** `spawn_agents` runs every task at the same time and
blocks until all of them finish. Its results come back in request order, so the prompt prefix
stays stable and the provider cache stays warm. A parent that wants to work beside its children
calls `spawn_agent` with `background: true`, once per child.

The `agent` property is an `enum` because a free-text field made the model invent names. Each
agent's `description` reaches the model in the tool description, which is the only way the
model can choose well.

`artifacts` takes file names only. A model cannot reach `ArtifactSpec::Command` through any
field it controls.

An unknown id is a **result**, not a fault. A child finishing between the model reading the
list and acting on it is ordinary. The refusal names the id and lists what is running.

## 9. The error set

Every refusal names the limit, its value, and what to do instead.

```rust
pub enum SubagentError {
    EmptyGoal,
    DepthExceeded { limit: u32, attempted: u32 },
    TooManyChildren { limit: usize, current: usize },
    TooManyLiveAgents { limit: usize, current: usize },
    WeakerSandbox { parent: SandboxMode, requested: SandboxMode },
    CycleDetected,
    RetryCapReached { deaths: u32, limit: u32 },
    QueueFull { scope: QueueScope, limit: usize },
}

/// Which wait line filled up. A reader must know which flag to raise.
pub enum QueueScope {
    Parent,
    Process,
}

pub enum QueueError {
    Full { capacity: usize },
}
```

`TooManyChildren` stays in the set, because `spawn_child` still refuses. `admit_child` never
returns it, because that cap queues. `TooManyLiveAgents` refuses twice: at admission, and again
at the start through `Dequeued::ProcessWideFull`, because a waiter holds no process-wide permit
while it waits.

**A refusal must teach something true.** The depth message once named a flag that did not
exist, so it sent the reader after a fix that could not work.

**A child failure is a result, not the end of the run.** A dead child, a refused limit, and an
unknown agent name all return a tool result the model can act on. The parent continues.

## 10. What is deliberately not here

- **No agent-to-agent messaging.** A dependent reads an upstream artifact. jcode built the chat
  version first, then wrote down that it was the wrong shape. See decision D-artifact-not-chat.
- **No wire protocol between a parent and a child.** They share an address space.
  `SPEC-jsonl-frontend` and `SPEC-acp` are host-to-rho contracts, not agent-to-agent.
- **No fault isolation.** A panic in a child can end the process. rho trades isolation for
  density, and a host that needs isolation runs rho in separate processes.
- **No resume of a finished child.** A child transcript is a debug log today, not a session
  record.
- **No definition-supplied acceptance check.** A Rust caller reaches `Acceptance` and the
  `Named` kind. A definition cannot declare one yet.
