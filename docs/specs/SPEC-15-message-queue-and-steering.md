# SPEC-15 — Message queue and steering

Status: draft for sprint 2.
Owning crate: `rho-core`.
Features: F-92 (steer, over ACP), and the TUI steer path.

## 1. Why a queue exists

A user types while the agent runs. That message must not be dropped. It must not race
into the middle of a provider request. So rho holds it in a queue, and delivers it at a
safe point. This is the steering path that ACP F-92 needs, and that the TUI needs.

The queue is bounded. An unbounded queue is a memory defect. This project already
shipped one: an unbounded `bash` line reader turned 8 MB of output into 805 MB of
memory. That is defect 7 in `.rho-work/progress.md`, and decision D-016 fixed it. The
same rule applies here. A queue has a cap, and a full queue is a typed error, not
silent growth.

## 2. The public API

```rust
use crate::ContentBlock;

/// The capacity of the steering queue. A full queue rejects a new message.
pub const STEER_QUEUE_CAPACITY: usize = 32;

/// A typed queue error.
#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    /// The queue is full. The caller must tell the user, and must not drop an
    /// earlier message.
    #[error("the steering queue is full at {capacity} messages")]
    Full { capacity: usize },
}

/// A bounded, ordered queue of user messages that arrive while a turn runs.
///
/// A clone shares the same queue, so a frontend and the driver hold one queue.
#[derive(Clone)]
pub struct MessageQueue { /* private */ }

impl Default for MessageQueue {
    fn default() -> Self;
}

impl MessageQueue {
    /// A queue with the default capacity.
    pub fn new() -> Self;

    /// A queue with a stated capacity.
    pub fn with_capacity(capacity: usize) -> Self;

    /// Enqueue one message at the back. Return `Err(Full)` when the queue is full.
    /// Never block. Never drop an earlier message.
    pub fn push(&self, message: Vec<ContentBlock>) -> Result<(), QueueError>;

    /// Take every queued message in arrival order. Clear the queue.
    pub fn drain(&self) -> Vec<Vec<ContentBlock>>;

    /// Drop every queued message. A frontend calls this for a clean slate.
    pub fn clear(&self);

    /// The number of queued messages.
    pub fn len(&self) -> usize;

    /// True when the queue holds nothing.
    pub fn is_empty(&self) -> bool;
}
```

## 3. The delivery point

The driver drains the queue at one point. It drains after the current tool calls
finish, and before it builds the next provider request. See `build_request` in
`crates/rho-core/src/agent.rs`.

- The driver runs one provider turn.
- The driver runs every tool call the turn requested.
- The driver drains the queue.
- The driver appends each drained message to the context, in arrival order.
- The driver builds the next request and runs the next turn.

So a steering message reaches the model at a turn boundary, never inside a request. The
model sees it as the next user message.

A message that arrives after `AgentEnd` is not delivered by this run. The run is over,
so there is no next turn to drain. The message stays in the queue. The frontend then
calls `Session::prompt` to start a new run, and the driver drains the queue at the
start of that run too. So the message is delivered on the next run, and no message is
lost.

## 4. Bounds

The queue capacity is `STEER_QUEUE_CAPACITY`, which is 32. A caller may set another
capacity with `with_capacity`.

`push` on a full queue returns `QueueError::Full`. It never grows the queue past the
cap. It never drops an earlier message to make room. The frontend tells the user that
the queue is full, and the user waits or cancels.

So the queue cannot turn a fast typist into a memory defect. This is decision D-016
applied to a new place.

## 5. Ordering

The queue is first-in, first-out. Two steering messages keep their arrival order.
`drain` returns them in the order they were pushed. So the model reads them in the
order the user sent them.

## 6. Interaction with cancel

A cancel keeps the queue. It does not drop a queued message.

A queued message is a user instruction. Dropping user input in silence is the worse
failure. So a cancel stops the running turn and leaves the queue intact. The frontend
delivers the queued messages on the next run.

A frontend that wants a clean slate calls `clear` on purpose. So the drop is explicit,
never a side effect of a cancel.

## 7. Interaction with the append-only prompt prefix

A steering message never rewrites an already-sent turn. The driver appends it through
`Context::append`, which is append-only by construction. See
`crates/rho-core/src/context.rs`.

So the stable prompt prefix stays byte-identical, and the provider prompt cache stays
warm. This is the rule in F-60 and in `SPEC-01` section 1. A steering message adds a
new user turn. It edits no earlier turn.

## 8. The events a frontend sees

A frontend must show that a message is queued, and show when it is delivered. So the
event stream gains two variants. These are new `AgentEvent` variants, added the same
way `SPEC-11` added three agent variants.

```rust
    /// A user message was queued while a turn ran. `position` is its place in the
    /// queue, counted from one.
    MessageQueued { position: usize },
    /// Queued messages were delivered to the model at a turn boundary. `count` is
    /// how many.
    MessageDelivered { count: usize },
```

`MessageQueued` fires on a successful `push` during a run. `MessageDelivered` fires
when the driver drains the queue before a turn. A frontend renders a queued message as
pending, then as sent.

**Implementation status.** The variants and the queue type are new in `rho-core`. The
driver drains the queue at the turn boundary. A frontend that does not handle the two
variants still works, because the variants are additive. This matches the `SPEC-11`
pattern, where new variants did not break an existing consumer.

## 9. Test cases

- `a_message_queued_mid_turn_is_delivered_before_the_next_request` — a `push` during a
  tool call reaches the context before the next provider request.
- `two_messages_keep_their_order` — two pushes drain in the order they arrived.
- `a_full_queue_returns_a_typed_error` — a `push` past the cap returns `QueueError::Full`
  and drops no earlier message.
- `a_message_after_agent_end_stays_queued` — a `push` after `AgentEnd` is not delivered
  by the ended run, and the next run delivers it.
- `a_cancel_keeps_the_queue` — a cancel leaves every queued message in place.
- `clear_drops_every_queued_message` — `clear` empties the queue.
- `a_delivered_message_appends_a_new_turn` — a drained message adds a user message and
  edits no earlier turn.
- `the_reported_position_equals_the_queue_length_at_push` — for any sequence of pushes
  during a run, each `MessageQueued` position equals the queue length at that push. The
  invariant, not one example.
- `the_delivered_count_equals_the_drained_count` — for any sequence, each
  `MessageDelivered` count equals the number of messages the drain removed.
- `with_capacity_sets_the_stated_cap` — `MessageQueue::with_capacity` rejects the push
  past its stated cap, not the default cap.
- `len_and_is_empty_track_the_queue` — `len` equals the number of queued messages, and
  `is_empty` is true exactly when `len` is zero, across a push and a drain.

Every test uses a scripted fake provider. No test uses the network. No test uses
`sleep`. A test synchronises with a channel or a `Notify`.

## 10. Out of scope for sprint 2

- Injecting a steering message into the middle of a provider request. The delivery
  point is a turn boundary, on purpose.
- A priority order among queued messages. The queue is first-in, first-out.
- Removing a duplicate queued message.
- Editing a message already sent to the provider. The prompt prefix is append-only.
- A per-session capacity from the config file. The cap is a constant, with a
  constructor override for a test.
