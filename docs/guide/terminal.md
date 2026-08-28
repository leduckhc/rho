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

All four hints work. `/guide` opens the two minute tour, described below.

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
| `/guide` | Open the two minute tour |
| `/help` | Open the help screen |
| `/quit` | Leave rho |

The list marks a command that does not work, so you see it before you press Enter:

```
 ❯ /model      pick the model for this session · not built yet
   /sessions   list, resume, or branch a session · not built yet
   /guide      the two minute tour
```

> **Not built yet.** `/model` and `/sessions` appear in the list and run nothing. Each prints a row that reads, for example: `/model is not built yet. See F-slash-commands in docs/features.md.`

## The two minute tour

```
/guide
```

Three pages. Page one says what rho is, and names the model and the provider this session
uses. Page two says what rho may do to your machine, and the two switches that narrow it.
Page three lists the keys that matter.

| Key | What it does |
|-----|-------------|
| Right arrow, or Space | Next page |
| Left arrow | Previous page |
| Esc | Close the tour |

The footer names the page, as in `page 2 of 3 · ← → pages · esc close`. A next press on the
last page holds there, so the tour never closes under your hand. Every key the tour lists comes
from the same table as the help screen, so the two cannot disagree.

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
| `◈ working · <duration>` | A turn is running. A sweep moves over the word, and `--no-motion` stops it |
| `◈ canceling · <duration>` | Cancellation is in progress |
| `◈ waiting · <duration>` | rho is waiting on an approval panel |
| `ready` | Idle |
| `done · <reason> · <duration>` | After a turn ends |
| `done · error · <duration>` | After a turn that failed |

Reasons after `done`: `end turn`, `max tokens`, `max turns`, `refusal`, `canceled`, `max tool calls`.

> **Not built yet.** rho draws no token count and no cost. The interface keeps a token
> field and never puts it on screen. So you cannot see what a turn spent.

### Stop the animation

```
rho --no-motion
```

`tui-motion = false` in a config file does the same, and so does `RHO_REDUCE_MOTION=1`. The
footer still names the state in words, so nothing is lost but the movement. A redirected
stdout stops it too.

Until this version nothing moved at all: the renderer read a flag that no code ever set, so
the sweep never drew. An earlier version of this page said the opposite.

## Startup notices

Notices appear as rows with a `!` glyph on startup.

- The default model, when you gave none.
- How many skills loaded and from where.
- A skill that loaded with a warning.
- MCP servers waiting to connect on the first turn.
- An MCP failure, with the text "The session continues".
- A result store that could not open.

## What does not work yet

> **Not built yet.** `/model` and `/sessions` sit in the command list and run nothing. Each
> prints `<command> is not built yet. See F-slash-commands in docs/features.md.` The list
> marks both with `· not built yet`, so you see it before you press Enter. Ctrl+O is on the
> help screen as "expand or collapse the newest tool row", and no code handles the key, so rho
> ignores it. There is no token count and no cost display anywhere in the interface.
