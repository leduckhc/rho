# D-no-list-cache-until-a-budget-fails — measure the list before you cache it

**Question:** a picker row wants a title, a time, a turn count, tokens, a cost, a model,
and a branch count. Only `cwd` and the version sit in the header. The file is append-only,
so no total can be written back. Where does a row come from?

## The decision

The wiring lane ships **no cache**. A list scans the session files it needs.

The spec states a budget: a list of 500 sessions completes under 100 milliseconds, on the
owner's machine, with the command written down. The measurement decides the next step, not
an assumption.

If the budget fails, the answer is one rewritable cache file per session:

```
<session-id>.jsonl        the truth, append-only
<session-id>.meta.json    a cache, rewritten, never authority
```

## Rules that hold

- The transcript is the only truth. A cache holds nothing the transcript lacks.
- A missing cache costs one scan. A corrupt cache is deleted and rebuilt.
- A cache is never read to rebuild a conversation.
- No number reaches a document without the command that measured it.

## Rules out

**A single index file per project, as codex keeps.** The owner runs many sessions at once.
One shared index means many writers on one file, so it invites a corrupt index and a lock.
A per-session file has one writer, always.

**Shipping the cache first.** It is a second format that can go stale, and no measurement
says it is needed yet. This project has a defect class for surface nobody reads. See
`D-dead-surface-is-a-defect-class`.

## Cost

Zero for now. The budget test is the work.
