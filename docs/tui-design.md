# The terminal interface design

Stage U1 of sprint 3. See `workflow-sprint-3.yaml`.

This document designs the interface that `SPEC-tui` will grow into. It builds on
`docs/tui-prior-art.md`, which is the verified brief, and on Makit's design language in
`~/Work/Vibe/makit/DESIGN.md`. The frame mocks live in `docs/design/tui-frames/`, one file
per state, each at an exact width.

The design goal is the owner's bar. The interface must read as well as pi, codex, claude
code, and jcode, in a package that stays faster than all of them. So every element below
earns its rows, its glyphs, and its frame cost. Nothing here is decoration.

Three rules from Makit carry over whole. Content floats and the frame recedes, which in a
terminal means whitespace and one thin rule. One hue accents a neutral field, and the
accent is never a wash. Status has fixed semantics, never ad hoc colour.

## 1. The layout

Ten frame mocks prove this layout. `100-idle.txt` is the reference frame.

| Rows | Region | Fixed or grows |
| --- | --- | --- |
| 1 | Header content | fixed, 1 row |
| 1 | Header rule, `─` full width | fixed, 1 row |
| rest | Transcript | grows, scrolls, newest row kept visible |
| 0 to 7 | A transient panel: slash list, help, or approval | fixed per panel, absent by default |
| 3 to 10 | Composer, a rounded box | grows with the draft, capped |
| 1 | Footer: activity and key hints | fixed, 1 row |

The footer holds two roles on one row. The activity word and its duration take `text`,
because they report the current state. The key hints on the right take `muted`.

The fixed cost is six rows: header, rule, a three-row composer, and the footer. A 24-row
terminal gives the transcript 18 rows. Only the transcript scrolls. When the composer or
a panel grows, it takes rows from the transcript, never from the header or the footer.
When the transcript is long, it drops rows from the top, exactly as today's renderer does.

## 2. The header

One content row and one rule. Two rows total.

```
ρ rho   ~/Work/Vibe/rho · main                sonnet-4.5 · openrouter · 48.2k in, 3.1k out   12m 08s
────────────────────────────────────────────────────────────────────────────────────────────────────
```

pi, codex, and claude code each spend two or three rows on identity. rho spends one, and
puts the whole session on it. That density is the statement: the header stuns by what it
fits, not by what it fills. Each element earns its place:

- `ρ rho` — the brand mark. A user with five terminals open needs the product named. The
  `ρ` glyph renders in the accent role, so the brand is the first green on screen.
- `~/Work/Vibe/rho · main` — the working directory and the git branch. An agent acts on a
  tree, so the tree must be named where the eye rests.
- `sonnet-4.5 · openrouter` — the model and the provider. The model decides quality and
  cost, so it never hides in a menu.
- `48.2k in, 3.1k out` — the token count, in the exact format `state.rs` already emits.
  It is the honest cost meter for providers that report no charge.
- `12m 08s` — the session clock, right aligned in the seven-column duration slot. It
  ticks while a turn runs. See section 7.

The rule row is the frame that recedes. It draws in the muted role, full width, computed
at draw time exactly like pi's `DynamicBorder`.

When the terminal narrows, the header drops elements in this order: the token count, the
provider, the directory, the branch, and last the model. The brand and the session clock
survive at every width. `40-streaming.txt` shows the 40-column result.

## 3. The palette, by role

jcode's lesson: name roles, never colours. A role list is testable, and a theme
(`F-themes`) is a role table a plugin may replace without touching layout. rho paints no
background of its own; the terminal keeps its own. The table below is the dark theme, the
default. Section 4 gives the light table and every measured ratio.

| Role | Used for | 256-colour | 16-colour | No colour |
| --- | --- | --- | --- | --- |
| `text` | assistant body, user text, header title, the footer activity word | terminal default | default | plain |
| `muted` | thinking, rules, hints, payloads, placeholders | 245 `#8a8a8a` | default + dim | dim |
| `accent` | brand, user marker `❯`, running, ok, selection | 78 `#5fd787` | green | bold |
| `error` | failed tools, error rows | 203 `#ff5f5f` | red | bold |
| `warn` | a live duration past a minute, awaiting input | 179 `#d7af5f` | yellow | bold |
| `caution` | the approval panel | 173 `#d7875f` | yellow + bold | bold, reversed glyph |

The three colour columns are alternatives, one per terminal mode. They never stack. A
renderer that paints the 256-colour value and then adds the no-colour modifier dims a role
twice, and the ratios in section 4 assume one dimming. See
`D-a-role-column-is-not-a-stack`.

