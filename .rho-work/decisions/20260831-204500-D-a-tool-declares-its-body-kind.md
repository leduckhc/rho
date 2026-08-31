# D-a-tool-declares-its-body-kind — the producer says what the body is

Date: 20260831. Reference: `D-a-tool-declares-its-body-kind`.
Spec: `docs/specs/20260831-201549-SPEC-the-tool-row-has-three-levels.md`.
Supersedes the classification rule in `D-diff-colour-by-line-kind-not-tool-name`, and keeps
that decision's goal.

## The question

The tool row wants a coloured diff for the `edit` tool. How does the renderer know that a
body is a diff?

The first answer read the text. `classify_body` marked a line starting with `---` or `+++` as
diff meta, and coloured the body from there.

## Why that answer is wrong

A contract review found the hole, and it is not a corner case:

- A Markdown file starts a front matter block with `---`. A `read` of one becomes a diff.
- A `cat` of a `.patch` file becomes a diff.
- `bash git diff` becomes a diff, in a row that is not an edit.

So the rule colours by content, across every tool. The mistake looks like a rendering quirk,
and nobody can reproduce it on demand.

The obvious repair is worse. Restricting the colour to the `edit` row makes the renderer match
on a tool name, and AGENTS.md calls that a wrong contract: a new tool would then need an edit
to shared code.

## The decision

**A tool declares the kind of its body. The renderer never guesses.**

```rust
/// What a tool row's body holds. The tool that produced the body sets it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    /// Ordinary text. Every line draws in the text role.
    Plain,
    /// A unified diff. The renderer colours each line by its mark.
    UnifiedDiff,
}
```

`Row::Tool` carries a `body_kind: BodyKind`. The renderer colours a line by its mark **only**
inside a body declared `UnifiedDiff`. A `Plain` body draws plain, whatever its text looks like.

## Why this keeps the contract open

The renderer matches on `BodyKind`, never on a tool name. A new tool that emits a unified diff
sets `BodyKind::UnifiedDiff` and gets colour with no renderer edit. A new tool that emits
prose sets `Plain` and cannot be mis-coloured. So the goal of
`D-diff-colour-by-line-kind-not-tool-name` survives, and the content heuristic does not.

The default is `Plain`. That is the fail-safe direction: an undeclared body draws plain rather
than borrowing a meaning it did not earn.

## What it rules out

- No classification of an undeclared body. The renderer never sniffs text to find a diff.
- No `BodyKind::Other` or `Unknown` variant. This project shipped `ToolKind::Other` counting
  as non-mutating, and a read-only policy then approved any tool that forgot its kind.
- No renderer branch on a tool name, now or later.

## Test cases

- `a_read_of_markdown_frontmatter_is_not_a_diff` — a `read` row whose body opens with `---`
  draws in the text role, with no added or removed colour.
- `a_declared_diff_colours_its_marks` — an `edit` row declared `UnifiedDiff` colours an added
  and a removed line, and the marks stay in the text.
- `an_undeclared_body_draws_plain` — the default is `Plain`.
