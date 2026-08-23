# The house style for rho user documentation

Internal note. It is the contract every writer of `docs/guide/` follows, human or agent.

## Who the reader is

A developer who wants to use rho today. Not a contributor. Not a reviewer. They have a
terminal open. They want the shortest path to a working command.

They have never read a spec. They do not know what a crate is called. They do not care
which sprint shipped a feature.

## The nine rules

1. **Write to "you".** "You pass `--read-only`." Never "the user passes" and never "we".
2. **Lead with the command.** A page that teaches a task opens with the command that does
   it. Explain after.
3. **One instruction per sentence.** Twenty words or fewer. `bench/check-prose.py` enforces
   this on every file under `docs/`.
4. **No internal names.** Never cite a spec id, a decision id, or a feature id. Never name a
   crate unless the reader types it, as in `cargo build -p rho-cli`. A reader who meets
   `SPEC-config` section 2 has met our filing system, not an answer.
5. **Say what happens when it fails.** Every task page states the error text or the
   behaviour for the common failure. A doc that covers only the happy path is half a doc.
6. **Show real output.** Paste output you ran. Never invent a transcript.
7. **No selling.** Cut "simply", "just", "powerful", "blazingly fast", "seamlessly", and
   "under the hood". Cut any sentence that would survive unchanged in a competitor's page.
8. **Tables for reference, prose for tasks.** A flag list is a table. A first run is prose.
9. **Say the version the page describes.** rho is `0.1.0` and moves fast.

## How to mark something that is not finished

This is the rule that matters most here. rho has real gaps. A reader who finds one alone
loses trust in every other page.

Use one of exactly three markers. Put it where the reader would first expect the feature,
never in a footnote.

```markdown
> **Not built yet.** `/sessions` is in the command list, and it does nothing. rho answers
> `sessions is not built yet.` Nothing records a session file today.
```

```markdown
> **Partly built.** rho reads the `[subagents]` table in a config file, and no code applies
> it. Pass the limits as flags instead.
```

```markdown
> **Not verified.** Azure OpenAI has unit tests and no live run behind it. Treat it as
> untested.
```

Rules for a marker:

- Name what happens **today** if the reader tries it. A clear error, a silent no-op, or a
  refusal. Silence is the worst case, so always say when it is silence.
- Give the workaround in the same block when one exists.
- Never write "coming soon" and never promise a date.
- Never mark something unfinished that works. Check the code first.

## Page shape

```markdown
# Title in sentence case

One sentence that says what this page gives the reader.

## First heading, usually a task

The command, then the explanation.
```

- No table of contents. The file is short enough to scroll.
- End a reference page with a "What does not work yet" section when it has gaps.
- Link with a relative path, as in `[permissions](permissions.md)`.

## What the gate checks

Run both before you report a page done.

```sh
python3 bench/check-prose.py docs/guide/<page>.md
python3 bench/check-ids.py
```

`check-prose.py` must report `VIOLATIONS 0`. It counts a sentence over 20 words, a passive
voice construction, and a banned word.
