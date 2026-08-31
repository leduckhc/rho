# D-a-steered-message-is-a-waiting-user-row

## Question

A user types while a turn runs. How does the TUI show the message before delivery, and
how does a reader tell a waiting message from a delivered one?

## Decision

A steered message appears at once as a user row. The row carries a `delivered` flag.
A waiting row draws a `waiting` tag. A delivered row drops the tag.

- The row is `Row::User { text, delivered }` in `crates/rho-tui/src/state.rs`.
- A message sent while idle starts `delivered: true`.
- A message steered while a turn runs starts `delivered: false`.
- `AgentEvent::MessageDelivered { count }` flips the oldest `count` waiting rows.

So the user sees the message the instant it is accepted. The user then sees it change
from waiting to delivered at the turn boundary.

## Why a flag, not a new row variant

A steered message is still a user message. Its text, its history, and its wrapping are
the same. Only its delivery state differs. A flag keeps one row type. A second variant
would duplicate the user row and split its rendering.

## Enter routes by activity

`KeyAction` gains a `Steer(String)` case. The Enter handler returns `Steer` while a turn
runs. It returns `Submit` while idle. Today the handler always returns `Submit`, and the
app always calls `Session::prompt`. That path never calls `Session::steer`, so the
steering surface is dead in the TUI. This decision wires it.

## What this rules out

- Dropping a steered message and rebuilding it on delivery.
- Calling `Session::prompt` a second time while a turn runs.
- Hiding a waiting message until the boundary. Silence reads as a lost message.
