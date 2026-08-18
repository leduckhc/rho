# D-jcode-edit-lessons — What reading jcode's `edit` tool changed


Recorded because `AGENTS.md` step 1 cites this as the reason to read prior art, and a
reviewer could not substantiate the claim. A war story with no record teaches nothing.

The owner asked for `replace_all` on the `edit` tool, and pointed at jcode's
`crates/jcode-app-core/src/tool/edit.rs`. Reading it produced four changes.

**Three features rho lacked.**

1. `replace_all`, plus an ambiguity error that names the escape hatch. Strictness alone
   was the trap: a rename touching three identical spans forced the model to add
   surrounding context three times, or to give up.
2. Near-miss diagnostics. A bare "not found" is a dead end, because the model cannot
   tell an absent span from one that differs in whitespace. A failed match now names the
   near miss it found, with a line number.
3. Familiar argument aliases. Models are trained on `file_path`, `old_string`, and
   `new_string`. rho keeps its own consistent names and accepts those too, because a
   schema error costs a whole turn.

**One bug in jcode, which rho now guards against.** jcode accepts an empty
`old_string`. An empty pattern matches at every character boundary, so with
`replace_all` the file is rewritten: `"abc"` becomes `"XaXbXcX"`. rho refuses the call.

`SPEC-tool-interface` section 6a records the outcome. The lesson for step 1 stands: reading the real
source found three gaps and one defect, and guessing would have found neither.
