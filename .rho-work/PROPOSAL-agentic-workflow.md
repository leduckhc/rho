# Proposal: the agentic workflow, filled by the agents that run it

Status: **proposal**. Not a decision. Score it, then I write the decision file and the spec.

**On the name.** This proposal calls it the **agentic workflow** throughout. It is the
pipeline that agents follow to build a feature. It is not a CI pipeline. `.github/workflows/`
holds the CI workflows, and they stay out of scope. Finding W-14 explains why the name had
to change.

This supersedes the earlier draft in this file, which proposed one workflow file per sprint.
That draft was wrong on the central point. It still required somebody to enumerate the
stages before the work was understood. The audit in section 1 stands unchanged.

Scope: the three agent sprint files at the repository root. Nothing in `.github/`.

---

## 1. The audit

I read all 1151 lines of `workflow.yaml`, `workflow-sprint-2.yaml`, and
`workflow-sprint-3.yaml`. I read `.rho-work/BRIEF.md`, `AGENTS.md`,
`.rho-work/progress.md`, and `bench/check-ids.py`. A severity is a hypothesis until a
command proves it, so each finding names the command I ran.

### Blockers

**W-01. Four definitions of the gate. Three are wrong.**

| Source | Gate commands it lists |
| --- | --- |
| `AGENTS.md` `## Gate` | fmt, clippy, test, minimal build, ids |
| `.rho-work/BRIEF.md` §5 | fmt, clippy, test |
| `workflow.yaml` | fmt, clippy, test, workspace build, doc |
| `workflow-sprint-2.yaml` | fmt, clippy, test, minimal build, prose, stubs |
| `workflow-sprint-3.yaml` | fmt, clippy, test, minimal build, prose, ids, stubs |

`BRIEF.md` is the first file every subagent reads. It names three commands. A subagent
that obeys its brief exactly still ships work that fails the real gate.

**W-02. Two stages dispatch a role that does not exist.** `workflow.yaml` stage S9 sets
`role: qa`, and `qa` is not in its `roles:` map. `workflow-sprint-3.yaml` stage U8 sets
`role: documenter`, and sprint 3 deleted `documenter` from its map. Both are in `main`.

### Majors

**W-03. Stage ids are counters, and `D-slug-ids` banned counters.** Sprint 2 stages appear
in this file order: `T0 T1 T1b T2 T3 T4 T5 T12 T13 T6 T7 T8 T9 T10 T11`. `T12` and `T13`
arrived late and took the next free number, so they sit five stages from their position.
`T1b` is the same disease with a letter.

**W-04. Sprint 3 dropped `release_policy`.** The rule is still live in `AGENTS.md`.
`grep -c '^release_policy:' workflow*.yaml` returns 1, 1, 0.

**W-05. Sprint 3 keeps `heartbeat_seconds: 90` with no `stall_signals` and no `actions`.**
The controller polls, and the file never says what a stall is, or what to do. That is dead
configuration that reads as live policy. It is also the most load-bearing rule in the file:
sprint 2 measured a stall or a turn limit in 6 of 16 dispatches, and sprint 3 in 3 of 8.

**W-06. The review gate forked three ways, and the weakest ran first.** Sprint 1's
reviewer pass never asks for the list of public items with no test. Sprint 2 adds it.
Sprint 1 is the sprint where three `todo!()` bodies survived a green stage, and one was the
path-confinement security boundary. See `D-todo-in-a-green-stage`.

**W-07. Nothing parses these files.** A duplicate stage id, a dangling `depends_on`, a
cycle, a missing role, a typo in a gate command, and a deleted invariant are all
undetectable today. W-02, W-04, and W-05 are live proof.

### Minors

**W-08.** The names do not sort in sprint order. `workflow.yaml` sorts last.
**W-09.** About 100 lines are copied per sprint, and a sprint 1 to sprint 2 preamble diff
reports 93 changed lines. The copies are not copies.
**W-10.** `parallel_group` is used once and documented nowhere. Sprint 3 invents `slices:`
for the same job.
**W-11.** Sprint 3's 40-line critic panel is reusable and trapped in one file.
**W-12.** CI runs the prose check over `docs/**` only. These 1151 lines are never checked.
**W-13.** `check-ids.py` resolves slug references, not file paths. So a renamed workflow
file breaks nine references silently.

