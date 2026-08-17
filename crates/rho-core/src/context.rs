//! The append-only conversation log for one session.
//!
//! The `Context` is append-only by construction. It exposes append and read. It
//! exposes no edit and no remove. This keeps the provider prompt prefix stable.

use crate::{Message, ToolSpec};

/// The append-only conversation log for one session.
#[derive(Clone, Debug, Default)]
pub struct Context {
    system: Option<String>,
    tools: Vec<ToolSpec>,
    messages: Vec<Message>,
}

impl Context {
    pub fn new(system: Option<String>, tools: Vec<ToolSpec>) -> Self {
        Self {
            system,
            tools,
            messages: Vec::new(),
        }
    }

    /// Append one message. This is the only way to add to the log.
    pub fn append(&mut self, message: Message) {
        self.messages.push(message);
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn system(&self) -> Option<&str> {
        self.system.as_deref()
    }

    pub fn tools(&self) -> &[ToolSpec] {
        &self.tools
    }
}
