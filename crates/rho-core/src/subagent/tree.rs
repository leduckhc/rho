use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::cancel::CancelToken;
use crate::subagent::error::SubagentError;
use crate::subagent::handles::{
    AgentRef, AliasError, derive_handle, is_digits_only, validate_alias,
};
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
    /// Queued for a slot. It has an id, and it has not started.
    ///
    /// The place is computed on read, never stored, because the children ahead leave
    /// and nothing would renumber a stored one. See
    /// `SPEC-subagent-slots-handles-grace` section 2.2b.
    Queued {
        agent: String,
        depth: u32,
        /// The place in the parent's wait line, counted from one.
        position: usize,
        /// Whether this child, or an ancestor, was cancelled while it waited.
        ///
        /// A cancel wakes the waiting task, and the entry leaves the map when that task
        /// drops it. In between, a reader that ignored this said the child had a place
        /// and would start. Both were false, and a live run proved it. See decision
        /// D-a-cancelled-waiter-says-so.
        cancelled: bool,
    },
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
    next_id: AtomicU64,
    /// The process-wide live cap, as permits. One permit is one live child.
    ///
    /// A counter cannot be waited on. A counter plus a notify loses a wake that fires
    /// between a failed retry and the next await, and the safety tests still pass
    /// while liveness breaks. See decision D-permits-not-counters.
    live_permits: Arc<tokio::sync::Semaphore>,
    /// Every index, under one lock, and never held across an await.
    ///
    /// The move from queued to live has to be one step. With a lock per index a
    /// lookup can find an id in both or in neither, a cancel can report failure while
    /// the child starts, and two admissions can derive one handle. See decision
    /// D-one-registry-state-lock.
    state: Mutex<RegistryState>,
}

#[derive(Debug, Default)]
struct RegistryState {
    /// Every live child, by id. A count cannot be cancelled or watched, so the
    /// registry holds the handles too.
    live: std::collections::HashMap<u64, LiveAgent>,
    /// Every child that holds an id and no slot.
    queued: std::collections::HashMap<u64, QueuedEntry>,
    /// The next wait-line sequence, for the whole registry. `position` filters by
    /// parent, and a subset of a monotonic sequence keeps its order.
    next_sequence: u64,
    /// The reports of children that finished, oldest first, bounded.
    ///
    /// A background child finishes while the parent is busy, so its outcome has to
    /// outlive its handle. The ancestor chain is kept beside the report, because the
    /// handle that carried it is gone and a read still has to be scoped.
    finished: std::collections::VecDeque<FinishedAgent>,
    /// The derived handle of every addressable child, by id.
    ///
    /// It is the union of the three indexes above, and nothing else, so it cannot
    /// outgrow them. A name is derived and stored in the same critical section as the
    /// registration, or two admissions of one agent name could pick one name. See
    /// decision D-one-registry-state-lock.
    handles: std::collections::HashMap<u64, HandleBinding>,
    /// Every caller-chosen alias, by tree and name.
    ///
    /// Keyed by the tree root, because a name belongs to one session and two sessions
    /// must be able to use the same one. It is pruned with its child.
    aliases: std::collections::HashMap<(AgentId, String), AgentId>,
}

/// A child's derived handle, and the tree it belongs to.
///
/// The tree is stored, not computed, because the ancestor chain lives on the live
/// handle or the queued entry and a remembered child has neither for long.
#[derive(Clone, Debug)]
struct HandleBinding {
    tree: AgentId,
    name: String,
}

/// A queued child's entry. The ancestor chain sits beside it, exactly as the
/// finished ring stores it, because the live handle that would carry it does not
/// exist yet. A lookup without the chain would let one tree reach another tree's
/// queued child. See decision D-a-caller-addresses-only-its-own.
#[derive(Clone, Debug)]
struct QueuedEntry {
    agent: String,
    depth: u32,
    ancestors: Vec<AgentId>,
    cancel: CancelToken,
    queue: crate::MessageQueue,
    /// The immediate parent, so siblings are identifiable from the entry alone.
    parent: AgentId,
    sequence: u64,
}

