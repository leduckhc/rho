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
    pub fn push(&self, mut message: Vec<ContentBlock>) -> Result<usize, QueueError> {
        let size = message_bytes(&message);
        if size > self.inner.max_message_bytes {
            return Err(QueueError::TooLarge {
                limit: self.inner.max_message_bytes,
                size,
            });
        }
        // The queue keeps what it counted, and no more. A caller may hand it a `Vec` or a
        // `String` with room for a million bytes and two bytes in it, and the count charges
        // for the two. Storing that allocation would put the cap and the memory back out of
        // step, which is the whole defect this cap exists to close.
        shrink_to_payload(&mut message, 0);
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

/// What one block costs, whatever its payload.
///
/// A count of payload bytes alone is not a bound on memory, because ten thousand empty
/// blocks hold ten thousand allocations and no payload. So every block is charged this
/// much before its payload is counted. A reviewer found the hole.
pub const BLOCK_OVERHEAD_BYTES: usize = 64;

/// What one JSON node costs, whatever it holds.
///
/// A node is never free, or a value of ten thousand empty nodes would count nothing. One
/// number covers every kind of node, because five separate constants were five charges no
/// test could see, and one of them was already redundant. See decision
/// D-a-steering-message-is-bounded-by-bytes.
pub const JSON_NODE_MIN_BYTES: usize = 4;

/// How deep the count walks a JSON value.
///
/// A value deeper than this is refused rather than walked, because the walk is recursive
/// and a hostile value would end the process on the stack. A body rho cannot measure is a
/// body it cannot bound, so the refusal is the safe answer.
pub const MAX_COUNTED_JSON_DEPTH: usize = 64;

/// Drop every byte of spare room in a message, so the queue keeps what the count charged.
///
/// A caller's `Vec::with_capacity` or `String::with_capacity` is invisible to a byte count,
/// because a count reads a length. An outside review found that a caller could pass one
/// small block inside a large allocation and the queue would retain it.
///
/// **It carries its own depth guard.** The refusal in `push` is not enough on its own: it
/// rests on `usize::MAX` being larger than the cap, and a host may set the cap to
/// `usize::MAX` through `with_limits` or `--max-agent-steer-bytes`. At that one value a
/// too-deep message is accepted, and an unguarded walk would then end the process. A
/// reviewer found that the guard was indirect and that one setting removed it. So the walk
/// stops where the count stops, and no caller can arrange the failure.
///
/// A part below the depth keeps its spare room. That is the safe trade: rho cannot measure
/// it, so rho does not walk it either.
///
/// A `serde_json::Map` keeps its own spare room, and serde_json exposes no way to shrink
/// it. That room is bounded by the entries the count charged for, keys included.
fn shrink_to_payload(message: &mut Vec<ContentBlock>, depth: usize) {
    if depth > MAX_COUNTED_JSON_DEPTH {
        return;
    }
    message.shrink_to_fit();
    for block in message.iter_mut() {
        match block {
            ContentBlock::Text { text } | ContentBlock::ReasoningTrace { text } => {
                text.shrink_to_fit()
            }
            ContentBlock::ReasoningReplay { text, state } => {
                text.shrink_to_fit();
                if let Some(state) = state {
                    shrink_json(&mut state.value, depth);
                }
            }
            ContentBlock::ToolCall {
                id,
                name,
                arguments,
                state,
            } => {
                id.shrink_to_fit();
                name.shrink_to_fit();
                shrink_json(arguments, depth);
                if let Some(state) = state {
                    shrink_json(&mut state.value, depth);
                }
            }
            ContentBlock::ToolResult {
                tool_call_id,
                content,
                is_error: _,
            } => {
                tool_call_id.shrink_to_fit();
                shrink_to_payload(content, depth + 1);
            }
            ContentBlock::Image { source } => {
                source.data.shrink_to_fit();
                source.mime_type.shrink_to_fit();
            }
        }
    }
}

/// Drop the spare room inside a JSON value. A map keeps its own, and serde_json hides it.
///
/// It carries the same depth guard as the count, for the reason `shrink_to_payload` states.
fn shrink_json(value: &mut serde_json::Value, depth: usize) {
    if depth > MAX_COUNTED_JSON_DEPTH {
        return;
    }
    match value {
        serde_json::Value::String(text) => text.shrink_to_fit(),
        serde_json::Value::Array(items) => {
            items.shrink_to_fit();
            for held in items.iter_mut() {
                shrink_json(held, depth + 1);
            }
        }
        serde_json::Value::Object(fields) => {
            for held in fields.values_mut() {
                shrink_json(held, depth + 1);
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

/// The bytes one message holds.
///
/// It counts the text of every block, the base64 payload of an image, the strings inside
/// tool-call arguments, and every `ProviderState` value. A block whose payload the count
/// skips is a place a large body hides, so the count skips none. It also charges
/// [`BLOCK_OVERHEAD_BYTES`] for each block, so a message of many empty blocks is not free.
///
/// The count walks a JSON value rather than serialising it, so a push allocates nothing
/// but a number. It counts the punctuation a writer would need, so it is never smaller
/// than the strings the value holds. A value nested deeper than
/// [`MAX_COUNTED_JSON_DEPTH`] counts as larger than any cap, so `push` refuses it.
pub fn message_bytes(message: &[ContentBlock]) -> usize {
    message
        .iter()
        .map(|block| BLOCK_OVERHEAD_BYTES.saturating_add(block_bytes(block, 0)))
        .fold(0, usize::saturating_add)
}

/// The bytes one block holds. Every variant is named, so a new one cannot be forgotten.
///
/// A review found both `state` fields missing from the first draft of this function.
///
/// A `ToolResult` holds blocks, so this walk recurses exactly as the JSON walk does. It
/// carries the same depth guard, because a block nested past the depth would end the
/// process on the stack. A second review found the guard on one walk and not the other,
/// and a probe proved the abort is real.
fn block_bytes(block: &ContentBlock, depth: usize) -> usize {
    if depth > MAX_COUNTED_JSON_DEPTH {
        return usize::MAX;
    }
    match block {
        ContentBlock::Text { text } | ContentBlock::ReasoningTrace { text } => text.len(),
        ContentBlock::ReasoningReplay { text, state } => text
            .len()
            .saturating_add(state.as_ref().map_or(0, |held| state_bytes(held, depth))),
        ContentBlock::ToolCall {
            id,
            name,
            arguments,
            state,
        } => id
            .len()
            .saturating_add(name.len())
            .saturating_add(json_bytes(arguments, depth))
            .saturating_add(state.as_ref().map_or(0, |held| state_bytes(held, depth))),
        ContentBlock::ToolResult {
            tool_call_id,
            content,
            is_error: _,
        } => tool_call_id.len().saturating_add(
            content
                .iter()
                .map(|held| BLOCK_OVERHEAD_BYTES.saturating_add(block_bytes(held, depth + 1)))
                .fold(0, usize::saturating_add),
        ),
        ContentBlock::Image { source } => source.data.len().saturating_add(source.mime_type.len()),
    }
}

/// The bytes a provider's replay payload holds. It is opaque, and it can be large.
fn state_bytes(state: &crate::ProviderState, depth: usize) -> usize {
    state
        .owner
        .provider
        .len()
        .saturating_add(state.owner.model.len())
        .saturating_add(json_bytes(&state.value, depth))
}

/// The bytes a JSON value would take, walked rather than serialised.
///
/// A value deeper than [`MAX_COUNTED_JSON_DEPTH`] returns `usize::MAX`, so every caller
/// refuses it. The walk stops there, so a hostile value cannot end the process.
fn json_bytes(value: &serde_json::Value, depth: usize) -> usize {
    if depth > MAX_COUNTED_JSON_DEPTH {
        return usize::MAX;
    }
    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) => JSON_NODE_MIN_BYTES,
        serde_json::Value::Number(number) => JSON_NODE_MIN_BYTES.max(number.to_string().len()),
        serde_json::Value::String(text) => JSON_NODE_MIN_BYTES.saturating_add(text.len()),
        serde_json::Value::Array(items) => items
            .iter()
            .map(|held| json_bytes(held, depth + 1))
            .fold(JSON_NODE_MIN_BYTES, usize::saturating_add),
        // A key is a payload too. A review deleted this charge and every test stayed
        // green, so one 60 KiB key counted ten bytes and passed a 64 KiB cap.
        //
        // An entry costs the floor on top of its key and its value, because a map entry is
        // a heap node in its own right. A first fix deleted that charge as redundant, and a
        // re-review showed it is not: an object of short keys and empty values then counted
        // four bytes an entry while each real entry costs tens. So an entry costs more than
        // an array element, and `no_json_node_is_free_to_hold` pins that difference.
        serde_json::Value::Object(fields) => fields
            .iter()
            .map(|(key, held)| {
                json_bytes(held, depth + 1)
                    .saturating_add(key.len())
                    .saturating_add(JSON_NODE_MIN_BYTES)
            })
            .fold(JSON_NODE_MIN_BYTES, usize::saturating_add),
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

    /// Every payload character a message holds, keys included.
    ///
    /// It is deliberately not `message_bytes`, because a test that measures with the
    /// function under test can only see its caller.
    fn payload_chars(message: &[ContentBlock]) -> usize {
        message
            .iter()
            .map(|block| match block {
                ContentBlock::Text { text } | ContentBlock::ReasoningTrace { text } => text.len(),
                _ => 0,
            })
            .sum()
    }

    /// A message that counts exactly `bytes`, block overhead included.
    ///
    /// The cap counts the block as well as its payload, so a test that wants a stated
    /// counted size has to say so once, here, rather than in every assertion.
    fn body(bytes: usize) -> Vec<ContentBlock> {
        let payload = bytes
            .checked_sub(BLOCK_OVERHEAD_BYTES)
            .expect("a message cannot count less than one block");
        let message = vec![ContentBlock::Text {
            text: "x".repeat(payload),
        }];
        assert_eq!(message_bytes(&message), bytes, "the helper must be exact");
        message
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
        queue.push(body(100)).expect("the cap itself passes");
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
        let limit = BLOCK_OVERHEAD_BYTES * 2;
        for size in BLOCK_OVERHEAD_BYTES..=(limit * 2) {
            let queue = MessageQueue::with_limits(32, limit);
            let message = body(size);
            let counted = message_bytes(&message);
            // An independent lower bound, so a bug inside the count is visible here too.
            // Both sides used `message_bytes` before, so this test could only see the
            // comparison in `push` and never the count itself.
            assert!(
                counted >= payload_chars(&message),
                "the count must never be smaller than the payload it holds"
            );
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
        let queue = MessageQueue::with_limits(32, BLOCK_OVERHEAD_BYTES + 10);
        queue.push(text("keep me")).unwrap();
        queue.push(body(BLOCK_OVERHEAD_BYTES + 11)).unwrap_err();
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
        let cap = BLOCK_OVERHEAD_BYTES + 8;
        let queue = MessageQueue::with_limits(4, cap);
        assert_eq!(queue.max_message_bytes(), cap);
        queue.push(body(cap)).expect("the cap itself passes");
        assert!(
            queue.push(body(cap + 1)).is_err(),
            "the stated cap binds, not the default cap"
        );
    }

    #[test]
    fn a_block_is_never_free_to_hold() {
        // A count of payload bytes alone is not a bound on memory. Ten thousand empty
        // blocks hold a real allocation each and counted nothing, so a message could be
        // large and cheap at the same time. The count now charges for the block itself.
        let empty = ContentBlock::Text {
            text: String::new(),
        };
        assert!(
            message_bytes(std::slice::from_ref(&empty)) >= BLOCK_OVERHEAD_BYTES,
            "one empty block is not free"
        );
        for count in [1usize, 10, 1000] {
            let message: Vec<ContentBlock> = std::iter::repeat_n(empty.clone(), count).collect();
            assert!(
                message_bytes(&message) >= count * BLOCK_OVERHEAD_BYTES,
                "{count} blocks must cost at least {count} block overheads"
            );
        }
        // And the cap refuses them, rather than holding a queue of cheap blocks.
        let queue = MessageQueue::with_limits(32, 1024);
        let many: Vec<ContentBlock> = std::iter::repeat_n(empty, 1024).collect();
        assert!(
            queue.push(many).is_err(),
            "a thousand empty blocks must not pass a 1 KiB cap"
        );
    }

    #[test]
    fn no_json_node_is_free_to_hold() {
        // A large body must not hide in a part of a value the count skips. An object
        // **key** is such a part: a review deleted the key charge and every test stayed
        // green, so a message of one 60 KiB key counted ten bytes and passed a 64 KiB
        // cap. The count charges for a key, a string, and every structural node.
        let big = "k".repeat(4096);
        let cases: Vec<(&str, serde_json::Value)> = vec![
            ("an object key", serde_json::json!({ big.clone(): null })),
            ("a string value", serde_json::json!({ "k": big.clone() })),
            ("an array of keys", serde_json::json!([{ big.clone(): 1 }])),
            (
                "a nested key",
                serde_json::json!({ "outer": { big.clone(): true } }),
            ),
        ];
        for (what, value) in cases {
            let message = vec![ContentBlock::ToolCall {
                id: String::new(),
                name: String::new(),
                arguments: value,
                state: None,
            }];
            let counted = message_bytes(&message);
            assert!(
                counted >= 4096,
                "{what}: the count said {counted} for a 4096 byte payload"
            );
            let queue = MessageQueue::with_limits(32, 1024);
            assert!(
                queue.push(message).is_err(),
                "{what}: a 4 KiB payload must not pass a 1 KiB cap"
            );
        }
        // A container is a node too, so even an empty one costs the floor. Without this
        // the seed of each fold was a charge no test could see.
        for (what, value) in [
            ("an empty object", serde_json::json!({})),
            ("an empty array", serde_json::json!([])),
            ("a null", serde_json::Value::Null),
        ] {
            let block = ContentBlock::ToolCall {
                id: String::new(),
                name: String::new(),
                arguments: value,
                state: None,
            };
            let counted = block_bytes(&block, 0);
            assert!(
                counted >= JSON_NODE_MIN_BYTES,
                "{what} counted {counted}, under the floor"
            );
        }

        // An object entry costs more than an array element, because an entry is a heap node
        // with a key slot. A first fix deleted that charge as redundant, and a map of short
        // keys and empty values then counted four bytes an entry.
        //
        // **The key is empty on purpose.** With any key at all the key charge alone makes an
        // object dearer than an array, and the entry charge could be deleted unseen. That is
        // exactly what happened to the first version of this assertion.
        let counted = |value| {
            block_bytes(
                &ContentBlock::ToolCall {
                    id: String::new(),
                    name: String::new(),
                    arguments: value,
                    state: None,
                },
                0,
            )
        };
        let one_entry = serde_json::json!({ "": null });
        let one_element = serde_json::json!([null]);
        assert!(
            counted(one_entry) > counted(one_element),
            "an entry with no key must still cost more than an element"
        );

        // And no node is free, so a value of many empty nodes still costs. The floor is
        // asserted per node, so an arm that charged nothing would fail here.
        for count in [1usize, 16, 512] {
            let empty_array = serde_json::Value::Array(vec![serde_json::Value::Null; count]);
            let mut empty_object = serde_json::Map::new();
            for index in 0..count {
                empty_object.insert(index.to_string(), serde_json::Value::Null);
            }
            for (what, value) in [
                ("an array", empty_array),
                ("an object", serde_json::Value::Object(empty_object)),
            ] {
                let message = vec![ContentBlock::ToolCall {
                    id: String::new(),
                    name: String::new(),
                    arguments: value,
                    state: None,
                }];
                let counted = message_bytes(&message);
                assert!(
                    counted >= count * JSON_NODE_MIN_BYTES,
                    "{what} of {count} empty nodes counted {counted}, under the floor"
                );
            }
        }
    }

    #[test]
    fn a_value_too_deep_to_count_is_refused() {
        // `push` is public, so a caller may hand it a tool-call block. A value nested
        // deeper than the count walks is refused, rather than walked into a stack
        // overflow. Fail closed, because a body rho cannot measure is a body it cannot
        // bound.
        let mut value = serde_json::json!("leaf");
        for _ in 0..(MAX_COUNTED_JSON_DEPTH + 20) {
            value = serde_json::Value::Array(vec![value]);
        }
        let message = vec![ContentBlock::ToolCall {
            id: "call-1".to_string(),
            name: "bash".to_string(),
            arguments: value,
            state: None,
        }];
        let queue = MessageQueue::with_limits(32, MAX_STEER_MESSAGE_BYTES);
        assert!(
            queue.push(message).is_err(),
            "a value too deep to measure must be refused"
        );
        assert_eq!(
            MAX_COUNTED_JSON_DEPTH, 64,
            "the stated depth is part of the contract"
        );
    }

    #[test]
    fn a_nested_tool_result_too_deep_to_count_is_refused() {
        // A `ToolResult` holds blocks, so the block walk recurses too. It had no depth
        // guard while the JSON walk did, and a probe proved an unguarded walk aborts the
        // process on the stack. One walk with a guard and one without is half a guard.
        let mut deep = ContentBlock::Text {
            text: "leaf".to_string(),
        };
        for _ in 0..(MAX_COUNTED_JSON_DEPTH + 20) {
            deep = ContentBlock::ToolResult {
                tool_call_id: String::new(),
                content: vec![deep],
                is_error: false,
            };
        }
        let queue = MessageQueue::with_limits(32, MAX_STEER_MESSAGE_BYTES);
        assert!(
            queue.push(vec![deep]).is_err(),
            "a block nested too deep to measure must be refused"
        );
    }

    #[test]
    fn an_oversized_message_is_too_large_and_not_merely_full() {
        // Both caps can fail at once. The byte cap answers first, because "the queue is
        // full" would send the writer to wait when a shorter message would be taken now.
        let queue = MessageQueue::with_limits(1, BLOCK_OVERHEAD_BYTES + 4);
        queue.push(text("fill")).expect("the queue takes one");
        let error = queue
            .push(body(BLOCK_OVERHEAD_BYTES + 5))
            .expect_err("a full queue and an oversized message both refuse");
        assert!(
            matches!(error, QueueError::TooLarge { .. }),
            "the size is the more useful answer: {error}"
        );
    }

    #[test]
    fn the_queue_keeps_no_capacity_the_message_did_not_need() {
        // A byte count reads a length, so a caller's spare room is invisible to it. An
        // outside review found that one small block inside a large allocation passed the
        // cap and the queue then held the allocation. What is stored is what was counted.
        let mut roomy: Vec<ContentBlock> = Vec::with_capacity(100_000);
        let mut text = String::with_capacity(1_000_000);
        text.push_str("hi");
        roomy.push(ContentBlock::Text { text });
        let mut nested_text = String::with_capacity(500_000);
        nested_text.push_str("deep");
        roomy.push(ContentBlock::ToolResult {
            tool_call_id: String::with_capacity(4096),
            content: Vec::with_capacity(2048),
            is_error: false,
        });
        if let Some(ContentBlock::ToolResult { content, .. }) = roomy.last_mut() {
            content.push(ContentBlock::Text { text: nested_text });
        }

        let queue = MessageQueue::new();
        queue.push(roomy).expect("a short message passes the cap");

        let held = queue.drain();
        let stored = &held[0];
        assert_eq!(
            stored.capacity(),
            stored.len(),
            "the queue must keep no room the message did not need"
        );
        match &stored[0] {
            ContentBlock::Text { text } => assert_eq!(
                text.capacity(),
                text.len(),
                "a string keeps no spare room either"
            ),
            other => panic!("the first block is text, got {other:?}"),
        }
        match &stored[1] {
            ContentBlock::ToolResult { content, .. } => {
                assert_eq!(
                    content.capacity(),
                    content.len(),
                    "a nested block list is normalised too"
                );
                match &content[0] {
                    ContentBlock::Text { text } => {
                        assert_eq!(text.capacity(), text.len(), "and its text")
                    }
                    other => panic!("the nested block is text, got {other:?}"),
                }
            }
            other => panic!("the second block is a tool result, got {other:?}"),
        }
    }

    #[test]
    fn the_shrink_walk_stops_where_the_count_stops() {
        // The refusal in `push` rests on `usize::MAX` being larger than the cap, and a host
        // may set the cap to `usize::MAX`. At that one value a too-deep message is accepted,
        // and an unguarded walk would then end the process. A reviewer found that the guard
        // was indirect and that one setting removed it.
        //
        // The proof is observable and needs no crash: what the walk reaches is shrunk, and
        // what lies past the depth keeps its spare room.
        let mut deep_text = String::with_capacity(4096);
        deep_text.push_str("deep");
        let mut deep = ContentBlock::Text { text: deep_text };
        for _ in 0..(MAX_COUNTED_JSON_DEPTH + 2) {
            deep = ContentBlock::ToolResult {
                tool_call_id: String::new(),
                content: vec![deep],
                is_error: false,
            };
        }
        let mut top_text = String::with_capacity(4096);
        top_text.push_str("top");

        // The cap is the one value that makes the refusal in `push` stop working.
        let queue = MessageQueue::with_limits(32, usize::MAX);
        queue
            .push(vec![ContentBlock::Text { text: top_text }, deep])
            .expect("a cap of usize::MAX refuses nothing");

        let held = queue.drain();
        match &held[0][0] {
            ContentBlock::Text { text } => assert_eq!(
                text.capacity(),
                text.len(),
                "the walk reaches the top block, so it is shrunk"
            ),
            other => panic!("the first block is text, got {other:?}"),
        }

        // Walk to the bottom and read the block the guard stopped short of.
        let mut here = &held[0][1];
        let mut levels = 0usize;
        while let ContentBlock::ToolResult { content, .. } = here {
            here = &content[0];
            levels += 1;
        }
        assert_eq!(
            levels,
            MAX_COUNTED_JSON_DEPTH + 2,
            "the nesting is what it was"
        );
        match here {
            ContentBlock::Text { text } => assert!(
                text.capacity() > text.len(),
                "a block past the depth keeps its room, because the walk stopped"
            ),
            other => panic!("the deepest block is text, got {other:?}"),
        }
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
