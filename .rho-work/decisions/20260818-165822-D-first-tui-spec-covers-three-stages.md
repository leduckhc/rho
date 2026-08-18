# D-first-tui-spec-covers-three-stages — Scroll, copy, and the composer ship as one spec

**Question (owner, choosing the scope of the first spec):** the outline stages the work as
S1 to S6. How much does the first spec cover?

**Decision:** Three stages: S1 scrolling and the follow rule, S2 copy and the mouse
decision, and S3 the composer wired to the `Composer` that already exists. The spec is
`SPEC-tui-scroll-copy-composer`. S4 discovery, S5 the turn, and S6 polish each wait for
their own spec.

**Reason:** The three stages share one seam. Scrolling needs a rendered line index. Copy
needs the same index to name the rows it copies. The composer owns the rows below it, so its
height changes the window both other stages measure. Three specs over one seam would
re-litigate the same geometry three times.

The user asked for this scope after reading the recommendation for S1 and S2.

A second reason is honesty about the tree. `Composer`, `route_burst`, and `attach_image` are
built, tested, and reachable by nothing. That is the defect class in
`D-a-panel-nobody-can-open`, and it stays a defect until a key reaches the code.

**Rules out:** A spec that ships scrolling with no copy. Fuzzy palette filtering, `@`
mentions, and `!` shell mode, which belong to S4. Queued messages, folds, and the approval
gate, which belong to S5. A vim mode, per `D-vim-mode-waits`.
