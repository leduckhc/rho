//! The bounded steering queue.
//!
//! A user types while the agent runs. That message must not be dropped, and it
//! must not race into the middle of a provider request. So rho holds it here and
//! the driver delivers it at a turn boundary. See
//! `docs/specs/20260818-014343-SPEC-steering.md`.
//!
//! The queue is bounded on purpose. An unbounded queue is a memory defect, and
//! this project already shipped one: an unbounded `bash` reader turned 8 MB of
//! output into 805 MB of memory. See decisions D-bash-line-cap and
//! D-bounded-steering-queue.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::ContentBlock;

/// The capacity of the steering queue. A full queue rejects a new message.
pub const STEER_QUEUE_CAPACITY: usize = 32;

/// The largest one message a queue accepts, in bytes.
///
/// A queue built with [`MessageQueue::new`] or [`MessageQueue::with_capacity`] uses this
/// value. A person may paste a stack trace into a session queue, so it is roomy. A child
/// queue uses `SubagentLimits::max_steer_message_bytes`, which is smaller, because a model
/// writes those messages and there may be 160 such queues. See decision
/// D-a-steering-message-is-bounded-by-bytes.
pub const MAX_STEER_MESSAGE_BYTES: usize = 64 * 1024;

/// A typed queue error.
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum QueueError {
    /// The queue is full. The caller must tell the user, and it must not drop an
    /// earlier message.
    #[error(
        "the steering queue is full at {capacity} messages. Wait for the agent to \
         read them, or cancel the run."
    )]
    Full { capacity: usize },
    /// One message is larger than this queue accepts.
    ///
    /// A count cap bounds nothing on its own, because one message can be any size. The
    /// refusal names both numbers, so the writer knows how much to cut.
    #[error(
        "the message is {size} bytes and the limit is {limit} bytes. Send a shorter \
         message, or write the detail to a file and name the file."
    )]
    TooLarge { limit: usize, size: usize },
}

/// A bounded, ordered queue of user messages that arrive while a turn runs.
///
/// A clone shares the same queue, so a frontend and the driver hold one queue.
#[derive(Clone)]
pub struct MessageQueue {
    inner: Arc<QueueInner>,
}

struct QueueInner {
    messages: Mutex<VecDeque<Vec<ContentBlock>>>,
    capacity: usize,
    /// The largest one message may be. There is no queue without this bound.
    max_message_bytes: usize,
    /// Where to announce a push, while a run is happening.
    ///
    /// The announcement lives here, not in `Session::steer`, so **every** pusher
    /// announces. A subagent is steered through `LiveAgent::steer`, which pushes
    /// straight into this queue, and a frontend must see that too. Putting the
    /// announcement in one caller left `MessageQueued` unemitted for every other.
    observer: Mutex<Option<tokio::sync::mpsc::Sender<crate::AgentEvent>>>,
}

impl std::fmt::Debug for MessageQueue {
    /// It prints the length and the capacity, never the message bodies.
    ///
    /// A queued message is user input, and user input can hold a secret. A derived
    /// `Debug` would put it into any log that formats a handle. See decision
    /// D-redact-tool-arguments for the same rule applied to tool arguments.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageQueue")
            .field("queued", &self.len())
            .field("capacity", &self.inner.capacity)
            .field("max_message_bytes", &self.inner.max_message_bytes)
            .finish()
    }
}

impl Default for MessageQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageQueue {
    /// A queue with the default capacity.
    pub fn new() -> Self {
        Self::with_capacity(STEER_QUEUE_CAPACITY)
    }

