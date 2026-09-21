# D-resume-restores-the-recorded-provider — A resume restores the recorded provider and model


**Question (architect):** what does a resume run on, after the user switched the provider
mid-session and closed the session?

**Decision:** A resume restores the provider and the model together. It reads both from the
newest `Record::ModelChange`, which names the running pair. An explicit `--provider` flag
wins over the record, and rho announces the override. Absent the flag, the recorded provider
wins. Absent any `ModelChange`, the resume keeps the resolved provider. A recorded provider
that cannot rebuild refuses the resume through `ResumeProviderError`, which names the
recorded provider and the fix. rho never falls back to another provider in silence.

**Reason:** the user ruled it. They said: switching to provider XXX with model YYY, resuming
tomorrow, should return to provider XXX and model YYY. The data already exists.
`Record::ModelChange { provider, model }` carries the provider, and the reader dropped it
with `..`. This is the project's named defect class: a field the pattern hides and no caller
reads. So the fix is small: read both fields, and build the recorded provider through the
same gate startup uses.

The flag wins because it is a present-tense instruction, and the file is a record of the
past. A resume on a different provider than the file records is the same silent lie.
`open_recording` already refuses that lie. rho would otherwise answer from the wrong provider
while the user believes it continued theirs.

`D-resume-never-widens` guards the approval mode and the sandbox mode. A provider is neither,
so a provider restore does not touch the widen order, and the mode check still runs first. A
provider restore can only narrow. The current config may remove the entry, revoke the
credential, or drop trust. Each tighten refuses the build. So a resume cannot gain a
credential the current config denies. This follows the reasoning of `D-resume-never-widens`
where it applies. It differs in one way. A provider has no total order, so it has no
`--allow-widen`. A build failure refuses it, not an order check.

## What this rules out

- A silent fallback to another provider when the recorded one cannot rebuild.
- A recorded provider that overrides an explicit `--provider` flag.
- A resume that reads the model back but drops the provider.
- A flat error string. `ResumeProviderError` names the recorded provider and the fix.
- A resume that rebuilds a provider the current config no longer trusts.
- A new session record type or a format version bump. `ModelChange` is unchanged.
- A new reader that silently drops the recorded provider. It refuses an unknown recorded
  provider instead.
