# D-duration-rounds-once — the duration ladder rounds exactly once

**Question.** Where does the duration formatter round a fractional span to a whole unit?

**Decision.** Round exactly once, at the top of `format_duration`. Then use integer
arithmetic for every tier below. Never round inside a tier.

**Reason.** Rounding inside a tier lets a carry escape it. Makit's own mockup shipped
that defect. It turned `59.5s` into `60s`, `119.7s` into `1m 60s`, and `3599.7s` into
`59m 60s`. Every documented rung passed with the defect in place. So the carry cases are
required tests, not optional ones. `SPEC-tui-experience` pins all four.

**It rules out.** It rules out a per-tier round. It rules out a format that reads a
documented rung as proof. A test suite that only walks the ladder proves nothing here.
