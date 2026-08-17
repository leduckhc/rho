//! The `grep` tool. It searches file contents by a regular expression.

use crate::args::parse_args;
use async_trait::async_trait;
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use regex::Regex;
use rho_core::{Tool, ToolContext, ToolError, ToolKind, ToolOutput, confine};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The largest file `grep` reads. It skips a larger file, so one huge file does
/// not stall the search.
const MAX_GREP_FILE_BYTES: u64 = 5_000_000;

/// Arguments for `grep`.
#[derive(Debug, Serialize, Deserialize)]
struct GrepArgs {
    /// The regular expression to match on each line.
    pattern: String,
    /// The directory to search, relative to the session root. The default is
    /// the root.
    #[serde(default)]
    path: Option<String>,
    /// An optional glob to filter the files searched.
    #[serde(default)]
    glob: Option<String>,
}

/// Searches file contents by a regex. It walks the session root only. It does
/// not follow a symlink out of the root. It honours `.gitignore`. Read-only.
pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }
    fn description(&self) -> &str {
        "Search file contents by a regular expression under the session root. \
         It honours .gitignore. Set glob to filter the files."
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Regular expression to match on each line." },
                "path": { "type": "string", "description": "Directory under the session root to search." },
                "glob": { "type": "string", "description": "Glob to filter the files, for example *.rs." }
            },
            "required": ["pattern"]
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: GrepArgs = parse_args(args)?;
        let regex = Regex::new(&args.pattern).map_err(|error| {
            ToolError::InvalidArguments(format!(
                "the regular expression is not valid: {error}. Check the pattern."
            ))
        })?;

        let rel = args.path.clone().unwrap_or_default();
        let search_root = confine(&ctx.session_root, Path::new(&rel))?;

        let glob_set: Option<GlobSet> = match &args.glob {
            Some(pattern) => {
                let glob = Glob::new(pattern).map_err(|error| {
                    ToolError::InvalidArguments(format!(
                        "the glob pattern is not valid: {error}. Check the glob."
                    ))
                })?;
                let mut builder = GlobSetBuilder::new();
                builder.add(glob);
                Some(builder.build().map_err(|error| {
                    ToolError::InvalidArguments(format!("cannot build the glob: {error}."))
                })?)
            }
            None => None,
        };

        let matches = tokio::task::spawn_blocking(move || -> Vec<String> {
            let mut found: Vec<String> = Vec::new();
            for result in WalkBuilder::new(&search_root)
                .follow_links(false)
                .require_git(false)
                .build()
            {
                let Ok(entry) = result else { continue };
                if !entry.file_type().is_some_and(|t| t.is_file()) {
                    continue;
                }
                let path = entry.path();
                let rel = path.strip_prefix(&search_root).unwrap_or(path);
                if let Some(set) = &glob_set
                    && !set.is_match(rel)
                {
                    continue;
                }
                // Skip a very large file, so one huge file does not stall.
                if entry.metadata().map(|m| m.len()).unwrap_or(0) > MAX_GREP_FILE_BYTES {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(path) else {
                    // A non-UTF-8 file is not text. Skip it.
                    continue;
                };
                for (line_number, line) in text.lines().enumerate() {
                    if regex.is_match(line) {
                        found.push(format!("{}:{}:{line}", rel.display(), line_number + 1));
                    }
                }
            }
            found
        })
        .await
        .map_err(|error| ToolError::Io(format!("the grep walk failed: {error}.")))?;

        Ok(ToolOutput::text(matches.join("\n")))
    }
}
