//! Anthropic Server-Sent Events decoder.
//!
//! The wire is described in `SPEC-anthropic-messages-provider` section 4. Every event
//! comes as `(name, JSON)`. The decoder is stateful, because a `content_block_delta` alone
//! does not know whether its content is text, a tool call, or reasoning; the earlier
//! `content_block_start` decided that.
//!
//! This module exposes `Decoder`. The provider builds one per request, feeds each event
//! through it, and yields every `StreamEvent` the decoder returns to its consumer. A
//! rejected event returns `ProviderError::Decode`.

use rho_core::{ProviderError, Role, StopReason, StreamEvent};
use serde_json::Value;
use std::collections::HashMap;

/// What a currently-open content block holds. The block's `content_block_start` picked
/// this, and every `content_block_delta` on the same index dispatches by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpenBlock {
    Text,
    // Placeholder arms grow with the tests. A new arm is a compile error at the delta
    // match, which the launch amendment on fail-open events requires.
}

/// Anthropic SSE decoder.
///
/// A fresh decoder starts with no open blocks. Each `content_block_start` opens one, keyed
/// by the block's `index`; each `content_block_stop` closes it. A `content_block_delta`
/// dispatches by the open block's kind.
pub struct Decoder {
    /// Open content blocks, keyed by index. A `content_block_start` inserts. A
    /// `content_block_stop` removes.
    open: HashMap<u32, OpenBlock>,
    /// The stop reason reported on `message_delta`. `message_stop` emits it as `Done`.
    stop_reason: Option<StopReason>,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            open: HashMap::new(),
            stop_reason: None,
        }
    }

    /// Feed one SSE event into the decoder. Returns the `StreamEvent`s the event produces.
    /// An unknown content-bearing event returns `ProviderError::Decode`.
    pub fn on_event(
        &mut self,
        name: &str,
        data: &Value,
    ) -> Result<Vec<StreamEvent>, ProviderError> {
        match name {
            "message_start" => Ok(self.on_message_start(data)?),
            "content_block_start" => Ok(self.on_content_block_start(data)?),
            "content_block_delta" => Ok(self.on_content_block_delta(data)?),
            "content_block_stop" => Ok(self.on_content_block_stop(data)?),
            "message_delta" => {
                self.on_message_delta(data)?;
                Ok(Vec::new())
            }
            "message_stop" => Ok(self.on_message_stop()),
            // Framing events. The launch amendment names ping explicitly. `error` is
            // handled by the streaming layer, not here.
            "ping" => Ok(Vec::new()),
            // Any other name is content-bearing until proven otherwise. rho does not
            // guess. See the amendment on unknown events.
            other => Err(ProviderError::Decode(format!(
                "unknown anthropic SSE event: {other}"
            ))),
        }
    }

    fn on_message_start(&mut self, data: &Value) -> Result<Vec<StreamEvent>, ProviderError> {
        let role = data
            .pointer("/message/role")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProviderError::Decode("message_start has no message.role".to_string())
            })?;
        let role = match role {
            "assistant" => Role::Assistant,
            other => {
                return Err(ProviderError::Decode(format!(
                    "message_start.role is not assistant: {other}"
                )));
            }
        };
        Ok(vec![StreamEvent::MessageStart { role }])
    }

    fn on_content_block_start(&mut self, data: &Value) -> Result<Vec<StreamEvent>, ProviderError> {
        let index = require_index(data)?;
        let kind = data
            .pointer("/content_block/type")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProviderError::Decode("content_block_start has no content_block.type".to_string())
            })?;
        match kind {
            "text" => {
                self.open.insert(index, OpenBlock::Text);
                Ok(vec![StreamEvent::TextStart { index }])
            }
            other => Err(ProviderError::Decode(format!(
                "content_block_start type not yet supported: {other}"
            ))),
        }
    }

    fn on_content_block_delta(&mut self, data: &Value) -> Result<Vec<StreamEvent>, ProviderError> {
        let index = require_index(data)?;
        let open = self.open.get(&index).copied().ok_or_else(|| {
            ProviderError::Decode(format!("content_block_delta for closed index {index}"))
        })?;
        let delta_kind = data
            .pointer("/delta/type")
            .and_then(Value::as_str)
            .ok_or_else(|| ProviderError::Decode("delta has no type".to_string()))?;
        match (open, delta_kind) {
            (OpenBlock::Text, "text_delta") => {
                let text = data
                    .pointer("/delta/text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ProviderError::Decode("text_delta has no text".to_string()))?;
                Ok(vec![StreamEvent::TextDelta {
                    index,
                    delta: text.to_string(),
                }])
            }
            (open, delta) => Err(ProviderError::Decode(format!(
                "delta {delta} does not fit an open {open:?} block"
            ))),
        }
    }

    fn on_content_block_stop(&mut self, data: &Value) -> Result<Vec<StreamEvent>, ProviderError> {
        let index = require_index(data)?;
        let open = self.open.remove(&index).ok_or_else(|| {
            ProviderError::Decode(format!("content_block_stop for closed index {index}"))
        })?;
        match open {
            OpenBlock::Text => Ok(vec![StreamEvent::TextEnd { index }]),
        }
    }

    fn on_message_delta(&mut self, data: &Value) -> Result<(), ProviderError> {
        if let Some(reason) = data.pointer("/delta/stop_reason").and_then(Value::as_str) {
            self.stop_reason = Some(map_stop_reason(reason));
        }
        Ok(())
    }

    fn on_message_stop(&mut self) -> Vec<StreamEvent> {
        // If no reason was reported, treat it as an end turn. A missing reason is not an
        // error at this layer; the stream ended, the caller sees Done, and the turn ends.
        let reason = self.stop_reason.take().unwrap_or(StopReason::EndTurn);
        vec![StreamEvent::Done {
            stop_reason: reason,
        }]
    }
}

fn require_index(data: &Value) -> Result<u32, ProviderError> {
    data.get("index")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| ProviderError::Decode("event has no valid index".to_string()))
}

fn map_stop_reason(reason: &str) -> StopReason {
    match reason {
        "end_turn" => StopReason::EndTurn,
        "tool_use" => StopReason::ToolUse,
        "max_tokens" => StopReason::MaxTokens,
        "stop_sequence" => StopReason::StopSequence,
        // A future stop reason lands here. rho maps unknowns to EndTurn rather than
        // failing the stream mid-answer; a hostile reason cannot smuggle a wrong
        // classification because the stream layer has already produced the answer text.
        _ => StopReason::EndTurn,
    }
}
