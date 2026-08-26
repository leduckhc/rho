# SPEC-subagent-slots-handles-grace — Slot queue, named handles, and grace turns

Status: delivered. `bench/check-spec-tests.py` enforces every test name below.

One line in section 5 is exempt, and it says on the line itself why no test can exist for it.
The status line must not carry that word, because the guard reads this line first and exempts a
whole spec that mentions it. That is how this spec sat unchecked for one run.
Owning crates: `rho-core` for the spawner, the registry, and the run loop. `rho-tools` for
the five model-facing tools. `rho-cli` for the flags.

**`rho-cli` is a side of this contract, not a reader of it.** `SubagentLimits` is a struct
literal in `subagent_limits`, at `crates/rho-cli/src/cli.rs`, so a new field is a compile
error there until the flag is wired. That is deliberate: a limit rho refuses on, and that no
flag can raise, teaches the user a lie. A flag that parses and changes nothing is the same
defect from the other side. Three tests hold that wiring, in section 5.
Features: three new rows, F-agent-slot-queue, F-agent-handles, and F-agent-grace-turns. See
`docs/features.md`.

Every contract named here belongs in `docs/contracts-subagents.md`. That page is the
reference. This page is the reasoning. This page follows `SPEC-subagents`.

## 1. Why one spec for three gaps

A source comparison against pi and jcode found three gaps. All three touch the same types:
`AgentNode::spawn_child`, `AgentId`, `AgentStatus`, `LiveAgent`, and the run loop. A
separate spec each would fight over those types. So one spec settles all three.

The three gaps:

- rho refuses a spawn over a concurrency cap. pi queues it and returns an id.
- A model addresses a child by a bare integer. pi derives a name.
- A child hits the turn cap with no warning. pi warns it first.

Two rules from `SPEC-subagents` hold throughout, and this spec never weakens them.

- **A caller addresses only its own descendants.** Every lookup is scoped. See decision
  D-a-caller-addresses-only-its-own.
- **A refusal must teach, and it must name no flag that does not exist.**

## 2. The slot queue

### 2.1 Where the waiting happens

**The per-parent cap queues the child. It does not refuse.** The spawn returns a live id at
once. The child starts when a slot frees. rho does the waiting, not the model. See decision
D-queue-over-refuse.

**The process-wide cap still refuses.** It is shared by every tree in the process, so a wait
on it would let one session's children hold another session's fan-out for as long as they run.
The old refusal always made progress, and a cross-tree wait does not. So only the cap that one
parent controls may queue. See decision D-the-process-wide-cap-still-refuses.

The alternative was a tool call that blocks the model until a slot frees. rho rejects it.
A blocked model holds no id, so it cannot poll, steer, or cancel the pending child. A
queued id gives the model all three, exactly as a background spawn does.

This still obeys the rule that a refusal must teach. A per-parent cap is not a refusal any
more, because the work is admissible under one parent. It only cannot start yet. The caps that
stay a refusal still teach. See section 2.7.

### 2.2 The admission contract

```rust
/// The result of admitting a child when the per-parent cap may be full.
///
/// The per-parent cap no longer refuses. It queues. So a caller must handle both a
/// slot that was free and one that was not. See decision D-queue-over-refuse.
pub enum Admission {
    /// A slot was free. The child holds it and may start now.
    Started(ChildSpawn),
    /// No slot was free. The child has an id and waits in its parent's line.
    Queued(QueuedChild),
}

/// A child that holds an id but not yet a slot.
///
/// It is addressable while it waits, through the registry, by the same id a started
/// child uses. Its `CancelToken` and its `MessageQueue` exist now, so a steer buffers
/// and a cancel lands before the child ever runs.
pub struct QueuedChild {
    /* private:
       - `registry: AgentRegistry`, a clone, because `position`, `cancel`, and
         `started` all read or change `RegistryState`. Without it the struct cannot
         back its own methods, and a cold implementer found exactly that.
       - `id`, `agent`, `depth`, `ancestors`, `cancel`, `queue`, `sequence`.
       - `child_permits: Arc<Semaphore>`, the parent's semaphore. `started` creates
         the `acquire_owned` future inside itself, so a cancel drops the future and
         takes no permit. A stored future would need a different field type and would
         make `Drop` harder.
       - `handed_out: bool`, which `Drop` reads. */
}

impl QueuedChild {
    pub fn id(&self) -> AgentId;
    pub fn agent(&self) -> &str;
    pub fn depth(&self) -> u32;
    /// The child's place in its parent's wait line, counted from one.
    pub fn position(&self) -> usize;
    /// Give up the place. Then `started` resolves `Err(Dequeued::Cancelled)`.
    pub fn cancel(&self);
    pub fn is_cancelled(&self) -> bool;
    /// The queue the child reads once it starts. A steer buffers here meanwhile.
    pub fn queue(&self) -> MessageQueue;
    /// Wait for a permit, then return the live spawn.
    ///
    /// It resolves `Err(Dequeued::Cancelled)` when the child was cancelled while it
    /// waited, alone or with its parent. It resolves
    /// `Err(Dequeued::WaitedTooLong)` when `SubagentLimits::queue_wait` passed and no
    /// slot freed. It resolves `Err(Dequeued::ProcessWideFull)` when a slot freed
    /// under this parent and the process-wide cap was full at that moment. It never
    /// waits on the process-wide cap, because that is a wait on another tree.
    ///
    /// The deadline clock starts at the first poll of this future, which is when the
    /// child begins to wait.
    pub async fn started(self) -> Result<ChildSpawn, Dequeued>;
}

/// Dropping a queued child removes its registry entry.
///
/// Every path that is not a successful handout runs this: a cancel, a parent
/// cancel, a full process-wide cap, and a caller that drops the value without ever
/// awaiting `started`. Without it the entry stays in the map, the map grows with
/// every model-driven spawn, and `agent_status` keeps answering "queued" for a
/// child that will never run. `ChildSlot` already does exactly this for a live
/// child. See decision D-a-queued-child-lives-in-the-registry.
///
/// **Rust forbids a partial move out of a type that implements `Drop`.** So
/// `started` cannot move the queue or the permits out of `self`. It wraps those
/// fields in an `Option` and takes them. `Drop` removes the entry only when
/// `handed_out` is false. An implementer who misses this fights the borrow checker
/// and may delete the guard to escape it.
///
/// **`handed_out` becomes true at one moment only: after the process-wide permit is
/// held and the live handle is inserted.** Setting it earlier, when the fields are
/// taken, would skip the removal on the `ProcessWideFull` and `Cancelled` exits, and
/// the entry would then answer "queued" for ever. That is the leak this flag exists
/// to prevent, reintroduced by an ordering mistake. A fourth reviewer found it here.
impl Drop for QueuedChild {
    fn drop(&mut self) { /* remove the queued entry, unless it was handed out */ }
}

/// Why a queued child never started.
///
/// Every variant is a result the parent can act on, and none ends the run.
pub enum Dequeued {
    /// The child was cancelled while it waited, alone or with its parent.
    ///
    /// A cancel wins over the deadline below. A cancel is what the parent asked for, so
    /// a waiter that is cancelled and past its deadline still reports this.
    Cancelled,
    /// The child waited for `SubagentLimits::queue_wait` and no slot freed.
    ///
    /// It bounds one tool call. Without it, one blocking spawn could hold a parent's
    /// turn for the wait line depth times `child_timeout`. See section 2.8 and decision
    /// D-a-waiter-has-a-deadline.
    WaitedTooLong { limit: Duration },
    /// A slot freed under this parent, and the process-wide cap was full by then.
    ///
    /// A queued child holds no process-wide permit while it waits, so the cap can
    /// fill between the admission and the start. rho refuses rather than wait,
    /// because a wait on that cap is a wait on another tree. The refusal names the
    /// limit and `--max-live-agents`, which is the flag `TooManyLiveAgents` names.
    /// It cannot share that message, because `TooManyLiveAgents` also interpolates
    /// `current` and this variant does not carry it. See decision
    /// D-the-process-wide-cap-still-refuses.
    ProcessWideFull { limit: usize },
}
```

