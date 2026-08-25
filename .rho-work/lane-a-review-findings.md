# Lane A review findings

Four hostile reviewers read `SPEC-session-store-wiring` before any code. Each had one
mandate, so none could repeat another's work.

- test quality
- architecture and the data model
- product, against what the owner actually asked for
- operations and portability

This file records every finding, and my verdict on it. A finding I could check, I checked.
One reviewer's mechanism was wrong, and that is recorded too.

Two findings need the owner's decision. They are in section 4.

---

## 1. Blocking, and confirmed against the code

### C1. A colliding id truncates a live session file

`SessionStore::create` calls `File::create`, and the comment says "Create or truncate". See
`crates/rho-core/src/session/mod.rs` line 924.

The id is a one-second stamp plus four hex characters. That is 16 bits, so 65536 values.
For N sessions minting in one second the collision chance is about `N(N-1)/2 / 65536`. At 50
concurrent sessions that is 1225 in 65536, which is 1.87 percent.

So about one run in 53 would erase another session, in silence.

**Fix.** Create the file with `create_new`, which fails when the path exists. On a failure,
mint again. The id then cannot collide, whatever the random suffix does.

### C2. The row seam is bypassable, so its test is theatre

`row_from` takes both a `source` and a `path`. An implementation can ignore the source, open
the path, and read the whole file. The counting source then reports almost nothing, and the
byte assertion passes.

That is the memory-cap defect again. See `D-bash-line-cap`.

**Fix.** The row builder never opens a path. It takes the display path, the size, and the
modification time as data. The test passes a path that does not exist, and content that
lives only in the source. So an implementation that opens the path fails the test.

The counting reader also sits **under** any buffered reader, so a full forward scan is
counted.

### C3. The key resolver has no seam, so its cap test is theatre

`ProjectKey::resolve` opens the `.git` entry itself. An implementation can read all ten
megabytes and then keep the first 4096 bytes. It returns the right key, so the test passes.

**Fix.** The bounded read takes its input through a `BufRead`, like `SessionReader::read_from`.

### C4. The pi import writes files that the new orphan check refuses

This is the finding I least expected, and it is real.

`import_pi_session` keeps every `parentId` verbatim. See
`crates/rho-session-import-pi/src/lib.rs` line 149, and the comment at line 47.

`SPEC-sessions` section 9 drops `compaction`, `custom`, and `custom_message` records. A
`compaction` record sits **inside** the parent chain. Its children keep pointing at the
dropped id.

Real data from 60 pi files: 5 `compaction`, 96 `custom`, and 45 `custom_message`.

So the orphan refusal in section 6a would refuse every such imported file. One spec would
produce files that another spec refuses to read.

**Fix.** The import re-parents a kept record onto its nearest kept ancestor. A dropped
record then leaves no hole. One new test reads an imported file back.

### C5. Two worktrees continue the same file, with no lock

The project key is shared by every worktree. `--continue` resolves to the newest open
session for that key. So two worktrees can append to one file at the same time.

Both seed their next id from the same read. Both mint the same ids. The lines interleave.

**Fix.** Two parts. An advisory lock on append, and a session that another process holds is
not offered by `newest_open`. See the decision needed in section 4.

### C6. The owner's headline ask has no path, and the surface is advertised

The owner asked to fork from any turn and any assistant message.

A fork needs a record id. Record ids are `r1`, `r2`, and so on. Nothing shows them.

- `rho sessions show` is named once, with no output format and no test.
- `/tree` is named in the spec and in a decision. It is specified nowhere, and tested nowhere.
- `/tree` and `/fork` are not in the terminal's command list today.

That is `D-dead-surface-is-a-defect-class`, word for word. It is the `/guide` defect again,
where the first frame advertised a command that answered an error.

**Fix.** See the decision needed in section 4.

---

## 2. One finding whose mechanism was wrong, and what is really there

The architecture reviewer said `fork` copies the source header, so a forked file holds two
`Session` records and two `r0` ids. It concluded that a fork writes a file the new checks
refuse.

**The mechanism is wrong.** `SessionReader::read_from` consumes the first line before its
loop, so `read.entries` never holds the header. The parent walk in `fork` stops when an id is
not in that map. So the walk stops **at** the header and never copies it. There is one
`Session` record in a forked file, and no duplicate `r0`.

**The conclusion is right for a different reason.** The first copied record keeps
`parent_id = Some(r0)`, because the source header was minted first and every header is `r0`.
The new file's header is also `r0`, minted first. So the link resolves **by coincidence**.

An imported pi file breaks the coincidence, because a pi header id is eight hex characters.
Then the first copied record points at an id the new file does not hold, and the orphan check
refuses the fork.

**Fix.** `fork` re-parents the first copied record onto the new header id, and the spec states
that as an invariant. The coincidence stops being load-bearing.

I checked this rather than trusting the rating. `AGENTS.md` step 9 asks for exactly that.

---

## 3. Confirmed, and not blocking

### Test quality

- `a_chain_record_set_is_frozen` is vacuous. An exhaustive match fails at compile time, so as
  a runtime test it asserts nothing. Keep it as a compile guard, and add a version assertion.