Makit's status semantics map one to one. Running, connected, and ok take `accent`, which
is Makit's primary green. Error and offline take `error`. Idle, exited, and muted take
`muted`, which is Makit's outline. Awaiting input takes `warn`, Makit's `--status-warning`.
Awaiting approval takes `caution`, Makit's `--status-caution`, the stronger act-now hue.

In 16-colour mode, `warn` and `caution` share yellow. The glyph and the bold weight keep
them apart, so the fallback loses no meaning. With no colour at all, every state still
reads, because every state carries a glyph. Section 5 lists them.

The header roles follow jcode's split: the brand mark takes `accent`, the title takes
`text`, and the metadata takes `muted`.

### The glyph set, and its ASCII tier

Every glyph is single width and has an ASCII fallback. The fallback tier is rho's
existing set from `render.rs`, kept on purpose. It activates when the locale is not
UTF-8, or when the user sets `tui.glyphs = "ascii"`.

| Meaning | Glyph | ASCII |
| --- | --- | --- |
| user marker, prompt, selection | `❯` | `>` |
| thinking | `∴` | `~` |
| tool ok | `✓` | `+` |
| tool failed, error row | `✗` | `x` |
| tool pending | `·` | `.` |
| tool running | `●` | `*` |
| collapsed caret | `▸` | `>` |
| expanded caret | `▾` | `v` |
| activity mark | `◈` | `*` |
| approval mark | `!` | `!` |
| separator | `·` | `\|` |
| rules and borders | `─ ╭ ╮ ╰ ╯ │` | `- + \|` |
| truncation | `…` | `..` |

## 4. Contrast

rho draws no background, so ratios are measured against reference surfaces. The reference
pair comes from Makit's neutral ramp: dark `#171717`, light `#FAFAFA`. The dark table is
the default. `tui.theme = "light"` switches tables. Every pair below meets WCAG AA for
normal text, which requires 4.5 to 1.

Dark theme, on `#171717`:

| Role | Colour | Ratio | AA |
| --- | --- | --- | --- |
| `text` | `#eeeeee` (reference default) | 15.45:1 | pass |
| `muted` | 245 `#8a8a8a` | 5.19:1 | pass |
| `accent` | 78 `#5fd787` | 9.87:1 | pass |
| `error` | 203 `#ff5f5f` | 6.02:1 | pass |
| `warn` | 179 `#d7af5f` | 8.70:1 | pass |
| `caution` | 173 `#d7875f` | 6.42:1 | pass |

Light theme, on `#FAFAFA`. The vivid hues fail on light, exactly as Makit found, so the
light table resolves darker indices:

| Role | Colour | Ratio | AA |
| --- | --- | --- | --- |
| `text` | 234 `#1c1c1c` | 16.33:1 | pass |
| `muted` | 241 `#626262` | 5.84:1 | pass |
| `accent` | 22 `#005f00` | 7.63:1 | pass |
| `error` | 160 `#d70000` | 5.17:1 | pass |
| `warn` | 94 `#875f00` | 5.49:1 | pass |
| `caution` | 130 `#af5f00` | 4.51:1 | pass |

The slash-list selection renders reversed, foreground and background swapped, so its
ratio equals the unreversed pair by symmetry. The cursor is one reversed cell, with the
same property. No information rides on colour alone: every state pairs a colour with a
glyph or a modifier, so a monochrome terminal loses nothing but warmth.

## 5. The transcript

Each row kind is told apart by a glyph and by shape, never by colour alone. A blank row
separates turns, which is the whitespace that Makit's glass becomes in a terminal.

| Kind | Shape |
| --- | --- |
| User turn | `❯ ` marker in `accent`, then the text in `text` |
| Assistant turn | plain `text`, flush left, no marker, wrapped to width |
| Thinking | `∴ thought for 2.4s` in `muted`, collapsed to one row |
| Tool row | verb, payload, duration slot, status glyph, caret. Section 6 |
| Error | `✗ error · <message>` in `error`, detail lines indented and `muted` |
| Approval | a framed panel above the composer. See below |

A thinking row while streaming reads `∴ thinking · 2.4s`, with the duration ticking. Once
finished it reads `∴ thought for 2.4s`. Pressing enter on a selected thinking row expands
the full text, indented two columns in `muted`.

An error row shows the message on the glyph line and the verbatim detail under it,
indented two columns. `100-error.txt` shows a provider error with its retry header kept.

The approval prompt is the one element that interrupts, so it is the one element with a
frame. Two full-width rules in `caution` bracket three lines: the request, the verbatim
command, and the choices.

```
────────────────────────────────────────────────────────────────
!  approval · bash asks to run                              8s
     rm -rf target && cargo build --release
     [y] allow once    [n] deny    [esc] deny and cancel the turn
────────────────────────────────────────────────────────────────
```

