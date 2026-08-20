# The inline band, driven for real

Date: 2026-08-18. Author: controller. Stage: sprint 4, S1 to S3 of
`SPEC-tui-inline-and-composer`.

The suite said 806 tests pass. This page holds what a terminal said. Four defects appear
here that no test caught, and each one names why.

## The commands

```sh
cargo build --release -p rho-cli
tmux new-session -d -s rho -x 100 -y 30 \
  "printf 'BEFORE\n'; ./target/release/rho --model anthropic/claude-haiku-4.5; printf 'AFTER\n'"
tmux send-keys -t rho "<prompt>" && tmux send-keys -t rho Enter
tmux capture-pane -p -t rho            # the screen
tmux capture-pane -p -t rho -S -300    # the screen and the history
```

`tmux` is the terminal here because it keeps a real scrollback that a script can read. A
screen model such as `pyte` cannot answer the question this design rests on.

## What a session looks like

```
BEFORE                                                       ← the shell output, untouched
ρ rho  ~/Work/Vibe/rho · main · anthropic/claude-haiku-4.5 · openrouter
❯ reply with exactly: hello from rho
hello from rho
❯ run bash to print the word banana, then tell me what it printed
bash  banana                                                        0s ✓
It printed: banana
╭──────────────────────────────────────────────────────────────────────╮
│ ❯ Type a prompt. / for commands. ? for help.                         │
╰──────────────────────────────────────────────────────────────────────╯
  done · end turn · 1.2s                    enter send · / commands · ? help
```

Everything above the composer box is ordinary terminal output. The wheel scrolls it, a drag
selects it, and the terminal search finds it. rho draws none of that.

## What the run proves

| Claim | How it was checked | Result |
| --- | --- | --- |
| No alternate screen | the escape stream holds no `?1049h` | true |
| A finished row reaches the scrollback | `capture-pane -S -300`, then count each row | each row once |
| The same failure twice is stable | two prompts that both escape the session root | two rows, two errors |
| A resize keeps the band whole | `resize-window` to 70x20 during a session | the box redrew at the new width |
| The turn clock works | the footer after each turn | `done · end turn · 1.2s` |
| A tool duration works | the tool row's slot | `0s`, `0.5s`, `1.4s` |
| Exit leaves the terminal sane | `ctrl-c ctrl-c`, then the shell prints | `AFTER` printed under the transcript |
| The composer edits | `ctrl-j`, `ctrl-a`, `ctrl-e`, `ctrl-u`, `ctrl-y` | each key did its job |
| The history recalls | `↑` after two submits | the previous draft came back |
| The search lists matches | `ctrl-r` then `pine` | `❯ say pineapple` listed and marked |

## The four defects the tests could not see

**1. The first frame panicked.**

    index outside of buffer: the area is Rect { x: 0, y: 2, width: 100, height: 14 }
    but index is (0, 0)

An inline viewport anchors to the cursor row, so `Frame::area()` has a non-zero origin. The
renderer wrote at an absolute `(0, 0)`. Every fixture used `TestBackend` through
`Terminal::new`, which is fullscreen, where the origin really is `(0, 0)`. So 226 tests
passed against a product that could not draw one frame.

The guard is now a test with `Viewport::Fixed(Rect::new(0, 2, 100, 14))`, which reproduces
the offset with no terminal: `the_band_draws_at_the_viewport_origin`.

**2. The banner read `ρ rho   ·  · model ·`.**

Nothing in the product ever wrote `cwd`, `branch`, or `provider`. The renderer has drawn
three separators around empty fields since sprint 3. `TuiState::set_context` and
`App::with_context` fill them, `rho-cli` passes them, and an empty field now draws no
separator.

**3. Exit left a lie on screen.**

The last frame stayed after rho left, so the footer still read `ctrl-c again quits`. The
restore path clears the band first now.

**4. `ctrl-j` typed nothing.**

The draft read `line oneline two` on one row. `Composer` had a cursor, motions, and a kill
buffer, all tested, and **no key reached any of it**, because `TuiState` still held
`input: String`. This is the family this project keeps repeating: `confine`,
`ToolKind::Other`, the approval panel, the row durations, and the whole `rho-config` crate.

A fifth defect came from the same live pass. `ctrl-r` opened a panel that listed nothing,
while the function's own doc comment claimed it drew "the matches, newest first". The panel
never received the history. It lists matches now, and it says `no match in this session`
when it finds none.

## What is still unverified

The mouse path. `--mouse` and `RHO_TUI_MOUSE` reach `setup_sequences`, and no live run has
tested a wheel or a click with capture on.

The external editor. `ctrl-g` returns the action and the loop spawns `$EDITOR`. A live run
has not driven a real editor yet.

iTerm2, Terminal.app, and Alacritty. Every live run here used `tmux` inside one terminal.

The band flicker. `docs/verification/inline-viewport-spike.md` measures the write cost, and
no human has judged the result.