The `ProcessWideFull` text, verbatim, so nobody has to invent it:

```text
a slot freed for this child, and the process-wide limit of {limit} live agents was reached
first. The work was not started. Run it again when a child finishes, or ask the user to raise
--max-live-agents.
```

The `WaitedTooLong` text, verbatim:

```text
the child waited {seconds} seconds for a slot and none freed, so the work was not started.
Run it again later, spawn it with background: true, or ask the user to raise
--queue-wait-secs.
```

### 2.2a A permit, not a counter

**The reservation becomes a semaphore permit. The compare-and-swap counters go.** A counter
cannot be waited on. A wait built on a counter plus a notify loses a waiter: a task that
fails its retry, and then reaches its next `await`, misses a `notify_waiters` that fired in
between. Then it sleeps until the next child happens to finish, and it hangs when none does.
See decision D-permits-not-counters.

```rust
/// Inside `AgentRegistry`. One permit is one live child.
struct RegistryInner {
    limits: SubagentLimits,
    /// The process-wide live cap. `max_live_total` permits.
    live_permits: Arc<tokio::sync::Semaphore>,
    // ... the id counter, the live map, the finished ring, and the queued map ...
}

/// Inside `AgentNode`. One semaphore per parent, shared by that parent's clones.
struct NodeInner {
    /// The per-parent cap. `max_children_per_parent` permits.
    child_permits: Arc<tokio::sync::Semaphore>,
}

/// The reservation. Dropping it frees both permits and deregisters the handle.
pub struct ChildSlot {
    /* private: two OwnedSemaphorePermit values, plus the registry handle */
}
```

`tokio::sync::Semaphore` grants permits in first-in-first-out order. So the queue order in
section 2.9 comes from the primitive, not from a second structure that could disagree with it.

- `spawn_child` calls `try_acquire_owned` on both semaphores. A failure is the same refusal it
  returns today, so the immediate form is unchanged for every caller.
- `QueuedChild::started` calls `acquire_owned` on the per-parent semaphore, then
  `try_acquire_owned` on the process-wide one. It selects on the child's `CancelToken` and on
  a `queue_wait` timer, so a cancel and a deadline each resolve the wait at once.
- **The cancel branch is checked before the deadline branch, and the select is `biased`.** A
  waiter that is cancelled and past its deadline reports `Cancelled`, because a cancel is what
  the parent asked for. A fair select would report either one, so the order is stated in code.
- **When the process-wide try fails, the child releases its per-parent permit and refuses.** It
  resolves `Err(Dequeued::ProcessWideFull)`. It must release first, because a waiter that keeps a
  per-parent permit while it gives up blocks a sibling for ever. It must not retry, because a
  retry with no wake source is the spin this design removed. It must not await, because that is
  the cross-tree wait section 2.1 forbids.
- **The acquire order is per-parent first, then process-wide.** A per-parent permit is
  contended only by one parent's own children, and each of those is either running, and so
  will finish, or queued behind this child in one line. So the order cannot deadlock. A
  waiter that holds a per-parent permit does hold it while it takes the second one, and that
  is deliberate: it keeps one parent's start order stable.
- **`acquire_owned` is cancel-safe.** Dropping the future takes no permit, and a permit taken
  and then dropped returns to the semaphore. So the cancel branch of the select leaks nothing.

### 2.2b A queued child lives in the registry, under one state lock

**A queued child is registered, and every lookup that finds it is scoped.** Without this,
"addressable by id" is false: the tools reach a child only through
`AgentRegistry::descendant` and `AgentRegistry::status`, which read the live map and the
finished ring. A `QueuedChild` held by the spawning task alone appears in neither, so
`steer_agent`, `cancel_agent`, and `agent_status` would all answer "no subagent with id N".
See decision D-a-queued-child-lives-in-the-registry.

```rust
struct RegistryInner {
    limits: SubagentLimits,
    /// The process-wide live cap. `max_live_total` permits.
    live_permits: Arc<tokio::sync::Semaphore>,
    next_id: AtomicU64,
    /// Every index, under one lock. Never held across an await.
    state: Mutex<RegistryState>,
}

struct RegistryState {
    /// Every live child, by id.
    live: HashMap<u64, LiveAgent>,
    /// Every child that holds an id and no slot.
    queued: HashMap<u64, QueuedEntry>,
    /// The next queue sequence, for the whole registry.
    ///
    /// One counter is enough. `position()` filters by parent, and a subset of a
    /// monotonic sequence keeps its order. A counter per parent would need its own
    /// map, and that map would need its own cleanup.
    next_sequence: u64,
    /// The reports of children that finished, oldest first, bounded at 64.
    /// `FinishedAgent` is the existing private type in `tree.rs`, unchanged.
    finished: VecDeque<FinishedAgent>,
    /// The derived handles and the aliases, by tree root. See section 3.
    handles: HashMap<AgentId, HandleTable>,
}

/// One tree's names. A handle is derived, an alias is chosen, and neither may
/// shadow the other. Keyed by tree root, so a name never crosses a tree.
struct HandleTable {
    /// The derived handle of each child in this tree, by id. `explore`, `explore-2`.
    derived: HashMap<u64, String>,
    /// How many children of each agent name this tree has ever numbered. It only
    /// grows, so a number is never reused while a report is remembered.
    counts: HashMap<String, u32>,
    /// The caller-chosen alias of each child, by id.
    aliases: HashMap<u64, String>,
}

/// A queued child's entry. The ancestor chain sits beside it, exactly as the finished
/// ring stores it, because the live handle that would carry it does not exist yet. A
/// lookup without the chain would let one tree reach another tree's queued child. That
/// is the escape decision D-a-caller-addresses-only-its-own forbids.
struct QueuedEntry {
    agent: String,
    depth: u32,
    ancestors: Vec<AgentId>,
    cancel: CancelToken,
    queue: MessageQueue,
    /// A monotonic number, taken from `RegistryState::next_sequence` when the child
    /// queued.
    ///
    /// The place in the line is **not** stored. The children ahead leave, and nothing
    /// would renumber the rest. A stored place goes stale, and a wrong number is worse
    /// than none, because a model acts on it. `position()` computes the place under the
    /// state lock: one, plus every entry with the same immediate parent and a smaller
    /// sequence. The immediate parent is `ancestors.last()`, so siblings are
    /// identifiable from the entry alone.
    sequence: u64,
}
```

**One mutex holds every index, and nothing holds it across an await.** Two reviewers reached
this independently. The reason is that the move from queued to live must be one step. With
separate locks a lookup can find an id in both indexes or in neither, a cancel can report failure
while the child starts, and two concurrent admissions can derive one handle. A stated lock order
would only make those races rarer. See decision D-one-registry-state-lock.

**Registration and handle allocation happen together.** `admit_child` takes the state lock once.
It inserts the queued entry, derives the handle against all three indexes, and stores the
binding. So two concurrent admissions of one agent name cannot pick one handle.

**The order inside `admit_child` is: check, then allocate.** It checks the depth limit, the cycle
guard, the process-wide live cap, **the per-parent wait line, then the process wait line**, before
it takes an id or a sequence number. So a refused admission burns neither. The two wait-line checks
are ordered on purpose: the per-parent limit is the tighter and more actionable one, so a caller
that trips both is told about its own line first. Without a stated order, two implementers would
report different scopes for the same call.

**Both wait-line counts are derived from the map, never stored.** The process count is the size of
`queued`. A parent's count is the number of entries whose `ancestors.last()` matches that parent.
Both are read under the state lock. So the removal in `started` and in `Drop` **is** the decrement,
and no separate counter exists to drift. A stored counter would need decrementing on four exits: a
start, a cancel, a parent cancel, and a drop without an await. A leaked count is worse than a
leaked entry, because it refuses work for ever with no visible cause.

