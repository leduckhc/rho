# Lane A contract brief

Status: brief. This is step 2 of the flow in `AGENTS.md`. No code yet.

Scope: Lane A only. Wire the session store to the command line and the terminal.

Out of scope here: the rewind, the replay, and retention. See the brainstorm file.

This brief is the input to one spec. Each item below states the question, the choice
I recommend, the reason, and what the choice rules out.

Three items need the owner's call. They sit in section 4.

---

## 1. Four defects the contract must fix first

All four are latent today, because no caller reaches this code. Lane A is the caller.
So Lane A must fix them, or it ships them.

Each one needs a red test before a fix. That is step 5 and step 7 of the flow.

### Defect A. A record id is minted from a count, so two ids can collide

`SessionWriter::mint_id` formats `r{next_id}` and adds one. See line 291.

Two callers guess `next_id` from a count, not from the ids in the file.

**`append_to`, line 942.** It sets `next_id = read.entries.len() + 2`. The reader drops
a record it cannot decode. So with two dropped records the count is short by two, and
the next append mints an id the file already holds.

**`fork`, line 1072.** It adds one to `next_id` per copied record. A fork copies a
branch, and a branch is not contiguous. Take a chain of `r1`, `r2`, `r4`, where `r3`
is a sibling. Three records copied, so `next_id` becomes four, and the next append
mints `r4`. The file already holds `r4`.

A duplicate id breaks the tree. `branch_messages` walks parent links, so it can walk
into the wrong ancestor. A fork takes a record id, so the target becomes ambiguous.

There is a third case. `F-pi-session-import` brings pi ids, and a pi id is 8 hex
characters. A counter means nothing in that file.

**The rule to write into the contract.** A writer holds the set of ids in the file. It
mints an id that the set does not hold. The count never decides an id.

### Defect B. One bad file fails the whole list

`SessionStore::list` calls `File::open(&path)?` and `parse_header(...)?`. See lines
1069 and 1076. So one unreadable file makes `rho sessions list` fail with nothing shown.

A store gathers junk over a year. A foreign `.jsonl` is enough to break the list.

**The rule.** A file that does not open, or does not parse, becomes a row marked
unreadable. The list never fails because of one file.

### Defect C. The header does not hold its own session id

`parse_header` sets `session_id: String::new()`. See line 754. The id lives in the file
name only, and `list` recovers it from the file stem.

So a session file cannot say who it is. Copy it, rename it, and the id changes.

**The rule.** The header record carries the session id. See item 2.3.

### Defect D. A reopen skips an id

`append_to` sets `next_id` to the count plus two, so one id is never used. It is
harmless. It is also proof that the counter is a guess. Item A's rule removes it.

---

## 2. The contract items

### 2.1 Where sessions live

```
~/.rho/sessions/<project-key>/<session-id>.jsonl
```

The project key is `<directory-name>-<8 hex characters>`. The hex is a digest of the
project identity path. So the name stays readable, and two projects never share a key.

**Project identity, in order:**

1. The git common directory, when `.git` names one. So every worktree shares one key.
2. Else the physical root path.

rho reads the `.git` entry. When `.git` is a file, it holds a line like
`gitdir: /path/to/main/.git/worktrees/<name>`. rho parses that line and walks up.

So rho needs no git subprocess and no git dependency. That protects `F-fast-cold-start`.

**What this rules out.** It rules out pi's scheme, which turns a path into dashes. Two
different paths can make one slug there. It rules out the flat directory codex uses.

**Why the worktree rule matters.** This repository is a worktree. A key from the
absolute path would hide its sessions from the main checkout. It is one project.

### 2.2 The session id and the file name

The id is `<YYYYMMDD-HHMMSS>-<4 hex characters>`. The file is `<id>.jsonl`.

- A directory sort gives newest first. So an ordered list needs no file read.
- The stamp matches the id convention already in `docs/ids.md`.
- The hex makes two sessions in one second safe. Two worktrees can start together.
- A prefix resolves a short id. `--resume=20260825-09` is enough when it is unique.
- An ambiguous prefix is an error that lists the matches. It never guesses.

