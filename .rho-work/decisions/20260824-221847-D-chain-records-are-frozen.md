# D-chain-records-are-frozen — what tomorrow's record does to today's reader

**Question:** the session file is a persisted contract, so it binds every later rho. What
happens when an old reader meets a record kind it does not know?

## The decision

Records split into two classes, and the class decides the compatibility rule.

**A chain record may be a parent.** The set is frozen: `Session`, `ModelChange`,
`Message`, `Usage`, `Stop`, `Closed`, and `Reopened`. Every version must decode all seven.
A new chain record needs a format version bump.

**A leaf record is never a parent.** A `Name` record is one. An unknown leaf is skipped,
counted, and warned about, exactly as `D-a-bad-middle-record-is-skipped-and-counted`
already states.

**A reader refuses an orphan.** A reader that finds a `parent_id` which resolves to no
decoded record refuses the file. It names both ids.

## What the review changed here

The first draft of this decision said an unknown **leaf** is skipped, and an unknown
**chain** record needs a version bump. A reviewer showed that the reader cannot tell them
apart.

`Record` is a `#[serde(tag = "type")]` enum. An unknown tag fails to decode, so both kinds
land in one undifferentiated dropped count. A rule that assumes the unknown thing is a
harmless leaf is `ToolKind::Other` again.

A second idea also fails. The reader cannot record the id of a line it could not decode,
because the id sits inside that line.

**So the check runs from the child side, and it is referential integrity.** Every non-root
`parent_id` must resolve to a record the reader decoded. That catches a skipped chain
record, because its children point at nothing. It ignores a skipped leaf, because nothing
points at a leaf.

The class is therefore **derived from the data**, and never declared by a writer. A later
rho needs no cooperation from this build.

**A silent early stop becomes an error.** `branch_messages` and `fork` both walk parent
links with `None => break`. So a hole ends the walk in silence. Both now return an error on
a missing parent, as defence in depth for a caller that builds entries by hand.

**Prefer a field over a variant.** No type in the session module sets
`deny_unknown_fields`. So serde ignores an unknown field, and an added optional field is
backward compatible. An added variant is not. So the header gains two optional fields
rather than a new record kind:

```rust
/// The session id. It was implicit in the file name before.
#[serde(default, skip_serializing_if = "Option::is_none")]
session_id: Option<String>,
/// Set when this file came from a fork. It names the source session and record.
#[serde(default, skip_serializing_if = "Option::is_none")]
forked_from: Option<ForkOrigin>,
```

## Why the refusal matters

Today a skipped middle record orphans its children. `branch_messages` walks parent links,
so the walk stops at the hole. A resume would then lose the end of a conversation and
report only a count.

That is a fail-open default, and it is the shape of
`D-plugin-does-not-classify-itself`. A silent truncation of a conversation is worse than
a refusal that names the record.

## Rules that hold

- The header carries the session id, so a file can say who it is. Today
  `parse_header` returns an empty id, because the id lives in the file name only.
- `forked_from` sits in the header, so a list shows lineage from one line.
- A leaf record never appears in a parent chain, and a test asserts that.

## Rules out

**Adding a chain record without a version bump.** It would break every older reader.

**Keeping an unknown record whole and re-writing it.** A reader would carry a payload it
cannot understand, and the append-only file already keeps the original bytes.

**Refusing a file whose version is newer, as the only rule.** It is honest and blunt. It
would refuse a file that only added a leaf record we can safely ignore.

## Cost

Two optional fields, one `ForkOrigin` type, and one orphan check in the reader.
