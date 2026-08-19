//! Subagents: the spawner, the policy composition, and the result contract.
//!
//! A subagent is another `Session` on the same runtime. The parent asks for a
//! result, a child does the reading and the trying, and only a summary comes
//! back. See `docs/specs/20260818-000223-SPEC-subagents.md`.
//!
//! This module owns the security core. A child is confined by composition, not
//! by comparison: [`BothPolicies`] allows a call only when the parent and the
//! child both allow it, so a child can only ever be more restrictive. The tool
//! set follows the same rule with [`intersect_tools`], and the sandbox mode may
//! only narrow with [`narrow_sandbox`]. See decision D-child-confined-by-composition.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use crate::{
    AgentEvent, AgentEvents, AgentStopReason, ApprovalDecision, ApprovalPolicy, CancelToken,
    SandboxMode, StreamEvent, ToolKind, Usage,
};

/// Identifies one agent in the spawn tree. A fresh id comes from a process-wide
/// atomic counter, so two live agents never share one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AgentId(pub u64);

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "agent-{}", self.0)
    }
}

/// The most characters a child summary may carry back to the parent.
///
/// The summary is the only thing the parent's context receives, so it must stay
/// small. A longer final answer is truncated to this many characters. See
/// `SPEC-subagents` section 6.
pub const MAX_SUMMARY_CHARS: usize = 8_000;

/// The most times a unit of work may die before it is reported failed.
///
/// This is jcode's reclaim cap. Without it a poisoned task loops until the
/// budget is gone. See `SPEC-subagents` section 8.
pub const MAX_CHILD_RETRIES: u32 = 3;

// --- The security core: composition, not comparison (D-child-confined-by-composition) ---

/// Allow a call only when **both** policies allow it.
///
/// This is how a child is confined. The parent's policy is always one of the two
/// conjuncts, so a child can only ever be more restrictive. Escalation is not
/// checked, it is unrepresentable. See decision D-child-confined-by-composition.
pub struct BothPolicies {
    parent: Arc<dyn ApprovalPolicy>,
    child: Arc<dyn ApprovalPolicy>,
}

impl BothPolicies {
    /// Compose a parent policy and a child policy. The parent is checked first.
    pub fn new(parent: Arc<dyn ApprovalPolicy>, child: Arc<dyn ApprovalPolicy>) -> Self {
        Self { parent, child }
    }
}

#[async_trait]
impl ApprovalPolicy for BothPolicies {
    async fn approve(
        &self,
        tool: &str,
        kind: ToolKind,
        args: &serde_json::Value,
    ) -> ApprovalDecision {
        match self.parent.approve(tool, kind, args).await {
            ApprovalDecision::Deny => ApprovalDecision::Deny,
            ApprovalDecision::Allow => self.child.approve(tool, kind, args).await,
        }
    }
}

/// The result of intersecting a child's requested tools with the parent's set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolIntersection {
    /// The tools the child keeps: the parent's set filtered by the child's list.
    pub allowed: Vec<String>,
    /// The names the child asked for that the parent does not hold. Reported to
    /// the caller so a bad definition is visible rather than silent.
    pub dropped: Vec<String>,
}

/// Intersect a child's requested tool names with the parent's set.
///
/// A child that names no tools inherits the parent's set unchanged. A name the
/// parent does not hold is dropped and reported. This removes a class of
/// privilege escalation by delegation: a child can never receive a tool its
/// parent never had. See decision D-child-confined-by-composition.
pub fn intersect_tools(parent: &[String], child_request: Option<&[String]>) -> ToolIntersection {
    let Some(requested) = child_request else {
        return ToolIntersection {
            allowed: parent.to_vec(),
            dropped: Vec::new(),
        };
    };
    let parent_set: HashSet<&str> = parent.iter().map(String::as_str).collect();
    let mut allowed = Vec::new();
    let mut dropped = Vec::new();
    for name in requested {
        if parent_set.contains(name.as_str()) {
            allowed.push(name.clone());
        } else {
            dropped.push(name.clone());
        }
    }
    ToolIntersection { allowed, dropped }
}

/// Narrow a sandbox mode. A child may ask for a stricter mode, never a weaker
/// one.
///
/// A child that names no mode inherits the parent's mode. A child that asks for
/// a mode at least as strict as the parent's gets that mode. A child that asks
/// for a weaker mode is refused, and the refusal names both modes. See `SPEC-subagents`
/// section 4.
pub fn narrow_sandbox(
    parent: SandboxMode,
    child_request: Option<SandboxMode>,
) -> Result<SandboxMode, SubagentError> {
    let Some(requested) = child_request else {
        return Ok(parent);
    };
    if sandbox_rank(requested) >= sandbox_rank(parent) {
        Ok(requested)
    } else {
        Err(SubagentError::WeakerSandbox { parent, requested })
    }
}

/// Rank the confinement strength of a sandbox mode. A higher rank is stricter.
fn sandbox_rank(mode: SandboxMode) -> u8 {
    match mode {
        SandboxMode::Off => 0,
        SandboxMode::Confined => 1,
        SandboxMode::Strict => 2,
    }
}

// --- Limits (SPEC-subagents section 7) ---

