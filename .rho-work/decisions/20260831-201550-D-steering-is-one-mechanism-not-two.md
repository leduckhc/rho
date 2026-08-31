# D-steering-is-one-mechanism-not-two

## Question

The user asked for "message queue vs message steering clearly stated". The user believes
there may be two mechanisms. Are there two, or one?

## Decision

There is one mechanism. Its name is **steering**. The queue is how steering holds a
message. The queue is not a second feature.

The evidence is in `SPEC-steering` and in the code.

- `crates/rho-core/src/agent.rs` names the type "the bounded steering queue".
- `MessageQueue::push` is the one door. `Session::steer` calls it.
- `SPEC-steering` section 1 says the queue exists to serve the steering path.
- `QueueError::Full` says "the steering queue is full". The error names both words as
  one thing.

So the queue is the data structure. Steering is the behaviour. They are one feature seen
from two angles.

## What this rules out

- A second user-facing word for the same idea. The interface uses "steer" everywhere.
- A design that shows the user a "queue" as if it were separate from steering.
- Any copy that names a "message queue" to the user. The word "queue" stays inside the
  code and inside the core error text.

## The user-facing words

- The feature and the verb: **steer** / **steering**.
- A steered message the model has not received yet: **waiting**.
- A steered message the model has received: **delivered**.

One word per meaning. See the prose rules in `AGENTS.md`.
