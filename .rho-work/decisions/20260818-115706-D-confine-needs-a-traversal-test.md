# D-confine-needs-a-traversal-test — A path boundary is tested with a traversal, not a sibling

**Question (controller, U4c verification):** the attachment path test refused a file in a
sibling directory, and it passed. Does it prove the boundary holds?

**Decision:** No. A path boundary needs a test that walks **out of the root**, not a test
that starts outside it. Two tests are now required for any confinement check: a path that
starts with the root and escapes it with `..`, and a symlink inside the root that points
outside it. The sibling-directory case stays, because it is cheap, but it proves the least.

**Reason:** The controller replaced `confine` with a lexical prefix test,
`path.starts_with(session_root)`, which is the shape a careless refactor takes. All ten
paste tests still passed. A driver written for the same verification then attached
`<root>/../escape.png` in the first attempt.

The sibling case passes under the broken implementation because a sibling path does not share
the root prefix. So the original test could not fail against the exact defect it was written
to prevent. That is the `D-bash-line-cap` family again: a test that passes against its own
bug buys false confidence.

The real implementation canonicalizes both sides, so `..` collapses and a symlink resolves
before the comparison. The two new tests fail against the lexical version and pass against
the canonical one, which is the proof this decision required.

**Rules out:** A confinement test that only supplies an unrelated outside path. A lexical
prefix comparison in any path boundary. A confinement check that compares a path before it
is canonicalized. A claim that a boundary holds without a traversal case and a symlink case
beside it.
