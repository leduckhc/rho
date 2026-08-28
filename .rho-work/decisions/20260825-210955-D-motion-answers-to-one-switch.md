# D-motion-answers-to-one-switch — an accessibility escape that reached nobody

**Question:** `MotionInputs` documents four ways to stop the sweep animation, and nothing sets
any of them. Which of the four ship?

**A correction first, because my first draft of this decision had the premise backwards.** I
wrote that the animation always runs and cannot be stopped. The opposite is true.
`apply_sweep` reads `state.animate`, `TuiState` derives `Default`, and **nothing in the
workspace ever assigns `animate`**. So the flag is false on every frame and the sweep never
draws. `motion_enabled`, `MotionInputs`, and `sweep_frame` have no production caller at all.

I got that wrong by reading the code's intent instead of driving it. My pseudo-terminal probe
never ran a turn, so I never watched the footer, and I then wrote "with a sweep over the word"
into the user guide. The docs are corrected in the same change as the code.

So this is not an accessibility escape from a moving interface. It is the reverse: a designed
feature, `F-working-motion`, that never reached a screen, plus the switch it must ship with.

**Decision: one flag, one config key, and one variable.** Not four.

- `--no-motion` stops the sweep.
- `tui-motion = false` does the same from a config file.
- `RHO_REDUCE_MOTION=1` does the same from the environment, because that is the name the code
  already documents and a user may set it once for every tool.

The renderer asks `motion_enabled` before it animates. That is the missing call, and it is the
whole defect.

## Why it still ships, and what changes for a user

Wiring it turns the animation **on for the first time**. That is a behaviour change, and the
docs must say so rather than claim nothing changed.

The switch ships in the same commit as the animation, never after. A moving interface with no
off switch is the accessibility defect this decision was written about, and shipping the motion
without the switch would create the very thing I mistakenly reported.

The prefers-reduced-motion convention exists on every other platform, and rho already wrote the
code for it. It just never called it.

## Rules out

**Four switches.** `tui.motion` and `tui.reduce_motion` say the same thing in opposite
directions, and two names for one idea is how a config grows a contradiction. One key, named
for the thing it controls.

**Reading the terminal or the OS for a reduced-motion preference.** No portable source exists
for it in a terminal, and a wrong guess would disable a feature nobody asked to lose.

**A motion level.** Off and on. A slider is a setting nobody tunes twice.

**Leaving `MotionInputs` with its four fields.** A struct that documents switches nobody sets
is how this defect survived a whole sprint. The struct keeps the conditions the renderer
really consults, and no more.

## Rules that hold

- The default is motion on, which is `F-working-motion` as specified. It is new on screen, and
  `docs/guide/` says so.
- A non-terminal stdout already stops the sweep, and that stays.
- With motion off, the word renders plain. The footer still says `working`, so the state is
  never carried by the animation alone. That was already true, and now a test pins it.
