# AGENTS.md — rho

Rules for any agent or human who changes this repository.

> **This tree has more than one writer.** A separate agent keeps the repository private
> before release, and it edits `AGENTS.md`, `docs/release-checklist.md`, `docs/index.md`,
> and `.github/workflows/` without committing. So prefer a small anchored edit over a
> section rewrite, and read the diff before you stage. See decision D-shared-working-tree.

## If you read nothing else

1. Spec before code. Contract before either side. Test before logic.
2. **Break your implementation and watch your new test fail.** A test that passes
   against the bug it was written for is worse than no test.
3. Grep for `todo!` before you believe a green suite.
4. Run the thing for real. Tests here passed while the product was unusable.
5. Reconcile the docs with the code, last, every time.

The rest of this page is those five rules with their evidence, plus the lanes for a
change that is not a feature.

## The development flow

Copy this list into your working notes. Tick a box only when its check passes.

Do the steps in order. Every step exists because skipping it cost this project a real
defect. The note under each one says which, and names the record that proves it.

### 0. Classify the change, and pick your lane

Not every change is a feature. Tick the one that fits, then follow its lane.

- [ ] **Feature or new public API.** Every step applies.
- [ ] **Bug fix.** Skip step 3. Start at step 5, and write the test that reproduces the
      bug before you touch the logic. Step 7 is then the whole point.
- [ ] **Refactor, with no behaviour change.** Skip steps 3 and 5. The existing tests are
      the specification, so they must pass **unchanged**. If you must change a test, it
      is not a refactor. Step 13 still applies, because a comment or a spec may name the
      old shape.
- [ ] **Dependency bump.** Skip steps 3 to 7. Run the full gate. Then do step 8's
      fail-open check on any new default, and step 11 against a real service. Confirm
      `Cargo.lock` holds exactly one `rustls`.
- [ ] **Revert.** Say in the commit why the change is going out. Keep the test the
      reverted work added, unless the test is what was wrong. Then step 13.
- [ ] **Docs only.** Step 13, then step 14.

> A dependency bump is not a small case. Defect six in `.rho-work/progress.md` was a
> panicking `rustls-webpki`, pulled in by a feature name that looked safer than the
> modern one. Dependabot found it on the first push.

### 1. Understand and brainstorm

- [ ] State the problem in one sentence, in the user's own terms.
- [ ] Find where the change goes. `docs/features.md` maps every feature to its owning
      crate. Read that row before you pick a file.
- [ ] Read the prior art before you design. `docs/comparison.md` says what we take from
      pi and jcode, and why. Read the real source, not your memory of it.
- [ ] Search `.rho-work/DECISIONS.md` for a decision that already settles this.
- [ ] Ask, when a choice is the user's to make. Ask once, with options and trade-offs.

> Reading jcode's `edit` tool gave rho three features it lacked, and showed one defect to
> avoid. Decision D-jcode-edit-lessons records exactly which, and `docs/comparison.md` repeats it.
> Guessing would have found neither.

### 2. Decide, and write the decision down

- [ ] Add a decision file, `.rho-work/decisions/<yyyymmdd-hhmmss>-D-<slug>.md`, for every
      choice that constrains later work. State the question, the decision, and the reason.
      One decision is one file, so two worktrees never edit the same file.
- [ ] Say what the decision rules **out**, not only what it allows.

> The decisions in `.rho-work/decisions/` stopped later stages re-litigating settled
> questions. A decision with no written reason gets reversed by the next person.

### 3. Spec before any code, and contract before any side

- [ ] Write `docs/specs/<yyyymmdd-hhmmss>-SPEC-<slug>.md`. Take the stamp from
      `date -u +%Y%m%d-%H%M%S`. Never number a spec, because a counter clashes between
      worktrees. See `docs/ids.md`.
- [ ] Put the public API in it **verbatim**, as compilable Rust.
- [ ] Name every test, with the assertion each one proves.
- [ ] Add an `## Out of scope` section. An unbounded spec never finishes.
- [ ] Check the signatures compile. Paste them into a scratch crate outside the repo.

**When the change has more than one side, design the contract first.** A side is any
two places that must agree. A caller and a callee, two crates, a provider, a frontend, a
plugin, a stored file, or a future version of rho. Two sides cannot repair a wrong
contract alone, so we pay for the mistake once per side.

A contract is not only a trait. Treat each of these as a contract, and design it first:

