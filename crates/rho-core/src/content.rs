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

/// Which provider and which model produced a replay payload.
///
/// A provider reads a payload only when this pair matches its own. So a model switch never
/// sends a foreign payload back. See `SPEC-reasoning-across-providers` rule 8.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasoningOwner {
    /// The value of `Provider::id`, for example `bedrock`.
    pub provider: String,
    /// The model id of the request that produced the payload.
    pub model: String,
}

/// One provider's own replay payload. Shared code never reads inside `value`.
///
/// `value` holds opaque provider bytes: a signature, an encrypted blob, an item id, or a
/// status. **No readable text.** Readable text lives in the block's `text`, where the
/// session cap and the reader both reach it. That is what makes the redaction exemption of
/// rule 9 safe. See `D-reasoning-replay-is-opaque-provider-state`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderState {
    /// The pair that may read the value.
    pub owner: ReasoningOwner,
    /// The provider's private shape.
    pub value: serde_json::Value,
}

impl ProviderState {
    /// The payload, but only for its own owner. `None` for anybody else.
    ///
    /// Rule 8 lives here, in one place, so no provider hand-rolls the comparison. fx keeps
    /// the same kind of payload with no owner at all, and its own code cannot tell one
    /// provider's payload from another's.
    pub fn for_owner(&self, provider: &str, model: &str) -> Option<&serde_json::Value> {
        if self.owner.provider == provider && self.owner.model == model {
            Some(&self.value)
        } else {
            None
        }
    }
}

/// One typed unit of message content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "DiskBlock", into = "DiskBlock")]
pub enum ContentBlock {
    /// Plain assistant or user text.
    Text { text: String },
    /// Readable reasoning, kept for the reader. **It never reaches a provider.**
    ReasoningTrace { text: String },
    /// Reasoning a provider needs echoed back. `state` is `None` when there is nothing to
    /// replay, and the block is then history with a name that says it could travel.
    ReasoningReplay {
        text: String,
        state: Option<ProviderState>,
    },
    /// A model request to call a tool. `arguments` is the parsed JSON object.
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
        /// The replay payload a provider binds to this call. Gemini rejects a request with
        /// `Function call is missing a thought_signature` when it is absent.
        state: Option<ProviderState>,
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

/// The on-disk shape of a content block. One `thinking` record carries both reasoning
/// variants.
///
/// Two enum variants **cannot** share one serde tag. Both compile, and the reader then
/// returns the first one every time, so a replay block came back as a trace with no error.
/// A compile proved that before any code shipped. See
/// `D-two-variants-cannot-share-a-serde-tag`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DiskBlock {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
        /// `true` for a replay block. A missing key reads as `false`, which never replays.
        #[serde(default, skip_serializing_if = "is_false")]
        replay: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state: Option<ProviderState>,
        /// Read from a file written before the split, and never written again.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state: Option<ProviderState>,
    },
    ToolResult {
        tool_call_id: String,
        content: Vec<ContentBlock>,
        #[serde(default)]
        is_error: bool,
    },
    Image {
        source: ImageSource,
    },
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl From<DiskBlock> for ContentBlock {
    fn from(disk: DiskBlock) -> Self {
        match disk {
            DiskBlock::Text { text } => ContentBlock::Text { text },
            DiskBlock::Thinking {
                thinking,
                replay,
                state,
                signature: _,
            } => match (replay, state) {
                // A payload and the flag together make a replay block.
                (true, Some(state)) => ContentBlock::ReasoningReplay {
                    text: thinking,
                    state: Some(state),
                },
                // `replay` with no payload has nothing to send, so it is history. rho says
                // so once, because a provider that forgets its payload must not hide. See
                // rule 11.
                (true, None) => {
                    tracing::warn!(
                        "a session record claims a reasoning replay and carries no payload, \
                         so it loads as history"
                    );
                    ContentBlock::ReasoningTrace { text: thinking }
                }
                // An old record, or a trace. Its signature is stale, so it is dropped.
                (false, _) => ContentBlock::ReasoningTrace { text: thinking },
            },
            DiskBlock::ToolCall {
                id,
                name,
                arguments,
                state,
            } => ContentBlock::ToolCall {
                id,
                name,
                arguments,
                state,
            },
            DiskBlock::ToolResult {
                tool_call_id,
                content,
                is_error,
            } => ContentBlock::ToolResult {
                tool_call_id,
                content,
                is_error,
            },
            DiskBlock::Image { source } => ContentBlock::Image { source },
        }
    }
}

impl From<ContentBlock> for DiskBlock {
    fn from(block: ContentBlock) -> Self {
        match block {
            ContentBlock::Text { text } => DiskBlock::Text { text },
            ContentBlock::ReasoningTrace { text } => DiskBlock::Thinking {
                thinking: text,
                replay: false,
                state: None,
                signature: None,
            },
            ContentBlock::ReasoningReplay { text, state } => DiskBlock::Thinking {
                thinking: text,
                // A payload is what makes a record replayable, so the flag follows it.
                replay: state.is_some(),
                state,
                signature: None,
            },
            ContentBlock::ToolCall {
                id,
                name,
                arguments,
                state,
            } => DiskBlock::ToolCall {
                id,
                name,
                arguments,
                state,
            },
            ContentBlock::ToolResult {
                tool_call_id,
                content,
                is_error,
            } => DiskBlock::ToolResult {
                tool_call_id,
                content,
                is_error,
            },
            ContentBlock::Image { source } => DiskBlock::Image { source },
        }
    }
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
