# Terminal interface

This page describes every key, every slash command, and the status display you see while rho works. rho 0.1.0.

## Starting rho

```
rho
```

`rho` with no subcommand opens the interactive terminal. It enters the alternate screen, draws a header and a footer, and waits for your first prompt.

The first frame shows brand art, your directory, your git branch, the model, the provider,
and four hints:

```
ρ rho  ~/Work/Vibe/rho · main · anthropic/claude-haiku-4.5 · openrouter
                            rho · the harness, unbundled
                  anthropic/claude-haiku-4.5 · openrouter · ready
                          ❯       type a prompt to begin
                          /       list the commands
                          ?       show the keys
                          /guide  take the two minute tour
```

> **Not built yet.** The fourth hint is wrong. `/guide` does not exist, and typing it answers
> `✗ error · /guide is not built yet. See F-slash-commands in docs/features.md.` Use `?` for
> the keys and `/` for the command list. Both work.

## Keys

### Sending and quitting

| Key | What it does |
|-----|-------------|
| Enter | Send the draft |
| Ctrl+D | Quit when the draft is empty |
| Ctrl+C | Cancel the running turn |
| Ctrl+C (twice, while idle) | Quit |

### Writing a multi-line draft

| Key | What it does |
|-----|-------------|
| Shift+Enter | Insert a newline, where the terminal reports the key |
| Ctrl+J | Insert a newline, in every terminal |
| Alt+Enter | Insert a newline, in every terminal |

### Moving and editing in the composer

| Key | What it does |
|-----|-------------|
| Ctrl+A | Move to the line start |
| Ctrl+E | Move to the line end |
| Alt+B | Move one word left |
| Alt+F | Move one word right |
| Ctrl+K | Cut to the line end |
| Ctrl+U | Cut to the line start |
| Ctrl+W | Cut the word to the left |
| Ctrl+Y | Paste the last cut |

### History and panels

| Key | What it does |
|-----|-------------|
| Up / Down | Recall history, or move the selection in an open list |
| Ctrl+R | Open reverse history search |
| Escape | Close an open panel |
| Escape (twice) | Clear the draft |
| ? | Open the help screen |

Ctrl+R matches any part of a past line. The match ignores case.

### Scrolling

PageUp and PageDown scroll whenever the transcript overflows.
Home and End move the view only while the draft is empty.
Inside a draft, Home and End move the cursor to the start and the end of the line.

| Key | What it does |
|-----|-------------|
| PageUp | Scroll the transcript one screen up |
| PageDown | Scroll the transcript one screen down |
| Home | Jump to the oldest row |
| End | Jump to the newest row |

The view follows new output. It stops following when you scroll away.

### External editor

| Key | What it does |
|-----|-------------|
| Ctrl+X Ctrl+E | Open the draft in an external editor |
| Ctrl+G | Open the draft in an external editor |

rho reads `$VISUAL`, then `$EDITOR`, then falls back to `vi`. The draft goes to a temporary file named `rho-draft-<pid>-<nanoseconds>.txt`. rho splits the editor command on whitespace. It does not pass the command to a shell, so a value like `vi; rm -rf ~` runs only `vi`.

### Tool rows

> **Not built yet.** Ctrl+O appears on the help screen as "expand or collapse the newest tool row". No code handles the key today. rho ignores it.

## Slash commands

Type `/` to open the command list. Tab completes. Enter runs the selected command. A click on a list row also runs it.

| Command | What it does |
|---------|-------------|
| `/help` | Open the help screen |
| `/quit` | Leave rho |

> **Not built yet.** `/model`, `/sessions`, and `/guide` appear in the list. Running any of them prints a row that reads, for example: `/model is not built yet. See F-slash-commands in docs/features.md.`

## Reasoning display

rho can show model reasoning in four modes. Set the mode with `--reasoning`, the `RHO_TUI_REASONING` environment variable, or the `tui-reasoning` config key.

| Mode | What you see |
|------|-------------|
| `summary` (default) | One dimmed row, for example `∴ thought for 2.4s` |
| `full` | Summary, plus the reasoning text in dimmed type |
| `live` | Reasoning text while it streams, then collapsed to the summary |
| `off` | Nothing |

## Mouse and copying text

The mouse is captured by default. The wheel scrolls the transcript one row at a time. A click on a slash command row runs it.

With mouse capture on, a plain drag does not select text. In Ghostty and iTerm2, hold Option and drag to select text.

Pass `--no-mouse` to give the mouse back to the terminal. Drag then selects text without a modifier. The wheel stops working, because the alternate screen has no scrollback.

rho runs in the alternate screen. On exit, the shell gets its own buffer back. The transcript is gone. **No transcript file is written to disk.** There is no log to open after a session ends.

## Pasting

A paste of 1000 characters or more collapses to a chip, for example `[paste 12431 chars]`. The full text sends when you submit. A burst of keys within 10 milliseconds counts as a paste, so a pasted `?` never opens help.

## Status line

The footer shows the current state.

| What you see | When |
|-------------|------|
| `◈ working · <duration>` | A turn is running, with a sweep over the word |
| `◈ canceling · <duration>` | Cancellation is in progress |
| `◈ waiting · <duration>` | rho is waiting on an approval panel |
| `ready` | Idle |
| `done · <reason> · <duration>` | After a turn ends |
| `done · error · <duration>` | After a turn that failed |

Reasons after `done`: `end turn`, `max tokens`, `max turns`, `refusal`, `canceled`, `max tool calls`.

> **Not built yet.** rho draws no token count and no cost. The interface keeps a token
> field and never puts it on screen. So you cannot see what a turn spent.

> **Not built yet.** You cannot turn the sweep animation off. The code holds four ways to
> stop it, including a `--no-motion` flag and an `RHO_REDUCE_MOTION` variable, and nothing
> sets any of them. The renderer never asks whether motion is allowed, so the sweep always
> runs. If the movement bothers you, the only escape today is `rho run`, which draws no
> footer at all.

## Startup notices

Notices appear as rows with a `!` glyph on startup.

- The default model, when you gave none.
- How many skills loaded and from where.
- A skill that loaded with a warning.
- MCP servers waiting to connect on the first turn.
- An MCP failure, with the text "The session continues".
- A result store that could not open.

## What does not work yet

> **Not built yet.** `/model`, `/sessions`, and `/guide` sit in the command list and run
> nothing. Each prints `<command> is not built yet. See F-slash-commands in
> docs/features.md.` Ctrl+O is on the help screen as "expand or collapse the newest tool
> row", and no code handles the key, so rho ignores it. There is no token count and no cost
> display anywhere in the interface.
