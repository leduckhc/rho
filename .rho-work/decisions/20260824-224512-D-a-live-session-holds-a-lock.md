# D-a-live-session-holds-a-lock — fx is the only prior art that solved this race

**Question:** two worktrees share one project key, so `--continue` in both can open one
session file. Both seed their ids from the same read, so the lines interleave and the ids
repeat. What stops that?

## What the prior art does

I read three tools before choosing, because a guess here costs a corrupt file.

| Tool | Key | Session lock | Two writers possible |
| --- | --- | --- | --- |
| pi | the absolute working directory | none | yes, unguarded |
| jcode | the session id only, one flat directory | none per session | no, one daemon owns writes |
| fx | the session id | yes, a per-session advisory lock | no |

**pi** mangles the absolute path into a directory name, and takes no session lock. Two pi
processes in one directory both append to one file. Its record ids are checked against an
in-memory index only, so two processes can mint one id.

**jcode** keeps every session in one flat directory. It has no per-session lock. Its
protection is architectural, because one daemon owns every write and holds a process-lifetime
lock. Its storage code states the interleave risk in a comment, and it guarantees one whole
line per write.

**fx** holds `session.lock` per session. A second writer gets a busy error, reported as "open
elsewhere". When the filesystem cannot lock, fx refuses instead of continuing. It also creates
a session exclusively, so an id collision can never truncate an existing session.

**Nobody consults git.** No tool unifies worktrees, so the shared project key is rho's own
bet. See the note at the end.

## The decision

rho copies fx. A live session holds an advisory lock.

- Every write path takes the lock: a create, a resume, and a fork of what it writes.
- `SessionError::Busy` names the session when another process holds it.
- `newest_open` skips a locked session, so `--continue` never picks a live one.
- A read-only path takes no lock, so `list` and `show` always work.
- A filesystem that cannot lock is a **refusal**, not a warning.
- The lock releases on drop, and the operating system releases it when a process dies.

## Why the lock, and not a narrower key

A per-directory key does not remove the race. pi proves that, because two pi processes in one
directory still collide. So the lock is needed whichever key rho picks.

Given that, the key choice is about what a user wants to see, and not about safety. The owner
wants one pool per repository, so worktrees share.

## Rules out

**A lock file with a pid inside.** A crash leaves it behind, and then a user must delete a
file to work. An advisory lock dies with the process.

**Warning and continuing when a lock is unavailable.** That fails open, and it is the shape
of `D-plugin-does-not-classify-itself`.

**A queue that waits for the lock.** A user would sit and wait with no explanation. fx sets a
deadline and refuses, and a refusal that names the other session is more useful.

**Serialising writes through one daemon, as jcode does.** rho has no daemon, and adding one
for this would be a larger change than the problem.

## The part with no precedent

Sharing one pool across worktrees is rho's own bet. No tool read here does it. So the risk is
real, and the reversal is cheap: it is one identity function. If it proves wrong, the key
becomes the physical worktree path, and the lock stays exactly as it is.

## Cost

One lock type, two error variants, one skip in `newest_open`, and six tests.
