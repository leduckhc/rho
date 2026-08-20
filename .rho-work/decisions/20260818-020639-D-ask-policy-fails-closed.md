# D-ask-policy-fails-closed — The Ask policy asks a frontend over a channel and fails closed


**Question (T1b architect):** what is the public shape of the interactive policy, and
what does it do when no answer arrives?

**Decision:** `AskPolicy` implements the existing `ApprovalPolicy` trait, so it changes
no caller. It holds an `mpsc::Sender<ApprovalRequest>` to the frontend and a `Duration`
timeout. It sends an `ApprovalRequest` for a mutating kind and awaits an
`ApprovalAnswer` on a `oneshot`. A read-only kind is allowed without a question. A
timeout, a closed channel, and a dropped answer sender are each a denial. A denial
reaches the model as a tool error and never ends the run. See `SPEC-approval`.

**Reason:** the trait must not change, because existing callers depend on it. A
channel keeps the policy free of any UI type, so the TUI and the ACP frontend both use
it. Fail-closed matches the project rule that an uncertain boundary denies.

**Rules out:** a change to the `ApprovalPolicy` trait. A UI dependency inside
`rho-core`. A timeout or a closed channel that reads as an allow. A denial that ends
the run.
