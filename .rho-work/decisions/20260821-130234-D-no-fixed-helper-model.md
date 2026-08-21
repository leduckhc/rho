# D-no-fixed-helper-model — rho never hard-codes a second model


**Question (controller):** fx sends a second model request for two features. An unresolved
permission call goes to `openai/gpt-5.4` for review. An image that the chosen model cannot
read goes to `google/gemini-2.5-flash`. Neither is configurable. Does rho copy this?

**Decision:** No, not in that shape. rho takes both capabilities and refuses the fixed
model id. Where rho needs a second model, the caller supplies it as a `Provider` and a
model id, exactly as it supplies the main one. A missing helper is a stated refusal, never
a silent fall back to a vendor default.

**Reason:** rho is provider-agnostic and runs against local models. A hard-coded id has
three costs. It sends a private prompt to a vendor the user did not choose. It fails
closed on an air-gapped machine, and the user cannot see why. It bills a second account.
An enterprise user on Bedrock alone cannot reach an OpenAI id at all.

There is a second cost that is easy to miss. An automatic permission reviewer is a
security control. A security control whose model the user cannot audit or replace is not
one the user can trust. rho's `ApprovalPolicy` is already a trait, so a reviewing policy
is one more impl behind it. That is the extension point, and it needs no new mechanism.

**What rho does take:** the idea that a blocked call can be reviewed instead of stalling,
and the idea that a model without native image input can still receive image evidence.

**Rules out:** any constant model id in rho source. A second model request the user did
not configure and cannot see. A vision path that silently sends an image off the machine.
Charging a user for a request rho never announced.