/// The four subagent limits. A child that spawns a child fans out
/// geometrically, so one limit is not enough. See `SPEC-subagents` section 7.
#[derive(Clone, Copy, Debug)]
pub struct SubagentLimits {
    /// How deep the tree may go. A depth of 0 forbids spawning.
    pub max_depth: u32,
    /// How many children one parent may run at once.
    pub max_children_per_parent: usize,
    /// How many agents may be live in the whole process, at any depth. jcode's
    /// absolute cap, and it protects the machine rather than the run.
    pub max_live_total: usize,
    /// How long a child may run before it is cancelled.
    pub child_timeout: Duration,
    /// How many tool calls one child may make.
    ///
    /// A turn cap counts provider round trips, so it does not bound a child that
    /// makes forty tool calls inside one turn. This does.
    pub max_tool_calls: u32,
}

impl SubagentLimits {
    /// The starting limits, stated here and not hidden. See decision D-no-four-argument-session-new.
    ///
    /// Depth 2, four children per parent, 32 live in total, and a ten minute
    /// child timeout.
    pub fn new() -> Self {
        Self {
            max_depth: 2,
            max_children_per_parent: 4,
            max_live_total: 32,
            child_timeout: Duration::from_secs(600),
            max_tool_calls: 64,
        }
    }
}

impl Default for SubagentLimits {
    fn default() -> Self {
        Self::new()
    }
}

// --- The result contract (SPEC-subagents section 6) ---

/// What a child returns to its parent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentReport {
    pub agent: String,
    pub outcome: AgentOutcome,
    /// The child's final answer, capped. This is what the model sees.
    pub summary: String,
    /// Every turn's usage, summed. Feeds the budget governor and the status
    /// line.
    pub usage: Usage,
    pub turns: u32,
    /// rho's verified verdict on the task. Empty when the task declared no checks.
    ///
    /// This is the truth. `claims` is what the child said. An old record with no
    /// field reads as an empty, passing report, which is correct: a task with no
    /// declared check has nothing to fail.
    #[serde(default)]
    pub gate: crate::GateReport,
    /// The child's own, unverified claims. Never a substitute for `gate`.
    #[serde(default)]
    pub claims: crate::ChildClaims,
    /// Where the full transcript was written, for a human. Never sent to the
    /// model.
    pub transcript: Option<PathBuf>,
}

/// How a child's run ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentOutcome {
    Done,
    /// The child hit its own turn cap. The summary holds what it had.
    OutOfTurns,
    /// The child was cancelled, with its parent or alone.
    Canceled,
    /// The child failed. The parent continues.
    Failed {
        reason: String,
    },
    /// The child stopped, but rho's gate failed one or more checks. It holds the
    /// failed labels.
    ///
    /// It is never `Done`, so a reader that trusts only the outcome still sees a
    /// failure. Added by `SPEC-agent-tasks`. See decision
    /// D-a-child-does-not-grade-itself.
    Rejected {
        failed: Vec<String>,
    },
}

impl AgentOutcome {
    /// Whether this outcome is a failure the parent must notice.
    ///
    /// The only place that decides. Four sites used to answer this question in their
    /// own words, so a new variant meant editing four matches and the newest one
    /// defaulted to success. Ask the type instead.
    pub fn is_failure(&self) -> bool {
        !matches!(self, Self::Done)
    }

    /// The wire name, matching this enum's own serde spelling.
    ///
    /// A transcript groups by it, so a second spelling would split one outcome into
    /// two buckets.
    pub fn wire_name(&self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::OutOfTurns => "out_of_turns",
            Self::Canceled => "canceled",
            Self::Failed { .. } => "failed",
            Self::Rejected { .. } => "rejected",
        }
    }

    /// A short phrase for a human or a model, including the reason when there is one.
    pub fn label(&self) -> String {
        match self {
            Self::Done => "done".to_string(),
            Self::OutOfTurns => "out of turns".to_string(),
            Self::Canceled => "cancelled".to_string(),
            Self::Failed { reason } => format!("failed: {reason}"),
            Self::Rejected { failed } => format!("rejected: {}", failed.join(", ")),
        }
    }
}

// --- Refusals (SPEC-subagents section 7: refusing must teach) ---

/// A refusal to spawn or run a child. Every variant names the limit, its value,
/// and what to do. See `SPEC-subagents` section 7.
#[derive(Clone, Debug, thiserror::Error)]
pub enum SubagentError {
    #[error("a task needs a goal. Say what the child must achieve, not only which agent to run.")]
    EmptyGoal,
    #[error(
        "the depth limit is {limit} and this would be depth {attempted}. \
         Do the work here. A subagent started from the rho command line holds no \
         spawn tool, so it cannot delegate further."
    )]
    DepthExceeded { limit: u32, attempted: u32 },
    #[error(
        "the per-parent child limit is {limit} and this parent already runs {current}. \
         Wait for a child to finish, or ask the user to raise --max-children-per-parent."
    )]
    TooManyChildren { limit: usize, current: usize },
    #[error(
        "the process-wide agent limit is {limit} and {current} agents are live. \
         Wait for an agent to finish, or ask the user to raise --max-live-agents."
    )]
    TooManyLiveAgents { limit: usize, current: usize },
    #[error(
        "a child may not weaken the sandbox. The parent mode is {parent} and the child \
         asked for {requested}. Ask for {parent} or a stricter mode."
    )]
    WeakerSandbox {
        parent: SandboxMode,
        requested: SandboxMode,
    },
    #[error(
        "a cycle was found in the parent chain, so the spawn is refused rather than looped. \
         This needs a bug fix, not a retry."
    )]
    CycleDetected,
    #[error(
        "the work failed {deaths} times, at the retry limit of {limit}. \
         Report the failure to the user rather than retry the same work."
    )]
    RetryCapReached { deaths: u32, limit: u32 },
}

