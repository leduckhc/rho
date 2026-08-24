# The two minute tour, driven in a real terminal

Date: 2026-08-24. Binary: `target/release/rho`, 0.1.0, default features.
Feature: F-tui-guide. Spec: `SPEC-tui-guide`.

`/guide` was promised in two places and built in neither. The slash list carried it, and the
first frame advertised it as a starter hint, so the first screen a new user saw invited them to
run a command that failed. See `docs/verification/tui-slash-commands.md` for the capture that
found it.

`bench/tui_slash_probe.py` drives the real interface in a pseudo-terminal and renders it with
`pyte`. It now walks the pages.

```sh
python3 bench/tui_slash_probe.py
```

## The first frame keeps its promise now

```
                                            ❯       type a prompt to begin
                                            /       list the commands
                                            ?       show the keys
                                            /guide  take the two minute tour
```

Every starter hint that names a command now names a working one. A unit test in
`crates/rho-tui/src/render.rs` pins the rule, so the next hint cannot repeat the mistake.

## The command list says what is unbuilt

```
 ❯ /model      pick the model for this session · not built yet
   /sessions   list, resume, or branch a session · not built yet
   /guide      the two minute tour
   /help       every key and every command
   /quit       leave rho
```

The help screen already marked an unwired key. The list said nothing, so a user spent a
keystroke to learn that `/model` does nothing. `SlashCommand` now carries `built`, the same
flag `Binding` carries.

## Page one

```
  What rho is
rho sends your prompt to a model. It then runs the tools the model asks for.
  model         anthropic/claude-haiku-4.5
  provider      openrouter
  enter         send the draft
  ctrl-c        cancel the turn · press twice while idle to quit
  ready                                          page 1 of 3 · ← → pages · esc close
```

The model and the provider are this session's own. The two key rows carry the binding table's
own summaries, not copies.

## Page two, reached with the space bar

```
  What rho may do here
rho reads, writes, and edits files, and it runs shell commands.
A path outside the session root is refused.
It approves every tool call unless you narrow it.
  --read-only   deny every tool that changes state
  --sandbox     confine the shell: off, confined, or strict
  ready                                          page 2 of 3 · ← → pages · esc close
```

## Page three, reached with the right arrow

```
  Getting around
  /             open the command list · tab completes · enter runs
  ?             open this help
  ctrl-r        search the history
  ctrl-x ctrl-e edit the draft in the editor
  ↑ ↓           recall the history · move the selection in an open list
  pageup        scroll the transcript one screen up
  ready                                          page 3 of 3 · ← → pages · esc close
```

Every row here is generated. Rename a binding and this page follows.

## The boundaries and the exit

A further next press on page three kept page three, and the panel stayed open. The left arrow
returned to page two. Esc closed the panel, and the footer went back to
`enter send · / commands · ? help`.

## The breaks that prove the tests

Six deliberate breaks, each restored from a copy in `/tmp` and never with `git checkout`.

| Break | Result |
| --- | --- |
| Remove the guide from the `handle_chord` guard | FAILED: `ctrl-r must not swap the panel` |
| Let a next press past the last page keep counting | FAILED: `left: 5, right: 2` |
| Hand-copy a key summary instead of reading the table | FAILED: `the row for / must carry the table's own summary, not a copy` |
| Make `guide_panel` return nothing | FAILED: all four render tests |
| Advertise `/sessions` on the first frame | FAILED: `the first frame may not advertise /sessions, which is not built` |
| Stop marking an unbuilt command in the list | FAILED: `an unbuilt command says so in the list` |

**One break lied at first.** The formatter had reflowed the marking code across four lines, so
a one-line pattern did not match, the file never changed, and the test passed. A break that
silently fails to apply gives false confidence, exactly as a self-matching guard does. The
script now asserts the file changed before it trusts the result.

## What this did not cover

- Mouse navigation of the pages. It is out of scope in the spec.
- A short terminal. `panel_lines` truncates from the bottom, and the page cap keeps the loss
  small, but no live run tested a four-row screen.
- The reasoning display modes and the paste chip, which the probe does not touch.