impl AgentRegistry {
    /// Build a registry with the given limits.
    pub fn new(limits: SubagentLimits) -> Self {
        Self {
            inner: Arc::new(RegistryInner {
                live_permits: Arc::new(tokio::sync::Semaphore::new(limits.max_live_total)),
                limits,
                next_id: AtomicU64::new(0),
                state: Mutex::new(RegistryState::default()),
            }),
        }
    }

    /// The limits this registry enforces.
    pub fn limits(&self) -> &SubagentLimits {
        &self.inner.limits
    }

    /// The number of agents live in the whole process now.
    pub fn live_total(&self) -> usize {
        self.state().live.len()
    }

    /// The one lock. Never hold it across an await.
    fn state(&self) -> std::sync::MutexGuard<'_, RegistryState> {
        self.inner
            .state
            .lock()
            .expect("the registry state lock is poisoned")
    }

    /// Every live child in the process, in id order. Private on purpose.
    ///
    /// A caller acts for one tree, so it gets [`AgentRegistry::live_under`]. This view
    /// used to be public behind a doc comment that said "a tool must use the scoped
    /// one", and a doc comment is not a boundary: the `agent_status` tool reached for
    /// this one first. See decision D-a-caller-addresses-only-its-own.
    fn live(&self) -> Vec<LiveAgent> {
        let mut handles: Vec<LiveAgent> = self.state().live.values().cloned().collect();
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
            // A queued child is addressable too, and it holds no live handle.
            None => self.cancel_queued_descendant(caller, id),
        }
    }

    /// The handle for one live child anywhere in the process. Private on purpose.
    ///
    /// [`AgentRegistry::descendant`] is the scoped form every caller uses.
    fn handle(&self, id: AgentId) -> Option<LiveAgent> {
        self.state().live.get(&id.0).cloned()
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
        let mut state = self.state();
        if state.finished.len() >= MAX_REMEMBERED_REPORTS {
            // The oldest report leaves, and its name leaves with it, so a handle answers
            // for exactly as long as `agent_status` can.
            if let Some(evicted) = state.finished.pop_front() {
                let evicted_id = evicted.id;
                prune_handle(&mut state, evicted_id);
            }
        }
        state.finished.push_back(FinishedAgent {
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
        let state = self.state();
        // A queued child answers too, or "pollable by id" is false for it. The place
        // is computed here, under the lock, so it is never stale.
        if let Some(entry) = state
            .queued
            .get(&id.0)
            .filter(|entry| entry.ancestors.contains(&caller.id()))
        {
            return Some(AgentStatus::Queued {
                agent: entry.agent.clone(),
                depth: entry.depth,
                position: position_in_line(&state, entry),
                cancelled: entry.cancel.is_cancelled(),
            });
        }
        state
            .finished
            .iter()
            .rev()
            .find(|entry| entry.id == id && entry.ancestors.contains(&caller.id()))
            .map(|entry| AgentStatus::Finished {
                report: entry.report.clone(),
            })
    }

    /// Register a live child. Called by `spawn_child`, which cannot forget.
    fn register(&self, handle: LiveAgent) {
        let mut state = self.state();
        let tree = tree_root(&handle.ancestors, handle.id);
        bind_handle(&mut state, handle.id, tree, &handle.agent);
        state.live.insert(handle.id.0, handle);
    }

    /// Deregister a child. Called by `ChildSlot::drop`, which cannot forget.
    fn deregister(&self, id: AgentId) {
        let mut state = self.state();
        state.live.remove(&id.0);
        // The name outlives the slot only while something still answers for the id: a
        // queued entry, or a remembered report. Otherwise it goes, or the table would
        // grow with every finished child.
        prune_handle(&mut state, id);
    }

    /// The derived handle of one child, or `None` when nothing answers for it.
    ///
    /// Not scoped, because a name is not an address on its own: [`AgentRegistry::resolve`]
    /// is the scoped read, and every accessor re-checks the id it returns.
    pub fn handle_of(&self, id: AgentId) -> Option<String> {
        self.state()
            .handles
            .get(&id.0)
            .map(|binding| binding.name.clone())
    }

    /// Resolve a reference to a child's id, inside the caller's own tree.
    ///
    /// An id resolves when it names a descendant of `caller`. A digits-only name
    /// resolves as an id. Any other name resolves as a derived handle, then an alias, so
    /// an alias can never hide a real child. It returns `None` for a child of another
    /// tree, exactly as an unknown id returns `None`, so a caller cannot tell the two
    /// apart. See decision D-a-caller-addresses-only-its-own.
    pub fn resolve(&self, caller: &AgentNode, reference: &AgentRef) -> Option<AgentId> {
        match reference {
            AgentRef::Id(id) => self.scoped(caller, AgentId(*id)),
            AgentRef::Name(name) if is_digits_only(name) => {
                // A digits-only name is always an id. A name that cannot fit in a u64 is
                // simply no id, and that is an ordinary not-found.
                let id = name.parse::<u64>().ok()?;
                self.scoped(caller, AgentId(id))
            }
            AgentRef::Name(name) => {
                let tree = caller.tree_root();
                let found = {
                    let state = self.state();
                    let by_handle = state
                        .handles
                        .iter()
                        .find(|(_, binding)| binding.tree == tree && binding.name == *name)
                        .map(|(id, _)| AgentId(*id));
                    // The handle is read first, so the shadow rule holds at read time and
                    // not only at write time.
                    by_handle.or_else(|| state.aliases.get(&(tree, name.clone())).copied())
                };
                self.scoped(caller, found?)
            }
        }
    }

    /// The id, but only when something in the caller's own tree answers for it.
    ///
    /// It accepts a live child, a queued one, and one whose report is remembered, which
    /// is exactly the set `status` answers for. So a name resolves for as long as the id
    /// does, and never longer.
    fn scoped(&self, caller: &AgentNode, id: AgentId) -> Option<AgentId> {
        self.status(caller, id).map(|_| id)
    }

    /// Set an alias for a child, inside the caller's tree.
    ///
    /// It refuses a name that a derived handle already holds, and a name another alias
    /// holds. A derived handle always resolves first, so an alias can never hide a real
    /// child. A refused alias never fails the spawn: the caller reports the refusal and
    /// keeps the work. See `SPEC-subagent-slots-handles-grace` section 3.2.
    pub fn set_alias(
        &self,
        caller: &AgentNode,
        id: AgentId,
        alias: impl Into<String>,
    ) -> Result<(), AliasError> {
        let alias = alias.into();
        // The shape is checked before the scope, because a malformed name is the
        // caller's own mistake and needs no lookup.
        validate_alias(&alias)?;
        if self.scoped(caller, id).is_none() {
            return Err(AliasError::Unknown { id });
        }
        let tree = caller.tree_root();
        let mut state = self.state();
        if state
            .handles
            .values()
            .any(|binding| binding.tree == tree && binding.name == alias)
        {
            return Err(AliasError::ShadowsHandle { name: alias });
        }
        if state.aliases.contains_key(&(tree, alias.clone())) {
            return Err(AliasError::Taken { name: alias });
        }
        state.aliases.insert((tree, alias), id);
        Ok(())
    }

    /// Cancel one queued descendant of `caller`. Returns true when it found one.
    ///
    /// A queued child holds no live handle, so `descendant` cannot reach it. Without
    /// this branch `cancel_agent` would answer "no such subagent" for a child the
    /// model was just told about.
    fn cancel_queued_descendant(&self, caller: &AgentNode, id: AgentId) -> bool {
        let state = self.state();
        match state
            .queued
            .get(&id.0)
            .filter(|entry| entry.ancestors.contains(&caller.id()))
        {
            Some(entry) => {
                let token = entry.cancel.clone();
                drop(state);
                token.cancel();
                true
            }
            None => false,
        }
    }

    /// Steer one descendant of `caller`, whether it runs or waits.
    ///
    /// One entry point, so a queued child and a live child cannot drift apart. A
    /// message pushed into a queued child buffers, and its first turn boundary
    /// delivers it. `None` means the id names no child of this caller.
    pub fn steer_descendant(
        &self,
        caller: &AgentNode,
        id: AgentId,
        message: Vec<crate::ContentBlock>,
    ) -> Option<Result<usize, crate::QueueError>> {
        if let Some(handle) = self.descendant(caller, id) {
            return Some(handle.steer(message));
        }
        let queue = {
            let state = self.state();
            state
                .queued
                .get(&id.0)
                .filter(|entry| entry.ancestors.contains(&caller.id()))
                .map(|entry| entry.queue.clone())
        }?;
        Some(queue.push(message))
    }

    /// Remove a queued entry. Called by `QueuedChild::drop` and by a handout.
    fn dequeue(&self, id: AgentId) {
        let mut state = self.state();
        state.queued.remove(&id.0);
        // A waiter that never ran and left no report takes its name with it.
        prune_handle(&mut state, id);
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
            child_permits: Arc::new(tokio::sync::Semaphore::new(
                self.limits().max_children_per_parent,
            )),
        }
    }
}