**W-14. The word "workflow" names two unrelated things in this repository.** `workflow.yaml`
at the root drives agents. `.github/workflows/ci.yml` drives CI. Both are called a workflow,
and neither name says which. A guard named `check-workflow.py` would be read as a CI check
by every reader.

The evidence is this proposal. The first question I had to ask before writing a line was
which of the two families you meant, and both fitted your request exactly. A name that
forces a clarifying question before any work starts is a defect in the name.

Command: `ls .github/workflows/ && ls workflow*.yaml`.

---

## 2. The insight

**The generic agentic workflow already exists. It is `AGENTS.md`, steps 0 to 14.**

Compare the three sprint files stage by stage. I did this mechanically, and the result is
more useful than "they are all the same".

| Stage kind | Sprint 1 | Sprint 2 | Sprint 3 |
| --- | --- | --- | --- |
| classify the lane | absent | absent | absent |
| recon | S1, partly | T0 | U0 |
| design | absent | absent | U1 |
| plan and contract | S2 | T1, T1b | U2 |
| red | S3, S5 | T2, T4, T12, T6 | U3 |
| green | S4, S6, S7, S8 | T3, T5, T13, T7 | U4 |
| measure | S11 | T10 | U5 |
| **prove the test catches the bug** | **absent** | T8 | **absent** |
| **sweep the whole surface** | **absent** | **absent** | **absent** |
| review | every stage | every stage | every stage, plus U7 |
| drive it for real | S9 | T9 | U6 |
| guard, then reconcile the docs | S11, partly | T10, T11 | U8 |

Command: `grep -E '^  - id:|^    name:' workflow*.yaml`.

The backbone is genuinely identical. Plan, red, green, review, drive, and land appear in
all three. So the three files are `AGENTS.md` transcribed by hand, three times, with one
sprint's artifact paths pasted in. Every blocker and every major above is a transcription
error.

The two bold rows matter more than the backbone. **The steps that catch defects are the
steps a sprint drops**, and each drop is traceable:

- Sprint 1 had no mutation stage and no surface sweep. Sprint 1 shipped three `todo!()`
  bodies through a green stage, one of them the path-confinement security boundary, and a
  memory-cap test that passed against the very bug it was written for. See
  `D-todo-in-a-green-stage` and `D-bash-line-cap`.
- Sprint 2 added the mutation stage as T8, and it caught vacuous tests. See
  `D-two-weak-tests`.
- Sprint 3 dropped the mutation stage again. Its critic panel then found a vacuous-test
  family, a silent data loss on large input, and two unreadable colour pairs.

Nobody chose to drop those steps. They were dropped because the pipeline was retyped from
memory each sprint, and the unglamorous steps are the ones that fall out of memory.

`AGENTS.md` step 0 already solves the problem you named. It classifies the change into a
lane, and the lane says which steps apply. A bug fix skips step 3. A refactor skips 3 and
5. A dependency bump skips 3 to 7. You do not need to know what the work looks like. You
need to know its lane, and the lane is a one-line answer.

---

## 3. What the template must handle, and what it must never allow

You asked whether the filling happens by sprint or by feature. The honest answer needs
one more question answered first: **which parts can an agent be trusted to author?**

The evidence says: not the parts it is scored against.

`.rho-work/progress.md` records two false claims. An S1 documenter measured prose at 22
words when the limit is 20. An S4 developer reported "no `todo!()` remains" while three
remained, and one was `confine`, the path-confinement security boundary. A green suite hid
it. `D-no-four-argument-session-new` records a worse case: a test shape demanded a bad
public API, and every caller carried the mistake.

So the template splits into two kinds of slot, and the split is the whole design.

