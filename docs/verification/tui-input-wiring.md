# Verification: wiring the keys the interface already promised

Date: 2026-08-18. Every command and output here is real. The binary is
`target/release/rho`, built from this tree.

## What the user reported

The user drove the interface and reported four defects.

1. Ctrl-C and Ctrl-D did not close the session.
2. `/` showed no command list.
3. A terminal resize did not redraw.
4. `?` did not open help.

## What was actually wrong

A pty probe against the binary reproduced all four. The screen shows what the user saw:

```
--- 2. after pressing ?
   |╭────────────────────────────────────────────────────────────────╮
   |│ ❯ ?█                                                           │
   |╰────────────────────────────────────────────────────────────────╯
--- 3. after pressing /
   |│ ❯ /█                                                           │
--- 4. resize to 30x60
    widest row after resize to 60 cols: 100
--- 5. Ctrl-D: alive before=True
    alive after Ctrl-D=True
```

The cause was not four bugs. It was one: the input layer was never wired to the interface
sprint 3 had already built. `Panel::SlashList`, `Panel::Help`, `slash_panel`, `help_panel`,
`slash_commands`, `filter_slash_commands`, and `help_rows` all existed, and all passed
tests. `handle_key` answered four keys: Enter, Backspace, a printable character, and
Ctrl-C.

The exit was the sharpest case. `handle_ctrl_c` wrote `press Ctrl-C again to exit` into
`state.status`, and nothing draws that field:

```sh
$ grep -rn "\.status" crates/rho-tui/src/render.rs
$
```

Two Ctrl-C presses did quit. The measurement says 11 ms. The user could not know, because
the interface said nothing. The same silence hid `canceling…` and every `run error: …`.

A resize was one arm: `Some(Ok(_)) => {}` in `app.rs` swallowed `Event::Resize`.

## Driving the fix

Nine checks, and the same script run twice:

```sh
cargo build --release -p rho-cli
python3 /tmp/verify_tui.py
```

```
== 1. pressing / opens a selectable command list ==
  [PASS] the list is on screen
  [PASS] the draft shows the slash
== 2. typing filters the list ==
  [PASS] only /quit survives the filter
== 3. the arrow keys and esc work ==
  [PASS] esc closed the panel and kept the draft
  [PASS] the placeholder returns once the draft is empty
== 4. pressing ? opens help, not a question mark ==
  [PASS] the help panel is on screen
== 5. a resize redraws ==
  [PASS] no row is wider than 60 columns — widest=60
== 6. the first Ctrl-C says how to quit ==
  [PASS] the footer tells the user to press it again
== 7. the second Ctrl-C quits ==
  [PASS] the session closed — 2 ms

9/9 checks passed
```

Four more checks, also twice, each in a fresh process:

```
  [PASS] Ctrl-D on an empty draft quits — 1 ms
  [PASS] Ctrl-D with a draft keeps it and types no letter
  [PASS] /quit plus Enter quits — 2 ms
  [PASS] a mouse click runs the command under it — clicked row 20: reported
```

The mouse check sends a real SGR mouse press, `\x1b[<0;COL;ROWM`, at the drawn row of
`/model`. The interface answers with `\/model is not built yet`.

## Guards, and proof that each one catches its bug

Each mutation went into the source, the suite ran, and the file came back from a copy in
`/tmp`. Never from `git checkout`.

| Mutation | Test that failed |
| --- | --- |
| A chord types its letter again | `a_control_chord_never_becomes_a_letter` |
| `/` no longer opens the list | six slash tests |
| `?` no longer opens help | `question_mark_on_an_empty_draft_opens_help` |
| The armed-exit hint is dropped | `the_footer_tells_the_user_to_press_ctrl_c_again` |
| The `canceling` word is dropped | `the_footer_says_canceling_while_a_cancel_is_in_flight` |
| The mouse mapper is off by one | `a_mouse_row_maps_to_the_command_under_it` |
| Tab runs instead of completing | `tab_completes_the_selected_command_without_running_it` |
| `end_run` stops clearing `canceling` | `a_run_that_ends_with_no_stop_event_returns_to_idle` |
| `end_run` stops returning to idle | three tests, including the Ctrl-C gate |
| A click always runs the first row | `a_click_runs_the_command_on_that_row` |
| A click stops disarming the gate | `a_click_disarms_the_exit_gate` |
| `push_error` stops sanitising | `an_error_row_is_sanitised` |
| The dropped-panel guard is removed | `a_click_below_a_dropped_panel_maps_to_nothing` |

## Two hollow tests of my own, found by mutation

The mutation pass is what caught them, not the review and not the green suite.

