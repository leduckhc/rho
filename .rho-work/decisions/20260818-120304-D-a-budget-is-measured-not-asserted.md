# D-a-budget-is-measured-not-asserted — A cost budget is measured, or it is prose

**Question (controller, U5 verification):** the experience spec carried a cost budget with
zeroes in most rows. Does writing a zero in a spec make it true?

**Decision:** No. A budget row is a claim, and it holds only once a benchmark reports it.
`bench/tui_frame.py` now measures the frame time, the allocation count per frame, the bytes
per frame, the resident memory, and the first frame on a real pseudo-terminal. Every budget
row states the measured number, or it states that the wiring stage must reach it.

**Reason:** The measurement contradicted the spec on its first run. Motion claimed zero
allocations per frame. `sweep_frame` returns a `Vec`, so it costs one allocation per call.
The per-character allocation that this project refuses from codex is genuinely absent, and
the eight held bytes are genuinely eight, so the row was mostly right and precisely wrong.

The renderer measured 300 allocations and 15406 bytes per frame, on the minimal renderer
from `SPEC-tui`. Nobody had measured that before, so the zero in the budget was not a target
anybody was tracking. It is now the number the wired renderer must beat.

A second correction came out of the same stage. The first draft of the benchmark page
reported 6.1 milliseconds to first frame. That was the **minimum** of one run, quoted as a
median. Three fresh runs gave medians of 6.8, 6.5, and 6.3 milliseconds. So the page now
reports 6.5, states the run count, and records the mistake, because a minimum is the
friendliest number in any set and it is the easiest one to reach for.

**Rules out:** A budget row with no measurement behind it. A minimum presented as a median.
A performance claim in a spec that no command reproduces. A zero that nobody has counted.