// --- A finished child stays reportable (background children) ---

/// What a caller learns when it asks about one child.
///
/// A live handle disappears when `ChildSlot` drops, so a background child that
/// finished would otherwise vanish before the parent asked. jcode keeps a
/// `latest_completion_report` for the same reason, and pi keeps the record until the
/// result is consumed.
#[derive(Clone, Debug)]
pub enum AgentStatus {
    /// Still working. The numbers come from the live handle.
    Running {
        agent: String,
        depth: u32,
        progress: AgentProgress,
        queued: usize,
    },
    /// Finished. The report is the same one a blocking spawn would have returned.
    Finished { report: AgentReport },
}

/// How many finished reports the registry keeps.
///
/// Bounded, because a report holds a summary a model wrote. An unbounded map keyed by
/// model output is the shape that already cost this project 805 MB once. See decision
/// D-bash-line-cap.
const MAX_REMEMBERED_REPORTS: usize = 64;

/// One finished child, kept so a parent can still ask about it.
///
/// `AgentReport` carries no id, so the id is kept beside it. A lookup by ancestors
/// alone would answer with some other child's report, which is worse than answering
/// nothing.
#[derive(Clone, Debug)]
struct FinishedAgent {
    id: AgentId,
    ancestors: Vec<AgentId>,
    report: AgentReport,
}

// --- A live child is addressable (SPEC-subagents section 7a) ---

/// What a running child has done so far.
///
/// A frontend reads this to render a live child. It carries no transcript, so
/// watching a child costs the parent no context.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentProgress {
    /// Turns the child has started.
    pub turns: u32,
    /// The child's usage, summed so far.
    pub usage: Usage,
}

/// A live, addressable child.
///
/// The registry used to count children without knowing them, so nothing could
/// address a single child. Cancelling one child, steering one child, and watching
/// one child were three missing features with one cause. This is that missing
/// contract.
#[derive(Clone, Debug)]
pub struct LiveAgent {
    /// The child's id.
    pub id: AgentId,
    /// The agent definition it runs.
    pub agent: String,
    /// How deep it sits in the spawn tree.
    pub depth: u32,
    /// Every node above this child, from the root down to its parent.
    ///
    /// It is what makes ownership checkable. A caller may address a child only when
    /// its own id appears here. See decision D-a-caller-addresses-only-its-own.
    ancestors: Vec<AgentId>,
    cancel: CancelToken,
    progress: tokio::sync::watch::Receiver<AgentProgress>,
    queue: crate::MessageQueue,
}

impl LiveAgent {
    /// Stop this child, and only this child.
    ///
    /// The token is derived from the parent's, so it cancels downward and never
    /// upward. A sibling is untouched. See [`CancelToken::child`].
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Whether this child is cancelled, including by an ancestor.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// The child's own token, for a caller that needs to await it.
    pub fn cancel_token(&self) -> CancelToken {
        self.cancel.clone()
    }

    /// What the child has done so far.
    pub fn progress(&self) -> AgentProgress {
        self.progress.borrow().clone()
    }

    /// Send this child a message, to arrive at its next turn boundary.
    ///
    /// The child reads it as the next user message. It never lands inside a
    /// provider request, and it never rewrites an already-sent turn, so the child's
    /// prompt prefix stays stable. See `SPEC-steering` section 3.
    ///
    /// A full queue is a typed error, never a silent drop. A message queued after
    /// the child finished stays in the queue and is never delivered, because the
    /// child has no next turn. So a caller should check the child is still live.
    pub fn steer(&self, message: Vec<crate::ContentBlock>) -> Result<usize, crate::QueueError> {
        self.queue.push(message)
    }

    /// How many messages wait for this child's next turn boundary.
    pub fn queued(&self) -> usize {
        self.queue.len()
    }
}

/// Everything a caller gets when it spawns a child.
///
/// A named struct, not a tuple, because it already carries three things and a
/// tuple would grow silently. See decision D-no-four-argument-session-new.
#[derive(Debug)]
pub struct ChildSpawn {
    /// The child's place in the tree.
    pub node: AgentNode,
    /// The reservation. Dropping it frees the slot and deregisters the handle.
    pub slot: ChildSlot,
    progress: tokio::sync::watch::Sender<AgentProgress>,
    queue: crate::MessageQueue,
}

impl ChildSpawn {
    /// The queue this child must read.
    ///
    /// The caller passes it to `Session::with_queue`, so the handle and the child
    /// hold one queue. Without that, a steer would push into a queue nobody drains.
    pub fn queue(&self) -> crate::MessageQueue {
        self.queue.clone()
    }

