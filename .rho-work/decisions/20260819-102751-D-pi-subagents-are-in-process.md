# D-pi-subagents-are-in-process — pi runs a child in-process, not as a separate process


**The question.** What does a pi subagent actually cost? The spec claimed pi pays a
separate Node process per child. That claim drove the comparison in `SPEC-subagents`
section 1. It was wrong.

**The finding, with file and line evidence.**

Reading `@tintinweb/pi-subagents` version 0.17.1, source at
`~/.pi/agent/npm/node_modules/@tintinweb/pi-subagents/src/`:

- `agent-runner.ts` line 921: a child session is created with
  `runInChildSessionContext(() => createAgentSession(sessionOpts))`.
  That is a call into the same process.
- `child-context.ts` lines 1–15: `runInChildSessionContext` wraps `AsyncLocalStorage.run`.
  It is a context flag. It is not a process boundary.
- `agent-manager.ts` line 237: `spawn` is a method on a manager class.
  It is not `child_process.spawn`.
- `agent-manager.ts` line 441: `record.result = responseText`.
  The result travels through a struct field, not a wire or IPC.
- `nested-tools.ts` line 189: `if (context.depth >= context.maxSubagentDepth)`.
  The depth cap is a local integer compare.
- The only `child_process` import in the whole extension is `worktree.ts`.
  That file runs git commands. It does not run agent sessions.

**The correction to the spec.** `SPEC-subagents` section 1 table now reads:

> pi | an in-process session, created inside the same Node process | not published

The sentence "both pay a process per child" is removed. It was only true of jcode.

Section 6 removed the claim that "pi's child transcript lives in another process's
session." pi is in-process, so that sentence had no foundation.

**What this rules out.** No agent or human may claim a process-boundary advantage over
pi's subagent architecture without first reading the source. The original claim entered
because the spec author used a remembered design. That is the violation of AGENTS.md
step 1: read the real source, not your memory of it.

**The advantage that remains.** rho's real advantage over pi is measured density.
50 live sessions consume 24.98 MiB, about 247 KB per session (`docs/benchmarks.md`
line 237). pi's footprint per session is not published, and this decision makes no claim
about it. The density number for rho is measured and stands.
