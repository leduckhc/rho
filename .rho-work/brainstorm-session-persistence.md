# Brainstorm: persistent sessions, resume, delete, fork

Status: brainstorm. No decision is made here. This is step 1 of the flow in `AGENTS.md`.

Owner asked for this after reading the session page. Nothing here is a promise.

---

## 0. Two corrections first

**The file.** There is no `docs/design/session.md`. The page you read is
`docs/guide/sessions.md`. The design lives in `docs/specs/20260818-014343-SPEC-sessions.md`.

**The assumption.** The page is right about the `rho` command. It is wrong as a
statement about rho. The library already has all four things you asked for.

Here is the proof, by line number in `crates/rho-core/src/session/mod.rs`:

| You asked for | It exists | Line |
| --- | --- | --- |
| Persistence | `SessionStore::create`, `SessionWriter::append` | 913, 322 |
| Resume | `SessionReader::read`, `branch_messages` | 770, 1084 |
| Delete | `SessionStore::delete` | 1003 |
| Fork | `SessionStore::fork` | 1028 |
| List | `SessionStore::list` | 966 |
| Branch inside one file | parent links on every `Entry` | 30 |
| Resume cannot widen a permission | `check_resume_permission` | 227 |
| Import from pi | `rho-session-import-pi` | crate |

So the work is not "build persistence". The work is "connect it, and then go past
the others". That is a much better place to start from.

---

## 1. The exact gap today

Nothing in `crates/rho-cli` and nothing in `crates/rho-tui` calls any of it.

- No store root. rho has `~/.rho` for config, skills, agents, and MCP. It has no
  session directory.
- `SessionRecorder` is never built. So no event ever becomes a record.
- The config keys `session-file` and `ephemeral` parse, and nothing reads them.
  See `crates/rho-config/src/lib.rs` lines 768 and 769.
- There is no `--resume`, no `--continue`, no `--fork`, and no `--allow-widen`.
- `SessionError::Widen` tells the user to pass `--allow-widen`. That flag does not
  exist. The error names a flag rho does not have.
- `/sessions` answers that it is not built.
- Nothing writes a title. So a list of 500 rows would show 500 file names.
- `fork` needs a record id. No frontend can show a record id, so no user can pick one.

**A naming trap.** In `rho-cli`, `session_root` means the project directory. It is the
path confinement root. It is not the session store. Two different things want that
name. We must pick another name for the store, or a reader will confuse them.

---

## 2. What the other five tools do

Every fact below came from reading the real files on this machine.

### pi

- Store: `~/.pi/agent/sessions/--path-with-dashes--/<stamp>_<uuid>.jsonl`.
- Format: JSONL. Every line has `id`, `parentId`, and a timestamp. So it is a tree.
- Resume: `pi -c` continues the newest. `pi -r` opens a picker. `--session <part-of-id>`
  resumes by id.
- Tree: `/tree` shows the whole tree. Arrows walk it. Enter picks a record.
- Fork: `/fork` starts a new file from an earlier **user** message. The old text is
  put back in the editor. `/clone` copies the current branch.
- Delete: `Ctrl+D` in the picker. It uses the `trash` command when there is one.
- Titles: `/name "..."` writes a title record. The picker shows it.
- Picker row: name or first prompt, time ago, message count, token estimate.
- Compaction: `/compact` writes a summary record with a kept tail.
- Export: `/export` writes HTML.

### jcode

- Store: `~/.jcode/sessions/session_<word>_<stamp>_<hash>.json`. One flat directory.
- Format: one JSON file with a flat list of messages. No parent links.
- A sidecar `*.journal.jsonl` streams live updates, so the main file is not rewritten.
- Fork: a notice message names the parent. The link is text, not structure.
- Picker row: title, time, message counts split by role, tokens, working directory,
  model, status, and the first prompt.
- Crash recovery has its own batch screen.

### codex

- Store: `~/.codex/archived_sessions/rollout-<stamp>-<uuid>.jsonl`. One flat directory.
- The path holds no project. So sessions from every project sit together.
- A `history.jsonl` index makes the list cheap.
- No parent link. So the format cannot express a fork.
- `~/.codex/shell_snapshots/` keeps the shell environment per session.
- The header holds the whole system prompt as one string.

### claude

- Store: `~/.claude/projects/-Users-le-Work-Vibe-rho/<uuid>.jsonl`. Per project.
- Records carry `parentUuid`, `sessionId`, `version`, `cwd`, and `gitBranch`.
- `~/.claude/file-history/<uuid>/<hash>@vN` keeps old versions of edited files.
- So the tool can put a file back the way it was.
- The store grows with no limit. Nothing prunes it.

### What to take, and what to refuse