/// One agent's place in the spawn tree.
///
/// A clone shares the same parent permits and the same registry, so a queued child
/// can hold its parent and still count against one cap.
///
/// The root session holds the root node. A successful [`AgentNode::spawn_child`]
/// returns a child node and a live-agent guard. The guard decrements both the
/// per-parent count and the process-wide count when it drops, so a finished
/// child frees its slot.
#[derive(Clone, Debug)]
pub struct AgentNode {
    id: AgentId,
    depth: u32,
    ancestors: Vec<AgentId>,
    registry: AgentRegistry,
    /// This parent's own cap, as permits. A clone of the node shares them, so a
    /// clone cannot exceed the cap its original obeys.
    child_permits: Arc<tokio::sync::Semaphore>,
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

    /// The root of this node's tree. A root node is its own tree.
    ///
    /// A handle and an alias are keyed by this, so two sessions in one process may both
    /// hold an `explore` and neither can reach the other's.
    pub fn tree_root(&self) -> AgentId {
        tree_root(&self.ancestors, self.id)
    }

    /// The number of live children this node runs now.
    pub fn live_children(&self) -> usize {
        self.registry
            .limits()
            .max_children_per_parent
            .saturating_sub(self.child_permits.available_permits())
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
        let limits = *self.registry.limits();
        let child_depth = self.depth + 1;
        if child_depth > limits.max_depth {
            return Err(SubagentError::DepthExceeded {
                limit: limits.max_depth,
                attempted: child_depth,
            });
        }

        // Take the per-parent permit, then the process-wide one. A permit is atomic,
        // so two racing spawns cannot both pass a cap of one, and the counter loops
        // this replaced could not be waited on. See decision D-permits-not-counters.
        let child_permit = Arc::clone(&self.child_permits)
            .try_acquire_owned()
            .map_err(|_| SubagentError::TooManyChildren {
                limit: limits.max_children_per_parent,
                current: limits.max_children_per_parent,
            })?;
        let live_permit = Arc::clone(&self.registry.inner.live_permits)
            .try_acquire_owned()
            .map_err(|_| SubagentError::TooManyLiveAgents {
                limit: limits.max_live_total,
                current: limits.max_live_total,
            })?;

        let mut ancestors = self.ancestors.clone();
        ancestors.push(self.id);
        // The tree cannot cycle by construction, so a duplicate id means a bug.
        // The guard costs one small set and stops an infinite loop inside a lock.
        check_no_cycle(&ancestors)?;

        let (spawn, handle) = self.build_child(self.child_build(
            agent.into(),
            cancel,
            ancestors,
            child_permit,
            live_permit,
        ));
        self.registry.register(handle);
        Ok(spawn)
    }

