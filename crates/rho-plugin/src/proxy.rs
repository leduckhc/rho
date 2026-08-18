//! The host-side proxy that presents a plugin tool as a `rho_core::Tool`.

use std::sync::Arc;

use async_trait::async_trait;
use rho_core::{Tool, ToolContext, ToolError, ToolKind, ToolOutput};

use crate::cache::PluginToolSpec;
use crate::process::PluginProcess;

/// A plugin tool the agent loop calls like any built-in tool.
///
/// It forwards the call to its plugin process. A plugin failure becomes an error
/// `ToolOutput`, not an `Err`. So a crashed plugin does not abort the agent run.
/// The model sees the error result and can change course. See `SPEC-hooks-and-plugins` 4.6.
pub struct PluginTool {
    process: Arc<PluginProcess>,
    spec: PluginToolSpec,
}

impl PluginTool {
    pub(crate) fn new(process: Arc<PluginProcess>, spec: PluginToolSpec) -> Self {
        Self { process, spec }
    }
}

#[async_trait]
impl Tool for PluginTool {
    fn name(&self) -> &str {
        &self.spec.name
    }
    fn description(&self) -> &str {
        &self.spec.description
    }
    fn kind(&self) -> ToolKind {
        // A plugin does not get to classify itself.
        //
        // `ToolKind` drives the approval boundary. `ReadOnlyPolicy` allows a
        // read-only kind and denies everything else. If the host trusted the kind a
        // plugin advertises, then a hostile or compromised plugin would declare a
        // destructive tool as `Read` and run under a read-only policy. That is the
        // same fail-open shape as decision D-todo-in-a-green-stage, only now the value arrives from
        // another process.
        //
        // So the host reports `Other`, which `ToolKind::is_read_only` treats as
        // mutating. A plugin tool therefore needs an explicit approval, and a
        // read-only session denies it.
        //
        // The plugin's own claim stays in `self.spec.kind`. Nothing reads it today.
        // A frontend may later show it, clearly marked as the plugin's own claim.
        //
        // A later feature may let the **user's** configuration grant a kind to a
        // named plugin tool. The trust would then come from the user, not from the
        // plugin. See `SPEC-hooks-and-plugins`.
        ToolKind::Other
    }

    fn input_schema(&self) -> serde_json::Value {
        self.spec.input_schema.clone()
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        match self
            .process
            .call_tool(&self.spec.name, args, ctx.updates, ctx.cancel)
            .await
        {
            Ok(output) => Ok(output),
            // Turn a plugin failure into an error result, so the session
            // continues. The model reads the message and can try another path.
            Err(error) => Ok(error_output(&self.spec.name, &error.to_string())),
        }
    }
}

/// A placeholder tool advertised from the schema cache before the plugin
/// connects. A call to it returns an error result that says the tool is not
/// ready yet. The tool list shape does not change when the plugin connects, so
/// the prompt prefix stays stable. See `SPEC-hooks-and-plugins` section 5.
pub struct CachedTool {
    spec: PluginToolSpec,
}

impl CachedTool {
    pub(crate) fn new(spec: PluginToolSpec) -> Self {
        Self { spec }
    }
}

#[async_trait]
impl Tool for CachedTool {
    fn name(&self) -> &str {
        &self.spec.name
    }
    fn description(&self) -> &str {
        &self.spec.description
    }
    fn kind(&self) -> ToolKind {
        // A plugin does not get to classify itself.
        //
        // `ToolKind` drives the approval boundary. `ReadOnlyPolicy` allows a
        // read-only kind and denies everything else. If the host trusted the kind a
        // plugin advertises, then a hostile or compromised plugin would declare a
        // destructive tool as `Read` and run under a read-only policy. That is the
        // same fail-open shape as decision D-todo-in-a-green-stage, only now the value arrives from
        // another process.
        //
        // So the host reports `Other`, which `ToolKind::is_read_only` treats as
        // mutating. A plugin tool therefore needs an explicit approval, and a
        // read-only session denies it.
        //
        // The plugin's own claim stays in `self.spec.kind`. Nothing reads it today.
        // A frontend may later show it, clearly marked as the plugin's own claim.
        //
        // A later feature may let the **user's** configuration grant a kind to a
        // named plugin tool. The trust would then come from the user, not from the
        // plugin. See `SPEC-hooks-and-plugins`.
        ToolKind::Other
    }

    fn input_schema(&self) -> serde_json::Value {
        self.spec.input_schema.clone()
    }
    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        Ok(error_output(
            &self.spec.name,
            "the plugin has not connected yet. Try again shortly.",
        ))
    }
}

/// Build an error tool result that names the tool and the reason.
fn error_output(tool: &str, reason: &str) -> ToolOutput {
    ToolOutput {
        content: vec![rho_core::ContentBlock::Text {
            text: format!("the plugin tool {tool} failed: {reason}"),
        }],
        is_error: true,
    }
}
