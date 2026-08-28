# D-a-dialog-timeout-cancels — the default answer denies, it does not pick

Date: 20260826. Reference: `D-a-dialog-timeout-cancels`.
Spec: `docs/specs/20260819-102749-SPEC-jsonl-frontend.md`, section 3.4.

## The question

The draft spec said a dialog timeout "resolves with a default and continues". It did not say
what the default is. `docs/features.md` says the same thing for the ACP row. What is the
default answer?

## The decision

A timeout resolves as `DialogAnswer::Cancelled`, for every blocking dialog method. One rule
covers `select`, `confirm`, and `input`. `notify` blocks nothing, so it has no timeout.

`Cancelled` reads as a denial everywhere it is consumed. The approval policy in
`rho-jsonl` maps it to `ApprovalDecision::Deny`.

## Why this and not a value

The alternatives all fail open:

- **The first option of a `select`.** The first option is chosen by whoever wrote the
  dialog, not by the user. A prompt-injected model could put the dangerous option first.
- **`true` for a `confirm`.** A silent yes to a question nobody read. This is exactly the
  `ToolKind::Other` shape: a default that a careless author gets for free, and that grants
  rather than denies. See `D-plugin-does-not-classify-itself`.
- **An empty string for an `input`.** Sometimes harmless, sometimes a wiped value. It
  depends on the caller, so it is not a rule.

A denial is never a breach. It is at worst an annoyance, and the client can ask again.
`D-ask-policy-fails-closed` already set this direction for the approval gate.

## What the client sees

Nothing new. The client receives one `dialog` event and no second event for that `id`. It
may still send a late `DialogResponse`, and `rho-jsonl` drops it, because the dialog id is
already resolved. The drop is not silent to a reader of the code, and it is invisible on the
wire on purpose: a second reply for a resolved dialog would race with the run.

## What it rules out

- No timeout default that grants a permission.
- No per-method default. One rule, so a reader does not have to look it up.
- No client-side timeout. The agent side owns every timeout, so the two sides cannot
  disagree about whether a dialog is still open.
