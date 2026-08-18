# D-approval-option-not-enum — The resolved approval mode is an Option, so an unset value stays unset

**Question (controller, red-stage review):** `Config` typed `approval` as `ApprovalMode`,
and `SPEC-config` said `Config::defaults` leaves the approval unset. Where does an unset value
live, when the type has no unset state?

**Decision:** `Config::approval` is `Option<ApprovalMode>`. `None` means the user stated
no mode, and the frontend resolves the mode from the table in `SPEC-approval` section 4. A
stated mode is `Some`, and it always survives the load.

**Reason:** The type could not express the case the spec described, so `Config::load` had
to invent a value. A test asserted that invented value, `read-only`, and the test passed.
Two faults followed. First, the `ask` default became unreachable: an interactive run with
no config file would resolve `read-only`, and the owner chose `ask` for exactly that case.
Second, the frontend could not tell an explicit `read-only` from an absent value, so it
could not honour the rule that only a user widens a mode. This is the `--read-only: bool`
fault from the T1 review, in a second place. A boolean and a non-optional enum both erase
the difference between a choice and a default.

**Rules out:** A static resolved default for `approval` inside `rho-config`. A frontend
that guesses whether a mode was stated. A test that asserts an invented default as if a
user had chosen it.
