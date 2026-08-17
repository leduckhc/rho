//! The `glob` tool. It matches files by a glob pattern under the session root.

use crate::args::parse_args;
use async_trait::async_trait;
use globset::{Glob, GlobSetBuilder};
use ignore::WalkBuilder;
use rho_core::{Tool, ToolContext, ToolError, ToolKind, ToolOutput};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Arguments for `glob`.
#[derive(Debug, Serialize, Deserialize)]
struct GlobArgs {
    /// The glob pattern, for example `src/**/*.rs`.
    pattern: String,
}

/// Matches files by a glob pattern. It walks the session root only. It does not
/// follow a symlink out of the root. It honours `.gitignore`. Read-only.
pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }
    fn description(&self) -> &str {
        "Match files by a glob pattern under the session root, for example \
         src/**/*.rs. It honours .gitignore."
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Glob pattern, for example src/**/*.rs." }
            },
            "required": ["pattern"]
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: GlobArgs = parse_args(args)?;
        let glob = Glob::new(&args.pattern).map_err(|error| {
            ToolError::InvalidArguments(format!(
                "the glob pattern is not valid: {error}. Check the pattern."
            ))
        })?;
        let mut builder = GlobSetBuilder::new();
        builder.add(glob);
        let set = builder.build().map_err(|error| {
            ToolError::InvalidArguments(format!("cannot build the glob: {error}."))
        })?;

        let root = ctx.session_root.clone();
        // The walk is synchronous, so run it off the async runtime.
        let matches = tokio::task::spawn_blocking(move || -> Vec<String> {
            let mut found: Vec<String> = Vec::new();
            // `follow_links(false)` stops the walk from leaving the root through
            // a symlink. `.gitignore` is honoured by default.
            for result in WalkBuilder::new(&root)
                .follow_links(false)
                .require_git(false)
                .build()
            {
                let Ok(entry) = result else { continue };
                let path: PathBuf = entry.path().to_path_buf();
                let Ok(rel) = path.strip_prefix(&root) else {
                    continue;
                };
                if rel.as_os_str().is_empty() {
                    continue;
                }
                if set.is_match(rel) {
                    found.push(rel.to_string_lossy().into_owned());
                }
            }
            found.sort();
            found
        })
        .await
        .map_err(|error| ToolError::Io(format!("the glob walk failed: {error}.")))?;

        Ok(ToolOutput::text(matches.join("\n")))
    }
}
