# D-diff-colour-by-line-kind-not-tool-name — a diff colours by a classified line kind

## Question

The user wants the `edit` tool to show a colour-coded diff, like git. The tool row today
carries a `preview` string only. Where is the diff computed, and how does the renderer
colour it without a per-tool code branch?

## Decision

Compute the diff in `rho-tools`, where `edit.rs` already builds a `similar` unified diff
and returns it as text in `ToolOutput`. Add no diff code to `rho-core`.

The renderer classifies each sanitised body line into a `LineKind`
(`Context`, `Added`, `Removed`, `Meta`) with a pure function `classify_body` in
`rho-tui`. A line is `Added` or `Removed` only inside a `@@` hunk. The renderer maps a
`LineKind` to a theme role, and never matches on the tool name. Add two theme roles,
`Role::DiffAdd` and `Role::DiffDel`.

The `+` and `-` marks stay in the line text. In the no-colour mode both roles resolve to
`RoleStyle::plain()`, because the mark carries the meaning.

The reducer must store the body. `on_tool_end` in `state.rs` drops `output.content`
today, so the diff never reaches a row. It must classify the output and store it, and
`row_bodies` changes type to `Vec<Vec<BodyLine>>`.

## Reasons

- The unified-diff text must reach the model, so it travels as text anyway.
- `rho-core` must hold no diff library and no terminal concern.
- A `+` or `-` prefix is a mark a colour-blind reader reads. Colour never carries meaning
  alone.
- Classifying by line kind, not by tool name, keeps the contract open for extension. A
  new tool that emits a diff gets colour free, with no renderer edit.

## What this rules out

- A diff computed in the renderer. The renderer classifies, it does not diff.
- A diff-specific field on `Row::Tool`. The body lines carry the diff.
- Syntax highlighting inside a diff line. Only the four kinds get a colour.
- Colour that carries meaning alone. Every diff line keeps its `+`, `-`, or ` ` mark.
- A `match` on the tool name in the renderer to decide diff colouring.
