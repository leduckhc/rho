//! The `read` tool. It reads a UTF-8 text file under the session root.

use crate::args::parse_args;
use async_trait::async_trait;
use rho_core::{Tool, ToolContext, ToolError, ToolKind, ToolOutput, confine};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The largest text `read` returns. Output over this size is truncated, and the
/// result states that. This bounds memory for a very large file.
const MAX_READ_BYTES: usize = 100_000;

/// Arguments for `read`.
#[derive(Debug, Serialize, Deserialize)]
struct ReadArgs {
    /// The file path, relative to the session root.
    ///
    /// Accepts the alias `file_path`, because models trained on other harnesses reach
    /// for that name. A schema error there costs a whole turn. See `edit.rs`.
    #[serde(alias = "file_path")]
    path: String,
    /// The first line to return, one-based. The default is line one.
    #[serde(default)]
    offset: Option<usize>,
    /// The largest number of lines to return. The default is every line.
    #[serde(default)]
    limit: Option<usize>,
}

/// Reads a text file. Read-only, so a read-only policy allows it.
pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }
    fn description(&self) -> &str {
        "Read a UTF-8 text file. Give a path under the session root. \
         Set offset and limit to read a line range."
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path under the session root." },
                "offset": { "type": "integer", "minimum": 1, "description": "First line to read, one-based." },
                "limit": { "type": "integer", "minimum": 1, "description": "Largest number of lines to read." }
            },
            "required": ["path"]
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: ReadArgs = parse_args(args)?;
        let path = confine(&ctx.session_root, Path::new(&args.path))?;
        read_file(&path, args.offset, args.limit).await
    }
}

/// Read the file, slice the line range, and cap the returned bytes.
async fn read_file(
    path: &Path,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<ToolOutput, ToolError> {
    let metadata = tokio::fs::metadata(path).await.map_err(|error| {
        ToolError::Io(format!(
            "cannot read {}: {error}. Check the path exists.",
            path.display()
        ))
    })?;
    if metadata.is_dir() {
        return Err(ToolError::InvalidArguments(format!(
            "{} is a directory. Give a file path, or use the list tool.",
            path.display()
        )));
    }

    let bytes = tokio::fs::read(path)
        .await
        .map_err(|error| ToolError::Io(format!("cannot read {}: {error}.", path.display())))?;
    let text = String::from_utf8(bytes).map_err(|_| {
        ToolError::InvalidArguments(format!(
            "{} is not UTF-8 text. This tool reads text files only.",
            path.display()
        ))
    })?;

    let start = offset.unwrap_or(1).saturating_sub(1);
    let selected: Vec<&str> = match limit {
        Some(limit) => text.lines().skip(start).take(limit).collect(),
        None => text.lines().skip(start).collect(),
    };
    let mut body = selected.join("\n");

    let mut truncated = false;
    if body.len() > MAX_READ_BYTES {
        // Cut on a char boundary so the string stays valid UTF-8.
        let mut cut = MAX_READ_BYTES;
        while !body.is_char_boundary(cut) {
            cut -= 1;
        }
        body.truncate(cut);
        truncated = true;
    }
    if truncated {
        body.push_str(&format!(
            "\n[truncated: output over {MAX_READ_BYTES} bytes. Use offset and limit to read a range.]"
        ));
    }

    Ok(ToolOutput::text(body))
}
