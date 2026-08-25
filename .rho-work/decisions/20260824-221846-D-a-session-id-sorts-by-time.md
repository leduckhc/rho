# D-a-session-id-sorts-by-time — the id is a stamp and four hex characters

**Question:** what shape is a session id, and what is the file name?

## The decision

The id is `<YYYYMMDD-HHMMSS>-<4 hex characters>`. The file is `<id>.jsonl`.

A sort of the directory entries gives newest first. So an ordered list needs no file read
at all.

The stamp matches the artifact rule this project already follows. See `D-slug-ids` and
`docs/ids.md`. One convention covers a spec, a decision, and now a session.

## Rules that hold

- A prefix resolves an id. `--resume 20260825-09` is enough while it is unique.
- An ambiguous prefix is an error that lists every match. It never picks one.
- The four hex characters make two sessions in the same second safe. Two worktrees can
  start together.
- The id also goes inside the header record. See `D-chain-records-are-frozen`.

## Rules out

**A bare uuid.** It does not sort, so every ordered list would need a read or a stat.

**A memorable word pair, as jcode uses.** It is easy to say out loud. It does not sort,
and a word set repeats sooner than a stamp.

**A counter.** A counter clashes between worktrees. That is settled by `D-slug-ids`.

## Cost

One id function. One prefix resolver, with an ambiguity error.