| Idea | Take it? | Why |
| --- | --- | --- |
| A directory per project | Yes | A flat directory does not scale. codex proves that. |
| Parent links on every record | Already have | This is what makes a fork cheap. |
| Continue the newest, and resume by id | Yes | Both are one line of work each. |
| A picker with rich rows | Yes | jcode's row is the best of the five. |
| A title record | Yes | A list with no title is a list of file names. |
| File versions, for a real rewind | Yes, and go further | See idea 1 below. |
| An index file for the list | Not yet | Measure first. Our list reads one line per file. |
| A path turned into dashes | No | Two paths can make one slug. Add a short hash. |
| The system prompt stored verbatim | No | It can hold pasted secrets. Store a digest. |
| A store that grows for ever | No | Speed and memory are features here. |

---

## 3. Table stakes: the wiring

This part is not clever. It is the part that makes rho equal to the others.

1. A store root. `~/.rho/sessions/<project-slug>-<short-hash>/<stamp>-<id>.jsonl`.
2. Record by default. Ephemeral becomes an opt-out, not the only mode.
3. `rho run --continue` picks up the newest session for this directory.
4. `rho run --resume <id>` takes an id, or the first few characters of one.
5. `rho sessions list`, `show`, `delete`, `fork`, `export`. One subcommand, five verbs.
6. `--allow-widen`, because the library error already names it.
7. `/sessions`, `/resume`, `/tree`, and `/fork` in the terminal.
8. A title record, plus an automatic title from the first prompt.
9. A picker row: title, time ago, turns, tokens, cost, model, and the branch count.

That closes `F-session-resume`, `F-session-list`, `F-session-delete`, `F-session-fork`,
`F-session-branching`, and `F-ephemeral-mode` at the command line.

---

## 4. Ten ideas to go past them

Each idea has a cost note. I ranked them by value over cost.

### Idea 1. One rewind that moves the code and the talk together

claude keeps old file versions. It keeps them beside the transcript, not inside it.

rho can pair them. Every tool that writes a file already knows the bytes. So rho
stores the old bytes under the session, keyed by the record id that changed them.

Then one command does the whole job:

```sh
rho sessions rewind <session> --to r-0042
```

The conversation goes back to record `r-0042`. Every file rho wrote after `r-0042`
goes back too. The old branch stays on disk, because the file is append-only.

Rules that keep it safe:

- rho never touches git. It writes no commit and no stash.
- rho refuses to restore a file that changed outside rho. It says which file. It
  is a refusal, not a guess.
- A snapshot is content addressed. So ten edits of one small file cost one copy each,
  and an unchanged file costs nothing.
- A snapshot over a size cap is not stored. The rewind then says the file is not covered.

This is the strongest idea here. `F-undo-tracked-change` is already planned, and it
already says the change log is read from the session file. This is that feature, done
properly, with the record id as the key.

Cost: medium. It needs a new record type and a blob directory. It needs a size cap.

### Idea 2. A session you can replay with no network

A session file holds every tool call and every tool result. That is a recording.

So rho can replay one. The provider is a stub that returns the recorded answers. The
tools are stubs that return the recorded results.

```sh
rho replay <session> --check
```

Why this is worth more than it sounds:

- `AGENTS.md` forbids network access in tests. Today a fixture is written by hand.
  With replay, one real run becomes a fixture.
- Step 11 of the flow asks us to drive rho for real. A replay makes step 11 repeatable.
- A regression test becomes "replay session X, and the tool call sequence must match".
- Every defect we found live becomes a recorded case, not a paragraph in a doc.

This turns the session format into a test tool. None of the five tools do this.

Cost: low to medium. The provider seam and the tool trait already exist.

### Idea 3. The same session, a different model

A replay can also swap the model back in. Then rho sends the real prompts to a
different provider, and compares.

```sh
rho replay <session> --model <other> --diff
```

The output says what changed: the tool calls, the token count, the cost, the time.

So a user answers a real question cheaply. "Would the small model have done this
job?" Today nobody can answer that without doing the work twice by hand.

Cost: medium. It needs a clear report format, and it spends real money. So it is
never automatic.

### Idea 4. Sessions follow the project, not the path

pi and claude both key the store on the absolute path. That breaks in a worktree.

This very repository is a worktree at `/Users/le/.worktrees/rho/worktree-e56c35`.
Its sessions would not be found from the main checkout. That is wrong, because it
is one project.

So rho keys a session on project identity:

- The git common directory, when there is one. So every worktree shares one identity.
- Else the physical root path.
- The store keeps the path too, so a list can show where a session ran.

Then `--continue` finds the work you did in the other worktree this morning.

Cost: low. It is one function with a fallback.

### Idea 5. Compare two branches

Records form a tree. Two branches share a prefix. So a compare is cheap.

```sh
rho sessions compare <session> --a r-0042 --b r-0071
```

It prints what each branch did after the split: the turns, the tools, the files
touched, the tokens, and the cost.

This makes exploration measurable. You tried two ways. Now you can say which one
was cheaper, and which one touched less of the tree.

Cost: low. It is a read and a diff over data we already hold.

### Idea 6. A bounded store

codex and claude both grow with no limit. rho sells memory discipline, so it should
not ship the same hole.

Config keys: `sessions.keep-days`, `sessions.max-bytes`, `sessions.keep-named`.