**What this rules out.** A bare uuid, because it does not sort. A memorable word pair,
because words collide and do not sort. See section 4 for the owner's call.

### 2.3 Two new header fields, and no new chain record

The header record gains two optional fields:

```rust
Session {
    version: u32,
    cwd: PathBuf,
    approval: String,
    sandbox: String,
    /// The session id. It was implicit in the file name before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    /// Set when this file came from a fork. It names the source and the record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    forked_from: Option<ForkOrigin>,
}
```

An old reader ignores a field it does not know, because no type here sets
`deny_unknown_fields`. I checked. So a new field is safe and a new variant is not.

`forked_from` sits in the header, so a list shows the lineage from one line.

### 2.4 The version rule, and what an old reader does

This is the most important item. A persisted format binds every later rho.

Records split into two classes.

**A chain record may be a parent.** The set is frozen: `Session`, `ModelChange`,
`Message`, `Usage`, `Stop`, `Closed`, `Reopened`. Every version must decode all of them.
No new chain record may be added without a version bump.

**A leaf record is never a parent.** A `Name` record is one. An unknown leaf is skipped,
counted, and warned about. The tree survives, because nothing points at it.

**The refusal.** A reader that meets an unknown record, and then finds another record
naming it as a parent, refuses the file. It states the record id. It does not walk past
a hole in silence.

That last rule closes the trap in the brainstorm. Today a dropped middle record orphans
its children, and the parent walk stops early. So a resume would lose the end of a
conversation and report only a count. That is the fail-open shape of
`D-plugin-does-not-classify-itself`.

**The preference, stated as a rule.** Add an optional field to an existing record.
Add a new record kind only when a field cannot carry the meaning.

### 2.5 The cheap list, and what a picker row shows

A row wants: title, time, turns, tokens, cost, model, and the branch count.

Only `cwd` and the version are in the header. Everything else arrives later in the file.
The file is append-only, so the header can never be updated with a total.

**The choice.** One rewritable cache file per session, beside the transcript.

```
<session-id>.jsonl        the truth, append-only
<session-id>.meta.json    a cache, rewritten, never authority
```

Rules that keep the cache honest:

- The transcript is the only truth. The cache holds nothing that the transcript lacks.
- A missing cache costs one scan of that file. A stale cache is repaired by a scan.
- A corrupt cache is deleted and rebuilt. It never fails a list.
- The cache is never read to rebuild a conversation.

**Why one file per session, and not one index per project.** The owner runs many
sessions at once. One shared index means many writers on one file. That invites a
corrupt index and a lock. A per-session file has one writer, always.

**What this rules out.** It rules out the single `history.jsonl` that codex keeps.

**Measure before you trust this.** The spec states a budget: a list of 500 sessions
under 100 milliseconds, on the owner's machine, with the command written down. If a
plain scan already meets that budget, the cache is not needed, and it does not ship.

### 2.6 Titles

- A new session takes an automatic title from the first prompt. One line, 60 characters.
- `rho sessions name <id> "<text>"` and `/name` write a `Name` leaf record.
- The newest `Name` record wins. The cache holds a copy for speed.

No model call. A title must never cost money or time.

### 2.7 Recording is on by default

Today rho writes no session file. After Lane A it writes one for every run.

- `--ephemeral`, `ephemeral = true`, and `RHO_EPHEMERAL=1` each turn it off.
- `session-file <path>` names one file, and it overrides the store.
- A write failure degrades to ephemeral and warns. That is `D-write-failure-degrades`.
- A subagent does not get its own session file in Lane A. It keeps its transcript.

That makes `F-ephemeral-mode` real, and it makes the two dead config keys live.

### 2.8 The command surface

