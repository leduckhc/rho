# D-a-named-session-file-is-a-session-like-any-other — one rule, not two

**Question:** the `session-file` config key names one exact file and overrides the store. What
happens when that file already exists?

## The defect

The first version appended to it. No lock, no permission check, and no replay.

Three holes, all in one branch of `crates/rho-cli/src/recording.rs`:

1. **No lock.** The key names one file for every run with that key set. So two `rho run`
   processes appended to one file. Both seeded their record ids from the same read, both minted
   the same ids, and the lines interleaved. That is exactly the race
   `D-a-live-session-holds-a-lock` exists to stop, reached by a different door.
2. **No permission check.** A file written under `read-only` came back under `allow-all` in
   silence. `D-resume-never-widens` says a stored mode may only tighten a run, and this path
   never read the stored mode at all.
3. **No replay.** The run appended new records to a conversation the model had never seen. The
   file then said something the model did not read, which is the shape of
   `D-a-recorder-writes-the-assistant-turn`.

Codex found the first two. The third followed from them: a file that is a session for the lock
and for the permission check is a session for the replay too.

## The decision

**An existing `session-file` is a resume.** It takes the lock, it checks the stored modes, and it
replays the conversation.

A file that does not exist yet is a create, and it takes the lock as well.

So the key changes **where** the session file lives, and nothing else. That is what
`D-session-store-layout` says it is: a store override.

## Rules that hold

- `SessionStore::lock_file` locks any path, and `SessionStore::lock` calls it. One rule has one
  spelling, so a store session and a named file cannot drift.
- `rebuild` holds the shared half of both resume paths: the branch walk, the expiry of a stale
  result handle, and the two warnings. A first version had that code once per path, and the named
  path would have been the one to lose a rule later.
- A hole in the chain is an error on both paths. `rebuild` propagates it, and never returns a
  short list.
- The id of a named file is the file stem when the stem is a session id, and `None` otherwise. A
  name that is not an id is fine, because the file is named by config and not by the store.

## Rules out

**Appending with no lock.** Two writers in one file is the worst outcome the store has, because
it is silent and it corrupts.

**Treating the key as an ephemeral escape hatch.** It writes a file, so it is a session.

**A second set of rules for a named file.** Every rule that held for a store session had to be
restated, and a restated rule is a rule that drifts.

## Cost

One `lock_file`, one `rebuild` shared by two paths, and three tests.
