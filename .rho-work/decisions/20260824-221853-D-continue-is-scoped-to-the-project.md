# D-continue-is-scoped-to-the-project — the newest session, across worktrees

**Question:** `--continue` picks the newest session. Newest where? In this exact
directory, or in this project?

## The decision

In this project. The scope is the project key from `D-session-store-layout`, so every
worktree of one repository shares one pool.

Each row in a list states the directory the session ran in. So the wider scope is visible,
never hidden.

## Rules that hold

- `--continue` prints the id, the title, and the directory before it continues.
- A resume still checks the stored approval and sandbox modes. See `D-resume-never-widens`.
- A continued session that ran in another directory says so in one line.

## Rules out

**Scoping to the exact directory.** The owner works in many worktrees of one repository.
A directory scope hides this morning's work behind a path that changed.

**Continuing in silence.** A user must see which session they got. A wrong guess costs a
whole turn.

## Cost

One newest-first scan, which the sortable file name already gives.
