//! The `write` tool. It writes a whole file under the session root.

use crate::args::parse_args;
use async_trait::async_trait;
use rho_core::{Tool, ToolContext, ToolError, ToolKind, ToolOutput, confine};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Arguments for `write`.
#[derive(Debug, Serialize, Deserialize)]
struct WriteArgs {
    /// The file path, relative to the session root.
    path: String,
    /// The whole file content.
    content: String,
}

/// Writes a whole file. It creates parent directories. It overwrites an existing
/// file. Mutating, so it needs approval.
pub struct WriteTool;

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }
    fn description(&self) -> &str {
        "Write a whole file under the session root. It creates parent \
         directories. It overwrites an existing file."
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path under the session root." },
                "content": { "type": "string", "description": "The whole file content." }
            },
            "required": ["path", "content"]
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: WriteArgs = parse_args(args)?;
        let path = confine(&ctx.session_root, Path::new(&args.path))?;

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                ToolError::Io(format!(
                    "cannot create the parent directory of {}: {error}.",
                    path.display()
                ))
            })?;
        }
        let bytes = args.content.len();
        tokio::fs::write(&path, args.content.as_bytes())
            .await
            .map_err(|error| ToolError::Io(format!("cannot write {}: {error}.", path.display())))?;

        Ok(ToolOutput::text(format!(
            "wrote {bytes} bytes to {}",
            path.display()
        )))
    }
}