Rules: a named session is never pruned by age. A prune never blocks startup. A prune
reports what it removed. A dry run prints the plan.

Cost: low. It pairs with `F-fast-cold-start`, because the prune must not slow a start.

### Idea 7. Search every session, then fork from the hit

We already have a bounded reader. So a search over the store is a small step.

```sh
rho sessions grep "sandbox-exec"
```

Each hit names the session, the record id, and the line. Then one more command forks
from that record. So an old good answer becomes a new starting point.

Cost: low.

### Idea 8. Take one turn from another session

Sometimes you want one thing from an old session. A plan. A long tool result.

```sh
rho sessions graft <from-session> r-0042 --into <to-session>
```

rho appends the chosen records, and marks where they came from. The origin is a
field, not a sentence in the text. So a reader can always tell grafted context apart.

Cost: medium. The contract needs the origin field, and the model must be told.

### Idea 9. Watch a running session from another terminal

rho already streams a JSONL transcript per subagent. A user can `tail -f` it.

Give the main session the same courtesy, then add a reader:

```sh
rho sessions attach <id>
```

It renders the live session read-only. Two terminals, one run. It also gives the
owner's app a supported way to render a session it did not start.

Cost: low, because the transcript writer exists.

### Idea 10. A crash offers to continue

The reader already handles a half-written last line. So the hard part is done.

On start, rho checks this project for a session with no `Closed` record. It offers
to continue that session. jcode does this with a batch screen, and it works well.

Cost: very low. It is one check and one prompt.

### Smaller ideas worth one line each

- A denied tool call is a decision point. Mark it, so "what if I had allowed it" is one key.
- A cost ledger per branch, from the usage records we already write.
- A digest of the redaction rules in the header. So a reader can tell how safe a file is.
- A per-session bundle: transcript, model, tools, instruction digest. One file to hand over.
- `rho sessions export --html`, because pi has it and users like it.
- A session name from the model at close, off by default, because it costs money.

---

## 5. Contract questions to settle before any code

`AGENTS.md` says the contract comes first, and a persisted format binds the next
version of rho. So these need answers, in a spec, before we type.

1. **The store root.** Where, and what is the slug? Who owns the identity function?
2. **The id.** A stamp plus a uuid, like pi? Or a memorable word, like jcode? A
   memorable id is easier to say out loud. It is also easier to collide.
3. **The new record types.** A title. A fork origin. A file snapshot. A checkpoint.
   Each one is a new variant, so each one is a version question.
4. **The version rule.** What does today's reader do with tomorrow's record? See the
   trap in section 6. This is the most important answer in this list.
5. **What a snapshot covers.** Which tools report a change? What is the size cap?
   What happens when a file changed outside rho?
6. **The flag names.** `--continue` or `-c`. `--resume` with and without an id.
7. **The retention keys**, and whether a prune runs on start.
8. **The result store.** Today it dies with the run, and a nonce stops a resumed
   session reading old evidence. See `D-tool-result-handle`. Should a resume keep its
   evidence? If yes, the nonce rule needs a new statement.
9. **Which parts belong over ACP.** `F-session-commands-over-acp` is already planned.

---

## 6. Traps found while reading

**A dropped record breaks the chain.** This is the serious one.

The reader skips a line it cannot decode. It counts the skip and warns. See
`read_from` in `crates/rho-core/src/session/mod.rs` line 784.

Now suppose a later rho writes a new record type in the middle of a conversation. An
older rho cannot decode it. It skips it. The children of that record still name it as
their parent. `branch_messages` walks parent links, so the walk stops at the hole.

So an old reader would silently lose the end of a conversation. It would report a
count, not a broken chain. That is the same family as the fail-open default in
`D-plugin-does-not-classify-itself`.

Options to weigh in the spec:

- Keep an unknown record whole, with its id and its parent, so the chain survives.
- Or let a new record type never sit in the parent chain. A snapshot is a leaf.
- Or refuse a file whose version is newer, which is honest but blunt.

I lean to the first two together. A chain record must be readable by every version.
A leaf record may be new.

**Other traps.**

- `session_root` already means the project root. The store needs a different name.
- A path slug can collide. Add a short hash of the real path.
- A prune on start fights `F-fast-cold-start`. It must not block the first frame.
- A snapshot store can grow faster than the transcript. It needs its own cap.
- `check-ids.py` reads `.rho-work` too. So a new feature id must be registered in
  `docs/features.md` before it is written anywhere.
- Do not store the system prompt verbatim, as codex does. A user can paste a key.

---

## 7. What I suggest we do next

Three lanes, in this order. Each lane ships on its own.

**Lane A, the wiring.** Section 3. It makes rho equal to the five. Small, and it
turns six built features into real ones.

**Lane B, the rewind.** Idea 1. It is the feature a user feels the first day.

**Lane C, the replay.** Ideas 2 and 3. It is the feature that makes this project
better at building itself.

Ideas 4, 6, and 10 are cheap enough to ride along with Lane A.

Before any code, one spec for the contract, and one review of that spec. The
persisted format binds every later version, so it gets the real review.
