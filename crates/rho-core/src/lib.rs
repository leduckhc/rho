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
mod retry;
mod sandbox;
mod secret;
mod session;
mod subagent;
mod tasks;
mod tool;
mod usage;

pub use agent::{AgentConfig, AgentEvent, AgentEvents, AgentStopReason, Session, SessionConfig};
pub use agent_task::{
    Acceptance, AgentTask, ArtifactChecker, ArtifactSpec, CheckOutcome, CheckResult, ChildClaims,
    CommandRunner, DefaultGate, Gate, GateContext, GateReport,
};
pub use cancel::CancelToken;
pub use content::{ContentBlock, ImageSource, Message, Role};
pub use context::Context;
pub use error::{Error, ProviderError};
pub use event::StreamEvent;
pub use hook::{Hook, HookChain, HookOutcome, ToolCallView};
pub use provider::{CompletionRequest, Provider, ProviderStream, ToolSpec};
pub use retry::RetryPolicy;
pub use sandbox::SandboxMode;
pub use secret::Secret;
pub use session::{
    Entry, MAX_LINE_BYTES, MAX_RECORD_BYTES, ReadResult, Record, RecordId, SessionError,
    SessionHeader, SessionLog, SessionReader, SessionRecorder, SessionStore, SessionSummary,
    SessionWriter, StoredApproval, StoredSandbox, branch_messages, check_resume_permission, decode,
    encode,
};
pub use subagent::{
    AgentId, AgentNode, AgentOutcome, AgentProgress, AgentRegistry, AgentReport, BothPolicies,
    ChildSlot, ChildSpawn, LiveAgent, MAX_CHILD_RETRIES, MAX_SUMMARY_CHARS, RetryLedger,
    SubagentError, SubagentLimits, ToolIntersection, cap_tool_calls, check_no_cycle,
    collect_report, intersect_tools, narrow_sandbox,
};
pub use tasks::{
    BackgroundReason, DEFAULT_FOREGROUND_LIMIT_MS, RunMode, TaskError, TaskHandle, TaskId,
    TaskLimits, TaskProgress, TaskRegistry, TaskSnapshot, TaskState, WaitUntil, decide_run_mode,
};
pub use tool::{
    AllowAllPolicy, ApprovalDecision, ApprovalPolicy, ReadOnlyPolicy, Tool, ToolContext, ToolError,
    ToolKind, ToolOutput, ToolRegistry, confine,
};
pub use usage::{StopReason, Usage};