- The **public API**: a trait, a function signature, a builder, a constructor.
- The **data model**: a struct, an enum, a field name, an optional field, a default value.
- The **error taxonomy**: which errors exist, and which side must handle each one.
- The **wire format**: JSON sent to a provider, ACP messages, a stream event.
- The **persisted format**: a session file, a transcript, a cache, a lock file.
  A persisted format also binds the next version of rho, so plan the migration.
- The **configuration**: a config key, a CLI flag, an environment variable, a default.
- The **extension surface**: a tool schema, a plugin hook, a feature flag, a capability.
- The **behaviour rules**: ordering, retries, cancellation, and timeouts.
  Also the invariants each side may trust, such as "the prompt stays append-only".

- [ ] Name the sides. Say which crate owns each one.
- [ ] List which of the contract kinds above the change touches.
- [ ] Write the contract in the spec first: the trait, the types, and the error enum.
      Write it as compilable Rust, before either side starts.
- [ ] Send the contract through review before any side implements it. Use step 9. The
      contract is the cornerstone of the project, so it gets a real review, not a glance.
- [ ] Make the contract open for extension and closed for modification. A new case
      arrives as a new impl or a new variant. It never arrives as an edit to shared code.
- [ ] Name the extension point in the spec. Say what a third party adds without a fork.
- [ ] Keep the contract small. Every method is a promise that every side must keep.
- [ ] Write down what the contract forbids, and give each error case a name.
- [ ] Say what an old reader does with a new field. A silent drop is a defect.
- [ ] Change a frozen contract only in the spec first. Then tell every side.

> A tester cannot write a test from a vague spec. A named test with its assertion is
> the handover, and it is what made parallel work possible here.
>
> A contract that grows one field per caller stops being a contract. A four-argument
> `Session::new` hid a fake model id, a stray session root, and an approve-all policy.
> Every caller carried the mistake. See decision D-no-four-argument-session-new.
> A data model is a contract too. `ToolKind::Other` was a fail-open enum variant, and a
> read-only policy approved any tool that forgot its kind. A wire format is a contract
> too, and three providers rejected our requests in sprint 1. No fixture caught that.
> We want code that a maintainer can extend without reading all of it. That comes from
> a small, reviewed, stable contract, plus new impls behind it.

### 4. Put the code in the right place

Most changes need no new crate. A new built-in tool goes in `rho-tools` and implements
`rho_core::Tool`. A new provider gets its own crate, because a provider is optional.

- [ ] Decide whether this needs a new crate. It does only when a user should be able to
      leave it out of the build.
- [ ] For a new crate, run `cargo new`. Never hand-write a manifest.
- [ ] Add a dependency with `cargo add`. Never invent a version.
- [ ] Keep `rho-core` free of HTTP and terminal dependencies.

> A hand-written version pins a release that may not exist. A needless crate is not free
> either. It adds a manifest, a feature flag, and one more boundary to keep straight.

### 5. Red: write the failing test first

- [ ] Write the test before the logic.
- [ ] Run it. Watch it fail.
- [ ] Confirm it fails for the **right reason**, which is an unimplemented body, never a
      type error or a missing import.
- [ ] Use no `sleep` in an async test. Synchronise with a channel, `Notify`, or
      `tokio::time` pause and advance.
- [ ] Use no network. Use a stub binary, `wiremock`, or a recorded fixture.
- [ ] Isolate the filesystem with `tempfile`. A test must never read the real
      `~/.rho` or `~/.agents`, because its result would change per machine.

### 6. Green: make it pass

- [ ] Write only the code the failing test needs. Add no branch, field, or parameter
      that no test reaches, because step 8 will make you delete it.
- [ ] Do not edit a test to fit the implementation. If the test is wrong, stop and say
      so. A test that forces a defect into the public API is grounds to change it, and
      that decision belongs to the reviewer, not to you.

> A four-argument `Session::new` survived because a test shape demanded it. It hid a
> fake model id, an accidental session root, and a policy that approved every tool call.
> See decision D-no-four-argument-session-new.

### 7. Prove the test catches the bug

- [ ] Copy the file first, for example to `/tmp`. **Never restore it with git**, because
      `git checkout` throws away every uncommitted change in that file.
- [ ] Break the implementation on purpose.
- [ ] Run the test. **Watch it fail.**
- [ ] Copy the good file back. Watch it pass.
- [ ] Record the before-and-after in your report.