There is no allow-always choice, because D-no-remembered-execute-allow forbids a
remembered execute approval. With no colour, the rules and the `!` render bold, and the
`!` cell reverses. Subagent and task rows keep their current grammar and adopt the same
duration slot and glyph tier.

## 6. The tool row

The concise form follows Makit exactly: a verb, a compact payload, a duration in a fixed
slot, a status glyph, and a caret. In that order.

```
read  crates/rho-tui/src/render.rs · 220 lines                                            0.3s ✓ ▸
bash  cargo test --workspace --all-features                                             1m 12s ● ▾
```

The exact characters, left to right:

- The verb, in `text` with bold. One word, the tool name.
- Two spaces, then the payload in `muted`. The payload drops a shell prologue such as
  `cd x && `, and truncates a path to its last three segments with a leading `…/`.
- The duration, right aligned in the seven-column slot, then one space.
- The status glyph: `·` pending, `●` running in `accent`, `✓` ok in `accent`, `✗` failed
  in `error`. Then one space.
- The caret, in `muted`: `▸` collapsed, `▾` expanded. The caret is the last column.

Collapsed is the default. This is concise mode, and it is not a mode a user must find:
it is simply how rho renders. A failed tool row expands itself, because the output is
now the point. Expanded, the header row stays and the body shows the last twelve output
lines, indented four columns, verbatim and sanitised. `ctrl-o` toggles the newest row,
enter toggles a selected row, and `ctrl-e` expands everything at once for a review pass.
`100-tool-run.txt` shows one collapsed row and one expanded running row.

## 7. Durations

The format is Makit's ladder, taken exactly, with its carry tests. Rounding happens once,
at the top, then integer arithmetic below, so `59.5s` can never print `60s`.

| Span | Output |
| --- | --- |
| under 9.95 seconds | one decimal, trailing `.0` stripped: `2.4s`, `2s` |
| 10 to 59 seconds | whole seconds: `13s`, `59s` |
| 1 to 59 minutes | zero-padded seconds: `2m 41s`, `18m 04s` |
| 1 to 23 hours | zero-padded minutes: `4h 12m`, `1h 00m` |
| a day or more | `3d 4h` |

An unrepresentable span renders as an empty slot: a negative span, or a span with no end
event. The widest rung is seven columns, `18m 04s`, so every duration sits in a
right-aligned seven-column slot. Text after the slot never moves when a live value grows
from `9.1s` to `2m 41s`. Where each duration lives:

- The session: the header, rightmost. It ticks while a turn runs.
- The turn: the footer, after the working word, live: `◈ working · 12s`. After the turn
  ends the footer keeps it: `done · end turn · 41s`.
- A tool call: its row slot, live while running, final when done.
- A thinking block: its `∴` row, live while streaming, final when done.

A live duration turns `warn` amber once it passes a minute, Makit's escalation, and
returns to its normal role when it completes. Every tick comes from the tick count the
state carries. No render reads a clock.

## 8. The composer

A rounded box, `╭─╮ │ ╰─╯`, borders in `muted`, prompt `❯ ` in `accent`. The box is the
one drawn frame in the resting interface, because the draft is the one thing the user
owns. One text row at rest, so three rows with borders.

The placeholder, shown in `muted` when the draft is empty, is exactly:

```
Type a prompt. / for commands. ? for help.
```

- **Multi-line drafts.** `alt+enter` inserts a newline, and `shift+enter` does too where
  the terminal distinguishes it. Continuation rows indent two columns to align under the
  prompt. The box grows one row per line, to a cap of eight text rows, ten with borders.
  Past the cap the draft scrolls inside the box, the cursor row stays visible, and the
  top row shows a `muted` `…` in its first column.
- **A large paste.** A paste over 1000 characters collapses to one placeholder chip:
  `[paste 12431 chars]`. A second paste of the same size reads `[paste 12431 chars #2]`,
  codex's repeat suffix, because two same-size pastes must stay distinct. The full text
  is held aside and reaches the model on send. The chip renders in `muted` reversed, is
  deleted as one unit, and the burst detector from the prior-art brief feeds terminals
  without bracketed paste through the same path.
- **An image attachment.** An image paste or path becomes the chip `[image #1 1.2MB]`,
  same behaviour as a paste chip. An attachment over the provider limit is refused in
  place: the footer states `image too large: 12MB, the limit is 5MB` in `warn`.

## 9. The motion

One motion. While a turn runs, a highlight band sweeps the working word in the footer,
left to right. Nothing else animates: no spinner frames, no idle motion, no 3D set.