**`admit_child` is synchronous, and that is what makes the allocation atomic.** It takes a
`std::sync::Mutex` and returns without awaiting. So no other task can observe a half-registered
child, and no test can inject itself into the critical section. The earlier draft promised a
barrier test, and there is no seam for one.

**The handout is one step.** `QueuedChild::started` takes the state lock after it holds both
permits. It removes the queued entry and inserts the live handle before it releases the lock. So
no id is ever in both indexes, and none is ever in neither.

**That forced one shape on the builder.** A first implementation registered the handle inside the
function that builds a child, which meant three separate locks and made this promise false. So the
builder now returns the node and the handle, and each caller registers under its own lock.
`build_child` takes a named `ChildBuild`, because clippy refused eight arguments and decision
D-no-four-argument-session-new says the same thing.

**A handout reuses the id and the queue the caller already holds.** Two tests found this the hard
way: the first implementation allocated a fresh id, so the id the model was told went nowhere, and
it built a fresh queue, so a steer the caller had been told was accepted was silently dropped. See
`the_started_child_keeps_the_id_the_caller_was_given` and
`steering_a_queued_child_buffers_until_it_starts`.

**`status` prefers the live answer**, because it is the newer truth.

The scoped accessors read three indexes, in this order: live, queued, then finished.
`descendant` returns `None` for a queued child, because a `LiveAgent` cancels and steers a
running child. `status`, `cancel_descendant`, and `resolve` all answer for a queued child. So
`cancel_descendant` gains a queued branch, rather than routing only through `descendant`.

**Steering a queued child goes through the registry too.** `AgentRegistry::steer_scoped`
pushes into the live handle's queue, or into the queued entry's queue, whichever holds the
id. One entry point, so a queued child and a live child cannot drift apart.

The new entry point on `AgentNode`:

```rust
impl AgentNode {
    /// Reserve a slot now, or queue when the per-parent cap is full.
    ///
    /// It refuses at once for a cap that waiting cannot fix: the depth limit, the
    /// cycle guard, the process-wide live cap, and a full wait line. It queues for
    /// the per-parent child cap alone. The tools call this. See decision
    /// D-queue-over-refuse and decision D-caps-that-cannot-wait-refuse.
    pub fn admit_child(
        &self,
        agent: impl Into<String>,
        cancel: CancelToken,
    ) -> Result<Admission, SubagentError>;
}
```

`spawn_child` stays, unchanged, as the immediate form:

```rust
impl AgentNode {
    /// Reserve a slot now, or refuse. This never queues.
    ///
    /// A Rust caller uses it to bypass the queue, the way pi's scheduled job passes
    /// `bypassQueue: true`. `admit_child` is the queueing form the tools use. So the
    /// bypass is a method choice, not a flag. This is the extension point for a
    /// caller that must not wait.
    pub fn spawn_child(
        &self,
        agent: impl Into<String>,
        cancel: CancelToken,
    ) -> Result<ChildSpawn, SubagentError>;
}
```

**The wake path.** `ChildSlot::drop` releases both permits. The semaphore then grants the
next waiter in line, and no notify is involved. So a waiter cannot miss a wake, and a lost
wakeup is unrepresentable rather than tested.

### 2.3 A queued state for status

`AgentStatus` gains a third variant. A child with an id and no slot is neither running nor
finished.

```rust
pub enum AgentStatus {
    /// Queued for a slot. It has an id, and it has not started.
    Queued {
        agent: String,
        depth: u32,
        /// The place in the parent's wait line, counted from one.
        position: usize,
        /// Whether this child, or an ancestor, was cancelled while it waited.
        ///
        /// A cancel wakes the waiting task, and the entry leaves the map when that task
        /// drops it. A reader that ignored this told a model the child held a place and
        /// would start, moments after the model cancelled it. A live run found it. See
        /// decision D-a-cancelled-waiter-says-so.
        cancelled: bool,
    },
    Running {
        agent: String,
        depth: u32,
        progress: AgentProgress,
        queued: usize,
    },
    Finished {
        report: AgentReport,
    },
}
```

**An older reader is source code, and the compiler stops it.** `AgentStatus` is an
in-process type. A frontend and the `agent_status` tool match it. A new variant is a
source-breaking change, so every match fails to compile until it handles `Queued`. That is
the safe outcome. A silent fall-through is impossible.

**A newer reader meets no older record, because rho never serialises `AgentStatus`.** The
only persisted subagent record is `AgentReport`, and a queued child has no report yet. So a
queued state is never written and never read back. `AgentReport` keeps its forward compatibility
through `#[serde(default)]`, and **this** spec adds no serialised field.
`SPEC-subagent-worktree-isolation` adds one, `isolation`, with the same `#[serde(default)]` rule.
The two specs share that record, so neither may claim it is frozen.

**`AgentStatus` gains no catch-all variant.** A catch-all is the fail-open shape decision
D-plugin-does-not-classify-itself warns against. So `AgentStatus` stays not serialised
rather than gain an `Unknown` case.

### 2.4 When the timeout clock starts

**The `child_timeout` clock starts when the child starts, not when it queues.** A queued
child spends no part of its 600 seconds while it waits.

The clock lives in `collect_report`, which runs only after `started` resolves. A queued
child never reaches `collect_report` until it holds a slot. So the property holds by
construction. The test names it: `a_queued_child_does_not_spend_its_timeout_while_it_waits`.

### 2.5 Cancel, steer, and a cancelled parent

- **Cancel a queued child.** `QueuedChild::cancel` marks it. `started` then resolves
  `Err(Dequeued::Cancelled)`. The caller records `AgentOutcome::Canceled`. It held no slot,
  so nothing frees except its place in the line. **`status` reports the cancel at once**, through
  `AgentStatus::Queued { cancelled: true, .. }`, because the entry stays in the map until the
  waiting task drops it. In that window `agent_status` states that the child will not start, and
  it offers no place and no steer. See decision D-a-cancelled-waiter-says-so.
- **Steer a queued child.** A steer buffers in the child's queue. The queue exists now, so
  the push is accepted, up to the cap. The child's first turn boundary delivers it once the
  child starts. A full queue returns `QueueError::Full` as usual.
- **A parent cancelled while three children wait.** The parent's cancel is an ancestor
  cancel, so every descendant token fires. All three queued children resolve
  `Err(Dequeued::Cancelled)`. The caller records each as `Canceled`. No slot ever opens for
  them.

### 2.6 What bounds the queue

**The wait line is bounded.** An unbounded queue is a memory defect, and this project
shipped one. A queued child holds a cancel token and a message queue, so an unbounded line
grows without limit.

`SubagentLimits` gains two fields:

```rust
pub struct SubagentLimits {
    pub max_depth: u32,
    pub max_children_per_parent: usize,
    pub max_live_total: usize,
    pub child_timeout: Duration,
    pub max_tool_calls: u32,
    /// How many children one parent may queue for a slot. A full line refuses.
    pub max_queued_per_parent: usize,
    /// How many children may wait in the whole process. A full process refuses.
    ///
    /// A per-parent cap alone does not bound the process. A session root holds no
    /// live-child slot, so `max_live_total` does not cap the number of roots, and a
    /// host may run many sessions in one process. Without this field the waiting
    /// total is the per-parent cap times an unbounded number of roots.
    pub max_queued_total: usize,
    /// How long one child may wait for a slot. Then rho refuses it.
    ///
    /// It bounds one blocking spawn, which the wait line depth used to multiply. The
    /// default is 600 seconds, which is one `child_timeout`. Zero refuses any child
    /// that has to wait, and no value turns the deadline off. See section 2.8 and
    /// decision D-a-waiter-has-a-deadline.
    pub queue_wait: Duration,
    /// The largest steering message a child's queue accepts, in bytes.
    ///
    /// A child queue is written to by a model, and there may be 160 of them. So this
    /// is 16 KiB, where a session queue allows 64 KiB. See `SPEC-steering` section 4
    /// and decision D-a-steering-message-is-bounded-by-bytes.
    pub max_steer_message_bytes: usize,
    /// Turns of warning before the child's turn cap. See section 4.
    pub grace_turns: u32,
}
```

