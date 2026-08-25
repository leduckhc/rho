# D-retention-is-not-in-the-wiring-lane — no config key that nothing reads

**Question:** the store grows without a bound once recording is on. codex and claude both
have that hole. Does the wiring lane add a retention policy?

## The decision

No. The wiring lane adds no retention key and no prune.

The spec states the gap in writing. So no document claims a bound that does not exist.

## Why

A config key that nothing reads is dead surface, and this project has a named defect class
for it. See `D-dead-surface-is-a-defect-class`. Six such items were found by driving the
product, not by reading the code.

So retention ships with its own prune, its own tests, and its own measurement, or it does
not ship.

## Rules that hold

- The store grows. `docs/guide/sessions.md` says so plainly, with no promise of a bound.
- A user can delete a session today, with `rho sessions delete`.
- A later lane owns `keep-days`, `max-bytes`, and `keep-named`, together with the prune.

## Rules out

**Parsing a retention key now and reading it later.** That is the exact shape of the
defect class above.

**A prune on the start path with no measurement.** `F-fast-cold-start` is a feature, and a
scan of 500 files on every start would spend it.

## Cost

None. The cost is a documented gap.
