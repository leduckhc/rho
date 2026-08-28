# D-a-waiter-has-a-deadline — a queued child stops waiting, and the parent's turn returns

**Question:** a blocking `spawn_agents` call waits for a slot. The wait can be
`ceil(max_queued_per_parent / max_children_per_parent)` times `child_timeout`, which is about
forty minutes at the defaults. A prompt-injected model picks the fan-out width and the
children that sleep. What bounds one call?

## The decision

`QueuedChild::started` takes a deadline. `SubagentLimits` gains `queue_wait`, flag
`--queue-wait-secs`. When the deadline passes and no slot freed, `started` resolves
`Err(Dequeued::WaitedTooLong { limit })`.

The default is 600 seconds, which is one `child_timeout`. So a waiter gets one whole sibling
run of patience, and no more. The command line defaults the deadline to the effective
`--child-timeout-secs`, so a host that lengthens a child run lengthens the patience with it.

The deadline is separate from `child_timeout`, and it starts when the child begins to wait.
A child that starts still gets its whole run budget, exactly as section 2.4 of
`SPEC-subagent-slots-handles-grace` states.

**Zero means no waiting, not no deadline.** `--queue-wait-secs 0` refuses any child that has
to wait. There is no value that turns the deadline off.

**A cancel wins.** A waiter that is cancelled and past its deadline reports `Cancelled`,
because a cancel is what the parent asked for.

## Why

The worst case was a product of the wait line depth and the child timeout, and a model chose
both factors. Now the worst case for one tool call is `queue_wait` plus `child_timeout`,
which is about twenty minutes at the defaults, and it no longer grows with the wait line.

The deadline applies to a background waiter too. A background spawn does not hold the
parent's turn, so the wait itself is cheap there. But a queued entry holds a cancel token
and a message queue, and a deadline bounds how long it holds them. A refused background
waiter records a report, so a parent that polls learns why it never ran.

A zero that meant "wait for ever" would be a fail-open default in a security-relevant value.
This project has a named defect for exactly that shape. See
`D-plugin-does-not-classify-itself`.

## Rules out

**A deadline built on `child_timeout`.** The two answer different questions. One bounds a
run, the other bounds a wait, and one number cannot be raised without raising the other.

**A deadline the model chooses.** The wait is the cost the model would set, so the value is
a host flag and never a tool argument.

**An off switch.** There is no `queue_wait` that means unlimited.

**A retry inside `started`.** A refused waiter returns to the caller, which reports it. rho
never re-queues the work by itself, because that would hide the cost again.

**A different deadline for a blocking spawn and a background one.** One rule, one number, and
one test set. A second rule would need a second flag and a second error case.

## Cost

A wide fan-out under a small child cap can now lose its last tasks. At the defaults, a line
deeper than one round of children refuses the rest with a message that names
`--queue-wait-secs` and `background: true`. That is the trade: rho refuses work rather than
hold a parent's turn for forty minutes.