    /// Reserve a slot now, or queue when this parent's cap is full.
    ///
    /// It refuses at once for a cap that waiting cannot fix: the depth limit, the
    /// cycle guard, the process-wide live cap, and either wait line. It queues for the
    /// per-parent child cap alone. The tools call this. See decision
    /// D-queue-over-refuse and decision D-caps-that-cannot-wait-refuse.
    pub fn admit_child(
        &self,
        agent: impl Into<String>,
        cancel: CancelToken,
    ) -> Result<Admission, SubagentError> {
        let limits = *self.registry.limits();
        let child_depth = self.depth + 1;
        if child_depth > limits.max_depth {
            return Err(SubagentError::DepthExceeded {
                limit: limits.max_depth,
                attempted: child_depth,
            });
        }
        let mut ancestors = self.ancestors.clone();
        ancestors.push(self.id);
        check_no_cycle(&ancestors)?;

        let agent = agent.into();
        // A free per-parent slot needs the process-wide slot too, and that cap always
        // refuses rather than queues, because waiting on it is waiting on another tree.
        if let Ok(child_permit) = Arc::clone(&self.child_permits).try_acquire_owned() {
            let live_permit = Arc::clone(&self.registry.inner.live_permits)
                .try_acquire_owned()
                .map_err(|_| SubagentError::TooManyLiveAgents {
                    limit: limits.max_live_total,
                    current: limits.max_live_total,
                })?;
            let (spawn, handle) = self.build_child(self.child_build(
                agent,
                cancel,
                ancestors,
                child_permit,
                live_permit,
            ));
            self.registry.register(handle);
            return Ok(Admission::Started(spawn));
        }

        // The parent is full, so the child waits. Both wait lines are checked here,
        // and the registration happens in the same critical section as the id, so no
        // other task can observe a half-admitted child. Per-parent is checked first,
        // because it is the tighter and more actionable bound.
        let id = self.registry.allocate_id();
        let queue = crate::MessageQueue::new();
        {
            let mut state = self.registry.state();
            let waiting_here = state
                .queued
                .values()
                .filter(|entry| entry.parent == self.id)
                .count();
            if waiting_here >= limits.max_queued_per_parent {
                return Err(SubagentError::QueueFull {
                    scope: crate::QueueScope::Parent,
                    limit: limits.max_queued_per_parent,
                });
            }
            if state.queued.len() >= limits.max_queued_total {
                return Err(SubagentError::QueueFull {
                    scope: crate::QueueScope::Process,
                    limit: limits.max_queued_total,
                });
            }
            let sequence = state.next_sequence;
            state.next_sequence += 1;
            // The name is derived here, in the same critical section as the entry, so a
            // waiter is addressable by name from the moment it exists.
            bind_handle(&mut state, id, tree_root(&ancestors, id), &agent);
            state.queued.insert(
                id.0,
                QueuedEntry {
                    agent: agent.clone(),
                    depth: child_depth,
                    ancestors: ancestors.clone(),
                    cancel: cancel.clone(),
                    queue: queue.clone(),
                    parent: self.id,
                    sequence,
                },
            );
        }

        Ok(Admission::Queued(QueuedChild {
            id,
            agent,
            depth: child_depth,
            ancestors,
            cancel,
            queue,
            parent: self.clone(),
            handed_out: false,
        }))
    }

