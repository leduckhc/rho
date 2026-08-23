# D-scroll-keys-yield-to-an-empty-draft — one rule decides when a key scrolls the transcript

Date: 20260819

## The question

`SPEC-tui-alternate-screen` section 5 gives `pageup`, `pagedown`, `ctrl-u`, `ctrl-d`,
`home`, `end`, and `↑ ↓` a transcript-scroll meaning. But six of these keys already carry
tested behaviour in `TuiState::handle_key`:

- `ctrl-u` cuts to the line start.
- `ctrl-d` quits while the draft is empty.
- `home` and `end` move the draft cursor.
- `↑` and `↓` recall the history and move the draft cursor row.

One key cannot answer two intents at once. So a single rule must decide.

## The decision

**A scroll key scrolls the transcript only when all three hold: no panel is open, the draft
is empty, and the transcript overflows the screen.** Otherwise the key keeps its old
meaning.

`pageup` and `pagedown` had no draft meaning, so they scroll whenever the transcript
overflows and no panel is open.

The frontend cannot ask this from a pure reducer, because a reducer reads no screen. So the
event loop writes `transcript_total` and `transcript_visible` into the state each frame,
the same way it already writes `composer_width` and `help_visible_rows`. The reducer reads
those two counts and never a terminal.

## The reason

The rule keeps every existing test passing, because each existing test has either a
non-empty draft or a transcript that does not overflow. A unit test builds a state with
`transcript_total` and `transcript_visible` at zero, so the transcript never overflows
there, so the readline and history keys keep answering.

It also keeps the composer whole, which `D-ledger-wins-the-band` still requires. A user who
is typing keeps every readline key, because a non-empty draft blocks the scroll path.

## What this rules out and its cost

- **No modal scroll mode.** rho does not add a key to enter or leave a scroll mode. The
  draft state already says whether the user reads or types.
- **The cost: `ctrl-d` stops quitting while the draft is empty and the transcript
  overflows.** In that one state `ctrl-d` scrolls down instead. `ctrl-c` pressed twice still
  quits in every state, and the footer states it. This cost is small, and it is the price of
  one coherent rule over six per-key exceptions.

## Note on process

The spec's section 5 did not state the reconciliation, and the choice constrains later work.
The implementing agent could not reach the owner interactively, so it recorded this decision
and made the conflict and its cost explicit. Reverse it here if the owner wants `ctrl-d` to
keep quitting in every state.

---

**Superseded on 2026-08-19 by the owner, in the same day.**

The cost above is not small. "The draft is empty and the transcript overflows" is the normal
state of a session, so `ctrl-d` would almost never quit. A habit that works sometimes is
worse than a habit that never worked, because the user stops trusting the key.

**The corrected rule: a key that already carries a meaning never scrolls.**

- `ctrl-d` quits on an empty draft, always.
- `ctrl-u` cuts to the line start, always.
- `↑ ↓` recall the history and move a selection, always.
- `pageup` and `pagedown` scroll a page, because they carry no other meaning.
- `home` and `end` jump to the ends of the transcript while the draft is empty. A draft with
  text needs them for the cursor.
- The wheel scrolls one row.

The fault was in the spec, and not in the implementing agent. Section 5 listed six keys
without checking that four were already taken. The agent found the conflict, recorded it,
stated the cost, and asked for a reversal. That is the process working.

Three tests pin the corrected rule: `ctrl_d_still_quits_while_the_transcript_overflows`,
`ctrl_u_still_cuts_while_the_transcript_overflows`, and
`the_up_arrow_still_recalls_the_history`.