    /// The sender `collect_report` publishes through, so progress is live.
    pub fn progress_sender(&self) -> tokio::sync::watch::Sender<AgentProgress> {
        self.progress.clone()
    }

    /// Publish the child's progress, so a frontend can render it live.
    pub fn publish(&self, progress: AgentProgress) {
        // A send fails only when every receiver is gone, and that is not an
        // error: it means nobody is watching.
        let _ = self.progress.send(progress);
    }
}

// --- The spawn tree and its guards ---

/// Process-wide subagent state, shared by every session in one process.
///
/// It holds the limits, the count of live agents, and the id allocator. A clone
/// shares the same state, so a child created from any node counts against the
/// same process-wide cap. See `SPEC-subagents` section 7.
#[derive(Clone, Debug)]
pub struct AgentRegistry {
    inner: Arc<RegistryInner>,
}

#[derive(Debug)]
struct RegistryInner {
    limits: SubagentLimits,
    live_total: AtomicUsize,
    next_id: AtomicU64,
    /// Every live child, by id. A count cannot be cancelled or watched, so the
    /// registry holds the handles too.
    live: Mutex<std::collections::HashMap<u64, LiveAgent>>,
    /// The reports of children that finished, oldest first, bounded.
    ///
    /// A background child finishes while the parent is busy, so its outcome has to
    /// outlive its handle. The ancestor chain is kept beside the report, because the
    /// handle that carried it is gone and a read still has to be scoped.
    finished: Mutex<std::collections::VecDeque<FinishedAgent>>,
}

impl AgentRegistry {
    /// Build a registry with the given limits.
    pub fn new(limits: SubagentLimits) -> Self {
        Self {
            inner: Arc::new(RegistryInner {
                limits,
                live_total: AtomicUsize::new(0),
                next_id: AtomicU64::new(0),
                live: Mutex::new(std::collections::HashMap::new()),
                finished: Mutex::new(std::collections::VecDeque::new()),
            }),
        }
    }

    /// The limits this registry enforces.
    pub fn limits(&self) -> &SubagentLimits {
        &self.inner.limits
    }

    /// The number of agents live in the whole process now.
    pub fn live_total(&self) -> usize {
        self.inner.live_total.load(Ordering::SeqCst)
    }

    /// Every live child in the process, in id order.
    ///
    /// **Process-wide, and unscoped.** It is for a host that owns the process. A tool
    /// must use [`AgentRegistry::live_under`], because a tool acts for one session and
    /// must not see another session's children.
    ///
    /// The list holds no transcript, so reading it costs the parent no context.
    pub fn live(&self) -> Vec<LiveAgent> {
        let live = self
            .inner
            .live
            .lock()
            .expect("the live agent lock is poisoned");
        let mut handles: Vec<LiveAgent> = live.values().cloned().collect();
        handles.sort_by_key(|handle| handle.id.0);
        handles
    }

    /// Every live descendant of `caller`, in id order.
    ///
    /// **A tool uses this, never [`AgentRegistry::live`].** The registry is
    /// process-wide, so `live` shows every session's children. A security review
    /// proved that resolving a bare id against the whole registry let one tree
    /// cancel another tree's child. See decision D-a-caller-addresses-only-its-own.
    pub fn live_under(&self, caller: &AgentNode) -> Vec<LiveAgent> {
        self.live()
            .into_iter()
            .filter(|handle| handle.ancestors.contains(&caller.id()))
            .collect()
    }

    /// The handle for one live child, but only when `caller` is above it.
    ///
    /// It returns `None` for a child of another tree, exactly as it does for a child
    /// that already finished. A caller cannot tell the two apart, and it does not
    /// need to: neither is addressable.
    pub fn descendant(&self, caller: &AgentNode, id: AgentId) -> Option<LiveAgent> {
        self.handle(id)
            .filter(|handle| handle.ancestors.contains(&caller.id()))
    }

    /// Cancel one descendant of `caller`, and only that one.
    ///
    /// It returns false when the id names no live child of this caller, including a
    /// child that belongs to another tree.
    pub fn cancel_descendant(&self, caller: &AgentNode, id: AgentId) -> bool {
        match self.descendant(caller, id) {
            Some(handle) => {
                handle.cancel();
                true
            }
            None => false,
        }
    }

    /// The handle for one live child, if it is still running.
    ///
    /// **Process-wide, and unscoped.** It is for a host that owns the whole process,
    /// such as a terminal frontend rendering every session. A tool must use
    /// [`AgentRegistry::descendant`] instead, because a tool acts for one session.
    pub fn handle(&self, id: AgentId) -> Option<LiveAgent> {
        self.inner
            .live
            .lock()
            .expect("the live agent lock is poisoned")
            .get(&id.0)
            .cloned()
    }

    /// Cancel one child anywhere in the process, and only that child.
    ///
    /// **Process-wide, and unscoped.** A tool must use
    /// [`AgentRegistry::cancel_descendant`]. This one is for a host that owns the
    /// process, for example to stop everything on shutdown.
    ///
    /// It returns false when no live child holds that id, which happens whenever the
    /// child finished first. That is a result, not a fault.
    pub fn cancel(&self, id: AgentId) -> bool {
        match self.handle(id) {
            Some(handle) => {
                handle.cancel();
                true
            }
            None => false,
        }
    }

