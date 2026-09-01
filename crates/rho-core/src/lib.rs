//! `rho-core` is the composable agent runtime core.
//!
//! It is a pure library. It links no HTTP client and no terminal. It defines the
//! message model, the streaming event model, the provider and tool traits, the
//! hook chain, the cancellation type, and the agent loop. Every other crate
//! builds on the types here.

mod agent;
mod agent_task;
mod cancel;
mod content;
mod context;
mod error;
mod event;
mod hook;
mod provider;
mod queue;
mod reasoning;
mod results;
mod retry;
mod sandbox;
mod secret;
mod session;
mod subagent;
mod tasks;
mod thinking;
mod tool;
mod transcript;
mod usage;

pub use agent::{
    AgentConfig, AgentEvent, AgentEvents, AgentStopReason, ModelSelection, Session, SessionConfig,
};
pub use agent_task::{
    Acceptance, AgentTask, ArtifactChecker, ArtifactSpec, CheckOutcome, CheckResult, ChildClaims,
    CommandRunner, DefaultGate, Gate, GateContext, GateReport,
};
pub use cancel::CancelToken;
pub use content::{ContentBlock, ImageSource, Message, ProviderState, ReasoningOwner, Role};
pub use context::Context;
pub use error::{Error, ProviderError};
pub use event::StreamEvent;
pub use hook::{Hook, HookChain, HookOutcome, ToolCallView};
pub use provider::{CompletionRequest, Provider, ProviderStream, ToolSpec};
pub use queue::{
    BLOCK_OVERHEAD_BYTES, JSON_NODE_MIN_BYTES, MAX_COUNTED_JSON_DEPTH, MAX_STEER_MESSAGE_BYTES,
    MessageQueue, QueueError, STEER_QUEUE_CAPACITY, message_bytes,
};
pub use reasoning::{MIN_THINKING_BUDGET, ReasoningDisplay, ReasoningEffort};
pub use results::{
    CappedText, FileResultStore, HeadPreview, MAX_MATCH_LINE_BYTES, READ_CEILING_BYTES,
    ResultLimits, ResultPolicy, ResultPreview, ResultStore, ResultStoreError, StoredMatch,
    StoredSlice, cap_result_text, default_search, is_valid_handle,
};
pub use retry::RetryPolicy;
pub use sandbox::SandboxMode;
pub use secret::Secret;
pub use session::{
    Entry, ForkOrigin, GIT_ENTRY_MAX_BYTES, MAX_DROPPED_RECORDS, MAX_LINE_BYTES, MAX_RECORD_BYTES,
    MINT_ATTEMPTS, NewSession, NewSessionWithoutId, PrefixMatch, ProjectKey, ROW_HEAD_LINES,
    ROW_TAIL_BYTES, ReadResult, Record, RecordId, RowMeta, SessionError, SessionHeader, SessionId,
    SessionLock, SessionLog, SessionReader, SessionRecorder, SessionRow, SessionStore,
    SessionSummary, SessionWriter, StoredApproval, StoredSandbox, branch_messages,
    check_resume_permission, classify_lock_failure, decode, default_store_root, encode,
    expire_stale_result_handles, row_from,
};
pub use subagent::{
    Admission, AgentId, AgentNode, AgentOutcome, AgentProgress, AgentRef, AgentRegistry,
    AgentReport, AgentStatus, AliasError, BothPolicies, ChildSlot, ChildSpawn, CollectOptions,
    DEFAULT_AGENT_STEER_MESSAGE_BYTES, DEFAULT_QUEUE_WAIT, DEFAULT_SUBAGENT_GRACE_TURNS, Dequeued,
    LiveAgent, MAX_ALIAS_LENGTH, MAX_CHILD_RETRIES, MAX_SUMMARY_CHARS, QueueScope, QueuedChild,
    RetryLedger, SubagentError, SubagentLimits, ToolIntersection, cap_tool_calls, check_no_cycle,
    collect_report, intersect_tools, narrow_sandbox,
};
pub use tasks::{
    BackgroundReason, DEFAULT_FOREGROUND_LIMIT_MS, RunMode, SessionEvents, TaskError, TaskHandle,
    TaskId, TaskLimits, TaskProgress, TaskRegistry, TaskSnapshot, TaskState, WaitUntil,
    decide_run_mode,
};
pub use thinking::{ThinkingPiece, ThinkingSplitter};
pub use tool::{
    AllowAllPolicy, ApprovalDecision, ApprovalPolicy, ReadOnlyPolicy, Tool, ToolContext, ToolError,
    ToolKind, ToolOutput, ToolRegistry, confine,
};
pub use transcript::{TranscriptBody, TranscriptEntry, TranscriptWriter, session_transcript_dir};
pub use usage::{StopReason, Usage};
