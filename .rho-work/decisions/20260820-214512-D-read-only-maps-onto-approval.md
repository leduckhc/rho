# D-read-only-maps-onto-approval — `--read-only` writes the approval key, and only when true

Date: 20260820

Answers question U3 of `.rho-work/reasoning-task.md`.

## The question

`--read-only` is a flag. `approval` is a config key. Both choose the same thing, and only
the flag is read today. `SPEC-config-call-site` names no winner, and the task notes recorded
it as U3: "Both exist, and only the flag is read today. One must win, and no ruling names
which."

`ConfigLayer` has no `read_only` field, so the flag has to land somewhere or be dropped.

## The decision

**`--read-only` maps onto `approval = "read-only"` in layer 6.** The flag then beats a
file's `approval` key because layer 6 is the strongest layer, and no special case is needed.

**`--read-only=false` writes nothing.** Only the true case maps.

## The reason

U3 asked for a new rule. It did not need one, because the merge order already answers it.
A flag that writes into layer 6 wins over a file in layer 3 by the rule that already
governs every other key. Inventing a second rule for one flag would be a per-key exception,
and `D-the-config-call-site-lands` already rules those out.

The false case is different, and the argument is about types. `read_only` is one boolean
over a three-valued enum, so `false` has no single target. It says "do not force
read-only", and it does not say whether the user wants `ask` or `allow-all`. Writing either
one would invent an intent the user never stated, and `allow-all` would weaken the run.
So the negated flag writes nothing and lets the file or the default decide.

## What this rules out

- **No `read-only` key in `ConfigLayer`.** Two keys for one setting would need a precedence
  rule between them, which is the defect this avoids.
- **No approval value from a negated flag.** `--read-only=false` never sends `allow-all`,
  because that would be a weakening the user did not ask for.
- **No special case in the merge.** The flag wins by layer order, and `merge` gains no
  branch for it.