    /// Remember a finished child's report, so a parent can still read it.
    ///
    /// The oldest report is dropped at the cap. Losing an old report is safe, because
    /// the parent was told the id and the transcript path when the child started.
    pub fn record_report(&self, caller: &AgentNode, id: AgentId, report: AgentReport) {
        let mut ancestors = self
            .handle(id)
            .map(|handle| handle.ancestors.clone())
            .unwrap_or_default();
        if ancestors.is_empty() {
            // The handle is already gone, so trust the caller's own chain.
            ancestors = vec![caller.id()];
        }
        let mut finished = self
            .inner
            .finished
            .lock()
            .expect("the finished agent lock is poisoned");
        if finished.len() >= MAX_REMEMBERED_REPORTS {
            finished.pop_front();
        }
        finished.push_back(FinishedAgent {
            id,
            ancestors,
            report,
        });
    }

    /// What one child is doing, or what it did.
    ///
    /// It answers for a live child and for one that finished, and only for a
    /// descendant of `caller`. See decision D-a-caller-addresses-only-its-own.
    pub fn status(&self, caller: &AgentNode, id: AgentId) -> Option<AgentStatus> {
        if let Some(handle) = self.descendant(caller, id) {
            return Some(AgentStatus::Running {
                agent: handle.agent.clone(),
                depth: handle.depth,
                progress: handle.progress(),
                queued: handle.queued(),
            });
        }
        let finished = self
            .inner
            .finished
            .lock()
            .expect("the finished agent lock is poisoned");
        finished
            .iter()
            .rev()
            .find(|entry| entry.id == id && entry.ancestors.contains(&caller.id()))
            .map(|entry| AgentStatus::Finished {
                report: entry.report.clone(),
            })
    }

    /// Register a live child. Called by `spawn_child`, which cannot forget.
    fn register(&self, handle: LiveAgent) {
        self.inner
            .live
            .lock()
            .expect("the live agent lock is poisoned")
            .insert(handle.id.0, handle);
    }

    /// Deregister a child. Called by `ChildSlot::drop`, which cannot forget.
    fn deregister(&self, id: AgentId) {
        self.inner
            .live
            .lock()
            .expect("the live agent lock is poisoned")
            .remove(&id.0);
    }

    /// Allocate a fresh, unique agent id.
    fn allocate_id(&self) -> AgentId {
        AgentId(self.inner.next_id.fetch_add(1, Ordering::SeqCst))
    }

    /// The root node of a spawn tree. Depth 0, no ancestors, no parent.
    pub fn root(&self) -> AgentNode {
        AgentNode {
            id: self.allocate_id(),
            depth: 0,
            ancestors: Vec::new(),
            registry: self.clone(),
            children: Arc::new(AtomicUsize::new(0)),
        }
    }
}

/// One agent's place in the spawn tree.
///
/// The root session holds the root node. A successful [`AgentNode::spawn_child`]
/// returns a child node and a live-agent guard. The guard decrements both the
/// per-parent count and the process-wide count when it drops, so a finished
/// child frees its slot.
#[derive(Debug)]
pub struct AgentNode {
    id: AgentId,
    depth: u32,
    ancestors: Vec<AgentId>,
    registry: AgentRegistry,
    children: Arc<AtomicUsize>,
}

impl AgentNode {
    /// This node's id.
    pub fn id(&self) -> AgentId {
        self.id
    }

    /// The registry this node belongs to.
    ///
    /// A caller needs it to list the live children, to steer one, or to cancel
    /// one. See `SPEC-subagents` section 7a.
    pub fn registry(&self) -> &AgentRegistry {
        &self.registry
    }

    /// This node's depth. The root is depth 0.
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// The number of live children this node runs now.
    pub fn live_children(&self) -> usize {
        self.children.load(Ordering::SeqCst)
    }

    /// The limits this node's tree enforces.
    pub fn limits(&self) -> SubagentLimits {
        *self.registry.limits()
    }