The default `max_queued_per_parent` is 16, flag `--max-queued-per-parent`. The default
`max_queued_total` is 128, flag `--max-queued-total`. Either line, once full, refuses at once.
The default `queue_wait` is 600 seconds, flag `--queue-wait-secs`. The default
`max_steer_message_bytes` is 16 KiB, flag `--max-agent-steer-bytes`.

The refusals:

```rust
pub enum SubagentError {
    EmptyGoal,
    DepthExceeded { limit: u32, attempted: u32 },
    TooManyChildren { limit: usize, current: usize },
    TooManyLiveAgents { limit: usize, current: usize },
    WeakerSandbox { parent: SandboxMode, requested: SandboxMode },
    CycleDetected,
    RetryCapReached { deaths: u32, limit: u32 },
    /// A wait line is full. The message names which line, its limit, and its flag.
    QueueFull { scope: QueueScope, limit: usize },
}

/// Which line filled up. A reader must know which flag to raise.
pub enum QueueScope {
    Parent,
    Process,
}
```

`TooManyChildren` stays in the enum, because `spawn_child` still returns it.
`TooManyLiveAgents` stays for both forms, because the process-wide cap refuses in both. See
decision D-bounded-slot-queue.

**The process-wide ceiling, and it is a real bound now.** A first draft multiplied 16 waiters by
32 parents and called the result bounded. That arithmetic was wrong, because a session root holds
no live-child slot, so `max_live_total` bounds neither the number of roots nor the number of
lines. A reviewer found it. `max_queued_total` fixes it: at most 128 children wait in the process,
each holding a `CancelToken` and a `MessageQueue` of 32 messages, so the ceiling is about four
thousand queued messages. That number is bounded by a limit rho owns, not by how many sessions a
host decides to open.

**A message count is not a memory bound, so the message is capped in bytes too.** A child queue
takes `max_steer_message_bytes`, which is 16 KiB. So one child queue holds at most 512 KiB, and
the 160 queues of 128 waiting and 32 live children hold at most 80 MiB. `MessageQueue::push`
refuses a larger message with `QueueError::TooLarge`. See `SPEC-steering` section 4 and decision
D-a-steering-message-is-bounded-by-bytes.

### 2.7 Which caps still refuse

Waiting frees a per-parent slot. Waiting adds no depth and breaks no cycle. Waiting on a
process-wide slot waits on another tree.

| Cap | `admit_child` behaviour | Why |
| --- | --- | --- |
| `max_children_per_parent` | queue | this parent's own child frees a slot |
| `max_live_total` | refuse at once | only another tree can free it, so a wait is not bounded |
| `max_depth` | refuse at once | waiting adds no depth |
| the cycle guard | refuse at once | waiting breaks no cycle |
| `max_queued_per_parent` | refuse at once, checked first | this parent's wait line is full |
| `max_queued_total` | refuse at once, checked second | the process wait line is full |
| `queue_wait` | not checked here | it bounds the wait itself, so it fires in `started` |

**The check order decides which scope a caller hears.** A call that trips both wait lines is told
about its own line, because that is the tighter and more actionable bound. The order is stated here
and in section 2.2b, so two implementers report the same `QueueScope` for the same call.

**One cap refuses twice.** `max_live_total` refuses at admission, and it refuses again at the
start, through `Dequeued::ProcessWideFull`. A queued child holds no process-wide permit while it
waits, so the cap can fill in between. Both refusals name the same limit and the same flag.

Each refusal names the limit and its value. See decision D-caps-that-cannot-wait-refuse and
decision D-the-process-wide-cap-still-refuses.

### 2.8 The fan-out and the queue

**`spawn_agents` uses the queue.** A task over the per-parent cap now queues instead of a
per-task refusal. Every task runs, in the end, unless the wait line is full or the
process-wide cap is reached. Those two remain per-task refusals, and each names its limit.

**Both spawn tools call `admit_child`. This is built.** `spawn_agent` and `spawn_agents` share
one path, so a single spawn and a fan-out cannot drift. A background spawn sends the id as soon
as the child is admitted, queued or started, because the model polls, steers, and cancels by
that id while the child waits. A queued child that never starts is recorded as a report, so a
parent that polls learns why: `Canceled` for a cancel, and `Failed` for a full process.
`AgentNode::spawn_child` keeps its refusal, and no tool calls it now.

**The result order is unchanged.** `spawn_agents` collects with `join_all`, which keeps the
results in request order whatever the start order. So the request-order promise holds. The
prompt prefix stays stable and the provider cache stays warm. See decision
D-per-parent-fifo-start-order.

**A task that waits long still gets its full budget.** The `child_timeout` clock starts when
the child starts, so a task that waited does not arrive with less time. The retry ledger counts
a death, and a wait is not a death, so waiting cannot consume a retry.

**The cost, and the deadline that bounds it.** A blocking spawn waits instead of refusing. The
worst case for one tool call used to be
`ceil(max_queued_per_parent / max_children_per_parent) x child_timeout`. At the shipped defaults
that was `ceil(16 / 4) x 600 s`, about forty minutes, and a prompt-injected model chose both the
fan-out width and the children that sleep. A security review named it, and this spec left it
open.

**It is closed now. `SubagentLimits::queue_wait` bounds the wait.** One waiter waits at most
`queue_wait`, which is 600 seconds by default and one `child_timeout`. Then `started` resolves
`Err(Dequeued::WaitedTooLong)`, and the parent gets its turn back. So the worst case for one
tool call is `queue_wait + child_timeout`, about twenty minutes at the defaults, and it no
longer grows with the wait line. The flag is `--queue-wait-secs`, and the command line defaults
it to the effective `--child-timeout-secs`. A value of zero refuses any child that has to wait,
and no value turns the deadline off. See decision D-a-waiter-has-a-deadline.

The deadline applies to a background waiter too, because a queued entry holds a cancel token
and a queue while it waits. A background waiter that runs out of patience records a report, so
a parent that polls learns why the child never ran. A caller that cannot afford any wait uses
`background: true`, which returns at once with an id.

**Two rules for a test, because the default hides a mistake.** The default `queue_wait` and
the default `child_timeout` are both 600 seconds, so a test at the defaults cannot tell which
number ended the wait. Every deadline test therefore sets a `queue_wait` that differs from its
`child_timeout`. And `started` now holds a timer, so a test with a paused tokio clock advances
to that timer once nothing else is ready. A test that wants a waiter to stay in the line either
never polls `started`, or it keeps the real clock.

### 2.9 Fairness and order

**Start order is first-in-first-out per parent, and the semaphore provides it.**
`tokio::sync::Semaphore` grants permits in the order tasks began to wait. So no second
ordering structure exists to disagree with the first. Order between two parents is
unspecified.

A single fan-out cares that its own tasks start in the order it asked. Two independent library
callers share no clock. A global order would need a central scheduler, and this design avoids
one on purpose.

**The order is a policy, and it is closed.** A priority queue would edit this rule rather than
add an impl. That is deliberate, because a start order is not third-party surface. Section 6
says so plainly. See decision D-per-parent-fifo-start-order.

## 3. Named handles

### 3.1 The handle contract

**The id stays the identity, unique per process. A handle is a second address for the same
child.** rho derives a handle from the agent name. It numbers a collision. A first
`explore` becomes `explore`. A second becomes `explore-2`. A third becomes `explore-3`.

**A handle is unique per tree, not per process.** Tree A's `explore` and tree B's `explore`
are two different children. The registry keys a handle by the tree root, so the numbering
never crosses a tree. See decision D-handle-is-a-second-address.

**The number counts a live child, a queued one, and a remembered one.** A finished child stays
reportable while its report is in the 64-deep ring, and its handle still resolves. So a new
`explore` must not take a name that a remembered `explore` still answers to. The numbering reads
all three indexes, and it happens **inside the same state lock as the registration**. So two
concurrent admissions cannot choose one name. Without that rule one name binds two children, and
`resolve` picks one of them in silence. Two tests name it:
`a_handle_is_not_reused_while_a_finished_child_is_remembered` and
`sixteen_threads_admitting_one_agent_name_get_sixteen_distinct_handles`. The second races real
threads and asserts the set size, because `admit_child` is synchronous and offers no seam for a
barrier. An earlier draft promised a barrier, and a cold implementer proved there is nowhere to put
one.

