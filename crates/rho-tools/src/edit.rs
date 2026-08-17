//! The `edit` tool. It replaces an exact text span in a file under the root.

use crate::args::parse_args;
use async_trait::async_trait;
use rho_core::{Tool, ToolContext, ToolError, ToolKind, ToolOutput, confine};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// How many lines of context the result shows around the change.
const CONTEXT_LINES: usize = 3;

/// Arguments for `edit`.
///
/// The canonical names are `path`, `old_text`, and `new_text`, which match every other
/// tool in `rho-tools`.
///
/// Each one also accepts a familiar alias. Models are heavily trained on other
/// harnesses, which name these `file_path`, `old_string`, and `new_string`. A model
/// that reaches for a name it knows should not get a schema error, because the retry
/// costs a whole turn. So rho advertises its own names and accepts both.
#[derive(Debug, Serialize, Deserialize)]
struct EditArgs {
    /// The file path, relative to the session root.
    #[serde(alias = "file_path")]
    path: String,
    /// The exact text to replace.
    #[serde(alias = "old_string")]
    old_text: String,
    /// The replacement text.
    #[serde(alias = "new_string")]
    new_text: String,
    /// Replace every match instead of requiring exactly one.
    #[serde(default)]
    replace_all: bool,
}

/// Replaces an exact text span in a file.
///
/// By default `old_text` must appear exactly once, and an ambiguous span is refused,
/// because a silent first-match replacement is a data-loss bug. Set `replace_all` to
/// change every match on purpose.
///
/// Mutating, so it needs approval.
pub struct EditTool;

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }
    fn description(&self) -> &str {
        "Replace an exact text span in a file. By default old_text must appear \
         exactly once. Set replace_all to change every match."
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path under the session root." },
                "old_text": {
                    "type": "string",
                    "description": "Exact text to replace, including its indentation. \
                                    It must be unique unless replace_all is true."
                },
                "new_text": { "type": "string", "description": "Replacement text." },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace every match. Use it when the same span \
                                    repeats on purpose. Default false."
                }
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

        // An empty pattern matches at every character boundary, so `replace_all` would
        // rewrite the whole file: "abc" becomes "XaXbXcX". Another harness has this
        // defect. Refuse the call rather than corrupt the file.
        if args.old_text.is_empty() {
            return Err(ToolError::InvalidArguments(
                "old_text is empty. An empty span matches everywhere, so this would \
                 rewrite the whole file. Give the exact text to replace."
                    .to_string(),
            ));
        }

        // Equal texts mean the model believes it changed something. Reporting success
        // would be a silent failure, and it would waste a turn.
        if args.old_text == args.new_text {
            return Err(ToolError::InvalidArguments(
                "old_text and new_text must differ. The edit would change nothing.".to_string(),
            ));
        }

        let path = confine(&ctx.session_root, Path::new(&args.path))?;

        let before = tokio::fs::read_to_string(&path).await.map_err(|error| {
            ToolError::Io(format!(
                "cannot read {}: {error}. Check the path exists.",
                path.display()
            ))
        })?;

        let matches = before.matches(&args.old_text).count();
        if matches == 0 {
            return Err(explain_no_match(&before, &args.old_text, &path.display()));
        }
        if matches > 1 && !args.replace_all {
            return Err(ToolError::InvalidArguments(format!(
                "old_text appears {matches} times in {}. Either add surrounding lines \
                 to make it unique, or set replace_all to true to change every match.",
                path.display()
            )));
        }

        let after = if args.replace_all {
            before.replace(&args.old_text, &args.new_text)
        } else {
            before.replacen(&args.old_text, &args.new_text, 1)
        };
        tokio::fs::write(&path, after.as_bytes())
            .await
            .map_err(|error| ToolError::Io(format!("cannot write {}: {error}.", path.display())))?;

        let changed = if args.replace_all { matches } else { 1 };
        let diff = similar::TextDiff::from_lines(&before, &after)
            .unified_diff()
            .context_radius(CONTEXT_LINES)
            .header("before", "after")
            .to_string();

        Ok(ToolOutput::text(format!(
            "edited {}, {changed} span(s) replaced\n{diff}",
            path.display()
        )))
    }
}

/// Explain why an exact span did not match.
///
/// A bare "not found" is a dead end. The model has to guess whether the text is absent,
/// or present with different whitespace. So this looks for the near misses and names
/// the one it finds. That turns a wasted turn into a recoverable error.
fn explain_no_match(content: &str, old_text: &str, path: &impl std::fmt::Display) -> ToolError {
    // Near miss one: the span is there, but the model padded or trimmed it.
    let trimmed = old_text.trim();
    if !trimmed.is_empty() && trimmed != old_text && content.contains(trimmed) {
        return ToolError::InvalidArguments(format!(
            "old_text is not present in {path}, but it matches after whitespace is \
             trimmed. Copy the exact text from the file, with its leading and trailing \
             whitespace."
        ));
    }

    // Near miss two: every line matches once indentation is ignored. Report the line
    // where the block starts, so the model can read that region again.
    let wanted: Vec<&str> = old_text.lines().collect();
    if !wanted.is_empty() {
        let have: Vec<&str> = content.lines().collect();
        if have.len() >= wanted.len() {
            for (index, window) in have.windows(wanted.len()).enumerate() {
                let same = window
                    .iter()
                    .zip(wanted.iter())
                    .all(|(left, right)| left.trim() == right.trim());
                if same {
                    return ToolError::InvalidArguments(format!(
                        "old_text is not present in {path}, but the same lines appear \
                         with different indentation at line {}. Copy the exact \
                         indentation from the file.",
                        index + 1
                    ));
                }
            }
        }
    }

    ToolError::InvalidArguments(format!(
        "old_text is not present in {path}. Read the file again, then copy the exact \
         text to replace."
    ))
}