    /// A queue with a stated capacity and the default byte cap.
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_limits(capacity, MAX_STEER_MESSAGE_BYTES)
    }

    /// A queue with a stated capacity and a stated byte cap for one message.
    ///
    /// There is no constructor that makes a queue with no byte cap, because a
    /// constructor that opts out of a bound is the shape that already cost this project
    /// 805 MB of memory once.
    pub fn with_limits(capacity: usize, max_message_bytes: usize) -> Self {
        Self {
            inner: Arc::new(QueueInner {
                messages: Mutex::new(VecDeque::new()),
                capacity,
                max_message_bytes,
                observer: Mutex::new(None),
            }),
        }
    }

    /// The byte cap this queue applies to one message.
    pub fn max_message_bytes(&self) -> usize {
        self.inner.max_message_bytes
    }

    /// Enqueue one message at the back.
    ///
    /// It returns `Err(Full)` when the queue is full, and `Err(TooLarge)` when the
    /// message is over this queue's byte cap. It never blocks, it never drops an earlier
    /// message to make room, and it never shortens a message. On success it returns the
    /// new queue length, which is the message's position counted from one.
    ///
    /// **The byte cap lives here, and not in a caller.** This is the one door into the
    /// queue: a frontend, `Session::steer`, `LiveAgent::steer`,
    /// `AgentRegistry::steer_descendant`, and the grace warning all arrive here. A cap in
    /// one caller would leave the others open. See decision
    /// D-a-steering-message-is-bounded-by-bytes.
    pub fn push(&self, message: Vec<ContentBlock>) -> Result<usize, QueueError> {
        let size = message_bytes(&message);
        if size > self.inner.max_message_bytes {
            return Err(QueueError::TooLarge {
                limit: self.inner.max_message_bytes,
                size,
            });
        }
        let position = {
            let mut messages = self.lock();
            if messages.len() >= self.inner.capacity {
                return Err(QueueError::Full {
                    capacity: self.inner.capacity,
                });
            }
            messages.push_back(message);
            messages.len()
        };
        // Announce outside the lock, so a slow receiver cannot block a pusher.
        self.announce(position);
        Ok(position)
    }

    /// Send `MessageQueued` while a run is happening.
    ///
    /// A closed or full channel is not an error. The run is over, or nobody is
    /// listening, and the message stays queued either way.
    fn announce(&self, position: usize) {
        if let Ok(slot) = self.inner.observer.lock()
            && let Some(sender) = slot.as_ref()
        {
            let _ = sender.try_send(crate::AgentEvent::MessageQueued { position });
        }
    }

    /// Announce each push on `sender` until [`MessageQueue::unobserve`].
    ///
    /// A session calls this when a run starts, so a frontend sees a queued message
    /// as pending rather than lost.
    pub fn observe(&self, sender: tokio::sync::mpsc::Sender<crate::AgentEvent>) {
        if let Ok(mut slot) = self.inner.observer.lock() {
            *slot = Some(sender);
        }
    }

    /// Stop announcing. A session calls this when a run ends.
    pub fn unobserve(&self) {
        if let Ok(mut slot) = self.inner.observer.lock() {
            *slot = None;
        }
    }

    /// Take every queued message in arrival order, and clear the queue.
    pub fn drain(&self) -> Vec<Vec<ContentBlock>> {
        self.lock().drain(..).collect()
    }

    /// Drop every queued message. A frontend calls this for a clean slate.
    ///
    /// A cancel must **not** call this. Dropping user input in silence is the worse
    /// failure, so the drop is always explicit. See D-bounded-steering-queue.
    pub fn clear(&self) {
        self.lock().clear();
    }

    /// The number of queued messages.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// True when the queue holds nothing.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<Vec<ContentBlock>>> {
        self.inner
            .messages
            .lock()
            .expect("the steering queue lock is poisoned")
    }
}

/// The bytes one message holds.
///
/// It counts the text of every block, the base64 payload of an image, the strings inside
/// tool-call arguments, and every `ProviderState` value. A block whose payload the count
/// skips is a place a large body hides, so the count skips none. A review found both
/// `state` fields missing from the first draft of this function.
///
/// The count walks a JSON value rather than serialising it, so a push allocates nothing
/// but a number. It counts the punctuation a writer would need, so it is never smaller
/// than the strings the value holds.
pub fn message_bytes(message: &[ContentBlock]) -> usize {
    message.iter().map(block_bytes).sum()
}

/// The bytes one block holds. Every variant is named, so a new one cannot be forgotten.
fn block_bytes(block: &ContentBlock) -> usize {
    match block {
        ContentBlock::Text { text } | ContentBlock::ReasoningTrace { text } => text.len(),
        ContentBlock::ReasoningReplay { text, state } => {
            text.len() + state.as_ref().map_or(0, state_bytes)
        }
        ContentBlock::ToolCall {
            id,
            name,
            arguments,
            state,
        } => id.len() + name.len() + json_bytes(arguments) + state.as_ref().map_or(0, state_bytes),
        ContentBlock::ToolResult {
            tool_call_id,
            content,
            is_error: _,
        } => tool_call_id.len() + content.iter().map(block_bytes).sum::<usize>(),
        ContentBlock::Image { source } => source.data.len() + source.mime_type.len(),
    }
}

/// The bytes a provider's replay payload holds. It is opaque, and it can be large.
fn state_bytes(state: &crate::ProviderState) -> usize {
    state.owner.provider.len() + state.owner.model.len() + json_bytes(&state.value)
}

