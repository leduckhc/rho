# D-select-mode-keys

Status: accepted
Date: 20260831-201552
Spec: SPEC-select-text-and-find-the-bottom

## The question

Which keys drive a keyboard-only selection? They must not collide with the composer keys,
the scroll keys, or the cut keys.

## The decision

rho adds a select mode. It works only while the draft is empty and no panel is open.

- `alt-v` begins a selection at the newest visible cell.
- The arrows, `pageup`, `pagedown`, `home`, and `end` move the cursor and extend the
  selection.
- `y` copies and leaves select mode.
- `esc` cancels the selection and leaves select mode.

## Why

`alt-v` is free. `crates/rho-tui/src/bindings.rs` uses `alt-b`, `alt-f`, and `alt+enter`, so
`alt` is already a modifier rho reads. `v` means "visual", like a common editor.

`y` is safe, because select mode captures the key before the draft sees it. `y` means
"yank", like a common editor. `esc` already closes a panel, so select mode is one more panel
it closes.

The empty-draft guard follows `D-scroll-keys-yield-to-an-empty-draft`. A scroll key and a
selection key never steal a key the composer needs.

## What this rules out

- A printable key such as `v` or `y` as a global chord that fires while the user types.
- A collision with `ctrl-k`, `ctrl-u`, `ctrl-w`, `ctrl-y`, or `ctrl-c`.
- A selection key that works while the draft holds text.

## The risk

Some terminals do not report `alt-v` distinctly. The mouse path still works there. A later
change may add a second entry key if a terminal proves unable to send `alt-v`.