    /// Reserve a slot for a new child, or refuse and name the limit.
    ///
    /// It checks, in order: the depth limit, the per-parent child limit, the
    /// process-wide live cap, and the cycle guard. On success it returns the new
    /// child node and a guard. The guard holds both reserved counts and frees
    /// them on drop. So a child that finishes, fails, or is cancelled always
    /// frees its slot.
    /// `agent` names the definition, and `cancel` is the child's own token. Both
    /// are required, because a handle without them can be neither rendered nor
    /// stopped. Derive the token with [`CancelToken::child`], so cancelling the
    /// child never cancels the parent.
    pub fn spawn_child(
        &self,
        agent: impl Into<String>,
        cancel: CancelToken,
    ) -> Result<ChildSpawn, SubagentError> {
        let limits = self.registry.limits();
        let child_depth = self.depth + 1;
        if child_depth > limits.max_depth {
            return Err(SubagentError::DepthExceeded {
                limit: limits.max_depth,
                attempted: child_depth,
            });
        }

        // Reserve the per-parent slot with a compare-and-swap loop, for the same
        // reason as the process-wide slot below.
        //
        // The comment here used to claim the read and the add happened "under the
        // atomic", and they did not: a load, then a check, then a later `fetch_add`
        // leaves a window where two racing spawns both pass a cap of one. A review
        // found the false comment. The loop makes the comment true rather than
        // deleting it.
        loop {
            let current_children = self.children.load(Ordering::SeqCst);
            if current_children >= limits.max_children_per_parent {
                return Err(SubagentError::TooManyChildren {
                    limit: limits.max_children_per_parent,
                    current: current_children,
                });
            }
            if self
                .children
                .compare_exchange(
                    current_children,
                    current_children + 1,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                break;
            }
        }

        // Reserve the process-wide slot with a compare-and-swap loop, so two
        // parents cannot both pass a cap of one. This is the case a per-parent
        // cap misses.
        let live = &self.registry.inner.live_total;
        loop {
            let current = live.load(Ordering::SeqCst);
            if current >= limits.max_live_total {
                // The per-parent slot is already held, so release it before refusing.
                self.children.fetch_sub(1, Ordering::SeqCst);
                return Err(SubagentError::TooManyLiveAgents {
                    limit: limits.max_live_total,
                    current,
                });
            }
            if live
                .compare_exchange(current, current + 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                break;
            }
        }

        let mut ancestors = self.ancestors.clone();
        ancestors.push(self.id);
        // The tree cannot cycle by construction, so a duplicate id means a bug.
        // The guard costs one small set and stops an infinite loop inside a lock.
        if let Err(error) = check_no_cycle(&ancestors) {
            // Release both reserved slots before the refusal returns.
            self.children.fetch_sub(1, Ordering::SeqCst);
            live.fetch_sub(1, Ordering::SeqCst);
            return Err(error);
        }

        let child = AgentNode {
            id: self.registry.allocate_id(),
            depth: child_depth,
            ancestors,
            registry: self.registry.clone(),
            children: Arc::new(AtomicUsize::new(0)),
        };
        // Register here, and deregister in `ChildSlot::drop`. Both sides live in
        // this file, so a caller cannot forget either one.
        let (progress_tx, progress_rx) = tokio::sync::watch::channel(AgentProgress::default());
        let queue = crate::MessageQueue::new();
        self.registry.register(LiveAgent {
            id: child.id,
            agent: agent.into(),
            depth: child.depth,
            ancestors: child.ancestors.clone(),
            cancel,
            progress: progress_rx,
            queue: queue.clone(),
        });

        let slot = ChildSlot {
            parent_children: Arc::clone(&self.children),
            registry: self.registry.clone(),
            id: child.id,
        };
        Ok(ChildSpawn {
            node: child,
            slot,
            progress: progress_tx,
            queue,
        })
    }
}

/// A live-agent reservation. It holds one per-parent slot and one process-wide
/// slot. Both free when it drops, so a finished child always frees its slot.
#[derive(Debug)]
pub struct ChildSlot {
    parent_children: Arc<AtomicUsize>,
    registry: AgentRegistry,
    /// The child this slot holds, so dropping the slot drops the handle too.
    id: AgentId,
}

impl Drop for ChildSlot {
    fn drop(&mut self) {
        self.parent_children.fetch_sub(1, Ordering::SeqCst);
        self.registry
            .inner
            .live_total
            .fetch_sub(1, Ordering::SeqCst);
        // The handle goes with the slot. A registry that kept a finished child
        // would leak, and it would hand out a handle that cancels nothing.
        self.registry.deregister(self.id);
    }
}

/// Walk an ancestor chain and refuse a cycle.
///
/// The tree cannot cycle by construction, because the parent link is stored
/// directly and every id is fresh. So a duplicate id means a bug, not a race.
/// The visited set costs little and stops an infinite loop inside a lock. See
/// `SPEC-subagents` section 7.
pub fn check_no_cycle(ancestors: &[AgentId]) -> Result<(), SubagentError> {
    let mut seen = HashSet::with_capacity(ancestors.len());
    for id in ancestors {
        if !seen.insert(*id) {
            return Err(SubagentError::CycleDetected);
        }
    }
    Ok(())
}

/// Cap a child's tool-call budget by its parent's.
///
/// A definition may ask for less. It may never ask for more, exactly like the turn
/// cap. Otherwise a definition file could raise its own budget, and a budget a
/// child can raise is not a budget. See `SPEC-subagents` section 4.
pub fn cap_tool_calls(parent: u32, requested: Option<u32>) -> u32 {
    match requested {
        Some(asked) => asked.min(parent),
        None => parent,
    }
}

/// Tracks how many times a unit of work has died, keyed by a work key.
///
/// This is jcode's reclaim cap. When a caller re-delegates the same work and the
/// child dies again, the count grows. After [`MAX_CHILD_RETRIES`] the work is
/// reported failed rather than retried. See `SPEC-subagents` section 8.
pub struct RetryLedger {
    deaths: Mutex<std::collections::HashMap<String, u32>>,
    cap: u32,
}

impl RetryLedger {
    /// A ledger with the default retry cap.
    pub fn new() -> Self {
        Self {
            deaths: Mutex::new(std::collections::HashMap::new()),
            cap: MAX_CHILD_RETRIES,
        }
    }