- **What moves.** The word after `◈` in the footer: `working`, `thinking`, or `waiting`.
- **Period.** Two seconds. The state carries a tick count, advanced every 100
  milliseconds by the event loop, only while running. The frame is a pure function of
  `tick % 20`, so a test asserts any frame by picking a tick.
- **Shape.** A raised-cosine band of half-width five columns, with ten columns of
  padding at each end, so the sweep enters and leaves cleanly. This is codex's shape
  without codex's clock read, and without its per-character allocation.
- **True colour.** Each cell blends the `muted` foreground toward `text` by the cosine
  weight, up to 0.9, and takes bold at the peak.
- **256 colours.** Three steps by weight: below 0.2 the cell is `muted`, to 0.6 it is
  `text`, above 0.6 it is `text` bold.
- **No colour.** The same three steps as modifiers only: dim, plain, bold.

The sweep stops, and the word renders plain, under any of these: the user sets
`tui.motion = false` or passes `--no-motion`, stdout is not a terminal, or a
reduced-motion preference is set via `tui.reduce_motion = true` or `RHO_REDUCE_MOTION=1`.
With motion off the interface loses no information, because the word itself names the
state and the durations still count.

## 10. Discovery

A first-time user finds everything from the footer, which always shows the current three
moves. Idle, it reads `enter send · / commands · ? help`.

- **The slash-command list.** Typing `/` in an empty draft opens the list above the
  composer, framed by two full-width rules, pi's width-computed border. Typing filters
  it. `↑ ↓` choose, enter runs, tab completes, esc closes and keeps the draft. A left
  click runs the row under the pointer. The selected row carries the `❯` marker and
  renders reversed. `100-slash-list.txt` shows it open.
- **The shortcut list.** Typing `?` in an empty draft opens the key list in the same
  framed panel. Every binding on it is read from the real binding table, so the help can
  never drift from the keys. `100-help.txt` shows it.
- **The guide.** `/guide` runs a two-minute tour in the transcript itself: it prints a
  short sequence of example rows and names each part. The empty state and the help panel
  both name it, so the path is two keys long from first launch.

The first Ctrl-C while idle prints `press ctrl-c again to quit · any key to stay` in the
footer, which teaches the exit without a document.

## 11. The empty state

The first frame a new user sees, and the one people screenshot. The transcript area
centres a small block: the `ρ` mark drawn in four rows of half-blocks in `accent`, the
word row `rho · the harness, unbundled`, the session row `sonnet-4.5 · openrouter ·
ready`, and four starter lines that each name one key and one outcome. Below it, the
composer waits with its placeholder, and the footer shows the three moves.

```
      ▄▀▀▄
      █  █
      █▄▄▀
      █

      rho · the harness, unbundled
```

`100-empty.txt` is the full frame. Nothing in it is fake: every line states a real key,
the real model, and the real state. The block art drops in the ASCII tier and at narrow
widths, replaced by the plain `rho` wordmark, so the empty state survives every terminal.

## 12. Narrow terminals

`80-streaming.txt` and `40-streaming.txt` prove the layout at both widths.

At 80 columns: the header drops the provider name. Everything else holds, and the
transcript wraps to the narrower measure.

At 40 columns, in order: the header keeps only the brand, the model, and the session
clock. The footer hints reduce to `? help`. Tool payloads truncate with `…`. The empty
state drops the block art. What never drops: every glyph, every duration slot, every
caret, the composer box, and the approval panel's choices. Nothing at 40 columns is
unreadable; it is only shorter.

## 13. The frame index

| File | Shows |
| --- | --- |
| `100-idle.txt` | a finished exchange, collapsed tool rows, idle footer |
| `100-streaming.txt` | mid-turn text streaming, working sweep, live turn clock |
| `100-tool-run.txt` | an expanded running tool past a minute, amber-eligible |
| `100-approval.txt` | the caution-framed approval panel |
| `100-error.txt` | a provider error row with its detail |
| `100-empty.txt` | the first-launch frame |
| `100-slash-list.txt` | the command list, open and selected |
| `100-help.txt` | the shortcut list from the real binding table |
| `80-streaming.txt` | the 80-column drop tier |
| `40-streaming.txt` | the 40-column drop tier |

Every mock is exactly its stated width, generated and column-counted by script, with
every glyph single width.

## Out of scope

Markdown rendering, syntax highlighting, scrollback search, a model
picker, and a session picker stay out, as `SPEC-tui` already states. This document
designs no feature beyond the owner's list: paste collapsing, motion, durations, concise
mode, attachments, shortcuts and help, the guide, slash commands, the header, and the
theme surface for plugins.
