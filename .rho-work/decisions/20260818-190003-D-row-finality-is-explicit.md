# D-row-finality-is-explicit — every row kind states when it is final, and the default is never

**Question (controller):** `next_freeze` asks whether a row is final. How does the code
decide, and what happens to a row kind that nobody thought about?

**Decision:** One function answers it, with one arm per `Row` variant and no wildcard arm.
A new variant therefore fails the build, and the author must state its rule.

The rules, and each one names the code that could still change the row:

| Row | Final when | Why |
| --- | --- | --- |
| `User` | always | no reducer path writes it again |
| `Assistant` | a newer `Assistant` row exists, or the turn ended | `last_assistant_mut` reaches only the newest |
| `Thinking` | a newer `Thinking` row exists, or the turn ended | `last_thinking_mut` reaches only the newest |
| `Tool` | the status is `Ok` or `Failed` | `tool_row_mut` finds a pending or running row by id |
| `Agent` | `finished` is true | `agent_row_mut` finds it by id |
| `Task` | `finished` is true | `task_row_mut` finds it by id |
| `Error` | always | nothing updates an error row |

**Reason:** This project already shipped a fail-open enum. `ToolKind::Other` counted as
non-mutating, so a read-only policy approved any tool whose author forgot to declare a
kind. See `D-plugin-does-not-classify-itself`. A `_ => true` arm here would repeat that
mistake with a worse blast radius, because an early freeze is not repairable.

A wildcard that answered `false` would be safe but silent. A row kind would then never
reach the scrollback, and no test would say why. The compiler is the better guard.

**Rules out:** A wildcard arm in the finality function. A `Default` impl that answers
final. A finality rule that reads a clock or a tick. Any rule that depends on the
renderer, because finality is a property of the state.
