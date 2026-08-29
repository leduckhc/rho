# D-a-project-file-only-lowers-a-limit — the subagents table narrows, and never widens

**Question:** the `[subagents]` table now reaches the agent. A project file inside a clone
can hold one. Which layer wins when a project file and the user's own file disagree?

**Decision: a project `[subagents]` value applies only when it is stricter.** A limit is a
bound, so the stricter value is the smaller one. A project file may lower a cap. It may
never raise one. rho names each limit it refused to raise, so the refusal is not silent.

This is `D-your-settings-are-a-floor` applied to the four limits. That decision is settled,
and this file only states how it lands on `SubagentLimits`.

## The ceiling, and which layers set it

The ceiling is the built-in defaults merged with the **global** file. A home directory is
not a clone, so the user's own file may set any value, high or low.

The project file, and every profile the project file defines, is compared against that
ceiling. A value above the ceiling is lowered to the ceiling, and the limit is named.

A flag is layer 6, so a flag still wins. `--max-live-agents 64` raises the cap in any
checkout, because the user typed it now.

## `--trust-project` does not lift this

`D-your-settings-are-a-floor` separates the two rules in plain terms: the trust flag is
about **loading a capability**, and a limit is not a capability a file adds. It is a bound a
file relaxes. So the comparison holds in every checkout, trusted or not.

A user who wants a higher cap in one repository has two ways already: a flag for this run,
or their own global file for every run.

## Depth stays at one

`rho-cli` forces `max_depth` to 1, because it captures the parent tool set before
`spawn_agent` joins it, so a grandchild cannot exist. See `D-cli-depth-is-zero`.

A `max-depth` key in a file is therefore a bound too, and the same rule applies: rho takes
the smaller of the file value and 1. So `max-depth = 0` works and forbids spawning, and
`max-depth = 3` becomes 1. Without this the key would parse and do nothing, which is the
dead-switch defect this whole lane exists to close.

## The whole-table merge was a defect, and it is fixed here

`ConfigLayer::merge` replaced `subagents` wholesale with `over.subagents.or(self.subagents)`.
So a project table naming one limit erased every limit the global file set. This is the same
shape as the credentials defect that C4 found. The merge now joins the two tables field by
field.

## Rules out

**Letting `--trust-project` raise a cap.** It would fold two different rules into one flag,
and `D-your-settings-are-a-floor` separates them for a stated reason.

**Refusing the run when a project file asks for more.** A stricter run still works, so a
refusal would stop honest work over a value rho can simply lower.

**Dropping the whole table from an untrusted project file.** A repository asking for a
smaller fan-out is useful, and a silent drop teaches nothing.

**New config keys for the six limits that have no key.** The table has four keys today. Six
more keys is a new contract with its own review. See the spec's out-of-scope section.

**A `queue-wait-secs` key.** The deadline follows the child timeout when no flag sets it, so
a config `child-timeout-secs` already moves it.
