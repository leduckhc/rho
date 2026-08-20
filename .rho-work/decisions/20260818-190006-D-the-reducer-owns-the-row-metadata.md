# D-the-reducer-owns-the-row-metadata — one call writes a row and its duration

**Question (contract review):** a `Tool` row is final when its status is `Ok`. Its duration
lives in the parallel `row_durations` array. Who writes that, and when?

**Decision:** The reducer, in the same call that finalises the row. `TuiState::apply` takes
the clock as data, `apply(&mut self, event: &AgentEvent, now_millis: i64)`, so it stays pure
and reads no clock. It writes the tool duration, the thinking duration, and the turn clock.

There is one entry point, so no caller can forget the clock.

**Reason:** Two facts about one row must not live in two writers. A row frozen with an empty
duration slot states a wrong fact for ever, because the terminal owns those cells.

The fields were worse than unsynchronised. `row_durations` and `row_bodies` are public, and
**no production code writes either one.** Only `tests/frames.rs` and `tests/render.rs` do.
So the duration ladder rendered for fixtures and never for a user, while `docs/features.md`
claimed F-duration-ladder and F-duration-slot ship. That is the fifth instance of this
project's signature defect: tested code that no key and no event reaches.

Passing the clock keeps the reducer testable. A test states the instants and asserts the
span, so no test reads a real clock and no test sleeps.

**Rules out:** A duration written by the event loop. A duration written by a renderer. A
second `apply` that omits the clock. A row that freezes before its metadata is written.
`Instant::now()` inside `state.rs`.