```rust
impl AgentRegistry {
    /// The derived handle for a child, unique in its tree.
    pub fn handle_of(&self, id: AgentId) -> Option<String>;

    /// Resolve a reference to a child's id, inside the caller's own tree.
    ///
    /// An id resolves when it names a descendant of `caller`. A digits-only name
    /// resolves as an id. Any other name resolves as a derived handle, then an
    /// alias. It returns `None` for a child of another tree, exactly as an unknown
    /// id returns `None`.
    pub fn resolve(&self, caller: &AgentNode, reference: &AgentRef) -> Option<AgentId>;
}
```

### 3.2 The alias rule

**A caller may name a child. A name belongs to one child, and an alias never shadows a derived
handle.** The model sets an alias through an optional `alias` argument on `spawn_agent`, and on
each task of `spawn_agents`. A Rust caller sets one through the registry.

An earlier draft of this sentence said "one alias per child". The error set has no case for a
second name on one child, so the implementation would have had to invent one or lie with
`Taken`. A second name for one child costs one map entry and hides nothing, so it is allowed.
The rule that matters is the other direction: one name, one owner.

```rust
impl AgentRegistry {
    /// Set one alias for a child, inside the caller's tree.
    ///
    /// It refuses a name that a derived handle already holds. It refuses a name that
    /// another alias already holds. A derived handle always resolves first, so an
    /// alias can never hide a real child.
    pub fn set_alias(
        &self,
        caller: &AgentNode,
        id: AgentId,
        alias: impl Into<String>,
    ) -> Result<(), AliasError>;
}

pub enum AliasError {
    /// A derived handle already holds this name in this tree.
    ShadowsHandle { name: String },
    /// Another alias already holds this name in this tree.
    Taken { name: String },
    /// The id names no child of this caller.
    Unknown { id: AgentId },
    /// The name is longer than `MAX_ALIAS_LENGTH`.
    TooLong { limit: usize, length: usize },
    /// The name holds a character an alias may not hold.
    NotPrintable { name: String },
    /// The name is only digits, so an id would always win.
    DigitsOnly { name: String },
}

/// The longest alias, in characters. The same rule as an agent name.
pub const MAX_ALIAS_LENGTH: usize = 64;
```

**An alias is bounded and printable, because the model writes it.** A model is
prompt-injectable, and an alias is echoed into the parent's tool output beside the list of
running children. So an alias holds at most 64 characters, and it holds no control character
and no newline. A newline in an alias could forge a line that looks like rho's own output, and
an unbounded alias is stored per child. See decision D-an-alias-is-bounded-and-printable.

**A refused alias never fails the spawn.** The child is already admitted, and the work matters
more than the label. `spawn_agent` reports the refusal as a note in its result, names the
reason, and gives the id and the derived handle instead. In a fan-out, two tasks that ask for
one name give the name to the first and a note to the second. The tests name both:
`spawn_agent_reports_a_rejected_alias_without_failing_the_spawn` and
`a_fan_out_gives_one_name_to_one_child_and_notes_the_other`.

`resolve` checks a derived handle before an alias. So the shadow rule holds at read time
too, not only at write time.

**A digits-only name is always an id, so a digits-only handle is unreachable by name.** An
agent name may hold digits, so an agent called `42` derives the handle `42`. A model that
sends `"42"` reaches the child with id 42 instead. `set_alias` therefore refuses a
digits-only alias, and `handle_of` still returns the derived name for display. The id
always reaches the child, so no child becomes unreachable. The test names it:
`a_digits_only_name_resolves_as_an_id_and_never_as_a_handle`.

### 3.3 The tool schema change

**The `id` argument accepts both a number and a name. The old shape keeps working.** A model
already writes integers, and a hard switch to a string would reject them.

```rust
/// A model-facing reference to a child: an id, or a name.
///
/// A JSON number reads as `Id` and a JSON string reads as `Name`. A digits-only
/// name is resolved as an id, so a model that sends `"42"` reaches the same child as
/// one that sends `42`. See decision D-agent-ref-accepts-id-or-name.
///
/// The `Deserialize` impl is written by hand, not derived with `untagged`. An
/// untagged enum answers a boolean, a float, a negative number, `null`, or an object
/// with serde's own message, "data did not match any variant". That message teaches
/// nothing, and a refusal must teach. The hand-written impl names both accepted
/// shapes and shows what arrived.
#[derive(Clone, Debug)]
pub enum AgentRef {
    Id(u64),
    Name(String),
}
```

The refusal text, verbatim:

```text
the id must be a subagent id, such as 7, or a handle, such as "explore-2". This call sent
{arrived}. Call agent_status with no argument to list what is running.
```

An empty string is a name that matches no child, so it is an ordinary not-found result. A
negative number, a float, a boolean, `null`, and an object are all refusals with the text
above.

The schema of `id` on `steer_agent`, `agent_status`, and `cancel_agent` becomes:

```json
{ "id": { "type": ["integer", "string"],
          "description": "The subagent id, or its handle, from the spawn result." } }
```

One function builds that schema for all three tools, so a model that learns the shape from one
may use it on the next. A test asserts all three, because a model uses only what the schema
shows: the agent `enum` had to be added for exactly that reason.

**`agent_status` takes no required argument.** The refusal above tells the model to call it with
none, so that call has to work. It lists this session's live children, each with its handle and
its id, and it says plainly when nothing is running. A refusal that teaches a call rho refuses
is the defect family that already shipped here, so the field is optional and two tests hold it
that way.

**The migration.** A caller that sends the old integer shape resolves through `AgentRef::Id`.
A caller that sends a handle resolves through `AgentRef::Name`. No coordinated release is
needed, because no old shape is refused. This is a `oneOf` rejected in favour of one field,
because one field is simpler for the model and for `serde`.

### 3.4 Scoping

**A handle resolves only inside the caller's own descendants, exactly as an id does.**
`resolve` returns an id, and every scoped accessor then checks that id against the caller's
descendants. So a handle reaches no child of another tree.

Two guards stack, and **the second one is load-bearing, not belt-and-braces.** The handle table
is keyed per tree, so tree B holds no binding for tree A's child. But every node of one tree
shares that key, so the per-tree key alone does not scope a name inside a tree: a name lookup by
a mid-tree caller finds a cousin's binding. The ancestor re-check inside `resolve` is what
refuses it. A mutation that deleted that re-check passed every other handle test, because they
all used a root caller. Two tests now cover the case:
`a_child_cannot_reach_its_uncles_child_by_handle` and `an_alias_cannot_be_set_on_a_cousin`. The
cross-tree case is `one_tree_cannot_reach_another_by_handle`. See decision
D-a-caller-addresses-only-its-own.

**One place binds a name.** `bind_handle` runs inside the registration's own critical section,
for a fresh spawn, for an admission that queues, and for the handout that starts a waiter. It
never re-derives, so the name a queued child was told is the name it keeps. A mutation that made
the handout re-derive was invisible until the handout was routed through that one function.

### 3.5 What a handle does after the child finishes

**A handle resolves while the child is queued, while it is live, and while its report is
remembered.** The remembered-report store is bounded at 64, oldest dropped first. When a
report is evicted, its handle binding is removed too. So a handle resolves for exactly as
long as `agent_status` can answer for the child.

A foreground child records no report and surfaces no id, so its handle drops when its slot
drops. A background or queued child records a report, so its handle outlives its slot until
the report is evicted.

## 4. Grace turns

### 4.1 The trigger

**rho warns the child a fixed number of turns before its turn cap.** The default is 5 grace
turns. The warning fires at a turn boundary, when the turns that remain first reach the
grace count.