`a_click_runs_the_command_on_that_row` clicked `/model`, which is the first row. So it
passed while the click always ran row 0. It now clicks `/guide`, and it asserts that
`/model` did **not** run.

`a_click_disarms_the_exit_gate` armed the gate, then pressed `/`, then clicked. The `/`
key press already disarms the gate, so the click proved nothing. It now opens the panel
first and arms the gate second.

## A review finding that held

An independent review found a defect I had not: `canceling` and `activity` could stick.
`rho_core::Driver::run` returns on `TurnOutcome::Failed` and `Closed` without emitting
`AgentEnd` (`crates/rho-core/src/agent.rs:344`, against the normal path at line 364). The
frontend cleared its stream handles and left `activity` at `Running`. The footer then read
`canceling` forever, and the next Ctrl-C routed to a cancel on a token that was gone, so
**Ctrl-C could no longer quit**. `TuiState::end_run` now ends the run from the frontend
side, and four tests pin it.

The same review noted that `KeyEventKind::Repeat` would defeat the two-press exit gate on
a terminal that reports it. The loop now admits `Press` only.

## What changed in the design record

`SPEC-tui`, `SPEC-tui-experience`, and `docs/tui-design.md` all listed mouse support under
out of scope. The user asked for a list selectable by mouse, so the exclusion is gone in
all three, and the spec now names the new tests. `docs/features.md` moves
`F-slash-commands` from `planned` to a new `partial` status, and the legend defines it.

`docs/design/tui-frames/100-help.txt` changed, and this is the one place where a fixture
followed the code. The binding table grew from seven rows to ten, and every unwired
binding gained a `· not built yet` note, so the drawn help panel is three rows taller and
the transcript keeps three rows fewer. The fixture was regenerated from the renderer, then
read row by row against the design rules. The other 24 checks in `crates/rho-tui/tests/frames.rs`
still hold it to 24 rows, to a display width of 100, and to grid-safe characters.

**The trade-off, stated plainly.** A generated fixture cannot prove the renderer draws
what a human designed. That frame now guards against unintended change only. The other
nine frames stay hand-drawn and untouched.

## Still not wired, and now visible

`bindings()` promises `alt+enter`, `ctrl-o`, and `ctrl-e`, and no key handler answers any
of them. Each needs a fold model or a multi-row composer behind it. The help screen says
`· not built yet` on those three rows, because the help screen became reachable in this
change, and an interface that promises a key it ignores is the defect this whole page is
about.

## The gate

```
cargo fmt --all --check                                    ok
cargo clippy --workspace --all-targets --all-features      0 errors
cargo test --workspace --all-features                      698 passed, 0 failed
cargo build -p rho-cli --no-default-features --features minimal   ok
python3 bench/check-prose.py $(find docs -name '*.md')     VIOLATIONS 0
python3 bench/check-ids.py                                 VIOLATIONS 0
python3 bench/test_ptyharness.py                            0 failure(s)
```

The suite was 667 tests before this work and 698 after.

## Contrast, reported after the wiring landed

The user reported two faults after driving the wired interface. The placeholder read too
bright. The footer read as almost invisible. One line of code caused both.

`style_for` applied the 256-colour value from the role table and the no-colour modifier set
on top of it. So `muted` drew grey 245 **and** `DIM`. The design records 245 at 5.19 to 1,
and that number assumes one dimming. The placeholder drew in the default foreground, though
section 8 of the design gives it `muted`.

No test asserted a style anywhere in the crate. The frame fixtures compare symbols only,
because `render_rows` reads `cell.symbol()`. So every colour was unpinned.

Three tests now pin it, and each one was watched to fail:

| Mutation | Test that failed |
| --- | --- |
| The no-colour `DIM` goes back on top of grey 245 | `a_muted_cell_is_grey_but_never_dim_as_well` |
| The placeholder draws as text again | `the_placeholder_is_muted` |
| The whole footer goes back to muted | `the_footer_activity_word_reads_as_text_and_the_hints_stay_muted` |

A probe reads the escape codes the release binary really writes, rather than a model of
them. `pyte` cannot help here, because it does not model `DIM`.

```
idle frame: DIM sequences (ESC[2m) = 0
idle frame: grey 245 sequences     = 4

placeholder: ESC[38;5;245;49m  before "Type a prompt"
activity:    ESC[39;49m        before "ready"
hints:       ESC[38;5;245;49m  before "enter send"
```

So the placeholder is muted, the hints are muted, the activity word is the terminal
default, and no cell is dimmed twice. See `D-a-role-column-is-not-a-stack`.