| Slot kind | Who fills it | Example |
| --- | --- | --- |
| **Discovered** | the agent that runs the stage | where the code goes, what prior art says, what it changed |
| **Imposed** | a different role, before the stage runs | the DoD, the gate, the contract, the named tests |

**No agent ever authors the DoD it is scored against.** The architect writes the
developer's DoD. The tester writes the assertion the developer must satisfy. The reviewer
writes findings and nothing else. `gates.yaml` owns the gate, and no agent may add to it.

---

## 4. The proposal

### 4.1 The unit of work is a track, and it recurses once

A **track** is any unit of work with one contract and one red-to-green pair. A feature is
the natural size. A sprint is a schedule, not a unit of work.

Evidence: sprint 2's fourteen stages were really six features, each with its own red and
green pair. Sprint 3's nine stages were one feature in four slices.

So the same template applies at both levels, and it recurses **exactly once**:

```
track kind: epic     # what you call a sprint. its stages are track references.
track kind: task     # a feature, a bug fix, a refactor. its stages are stage kinds.
```

Depth is capped at two. A third level is where a DoD becomes vacuous, and nobody can
prove otherwise. An epic's DoD is the mechanical union of its tasks' DoDs. It may add no
item of its own, so an epic cannot hide work behind a summary.

That answers your question. **Fill it per feature. An epic is a filled template whose
stages are features.**

### 4.2 Three phases. Only the middle one is generated

This is the answer to "we have no clue what it will look like".

```
Phase A — FRAME    fixed. always these three stages.
  classify   controller  -> lane, from the AGENTS.md step 0 table
  recon      scout       -> where the code goes, prior art, decisions that already settle it
  plan       architect   -> the contract, the named tests, AND the phase B stage list

Phase B — BUILD    generated by `plan`. reviewed before any of it runs.
  a repeating red -> green cell, one per contract surface
  optional: design, spec-fix, slice

Phase C — PROVE    fixed. always these five stages.
  mutate     controller  -> break the implementation, watch each new test fail
  sweep      controller  -> list every public item added, grep for a stub, hunt a fail-open default
  review     reviewer    -> findings, plus the list of public items with no test
  drive      controller  -> run it for real, including the failure path, and twice
  land       devops+docs -> a guard per defect, then reconcile every doc
```

Phase A and phase C do not vary with the work, so the template fixes them. Only phase B
depends on the work, so the agent generates exactly the part it can know.

Be precise about why phase C is fixed. It is **not** a generalisation of what the three
sprints did. Section 2 shows two of its stages missing from sprint 1 and sprint 3. Phase C
is the set of steps that `AGENTS.md` requires and that a hand-typed sprint file kept
dropping. Each stage in it traces to a step in `AGENTS.md` and to a defect that reached
`main`:

| Phase C stage | `AGENTS.md` step | Defect that proves it is needed |
| --- | --- | --- |
| mutate | 7 | a memory-cap test passed against its own bug. `D-bash-line-cap` |
| sweep | 8 | three `todo!()` bodies survived green. `D-todo-in-a-green-stage` |
| review | 9 | a security audit rated a live credential leak as minor. `D-bash-scrubs-credentials` |
| drive | 11 | 222 tests passed while the product was unusable. `docs/verification/sprint-1.md` |
| land | 12, 13 | a stale spec re-introduced a deleted constructor. `D-no-four-argument-session-new` |

That is the point of fixing them in the template. **An unknown feature cannot delete its
own safety net, and a tired author cannot forget it.**

`plan` produces the stage list, so nothing needs enumerating up front. `plan` is reviewed
before a single build stage runs, which `AGENTS.md` step 3 already demands for a contract.

### 4.3 Layout

```
.agents/workflow/               # the template. `.agents/` already means agents here.
  pipeline.yaml                 # phases, stage kinds, lanes, the slot schema, the five rules
  policy.yaml                   # monitoring, stall signals, release policy, the ledger
  roles.yaml                    # role -> model, subagent_type, purpose
  gates.yaml                    # every gate command, named once. one source of truth
  gate-kinds.yaml               # reusable review shapes: review-gate, critic-panel
  prompts/<role>.md             # the prompt template per role. slots only
.rho-work/tracks/<ts>-<slug>/   # the instances. working state, one directory per track
  track.yaml                    # the filled template. starts nearly empty. grows per stage
  handovers/<stage>.yaml        # what one agent returned, typed, with evidence
bench/check-agentic-workflow.py # the validator
```