**The warning applies to the turn cap only, not the tool-call budget.** A tool-call budget
is spent inside a turn, so it has no safe boundary to warn at, and a count of remaining
calls is a number the child cannot act on. Both caps still map to `AgentOutcome::OutOfTurns`.
See decision D-grace-turn-warning.

The value lives in three types, and each has one owner:

```rust
/// The subagent default grace window, in turns.
pub const DEFAULT_SUBAGENT_GRACE_TURNS: u32 = 5;

/// `rho_core::AgentConfig`, the per-run caps. It holds `max_turns` and
/// `max_tool_calls` today, and it gains one field.
pub struct AgentConfig {
    pub max_turns: u32,
    pub max_tool_calls: u32,
    /// Turns of warning before `max_turns`. Zero disables the warning.
    pub grace_turns: u32,
}

/// `rho_core::SessionConfig`, the stated session shape. It gains the same field
/// and one builder method. `SessionConfig::new` sets it to zero, so a plain
/// session gets no warning until a caller opts in.
pub struct SessionConfig {
    pub model: String,
    pub session_root: PathBuf,
    pub approval: Arc<dyn ApprovalPolicy>,
    pub max_turns: u32,
    pub max_tool_calls: u32,
    pub sandbox: SandboxMode,
    pub queue: MessageQueue,
    /// Turns of warning before `max_turns`. Zero disables the warning.
    pub grace_turns: u32,
}

impl SessionConfig {
    /// Set the grace window. The driver enforces it.
    pub fn with_grace_turns(self, grace_turns: u32) -> Self;
}
```

`SubagentLimits::grace_turns` defaults to `DEFAULT_SUBAGENT_GRACE_TURNS`, flag
`--agent-grace-turns`. `build_child` copies it into the child's `SessionConfig`. This mirrors how
`cap_tool_calls` flows the tool-call budget into the child.

**`build_child` applies no clamp against `max_turns`, and an earlier draft said it did.** The
driver only warns at a boundary **after** the child has taken a turn, so a window wider than the
cap already fires once, in the right place. A clamp was written first, and a deliberate break
proved no test could see it, because the behaviour is identical either way. AGENTS.md step 6 says
to delete a branch no test reaches, so it is gone. The driver's `turns >= 1` rule is the guard, and
two tests pin it: `a_window_wider_than_the_cap_still_warns_once_and_not_before_the_first_turn` in
`rho-core`, and `a_grace_window_wider_than_the_child_turn_cap_still_warns_once_after_work` in
`rho-tools`.

**Three types hold the value, and the flow has one direction, so no two can disagree.** The
same three types already hold `max_tool_calls`, and the existing flow is the pattern to copy.
`SessionConfig::new` seeds its caps from `AgentConfig::default()`, at
`crates/rho-core/src/agent.rs:201`. `Session::run` then builds the driver's `AgentConfig` from
the session config, at `crates/rho-core/src/agent.rs:346`. So `SubagentLimits` states the
subagent policy, `SessionConfig` states one session's shape, and the driver's `AgentConfig`
holds the per-run caps it enforces. There is one copy site per hop, and no back edge.

**Do not collapse the field into `SubagentLimits` alone.** A subagent is not the only session
that may want a warning, and the driver enforces the cap. A single owner would make the driver
reach into a subagent type, and a plain session could never opt in.

### 4.2 The delivery path

**The warning is delivered through the steering queue.** The driver pushes it just before the
drain, and the same turn-boundary drain delivers it through `Context::append`. A child transcript
records the delivery as `TranscriptBody::Delivered { count }`, because a reader who sees a child
change course deserves to see what rho told it. So the sent prefix stays
byte-identical and the provider cache stays warm. There is one delivery point, and the
warning uses it.

**When the queue is full, the warning yields.** The push may return `QueueError::Full`,
because a user message may fill the queue at that moment. rho never drops a user message to
make room. So a full queue means the warning is skipped for now. The driver retries at the
next boundary, and it marks the warning delivered only on a successful push. The child then
hits the hard cap and reports `OutOfTurns` with whatever summary it had, exactly as today.
See decision D-grace-turn-warning.

### 4.3 Exactly one warning

**A `grace_warned` flag on the driver prevents a second warning.** The driver sets it only
when the push succeeds. A skipped push over a full queue leaves it unset, so the driver
retries. Once one warning lands, no second one ever does. The test names it:
`only_one_grace_warning_reaches_the_child`.

### 4.4 The text

The message tells the child to write its summary now. It states the true number of turns
that remain. `{remaining}` is the count computed at push time, so it never lies.

```rust
/// The verbatim grace warning. `{remaining}` is the true number of turns still
/// allowed at the moment of the push, so the text never overstates the budget.
const GRACE_MESSAGE: &str =
    "You have {remaining} turns left before rho stops you. Write your final summary \
     now. State what you did, what you did not check, and any open question. If you \
     keep working past this, rho returns the last summary you wrote.";
```

### 4.5 Whether the warning counts as a turn

**The warning is not a turn.** It is a user message delivered at a turn boundary, exactly
like a steer. It does not increment the turn counter. The child then spends real turns to
answer, and those count against `max_turns` as usual.

**The outcome keeps its meaning.** A child that writes its summary and stops ends `Done`. A
child that ignores the warning and hits the cap ends `OutOfTurns`, and the summary now holds
its wrap-up. `OutOfTurns` still means the child hit its turn cap.

### 4.6 Whether a caller can turn the warning off

**A Rust caller disables the warning with `with_grace_turns(0)`. The CLI disables it with
`--agent-grace-turns 0`.** A definition cannot set it, so a project file cannot turn off a
child's warning. The default stays 5 for a subagent, and 0 for a plain session.

## 5. Test cases

**This spec describes unwritten code.** Every test below is new. Each names the assertion it
proves. Every test uses a scripted fake provider. No network, and no `sleep`.

The slot queue, in `crates/rho-core/tests/subagent_slots.rs`. **This section is built**, and every
name below is a test that exists:
- `admit_child_over_the_per_parent_cap_queues_and_returns_an_id` — the per-parent cap queues.
- `admit_child_with_a_free_slot_starts_at_once` — the `Started` arm.
- `admission_reports_started_when_a_slot_was_free_and_queued_when_it_was_not` — both `Admission`
  variants, and every `QueuedChild` accessor.
- `admit_child_over_the_process_wide_cap_refuses_and_names_the_limit` — no cross-tree wait is
  offered, and the message names `--max-live-agents`.
- `a_queued_child_starts_when_a_slot_frees` — releasing a permit grants the next waiter.
- `every_waiter_eventually_starts_when_slots_free_one_at_a_time` — three waiters, one slot, and
  all three run. A lost wakeup would fail this and pass every other test here.
- `a_queued_child_refuses_when_the_process_wide_cap_filled_while_it_waited` —
  `Dequeued::ProcessWideFull`, and the parent continues.
- `a_process_wide_refusal_leaves_no_queued_entry_behind` — the `handed_out` flag must still be
  false on that exit, so the drop guard runs. An ordering mistake here reintroduces the leak.
- `a_refused_queued_child_releases_its_per_parent_permit` — its sibling starts straight after, so
  a waiter that gives up blocks nobody.
- `a_queued_child_starts_before_a_later_one_under_one_parent` — per-parent order is kept.
- `spawn_child_still_refuses_over_a_cap` — the bypass form never queues.
- `two_racing_starts_cannot_both_pass_a_cap_of_one` — the permit count holds under a race.
- `a_queued_child_does_not_spend_its_timeout_while_it_waits` — the clock starts at start. It
  lives in `crates/rho-tools/tests/subagent_tool.rs`, not here, because the clock is inside
  `collect_report` and only the tool path reaches it. The test pauses the tokio clock, so the
  waiter starts exactly when the first child's budget runs out.
- `cancelling_a_queued_child_resolves_started_with_cancelled` — a cancel frees the place.
- `status_says_a_cancelled_queued_child_will_not_start` — the state carries the cancel while the
  entry is still in the map. A live run read a place and a promised start after a cancel.
