# Verification: the agentic workflow guard

Date: 2026-08-20. Machine: macOS, Python 3.9.6, PyYAML 6.0.3.

This records what was run for `bench/check-agentic-workflow.py`, and what happened. The
guard is new, so step 7 of `AGENTS.md` applies: break the thing on purpose, and watch the
guard fail. Every file was copied to `/tmp` first. No file was restored with
`git checkout`. See D-jcode-bash-lessons.

## What the guard found on its first run

Three real defects in `agentic-workflow.yaml`, all of one family. A stage bound an input
from a stage that its own lane skips, so the controller would have had to guess.

```
$ python3 bench/check-agentic-workflow.py
VIOLATION: spec-fix.inputs.findings in lane bug-fix binds `plan.review.findings`, but lane bug-fix skips `plan`
VIOLATION: spec-fix.inputs.findings in lane refactor binds `plan.review.findings`, but lane refactor skips `plan`
VIOLATION: land.inputs.defects in lane docs-only binds `drive.defects`, but lane docs-only skips `drive`
VIOLATIONS 3 (stage kinds: 13, lanes: 6)
exit 1
```

Fix: the `bug-fix` lane and the `refactor` lane now skip `spec-fix`, and `land` binds
`recon.decisions_that_apply` in the `docs-only` lane.

## The eight deliberate breaks

Each break was applied with `sed`, checked, then undone by copying `/tmp/awf.good.yaml`
back. Every break tripped the guard.

| Break | What the guard said | Result |
| --- | --- | --- |
| `names: plan.test_namez` | `red.inputs.names in lane feature: names no slot that plan produces` | fail, as wanted |
| `role: qa` on `green`, and `qa` is not a role | `stage kind green names an unknown role qa` | fail, as wanted |
| `gate_sets.full` cut to three commands | four lines, one per missing `AGENTS.md` command | fail, as wanted |
| `dod_authored_by: developer` on `green` | `stage kind green lets its own role author the DoD it is scored against` | fail, as wanted |
| a `lanes:` list added back to `sweep` | `stage kind sweep carries a lanes list` | fail, as wanted |
| `type` removed from `review.produces.findings` | `review.produces.findings has no type` | fail, as wanted |
| `dod_authored_by_when.bug-fix: developer` on `green` | `green.dod_authored_by_when.bug-fix lets its own role author its own DoD` | fail, as wanted |
| a lane override on `red` for `docs-only`, which skips `red` | `stage kind red overrides lane docs-only, which skips it` | fail, as wanted |

Break two is finding W-02 of the proposal. Two stages in the deleted sprint files
dispatched a role that their own file did not hold, and both reached `main`. Break three is
finding W-01. Four files defined the gate, and three were wrong.

After the last restore:

```
$ python3 bench/check-agentic-workflow.py
VIOLATIONS 0 (stage kinds: 13, lanes: 6)
exit 0
```

## The track checks

A probe track was written under `.rho-work/tracks/`, then deleted.

```
$ python3 bench/check-agentic-workflow.py
VIOLATION: .../track.yaml: stage id `T12` is not a slug. See D-slug-ids
VIOLATION: .../track.yaml: stage `T12` has 5 dod items. The cap is 4
VIOLATION: .../track.yaml: stage `acp-frontend-green` uses an unknown kind `invented-kind`
VIOLATIONS 3 (stage kinds: 13, lanes: 6)
exit 1
```

`T12` is the real id of a sprint-2 stage. A counter took the next free number, so `T12`
sat five stages from its own position. The cap of four items comes from a measurement: six
of sixteen dispatches in sprints 1 and 2 hit a turn limit.

## Not covered

- CI does not run this guard. `.github/workflows/` belongs to another writer in this tree.
- The guard reads the template and a track. It does not read a spawned subagent prompt, so
  `spawn.prompt_max_words` and `handover.max_words` stay unenforced.
- The guard checks that a slot has a type. It cannot check that the evidence is true.