The template sits under `.agents/`, next to `.agents/skills/`. That path already means
"agents" in this repository, so no reader mistakes it for CI. The instances stay in
`.rho-work/`, which already holds the brief and the ledger. Template and working state never
share a directory.

The three root files, `workflow.yaml`, `workflow-sprint-2.yaml`, and
`workflow-sprint-3.yaml`, all go away. That closes W-08 and W-14 together.

Nothing under `.agents/workflow/` is per-sprint. A new sprint is a new directory under
`tracks/`. It edits no shared file. That is `AGENTS.md`'s rule: open for extension, closed
for modification.

### 4.4 The contract

Review this hardest. Every side must agree on it, and it is the hardest thing to change
later. Verbatim YAML follows.

A stage kind in `.agents/workflow/pipeline.yaml`:

```yaml
stage_kinds:
  - id: plan
    phase: frame
    role: architect
    inputs:                       # bound from a named prior handover. not prose.
      facts: recon.where_the_code_goes
      settled: recon.decisions_that_apply
    produces:
      contract:        {type: rust-signatures, evidence: compiles-in-scratch-crate}
      test_names:      {type: list, min: 1, evidence: name-plus-assertion}
      out_of_scope:    {type: prose, required: true}
      build_stages:    {type: stage-list, max_items: 4}
    dod_authored_by: pipeline      # the template imposes it. the architect cannot edit it.
    reviewed_by: [reviewer]
    review_question: >
      Does a new case need an edit to shared code? If yes, the contract is wrong.
    gates: [docs]
    lanes: [feature]               # a bug fix, a refactor, and a dependency bump skip this
```

A filled stage in a track's `track.yaml`:

```yaml
- kind: green
  id: acp-frontend-green           # a slug. never a counter. see D-slug-ids
  role: developer
  depends_on: [acp-frontend-red]
  gates: [rust, minimal-build]
  gate_extra: ["cargo test -p rho-acp --all-features"]
  dod:                             # written by the architect at `plan`. not by the developer.
    - text: Every red test passes, with no test edited.
      evidence: "cargo test -p rho-acp --all-features; exit 0"
```

### 4.5 Five rules the validator enforces, each from a measured defect

**R-1. An unfilled slot is a blocker. It never takes a default.**
A missing lane resolves to the strictest lane, never the loosest. This project shipped
`ToolKind::Other` as a fail-open enum variant, and a read-only policy then approved any
tool that forgot its kind. See `D-plugin-does-not-classify-itself` and
`D-config-fails-closed`.

**R-2. Every filled slot carries evidence.**
Evidence is a command with its exit code, or a `file:line`. A slot with no evidence is not
filled. A report is a claim, and this repo has two false claims on record.

**R-3. No stage may exceed four items.**
`.rho-work/progress.md`: six of sixteen dispatches in sprints 1 and 2 hit a turn limit,
and three of eight in sprint 3. The recorded lesson is exact: "a brief with more than about
four fix items will not finish." So the validator rejects a generated stage with five
items **before** dispatch, instead of after a wasted run. A template rule that comes from a
measurement, not from taste.

**R-4. A skipped stage must name the lane that permits the skip.**
The validator checks the `AGENTS.md` step 0 lane table actually permits it. There is no
free-form "not applicable", because that is the escape hatch every agent reaches for.

**R-5. A generated stage may not remove or reorder a phase A or phase C stage.**
Phase B is the only writable region. This is what stops an agent from planning its way
out of step 7 or step 11.

### 4.6 Prompts that fill and pass

`prompts/<role>.md` has exactly three sections.

```
## Inputs      slots bound by name from prior handovers. the spawner resolves them.
## Do          the steps for this stage kind, from pipeline.yaml. nothing else.
## Return      the exact slot schema to fill, with a type and an evidence field.
```

