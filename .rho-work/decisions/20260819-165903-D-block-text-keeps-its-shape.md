# D-block-text-keeps-its-shape — a newline survives, an escape does not

Date: 20260819

## The question

rho drew every assistant answer as one flowed paragraph. A markdown list, a paragraph
break, and a fenced code block all collapsed into prose. The owner said the line sanitiser
is wrong for a coding agent, and asked to remove it. Should it go?

## The decision

**The line folding goes. The escape filter stays.**

Block text now uses `sanitize_block`, which keeps every newline and drops every escape
sequence. A tab becomes four spaces. Single-line rows keep `sanitize_line`, because a
newline there would break a one-row layout.

`wrap_block` wraps one source line at a time, keeps a blank line between paragraphs, and
keeps the leading indent of each line.

## The reason

The owner was right about the fault and its cost. Measured, with a real model answer:

```text
model sent:  Intro paragraph.\n\n- alpha: first\n- beta: second\n\n```rust\nfn main() {}\n```
rho drew:    Intro paragraph. - alpha: first - beta: second ```rust fn main() {} ``` Done.
```

Mangling a code block is the serious half, because rho is a coding agent.

**The fault was narrower than the whole sanitiser.** `rho_redact::sanitize_text` already
keeps `\n` and `\t`, and it already drops escape sequences. `sanitize_line` is a four line
wrapper whose only extra job is folding a newline and a tab into a space. That wrapper was
the entire defect. So the repair is to stop calling the single-line wrapper on block text.

**Removing the filter was measured, not argued.** With no sanitiser at all, two tests fail,
and they fail because `\x1b[2J` and an OSC 52 clipboard write reach a terminal cell. Model
output and tool output are untrusted: a file's contents, an MCP response, and a prompt
injection all arrive this way. An escape that reaches the terminal can clear the screen,
move the cursor to redraw a fake approval prompt, or write the user's clipboard.

This project has already paid for underrating that family once. A security audit rated
credential inheritance as minor, and a thirty second live probe showed a prompt-injected
model reading `AWS_SECRET_ACCESS_KEY`. See `D-bash-scrubs-credentials`.

**A tab becomes spaces.** A tab has no defined width in a terminal cell, so it breaks the
column arithmetic every row depends on. Spaces keep the indent a code line needs.

## What this rules out

- **No markdown rendering.** rho draws the model's text as the model wrote it. It does not
  bold a `**` or hide a fence. That is a separate decision, and this one does not take it.
- **No syntax highlighting**, for the same reason.
- **No raw pass-through**, at any width or for any row. There is no flag to turn the escape
  filter off, because a flag that disables it would be a flag that hands the terminal to a
  prompt injection.
- **No newline in a single-line row.** A notice, an error headline, a tool header, and a
  banner all still fold, because each owns exactly one row.
- **No tab in a cell.** Expansion happens once, in `sanitize_block`, not in the renderer.
