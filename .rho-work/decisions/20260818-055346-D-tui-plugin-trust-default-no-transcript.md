# D-tui-plugin-trust-default-no-transcript — A terminal view reads nothing until the user opts in


**Question.** A terminal view is untrusted code linked into the binary. What may it read,
and who decides?

**Decision.** A view reads nothing by default. `Grants::none` is the default, so
`RenderInput::transcript` is empty. The user opts in with `Grants::read_transcript`, which
rho sets from the user config. A view cannot set its own grant. The same principle as
`SPEC-mcp` section 6 applies: a peer does not classify itself.

Even with the grant, a view sees only a projection of user, assistant, and thinking text.
A tool result, an approval prompt, and a credential-shaped value never enter the
projection. A credential never reaches a view under any grant, because redaction happens
before the projection, at the one home D-one-redaction-home defines.

A view cannot send input and cannot approve a tool call. `RenderInput` is data only, with
no session handle, no sender, and no approval channel. Approving is impossible, not
discouraged, because the surface has no type through which to express it.

**Reason.** A boundary fails closed. A view arrives from outside, so it gets no access
until a human grants it, exactly as a Tier-1 loader treats what it loads. A default that
exposed the transcript would leak a conversation to any linked view. A surface that could
approve a call would let a view act as the user.

**What it rules out.**

- A default that reads the transcript. The user must opt in.
- A view that reads a tool result, an approval prompt, or a credential.
- A view that grants itself more access than the user gave.
- A view that sends a prompt, steers a turn, or approves a tool call.
