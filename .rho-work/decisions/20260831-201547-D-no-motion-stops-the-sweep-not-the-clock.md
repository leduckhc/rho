# D-no-motion-stops-the-sweep-not-the-clock

Status: accepted

## Question

Does `--no-motion` stop the live duration counter, or only the sweep band?

## Decision

`--no-motion` stops only the sweep band. The live duration counter keeps growing.

The sweep is motion. The counter is information. A user who turns motion off still
needs to see that the turn is alive. The counter answers that need without any moving
highlight. The digits change once per second, which is data, not animation.

So the loop still ticks while a turn runs, even with motion off. Each tick refreshes
the counter. The renderer gates only the sweep on `motion_enabled`. The renderer never
gates the counter on motion.

## What this rules out

- `--no-motion` must not freeze the live turn clock.
- `--no-motion` must not stop the timer, because the timer also drives the counter.
- The counter must not depend on `state.animate`.

## Reason

The guide claims that `--no-motion` stops the working animation. Today that claim is
false, because nothing animates at all. This decision makes the claim true and narrow.
The flag silences the one moving thing, the sweep. See `F-reduced-motion` and
`D-motion-answers-to-one-switch`.
