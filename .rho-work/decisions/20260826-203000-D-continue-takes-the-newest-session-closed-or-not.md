# D-continue-takes-the-newest-session-closed-or-not — a clean run must still be resumable

**Question:** `SPEC-session-store-wiring` section 7 says `newest_open` is "the newest session
in this store that holds no `Closed` record", and that it is both "what a crash offers" and
"what `--continue` takes". Is one method right for both?

## The defect, found by driving it for real

No. A headless run that ends on its own writes a `Closed` record. So `newest_open` skips it,
and bare `--continue` answered:

```
rho: no session to continue in root-e3764aec; start one without --continue
```

That happened right after a successful run wrote a session. **The most common case a user
wants, "continue the conversation I just had", could never work.** Every unit test passed,
because every test seeded a session that never closed. Only step 11 found it. See
`docs/verification/session-store-wiring.md` section 6.

## The decision

The two questions are separate, and each gets its own method.

- `SessionStore::newest_open` is the **crash offer**. It returns the newest session with no
  `Closed` record. `F-session-crash-continue` owns it.
- `SessionStore::newest_resumable` is what **`--continue` takes**. It returns the newest
  readable session, closed or not.

Both skip a session another process holds, so `--continue` never picks a live session and two
worktrees never write one file.

A closed file reopens as it already did. `append_to` writes a `Reopened` record, so a reader
never finds `Closed` in the middle of a file with no explanation. That behaviour existed and
had a test; nothing reached it.

## Rules that hold

- A resume of a closed session states the reopen on disk. See `D-reopen-stated-on-disk`.
- An unreadable file is never resumable. It can be deleted, so a user can clean the store.
- A locked session is skipped by both methods.
- The crash offer keeps its own meaning, so `F-session-crash-continue` still says something
  true: it offers a session that really did not close.

## Rules out

**One method for both questions.** That is what shipped in the spec, and it made the feature
unusable for its main case.

**Never writing a `Closed` record.** Then a crash and a clean exit would look the same, and
the crash offer could not exist at all.

**Making `--continue` refuse a closed session and telling the user to pass the id.** A user
would have to run `rho sessions list` after every single run.

## Cost

One more store method, two tests, and one line changed in the command line.
