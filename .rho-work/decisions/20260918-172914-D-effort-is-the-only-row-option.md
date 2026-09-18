# D-effort-is-the-only-row-option

Date: 20260918-172914

## The question

The user asked to show a model option on the row and to let Tab change it. An option could
be one of many: reasoning effort, a fast or slow speed, or a context size. Which options does
a row carry now?

## The decision

The reasoning effort is the only per-row option. Tab cycles it, through
`next_effort_in_cycle`, in the order `None → Off → Low → Medium → High → XHigh → None`. The
change is a preview. `Enter` is what applies it.

Arrow keys keep moving the row selection. There is no arrow-key option selector. The picker
draws one option, the effort, and one key changes it.

This honours `D-a-model-descriptor-carries-no-capability-claim`. That decision says a
listing proves a model exists and never proves it is fast. So a row states existence and an
effort, and it claims no capability.

## What this rules out

- A fast or slow speed dimension on the row. Speed is a separate alias over the effort
  ladder, not a row option. See `D-a-model-descriptor-carries-no-capability-claim`.
- A context-size dimension on the row. The descriptor carries no context length, so the row
  cannot show one.
- An arrow-key option selector. Arrow keys move the row selection and nothing else. A second
  navigation axis needs a second real dimension first.
- Any branch for a second dimension that no test reaches. AGENTS.md step 6 forbids code that
  no test exercises.

## What a second dimension would need first

A second dimension is allowed only after all of these hold:

1. A real data source for the dimension, proven by a measurement, not by a listing.
2. A decision that amends `D-a-model-descriptor-carries-no-capability-claim`, or a new field
   with its own verified source.
3. A spec that defines the key map for two axes, so a user knows which key moves which axis.
4. A test for each new branch, so no dead branch ships.

## Why

The user made this scope choice. Effort is the one dimension rho can set today without a
capability claim. Building a multi-axis selector now would add a branch that no data feeds
and no test reaches, and the gate would then make us delete it.
