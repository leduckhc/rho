# D-a-command-line-is-capped — stdin is a bounded reader

Date: 20260826. Reference: `D-a-command-line-is-capped`.
Spec: `docs/specs/20260819-102749-SPEC-jsonl-frontend.md`, section 4.

## The question

The draft spec's framing section said how to split a line. It said nothing about how long a
line may be. How long may one command line be?

## Why the question matters here

This project has shipped an unbounded reader twice. An unbounded `bash` reader turned 8 MB
of output into 805 MB of memory. See `D-bash-line-cap`. The session reader then got the same
cap for the same reason. See `D-reader-line-cap`. A JSONL frontend reads from another
process, so it is the third reader of the same shape.

A line with no terminator is worse than a long line. A peer that opens the pipe and never
writes `\n` makes the reader grow until the host runs out of memory. No error, no log, no
event.

## The decision

`rho-jsonl` caps one command line at `MAX_COMMAND_LINE_BYTES`, which is 1 MiB. The cap
counts bytes read, not bytes kept. A test asserts the bytes read, because a test that
asserts the size of the kept value passes against the very bug it was written for. That is
the mistake `D-bash-line-cap` records.

An over-long line arrives as a reply with `success: false` and `error: "line_too_long"`. The
reader then discards bytes up to the next `\n` and carries on. The session stays open.

**The discard is bounded too, and every call returns.** A peer that sends no newline at all
would otherwise move the hang from the buffer into the loop: memory stays flat while the
reader spins for ever. So the reader reports `TooLong` once per cap-sized run, rather than
waiting for a newline that may never arrive. Each report says whether it ended on a
newline, so a caller sends one reply per command line and still collapses one enormous
line into one reply. `an_unterminated_line_cannot_grow_without_limit` asserts both halves:
each call returns, and the bytes read stay near the cap.

The cap is a constant, not a config key. A config key that nothing reads is dead surface,
and a cap a caller can raise is a cap a peer can escape. See `D-cap-at-one-choke-point`.

## Why 1 MiB

A prompt is text. 1 MiB of text is about 250,000 tokens, which is larger than most context
windows. So the cap cannot refuse a real prompt, and it still bounds the reader.

## What it rules out

- No unbounded read on stdin.
- No config key for the cap.
- No test that proves the cap by measuring what was kept.
