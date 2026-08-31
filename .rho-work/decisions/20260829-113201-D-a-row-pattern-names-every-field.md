# D-a-row-pattern-names-every-field

Date: 20260829

## The question

The renderer matched a task row like this:

```rust
Row::Task { command, state: task, .. } => { ... }
```

The `..` was the whole defect. `progress` was written by the reducer, ignored by the row,
and no test failed. `bench/check-dead-surface.py` cannot see this shape, because it finds
an uncalled function and this is a field a reader never reads.

So how does the project stop the next field hiding the same way?

## The decision

**A renderer pattern over `Row` names every field. No `..`, and no `_` on a field that
carries text.**

A field a row does not draw is named, and a comment beside it says why it is not drawn.
The compiler then holds the rule: a new field on a `Row` variant fails the build until
somebody decides whether the row draws it.

This is the same guard shape as `strip_powerful_keys` in `rho-config`. That destructures
`ConfigLayer` exhaustively, so a new config key cannot reach a user unclassified. See
`D-trust-is-provenance-not-a-field-list`. A rule the compiler holds is the only one nobody
can forget.

**A test guards the pattern itself.** The compiler catches a *new* field. It cannot catch a
contributor who adds `..` back, because `..` compiles. So one test reads the renderer
source and fails on a `Row::Task` pattern that ends in `..`. The test states the rule where
a contributor meets it, and it reads the field list from the enum declaration rather than
holding its own copy. A hand-written list is worthless against the case that matters. A
field added tomorrow is not in a list written today.

**What this cannot hold, said plainly.** The rule forces a *decision* per field. It cannot
force a *draw*, because `field: _` compiles and satisfies the pattern. A review named the
stronger claim as an overclaim, and it was right. Only a reviewer closes the last step, and
that is why step 9 of `AGENTS.md` asks for the list of public items with no test.

## What this rules out

- **No `..` in a renderer pattern over `Row`.** Not for brevity, and not for a field the
  row has no plan for yet.
- **No claim that the dead-surface guard covers this class.** It does not. The compiler and
  the source guard cover it, and `SPEC-wire-the-dead-switches` says so already.

## What this decision does not fix

`Row::Agent` still matches `{ name, outcome, .. }`, and it hides five fields, including the
`cost` summary the reducer computes. That is the same defect, in the row next door. It needs
its own layout decision, because `depth`, `turns`, and `cost` all have to find a place on
the row. This lane draws the task row and reports the agent row as an open defect. Applying
this decision to `Row::Agent` without that layout work would only make the omission look
approved.
