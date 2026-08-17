//! `rho-core` is the composable agent runtime core.
//!
//! It is a pure library. It links no HTTP client and no terminal. It defines the
//! message model, the streaming event model, the provider and tool traits, the
//! hook chain, the cancellation type, and the agent loop. Every other crate
//! builds on the types here.

mod agent;
mod cancel;
mod content;
mod context;
mod error;
mod event;
mod hook;
mod provider;
mod tool;
mod usage;

pub use agent::{AgentConfig, AgentEvent, AgentEvents, AgentStopReason, Session};
pub use cancel::CancelToken;
pub use content::{ContentBlock, ImageSource, Message, Role};
pub use context::Context;
pub use error::{Error, ProviderError};
pub use event::StreamEvent;
pub use hook::{Hook, HookChain, HookOutcome, ToolCallView};
pub use provider::{CompletionRequest, Provider, ProviderStream, ToolSpec};
pub use tool::{
    AllowAllPolicy, ApprovalDecision, ApprovalPolicy, ReadOnlyPolicy, Tool, ToolContext, ToolError,
    ToolKind, ToolOutput, ToolRegistry, confine,
};
pub use usage::{StopReason, Usage};