    /// How many distinct work keys the ledger remembers.
    ///
    /// The map is bounded because its key holds the whole prompt, which a model
    /// writes. A parent that fails many distinct tasks would otherwise grow it
    /// without limit, and this project has already shipped one unbounded buffer. See
    /// decision D-bash-line-cap.
    const MAX_TRACKED_WORK: usize = 256;

    /// How many distinct work keys the ledger holds now.
    pub fn tracked(&self) -> usize {
        self.deaths
            .lock()
            .expect("the retry ledger lock is poisoned")
            .len()
    }

    /// Record one death for `key`.
    ///
    /// It returns the death count while the work may still retry. It returns a
    /// `RetryCapReached` error once the count reaches the cap, so the caller
    /// reports the work failed rather than retry a poisoned task forever.
    pub fn record_death(&self, key: &str) -> Result<u32, SubagentError> {
        let mut deaths = self
            .deaths
            .lock()
            .expect("the retry ledger lock is poisoned");
        // Forget the oldest tracking when the map is full. Losing a count is safe:
        // the work simply gets its retries again. Growing without a bound is not.
        if deaths.len() >= Self::MAX_TRACKED_WORK && !deaths.contains_key(key) {
            deaths.clear();
        }
        let count = deaths.entry(key.to_string()).or_insert(0);
        *count += 1;
        if *count >= self.cap {
            Err(SubagentError::RetryCapReached {
                deaths: *count,
                limit: self.cap,
            })
        } else {
            Ok(*count)
        }
    }
}

impl Default for RetryLedger {
    fn default() -> Self {
        Self::new()
    }
}

// --- Running a child to a report (SPEC-subagents section 6) ---

/// Drive a child's event stream to an [`AgentReport`].
///
/// It sums usage, counts turns, keeps the child's final answer as the capped
/// summary, and writes the full transcript to `transcript_path`. The transcript
/// never reaches the model, only the summary does. See `SPEC-subagents` section 6.
///
/// A child that passes `timeout` is cancelled through the shared token and
/// reported `Canceled`. A child that ends without a report is reported `Failed`.
/// A child failure is a result, not the end of the parent's run. See decision
/// D-measured-cost-and-cache.
/// What [`collect_report`] needs besides the stream.
///
/// A struct, not three more parameters. The argument list already carried three
/// things, and a fourth and fifth would repeat the mistake in decision
/// D-no-four-argument-session-new. A new need arrives as a new field with a default.
#[derive(Default)]
pub struct CollectOptions {
    /// How long the child may run before it is cancelled.
    pub timeout: Duration,
    /// Where to write the child's full transcript, for a human.
    pub transcript_path: Option<PathBuf>,
    /// Where to publish progress **as the child works**.
    ///
    /// Without this, a handle read zero for the whole run and then jumped to the
    /// final number. That is a post-mortem, and the event is called
    /// `AgentProgressed`. See `SPEC-subagents` section 9.
    pub progress: Option<tokio::sync::watch::Sender<AgentProgress>>,
}

impl CollectOptions {
    /// The options with a timeout and nothing else.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            ..Self::default()
        }
    }

    /// Write the transcript here.
    pub fn transcript(mut self, path: PathBuf) -> Self {
        self.transcript_path = Some(path);
        self
    }

    /// Publish progress here, turn by turn.
    pub fn publishing(mut self, sender: tokio::sync::watch::Sender<AgentProgress>) -> Self {
        self.progress = Some(sender);
        self
    }
}

