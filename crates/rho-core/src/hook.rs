//! The hook trait and the hook chain.
//!
//! A hook is a compiled struct. It fires at defined points in the agent loop.
//! Sprint 1 defines two points. The loop calls them in registration order.

use crate::ToolOutput;
use async_trait::async_trait;
use std::sync::Arc;

/// A mutable view of a pending tool call, passed to `before_tool_call`.
pub struct ToolCallView<'a> {
    pub name: &'a str,
    /// The parsed arguments. A hook may edit them in place before execution.
    pub arguments: &'a mut serde_json::Value,
}

/// The outcome of a `before_tool_call` hook.
#[derive(Clone, Debug, PartialEq)]
pub enum HookOutcome {
    /// Let the call proceed to the next hook, then the tool.
    Continue,
    /// Stop the call. The loop makes an error tool result with this reason.
    Block { reason: String },
}

#[async_trait]
pub trait Hook: Send + Sync {
    /// A stable name for logs.
    fn name(&self) -> &str;

    /// Fires before a tool runs, after argument parsing. The hook may edit the
    /// arguments in place. The first `Block` stops the call.
    async fn before_tool_call(&self, _call: &mut ToolCallView<'_>) -> HookOutcome {
        HookOutcome::Continue
    }

    /// Fires after a tool finishes, before the result is appended. The hook may
    /// edit the output in place.
    async fn after_tool_result(&self, _name: &str, _output: &mut ToolOutput) {}
}

/// The hook chain. Registration order is the run order.
pub struct HookChain {
    hooks: Vec<Arc<dyn Hook>>,
}

impl HookChain {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }
    /// Add a hook. Registration order is the run order.
    pub fn push(&mut self, hook: Arc<dyn Hook>) {
        self.hooks.push(hook);
    }
    pub fn hooks(&self) -> &[Arc<dyn Hook>] {
        &self.hooks
    }
}

impl Default for HookChain {
    fn default() -> Self {
        Self::new()
    }
}
