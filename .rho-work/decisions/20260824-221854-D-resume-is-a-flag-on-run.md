# D-resume-is-a-flag-on-run — one session flag, two spellings, an optional value

**Question:** is a resume a new top-level verb, or a flag on `run`? And are `--continue`
and `--resume` two flags or one?

## The decision

One flag on `run`, with two spellings and an optional value. `--resume` is an **alias** of
`--continue`. The read-only operations get one `sessions` subcommand.

```
rho run "<prompt>"                    # a fresh session
rho run "<prompt>" --continue         # the newest session for this project
rho run "<prompt>" -c                 # the same
rho run "<prompt>" --resume           # the same, by the other spelling
rho run "<prompt>" --continue=<id>    # one named session
rho run "<prompt>" --resume=<id>      # the same
rho sessions list
rho sessions show <id-prefix>
rho sessions delete <id-prefix>
rho sessions fork <id-prefix> --at <record-id>
rho sessions name <id-prefix> "<text>"
--allow-widen                         # valid with the session flag only
```

In the terminal: `/sessions` opens the picker, `/tree` walks the records, and `/fork`
branches at the selected record.

`-c` is the short form, because pi and claude both spell it that way.

## Why one flag, and not two

Two flags mean two meanings that can disagree. The first draft had that, and it needed a
custom rule: "`--continue` with `--resume` is an error."

An alias removes the rule **and** its code. clap already refuses one argument used twice.
So the refusal is free, and it cannot rot.

It also removes a question a user should not have to answer. "Do I continue or resume?" has
no good answer, because both mean the same thing to a person.

## The value needs an equals sign, and that is measured

The prompt is a positional argument. An optional flag value beside a positional argument is
a trap, so the surface was driven against clap 4 before it was written down.

| Command | Result |
| --- | --- |
| `--continue "fix the bug"`, loose | the prompt becomes the flag value, and the run fails |
| `--continue "fix the bug"`, with `require_equals` | the prompt survives, the flag is bare |
| `--resume=<id> "fix the bug"` | both are correct |
| `--resume <id>` with a space | **the id becomes the prompt, and no error appears** |
| `--continue --resume=<id>` | refused, because one argument is used twice |

So the flag sets `require_equals = true`.

The space form stays wrong, and it is wrong in silence. It continues the newest session,
and it sends the id to the model as a question. So a prompt that matches the session id
shape is refused, and the message names `--resume=<id>`.

## Rules that hold

- A bare flag means the newest session for the project key.
- A flag with a value means that session, and a prefix resolves it.
- `--allow-widen` without the session flag is an error. A flag that does nothing teaches
  nothing.
- `--allow-widen` exists because `SessionError::Widen` already names it. The error names a
  flag rho does not have today.
- Every id argument takes a prefix, and an ambiguous prefix lists the matches.
- `/sessions` stops answering that it is not built. See `F-slash-commands`.

## Rules out

**A top-level `rho resume` verb.** A resume needs every flag `run` has, including the
provider, the model, the sandbox, and the approval mode. A second verb would copy them all,
and the copy would drift.

**A `--fork` flag on `run`.** A fork makes a file and does not prompt. It belongs with the
other file operations.

**A bare `--resume` that opens a picker.** That was the first draft. It gave one flag two
meanings, and the meaning depended on the frontend. A headless run cannot show a picker, so
the flag would have behaved differently in each place. The picker is `/sessions` in the
terminal, and `rho sessions list` on the command line.

**Guessing what a space meant.** rho refuses, and it names the correct form.

## Cost

One flag with an alias, one subcommand with five verbs, and three terminal commands. One
guard on a prompt that looks like an id.