> **Superseded on the day it was written.** Sections 2.2, 2.5, 2.8, and 3 were changed by
> two reviews and by one owner instruction. The spec `SPEC-session-store-wiring` is the
> contract. Read that, not this. This file stays as the record of how the choices were made.

```
rho run "<prompt>" --continue            # the newest session for this project
rho run "<prompt>" --resume=<id-prefix>  # a named session, by the alias
rho sessions list
rho sessions show <id-prefix>
rho sessions delete <id-prefix>
rho sessions fork <id-prefix> --at <record-id>
rho sessions name <id-prefix> "<text>"
--allow-widen                            # with the session flag only
```

In the terminal: `/sessions` opens the picker, `/tree` walks the
records, and `/fork` starts a branch at the selected record.

`-c` is the short form of `--continue`, because pi and claude both use it.

**What this rules out.** A top-level `rho resume` verb. A resume needs every flag that
`run` has, so it stays a flag on `run`.

**One refusal to state.** `--allow-widen` alone is an error. `--resume` is an alias of
`--continue`, so the two can never disagree.

### 2.9 A stale result handle after a resume

A resumed context holds text like `<tool_result_preview handle="r-0001" ...>`. The store
that backs those handles dies with the run. `D-tool-result-handle` says a resumed
session must not read an earlier run's evidence, and a nonce enforces that.

**The choice.** On a resume, rho rewrites each stale preview in the rebuilt context. The
text says the evidence expired, and it keeps the byte count. The model then re-runs the
command instead of calling a handle that fails.

The file on disk is not changed. Only the rebuilt context is.

**What this rules out.** It rules out keeping the result store alive across a resume.
That would need the nonce on disk, and it would weaken `D-tool-result-handle`.

### 2.10 A crash offers to continue

On start, rho looks for a session in this project with no `Closed` record. It offers to
continue that one. The reader already handles a torn last line. See
`D-truncated-tail-warns`.

One offer. One key to accept. No batch screen in Lane A.

### 2.11 Retention is not in Lane A

The store grows without a bound after Lane A. codex and claude both have that hole.

I still leave it out. A config key that nothing reads is a dead surface, and that is a
named defect class here. See `D-dead-surface-is-a-defect-class`. So retention ships with
its own prune, or not at all.

The spec states the gap in writing, so no document claims a bound that does not exist.

---

## 3. What Lane A does not touch

- No file snapshot, and no rewind. That is Lane B.
- No replay. That is Lane C.
- No compaction, and no branch summary. `F-context-compaction` and `F-branch-summary`
  own those.
- No search across sessions.
- No export to HTML.
- No graft between sessions.
- No ACP method. `F-session-commands-over-acp` stays planned, and the contract does not
  block it.

---

## 4. Three calls for the owner

**Call 1. The session id shape.**

- A: `20260825-094512-a3f9`. It sorts, and it matches the id rule in `docs/ids.md`.
- B: `session-otter-a3f9`. Easy to say out loud. It does not sort, and words repeat.

I recommend A. Sorting the directory is what makes the list cheap.

**Call 2. The cache file.**

- A: Ship `<id>.meta.json` per session, and get a fast row with no scan.
- B: Ship no cache. Scan the files for a list. Add a cache only if the budget fails.

I recommend B first, then A only if the measurement demands it. It is one less file
format, and one less thing that can go stale. The spec states the budget either way.

**Call 3. What `--continue` means.**

- A: The newest session for the project key. So a worktree finds this morning's work
  from the main checkout.
- B: The newest session started in this exact directory.

I recommend A, and `rho sessions list` shows the directory per row. So the wider scope
is visible, never hidden.

---

## 5. What happens after these answers

1. One decision file per settled item, in `.rho-work/decisions/`.
2. One spec, `docs/specs/<stamp>-SPEC-session-store-wiring.md`, with the types verbatim
   as compilable Rust, every test named, and an out-of-scope section.
3. A contract review before any code, per step 9. The reviewer answers one question:
   does a new case need an edit to shared code?
4. Then the red tests, starting with the four defects in section 1.