> This step is not optional, and it is the one most often skipped. A memory-cap test
> here passed against the very bug it was written for, because it asserted the size of
> the kept output while the read buffer still grew without limit. **A test that passes
> against broken code is worse than no test**, because it buys false confidence. See
> decision D-bash-line-cap. The controller once used `git checkout` to undo a deliberate break and
> destroyed the change it had just written. See decision D-jcode-bash-lessons.

### 8. Check the whole surface, not the diff

- [ ] List every public item your change adds. Confirm a test touches each one.
- [ ] `grep -rn 'todo!\|unimplemented!' crates/*/src` must find nothing.
- [ ] Look for a fail-open default: an `Other` or `Unknown` enum variant, a trait method
      with a default body, a value that crosses a process boundary, or a `Default` impl
      that picks a security-relevant value.

> Three defects here hid in untested public surface, and a green suite proved nothing
> about any of them. `confine`, the path boundary, was left `todo!()` through a stage
> that reported green. `ToolKind::Other` counted as non-mutating, so a read-only policy
> approved any tool whose author forgot to declare a kind. See decisions D-todo-in-a-green-stage and D-plugin-does-not-classify-itself.

### 9. Review

- [ ] Get a second pass that did not write the code. A subagent, another model, or a
      human all count. Re-reading your own diff does not, because you will read what you
      meant to write.
- [ ] Tell the reviewer the defect history, and ask it to assume another defect of the
      same family exists.
- [ ] Ask explicitly for **the list of public items with no test**. That list is where
      the bugs are.
- [ ] Review a contract on its own, before either side exists. Ask the reviewer one
      question: does a new case need an edit to shared code? If yes, the contract is wrong.
- [ ] Get a security review for anything that runs a command, reads a path, holds a
      credential, or trusts another process.
- [ ] Treat a severity rating as a hypothesis. Test it.

> A security audit rated credential inheritance as minor. A thirty-second live probe
> showed a prompt-injected model reading `AWS_SECRET_ACCESS_KEY`. See decision D-bash-scrubs-credentials.

### 10. Verify it yourself

- [ ] Re-run every gate command. Do not trust a report.
- [ ] Read the diff.
- [ ] Grep for stubs again.

> Two reports here claimed work that was not done. One measured prose at the wrong
> limit. One said no `todo!()` remained while three did. See the false-claims table in
> `.rho-work/progress.md`.

### 11. Drive it for real

- [ ] Run it for real. Build with `cargo build --release -p rho-cli`, then run
      `./target/release/rho run "<prompt>" --provider <name> --model <id>`.
- [ ] Cover the failure path too. At minimum: an absent file, a denied permission, and
      the same thing happening **twice**. "Twice" has caught two defects here.
- [ ] Exercise every provider the change touches. One provider is not every provider.
- [ ] Write the commands and their real output into `docs/verification/`.

> This step found the worst defects in the project. A tool error killed the whole
> session, and 222 tests passed while the product was unusable. Bedrock worked for one
> tool call and returned 400 for two. Azure rejected every tool call. **No fixture could
> catch any of them, because each fixture described a response and each defect was in the
> request.** See `docs/verification/sprint-1.md`.

### 12. Turn each defect into a guard

- [ ] Add a test that pins the invariant, not the example. Assert that a pairing is
      complete, or that a bound holds, rather than that one expected event appeared.
- [ ] Add a CI guard for a defect class that a test cannot see.
- [ ] Break the rule on purpose and confirm the guard trips.

> The CI guards here each come from a shipped defect: exactly one `rustls` version, no
> `todo!()` in crate source, and the prose check.

### 13. Update the docs, and make them match reality

Do this last, and do not skip it. A doc that disagrees with the code is worse than no
doc, because somebody will trust it.

- [ ] Amend the spec when the implementation diverged. The spec is the contract, so it
      follows the code or the code follows it. Never leave them disagreeing.
- [ ] Add the new test names to the spec's `## Test cases` section.
- [ ] Update `docs/features.md`: the status, the owning crate, and the extension point.
- [ ] Update `docs/benchmarks.md` when a number changed. Include the command.
- [ ] Update `docs/verification/` with what you actually ran.
- [ ] Update `README.md` when a user-visible thing changed.
- [ ] Record any decision you made along the way in `.rho-work/DECISIONS.md`.
- [ ] Delete a claim you can no longer prove. An unverified claim is a slogan.
- [ ] Run `python3 bench/check-prose.py $(find docs -name '*.md')`. It must report zero.

