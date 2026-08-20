# Prior art for the terminal interface

The recon stage of the terminal interface work, in sprint 3. `agentic-workflow.yaml` holds
that stage kind now.

Every claim here names the file it came from. A claim with no source was deleted, and a
claim that the source contradicted is recorded as a correction, because a plausible memory
of another tool's interface is exactly the kind of error that survives a review.

The tools are pi, the codex CLI, claude code, jcode, and Makit. Makit is the owner's own
product, and it is the reference for durations and for the concise form.

## How each source was read

| Tool | Source | Method |
| --- | --- | --- |
| pi | `/opt/homebrew/lib/node_modules/@earendil-works/pi-coding-agent/docs/tui.md`, 942 lines | read on disk |
| codex | `github.com/openai/codex`, branch `main`, `codex-rs/tui/src` | the contents API for names, then `curl` with `grep` and `sed` for regions |
| jcode | `github.com/1jehuang/jcode`, branch `master`, `crates/jcode-tui-*` | the same method |
| claude code | ships as a bundled binary | **not read.** No claim below rests on it. |
| Makit | `~/Work/Vibe/makit/DESIGN.md`, `app/lib/ui/session/elapsed.dart`, `app/test/elapsed_test.dart` | read on disk |

Reading a large source through `curl` with a filter matters. Two subagents failed this
stage, one by loading whole web pages until its context overflowed. The bounded method is
now in the sprint-3 monitoring rules.

## The one correction

A first report attributed the tool glyphs `·`, `*`, `+`, and `x` to pi. They are **rho's
own**, in `crates/rho-tui/src/render.rs` line 213 to 219. `docs/tui.md` states no glyph
set. So rho has no evidence about pi's glyphs, and it needs none.

## What rho renders today

`crates/rho-tui/src/render.rs` line 24 to 38 splits the frame into three parts: the
transcript, a one-row status line, and a one-row input. The spinner is the single character
`*`, at line 17. The tool glyphs are ASCII, at line 213. There is no header, no motion
beyond one character, no duration anywhere, and no fold.

That is the baseline the critics will judge.

## pi

| Aspect | Finding | Source |
| --- | --- | --- |
| Motion | A working indicator with a frame list and an interval. The documented example uses `· • ● •` at `intervalMs: 120`. An extension may replace the frames, empty the list to hide it, or restore the default. | `docs/tui.md` line 765 to 788 |
| Cursor | A component emits `CURSOR_MARKER`, a zero-width APC escape. The host scans the rendered output for it, so no component tracks a cursor position. | `docs/tui.md` line 36 to 51 |
| Framing | `DynamicBorder` renders a full-width rule and recomputes its width at draw time, so it needs no layout pass. It frames a `SelectList` for a picker. | `docs/tui.md` line 614 to 654 |
| Extension | A widget above or below the editor, through a render, invalidate, and optional input trio. A command registers by name. | `docs/tui.md`, the extension sections |

**Take:** the replaceable working indicator, and the width-computed rule.
**Refuse:** nothing here is refused. pi's cost is its runtime, not its design.

## codex

The closest prior art rho has, because it is Rust on ratatui. Its interface crate holds 146
files under `codex-rs/tui/src`, and its composer alone is 499 KB of source.

| Aspect | Finding | Source |
| --- | --- | --- |
| Large paste | A paste over `LARGE_PASTE_CHAR_THRESHOLD` becomes a placeholder element, and the full text is held aside in `pending_pastes`. The label is `[Pasted Content N chars]`. A second paste of the same size gets `#2`, a third `#3`. When every placeholder of a size is deleted, the next paste of that size reuses the plain label. | `bottom_pane/chat_composer.rs` line 111 to 119, and line 333 for the threshold, which is 1000 characters |
| Paste without bracketed paste | Some terminals deliver a paste as a burst of key events. A burst detector buffers them and flushes them through the paste path, so a pasted `?` cannot trip a shortcut. | `bottom_pane/chat_composer.rs` line 141 to 163, and `bottom_pane/paste_burst.rs` |
| Attachments | A remote image URL renders as a non-editable `[Image #N]` row above the text area. Local image paths and remote URLs are separate attachment kinds, and both survive a history recall. | `bottom_pane/chat_composer.rs` line 44 to 48, and line 122 to 124 |
| Motion | `shimmer.rs`, 80 lines. A band sweeps the text on a 2-second period. The band is a raised cosine of half width 5 characters. Ten characters pad each end, so the sweep enters and leaves cleanly. With true colour it blends the terminal background toward the foreground by up to 0.9 and adds bold. Without true colour it falls back to dim, plain, then bold at intensity 0.2 and 0.6. | `shimmer.rs` line 21 to 79 |
| Other motion | `ascii_animation.rs`, and a status indicator widget of its own. | `tui/src/ascii_animation.rs`, `tui/src/status_indicator_widget.rs` |
| Terminal control | A custom terminal of 47 KB, and a history-insert path of 44 KB. | `tui/src/custom_terminal.rs`, `tui/src/insert_history.rs` |

**Take three things.**

1. The paste placeholder, including the repeat suffix. It is the feature the owner asked
   for, and codex has already found the awkward part: two pastes of the same size must be
   distinguishable.
