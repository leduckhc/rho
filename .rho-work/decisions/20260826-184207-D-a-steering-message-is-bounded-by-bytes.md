# D-a-steering-message-is-bounded-by-bytes — the queue counts bytes, not only messages

**Question:** `MessageQueue` holds 32 messages, and each body is written by a user or by a
model. Nothing caps one body. The queue now exists for 128 waiting children and 32 live
ones. What bounds the memory?

## The decision

`MessageQueue::push` refuses a message that is larger than a stated byte cap. The refusal is
a new named error, `QueueError::TooLarge { limit, size }`.

Two owners hold a default, because the two kinds of queue face different writers.

- `MAX_STEER_MESSAGE_BYTES` is 64 KiB. It is the cap of any queue a caller does not tune,
  including the one a user types into. A person pastes a stack trace, so this one is roomy.
- `SubagentLimits::max_steer_message_bytes` is 16 KiB, flag `--max-agent-steer-bytes`. Every
  child queue takes this value. A steer to a child is written by a model, and there may be
  160 such queues in one process.

`MessageQueue::with_limits(capacity, max_message_bytes)` states both bounds. `new` and
`with_capacity` keep the default byte cap, so no constructor makes an uncapped queue.

`message_bytes` counts one message. It counts every byte the queue holds: the text of each
block, the base64 image payload, and the strings inside tool-call arguments.

## Why

An unbounded queue is a memory defect. This project shipped one, and 8 MB of `bash` output
became 805 MB of memory. See `D-bash-line-cap`. A count cap alone bounds nothing, because
one message can be any size.

The cap belongs in `push`, and not in a caller. `push` is the one door into the queue. A
frontend, `Session::steer`, `LiveAgent::steer`, `AgentRegistry::steer_descendant`, and the
grace warning all go through it. A cap in one caller leaves the other four open.

The product is now a real number. A child queue holds at most 32 by 16 KiB, which is
512 KiB. At the shipped subagent defaults there are at most 160 child queues, so the
process holds at most 80 MiB of queued messages. One session queue holds at most 2 MiB.
Before this change each of those numbers was unbounded.

## Rules out

**One default for both kinds of queue.** 64 KiB per child queue would put the process
ceiling at 320 MiB, and 16 KiB would refuse a paste from a user.

**A cap in a caller.** Five callers push, so five caps would drift.

**A silent truncation.** A queue that shortens a message changes what the user said. The
push fails, the caller reports the refusal, and the user or the model sends less.

**A drop of an earlier message to make room.** That rule already holds for the count cap.
See `D-bounded-steering-queue`.

**An uncapped constructor.** There is no `MessageQueue` with no byte cap, because a
constructor that opts out is the fail-open shape this project keeps paying for.

## Cost

A large paste is refused rather than queued. The refusal names both numbers, so the user
knows how much to cut. A message with a large image is refused at 64 KiB of base64, which
is a small image. A caller that must hand a large body to a child writes a file and steers
with the path.
