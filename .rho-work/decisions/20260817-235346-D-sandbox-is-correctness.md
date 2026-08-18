# D-sandbox-is-correctness — The bash sandbox is a correctness win, not a structural lead


The controller shipped the `SPEC-bash-sandbox` sandbox and described it as genuinely beating both
competitors. The architect review pushed back, and the correction is right.

A namespace or a sandbox profile is **available to anybody**. Any harness could add one, and
some will. So shipping it is a lead in correctness and in honesty, not in architecture.

rho does have one edge here, and it is narrow: many sessions in one process can amortise a
single sandbox supervisor rather than paying per process.

**The structural lead is density, not confinement.** `SPEC-bash-sandbox` and `docs/extending.md` now
say so, so the claim does not drift back.
