# D-context-in-the-footer — The footer states the context left, and money shows only when measured

**Question (owner):** where do the context percentage and the cost appear?

**Decision:** The footer carries the context left, as a percentage, for example
`94% context`. The header keeps the token counts. A money figure appears only when the
provider reports one, and it renders beside the tokens. rho computes no price from a rate
table.

**Reason:** The context left changes a decision the user makes now, so it belongs beside the
activity word. The token counts are history, so they stay in the header.

Two of the three providers report no charge. A number rho invented would read as measured.
This project already forbids an unmeasured performance claim, and a price is the same kind
of claim. See `D-measured-cost-and-cache`.

**Rules out:** A price built from a hard-coded rate table. A cost field that shows `$0.0000`
when the provider reported nothing. A context percentage in the header, which would push the
tokens off a narrow frame.
