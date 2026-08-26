# D-the-wire-reuses-the-core-stop-reason — no parallel wire enum

Date: 20260826. Reference: `D-the-wire-reuses-the-core-stop-reason`.
Spec: `docs/specs/20260819-102749-SPEC-jsonl-frontend.md`, section 3.3.

## The question

The draft spec declared `AgentStopReasonWire` and `ToolKindWire`. Each one copied a
`rho-core` enum. Should `rho-jsonl` keep its own copy of a core enum?

## What the code showed

The copies were already wrong. The draft's `AgentStopReasonWire` held five variants.
`rho_core::AgentStopReason` holds six. The missing one is `MaxToolCalls`, added when the
tool-call budget landed. The draft's `ToolKindWire` held four variants named `Read`,
`Write`, `Execute`, and `Ask`. `rho_core::ToolKind` holds ten, and none is called `Write`
or `Ask`.

So the copy drifted before either side existed. A copy of an enum is a second source of
truth, and this project already paid for one. See `D-one-redaction-home`.

## The decision

`rho-jsonl` puts `rho_core::AgentStopReason`, `rho_core::ToolKind`, and
`rho_core::StopReason` on the wire directly. Each one already derives `Serialize` with
`rename_all = "snake_case"`, and `AgentStopReason` already carries the
`serde(rename = "cancelled")` that ACP needs. See `D-acp-cancelled-spelling`.

One wire type stays local, and only because core has no variant for it. See
`D-the-frontend-settles-every-prompt`.

## Why not the alternatives

- **Keep a wire copy, and add a test that the two match.** A test over two enums cannot
  see a variant added to one of them, because a new core variant needs a new arm nowhere.
- **Map with a wildcard arm.** A wildcard turns a new core variant into a wrong wire value
  in silence. That is the `ToolKind::Other` defect family. See
  `D-plugin-does-not-classify-itself`.

## What it rules out

- No `AgentStopReasonWire`. No `ToolKindWire`. Neither name appears in the tree.
- No wildcard arm in any map from a core enum to a wire value. Every map is exhaustive, so
  a new core variant is a compile error and never a silent default.
