//! The bridge from an MCP tool to a `rho_core::Tool`.
//!
//! `tools_for` advertises tools from the schema cache, so the first provider
//! request already carries them. It returns at once and does not wait for a
//! handshake. Each tool holds a pooled handle, so a call connects on first use.
//! See `SPEC-mcp` sections 4, 5, and 6.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use rho_core::{ContentBlock, Tool, ToolContext, ToolError, ToolKind, ToolOutput};

use crate::cache::McpSchemaCache;
use crate::config::McpServerConfig;
use crate::error::McpError;
use crate::naming::{dispatch_name, validate_tool_name};
use crate::pool::{McpHandle, McpPool};

/// One MCP tool the agent loop calls like any built-in tool.
pub struct McpTool {
    /// The dispatch name, `mcp__<server>__<tool>`.
    dispatch_name: String,
    /// The raw tool name the server expects on the wire.
    server_tool_name: String,
    description: String,
    input_schema: serde_json::Value,
    handle: Arc<McpHandle>,
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.dispatch_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn kind(&self) -> ToolKind {
        // An MCP server does not classify its own tools. MCP has no `ToolKind`,
        // and a server's own opinion would not be trustworthy if it had one. So
        // every MCP tool reports `Other`, which `ToolKind::is_read_only` treats
        // as mutating. A read-only policy denies it, and any session needs an
        // explicit approval for it. This follows decision D-mcp-does-not-classify-itself, which applies
        // decision D-plugin-does-not-classify-itself to a second source. A later feature may let the user's
        // configuration grant a kind to a named MCP tool.
        ToolKind::Other
    }

    fn input_schema(&self) -> serde_json::Value {
        self.input_schema.clone()
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        // Wait for the handshake, then call. A failure becomes an error result,
        // not an `Err`, so a slow or broken server degrades one tool, never the
        // session. The model reads the message and can change course.
        let connection = match self.handle.connection().await {
            Ok(connection) => connection,
            Err(error) => return Ok(error_output(&self.dispatch_name, &error.to_string())),
        };
        match connection.call(&self.server_tool_name, args).await {
            Ok(output) => Ok(output),
            Err(error) => Ok(error_output(&self.dispatch_name, &error.to_string())),
        }
    }
}

/// Build `rho_core::Tool` objects for a set of servers, advertising from the
/// cache.
///
/// This returns at once. It does not wait for a handshake. It starts a
/// background connect for each server, so a later call connects on first use.
/// A duplicate final tool name is an error that names both servers.
pub async fn tools_for(
    pool: &Arc<McpPool>,
    configs: &[McpServerConfig],
    cache: &McpSchemaCache,
) -> Result<Vec<Arc<dyn Tool>>, McpError> {
    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
    // The final name maps to the server that first claimed it, so a collision
    // names both servers.
    let mut claimed: HashMap<String, String> = HashMap::new();

    for config in configs {
        let handle = Arc::new(pool.acquire(config).await?);
        for def in cache.tools_for(config) {
            validate_tool_name(&config.name, &def.name)?;
            let name = dispatch_name(&config.name, &def.name);
            if let Some(first) = claimed.get(&name)
                && first != &config.name
            {
                return Err(McpError::DuplicateToolName {
                    name,
                    first: first.clone(),
                    second: config.name.clone(),
                });
            }
            claimed.insert(name.clone(), config.name.clone());
            tools.push(Arc::new(McpTool {
                dispatch_name: name,
                server_tool_name: def.name.clone(),
                description: def.description.clone(),
                input_schema: def.input_schema.clone(),
                handle: Arc::clone(&handle),
            }));
        }
    }
    Ok(tools)
}

/// Build an error tool result that names the tool and the reason.
fn error_output(tool: &str, reason: &str) -> ToolOutput {
    ToolOutput {
        content: vec![ContentBlock::Text {
            text: format!("the MCP tool {tool} failed: {reason}"),
        }],
        is_error: true,
    }
}
