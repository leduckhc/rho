use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::cancel::CancelToken;
use crate::subagent::error::SubagentError;
use crate::subagent::limits::SubagentLimits;
use crate::subagent::report::AgentReport;
use crate::usage::Usage;

/// Identifies one agent in the spawn tree. A fresh id comes from a process-wide
/// atomic counter, so two live agents never share one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AgentId(pub u64);

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "agent-{}", self.0)
    }
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

    /// Every live child in the process, in id order. Private on purpose.
    ///
    /// A caller acts for one tree, so it gets [`AgentRegistry::live_under`]. This view
    /// used to be public behind a doc comment that said "a tool must use the scoped
    /// one", and a doc comment is not a boundary: the `agent_status` tool reached for
    /// this one first. See decision D-a-caller-addresses-only-its-own.
    fn live(&self) -> Vec<LiveAgent> {
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

    /// The handle for one live child anywhere in the process. Private on purpose.
    ///
    /// [`AgentRegistry::descendant`] is the scoped form every caller uses.
    fn handle(&self, id: AgentId) -> Option<LiveAgent> {
        self.inner
            .live
            .lock()
            .expect("the live agent lock is poisoned")
            .get(&id.0)
            .cloned()
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

    /// Start a new spawn tree, and return its root node.
    ///
    /// **Each call makes a new tree**, with a fresh id, no ancestors, and depth 0. It
    /// is not an accessor for one shared root, and the old name `root` read as if it
    /// were. One session calls this once and keeps the node.
    ///
    /// Two calls give two unrelated trees inside one registry, which is what makes a
    /// cross-tree ownership test possible, and what
    /// D-a-caller-addresses-only-its-own has to defend against.
    pub fn new_tree(&self) -> AgentNode {
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