- `a_leaf_record_never_appears_in_a_parent_chain` is vacuous over written files, because no
  writer can produce one. It must run over hostile hand-built entries.
- `the_key_resolver_spawns_no_process` cannot fail. There is no way to observe a spawn. Cut it.
- `rows_come_back_newest_first` proves the order, and not the claim of no file read. The
  counting seam proves the second half.
- `a_forged_fork_origin_opens_no_file` needs a sentinel path that the test asserts is never
  opened.
- The clap tests must parse through the real command. A hand-built parser would pass while
  the shipped flag lacks `require_equals`.
- The orphan tests must run through `read_from`, not through a helper. A guard with no caller
  is this project's signature defect.
- The budget test asserts wall-clock time, so it will be flaky in shared CI. It should assert
  the work: the bytes read and the files opened. The millisecond number stays a printed
  measurement, and it asserts nothing.
- Nothing tests `delete` at all. Not the sidecar removal, not the surviving fork, not a
  missing file.
- Nothing tests the twice cases: append twice, fork twice, resume twice, reopen a reopened
  file, or create twice.
- Nothing tests a file smaller than the tail window, or a first prompt beyond the head lines.
- Nothing tests the new header fields written and then read back.
- Nothing tests `Display for RecordId`, which every new error message needs.

### Architecture

- `seed_ids` is public surface that the spec forbids anyone to call. Delete it. The writer
  seeds itself from the file it opens, so no method can be misused.
- `PrefixMatch` and two error variants model the same three outcomes twice. Keep one.
- `ProjectKey::resolve` reads `.git` inside `rho-core`, while section 2 says the key is
  injected. Split it. Core keeps the sanitizer and the digest, which are the security part.
  The command line owns the git lookup.
- No compatibility matrix exists. Two cells matter. An old build calls a `Name` record
  corruption, and the new refusals reject files an old build read happily.
- Cut `title_is_explicit`, `started_millis`, and `NewSession.forked_from` until a reader
  wants them.
- The seams cannot be `pub(crate)`, because the session tests are integration tests in
  `crates/rho-core/tests/`. So each seam stays public and says in one line why it exists.

### Product

- Every error message needs a next step. `Orphan` and `DuplicateId` are jargon with no
  action. `NoSessionToContinue` joins two instructions with a semicolon.
- The refusal for an id passed with a space is described and never written verbatim.
- `rho sessions list` has no column layout, no example, and no test at 80 columns.
- A user cannot look at a session without inventing a prompt, because `run` needs one.
- After Lane A, rho is still behind pi on `/tree`, `/clone`, `/compact`, `/export`, and a
  delete key in the picker. It is behind claude on rewind.

### Operations

- Windows gets no hardening, and the workspace never says it is unsupported. CI runs Linux
  and macOS only. Yet three crates read `USERPROFILE`, so the code implies support.
- A synced folder, such as Dropbox or iCloud, copies the store as the user. So the `0o600`
  privacy claim does not hold there, and the spec must admit it.
- The start path has no budget. The list budget of 100 milliseconds is about 15 times rho's
  measured 6.5 millisecond first frame. `newest_open` and the crash offer both run at start.
- A sidecar spill has no size bound, so a heavy month reaches gigabytes with no ceiling.
- A failure inside `create` is unspecified. The degrade test drives the append sink only. So
  a full disk or a read-only home has no stated behaviour.
- `HOME` unset leaves the store root undefined. That happens in a container, under systemd,
  and under `sudo`.
- `delete` on a file a writer still holds keeps writing to an unlinked inode on unix.
- Nothing logs at error level, and nothing reports the store size. So the growth failure is
  invisible until a disk fills.
- A tail read can catch a torn last line, because an append does not `fsync`. The spec drops
  a partial first line only.

---

## 4. Two decisions for the owner

### Decision 1. Does Lane A deliver the record id, or does the ask wait?

The owner asked to fork from any turn and any assistant message. Lane A cannot do it today,
and it advertises two commands that do not exist.

- **A. Deliver it on the command line.** Specify `rho sessions show` verbatim, with one
  record per line and its id. Specify the `list` columns at 80 columns. Remove `/tree` and
  `/fork` from the Lane A surface, so nothing is advertised unbuilt. The terminal tree gets
  its own lane and its own spec.
- **B. Keep Lane A as it is, and accept that the ask waits.** Then the spec must delete every
  mention of `/tree` and `/fork`, and say plainly that a fork needs an id no surface shows.

I recommend A. It is small, it delivers the ask, and it stops advertising dead surface.

### Decision 2. Does the project key stay shared across worktrees?

The architecture reviewer named this the decision most likely to be reversed. It causes C5.

- **A. Keep the shared key, and add a lock.** An advisory lock on append. A session another
  process holds is not offered. The list shows the directory per row.
- **B. Reverse to a per-worktree key.** Then two worktrees never share a file, and the
  concurrency hazard goes away at the root. A picker can still show a sibling worktree's
  sessions, read-only. The lock is still wanted, but it guards less.

I recommend A, because it keeps the behaviour the owner chose, and the lock is needed either
way. B is safer and gives up the thing the owner asked for.
