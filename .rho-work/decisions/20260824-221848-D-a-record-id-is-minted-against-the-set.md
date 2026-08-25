# D-a-record-id-is-minted-against-the-set — a counter mints a duplicate id

**Question:** `SessionWriter::mint_id` formats `r{next_id}` and adds one. Two callers
guess that counter from a count. Is that safe?

## The defect

No. Two paths can mint an id the file already holds.

**`append_to`.** It sets `next_id = read.entries.len() + 2`. The reader drops a record it
cannot decode, so the count runs short. With two dropped records the next append mints an
id that is already in the file.

**`fork`.** It adds one to `next_id` for each copied record. A fork copies a branch, and a
branch is not contiguous. Take the chain `r1`, `r2`, `r4`, where `r3` is a sibling. Three
records copy, `next_id` becomes four, and the next append mints `r4`. The file holds `r4`.

**An imported file.** `F-pi-session-import` brings pi ids, and a pi id is eight hex
characters. A counter means nothing in that file.

A duplicate id breaks the tree. `branch_messages` can walk into the wrong ancestor, and a
fork that takes a record id has two targets.

## The decision

A writer holds the set of ids already in the file. It mints an id that the set does not
hold, and it adds the new id to the set. A count never decides an id.

## Rules that hold

- `append_to` and `fork` both seed the set from the file they opened.
- The invariant is a test, not an example: for any file, every record id is unique.
- The rule holds for an imported file, because it reads ids rather than counting them.

## Rules out

**Seeding from the maximum numeric id.** An imported id is not a number, so the maximum
is undefined there.

**Fixing only `append_to`.** The two call sites share one root cause. A fix in one place
leaves the other, and the defect is the same.

## Cost

One id set per writer. `fork` already builds a map of ids, so the data is at hand.

## Why it was invisible

No production caller writes a session file today. See `D-no-caller-writes-a-session-file`.
So both defects are latent, and the wiring lane is the first caller that can reach them.