- `cancelling_a_parent_dequeues_every_queued_child` — three waiters resolve cancelled.
- `steering_a_queued_child_buffers_until_it_starts` — the first boundary delivers it.
- `depth_beyond_the_cap_refuses_and_never_queues` — waiting adds no depth.
- `a_cycle_refuses_and_never_queues` — planned. A cycle is unreachable through the public API,
  because every id is fresh and a parent link is stored directly. So no test can build one
  through `admit_child`. The guard itself is proved by
  `a_cycle_in_the_parent_chain_is_refused_rather_than_looping`, and `admit_child` runs it before
  it touches either wait line.
- `a_full_wait_line_refuses_and_names_the_limit` — one parent's line is bounded, and the message
  names `--max-queued-per-parent`.
- `a_full_process_wait_line_refuses_and_names_the_other_limit` — many roots, each with one waiter,
  and `--max-queued-total` stops them. A per-parent cap alone would not.
- `the_wait_line_does_not_grow_without_a_bound` — a thousand tasks do not queue a thousand.
- `a_queued_entry_leaves_the_map_when_the_child_starts` — proved through capacity and position, not
  through `status`. `status` prefers the live answer, so it would pass with a stale entry still in
  the map.
- `a_queued_entry_leaves_the_map_when_the_child_is_cancelled` — the drop guard runs.
- `a_dropped_queued_child_leaves_no_entry_behind` — a caller that never awaits leaks nothing.
- `a_process_wide_refusal_leaves_no_queued_entry_behind` — the `handed_out` flag must still be
  false on that exit. A deliberate break that set it early failed only this test.
- `the_started_child_keeps_the_id_the_caller_was_given` — a handout reuses the id, so every steer
  and cancel the model already holds still lands.
- `a_position_counts_only_its_own_siblings` — two parents with waiters. A break that counted every
  waiter in the process passed every other test.
- `one_tree_cannot_steer_another_queued_child` — the scope guard covers the steer path too.
- `a_waiter_refuses_when_its_deadline_passes_and_names_the_flag` — `Dequeued::WaitedTooLong`,
  and the text names `--queue-wait-secs` and `background: true`. The deadline differs from the
  child timeout, so the assertion can only be about the deadline.
- `the_deadline_fires_before_the_worst_case_wait` — across three limit sets, the wait ends at
  `queue_wait` and always inside
  `ceil(max_queued_per_parent / max_children_per_parent) x child_timeout`. Every set states a
  `queue_wait` that is not the `child_timeout`. The invariant, not one example.
- `a_slot_that_frees_before_the_deadline_still_starts_the_child` — the deadline breaks no
  happy path.
- `a_zero_deadline_refuses_a_waiter_at_once` — zero means no waiting, so no value turns the
  deadline off.
- `a_timed_out_waiter_leaves_no_queued_entry_behind` — the drop guard runs on the new exit
  too, which is the family of the `handed_out` defect.
- `a_timed_out_waiter_frees_its_place_in_the_line` — the child behind it moves up from two to
  one.
- `a_cancel_beats_the_deadline` — a cancelled waiter past its deadline reports `Cancelled`.
- `a_child_queue_carries_the_byte_cap_from_the_limits` — a queued child's queue refuses a
  message over the byte cap the limits state, so the flag is not a dead switch.
- `a_started_child_queue_carries_the_byte_cap_from_the_limits` — the same for the arm that
  starts at once.

The flags, in `crates/rho-cli/src/cli.rs`. A limit with no flag teaches a lie, and a flag that
changes nothing is dead surface:
- `the_queue_wait_flag_reaches_the_limits` — `--queue-wait-secs 30` sets `queue_wait`.
- `an_unset_queue_wait_follows_the_child_timeout` — with only `--child-timeout-secs 900`, the
  deadline is 900 seconds. A host that lengthens a child run lengthens the patience with it.
- `the_agent_steer_byte_flag_reaches_the_limits` — `--max-agent-steer-bytes` sets the child
  queue cap, and the default is the stated 16 KiB.

The stated defaults, in `crates/rho-core/src/subagent/limits.rs`:
- `the_queue_wait_deadline_is_one_child_timeout` — the default patience is one whole sibling
  run, and the two numbers are equal on purpose.

One defect that only a live run showed, in `crates/rho-core/src/subagent/report.rs`:
- `a_failed_label_is_a_phrase_and_not_a_sentence` — `agent_status` printed
  `--queue-wait-secs.. 0 turn(s)` with two full stops, because the phrase ended a sentence its
  caller also ended. The fix covers every failed outcome, and not only a waiter.

A queued child is addressable, and only by its owner, in `crates/rho-core/tests/subagent_slots.rs`
and `crates/rho-tools/tests/subagent_tool.rs`:
- `status_reports_a_queued_child_with_its_position` — the third variant answers.
- `a_queued_position_is_computed_and_never_stale` — the child ahead starts, and the place moves
  from two to one.
- `status_reports_running_after_a_queued_child_starts` — the state moves on start.
- `one_tree_cannot_reach_another_queued_child_by_id` — the scope guard covers the new map.
- `agent_status_answers_for_a_queued_child` — the tool path, not only the type.
- `cancel_agent_stops_a_queued_child` — the tool path, through the new queued branch.
- `steer_agent_buffers_for_a_queued_child` — the tool path accepts the message and reports where
  it sits. That the message survives the start is proved by
  `steering_a_queued_child_buffers_until_it_starts`, in the core, where a start is drivable.
- `agent_status_says_a_cancelled_queued_child_will_not_start` — the tool path never promises a
  start after a cancel, whichever answer the timing gives.

The fan-out with the queue, in `crates/rho-tools/tests/subagent_tool.rs`:
- `a_fan_out_over_the_cap_queues_the_extra_tasks_and_runs_them_all`
- `a_fan_out_reports_in_request_order_though_start_order_differs` — the first task names an agent
  that does not exist, so it never starts. The report still leads with it.
- `a_blocking_spawn_over_the_cap_waits_and_then_runs` — a `background: false` spawn queues too. It
  must not return before a slot frees, and it must run once one does.
- `a_task_over_the_process_wide_cap_is_refused_and_the_others_still_run` — this replaces the older
  per-parent-cap refusal test, whose name went with the behaviour. The per-parent cap queues now,
  so the cap that still refuses is the one a fan-out test must pin.

A child that never started still owes its parent a report, in `crates/rho-tools/src/subagent.rs`:
- `a_cancelled_waiter_is_cancelled_and_a_full_process_is_a_failure` — the outcome of a
  `Dequeued`, as a pure function. A review swapped the two arms and every other test passed,
  because only a race reaches the second arm.
- `a_waiter_that_ran_out_of_patience_is_a_failure_that_names_the_wait` — the third arm, and it
  is a failure and never a cancel, because the parent did not ask for it.
- `an_unstarted_report_claims_no_work` — zero turns, no summary, and no transcript.

Named handles, in `crates/rho-core/tests/subagent_handles.rs`. **This section is built.**
- `a_handle_is_derived_from_the_agent_name` — the first `explore` is `explore`.
- `a_second_child_of_one_name_is_numbered` — the second `explore` is `explore-2`.
- `a_handle_is_unique_per_tree_not_per_process` — two trees each hold `explore`.
- `a_queued_child_holds_a_handle_too` — a waiter is addressable by name, not only by id.
- `a_started_child_keeps_the_handle_it_was_given_while_queued` — the handout never re-derives.
- `sixteen_threads_admitting_one_agent_name_get_sixteen_distinct_handles` — the set size is the
  assertion, so the test proves the invariant rather than one lucky interleave. It fails if the
  handle is derived outside the registration lock.
- `a_handle_is_not_reused_while_a_finished_child_is_remembered` — the numbering reads all three
  indexes, so a remembered `explore` keeps its name and a newcomer takes the next.
- `resolve_reads_an_integer_id` — the old shape still resolves.
- `resolve_reads_a_digits_only_string_as_an_id` — a numeric string reaches the same child.
- `a_digits_only_name_resolves_as_an_id_and_never_as_a_handle` — an agent called `42` keeps
  its derived handle for display, and `set_alias` refuses a digits-only alias.
