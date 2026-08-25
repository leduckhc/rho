# D-a-project-key-cannot-escape-the-store — a hostile repository must not choose the path

**Question:** the project key comes from the `.git` entry of a checked-out repository. A
repository is untrusted input. What stops it choosing where rho writes?

## The defect it prevents

A repository can ship a `.git` file holding `gitdir: ../../../../etc`. The last component of
the resolved path becomes the directory name. So a separator or a `..` inside that name
would make `store_root.join(key)` escape the store.

A one-line `.git` file of ten megabytes is a second problem. It is a denial of service, not
a git directory.

This is the `confine` family. That path boundary sat at `todo!()` through a stage that
reported green. See `D-todo-in-a-green-stage`.

## The decision

- `ProjectKey::resolve` reads at most one line, and at most `GIT_ENTRY_MAX_BYTES` of it.
- The directory-name part of the key is exactly one path segment. It holds no `/`, no `\`,
  and no `..`. Every byte outside `[A-Za-z0-9._-]` is replaced.
- A name that sanitizes to nothing becomes a fixed placeholder, never an empty segment.
- The 8 hex characters come from a digest of the full identity path. So two names that
  sanitize to the same text still get two directories.

## Rules that hold

- The invariant is the test: for any `.git` content, the joined path stays under the store
  root. It is driven by a table of hostile lines, not by one example.
- The byte cap is proven with a counting reader, so the test can fail against an unbounded
  read. An assertion on the returned key alone would not.

## Rules out

**Trusting the resolved path because git wrote it.** A repository is checked out from the
network, and rho reads its files before any user reviews them.

**Refusing a repository with an odd `.git` file.** rho would refuse to run in a directory a
user cares about. It sanitizes and continues instead.

## Cost

One bounded read, one sanitizer, and four tests.
