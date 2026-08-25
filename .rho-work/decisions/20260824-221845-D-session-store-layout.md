# D-session-store-layout — where a session file lives, and how a project is keyed

**Question:** rho writes no session file today. Where does one go, and how does a run
find the sessions that belong to this project?

## The decision

```
~/.rho/sessions/<project-key>/<session-id>.jsonl
```

The project key is `<directory-name>-<8 hex characters>`. The hex is a digest of the
project identity path. So a human reads the directory name, and two projects never
collide.

**Project identity resolves in this order.**

1. The git common directory, when `.git` names one. Every worktree of one repository
   then shares one key.
2. Else the physical root path.

rho reads the `.git` entry itself. When `.git` is a file it holds one line, such as
`gitdir: /path/to/main/.git/worktrees/<name>`. rho parses that line and walks up to the
repository. So rho spawns no git process and adds no git dependency.

## Rules that hold

- The store root is separate from the project root. `rho-cli` already calls the project
  root `session_root`, and that name keeps its meaning. The store gets another name.
- The identity function is one function with a fallback. It never fails a run. An
  unreadable `.git` falls back to the physical path.
- A session file states its own `cwd`, so a list shows where a session ran.
- `session-file <path>` still names one exact file, and it overrides the store.

## Rules out

**pi's scheme, which turns a path into dashes.** Two different paths can produce one
slug there, and the sessions then mix.

**The flat directory codex keeps.** Every project shares one directory, so a list has to
read every file to find the ones that matter.

**A git subprocess at startup.** A process spawn on the start path costs more than the
parse, and `F-fast-cold-start` is a feature.

**Keying on the absolute path.** This repository is a worktree. A path key would hide its
sessions from the main checkout, and it is one project.

## Cost

One identity function, one digest, and one directory creation. No new dependency.
