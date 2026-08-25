# D-recording-is-on-by-default — a session that is not written cannot be resumed

**Question:** rho writes no session file today. After the wiring lane, is recording on or
off by default?

## The decision

On. Every run writes a session file into the store.

- `--ephemeral`, `ephemeral = true`, and `RHO_EPHEMERAL=1` each turn it off.
- `session-file <path>` names one exact file, and it overrides the store.
- A write failure degrades to ephemeral and warns. That is `D-write-failure-degrades`.

## Rules that hold

- A resume needs a file, so a default of off would make resume unreachable for most users.
- The two config keys stop being dead surface. Both parse today, and nothing reads either.
- A recorder consumes the event stream, so `Session` gains no field.
  See `D-recorder-consumes-events`.
- Every credential-shaped tool argument is masked before a record is written. That is
  `D-redact-tool-arguments`.
- A subagent gets no session file in this lane. It keeps its own transcript.

## Rules out

**Off by default.** A user would find an empty list after a week of work, and would learn
about the flag only by reading a document.

**Recording without redaction.** No record may be written before the redaction function
exists. `SPEC-sessions` section 5a already states that.

## What recording does not protect, stated here on purpose

A security review found that redaction is narrower than the phrase "secrets are redacted"
suggests. `redact_block` masks `ToolCall.arguments`, and `redact_json_secrets` matches a
**key name**. Every other content block passes through unchanged.

So the file holds these verbatim:

- A secret pasted into a prompt.
- A secret inside a tool result, such as the output of `cat .env` or `env`.
- A secret on a `bash` command line, because the key is `command`.
- A token inside a URL, such as a git remote.

Recording still goes on by default, for three reasons. Every comparable tool writes the
same content. The store is private by construction, per `D-a-session-file-is-private`. And
rho already writes a subagent transcript with the same exposure.

But the gap is written down, in the spec and in the guide. A value-shaped scan is named as
required follow-up. And one test is renamed, because
`no_credential_reaches_the_file_on_the_run_path` claimed a guarantee the code does not give.
A test name is a claim.

## Cost

One recorder built on the run path, and one store root resolved at startup.
