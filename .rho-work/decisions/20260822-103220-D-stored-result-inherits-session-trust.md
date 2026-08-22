# D-stored-result-inherits-session-trust — the store is as private as the session file


**Question (controller):** the result store writes a full tool result to disk. Does that add
a new place a credential can leak?

**Decision:** No new place. The store replaces the session sidecar for a large tool result,
so the number of copies on disk stays at one. The store lives beside the session file, and
it inherits that file's privacy and its lifetime.

Today an oversize tool result already spills to `<session>.<record>.sidecar`, in full and
unredacted. Once the harness caps the result before it reaches the context, the session
record holds only the preview, so the writer has nothing left to spill. The full text
lives in the store instead, under a handle the model can read.

**Reason:** this is a move, not an addition. Claiming the change is neutral is only honest
if the old copy really goes away, so `a_capped_result_writes_no_sidecar` proves it.

**What is still true, and worth saying plainly.** A tool result can contain a credential.
`grep` over a `.env` file returns one. rho redacts a tool *argument* on the way into the
session file, per D-redact-tool-arguments, and rho redacts no tool *result* text anywhere,
before or after this change. So a session directory can hold a secret that a tool read. The
store does not change that, and it does not fix it.

**When there is no session file.** `rho-cli` does not persist a session yet, so there is no
file to sit beside. The faithful reading of "the session's privacy and lifetime" is then a
private directory that dies with the process. rho uses a temporary directory with owner-only
permissions, removed when the session ends. Once `rho-cli` writes a session file, the store
moves next to it and this paragraph goes away.

**Rules out:** writing the payload anywhere but beside its session. A store that outlives
its session file. A second unredacted copy alongside the sidecar. Claiming the store is
redacted, which would be a slogan. A store shared between sessions, which
D-no-cross-session-cache already refuses for a different reason and which would also let one
session read another's evidence.

**Left open, and named so it is not forgotten:** redacting tool result text. That is a
larger change than this spec, it needs a free-text detector rather than a JSON key match,
and it belongs with `F-no-secrets-in-logs`. This decision does not settle it.
