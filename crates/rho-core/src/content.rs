//! The message and content-block model.
//!
//! A message holds an ordered list of content blocks. The `type` tag drives
//! serialisation. This shape is stable. It is the on-disk session format.

use serde::{Deserialize, Serialize};

/// A base64-encoded image and its MIME type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageSource {
    /// Base64 payload with no data-URI prefix.
    pub data: String,
    /// MIME type, for example `image/png`.
    pub mime_type: String,
}

/// One typed unit of message content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Plain assistant or user text.
    Text { text: String },
    /// Model reasoning. `signature` carries a provider replay token when present.
    Thinking {
        thinking: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    /// A model request to call a tool. `arguments` is the parsed JSON object.
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    /// The result of a tool call. `content` holds only `Text` or `Image` blocks.
    ToolResult {
        tool_call_id: String,
        content: Vec<ContentBlock>,
        #[serde(default)]
        is_error: bool,
    },
    /// An image, in a user message or a tool result.
    Image { source: ImageSource },
}

/// The role of a conversation entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    Tool,
}

/// One conversation entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}