- `resolve_reads_a_handle_name` — a name reaches the child.
- `an_alias_resolves_to_its_child` — a set alias reaches the child.
- `an_alias_that_shadows_a_handle_is_refused` — a derived handle wins.
- `an_alias_that_is_already_taken_is_refused` — one name, one owner.
- `resolve_checks_a_handle_before_an_alias` — the shadow rule holds at read time.
- `an_alias_longer_than_the_cap_is_refused_and_the_cap_is_counted_in_characters` — 64 emoji pass.
- `an_alias_with_a_control_character_is_refused` — a newline, a bell, and an escape, so no
  forged output line.
- `an_alias_for_a_child_of_another_tree_is_unknown` — the write path is scoped too.
- `an_alias_cannot_be_set_on_a_cousin` — and it is scoped inside one tree, not only across two.
- `a_child_cannot_reach_its_uncles_child_by_handle` — the ancestor re-check, which a mutation
  proved was the only guard for this case.
- `one_tree_cannot_reach_another_by_handle` — the scope guard stands.
- `a_handle_resolves_while_the_report_is_remembered` — it outlives the live handle.
- `a_handle_stops_resolving_when_the_report_is_evicted` — the 64 cap bounds it too.
- `an_alias_goes_when_its_child_is_forgotten` — so the alias map is bounded by the same three
  indexes, and a freed name may be used again.
- `the_handle_table_holds_no_more_than_the_three_indexes` — a thousand short-lived children
  leave nothing behind.
- `an_agent_ref_reads_a_number_and_a_string` — the wire shape, both arms.
- `a_malformed_agent_ref_names_both_accepted_shapes` — a boolean, a float, a negative number,
  `null`, an object, and a list each teach what to send instead.
- `a_number_beyond_u64_is_refused_as_no_id` — serde reads it as a float, so it is not an id.
- `an_empty_agent_ref_name_is_an_ordinary_not_found`
- `every_alias_refusal_teaches_what_to_do` — all six variants are sentences that teach.

The naming rules themselves, as pure functions, in `crates/rho-core/src/subagent/handles.rs`:
- `a_free_name_is_taken_as_it_stands`
- `a_collision_counts_from_two_and_skips_what_is_held` — and it fills a hole a finished child left.
- `the_search_terminates_when_every_low_number_is_held` — the loop is bounded by the taken set.
- `a_digits_only_name_is_recognised`
- `an_alias_is_bounded_printable_and_never_digits`

The handle tools, in `crates/rho-tools/tests/subagent_tool.rs`. **This section is built.**
- `steer_agent_accepts_a_handle` — `AgentRef::Name` reaches the child.
- `steer_agent_still_accepts_an_integer_id` — the migration keeps the old shape.
- `cancel_agent_accepts_a_handle`
- `agent_status_accepts_a_handle`
- `agent_status_with_no_id_lists_this_tree_and_names_each_handle` — the call every malformed
  refusal recommends really exists.
- `agent_status_with_no_id_says_plainly_when_nothing_runs` — the empty case is not an empty string.
- `spawn_agent_sets_an_alias_from_its_argument`
- `spawn_agent_reports_a_rejected_alias_without_failing_the_spawn` — the work outlives the label.
- `a_fan_out_gives_one_name_to_one_child_and_notes_the_other` — one name, one owner.
- `an_alias_of_exactly_the_cap_is_accepted_and_one_more_is_refused` — the boundary, counted in
  characters, so 64 emoji pass and 65 do not.
- `the_schemas_offer_the_alias_and_both_id_shapes` — a model uses only what the schema shows.
- `a_malformed_agent_ref_names_both_accepted_shapes` — through a real tool call.
- `a_number_beyond_u64_is_refused_as_no_id`
- `an_empty_agent_ref_name_is_an_ordinary_not_found`

Grace turns, in `crates/rho-core/tests/subagent_grace.rs`. **This section is built**, and the
names below are the tests that exist:
- `a_child_is_warned_five_turns_before_its_cap` — the default fires at the right boundary.
- `the_warning_states_the_true_turns_remaining` — the text never overstates the budget.
- `only_one_grace_warning_reaches_the_child` — the flag prevents a second.
- `a_full_queue_at_grace_time_keeps_every_user_message` — the warning yields, never drops.
- `the_warning_is_retried_after_the_queue_drains` — a skipped warning lands later.
- `the_warning_does_not_count_as_a_turn` — the counter is unchanged.
- `a_child_that_ignores_the_warning_still_reports_out_of_turns` — the outcome keeps meaning.
- `the_tool_call_budget_gets_no_grace_warning` — the budget keeps its hard stop.
- `grace_turns_zero_disables_the_warning` — a caller turns it off.
- `a_child_with_a_zero_grace_window_is_not_warned` — the same, driven through the spawn tool.
- `a_child_is_warned_before_its_turn_cap` — a non-zero window reaches a real child.
- `a_definition_cannot_set_grace_turns` — a project file cannot change it.
- `a_plain_session_gets_no_warning_by_default` — `SessionConfig::new` sets zero.
- `the_driver_reads_the_grace_window_from_the_session_config` — the one copy site works.

## 6. What this spec forbids, and the extension points

**What it forbids.**

- A queue that grows without a bound. The wait line is capped, and the product is stated.
- A wait with no deadline. One waiter waits at most `queue_wait`, and no value turns that off.
- A queued entry that outlives its child, on any path.
- A wait on the process-wide cap, because it would let one tree hold another tree.
- A queued child that no lookup can reach, and an unscoped lookup that reaches one.
- A counter plus a notify as the wait primitive, because it loses a waiter.
- A stored place in the line, because nothing renumbers it and a wrong number misleads a model.
- A waiter that gives up while it still holds a per-parent permit.
- A blocked tool call with no id for the pending child.
- A handle that replaces the id, or that resolves across trees.
- A handle that binds to two children, including a remembered one.
- An alias that shadows a derived handle, or that is unbounded, or that holds a control
  character.
- An alias failure that fails the spawn.
- A breaking change to the `id` argument shape, and a refusal that names neither shape.
- A second grace warning, or a warning that drops a user message.
- A grace warning that overstates the turns that remain.
- A serialised `AgentStatus`, and any catch-all variant on it.

**The extension points.**

- A Rust caller bypasses the queue with `AgentNode::spawn_child`, the immediate form.
- A host tunes the queue and the grace window through `SubagentLimits` and its flags.
- A caller sets a memorable alias through `AgentRegistry::set_alias`.
- A caller disables the grace warning through `SessionConfig::with_grace_turns`.

**What is closed, on purpose.** The start order and the resolution order are policy, not
third-party surface. A priority queue edits the order rule. A second naming scheme edits
`resolve`. A per-agent grace value edits `SubagentLimits` and `build_child`. Each is a knob for
this project to turn, and none is a case a third party adds behind a trait. Saying so here is
better than implying an openness that does not exist.

## 7. Out of scope

- A global start order across two parents. Order is per parent only.
- A wait on the process-wide live cap. That cap refuses, as it does today.
- A retry of a waiter that ran out of patience. rho reports it, and the caller chooses again.
- The byte cap on one steering message. `MessageQueue::push` holds it, and `SPEC-steering`
  section 4 owns the numbers. This spec states only the child queue value.
- A handle for a nested grandchild. A child gets a handle inside its own tree only.
- A grace warning for the tool-call budget. The budget keeps its hard stop.
- A grace warning for a plain top-level session by default. It stays opt-in there.
- A definition-supplied grace value, and a definition-supplied alias. A definition is a
  project file, so only a trusted caller or the spawning model sets these. The model may name
  its own child through the `alias` argument, because a label on a child it already owns adds
  no reach.
- A persisted queued state. `AgentStatus` is never serialised.
- A priority queue. The wait line is first-in-first-out per parent.
- A `bypassQueue` argument for the model. The bypass is a Rust method, not a tool field.
