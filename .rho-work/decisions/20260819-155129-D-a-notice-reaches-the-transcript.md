# D-a-notice-reaches-the-transcript — a startup notice goes on the screen rho opens

Date: 20260819

## The question

rho printed its startup notices to the terminal, and then opened the alternate screen. So
the notices sat on a buffer the user never looks at again. Where does a notice go now?

## The decision

**A notice is a value, and the interface draws it.** `rho-cli` collects a notice instead of
printing it, and hands the list to `App::with_notices`. The transcript holds each one as a
`Row::Notice`.

A notice keeps its own row type. It does not borrow `Row::Error`.

The rule holds only while the interface exists. An error raised **before** the screen opens
still goes to stderr, because there is no screen to draw it on. A run with no interface, such
as `rho run`, still prints, because a print is the right channel there.

## The reason

Measured on the release binary, at 30 rows by 100 columns:

| What | Where in the output stream |
| --- | --- |
| The model-default notice | byte 5 |
| The skill notice | byte 320 |
| The alternate screen opens | byte 535 |

So every notice was visible for a few milliseconds. One of the hidden lines says a project
skill stays unloaded until the user trusts it. That is a security notice, and silence was the
worst outcome available.

**A notice is not an error.** A default model and an unloaded skill are both worth saying,
and neither one failed. Drawing them with the `✗ error` glyph would tell the user that rho
broke, so the row carries the `!` glyph and the `Warn` role.

**A notice wraps.** The skill notice ends with its action, `Pass --trust-project to load
them`. A padded single line clipped exactly that part at the screen edge, so the user would
read a warning and never read what to do about it.

## The splash survives a notice

The splash was gated on `rows.is_empty()`. Seeding a notice would have retired it, because
every startup in a repository with skills raises one.

A notice is chrome, not conversation. So the splash draws while every row is a notice, and
the notices draw under the starters. **When the notices do not fit, rho falls back to the
scrollable transcript**, because folding them into a block that gets truncated would rebuild
the defect in a new place.

## What this rules out

- **No silent drop.** `Row::Notice` is a variant, so every `match` on `Row` must answer for
  it. A new frontend cannot forget it and still compile.
- **No shared error row.** A caller that wants a notice calls `push_notice`. It does not pass
  a flag to `push_error`.
- **No print in the interactive path.** `build_config` still prints, and
  `build_config_with_notices` collects. The interactive path calls the collecting one. A
  print added back to that path is a defect, and `docs/verification/notices-live.md` holds
  the measurement that catches it.
- **No cap on the count.** rho does not keep the first three notices and drop the rest. It
  falls back to a scrollable transcript instead, so the wheel reaches every one.
