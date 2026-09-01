# D-model-arg-bypasses-the-picker

Date: 20260901-232014

## The question

`/model <id>` and `/effort <level>` know exactly what to apply. Should they open the
picker first, or apply and close?

## The decision

They apply and close.

- `/model <id>` sets the model to `<id>`, keeps the current effort, and pushes one notice
  row that reads `model set to <id>`. No picker.
- `/model` (no argument) opens the picker.
- `/effort` (no argument) pushes one notice row that reads
  `effort: <name>` where `<name>` is one of `unset`, `off`, `low`, `medium`, `high`,
  `xhigh`.
- `/effort <level>` sets the effort to `<level>` and pushes one notice row. The special
  value `unset` clears the effort so the provider uses its own default.
- An unknown level pushes one error row that names the valid levels. See
  `ReasoningEffort::from_str`.

The picker is **not** opened by `/effort` alone. Effort has one dimension, so the notice
shows the current level in one row and does not need a panel.

## What this rules out

- Opening the picker on `/model <id>` for confirmation. A typed id is the confirmation,
  and rho already accepts a typed id even when a listing would omit it. See
  D-a-listing-failure-never-stops-a-session.
- A `/model` that clears the model. rho refuses to send an empty id, per
  `SessionConfig::model`'s comment, and a slash command with no argument now means "show
  me the picker".
- An `/effort off` that is ambiguous with "unset". They are different: `off` sends the
  disable field to a provider that has one; `unset` sends no field at all.

## Why

A typed id is the shortest path to a change, and a user who typed it does not need a
picker to confirm. The picker exists for the case where the user knows they want to change
model but not which one.