    /// Everything one child needs to start. A named struct, not eight arguments.
    ///
    /// Clippy refused the eight-argument form, and it was right: a four-argument
    /// `Session::new` once hid a fake model id and an approve-all policy here. See
    /// decision D-no-four-argument-session-new.
    fn child_build(
        &self,
        agent: String,
        cancel: CancelToken,
        ancestors: Vec<AgentId>,
        child_permit: tokio::sync::OwnedSemaphorePermit,
        live_permit: tokio::sync::OwnedSemaphorePermit,
    ) -> ChildBuild {
        ChildBuild {
            agent,
            cancel,
            ancestors,
            child_permit,
            live_permit,
            id: None,
            queue: None,
        }
    }

    /// Build the child node and its handle. It registers nothing.
    ///
    /// The caller inserts the handle under the state lock, so a handout can remove the
    /// queued entry and insert the live one in one critical section. A version of this
    /// that registered internally forced three separate locks, and the spec's promise
    /// of one step was then false. See decision D-one-registry-state-lock.
    fn build_child(&self, build: ChildBuild) -> (ChildSpawn, LiveAgent) {
        let ChildBuild {
            agent,
            cancel,
            ancestors,
            child_permit,
            live_permit,
            id,
            queue,
        } = build;
        let child = AgentNode {
            id: id.unwrap_or_else(|| self.registry.allocate_id()),
            depth: self.depth + 1,
            ancestors,
            registry: self.registry.clone(),
            child_permits: Arc::new(tokio::sync::Semaphore::new(
                self.registry.limits().max_children_per_parent,
            )),
        };
        let child_id = child.id;
        let (progress_tx, progress_rx) = tokio::sync::watch::channel(AgentProgress::default());
        let queue = queue.unwrap_or_default();
        let handle = LiveAgent {
            id: child.id,
            agent,
            depth: child.depth,
            ancestors: child.ancestors.clone(),
            cancel,
            progress: progress_rx,
            queue: queue.clone(),
        };
        let spawn = ChildSpawn {
            node: child,
            slot: ChildSlot {
                registry: self.registry.clone(),
                id: child_id,
                _child_permit: child_permit,
                _live_permit: live_permit,
            },
            progress: progress_tx,
            queue,
        };
        (spawn, handle)
    }
}

