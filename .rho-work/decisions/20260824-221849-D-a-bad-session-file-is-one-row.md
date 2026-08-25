# D-a-bad-session-file-is-one-row — one unreadable file must not fail a list

**Question:** `SessionStore::list` propagates an error from `File::open` and from
`parse_header`. What does a user with one corrupt file see?

## The defect

Nothing. One unreadable file fails the whole list, so `rho sessions list` shows no rows.

A store gathers junk over a year. A foreign `.jsonl`, a truncated header, or a file from a
newer rho is enough.

## The decision

A file that does not open, or whose header does not parse, becomes one row marked
unreadable. The row names the file and the reason. The list never fails because of one
file.

## Rules that hold

- The list returns rows for every readable file, whatever the unreadable ones do.
- An unreadable row cannot be resumed. It can be deleted, so a user can clean the store.
- A version the reader does not know is a reason, not a crash. The row says so.
- The test is the invariant: for any directory, a list of N files returns N rows.

## Rules out

**Hiding an unreadable file.** A user would not know it is there, and would not delete it.

**Failing the list, as today.** One bad byte then hides every good session.

## Cost

One row variant, and a reason string. The read path already reports its own errors.
