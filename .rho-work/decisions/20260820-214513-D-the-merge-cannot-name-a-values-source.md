# D-the-merge-cannot-name-a-values-source — the merge loses which layer held a bad value

Date: 20260820

Blocks part of `SPEC-config-call-site`. Found while wiring layer 6.

## The question

`SPEC-config-call-site` deletes `resolve_reasoning` from `rho-cli` and makes every reader
take `&Config`. The reasoning display mode then parses inside `Config::load`.

That loses a message. `parse_reasoning` reports:

```
the tui-reasoning key value "loud" is not valid: ...
```

with the path `the merged configuration`. It cannot say **which** source held `loud`,
because `merge` keeps the winning value and drops where it came from.

The committed test `an_unknown_reasoning_mode_is_refused` asserts the message names
`--reasoning`. A sibling test asserts the environment case names `RHO_TUI_REASONING`. Both
came from `D-a-bad-reasoning-mode-is-refused`, which is the owner's S3 ruling that a wrong
value is an error at every source.

So the spec and the committed tests disagree. Routing reasoning through the merge makes a
refusal say "the merged configuration" where it used to say "the --reasoning flag is wrong".

## The decision

**Not decided. The work stops here and the owner picks.** No code landed for it.

The three candidates, with the cost of each:

1. **Validate at the source, before the merge.** `flag_layer` refuses a bad `--reasoning`
   and names the flag. `ConfigLayer::from_env` already refuses and names the variable.
   `Config::load` keeps its own check for a file value. Keeps every message, keeps one
   precedence, and costs one validation call per source.
2. **Carry the source in the layer.** `ConfigLayer` gains provenance per key, so the merge
   can name the origin of any bad value. This is the general fix, and it is a data-model
   change to a contract that is already reviewed.
3. **Accept the weaker message.** The refusal names the key and the value, never the source.
   This needs the owner's ruling, because it edits a committed test to fit an implementation,
   which `AGENTS.md` step 6 forbids without one.

## The reason it is recorded rather than solved

The wrong move was available and cheap: change the two committed tests to expect the merged
message. `AGENTS.md` step 6 says not to edit a test to fit the implementation, and to stop
and say so instead. The tests are not wrong here. A refusal that cannot name its source is a
worse refusal, and naming the source was the point of the ruling that created them.

Candidate 1 looks right and small. It is still the owner's call, because it puts a validation
in `rho-cli` that `SPEC-config-call-site` currently assigns to `rho-config`.

## What this rules out

- **No weakening of a committed refusal in silence.** The message names its source until a
  ruling says otherwise.
- **No deletion of `resolve_reasoning` yet.** It holds the flag and variable messages that
  the merge cannot yet produce.
