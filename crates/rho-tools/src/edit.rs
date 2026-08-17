//! The `edit` tool. It replaces one exact text span in a file under the root.

use crate::args::parse_args;
use async_trait::async_trait;
use rho_core::{Tool, ToolContext, ToolError, ToolKind, ToolOutput, confine};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Arguments for `edit`.
#[derive(Debug, Serialize, Deserialize)]
struct EditArgs {
    /// The file path, relative to the session root.
    path: String,
    /// The exact text to replace. It must appear exactly once.
    old_text: String,
    /// The replacement text.
    new_text: String,
}

/// Replaces one exact text span in a file. It fails when `old_text` is absent,
/// and it fails when `old_text` appears more than once. A first-match
/// replacement of an ambiguous span is a data-loss bug, so this tool refuses it.
/// Mutating, so it needs approval.
pub struct EditTool;

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }
    fn description(&self) -> &str {
        "Replace one exact text span in a file. old_text must appear exactly \
         once. It fails when old_text is absent or not unique."
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path under the session root." },
                "old_text": { "type": "string", "description": "Exact text to replace. It must be unique in the file." },
                "new_text": { "type": "string", "description": "Replacement text." }
            },
            "required": ["path", "old_text", "new_text"]
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: EditArgs = parse_args(args)?;
        let path = confine(&ctx.session_root, Path::new(&args.path))?;

        let before = tokio::fs::read_to_string(&path).await.map_err(|error| {
            ToolError::Io(format!(
                "cannot read {}: {error}. Check the path exists.",
                path.display()
            ))
        })?;

        let matches = before.matches(&args.old_text).count();
        if matches == 0 {
            return Err(ToolError::InvalidArguments(format!(
                "old_text is not present in {}. Copy the exact text to replace.",
                path.display()
            )));
        }
        if matches > 1 {
            return Err(ToolError::InvalidArguments(format!(
                "old_text appears {matches} times in {}. Make old_text unique, \
                 for example by adding surrounding lines.",
                path.display()
            )));
        }

        let after = before.replacen(&args.old_text, &args.new_text, 1);
        tokio::fs::write(&path, after.as_bytes())
            .await
            .map_err(|error| ToolError::Io(format!("cannot write {}: {error}.", path.display())))?;

        // Build a unified diff so the model sees what changed.
        let diff = similar::TextDiff::from_lines(&before, &after)
            .unified_diff()
            .header("before", "after")
            .to_string();

        Ok(ToolOutput::text(format!(
            "edited {}\n{diff}",
            path.display()
        )))
    }
}
