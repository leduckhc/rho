# D-an-alias-is-bounded-and-printable — the model may name a child, within limits

**Question (step-9 reviewer):** `spawn_agent` gains an `alias` the model sets. The model is
prompt-injectable. What is the worst a hostile alias does?

**Three things, and the draft prevented none.** An unbounded alias is stored per child, which is
the unbounded-growth family this project already paid for once. An alias holding a newline is
echoed into the parent's tool output beside the list of running children, so it can forge a line
that looks like rho's own words. And the draft never said what happens when `set_alias` refuses,
so a fan-out where two tasks ask for one name had no stated behaviour.

**Decision:**

- An alias holds at most `MAX_ALIAS_LENGTH`, which is 64 characters, the same rule as an agent
  name.
- An alias holds no control character and no newline.
- An alias that is only digits is refused, because an id would always win the resolution.
- A refused alias **never fails the spawn**. The child is already admitted, and the work matters
  more than the label. The result carries a note that names the reason, the id, and the derived
  handle.
- In a fan-out, the first task to ask for a name gets it, and the second gets a note.

**Reason:** A label is a convenience. A failed convenience must not destroy admitted work, and it
must not become a channel for text that the parent's model reads as rho's own output.

**Rules out:** An unbounded alias. A newline or a control character in a name that rho echoes. An
alias failure that discards a running child. An alias that shadows a derived handle, which was
already forbidden.