pub async fn collect_report(
    agent: impl Into<String>,
    mut events: AgentEvents,
    cancel: CancelToken,
    options: CollectOptions,
) -> AgentReport {
    let CollectOptions {
        timeout,
        transcript_path,
        progress,
    } = options;
    let agent = agent.into();
    let mut usage = Usage::default();
    let mut turns = 0u32;
    let mut current_text = String::new();
    let mut last_answer: Option<String> = None;
    // A streaming writer, not a buffer. The run a transcript is most wanted for is
    // the one that did not finish, and a buffer loses exactly that one. A writer that
    // cannot open is `None`: a transcript is a convenience and must never fail a run.
    let transcript = transcript_path.clone().and_then(|path| {
        crate::TranscriptWriter::new(&path)
            .map_err(|error| {
                tracing::warn!(path = %path.display(), "cannot open the child transcript: {error}");
            })
            .ok()
    });
    let mut outcome: Option<AgentOutcome> = None;

    let sleep = tokio::time::sleep(timeout);
    tokio::pin!(sleep);

    loop {
        tokio::select! {
            biased;
            () = &mut sleep => {
                // The child passed its timeout. Cancel it through the shared
                // token, then report what it had.
                cancel.cancel();
                outcome = Some(AgentOutcome::Canceled);
                break;
            }
            item = events.next() => {
                match item {
                    Some(Ok(event)) => {
                        if let Some(writer) = transcript.as_ref() {
                            // `StreamEvent::TextEnd` carries only an index, so the
                            // text comes from what this loop already accumulated.
                            let body = match &event {
                                AgentEvent::TurnStart => Some(crate::TranscriptBody::TurnStart {
                                    turn: turns + 1,
                                }),
                                AgentEvent::Stream(StreamEvent::TextEnd { .. })
                                    if !current_text.is_empty() =>
                                {
                                    Some(crate::TranscriptBody::Text {
                                        text: current_text.clone(),
                                    })
                                }
                                AgentEvent::ToolStart { id, name, .. } => {
                                    Some(crate::TranscriptBody::ToolStart {
                                        id: id.clone(),
                                        name: name.clone(),
                                    })
                                }
                                AgentEvent::ToolUpdate { id, output } => {
                                    Some(crate::TranscriptBody::ToolUpdate {
                                        id: id.clone(),
                                        line: output.clone(),
                                    })
                                }
                                AgentEvent::ToolEnd { id, output } => {
                                    Some(crate::TranscriptBody::ToolEnd {
                                        id: id.clone(),
                                        error: output.is_error,
                                    })
                                }
                                AgentEvent::Stream(StreamEvent::Usage(reported)) => {
                                    Some(crate::TranscriptBody::Usage {
                                        input: reported.input_tokens,
                                        output: reported.output_tokens,
                                    })
                                }
                                _ => None,
                            };
                            if let Some(body) = body {
                                let _ = writer
                                    .write(&crate::TranscriptEntry::now(&agent, body))
                                    .await;
                            }
                        }
                        match &event {
                            AgentEvent::TurnStart => {
                                turns += 1;
                                current_text.clear();
                                // Publish as the child works, not once at the end. A
                                // failed send means nobody is watching.
                                if let Some(sender) = &progress {
                                    let _ = sender.send(AgentProgress {
                                        turns,
                                        usage,
                                    });
                                }
                            }
                            AgentEvent::Stream(StreamEvent::TextDelta { delta, .. }) => {
                                current_text.push_str(delta);
                            }
                            AgentEvent::Stream(StreamEvent::TextEnd { .. })
                                if !current_text.is_empty() =>
                            {
                                last_answer = Some(std::mem::take(&mut current_text));
                            }
                            AgentEvent::Stream(StreamEvent::Usage(reported)) => {
                                usage.add(reported);
                                if let Some(sender) = &progress {
                                    let _ = sender.send(AgentProgress { turns, usage });
                                }
                            }
                            AgentEvent::AgentEnd { stop_reason } => {
                                outcome = Some(outcome_from_stop(*stop_reason));
                                break;
                            }
                            _ => {}
                        }
                    }
                    // A transport or provider fault. A child failure is a result.
                    Some(Err(error)) => {
                        outcome = Some(AgentOutcome::Failed {
                            reason: error.to_string(),
                        });
                        break;
                    }
                    // The stream ended with no `AgentEnd`. The child died holding
                    // work. Silence is the failure mode that wastes the most time.
                    None => break,
                }
            }
        }
    }

    let outcome = outcome.unwrap_or_else(|| AgentOutcome::Failed {
        reason: "the child ended without a report.".to_string(),
    });
    let summary = cap_summary(last_answer.unwrap_or_default());
    // One last line, naming the outcome, then hand back the path. The path is `Some`
    // only when the file really opened, so a caller is never pointed at nothing.
    let transcript = match transcript.as_ref() {
        Some(writer) => {
            let _ = writer
                .write(&crate::TranscriptEntry::now(
                    &agent,
                    crate::TranscriptBody::End {
                        outcome: outcome.wire_name().to_string(),
                    },
                ))
                .await;
            Some(writer.path.clone())
        }
        None => None,
    };
    let _ = transcript_path;

    AgentReport {
        agent,
        outcome,
        summary,
        usage,
        turns,
        // `collect_report` watches a stream. It does not run the gate, because a
        // gate needs a sandboxed command runner that `rho-core` must not hold. A
        // caller runs the gate and merges the verdict.
        gate: crate::GateReport::default(),
        claims: crate::ChildClaims::default(),
        transcript,
    }
}

/// Map a run's stop reason onto a child outcome.
fn outcome_from_stop(stop_reason: AgentStopReason) -> AgentOutcome {
    match stop_reason {
        AgentStopReason::EndTurn => AgentOutcome::Done,
        AgentStopReason::MaxTurnRequests => AgentOutcome::OutOfTurns,
        // The child spent its tool-call budget. It is out of room, like a child
        // out of turns, so the parent gets what it had rather than nothing.
        AgentStopReason::MaxToolCalls => AgentOutcome::OutOfTurns,
        AgentStopReason::Canceled => AgentOutcome::Canceled,
        AgentStopReason::MaxTokens => AgentOutcome::Failed {
            reason: "the child hit the token limit.".to_string(),
        },
        AgentStopReason::Refusal => AgentOutcome::Failed {
            reason: "the model refused, or a content filter stopped the output.".to_string(),
        },
    }
}

/// Truncate a summary to [`MAX_SUMMARY_CHARS`] characters. It cuts on a character
/// boundary, so a multi-byte character never splits.
fn cap_summary(mut summary: String) -> String {
    if summary.chars().count() <= MAX_SUMMARY_CHARS {
        return summary;
    }
    let cut = summary
        .char_indices()
        .nth(MAX_SUMMARY_CHARS)
        .map(|(index, _)| index)
        .unwrap_or(summary.len());
    summary.truncate(cut);
    summary
}
