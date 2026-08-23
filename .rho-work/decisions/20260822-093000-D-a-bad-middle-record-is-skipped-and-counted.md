# A bad record in the middle of a session file is skipped and counted

Date: 20260822. Reference: `D-a-bad-middle-record-is-skipped-and-counted`.
Found by a security review of the reasoning work. It is outside that diff.

## The question

`SessionReader::read_from` treated **any** line that did not decode as a truncated tail:

```rust
Err(_) => {
    truncated_tail = true;
    break;
}
```

So one flipped byte in the middle of a file discarded every record after it, and the warning
said "the session file had a truncated last line". Both halves are wrong: the loss is silent
in the count, and the message names the wrong place.

`D-truncated-tail-warns` is about a crash cutting the **last** line in half. That rule is
right, and it was applied to a case it does not cover.

## The decision

A bad line is a truncated tail only when nothing decodes after it. Otherwise it is corruption
in the middle: skip it, count it, and keep reading. `ReadResult` gains `dropped_records`, and
the reader reports the count once.

## Why

A session file is the user's history. Losing the newest half of it to one bad byte is worse
than loading it with a gap, and a gap that is counted can be investigated. The tail rule keeps
its own behaviour, because a half-written last line really is a crash.

## What it rules out

- No silent loss. A dropped record is counted and reported.
- No misreport. A middle drop never claims to be a truncated tail.
- No fail-open. A record that does not decode is never guessed at or partially applied.

## The evidence

`a_bad_middle_record_does_not_discard_the_rest` writes a header, a good record, a garbage
line, and a second good record. It asserts two entries, `dropped_records == 1`, and
`truncated_tail == false`. Breaking the fix back to `break` fails that test.

## The limit

No production caller writes a session file yet, so this path is reached by tests and by the pi
importer only. See `D-no-caller-writes-a-session-file`. The fix lands now because the reader is
a contract with every file rho will ever write.
