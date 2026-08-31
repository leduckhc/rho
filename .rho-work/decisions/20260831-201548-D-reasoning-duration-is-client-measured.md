# Reasoning duration is measured on the client, and it is not a Usage field

Date: 20260831. Reference: `D-reasoning-duration-is-client-measured`.
From `SPEC-usage-carries-reasoning`.

## The question

The user wants the reasoning duration too. A duration can come from a provider, or the
client can time it. Where does the number live?

## The decision

The client measures the reasoning duration. No provider reports it, and it is not a
field on `rho_core::Usage`.

`rho-tui` already times it. `State::settle_duration` writes the span of a thinking row
from its start instant to the answer start. The renderer draws `∴ thought for 2.4s` from
`row_durations`. This works today with no provider support.

So the token count and the duration come from two different places:

- The token count is a provider fact. It rides on `Usage.reasoning_tokens`.
- The duration is a client measurement. It stays in the frontend timing that exists now.

A duration on `Usage` would mix a client clock into a provider report. `Usage` counts
tokens the provider billed. A wall-clock span is not that. Keeping them apart respects
one meaning per type.

## What this rules out

- No `reasoning_duration` field on `Usage`.
- No duration read from a provider wire format.
- No persisted duration in the session file. It stays a live measurement.
