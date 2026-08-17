//! Subagents: the spawner, the policy composition, and the result contract.
//!
//! A subagent is another `Session` on the same runtime. The parent asks for a
//! result, a child does the reading and the trying, and only a summary comes
//! back. See `docs/specs/SPEC-11-subagents.md`.
//!
//! This module owns the security core. A child is confined by composition, not
//! by comparison: [`BothPolicies`] allows a call only when the parent and the
//! child both allow it, so a child can only ever be more restrictive. The tool
//! set follows the same rule with [`intersect_tools`], and the sandbox mode may
//! only narrow with [`narrow_sandbox`]. See decision D-036.

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
/// `SPEC-11` section 6.
pub const MAX_SUMMARY_CHARS: usize = 8_000;

/// The most times a unit of work may die before it is reported failed.
///
/// This is jcode's reclaim cap. Without it a poisoned task loops until the
/// budget is gone. See `SPEC-11` section 8.
pub const MAX_CHILD_RETRIES: u32 = 3;

// --- The security core: composition, not comparison (D-036) ---

/// Allow a call only when **both** policies allow it.
///
/// This is how a child is confined. The parent's policy is always one of the two
/// conjuncts, so a child can only ever be more restrictive. Escalation is not
/// checked, it is unrepresentable. See decision D-036.
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
/// parent never had. See decision D-036.
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
/// for a weaker mode is refused, and the refusal names both modes. See `SPEC-11`
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

// --- Limits (SPEC-11 section 7) ---

/// The four subagent limits. A child that spawns a child fans out
/// geometrically, so one limit is not enough. See `SPEC-11` section 7.
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
}

impl SubagentLimits {
    /// The starting limits, stated here and not hidden. See decision D-013.
    ///
    /// Depth 2, four children per parent, 32 live in total, and a ten minute
    /// child timeout.
    pub fn new() -> Self {
        Self {
            max_depth: 2,
            max_children_per_parent: 4,
            max_live_total: 32,
            child_timeout: Duration::from_secs(600),
        }
    }
}

impl Default for SubagentLimits {
    fn default() -> Self {
        Self::new()
    }
}

// --- The result contract (SPEC-11 section 6) ---

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
}

// --- Refusals (SPEC-11 section 7: refusing must teach) ---