2. The burst detector. A terminal without bracketed paste is common enough that codex wrote
   a state machine for it, and a pasted `?` opening a help overlay is a real defect.
3. The shimmer shape. A raised-cosine band with padding reads far better than a linear
   ramp, and the no-true-colour fallback is three modifiers rather than a colour.

**Refuse one thing, and improve on it.** `shimmer.rs` reads the clock inside the render, at
line 16 to 19, through a process-wide `Instant`. So a frame is not a pure function of state,
and no test can assert a shimmer frame. rho passes an elapsed tick into the state instead,
which keeps `SPEC-tui`'s rule that a render is pure and testable. The same function also
allocates a `String` for every character of the shimmering text, on every frame, at line 66.
rho must not.

## jcode

| Aspect | Finding | Source |
| --- | --- | --- |
| Crate split | 84 crates, of which 16 carry the interface. They separate render, style, anim, markdown, messages, tool display, permissions, and a usage overlay. | `crates/`, contents API |
| Theme roles | Named roles rather than colours. The roles are user, ai, tool, file link, dim, accent, system message, queued, asap, and pending. Text and background roles follow. It also names three header roles: header icon, header name, and header session. | `jcode-tui-style/src/theme.rs` line 5 to 59 |
| Motion | An animation crate with four 3D samplers. They are a donut, a black hole, a gyroscope, and orbit rings. It also holds a 3-by-3 shape character and an HSV conversion. | `jcode-tui-anim/src/lib.rs` line 52 to 593 |
| Palette | A harmony module of 40 KB and a palette of 37 KB, so the palette is generated rather than listed. | `jcode-tui-style/src/harmony.rs`, `palette.rs` |

**Take:** the named-role theme, and the separate header roles. A role list is testable, and
a colour list is taste.
**Refuse:** the 3D animation set. A spinning donut is a demo, not a working state, and it
costs a frame budget that rho spends on the transcript. One tasteful motion beats four.

## Makit

Makit is a Flutter app, so nothing here translates literally. Two things translate exactly.

### The duration ladder

`app/lib/ui/session/elapsed.dart` formats a finished span. The ladder is stated in the doc
comment, and `app/test/elapsed_test.dart` pins every rung.

| Span | Output |
| --- | --- |
| under 9.95 seconds | one decimal, trailing `.0` stripped: `2.4s`, `9.1s`, `2s` |
| 10 to 59 seconds | whole seconds, rounded: `13s`, `59s` |
| 1 to 59 minutes | zero-padded seconds: `2m 41s`, `18m 04s` |
| 1 to 23 hours | zero-padded minutes: `4h 12m`, `1h 00m` |
| a day or more | `3d 4h` |

**The rule that matters is not the ladder. It is where the rounding happens.** The function
rounds exactly once, at the top, then uses integer arithmetic for every tier below. Rounding
inside a tier lets a carry escape it, and the comment records what that produced: `59.5s`
became `60s`, `119.7s` became `1m 60s`, and `3599.7s` became `59m 60s`. That bug shipped in
the feature's own design mockup and a review caught it. Every documented rung passes with
the bug in place, which is why Makit pins the carry cases as required tests.

rho copies the ladder **and the carry tests**. A test suite that only walks the documented
ladder proves nothing.

An unrepresentable span returns nothing rather than a number: a negative span, or a span
with no terminal event. A clock can step backwards, so `end < start` is reachable in a real
log, and neither zero nor an absolute value is honest.

### The concise form

Sources: `DESIGN.md`, and the tool call card and tool summary widgets.

- A tool row is collapsed by default. It holds a verb, a compact payload, the duration, a
  status glyph, and a caret. The payload drops a shell prologue and truncates a path to its
  last few segments.
- The caret rotates to signal the expansion. Expanded, the same header stays and the body
  shows the verbatim output.
- A running row counts up live, and it turns amber once it passes about a minute.
- The duration sits in a fixed-width slot, so a live tick never reflows the summary beside
  it.

That last point is a real design constraint, and it is cheap to miss: a duration that grows
from `9.1s` to `13s` to `2m 41s` will shift every character after it unless the column is
reserved.

### What a terminal must drop

A type scale, fractional leading, a corner radius, and translucency. Makit's glass surfaces
carry a principle, which is that the content floats and the frame recedes. In a terminal
that principle survives as whitespace and one thin rule, not as a shader.

## The three ideas rho takes, and the three it refuses

**Takes.**

1. The paste placeholder with a repeat suffix, from codex. It is the exact feature asked
   for, and codex already found the awkward case.
2. The duration ladder with its carry tests, from Makit. It arrives with a proven defect
   class attached.
3. The named-role theme, from jcode. A role list is reviewable and testable. A palette is
   taste, and taste does not survive a critic.

**Refuses.**

1. A clock read inside a render, from codex. It makes a frame untestable, and rho's spec
   forbids it.
2. A 3D animation set, from jcode. It spends a frame budget on a demo.
3. A per-character allocation for motion, from codex. rho measures its frame cost, so it
   cannot afford one.

## What rho can claim that none of them can

Nothing yet. Every number in this section must come from `bench/` and from
`docs/benchmarks.md`, and stage U5 produces them. A claim written before the measurement is
a slogan, and the prose rules delete it.
