//! The `read_tool_result` tool. It reads a result the harness stored outside the context.
//!
//! See `docs/specs/20260822-103220-SPEC-tool-result-handle.md` section 7.
//!
//! The tool exists because a cap without a read-back forces the model to run a command twice.
//! It is read-only: it returns evidence rho already gathered, and changes nothing.

use crate::args::parse_args;
use async_trait::async_trait;
use rho_core::{
    ResultLimits, ResultStore, ResultStoreError, Tool, ToolContext, ToolError, ToolKind, ToolOutput,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Arguments for `read_tool_result`.
#[derive(Debug, Serialize, Deserialize)]
struct ReadToolResultArgs {
    /// The handle, copied exactly from a `tool_result_preview` block.
    handle: String,
    /// Where to start reading. The default is the start of the result.
    #[serde(default)]
    start_byte: Option<usize>,
    /// How many bytes to read. Clamped to the read maximum.
    #[serde(default)]
    byte_count: Option<usize>,
    /// A literal string to find. When present, the range arguments are ignored.
    #[serde(default)]
    query: Option<String>,
}

/// Reads a stored tool result by handle, as a byte range or a literal search.
pub struct ReadToolResultTool {
    store: Arc<dyn ResultStore>,
    limits: ResultLimits,
}

impl ReadToolResultTool {
    /// Build the tool over one session's store.
    ///
    /// A caller registers this only when the session has a store. Advertising a tool that
    /// always fails would spend schema bytes every turn and teach the model a false capability.
    pub fn new(store: Arc<dyn ResultStore>, limits: ResultLimits) -> Self {
        Self { store, limits }
    }
}

#[async_trait]
impl Tool for ReadToolResultTool {
    fn name(&self) -> &str {
        "read_tool_result"
    }

    fn description(&self) -> &str {
        "Read a large tool result that rho stored outside the context. Use it when a result \
         came back as a tool_result_preview block and you need evidence beyond the preview. \
         Pass the handle exactly as the preview gave it. Use query to find a literal string, \
         or start_byte and byte_count to read a range. Do not use it for a result you can \
         already see, for a file on disk, or to re-run a command."
    }

    fn kind(&self) -> ToolKind {
        // It reads evidence rho already holds and changes nothing, so a read-only policy
        // allows it.
        ToolKind::Read
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "handle": {
                    "type": "string",
                    "description": "The handle from a tool_result_preview block, copied exactly."
                },
                "start_byte": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Where to start. Defaults to the start of the result."
                },
                "byte_count": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "How many bytes to read. Clamped to the read maximum."
                },
                "query": {
                    "type": "string",
                    "description": "A literal string to find. Ignores start_byte and byte_count."
                }
            },
            "required": ["handle"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: ReadToolResultArgs = parse_args(args)?;

        if let Some(query) = args.query.as_deref() {
            let matches = self
                .store
                .search(&args.handle, query, self.limits.max_matches)
                .await
                .map_err(|error| explain(&args.handle, error))?;

            if matches.is_empty() {
                return Ok(ToolOutput::text(format!(
                    "No line of {} contains {query:?}.",
                    args.handle
                )));
            }
            let mut out = format!(
                "{} match(es) for {query:?} in {}:\n",
                matches.len(),
                args.handle
            );
            for found in &matches {
                out.push_str(&format!(
                    "{}:{} {}\n",
                    found.line_number, found.start_byte, found.line
                ));
            }
            return Ok(ToolOutput::text(out));
        }

        let start = args.start_byte.unwrap_or(0);
        let count = args
            .byte_count
            .unwrap_or(self.limits.read_default_bytes)
            .min(self.limits.read_max_bytes);

        let slice = self
            .store
            .read_range(&args.handle, start, count)
            .await
            .map_err(|error| explain(&args.handle, error))?;

        if slice.text.is_empty() {
            return Ok(ToolOutput::text(format!(
                "No bytes at {start} in {}. The whole result is {} bytes, so there is nothing \
                 after that offset.",
                args.handle, slice.total_bytes
            )));
        }

        Ok(ToolOutput::text(format!(
            "<tool_result handle=\"{}\" start_byte=\"{}\" end_byte=\"{}\" total_bytes=\"{}\">\n\
             {}\n</tool_result>",
            args.handle, slice.start_byte, slice.end_byte, slice.total_bytes, slice.text
        )))
    }
}

/// Turn a store error into a message that says what to do about it.
///
/// A wrong handle is the likeliest mistake, so its message says where a handle comes from and
/// that it belongs to one session.
fn explain(handle: &str, error: ResultStoreError) -> ToolError {
    match error {
        ResultStoreError::MalformedHandle(_) => ToolError::InvalidArguments(format!(
            "{handle:?} is not a result handle. A handle looks like \
             tr-0123456789abcdef-000001 and must be copied exactly from a tool_result_preview \
             block."
        )),
        ResultStoreError::NotFound(_) => ToolError::InvalidArguments(format!(
            "No stored result has handle {handle}. A handle belongs to the session that made \
             it, and it must be copied exactly from a tool_result_preview block in this \
             conversation."
        )),
        ResultStoreError::Io(detail) => {
            ToolError::Io(format!("cannot read stored result {handle}: {detail}"))
        }
    }
}