> A stale spec re-introduced a constructor that a decision had deleted. A README claimed
> that a third party could prove a provider conforms, and nobody had tried it; trying it
> took ten minutes and found three wrong API guesses. See decision D-provider-extension-verified-outside.

### 14. Ship

- [ ] Run the full gate. See the `## Gate` section below.
- [ ] Write a Conventional Commit. Say what changed, and say **why**, not how.
- [ ] Push, then confirm CI is green. A push is not a ship.

## Product rules

- rho is the harness, unbundled. The core is a library. Frontends and providers
  are thin, optional crates.
- The GitHub repository stays private until the first release. Do not publish the
  website and do not announce the project before that. `docs/release-checklist.md`
  holds the steps that make the repository public.
- Speed and memory are features. Never claim a performance win without a
  measurement and the command that produced it.
- No crate in `crates/` may depend on `rho-tui`, `rho-acp`, or `rho-cli`.
- `rho-core` has no HTTP dependency and no terminal dependency.
- Keep the system prompt short. Do not write an operating manual into it.
- Keep the model prompt append-only. A stable prefix keeps the provider cache
  warm. Never edit an already-sent turn.

## Engineering rules

- TDD. A failing test lands before production logic. Red, green, refactor.
- Never edit a test to make an implementation pass. If the test is wrong, say so.
- SOLID. If you cannot explain why a change respects each of the five
  principles, it probably violates one.
- Contract first. Design the interface before either side writes code. Get the
  contract reviewed, because it is the hardest thing to change later. A contract
  includes the data model, the error set, the wire format, the persisted format,
  the config keys, and the extension points.
- Open for extension, closed for modification. Extend rho with a new impl behind
  an existing trait. Do not make a caller edit shared code to add its case.
- Prefer the smaller interface. A method you do not add is a promise you do not keep.
- Never leave a verified bug unfixed. Fix a confirmed bug even outside the
  current diff. If the fix is unsafe or too large now, say so explicitly.
- No network access in tests. Use `wiremock` or a recorded fixture.
- No secret in a log, including at `trace` level. Redact by construction.
- Create crates with `cargo new`. Add dependencies with `cargo add`. Never
  hand-write a dependency line or invent a version number.

## Gate

All of these must pass before you report work as done.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build -p rho-cli --no-default-features --features minimal
python3 bench/check-ids.py
python3 bench/check-claimed-tests.py
python3 bench/check-flag-names.py
cargo test -p rho-cli --no-default-features --features minimal --no-run
python3 bench/check-prose.py $(find docs -name '*.md')
python3 bench/check-agentic-workflow.py
python3 bench/check-spec-tests.py
```

`bench/check-ids.py` proves that every spec, ADR, decision, and feature reference resolves,
and that no numeric id came back. See `docs/ids.md`.

`bench/check-claimed-tests.py` proves that every test a commit message names exists in the
tree. A green suite cannot see a missing test, and on this branch three tests were lost to a
concurrent writer while the suite stayed green and a commit claimed one of them by name. A test
that exists only in a commit message is a false claim about the work.

Record a deliberate removal in `bench/deleted-tests.txt`, with the commit and the reason. A
commit that deletes a test names it too, and the guard cannot tell that from a loss.
`bench/check-spec-tests.py` proves that every test a delivered spec names really exists. A
spec that names fifteen tests nobody wrote passes every other gate. A draft, a planned, and a
superseded spec are exempt, and only the first word of the `Status:` line decides that.

`bench/check-agentic-workflow.py` proves that `agentic-workflow.yaml` still holds together:
every stage input resolves, every role exists, no agent authors its own definition of done,
and this gate list matches `gate_sets.full`. See D-agentic-workflow-is-a-template.

## Prose rules

Write prose in ASD-STE100 Simplified Technical English.

- Active voice. Simple tenses.
- One instruction per sentence.
- Sentences of 20 words or fewer.
- One word per meaning. No idioms.
- Avoid unnecessary jargon. Restate complex ideas in plain human language.
- Speak coherently and concisely, like one human talking to another.
- Use small words, short sentences, and short paragraphs.
- Explain any needed big word right after you use it.
- Return only what the reader needs.
- For updates, say what changed, whether it worked, and what to do next.
- Applies to docs, comments, error messages, UI copy, and commit messages.
- Code identifiers, commands, and paths stay verbatim.

Commit messages follow Conventional Commits.

## Where things live

See the layout table in [docs/index.md](docs/index.md).
