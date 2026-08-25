# D-the-dead-surface-allowlist-names-its-lane — a ledger of pending wiring

**Question:** `D-dead-surface-is-a-defect-class` decided that a public function with no caller
needs a CI guard, and that the guard needs an allowlist because rho ships libraries. What shape
is the allowlist, and what stops it becoming a place to hide a defect?

**Decision:** one line per entry, in `bench/allowed-uncalled.txt`, and every line names the
reason. The format follows `bench/deleted-tests.txt`, which already works this way.

```
crates/rho-core/src/session/key.rs::mint  SPEC-session-store-wiring consumes it. The wiring lane is open.
crates/rho-provider-testkit/src/lib.rs::run_suite  A third party calls it. That is the crate's purpose.
```

Two kinds of entry, and the reason says which:

- **A library surface a third party calls.** `rho-provider-testkit` exists to be called from
  outside, so an internal caller would be the surprise.
- **Code that landed ahead of its wiring lane.** It names the spec that will consume it. The
  entry is a promise with an address.

## What makes it a ledger rather than a dustbin

**A reason is mandatory.** The guard fails on an entry with an empty reason. A bare path is how
an allowlist becomes a place to file defects.

**An entry that names a spec is checked.** The named spec must exist, so a lane cannot be
invented to silence the guard. `bench/check-ids.py` already resolves a `SPEC-` reference, and
this reuses that rule.

**A stale entry fails too.** When the function gains a caller, the guard reports the entry as
unnecessary and asks for its removal. Otherwise the file fills with lines nobody can question.

This turns "landed but unreachable" from invisible into a list a person can read. Today the
session store's `mint`, `default_store_root`, and `resolve_from` have no caller, because the
wiring lane is still a draft. That is legitimate, and it should be **visible** rather than
silent, because five defects of exactly that shape reached `main` unnoticed.

## Rules out

**Failing on any uncalled public function, with no escape.** It would break the testkit crate
and every primitive that lands before its lane, and the first person to hit it would delete
the guard.

**An allowlist keyed by crate.** One crate-wide exemption hides every future defect in that
crate. An entry names one function.

**A `#[allow(dead_code)]`-style attribute in the source.** The compiler's dead-code pass does
not see a public item, so the attribute would suggest a check that is not running. The list
lives beside the guard that reads it.

**Counting a test as a caller.** Every one of the five shipped defects had passing tests over
working code. A test proves the function works, never that anything uses it.
