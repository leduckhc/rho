# TUI experience outline — from a demo to a tool you live in

Working file. Not a spec yet. It ends with a list of decisions I need from the owner, and
`docs/specs/` gets a real spec once those are settled.

Date: 2026-08-18. Author: controller.

---

## 1. Where rho actually is today

Measured, not remembered. I read the code and drove the binary for this section.

### 1.1 There is no scrolling at all

`transcript_window` in `crates/rho-tui/src/render.rs` builds rows from the newest backwards,
then drops the overflow:

```rust
// Drop any overflow from the top, so the newest row stays visible.
if lines.len() > rows {
    lines.drain(0..lines.len() - rows);
}
```

No scroll offset exists in `TuiState`. `grep -n "pub scroll\|offset" crates/rho-tui/src/*.rs`
finds only a loop variable. So the transcript is not scrolled, it is **truncated**, and every
row above the window is gone from the screen for good. The session file keeps it. The user
cannot see it.

The app also enters the alternate screen, so the terminal's own scrollback holds nothing.

### 1.2 I made text selection worse, and I need to say so

The click support I added earlier enables mouse capture for the whole session:

```rust
execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
```

With capture on, the terminal hands drag events to rho instead of selecting text. rho does
nothing with them. So a user who could select and copy with the mouse before now cannot,
and gains one clickable list in exchange. That is a bad trade, and section 4 fixes it.

### 1.3 Five subsystems are built, tested, and unreachable

Each of these has a public API and passing tests. Nothing in `app.rs` or `state.rs` calls
any of them.

| Subsystem | Where | What it can already do | Reached by |
| --- | --- | --- | --- |
| `Composer` | `paste.rs` | multi-row draft, paste chips, image chips, backspace across chips | nothing |
| `route_burst` | `paste.rs` | tell a paste burst from typing | nothing |
| `attach_image` | `paste.rs` | copy an image into the session, with a size cap | nothing |
| folds | `concise.rs` | collapse and expand a tool row, with a caret | fixtures only |
| approval panel | `state.rs`, `render.rs` | draw a caution-framed prompt with the verbatim command | fixtures only |

`state.input` is a plain `String`. So the interface has a multi-row composer model in the
crate and a single-line string in the running product.

### 1.4 What does work

Enter sends. Backspace deletes. Ctrl-C cancels a turn, and twice while idle it quits.
Ctrl-D quits on an empty draft. `/` opens the command list, and it filters, and the arrows,
tab, enter, and a click all work. `?` opens help. Esc closes a panel and keeps the draft. A
resize redraws. The header, the footer, durations, the motion sweep, and the theme roles are
all real.

---

## 2. Prior art, from the documents

### 2.1 Claude Code

Read on 2026-08-18: `code.claude.com/docs/en/interactive-mode` and
`code.claude.com/docs/en/fullscreen`.

**It ships two renderers, and that is the important fact.**

| | classic | fullscreen |
| --- | --- | --- |
| Where history lives | the terminal's own scrollback | the alternate screen |
| Scrolling | the terminal does it | the app does it |
| Native selection and copy | yes, the terminal's | no, the app reimplements it |
| Flicker on a long session | yes | no |
| Memory | grows with the terminal | constant |

In fullscreen they then had to rebuild everything the terminal used to give them:

- `PgUp` and `PgDn` scroll half a screen. `Ctrl+Home` jumps to the start. `Ctrl+End` jumps
  to the latest message **and re-enables auto-follow**.
- The mouse wheel scrolls. `CLAUDE_CODE_SCROLL_SPEED` multiplies the notch, 1 to 20,
  because the VS Code terminal sends one event per notch.
- Click and drag selects. The selection copies on release, and `/config` can turn that off.
  `Ctrl+Shift+C` copies. `Shift` with the arrows extends the selection from the keyboard.
- `[` writes the whole conversation into the terminal's native scrollback, so `Cmd+F` and
  tmux copy mode can search it. `v` writes it to a file and opens `$EDITOR`.
- `CLAUDE_CODE_DISABLE_MOUSE=1` keeps native selection and gives up the wheel.

Everything else worth taking:

- **Queued messages.** Enter while the model works queues the message instead of
  interrupting. The queue shows above the input. `Up` from the first row takes it back.
