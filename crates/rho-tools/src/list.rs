//! The `list` tool. It lists one directory under the session root.

use crate::args::parse_args;
use async_trait::async_trait;
use rho_core::{Tool, ToolContext, ToolError, ToolKind, ToolOutput, confine};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Arguments for `list`.
#[derive(Debug, Serialize, Deserialize)]
struct ListArgs {
    /// The directory path, relative to the session root. The default is the
    /// root itself.
    #[serde(default)]
    path: Option<String>,
}

/// Lists a directory. Each directory entry ends with a slash. The list is
/// sorted. Read-only.
pub struct ListTool;

#[async_trait]
impl Tool for ListTool {
    fn name(&self) -> &str {
        "list"
    }
    fn description(&self) -> &str {
        "List a directory under the session root. A directory entry ends with a \
         slash. The default path is the root."
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Directory path under the session root." }
            }
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: ListArgs = parse_args(args)?;
        let rel = args.path.unwrap_or_default();
        let path = confine(&ctx.session_root, Path::new(&rel))?;

        let mut read_dir = tokio::fs::read_dir(&path).await.map_err(|error| {
            ToolError::Io(format!(
                "cannot list {}: {error}. Check the path is a directory.",
                path.display()
            ))
        })?;

        let mut entries: Vec<String> = Vec::new();
        while let Some(entry) = read_dir.next_entry().await.map_err(|error| {
            ToolError::Io(format!(
                "cannot read an entry of {}: {error}.",
                path.display()
            ))
        })? {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
            entries.push(if is_dir { format!("{name}/") } else { name });
        }
        entries.sort();

        Ok(ToolOutput::text(entries.join("\n")))
    }
}
