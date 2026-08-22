# A second writer can delete a test in silence, so a commit's claims are checked

Date: 20260822. Reference: `D-a-second-writer-can-delete-a-test-in-silence`.

## What happened

A review agent was spawned with worktree isolation so that it could mutate code safely. The
isolation did not take effect: `git worktree list` showed no worktree for it. So the agent
mutated files in the live checkout and restored them from a snapshot it had taken when it
started, which was older than the controller's edits.

It happened twice.

1. It reverted `crates/rho-core/src/session/mod.rs` mid-edit. A build failed at a line whose
   content had already changed, which is how it was noticed.
2. It restored the same file over three new tests. **Nothing failed.** The suite stayed green,
   because a missing test fails nothing. A commit then claimed one of them,
   `a_counter_stops_at_its_limit`, by name.

The second loss was found only because a later mutation "survived" a guard those tests were
written to hold, and the survival made no sense.

## The decision

Three rules, in order of how much they cost.

1. **A commit message's test names are checked against the tree.**
   `bench/check-claimed-tests.py` reads the commit messages of a range and asserts that
   `fn <name>` exists under `crates/`. It runs in the gate and in CI. It was proved by running
   it against the loss, where it printed the missing name.
2. **An agent that edits gets a copy, not a flag.** A reviewer that mutates code is given its
   own clone or an explicit path outside the tree. The isolation flag is not evidence, and
   `git worktree list` is.
3. **After a concurrent writer is detected, the commit is read, not trusted.** A green suite is
   not a statement about what the commit contains. `git show --stat` and a grep for the new
   test names are.

## Why the first rule is the important one

The other two depend on noticing. The first does not: it turns "I wrote a test" into a claim the
build can refuse. This project already has that shape twice, in `check-ids.py` for a reference
that resolves and in `check-spec-tests.py` for a spec that names a test. This is the same idea
pointed at a commit message.

## What it rules out

- No commit may name a test that does not exist, whatever the reason it went missing.
- No mutating agent runs in the controller's checkout again.
- No "the suite is green" as evidence that a change is complete.

## The cost

The guard reads commit messages, so it needs history, and CI checks out with
`fetch-depth: 0` for that job. It also means a commit message must spell a test name exactly,
which is a small discipline with an obvious payoff.
