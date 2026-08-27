# D-a-rejected-definition-is-reported — a definition file that does not load says so

**Question:** `load_definition` returns `Option<AgentDefinition>`. A file that fails to parse
returns `None`, and `discover_agents` drops it. `AgentSet` carries only `loaded` and
`withheld`, so the file has nowhere to go. The user sees nothing. Where does a rejected file
go, and who tells the user?

## What the defect cost

A definition wrote its tool list as a YAML sequence, `tools: [read, list]`. rho read `tools`
as a string, so `serde_yaml` failed and the file vanished. `spawn_agent` was then never
registered, because rho registers it only when at least one definition loaded. The model
answered that it had no such tool. Two live runs were spent before the cause was found. See
`.rho-work/progress.md` and `docs/verification/subagent-slot-queue.md` section 6.

The parse was right by the documented spelling. The failure path was wrong in three ways. It
threw the reason away. It reported nothing to the user. It removed a whole tool from the
session as a side effect.

## The decision

**A rejection is data, not a dropped value.**

1. `load_definition` returns `Result<AgentDefinition, RejectedDefinition>`. The error side
   carries the path, the origin, and a named reason.
2. `AgentSet` gains a third list, `rejected`. Discovery keeps every rejection it met.
3. `RejectionReason` names every case: `Unreadable`, `NoFrontmatter`, `UnclosedFrontmatter`,
   `BadFrontmatter`, `NoDescription`, and `BadToolsField`.
4. Each reason states the repair. `RejectedDefinition::notice` renders one line for the user.
   The line names the file, the reason, and the repair.
5. The command line prints the line before the session starts, with the other notices. A
   rejection is reported even when no definition loaded, because that is the case that
   removed the tool.
6. A rejected file is reported whatever its origin. An untrusted project file is a rejection
   with `SkillOrigin::Project`, not a hidden file.
7. A reason detail may quote the file, so it is untrusted text. The detail is the `Detail`
   newtype, and its constructor is private to `rho-skills`. So a detail cannot exist unless it
   was sanitised and capped at 200 characters. The rule is the type, not a habit.
8. Discovery drops the detail for a project file that the user has not trusted. The path, the
   reason, and the repair stay. So an untrusted repository cannot write its own prose onto a
   start-up line.

## Why the whole file, and not a partial load

A definition with a broken `tools` field could load with the field ignored. rho refuses that,
because an ignored `tools` field means "inherit the parent's whole set". So the repair for a
typo would be to widen the child's tools. That is the fail-open shape of
D-plugin-does-not-classify-itself. A refusal that names the file is safe and teachable.

## Why a Result, and not a second function

A second function, such as `load_definition_with_reason`, leaves the silent one in place. The
next caller picks the short name and loses the reason again. One function with one honest
return type cannot be used wrongly.

## Rules out

**A catch-all reason variant.** No `Other` and no `Unknown`. A catch-all lets a new reason
ship with no message and no test, which is the shape that `ToolKind::Other` had. Every
variant renders through an exhaustive match, so a new variant does not compile until its
repair line exists.

**A plain `String` detail.** The first draft of this decision wrote the cap and the strip in
prose, with a plain `String` in each variant. The review refused it, because a caller could
build a variant with raw file text and the suite would stay green. A private constructor on a
newtype is the only version that holds.

**Quoting an untrusted file to the user.** Five rejections, at 200 characters each, is a
thousand characters of repository prose printed before the session. A withheld project
definition shows only its sanitised name, and a rejected one now shows even less.

**A log line as the report.** `tracing::warn!` reaches a user who set `RHO_LOG`. The skill
loader does exactly this, and it is why nobody saw the skill half of this defect either.

**An unbounded notice list.** A directory of broken files must not push the real output off
the screen. The command line prints at most five rejection lines, then counts the rest.

**Loading a definition with no description.** That rule stands. It is now a reported
rejection instead of a silent drop.

**A fix to the skill loader in this change.** `parse_skill_file` has the same silent shape,
and `SkillSet` has no `rejected` list. That is a second contract, a second set of tests, and
a second reporting path in `extensions.rs`. It stays open, and `SPEC-definition-rejection`
section 7 records it.

## Cost

One new enum, one new newtype, one new struct, one new field on `AgentSet`, one changed
signature, and one notice builder in `rho-cli`. The tests are named in
`SPEC-definition-rejection` section 6.