- **A transcript viewer**, `Ctrl+O`, with `{` and `}` to jump between user prompts.
- **Readline editing**: `Ctrl+A`, `Ctrl+E`, `Ctrl+K`, `Ctrl+U`, `Ctrl+W`, `Ctrl+Y`,
  `Alt+B`, `Alt+F`, and `Ctrl+_` to undo.
- **History**: `Up` recalls, and `Ctrl+R` searches it, with a scope over session, project,
  and everything.
- **An external editor**: `Ctrl+G`, or the readline-native `Ctrl+X Ctrl+E`.
- **Multi-line**: `\`+Enter, `Option+Enter`, `Shift+Enter`, and `Ctrl+J` for any terminal.
- **`@` for a file**, `!` for a shell command, `:` for an emoji, `/` for a command.
- `Ctrl+L` redraws, and twice within two seconds it clears.
- `Ctrl+T` toggles a task checklist. `Ctrl+B` backgrounds a command. `Ctrl+Z` suspends.
- `Esc` interrupts the turn. `Esc Esc` clears the draft, or opens a rewind menu.
- A vim mode with normal, visual, text objects, and `.` to repeat.

### 2.2 Codex CLI

Read on 2026-08-18: `learn.chatgpt.com/docs/codex/cli`. Its own screenshot shows the shape:

```
╭──────────────────────────────────────────────────╮
│ >_ OpenAI Codex                                  │
│ model:     gpt-5.6-sol medium   /model to change │
│ directory: ~/code                                │
╰──────────────────────────────────────────────────╯
  /init /status /permissions /model /review         
› Improve documentation in @filename                
  100% context left · ? for shortcuts               