`BRIEF.md` §7 is already this, in prose. It defines a report with Stage, DoD, Gate
commands, Files, Findings, and Open questions. So the template is not new work. It is
`BRIEF.md` §7 given types and a schema.

Binding is explicit and checked at spawn time, so a missing upstream slot fails before the
agent starts, not twenty minutes in. `BRIEF.md` stays the shared preamble that every role
reads. A per-role prompt holds slots only.

**A prompt template has a size cap.** This repo's rule: keep the prompt short, and do not
write an operating manual into it. jcode's system prompt is about 670 tokens. A template
that balloons defeats its own purpose, so the validator caps it and CI enforces the cap.

### 4.7 What this rules out

A decision must say what it forbids, so here it is.

- No per-sprint agentic workflow file. Not one, ever again.
- No agent authors the DoD it is scored against.
- No unfilled slot, and no defaulted slot.
- No slot without evidence.
- No stage with more than four items.
- No generated stage that touches phase A or phase C.
- No recursion past depth two.
- No prose-only slot. Every slot has a type.
- Not a runner. The controller stays the executor, and the file stays a brief.

---

## 5. Where this can still fail

Naming the failure modes is the only honest way to present this.

**RISK-a. `plan` becomes a single point of failure.** A wrong stage list still looks
authoritative, and every later stage inherits the error. Mitigation: the plan is reviewed
before any build stage runs, and the reviewer answers one fixed question in writing. This
is a real residual risk, not a solved one.

**RISK-b. A generated DoD tends to be vacuous.** "The tests pass" is not a DoD. This repo has
a documented vacuous-test family: see `D-two-weak-tests`, and a memory-cap test that passed
against the very bug it was written for. Mitigation: R-2 forces a command or an assertion
per DoD item. A reviewer that accepts "the tests pass" fails its own stage.

**RISK-c. The template grows into an operating manual.** Then nobody reads it, and we are back
to copied prose. Mitigation: section 4.6's size cap, enforced.

**RISK-d. Depth two still lets an epic hide work.** Mitigation: an epic's DoD is the
mechanical union of its tasks' DoDs, and it may add nothing.

**RISK-e. Migration breaks nine references with no guard.** W-13. Either add a Markdown link
check, or do not rename.

**RISK-f. PyYAML is not guaranteed on a CI runner.** I confirmed PyYAML 6.0.3 on local Python
3.9.6. I did not check the runner image, so do not assume it.

**RISK-g. `AGENTS.md` and `docs/index.md` have another writer.** Make a small anchored edit,
and read the diff before staging. See `D-shared-working-tree`.

---

## 6. Migration plan

Each step ends with the full gate.

1. Write the decision file. Slug: `agentic-workflow`. State what it rules out, and state
   that the name is deliberate. See W-14.
2. Write `.agents/workflow/gates.yaml` and `.agents/workflow/roles.yaml`. Take the strongest
   version of each forked rule, not the newest. This alone closes W-01.
3. Write `.agents/workflow/pipeline.yaml`: lanes from `AGENTS.md` step 0, the eight fixed
   stage kinds, the slot schema, and the five rules in 4.5. Write
   `.agents/workflow/policy.yaml`: monitoring with its stall signals restored, and the
   release policy that sprint 3 dropped. That closes W-04 and W-05.
4. Write `bench/check-agentic-workflow.py`. The name says agentic, so nobody reads it as a
   CI check. Write each check as a failing check first, and break each one on purpose after.
   It is a guard, so steps 5 and 7 of `AGENTS.md` both apply. Record the break and the
   restore in `docs/verification/`.
5. Write `.agents/workflow/prompts/` for scout, architect, tester, developer, reviewer, and
   controller. Six files. Slots only.
6. **Prove it on a real track before converting any history.** Run the next real feature
   through the template end to end. If phase A cannot generate phase B for a live feature,
   this proposal is wrong, and we stop here having spent little.