/// Everything one child needs to start.
///
/// `id` and `queue` are `None` for a fresh spawn. A handout fills both, because a
/// queued child already owns an id the model was told and a queue a steer may have
/// filled.
#[derive(Debug)]
struct ChildBuild {
    agent: String,
    cancel: CancelToken,
    ancestors: Vec<AgentId>,
    child_permit: tokio::sync::OwnedSemaphorePermit,
    live_permit: tokio::sync::OwnedSemaphorePermit,
    id: Option<AgentId>,
    queue: Option<crate::MessageQueue>,
}

/// A live-agent reservation. It holds one per-parent permit and one process-wide
/// permit. Both return when it drops, so a finished child always frees its slot, and
/// the semaphore grants the next waiter in line.
#[derive(Debug)]
pub struct ChildSlot {
    registry: AgentRegistry,
    /// The child this slot holds, so dropping the slot drops the handle too.
    id: AgentId,
    /// Dropped with the slot. The name says it is never read.
    _child_permit: tokio::sync::OwnedSemaphorePermit,
    _live_permit: tokio::sync::OwnedSemaphorePermit,
}

impl Drop for ChildSlot {
    fn drop(&mut self) {
        // The handle goes with the slot. A registry that kept a finished child
        // would leak, and it would hand out a handle that cancels nothing. The two
        // permits return as they drop, which grants the next waiter directly.
        self.registry.deregister(self.id);
    }
}

/// The result of admitting a child when this parent's cap may be full.
///
/// The per-parent cap no longer refuses. It queues. So a caller must handle both a
/// slot that was free and one that was not. See decision D-queue-over-refuse.
#[derive(Debug)]
pub enum Admission {
    /// A slot was free. The child holds it and may start now.
    Started(ChildSpawn),
    /// No slot was free. The child has an id and waits in its parent's line.
    Queued(QueuedChild),
}

