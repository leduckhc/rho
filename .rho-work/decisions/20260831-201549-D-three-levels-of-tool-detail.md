# D-three-levels-of-tool-detail — RowFold becomes a three-level RowDetail

## Question

The user wants three tool-row views: a "super one-liner", a "short-form", and a "full
length" view. `crates/rho-tui/src/concise.rs` holds two levels, `RowFold::Collapsed` and
`RowFold::Expanded`. How many levels ship, and how does a user change one?

## Decision

Replace `RowFold` with `RowDetail { Line, Short, Full }` in `rho-tui`. A row carries one
level. `ctrl-o` cycles the newest tool row through the three levels, and wraps. The
default is `RowDetail::Short`. A failed row starts at `RowDetail::Full`, so a level never
hides an error.

`Line` shows the header alone. `Short` shows the header plus `SHORT_BODY_LINES` body
lines, then a `… +N more` line. `Full` shows the header plus every body line.

## Reasons

- Two levels cannot carry three views. The user named three.
- `Short` is the safe default. `Line` hides too much for a coding agent, and `Full`
  floods the transcript when a `read` returns a whole file.
- A failed row forces `Full`, so the default is fail-safe. An error is never hidden.
- `ctrl-o` is already advertised in the help, so the key is chosen for the user.

## What this rules out

- A global level for the whole transcript. Only the per-row cycle ships.
- A config key or a CLI flag for the default. The default lives in code.
- A lock on a failed row. `Full` is the start level, not a lock. `ctrl-o` still cycles it.
- Persisting a level across a restart.