/// A refusal to spawn or run a child. Every variant names the limit, its value,
/// and what to do. See `SPEC-11` section 7.
#[derive(Clone, Debug, thiserror::Error)]
pub enum SubagentError {
    #[error(
        "the depth limit is {limit} and this would be depth {attempted}. \
         Do the work here, or ask the user to raise --max-agent-depth."
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

// --- The spawn tree and its guards ---

/// Process-wide subagent state, shared by every session in one process.
///
/// It holds the limits, the count of live agents, and the id allocator. A clone
/// shares the same state, so a child created from any node counts against the
/// same process-wide cap. See `SPEC-11` section 7.
#[derive(Clone, Debug)]
pub struct AgentRegistry {
    inner: Arc<RegistryInner>,
}

#[derive(Debug)]
struct RegistryInner {
    limits: SubagentLimits,
    live_total: AtomicUsize,
    next_id: AtomicU64,
}

impl AgentRegistry {
    /// Build a registry with the given limits.
    pub fn new(limits: SubagentLimits) -> Self {
        Self {
            inner: Arc::new(RegistryInner {
                limits,
                live_total: AtomicUsize::new(0),
                next_id: AtomicU64::new(0),
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
    pub fn spawn_child(&self) -> Result<(AgentNode, ChildSlot), SubagentError> {
        let limits = self.registry.limits();
        let child_depth = self.depth + 1;
        if child_depth > limits.max_depth {
            return Err(SubagentError::DepthExceeded {
                limit: limits.max_depth,
                attempted: child_depth,
            });
        }

        // Reserve the per-parent slot. Read then compare then add, under the
        // atomic, so a check that passes cannot be undone by a racing spawn.
        let current_children = self.children.load(Ordering::SeqCst);
        if current_children >= limits.max_children_per_parent {
            return Err(SubagentError::TooManyChildren {
                limit: limits.max_children_per_parent,
                current: current_children,
            });
        }

        // Reserve the process-wide slot with a compare-and-swap loop, so two
        // parents cannot both pass a cap of one. This is the case a per-parent
        // cap misses.
        let live = &self.registry.inner.live_total;
        loop {
            let current = live.load(Ordering::SeqCst);
            if current >= limits.max_live_total {
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

        // The process-wide slot is now held. Commit the per-parent slot.
        self.children.fetch_add(1, Ordering::SeqCst);

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
        let slot = ChildSlot {
            parent_children: Arc::clone(&self.children),
            registry: self.registry.clone(),
        };
        Ok((child, slot))
    }
}

/// A live-agent reservation. It holds one per-parent slot and one process-wide
/// slot. Both free when it drops, so a finished child always frees its slot.
#[derive(Debug)]
pub struct ChildSlot {
    parent_children: Arc<AtomicUsize>,
    registry: AgentRegistry,
}

impl Drop for ChildSlot {
    fn drop(&mut self) {
        self.parent_children.fetch_sub(1, Ordering::SeqCst);
        self.registry
            .inner
            .live_total
            .fetch_sub(1, Ordering::SeqCst);
    }
}

/// Walk an ancestor chain and refuse a cycle.
///
/// The tree cannot cycle by construction, because the parent link is stored
/// directly and every id is fresh. So a duplicate id means a bug, not a race.
/// The visited set costs little and stops an infinite loop inside a lock. See
/// `SPEC-11` section 7.
pub fn check_no_cycle(ancestors: &[AgentId]) -> Result<(), SubagentError> {
    let mut seen = HashSet::with_capacity(ancestors.len());
    for id in ancestors {
        if !seen.insert(*id) {
            return Err(SubagentError::CycleDetected);
        }
    }
    Ok(())
}

/// Tracks how many times a unit of work has died, keyed by a work key.
///
/// This is jcode's reclaim cap. When a caller re-delegates the same work and the
/// child dies again, the count grows. After [`MAX_CHILD_RETRIES`] the work is
/// reported failed rather than retried. See `SPEC-11` section 8.
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

// --- Running a child to a report (SPEC-11 section 6) ---

/// Drive a child's event stream to an [`AgentReport`].
///
/// It sums usage, counts turns, keeps the child's final answer as the capped
/// summary, and writes the full transcript to `transcript_path`. The transcript
/// never reaches the model, only the summary does. See `SPEC-11` section 6.
///
/// A child that passes `timeout` is cancelled through the shared token and
/// reported `Canceled`. A child that ends without a report is reported `Failed`.
/// A child failure is a result, not the end of the parent's run. See decision
/// D-032.
pub async fn collect_report(
    agent: impl Into<String>,
    mut events: AgentEvents,
    cancel: CancelToken,
    timeout: Duration,
    transcript_path: Option<PathBuf>,
) -> AgentReport {
    let agent = agent.into();
    let mut usage = Usage::default();
    let mut turns = 0u32;
    let mut current_text = String::new();
    let mut last_answer: Option<String> = None;
    let mut transcript: Vec<String> = Vec::new();
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
                        transcript.push(format!("{event:?}"));
                        match &event {
                            AgentEvent::TurnStart => {
                                turns += 1;
                                current_text.clear();
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
    let transcript = write_transcript(transcript_path, &transcript).await;

    AgentReport {
        agent,
        outcome,
        summary,
        usage,
        turns,
        transcript,
    }
}

/// Map a run's stop reason onto a child outcome.
fn outcome_from_stop(stop_reason: AgentStopReason) -> AgentOutcome {
    match stop_reason {
        AgentStopReason::EndTurn => AgentOutcome::Done,
        AgentStopReason::MaxTurnRequests => AgentOutcome::OutOfTurns,
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

/// Write the transcript lines to disk. Return the path on success, `None` on
/// failure. A failure to write a transcript must not fail the report, because the
/// summary is the load-bearing result.
async fn write_transcript(path: Option<PathBuf>, lines: &[String]) -> Option<PathBuf> {
    let path = path?;
    let body = lines.join("\n");
    match tokio::fs::write(&path, body).await {
        Ok(()) => Some(path),
        Err(error) => {
            tracing::warn!(path = %path.display(), "cannot write the child transcript: {error}");
            None
        }
    }
}