/// Why a queued child never started. Both are results a parent can act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dequeued {
    /// The child was cancelled while it waited, alone or with its parent.
    Cancelled,
    /// A slot freed under this parent, and the process-wide cap was full by then.
    ///
    /// A queued child holds no process-wide permit while it waits, so the cap can
    /// fill between the admission and the start. rho refuses rather than wait,
    /// because a wait on that cap is a wait on another tree.
    ProcessWideFull { limit: usize },
}

impl std::fmt::Display for Dequeued {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Dequeued::Cancelled => f.write_str("the queued subagent was cancelled before it ran."),
            Dequeued::ProcessWideFull { limit } => write!(
                f,
                "a slot freed for this child, and the process-wide limit of {limit} live agents \
                 was reached first. The work was not started. Run it again when a child \
                 finishes, or ask the user to raise --max-live-agents."
            ),
        }
    }
}

/// A child that holds an id but not yet a slot.
///
/// It is addressable while it waits, through the registry, by the same id a started
/// child uses. Its `CancelToken` and its `MessageQueue` exist now, so a steer buffers
/// and a cancel lands before the child ever runs.
#[derive(Debug)]
pub struct QueuedChild {
    id: AgentId,
    agent: String,
    depth: u32,
    ancestors: Vec<AgentId>,
    cancel: CancelToken,
    queue: crate::MessageQueue,
    /// The parent, for its permits and its registry. A clone shares both.
    parent: AgentNode,
    /// True once the child holds a slot and a live handle exists.
    ///
    /// It flips at one moment only: after the process-wide permit is held and the
    /// live handle is inserted. Setting it when the fields are taken would skip the
    /// removal on the refusal exits, and the entry would answer "queued" for ever.
    handed_out: bool,
}

impl QueuedChild {
    /// This child's id, allocated at admission.
    pub fn id(&self) -> AgentId {
        self.id
    }

    /// The agent definition it will run.
    pub fn agent(&self) -> &str {
        &self.agent
    }

    /// How deep it will sit in the spawn tree.
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// The child's place in its parent's wait line, counted from one.
    ///
    /// Computed on read, under the state lock, so it is never stale. A stored place
    /// goes wrong the moment a child ahead leaves.
    pub fn position(&self) -> usize {
        let state = self.parent.registry.state();
        match state.queued.get(&self.id.0) {
            Some(entry) => position_in_line(&state, entry),
            None => 0,
        }
    }

    /// Give up the place. Then `started` resolves `Err(Dequeued::Cancelled)`.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// True when this child, or an ancestor, was cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// The queue the child reads once it starts. A steer buffers here meanwhile.
    pub fn queue(&self) -> crate::MessageQueue {
        self.queue.clone()
    }

    /// Wait for a permit, then return the live spawn.
    ///
    /// It waits on the per-parent permit only. When that arrives it takes the
    /// process-wide permit without waiting, because waiting on that cap is waiting on
    /// another tree. A failure there releases the per-parent permit first, so a
    /// waiter that gives up blocks no sibling.
    pub async fn started(mut self) -> Result<ChildSpawn, Dequeued> {
        let limits = *self.parent.registry.limits();
        let permits = Arc::clone(&self.parent.child_permits);
        let child_permit = tokio::select! {
            // `acquire_owned` is cancel-safe: dropping the future takes no permit,
            // and a permit taken on a lost race returns as it drops.
            permit = permits.acquire_owned() => match permit {
                Ok(permit) => permit,
                Err(_) => return Err(Dequeued::Cancelled),
            },
            () = self.cancel.cancelled() => return Err(Dequeued::Cancelled),
        };
        if self.cancel.is_cancelled() {
            return Err(Dequeued::Cancelled);
        }

        let live_permit =
            match Arc::clone(&self.parent.registry.inner.live_permits).try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    // Release the per-parent permit by dropping it, then refuse.
                    drop(child_permit);
                    return Err(Dequeued::ProcessWideFull {
                        limit: limits.max_live_total,
                    });
                }
            };

        let (spawn, handle) = self.parent.build_child(ChildBuild {
            // The id the caller already holds. A fresh one here would strand the id the
            // model was told, and every later steer and cancel with it.
            id: Some(self.id),
            // And the queue a steer may already have filled.
            queue: Some(self.queue.clone()),
            ..self.parent.child_build(
                self.agent.clone(),
                self.cancel.clone(),
                self.ancestors.clone(),
                child_permit,
                live_permit,
            )
        });

        // One critical section moves the id from the queued index to the live one, so
        // no lookup finds it in both or in neither. `bind_handle` runs here too, and it
        // is the guard inside it that keeps the name: a handout that derived a fresh one
        // would strand the name the model was told, exactly as a fresh id once stranded
        // every steer and cancel. One place binds a name, so no caller can forget.
        {
            let mut state = self.parent.registry.state();
            state.queued.remove(&self.id.0);
            bind_handle(
                &mut state,
                handle.id,
                tree_root(&handle.ancestors, handle.id),
                &handle.agent,
            );
            state.live.insert(handle.id.0, handle);
        }
        self.handed_out = true;
        Ok(spawn)
    }
}