7. Convert sprint 3 to a track. It holds the critic panel, so it exercises
   `gate-kinds.yaml`. Convert the ids to slugs.
8. Convert sprint 2. Sorting on `depends_on` deletes the `T1b` and `T12` damage.
9. Convert sprint 1. Decide `role: qa` explicitly, and record the reason.
10. Align `AGENTS.md` `## Gate` and `BRIEF.md` §5 with `gate_sets.full`. Anchored edits.
11. Update the nine path references: `README.md`, `docs/index.md`, `docs/features.md`,
    `docs/release-checklist.md`, `docs/tui-prior-art.md`, `docs/tui-design.md`,
    `.rho-work/BRIEF.md`, `.rho-work/progress.md`, and
    `.rho-work/decisions/20260817-175201-D-provider-contract-crate.md`.
12. Add `check-agentic-workflow.py` to the gate and to CI. Add the link guard, or accept
    RISK-e.
13. Rename the term everywhere it is loose. `docs/index.md` and `README.md` must say
    "agentic workflow", so no reader confuses it with CI again.

Step 6 is the important one. It puts the cheapest possible falsification before the
expensive conversion.

---

## 7. Out of scope

- `.github/workflows/`. You scoped it out.
- A runner. No code executes this YAML.
- Any change to a stage's meaning. A converted stage keeps its DoD text verbatim.
- Rewriting `.rho-work/progress.md`.
- A JSON Schema dependency. The validator is Python, like the two guards that exist.
- Recursion past depth two.

---

## 8. Two choices left for you

**C-1. Convert the three finished sprints, or leave them?**
Converting proves the template against real history and makes the validator pass on the
whole tree. Leaving them costs nothing now, and keeps three unparsed files in `main`
forever. A frozen sprint is a record, so converting it edits a record.
My recommendation: convert. Add `status: frozen` and the git sha, and let the validator
check a frozen track for internal consistency only.

**C-2. Where does the lane get decided?**
The controller deciding the lane is one more thing a human must not forget. An agent
deciding its own lane is an agent choosing how many steps to skip.
My recommendation: the controller proposes the lane, and the `recon` stage must confirm or
challenge it with evidence. A challenged lane goes back to the controller. An agent may
argue for a stricter lane freely, and for a looser one never.

---

## 9. Scorecard

Score each row 0 to 5. A row at 2 or lower needs a reason and a change.

| # | Criterion | Weight | Score |
| --- | --- | --- | --- |
| 1 | The audit is correct, and every finding reproduces. | 3 | |
| 2 | Section 2 is right: the generic agentic workflow is already `AGENTS.md`. | 3 | |
| 3 | The discovered-versus-imposed slot split is the correct cut. | 3 | |
| 4 | Phase B is the only generated region, and that is enough. | 3 | |
| 5 | Fixing phase C is justified by the drops in section 2, not by tradition. | 3 | |
| 6 | No agent can author the DoD it is scored against. | 3 | |
| 7 | The five rules in 4.5 each trace to a measured defect. | 3 | |
| 8 | The forbidden list in 4.7 is complete. Nothing fails open. | 3 | |
| 9 | The track recursion answers by-sprint-versus-by-feature honestly. | 2 | |
| 10 | Section 5 names the real failure modes, and hides none. | 2 | |
| 11 | Step 6 falsifies the idea before the expensive work. | 2 | |
| 12 | The prompt template stays small enough to be read. | 2 | |
| 13 | The template is worth its validator. | 2 | |
| 14 | The two choices in section 8 are the real forks. | 1 | |
| 15 | The name "agentic workflow" ends the collision in W-14 for good. | 1 | |

Reject the proposal outright if any of these is true:

- A finding in section 1 does not reproduce.
- A phase C stage cannot be traced to an `AGENTS.md` step and to a defect that reached
  `main`. Then it is ceremony, and I claimed it was a guard.
- Adding a track needs an edit to a shared file.
- An agent can fill a slot that decides whether its own work passed.
- The validator would pass today's three files unchanged. It must fail them.
- The gate still has more than one definition after step 10.