/// The bytes a JSON value would take, walked rather than serialised.
fn json_bytes(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Null => 4,
        serde_json::Value::Bool(_) => 5,
        serde_json::Value::Number(number) => number.to_string().len(),
        // Two quotes, so a string is never counted short.
        serde_json::Value::String(text) => text.len() + 2,
        serde_json::Value::Array(items) => {
            2 + items.len() + items.iter().map(json_bytes).sum::<usize>()
        }
        serde_json::Value::Object(fields) => {
            2 + fields.len()
                + fields
                    .iter()
                    .map(|(key, held)| key.len() + 3 + json_bytes(held))
                    .sum::<usize>()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(body: &str) -> Vec<ContentBlock> {
        vec![ContentBlock::Text {
            text: body.to_string(),
        }]
    }

    #[test]
    fn two_messages_keep_their_order() {
        let queue = MessageQueue::new();
        queue.push(text("first")).unwrap();
        queue.push(text("second")).unwrap();
        let drained = queue.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0], text("first"), "arrival order is kept");
        assert_eq!(drained[1], text("second"));
    }

    #[test]
    fn a_full_queue_returns_a_typed_error() {
        let queue = MessageQueue::with_capacity(2);
        queue.push(text("one")).unwrap();
        queue.push(text("two")).unwrap();

        let error = queue.push(text("three")).unwrap_err();
        assert_eq!(error, QueueError::Full { capacity: 2 });
        // The earlier messages must survive. Dropping one to make room would lose
        // user input in silence.
        assert_eq!(queue.len(), 2, "a full queue drops no earlier message");
        let drained = queue.drain();
        assert_eq!(drained[0], text("one"));
        assert_eq!(drained[1], text("two"));
    }

    #[test]
    fn with_capacity_sets_the_stated_cap() {
        let queue = MessageQueue::with_capacity(1);
        queue.push(text("one")).unwrap();
        assert!(
            queue.push(text("two")).is_err(),
            "the stated cap binds, not the default cap"
        );
    }

    #[test]
    fn clear_drops_every_queued_message() {
        let queue = MessageQueue::new();
        queue.push(text("one")).unwrap();
        queue.push(text("two")).unwrap();
        queue.clear();
        assert!(queue.is_empty(), "clear empties the queue");
        assert!(queue.drain().is_empty());
    }

    #[test]
    fn len_and_is_empty_track_the_queue() {
        let queue = MessageQueue::new();
        assert_eq!(queue.len(), 0);
        assert!(queue.is_empty());

        queue.push(text("one")).unwrap();
        assert_eq!(queue.len(), 1);
        assert!(!queue.is_empty());

        queue.drain();
        assert_eq!(queue.len(), 0);
        assert!(
            queue.is_empty(),
            "is_empty is true exactly when len is zero"
        );
    }

    #[test]
    fn the_reported_position_equals_the_queue_length_at_push() {
        // The invariant, not one example. See AGENTS.md step 12.
        let queue = MessageQueue::with_capacity(64);
        for expected in 1..=20usize {
            let position = queue.push(text("m")).unwrap();
            assert_eq!(
                position, expected,
                "the reported position must equal the queue length at that push"
            );
            assert_eq!(position, queue.len());
        }
    }

    #[test]
    fn a_clone_shares_one_queue() {
        // A frontend and the driver hold one queue, so a push on either is visible
        // to the other.
        let queue = MessageQueue::new();
        let other = queue.clone();
        queue.push(text("one")).unwrap();
        assert_eq!(other.len(), 1, "a clone shares the same queue");
        assert_eq!(other.drain().len(), 1);
        assert!(queue.is_empty(), "and a drain on the clone empties both");
    }

    // ---- the byte cap (SPEC-steering section 4) ----

    /// A message of exactly `bytes` counted bytes.
    fn body(bytes: usize) -> Vec<ContentBlock> {
        vec![ContentBlock::Text {
            text: "x".repeat(bytes),
        }]
    }

    #[test]
    fn a_message_over_the_byte_cap_is_refused_and_names_both_numbers() {
        // A count cap bounds nothing on its own, because one message can be any size.
        // See decision D-a-steering-message-is-bounded-by-bytes.
        let queue = MessageQueue::with_limits(32, 100);
        let error = queue.push(body(101)).unwrap_err();
        assert_eq!(
            error,
            QueueError::TooLarge {
                limit: 100,
                size: 101
            }
        );
        let text = error.to_string();
        assert!(
            text.contains("101") && text.contains("100"),
            "the refusal must state the size and the limit: {text}"
        );
    }

    #[test]
    fn the_byte_cap_holds_for_any_message_size() {
        // The invariant, not one example: a push is accepted exactly when the counted
        // size is inside the cap. See AGENTS.md step 12.
        let limit = 64;
        for size in 0..=(limit * 2) {
            let queue = MessageQueue::with_limits(32, limit);
            let message = body(size);
            let counted = message_bytes(&message);
            let accepted = queue.push(message).is_ok();
            assert_eq!(
                accepted,
                counted <= limit,
                "a message of {counted} counted bytes must be accepted only inside the \
                 cap of {limit}"
            );
            // And the queue never holds a byte it refused.
            let held: usize = queue.drain().iter().map(|held| message_bytes(held)).sum();
            assert!(
                held <= limit,
                "the queue held {held} bytes with a cap of {limit}"
            );
        }
    }

    #[test]
    fn a_refused_message_leaves_the_queue_as_it_was() {
        // The count cap already promises this. The byte cap must promise it too, or a
        // large message would cost the user an earlier one.
        let queue = MessageQueue::with_limits(32, 10);
        queue.push(text("keep me")).unwrap();
        queue.push(body(11)).unwrap_err();
        assert_eq!(queue.len(), 1, "an oversized push adds nothing");
        let drained = queue.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0], text("keep me"), "and it drops nothing");
    }

    #[test]
    fn the_counted_size_covers_every_block_kind() {
        // A block the count skips is a place a large body hides. A review found both
        // `ProviderState` payloads missing from the first draft of this list.
        let owner = crate::ReasoningOwner {
            provider: "bedrock".to_string(),
            model: "a-model".to_string(),
        };
        let big = "y".repeat(500);
        let cases: Vec<(&str, Vec<ContentBlock>)> = vec![
            ("text", body(500)),
            (
                "a reasoning trace",
                vec![ContentBlock::ReasoningTrace { text: big.clone() }],
            ),
            (
                "a replay payload",
                vec![ContentBlock::ReasoningReplay {
                    text: String::new(),
                    state: Some(crate::ProviderState {
                        owner: owner.clone(),
                        value: serde_json::json!({ "signature": big.clone() }),
                    }),
                }],
            ),
            (
                "tool call arguments",
                vec![ContentBlock::ToolCall {
                    id: "call-1".to_string(),
                    name: "bash".to_string(),
                    arguments: serde_json::json!({ "command": big.clone() }),
                    state: None,
                }],
            ),
            (
                "a tool call replay payload",
                vec![ContentBlock::ToolCall {
                    id: "call-1".to_string(),
                    name: "bash".to_string(),
                    arguments: serde_json::json!({}),
                    state: Some(crate::ProviderState {
                        owner: owner.clone(),
                        value: serde_json::json!([big.clone()]),
                    }),
                }],
            ),
            (
                "a nested tool result",
                vec![ContentBlock::ToolResult {
                    tool_call_id: "call-1".to_string(),
                    content: vec![ContentBlock::Text { text: big.clone() }],
                    is_error: false,
                }],
            ),
            (
                "an image payload",
                vec![ContentBlock::Image {
                    source: crate::ImageSource {
                        data: big.clone(),
                        mime_type: "image/png".to_string(),
                    },
                }],
            ),
        ];

        for (what, message) in cases {
            let counted = message_bytes(&message);
            assert!(
                counted >= 500,
                "{what}: the count must include the payload, and it said {counted}"
            );
            let queue = MessageQueue::with_limits(32, 100);
            assert!(
                queue.push(message).is_err(),
                "{what}: a 500 byte payload must not pass a 100 byte cap"
            );
        }
    }

    #[test]
    fn with_limits_sets_the_stated_byte_cap() {
        let queue = MessageQueue::with_limits(4, 8);
        assert_eq!(queue.max_message_bytes(), 8);
        queue.push(body(8)).expect("the cap itself passes");
        assert!(
            queue.push(body(9)).is_err(),
            "the stated cap binds, not the default cap"
        );
    }

    #[test]
    fn every_constructor_carries_a_byte_cap() {
        // No constructor opts out. A queue with no byte cap would be the fail-open
        // shape this project keeps paying for.
        for queue in [
            MessageQueue::new(),
            MessageQueue::with_capacity(4),
            MessageQueue::default(),
        ] {
            assert_eq!(
                queue.max_message_bytes(),
                MAX_STEER_MESSAGE_BYTES,
                "a queue with no stated cap uses the default one"
            );
            assert!(
                queue.push(body(MAX_STEER_MESSAGE_BYTES + 1)).is_err(),
                "every constructor caps a message"
            );
        }
    }
}
