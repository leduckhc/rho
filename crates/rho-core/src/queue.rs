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

    /// A queue with a stated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Arc::new(QueueInner {
                messages: Mutex::new(VecDeque::new()),
                capacity,
                observer: Mutex::new(None),
            }),
        }
    }

    /// Enqueue one message at the back.
    ///
    /// It returns `Err(Full)` when the queue is full. It never blocks, and it never
    /// drops an earlier message to make room. On success it returns the new queue
    /// length, which is the message's position counted from one.
    pub fn push(&self, message: Vec<ContentBlock>) -> Result<usize, QueueError> {
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
}
