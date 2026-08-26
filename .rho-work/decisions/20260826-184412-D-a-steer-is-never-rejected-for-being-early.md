# D-a-steer-is-never-rejected-for-being-early — no NotStreaming error

Date: 20260826. Reference: `D-a-steer-is-never-rejected-for-being-early`.
Spec: `docs/specs/20260819-102749-SPEC-jsonl-frontend.md`, section 3.2.

## The question

The draft spec gave `ReplyError` a variant called `NotStreaming`, for a `Steer` that arrives
while no run is going. Is that the right answer?

## What the code showed

It contradicts `rho-core`. `Session::steer` says:

> A message queued after the run ends stays queued, and the next run delivers it. No user
> message is dropped in silence.

So core accepts an early steer and keeps it. A `NotStreaming` reply would tell the client
the message was refused while core still held it. The client would then send it again, and
the model would read it twice.

`Session::steer` has exactly one failure, `QueueError::Full`. The draft spec had no wire
name for it, so a full queue would have arrived as `Internal`.

## The decision

`ReplyError::NotStreaming` does not exist. A `Steer` is accepted whenever the queue accepts
it, running or not. The reply carries the queued position in its `data` field.

`ReplyError::QueueFull` exists, and it is the only failure a `Steer` can report. It maps
from `QueueError::Full`.

## Why not the alternatives

- **Keep `NotStreaming`, and drop the message.** That is the silent drop core was written
  to prevent.
- **Keep `NotStreaming`, and keep the message.** Then `success: false` means the command
  worked, which is worse than either honest answer.

## What it rules out

- No wire error that reports a refusal core did not make.
- No mapping of `QueueError::Full` onto `Internal`. A named error case for each failure the
  caller can cause.
