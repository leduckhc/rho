# D-agentic-workflow-is-a-template — One template, filled per unit of work

**Question.** Three files at the repository root drove agent work: `workflow.yaml`,
`workflow-sprint-2.yaml`, and `workflow-sprint-3.yaml`. A fourth sprint would have added a
fourth file. Does the project keep one file per sprint, or one template?

**Decision.** One template. `agentic-workflow.yaml` holds it, and the three sprint files
are removed. The controller reads the template and fills a copy per unit of work, under
`.rho-work/tracks/<yyyymmdd-hhmmss>-<slug>/`.

**Why.** The three files were `AGENTS.md` transcribed by hand, once per sprint. Every
transcription error became a defect:

- Four files defined the gate, and three were wrong. `.rho-work/BRIEF.md` named three
  commands, so a subagent that obeyed its brief still failed the real gate.
- Two stages dispatched a role that their own `roles:` map did not hold.
- Sprint 3 dropped `release_policy`, and the rule was still live in `AGENTS.md`.
- Sprint 1 and sprint 3 both dropped the mutation stage. Sprint 1 shipped three `todo!()`
  bodies through a green stage, and one was the path-confinement boundary. See
  D-todo-in-a-green-stage and D-bash-line-cap.

The steps that catch defects are the steps a retyped file drops. So the template fixes
them. The frame phase and the prove phase never vary with the work, and only the build
phase is generated, by the `plan` stage, before any build stage runs.

**The name is deliberate.** "Agentic workflow" means the pipeline agents follow.
`.github/workflows/` holds the CI workflows. The old name `workflow.yaml` meant both, and
that ambiguity forced a clarifying question before any work could start.

**What this rules out.**

- No per-sprint workflow file. Not one, ever again.
- No second definition of the gate. `agentic-workflow.yaml` `gates` is the only one.
- No agent authors the definition of done it is scored against. The architect writes the
  developer's, and the template writes the architect's.
- No unfilled slot, and no slot that takes a default. A missing lane resolves to the
  strictest lane. See D-config-fails-closed.
- No slot without evidence. Evidence is a command with its exit code, or a `file:line`.
- No generated stage that removes or reorders a frame or a prove stage.
- No stage with more than four items. Six of sixteen dispatches in sprints 1 and 2 hit a
  turn limit.
- No track recursion past depth two. An epic's definition of done is the union of its
  tasks', and it adds nothing of its own.
- No stage id from a counter. A stage id is a slug. See D-slug-ids.

**What it does not do.** It is not a runner. No code executes the file. The controller
stays the executor, and the file stays a brief. It says nothing about
`.github/workflows/`.

**The guard.** `bench/check-agentic-workflow.py` enforces the template, and
`AGENTS.md` `## Gate` runs it. It proves that every stage input resolves to a real slot in
the lane that runs it, that every role exists, that no stage lets its own role author its
definition of done, and that `gate_sets.full` matches `AGENTS.md` `## Gate` command for
command. It also checks a filled track for a counter id, an invented stage kind, and a
stage with more than four items.

The guard found three real defects in the template on its first run. Each was the same
family: a stage bound an input from a stage that its own lane skips, so the controller
would have had to guess. Six deliberate breaks were run against it, and each one tripped
it. See `docs/verification/20260820-120345-agentic-workflow-guard.md`.

**Still not enforced.** CI does not run the guard yet, because `.github/workflows/`
belongs to another writer in this tree. Add the step there, and the class closes.

**Records of finished work.** `.rho-work/progress.md` holds what each sprint delivered,
and `docs/verification/` holds what was run for real. Git holds the three removed files.
