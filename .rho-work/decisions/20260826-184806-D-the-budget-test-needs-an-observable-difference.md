# D-the-budget-test-needs-an-observable-difference — a byte bound with no seam proves nothing

**Question:** `SPEC-session-store-wiring` section 11 names
`a_list_of_five_hundred_sessions_reads_only_the_head_and_the_tail`. It asserts a byte
bound over `SessionStore::rows`. Can that test fail against a `rows` that decodes every
file?

## The defect in the contract

No. `rows` opens every file itself, so no counting source sits under it. A full-decode
`rows` hands out no bytes through any seam the test holds. The byte bound is then a
sentence in a test that measures nothing.

This is the memory-cap defect of `D-bash-line-cap` one level up. `row_from` got a real
seam in section 5a, and the store method that calls it did not.

A review found it, and it was the answer to question 4 of section 14.

## The decision

The budget test observes a **difference in the row**, not a byte count.

One of the 500 files is a sentinel. It carries a `Name` record placed after
`ROW_HEAD_LINES` lines and more than `ROW_TAIL_BYTES` bytes before the end. So the record
sits in the part of the file that a bounded read never touches.

- A bounded `rows` cannot see that `Name` record. The sentinel row falls back to the first
  prompt, and `title_is_explicit` is false.
- A `rows` that decodes the whole file finds the `Name` record. The row then reports the
  explicit name, and `title_is_explicit` is true.

So the assertion is `title_is_explicit == false` on the sentinel row, plus 500 rows in
total. The test fails against a full decode, and it needs no seam.

The wall-clock number is still printed and still asserts nothing. See
`D-a-budget-is-measured-not-asserted`.

## Rules that hold

- Only the sentinel file must exceed `ROW_TAIL_BYTES`. The other 499 stay small, so the
  test writes about 70 kilobytes and not 32 megabytes.
- The same sentinel shape is reused by `a_first_prompt_beyond_the_head_lines_leaves_the_title_empty`.
- `row_from` keeps its counting source. The two tests cover two different risks: one bounds
  a single read, the other proves the store path uses the bounded builder.

## Rules out

**A counting source injected into `rows`.** It needs a new public trait or a closure
parameter that only a test passes. That is dead surface, and
`D-dead-surface-is-a-defect-class` governs it.

**Counting file opens alone.** A full decode opens exactly 500 files too, so the count
cannot tell the two apart.

**Trusting the wall clock.** A shared runner makes it flaky, and a fast machine passes a
full decode of small files.

## Cost

One sentinel file in one test, and one changed assertion.
