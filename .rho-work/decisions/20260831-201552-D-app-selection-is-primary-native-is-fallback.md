# D-app-selection-is-primary-native-is-fallback

Status: accepted
Date: 20260831-201552
Spec: SPEC-select-text-and-find-the-bottom

## The question

rho captures the mouse by default. Does rho own selection, or does rho release the mouse so
the terminal selects? What does `tui-mouse` mean after this change?

## The decision

rho keeps mouse capture on by default. The app-managed selection is the primary way to
select and copy. rho does not rely on a native modifier bypass.

`tui-mouse` stays the config key. `--mouse` and `--no-mouse` stay the flags. When capture is
on, rho owns the wheel, the clickable lists, the selection, and the affordance click. When
`--no-mouse` releases capture, the terminal owns selection and scrollback natively. rho then
draws no selection and answers no affordance click. The `end` key still works.

## Why

The user picked the full-depth option. rho owns the selection and holds its own copy. This
keeps selection consistent across every terminal and works when the terminal has no native
selection in the alternate screen.

A modifier bypass is terminal-specific. Shift works on xterm. Option works on iTerm2. rho
names the bypass as a fallback but does not depend on it.

## What this rules out

- A default that releases the mouse to the terminal.
- A design that depends on a native modifier bypass to select.
- App selection while capture is off.

## The risk

Two feature rows in `docs/features.md` disagree on the default. F-optional-mouse says rho
leaves the mouse to the terminal. F-mouse-capture says rho captures by default. The code
captures by default. This decision follows the code. A later docs pass reconciles the rows.
