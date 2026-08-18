# D-reopen-stated-on-disk — A reopen is stated on disk, and every io error names its path

**Question (controller, real drive of the session operations):** what happens when a resume
appends to a closed session, and what does an io error tell the user?

**Decision:** `SessionStore::append_to` writes a `Reopened` record when the file ended with
`Closed`. The invariant is that a `Closed` record is followed by nothing, or by exactly one
`Reopened` record. Every `SessionError::Io` names the path that failed.

**Reason:** A drive of the operations found both. A close followed by a resume left `Closed`
in the middle of the file, so a reader could not tell a closed session from one that kept
talking. Every unit test closed a session or resumed one, and none did both to one file. The
io message said `No such file or directory` and never said which file, which tells a user
with 500 sessions nothing. `ConfigError::Read` already names its path, so the session error
now matches it.

**Rules out:** A silent append after a close. A bare operating-system message with no path.