```

What that tells us, and what the docs confirm: a boxed banner with the model and directory,
a short command menu on the first frame, `@` file mentions in the composer, `?` for
shortcuts in the footer, and **context left as a percentage** in the footer. Also
`codex resume` to reopen a chat, image paste into the composer, a searchable plugin picker,
and a review preset menu.

I did not verify Codex's scrolling model from the documents, so this outline claims nothing
about it.

### 2.3 What neither does well

- Both put the whole burden of copy on the app once they take the mouse. Claude Code needed
  four features to repair one, `[`, `v`, copy-on-select, and a kill switch.
- Neither shows the cost of the turn beside the turn. Claude Code shows a PR badge and a
  context percentage; the money is behind a command.
- Neither offers a keyboard-only path to a specific earlier tool result. `{` and `}` jump
  between prompts, and a long turn with 40 tool calls still needs the wheel.

---

## 3. What rho should feel like

Three rules, in priority order. They decide every question below.

1. **The transcript is a document, not a log.** The user can reach any part of it, with the
   keyboard alone, and copy from it.
2. **The terminal keeps its habits.** A user who has never read our help must still be able
   to select text with the mouse and search their scrollback.
3. **Every promise on screen answers a key.** This is `D-a-panel-nobody-can-open`, and it is
   now the house rule.

---

## 4. The frames

100 columns unless stated. These are targets, not the current renderer.

### 4.1 Resting, with the scroll rail

The rail is one column on the right edge. It appears only when the transcript is longer than
the window, so a short session looks exactly as it does today.

```
ρ rho   ~/Work/Vibe/rho · main                sonnet-4.5 · openrouter · 48.2k in, 3.1k out   12m 08s
────────────────────────────────────────────────────────────────────────────────────────────────────
                                                                                                   ┃
❯ add a duration to every tool row, and keep the text beside it still                              ┃
                                                                                                   ┃
Done. Every tool row now carries a duration, and 214 tests pass.                                   █
                                                                                                   █
  ✓ edit  crates/rho-tui/src/render.rs · +18 −4                                        1.2s        █
  ✓ bash  cargo test -p rho-tui                                                        8.4s        ┃
▸   214 passed, 0 failed                                                                           ┃
╭──────────────────────────────────────────────────────────────────────────────────────────────────╮
│ ❯ █                                                                                              │
╰──────────────────────────────────────────────────────────────────────────────────────────────────╯
  ready                                                            94% context · enter send · ? help
```

Changes from today: the scroll rail, the context percentage in the footer, and a diffstat on
an edit row.

### 4.2 Scrolled back, so auto-follow is off

The moment the user scrolls up, new output must stop yanking the view. The follow banner
replaces the top rule and says how to get back.

```
ρ rho   ~/Work/Vibe/rho · main                sonnet-4.5 · openrouter · 48.2k in, 3.1k out   12m 08s
──── 42 new lines below · end jumps to the latest ──────────────────────────────────────────────────
  ✓ read  crates/rho-core/src/agent.rs                                                 0.1s        ┃
  ✓ read  crates/rho-core/src/session.rs                                               0.1s        █
▾ ✓ bash  cargo test --workspace                                                      31.7s        █
      running 698 tests                                                                            █
      test result: ok. 698 passed; 0 failed                                                        ┃
                                                                                                   ┃
❯ now make the footer legible                                                                      ┃
╭──────────────────────────────────────────────────────────────────────────────────────────────────╮
│ ❯ █                                                                                              │
╰──────────────────────────────────────────────────────────────────────────────────────────────────╯
  working · 12s                                        ⇞ ⇟ scroll · end follow · esc stop · ? help  
```

### 4.3 Selection, and the copy story

This is the part Claude Code had to build twice. rho takes the cheaper path first: **the
mouse stays the terminal's** unless the user asks otherwise, and rho gives a keyboard
selection that does not need the mouse at all.

```
────────────────────────────────────────────────────────────────────────────────────────────────────
  ✓ bash  cargo test --workspace                                                      31.7s        ┃
██████ running 698 tests ██████████████████████████████████████████████████████████████            █
██████ test result: ok. 698 passed; 0 failed ██████████████████████████████████████████            █
                                                                                                   ┃
╭──────────────────────────────────────────────────────────────────────────────────────────────────╮
│ ❯ █                                                                                              │
╰──────────────────────────────────────────────────────────────────────────────────────────────────╯
  select · 2 rows                              ↑ ↓ extend · y copy · o open in $EDITOR · esc cancel 
```

Three exits from a selection, and each one already exists in a tool people trust: `y` copies
like vim, `o` opens the selection in `$EDITOR`, and `esc` leaves.

Plus two whole-transcript escapes, both taken straight from Claude Code, because they cost
almost nothing and they end the copy problem:

- `ctrl-p` writes the whole transcript to the terminal's native scrollback, so `Cmd+F`,
  tmux, and mouse selection all work on it.
- `ctrl-x ctrl-e` writes it to a file and opens `$EDITOR`.

### 4.4 The command palette, filtered

Today's list filters by prefix. The target filters by subsequence, and it shows the key that
runs the row.

```
──── / ──────────────────────────────────────────────────────────────────────────────────────────── 
   ❯ /model     pick the model for this session                                            enter    
     /sessions  list, resume, or branch a session                                                   
     /review    review the working tree, and report findings                                        
     /compact   summarise the transcript, and keep the result                                       
╭──────────────────────────────────────────────────────────────────────────────────────────────────╮
│ ❯ /mo█                                                                                           │
╰──────────────────────────────────────────────────────────────────────────────────────────────────╯
  ready                                          ↑ ↓ choose · tab complete · enter run · esc close  
```

### 4.5 A file mention

`@` opens the same list widget over the working tree, so one widget serves both.

```
──── @ ────────────────────────────────────────────────────────────────────────────────────────────  
   ❯ crates/rho-tui/src/render.rs                                                     modified       
     crates/rho-tui/src/state.rs                                                      modified       
     crates/rho-tui/src/app.rs                                                                       
╭──────────────────────────────────────────────────────────────────────────────────────────────────╮ 
│ ❯ explain the layout in @render█                                                                 │ 
╰──────────────────────────────────────────────────────────────────────────────────────────────────╯ 
  ready                                          ↑ ↓ choose · tab complete · enter insert · esc close
```

### 4.6 An approval, with the diff in it

The panel exists and nothing opens it. The target shows the verbatim command, or the diff for
an edit, and the choices.

```
────────────────────────────────────────────────────────────────────────────────────────────────────
 ⚠  approval · edit asks to change crates/rho-core/src/agent.rs                            8s
     @@ -341,7 +341,9 @@
     -                TurnOutcome::Failed | TurnOutcome::Closed => return,
     +                TurnOutcome::Failed | TurnOutcome::Closed => {
     +                    self.emit(AgentEvent::AgentEnd { stop_reason }).await;
     +                }
     [y] allow once    [a] allow for this session    [n] deny    [esc] deny and stop the turn
────────────────────────────────────────────────────────────────────────────────────────────────────
```

### 4.7 A queue, while the model works

```
                                                                                                    ┃
  ◈ working · 41s                                                                                   ┃
     ✓ bash  cargo test --workspace                                                    31.7s        █
  ⋯ queued · 2                                                                                      █
     1  and update the benchmark page                                                               █
     2  then run the prose check                                                                    ┃
╭──────────────────────────────────────────────────────────────────────────────────────────────────╮ 
│ ❯ █                                                                                              │ 
╰──────────────────────────────────────────────────────────────────────────────────────────────────╯ 
  working · 41s                                 ↑ take back · esc stop · ctrl-c cancel · ? help      
```

### 4.8 Narrow, at 46 columns

Every region keeps its meaning, and the hints reduce.

```
ρ rho  ~/rho · main         haiku-4.5   12m   
──────────────────────────────────────────────
❯ make the footer legible                    ┃
                                             █
Done. The activity word takes text now.      █
  ✓ edit  render.rs · +12 −6           1.2s  ┃
╭────────────────────────────────────────────╮
│ ❯ █                                        │
╰────────────────────────────────────────────╯
  ready                              ? help   
```

---

## 5. Modes, and who owns the keyboard

One mode owns the keys at a time. This is the rule that made the wiring defect possible, so
the outline states it as a diagram.

```
                     ┌──────────────┐
        esc          │              │   / or @ or :
   ┌────────────────▶│    normal    │──────────────────┐
   │                 │  (composer)  │                  ▼
   │                 └──────────────┘          ┌───────────────┐
   │                    │        ▲             │  list panel   │
   │        ctrl-y      │        │  enter, esc │  (palette)    │
   │      or ⇞ ⇟        ▼        └─────────────└───────────────┘
   │            ┌──────────────┐                       │ enter
   │            │   scroll     │                       ▼
   │  esc, end  │  (follow off)│               ┌───────────────┐
   └────────────└──────────────┘               │  a command    │
                    │  v                       └───────────────┘
                    ▼
            ┌──────────────┐        y copy, o editor
            │  selection   │───────────────────────────▶ clipboard
            └──────────────┘
```

Two modes not in the diagram, both requested by the prior art and both cheap once the panel
router exists: `!` shell mode, and `ctrl-r` history search.

---

## 6. The target key table

`built` marks what works in the tree today. The help screen already prints a `not built yet`
note for the rest, so this table and the help screen must agree.

| Key | Action | Today |
| --- | --- | --- |
| `enter` | send, or run the selected row | built |
| `ctrl-j`, `alt+enter`, `shift+enter` | insert a newline | missing |
| `ctrl-c` | cancel the turn, twice while idle to quit | built |
| `ctrl-d` | quit on an empty draft | built |
| `/`, `?` | command palette, help | built |
| `@` | file mention | missing |
| `!` | shell mode | missing |
| `esc` | close a panel, keep the draft | built |
| `esc esc` | clear the draft into history | missing |
| `⇞ ⇟` | scroll half a screen | **missing** |
| `ctrl-home`, `ctrl-end` | first row, latest row and follow again | **missing** |
| wheel | scroll, when the mouse is ours | **missing** |
| `v` | start a selection | missing |
| `y` | copy the selection | missing |
| `o` | open the selection in `$EDITOR` | missing |
| `ctrl-p` | dump the transcript to native scrollback | missing |
| `ctrl-x ctrl-e`, `ctrl-g` | edit the draft in `$EDITOR` | missing |
| `↑ ↓` | history, or move the selection in a list | partial |
| `ctrl-r` | search history | missing |
| `ctrl-a ctrl-e ctrl-k ctrl-u ctrl-w ctrl-y` | readline editing | missing |
| `alt-b alt-f` | word motion | missing |
| `ctrl-o` | fold or unfold the newest tool row | missing, model exists |
| `ctrl-e` | fold or unfold everything | missing, model exists |
| `ctrl-t` | show the task list | missing |
| `ctrl-l` | redraw, twice to clear | missing |
| `ctrl-z` | suspend | missing |
| `y a n esc` | answer an approval | missing, panel exists |

---

## 7. Feature matrix

| | rho today | Claude Code | Codex | rho target |
| --- | --- | --- | --- | --- |
| In-app scrolling | none | yes, fullscreen | not verified | yes |
| Native mouse selection | **broken by capture** | optional | not verified | default on |
| Keyboard selection and copy | no | yes | not verified | yes |
| Dump to native scrollback | no | `[` | not verified | `ctrl-p` |
| Multi-row composer | model only | yes | yes | yes |
| History and reverse search | no | yes | not verified | yes |
| External editor | no | yes | yes | yes |
| Command palette | prefix filter | fuzzy, mouse | yes | fuzzy, mouse |
| File mentions | no | `@` | `@` | `@` |
| Shell mode | no | `!` | not verified | `!` |
| Queued messages | no | yes | not verified | yes |
| Approval prompt | panel only | yes | yes | yes |
| Tool row folding | model only | `ctrl-o` | yes | yes |
| Context left | no | yes | yes | yes |
| Cost on screen | tokens only | behind a command | not verified | tokens and cost |
| Image paste | model only | yes | yes | yes |
| Vim mode | no | yes | not verified | later |

---

## 8. A staged plan

Each stage is shippable on its own, and each names its tests. TDD, so the tests land red.

**S1. Scrolling and the follow rule.** A `scroll` offset in `TuiState`, a rail in the
renderer, `⇞ ⇟`, `ctrl-home`, `ctrl-end`, and auto-follow that switches off when the user
scrolls up and on again at the bottom. Wheel support behind the mouse decision in section 9.
Tests: `scroll_up_stops_auto_follow`, `new_output_does_not_move_a_scrolled_view`,
`end_returns_to_the_latest_and_follows`, `the_rail_shows_the_window_position`,
`a_short_transcript_draws_no_rail`, `scroll_clamps_at_both_ends`.

**S2. Copy, and the mouse decision.** Mouse capture becomes opt-in. Keyboard selection with
`v`, `y`, `o`. `ctrl-p` dumps to scrollback. Tests: `capture_is_off_by_default`,
`a_selection_copies_the_rows_it_covers`, `the_dump_writes_every_row_once`.

**S3. The composer, wired.** Replace `state.input` with the `Composer` that already exists.
Newline keys, readline motions, history, `ctrl-r`, `$EDITOR`. Tests: the paste-chip tests
already exist and become reachable, plus `history_recalls_the_previous_prompt`,
`reverse_search_filters_and_accepts`.

**S4. Discovery.** Fuzzy filter for `/`, `@` mentions, `!` shell mode.

**S5. The turn.** Queued messages, `ctrl-o` and `ctrl-e` folds, the approval panel wired to
the real gate, `ctrl-t` tasks.

**S6. Polish.** Context percentage, cost, `ctrl-l`, `ctrl-z`, and a vim mode if the owner
wants one.

---

## 9. Decisions I need

I have a recommendation for each. Say yes, or pick the other one.

1. **The mouse.** Recommend: **native selection wins by default.** rho leaves mouse capture
   off, so drag-select and the terminal's own wheel keep working. `tui.mouse = true` turns
   capture on for the wheel and the clickable list. Claude Code arrived here after shipping
   the opposite, and it needed four repairs.
2. **The scroll rail.** Recommend: draw it only when the transcript overflows. One column,
   muted, no arrows.
3. **The alternate screen.** Recommend: keep it, and add `ctrl-p` as the escape hatch.
   Dropping it would mean giving up the fixed composer, which is the shape both references
   use.
4. **Where the money shows.** Recommend: the footer carries `94% context` and the header
   keeps tokens. A `$` figure appears only when the provider reports one.
5. **Vim mode.** Recommend: out of scope until S6, and then only if you use it.
6. **Scope of the first spec.** Recommend: S1 and S2 together, because a transcript you can
   scroll but not copy is half a feature.

---

## 10. Out of scope for this outline

Markdown rendering, syntax highlighting, an image protocol such as sixel or kitty graphics,
a scrollback search of the transcript itself, a mouse-resizable split, and a second frontend
such as a web view. Each needs its own spec.
