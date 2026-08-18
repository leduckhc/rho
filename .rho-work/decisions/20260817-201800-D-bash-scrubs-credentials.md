# D-bash-scrubs-credentials — `bash` scrubs credential variables, and the spec stops overclaiming


A security audit listed three items as "documented but confirmed, minor". The
controller re-read them and disagreed on one, then demonstrated it live.

**The demonstration.** A real model, asked through `rho run`, printed the child's
environment. It saw `OPENROUTER_API_KEY` and `AWS_SECRET_ACCESS_KEY`. A single
network call would exfiltrate them. The model's tool arguments are
attacker-controlled input, because a prompt injection lives in any file the agent
reads. So this is not minor.

**Decision.** `bash` removes every variable whose **name** looks like a credential
before it starts the child. The filter reads the name, not the value, because a
value cannot be recognised reliably. It removes the name as well, so the presence of
a key leaks nothing.

The filter is a denylist, and that needs a reason. An allowlist is safer in
principle, but a command legitimately needs a wide and open-ended set of variables.
An allowlist would break ordinary work, and users would switch it off. A denylist
that catches the recognisable shapes is the useful trade. A false positive costs one
variable. A false negative costs a key.

**The more important half of this decision is the wording.** `SPEC-tool-interface` section 7
said that "path confinement and the approval policy" bound `bash`. The first half
was false, and a false claim about a boundary is worse than no claim. Path
confinement does not apply to `bash`, because `cd` and an absolute path both leave
the session root.

So the spec now states the truth:

- The **approval policy is the only real boundary** for `bash`. It declares
  `ToolKind::Execute`, so a read-only policy denies it outright. `--read-only` is the
  way to point rho at a repository you do not trust.
- Credential scrubbing is **defence in depth, not a boundary**. A shell can still
  read `~/.aws/credentials` from disk. Anything that runs commands can read files the
  user can read.
- Sprint 1 adds no container and no namespace sandbox. That is the honest limit.

**Verified live after the fix.** The same injected probe now reports zero matching
variables, and `PATH` still works.

**The two remaining audit items stay open, and are recorded rather than hidden.**
`bash` has no path confinement, which is now documented as intended. And
`PluginHost::launch` does no path validation, so a user who configures a plugin from
a writable directory trusts that directory. Both need a design decision, not a patch.
