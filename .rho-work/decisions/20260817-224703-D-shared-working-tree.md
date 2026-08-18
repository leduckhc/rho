# D-shared-working-tree — Another agent shares this working tree, so read the diff before you commit


**What happened.** A separate agent keeps this repository private before the first
release. It edits files without committing: it added the private rule to `AGENTS.md`, a
row to `docs/index.md`, and `docs/release-checklist.md`, and it flipped the GitHub
visibility to private through the API.

The controller did not notice. It committed those edits inside its own commits, under its
own messages. So `git log` says the controller wrote a rule it did not write. Nothing was
lost, and that was luck: the controller replaces whole sections with a script, and a
different ordering would have destroyed the other agent's work.

The controller also repeated "public, MIT" in several reports, hours after the repository
became private. It read its own memory instead of the current state.

**Decision.** Three rules.

1. **Read the diff before you commit.** `git status` and `git diff` first. A change you
   do not recognise belongs to somebody else. Commit it separately, and say whose it is,
   or leave it alone.
2. **Prefer a targeted edit over a section rewrite.** A script that replaces a whole
   section destroys a concurrent edit inside that section without a conflict. A small
   anchored replacement fails loudly instead.
3. **Re-read the state you are about to assert.** Visibility, a version, a test count,
   and a benchmark all change under you. Check, then claim.

**These files have another owner.** Treat them as shared: `AGENTS.md` product rules,
`docs/release-checklist.md`, `docs/index.md`, and `.github/workflows/`. Read them before
you write them.

**This is decision D-no-git-writes-by-a-subagent with the roles reversed.** There, a subagent ran `git stash` and
displaced a parallel agent's work. Here the controller was the one at risk of clobbering.
The lesson generalises: a shared working tree has no single owner, so no writer may assume
the tree is as it left it.