impl Drop for QueuedChild {
    fn drop(&mut self) {
        // Every path that is not a successful handout runs this: a cancel, a parent
        // cancel, a full process-wide cap, and a caller that drops the value without
        // ever awaiting `started`. Without it the entry stays, the map grows with
        // every spawn, and `agent_status` answers "queued" for a child that will never
        // run. `ChildSlot` does exactly this for a live child.
        if !self.handed_out {
            self.parent.registry.dequeue(self.id);
        }
    }
}

/// One plus every entry of the same parent that queued earlier.
///
/// The place is derived, so a child ahead leaving moves everyone behind it up.
fn position_in_line(state: &RegistryState, entry: &QueuedEntry) -> usize {
    1 + state
        .queued
        .values()
        .filter(|other| other.parent == entry.parent && other.sequence < entry.sequence)
        .count()
}

/// The root of the tree a child belongs to.
///
/// The first ancestor is the root node, and a root has no ancestors, so it is its own
/// tree. A handle is keyed by this, which is what keeps one session's numbering out of
/// another's. See decision D-handle-is-a-second-address.
fn tree_root(ancestors: &[AgentId], own: AgentId) -> AgentId {
    ancestors.first().copied().unwrap_or(own)
}

/// Derive and store a child's handle, unless it already holds one.
///
/// **It never re-derives.** A queued child is told its name at admission, so the handout
/// must keep it. A fresh name at start would strand the name the model was given, exactly
/// as a fresh id once stranded every steer and cancel.
///
/// The caller holds the state lock, so the read of the taken names and the write of the
/// new one are one critical section. Two admissions of one agent name therefore cannot
/// choose one name. See decision D-one-registry-state-lock.
fn bind_handle(state: &mut RegistryState, id: AgentId, tree: AgentId, agent: &str) {
    if state.handles.contains_key(&id.0) {
        return;
    }
    let taken: std::collections::HashSet<String> = state
        .handles
        .values()
        .filter(|binding| binding.tree == tree)
        .map(|binding| binding.name.clone())
        .collect();
    let name = derive_handle(&taken, agent);
    state.handles.insert(id.0, HandleBinding { tree, name });
}

/// Drop a child's handle and every alias for it, once nothing answers for the id.
///
/// A name must resolve for exactly as long as `status` answers, so the test is the three
/// indexes and not one of them. Without this the handle table would be an unbounded map
/// keyed by agent names a model chose. See decision D-bash-line-cap.
fn prune_handle(state: &mut RegistryState, id: AgentId) {
    if state.live.contains_key(&id.0)
        || state.queued.contains_key(&id.0)
        || state.finished.iter().any(|entry| entry.id == id)
    {
        return;
    }
    state.handles.remove(&id.0);
    state.aliases.retain(|_, owner| *owner != id);
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
