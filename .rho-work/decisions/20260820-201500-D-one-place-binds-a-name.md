# D-one-place-binds-a-name — every registration derives a handle through one function, and it never re-derives

**Question:** A child gets a derived handle at three moments: a plain `spawn_child`, an
`admit_child` that starts at once, and the handout that starts a waiter. Where is the name
chosen?

**What a mutation showed.** The first implementation derived a name in the first two places and
left the handout alone. The handout inserted the live handle directly, so the name survived by
accident: nothing bound a name there, so nothing could change it. A deliberate break that made
`bind_handle` re-derive on every call passed every test, because the handout never called it.
The invariant "a queued child keeps the name it was told" was therefore protected by nothing.
Any later refactor that routed the handout through the registration path, which is the obvious
tidy-up, would have renamed the child in silence.

**Decision:** One function, `bind_handle`, derives and stores a handle, and every registration
path calls it inside its own critical section. It returns early when the id already holds a
name, and that early return is the whole guard. The handout now calls it too, so the guard is
load-bearing and a test can see it.

**What becomes true by construction.**

- A name is derived and stored in the same critical section as the registration, so two
  admissions of one agent name cannot pick one name.
- A queued child's name survives the start, because the only binder refuses to re-derive.
- A new registration path cannot forget to name its child, and cannot rename one either.

**Why not derive the name at start instead.** A queued child is addressable by id from the moment
it is admitted, so it must be addressable by name from that moment too. A name derived at start
would leave the model holding a name that reached nothing, which is the same defect as the fresh
id that once stranded every steer and cancel. See decision D-queue-over-refuse.

**A second finding, recorded here because it has the same cause.** `resolve` re-checks the
resolved id against the caller's ancestors, and that check looked redundant: the handle table is
keyed by the tree root, so no other tree's name is visible. It is not redundant. Every node of
one tree shares that key, so a mid-tree caller can see a cousin's binding, and the ancestor check
is the only thing that refuses it. A mutation that deleted the check passed every handle test,
because every one of them used a root caller. Two tests now cover it,
`a_child_cannot_reach_its_uncles_child_by_handle` and `an_alias_cannot_be_set_on_a_cousin`.

**Rules out:** Deriving a name outside the state lock. A second place that binds or renames.
Naming a child only when it starts. Trusting the per-tree key alone to scope a name.
