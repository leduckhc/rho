# SPEC-subagent-worktree-isolation — A child gets its own working tree

Status: draft. This spec describes code that is not written yet. `bench/check-spec-tests.py`
exempts a draft, because every test below is a promise and not a claim.
Owning crates: `rho-core` for the isolation trait and the report fields, `rho-tools` for
the git implementation and the spawn wiring.
Prior specs: `SPEC-subagents` and `SPEC-agent-tasks`. This spec extends both.

## 1. The problem, and the collision it creates

rho runs up to four children at once. Every child edits the same working tree today. So a
fan-out that writes files is unsafe. Two children can edit one file at the same time.

Isolation gives each child its own tree. This is where the design gets hard.

`crates/rho-core/tests/subagent_security.rs` holds `a_child_cannot_change_the_session_root`.
Its comment says the root is a security boundary and is never overridable. A worktree gives
a child a different root. So it looks like a direct contradiction.

This spec resolves the contradiction. The child still cannot change its own root. The
parent hands the child a root instead. Section 3 states how, and the existing test stays
true and passes unchanged.

Prior art, verified from source:

- pi's installed extension `@tintinweb/pi-subagents` declares `createWorktree(cwd, agentId)`
  and `cleanupWorktree()` in its `worktree.d.ts`. On completion it checks for changes. It
  commits any change to a branch and returns the branch name. It deletes an unchanged tree.
  A definition may set `isolation: "off"` to refuse a worktree.
- jcode has no filesystem isolation for a swarm member.

rho takes pi's shape. rho adds a trait so a third party supplies a container or an overlay.

## 2. Who may grant isolation

A child must never widen its own permissions. So a child must never grant itself a
worktree. See `SPEC-subagents` section 3 for the parent rule.

**Only a trusted caller grants isolation.** A caller grants it in one of three ways:

- A Rust caller installs a `Workspace` implementation in the spawn environment.
- The CLI passes `--isolate-subagents`.
- A configuration key `subagents.isolate` waits on the config call site. `rho-config` parses a
  layer and no binary reads the resolved config yet, so the key cannot work first. See decision
  D-the-layered-config-has-no-caller. Ship the flag, and add the key when that call site exists.

**A definition or the model may refuse, and never grant.** This follows pi. A definition
sets `isolation: off` in its frontmatter. The model sends `isolation: "off"` to a spawn tool.
Both only decline a worktree. Neither can demand one.

**Granting is unrepresentable on both sides, not merely refused.** A definition parses into a
type with one variant, and the tool schema offers one value. So a request to grant cannot be
expressed, and no check has to catch it.

```rust
/// What a definition or a model may ask for. There is no `On` variant, so neither side
/// can grant isolation. Only a caller grants it. See D-isolation-granted-only-by-caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IsolationRequest {
    /// Run in the parent tree, even when the caller enabled isolation.
    Off,
}

// `AgentDefinition` gains one field. `None` accepts the caller's choice.
pub isolation: Option<IsolationRequest>,
```

The `spawn_agent` and `spawn_agents` schemas gain one property, and it holds one value:

```json
{ "isolation": { "type": "string", "enum": ["off"],
                 "description": "Send \"off\" to keep this child in the parent tree." } }
```

A definition that asks for isolation cannot be written, so nothing is ignored silently.

**Why refusing never widens.** A refused child runs in the parent tree. The parent tree is
the parent's own reach. A child is never more permissive than its parent, so a child in the
parent tree is still inside the parent's reach. The child keeps the same approval policy,
the same sandbox, and the same tool set. Refusing removes a worktree. Refusing removes no
restriction.

**Why granting is the caller's alone.** Granting moves the child's session root. The root
is a security boundary. Granting also creates a git branch and uses disk. So granting is a
resource choice and a boundary choice. Only the caller owns both. See
D-isolation-granted-only-by-caller.

## 3. How the session root moves without breaking the invariant

The type that carries the root is `SessionConfig`. Its field `session_root` is a `PathBuf`.
Every tool confines its paths under this root. See `crates/rho-core/src/agent.rs`.

Today the spawn code builds the child config with the parent root:

```rust
let child_config = SessionConfig::new(model, env.parent_config.session_root.clone(), approval)
    .with_sandbox(sandbox)
    .with_max_turns(max_turns)
    .with_max_tool_calls(max_tool_calls);
```

With isolation, the parent-side spawn code substitutes the worktree path:

```rust
// `isolated` is the root the `Workspace` returned. The parent constructs the config.
// The child holds no field that names a root, so the child cannot set this.
let child_root = isolated
    .as_ref()
    .map(|root| root.path.clone())
    .unwrap_or_else(|| env.parent_config.session_root.clone());

let child_config = SessionConfig::new(model, child_root, approval)
    .with_sandbox(sandbox)
    .with_max_turns(max_turns)
    .with_max_tool_calls(max_tool_calls);
```

**The child never changes its root.** The parent hands it one. An agent definition carries
no session-root field. A spawn tool exposes no root argument. So no child-controlled input
names a root.

**The invariant holds, and the test passes unchanged.**
`a_child_cannot_change_the_session_root` builds two configs by hand and asserts the derived
child root equals the parent root. That test pins the no-isolation path. It proves a child
definition carries no root field. It stays true, because the isolation root is a deliberate
substitution by the parent, not a value a child chose.

**Why a confined child cannot reach the parent tree through `..`.** The worktree is a
separate subtree. It lives outside the parent root. See section 10 for the path scheme.
`confine` canonicalises the child root once. It rejects any resolved path that does not
start with the child root. A `..` climbs out of the worktree. The result no longer starts
with the child root. So `confine` returns `PathEscape`. See `crates/rho-core/src/tool.rs`.

## 4. The extension point

A third party must add a container, a copy-on-write directory, or an overlay. It must edit
nothing in rho. So the capability is a trait.

**The trait lives in `rho-core`. `rho-core` gains no git dependency.** The git
implementation lives in `rho-tools`. This is the same split as `CommandRunner` and
`SandboxedRunner`. See `SPEC-agent-tasks` and D-workspace-trait-in-core-git-in-tools.

```rust
use async_trait::async_trait;
use std::path::{Path, PathBuf};

/// Give a child its own working tree. Reclaim it when the child stops.
///
/// The abstract capability lives in `rho-core`. `rho-core` gains no git
/// dependency. An implementation that needs git, a container, or an overlay
/// lives in another crate. `rho-tools` supplies the git one.
#[async_trait]
pub trait Workspace: Send + Sync {
    /// Make an isolated root for a child under `parent_root`.
    ///
    /// The returned path becomes the child's session root. It must be a
    /// directory that `confine` can canonicalise. It must live outside
    /// `parent_root`, so a confined child cannot reach the parent tree.
    async fn create(&self, parent_root: &Path, agent: &str) -> Result<Isolated, WorkspaceError>;

    /// Reclaim the child's root after the child stops and the gate has run.
    ///
    /// The implementation decides what to keep. The git one commits a change
    /// to a branch and returns the branch name. It deletes an unchanged tree.
    /// It never discards a change without keeping it. See section 11.
    async fn reclaim(&self, isolated: Isolated) -> Result<Reclaimed, WorkspaceError>;
}

/// A child's isolated root, plus an opaque handle the implementation needs later.
#[derive(Clone, Debug)]
pub struct Isolated {
    /// The child's session root. Every child tool confines to it.
    pub path: PathBuf,
    /// A value the implementation reads at reclaim time. `rho-core` never reads it.
    /// The git one stores its branch name here.
    pub handle: String,
}

/// The outcome of reclaiming a child's tree.
#[derive(Clone, Debug)]
pub struct Reclaimed {
    /// Where the kept changes live, if any. A git branch name.
    pub kept_at: Option<String>,
    /// The tree the child used. `None` once the implementation deleted it.
    pub left_on_disk: Option<PathBuf>,
}
```

**What a third party writes.** A third party writes one `impl Workspace`. It installs the
implementation in the spawn environment. Nothing in rho changes.

```rust
// `SpawnEnv` lives in `rho-tools`. It gains one optional field.
pub struct SpawnEnv {
    // ... the existing fields ...
    /// The isolation provider. `None` means no child is ever isolated.
    pub workspace: Option<std::sync::Arc<dyn rho_core::Workspace>>,
}
```

## 4a. Every git call is hardened, and the hardening is proved

A `Workspace` that shells out is another process that trusts its parent. A worktree also holds a
`.git` **file** that names its administrative directory, and that file sits inside the child's
own root. So `confine` allows the child to rewrite it. `confine` has no concept of `.git`, and a
boundary that lives only in prose is the defect family this project knows best.

**A live probe proved the attack, and then proved the fix.** The commands and the real output
are in `docs/verification/worktree-git-probe.md`. A child rewrote its own `.git` to name a
repository it controlled. rho's `git commit` then ran the child's `post-commit` hook as the rho
process, and the hook saw four `AWS_` variables. The commit also landed in the child's fake
repository, so a report that claimed a branch would have been a lie.

**Four rules. Each one alone stops the attack.**

1. **Hooks are off for every git call.** rho passes `-c core.hooksPath=/dev/null`, and
   `--no-verify` where a command accepts it. The probe shows no hook then runs.
2. **The administrative directory is recorded at create time, and passed explicitly.** Every
   later call uses `--git-dir <recorded>` and `--work-tree <worktree>`. So a rewritten `.git`
   file is never read. The probe shows the commit then lands on the real branch.
3. **The environment is scrubbed.** An implementation must apply the same scrub `bash` applies,
   at `crates/rho-tools/src/bash.rs:621`. A credential must not reach a git subprocess, exactly
   as it must not reach a model-written command. See decision D-bash-scrubs-credentials.
4. **The committer identity is explicit.** rho passes `-c user.name` and `-c user.email`. A
   machine with no configured identity then still keeps the child's work. Without this the
   commit fails and the work is lost.

rho also sets `GIT_TERMINAL_PROMPT=0`, so a git call never waits for a human. See decision
D-a-workspace-hardens-every-git-call.

**The child may not write `.git` inside its root.** The rules above make a rewritten pointer
harmless, and this rule stops the child from trying. The child's write path denies a path whose
components hold `.git`. This is defence in depth. Rule 2 is the load-bearing one.

**A nested repository is named, not swallowed.** A child may create its own repository inside
the worktree. `git add -A` then records a gitlink, and the nested content is **not** kept. The
probe showed git's own warning for this. So reclaim finds every nested `.git` and lists it in the
report. Silent partial work loss is worse than a named one.

## 5. Ordering with the gate

The gate checks a child's declared artifacts. See `SPEC-agent-tasks`. A worktree may be
removed at the end. So the order matters.

**The exact order:**

1. The child stops. It reaches `Done`, `OutOfTurns`, `Canceled`, `Failed`, or a timeout.
2. The gate runs. It uses the child's own root.
3. The workspace reclaims the tree. It commits a change to a branch, or deletes an empty tree.
4. The report is built. It carries the gate verdict and the isolation fields.

**The gate uses the child's root, always.** For an isolated child, `GateContext.session_root`
is the worktree path. For a shared child, it is the parent root. This forbids one defect by
construction.

**The forbidden defect.** An artifact written inside a worktree, and checked against the
parent root, is a defect. It cannot happen here, because the gate root is always the child's
root. The gate never sees the parent root for an isolated child.

**Reclaim runs after the gate.** The gate must read the child's files. So reclaim cannot run
first. A deleted tree has no files to check.

## 6. The persisted contract

The branch name and the isolation facts must reach the parent. So `AgentReport` gains **one**
field, not a field per fact. A struct behind one optional field stays open for extension, and a
pair of loose fields would grow one per question. See `SPEC-subagents` section 6 for the current
shape, and decision D-no-four-argument-session-new for the pattern this avoids.

```rust
pub struct AgentReport {
    pub agent: String,
    pub outcome: AgentOutcome,
    pub summary: String,
    pub usage: Usage,
    pub turns: u32,
    pub gate: GateReport,
    pub claims: ChildClaims,
    pub transcript: Option<PathBuf>,

    /// What isolation did. `None` means the child shared the parent root.
    #[serde(default)]
    pub isolation: Option<IsolationReport>,
}

/// The facts about one isolated run. Every one is rho's own observation, never a
/// child's claim, so it sits beside `gate` and not beside `claims`.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct IsolationReport {
    /// The child's own root while it ran.
    pub root: PathBuf,
    /// The branch that holds the kept changes. `None` when nothing changed, or
    /// when the commit failed.
    #[serde(default)]
    pub kept_at: Option<String>,
    /// The commit the worktree started from.
    pub base: String,
    /// True when the parent tree held uncommitted changes at create time. So a
    /// caller can assert on it, rather than read a sentence.
    #[serde(default)]
    pub parent_was_dirty: bool,
    /// Every nested repository found in the worktree. Their content is not kept.
    #[serde(default)]
    pub nested_repositories: Vec<PathBuf>,
    /// The tree still on disk, when reclaim could not finish.
    #[serde(default)]
    pub left_on_disk: Option<PathBuf>,
}
```

**`parent_was_dirty` is a field and not a sentence.** A prose notice in a tool result is
advice a model may ignore. A field lets a caller, a test, and a frontend all assert the same
fact. See section 9.

**One field carries the future.** A later question, such as the number of commits or the size of
the change, becomes a field on `IsolationReport`. `AgentReport` does not grow again.

**A newer reader with an older record.** `#[serde(default)]` fills `isolation` with `None`, and
`None` reads as "not isolated". That is the honest reading of a record written before this
feature.

**An older reader with a newer record.** serde ignores an unknown field, so an older reader drops
`isolation` and reads every other field. It loses only the isolation facts, which it could not
use.

## 7. The failure set

The trait returns `WorkspaceError`. It lives in `rho-core` beside the trait.

```rust
#[derive(Clone, Debug)]
pub enum WorkspaceError {
    /// The path is not a git repository.
    NotAGitRepo { root: PathBuf },
    /// The repository has no commit yet, so a worktree has no base.
    NoCommitYet { root: PathBuf },
    /// The branch name is already taken.
    BranchExists { branch: String },
    /// `git worktree add` failed.
    CreateFailed { detail: String },
    /// The child left changes and the commit failed.
    CommitFailed { branch: String, detail: String },
    /// Cleanup failed after the child already finished.
    ReclaimFailed { path: PathBuf, detail: String },
    /// A worktree was left on disk by a crash. A later run reports it.
    Orphaned { path: PathBuf },
    /// Too many orphans already sit on disk. rho refuses a new worktree rather
    /// than fill the disk. See section 10.
    TooManyOrphans { limit: usize, found: usize },
    /// The agent name cannot become a path or a branch, and the id alone was
    /// also unusable. See section 10.
    NameNotUsable { agent: String },
}
```

**Every one of these is a result, not the end of the run.** rho's rule holds: a child
failure returns a tool result the parent can act on. A dead child never kills its parent.
See `SPEC-subagents` section 8.

**Failures before the child runs.** `NotAGitRepo`, `NoCommitYet`, `BranchExists`, and
`CreateFailed` happen at create time. The child never started isolated. The spawn returns
`AgentOutcome::Failed` with the reason. The parent reads it and chooses again.

**rho never silently falls back to the shared tree.** A caller asked for isolation. A
silent fallback would put the child in the parent tree without consent. That would widen the
blast radius. So rho reports the failure and stops the isolated spawn. See
D-isolation-granted-only-by-caller.

**Failures after the child finished.** `CommitFailed` and `ReclaimFailed` happen at reclaim
time. The child's work already exists on disk. rho never discards that work. The report
keeps the child's real outcome. It sets `isolation_root` to the leftover worktree. The
summary names the failure. A human recovers the tree by hand.

**`Orphaned` is diagnostic.** A run never raises it while a child runs. A later run reports
it when it finds a leftover worktree. See section 10.

## 8. Concurrency

Four children live in four worktrees. They share one git repository. git takes an index lock
and a refs lock. Two concurrent `git worktree add` calls race on those locks. One would fail.

**The work is serialised, not retried, and the lock is keyed by repository.** A
`GitWorktreeWorkspace` holds a map from the git common directory to one async mutex. `create`
and `reclaim` both take the mutex for that repository. So two sessions in one process that share
a repository serialise, and two sessions on two repositories do not wait for each other. A
per-instance mutex would fail both cases: two instances on one repository would still race the
index lock, and two repositories would serialise for nothing. A retry loop on a lock is a
thundering herd, and a mutex is deterministic.

**Only the git command is serialised.** The child runs after `create` returns. So the
children still run at the same time. The mutex holds for milliseconds per child.

**What the caller sees while it waits.** A child that waits for the mutex is in a brief
"creating workspace" state. The fan-out reports each child as spawned when its worktree is
ready. A short wait for a lock is normal and invisible to the model.

## 9. What the parent sees: a dirty tree

The parent's tree may be dirty when a child starts. rho does not block a spawn on a dirty
tree.

**A worktree does not carry the parent's uncommitted work.** `git worktree add` checks out
from a commit. It does not copy the parent's unstaged edits. A model expects the child to see
the current files. The child sees only the last commit. So a model would be surprised.

**The rule.** rho bases the worktree on `HEAD`. The child sees the committed state. The child
does not see the parent's uncommitted edits.

**The fact is a field, and the sentence is extra.** `IsolationReport.parent_was_dirty` records
it, so a caller, a test, and a frontend can all assert it. A model may ignore prose, and a field
lets a strict caller refuse the result. The tool result also carries this notice:

```text
The parent tree has uncommitted changes. The isolated child sees the last commit.
The child does not see those changes. Commit them first, or spawn without isolation.
```

The parent may commit its work first. The parent may decline isolation for this child. rho
leaves that choice to the caller and the model.

## 10. Cleanup and orphans

**Who removes a worktree.** The parent-side spawn code calls `Workspace::reclaim`. It calls
it after the child stops and the gate runs. It calls it for every terminal state. A drop
guard runs reclaim even when the child failed, timed out, or was cancelled.

**Removal goes through git, never through a recursive delete.** Reclaim runs
`git worktree remove`, which refuses a tree that git does not know, and which refuses a dirty or
a locked tree. A bare recursive delete would follow a symlink the child created, and this project
has already destroyed a parallel worker's files once. So the removal path validates the target by
asking git, and it never deletes a path git does not list. See decision D-no-git-writes-by-a-subagent.

**A later run and an orphan.** A crash can leave a worktree on disk. A later run never
deletes it. Auto-deletion could destroy unrecovered work. A later run lists an orphan and
reports it. A human removes it.

**An orphan count is bounded, because a disk is bounded.** rho counts the orphans under
`.rho/worktrees/` before it creates a new tree. Past `MAX_ISOLATION_ORPHANS`, which is 32, it
refuses with `TooManyOrphans` and names the directory to clean. Never deleting and never counting
would grow without a limit, which is the family that already cost this project 805 MB. So rho
keeps the work and refuses to add more. See decision D-an-orphan-is-capped-not-deleted.

**The naming scheme, and it never trusts the agent name.** A name in a definition only warns
when it breaks the character rule, and a bad name still loads. `sanitize` strips control
characters and keeps `/`, `..`, a space, and a leading dash. A user-scope definition needs no
`--trust-project`. So the raw name must never reach a path or a git ref.

rho builds a slug from the name: it keeps `[a-z0-9-]`, lowercases the rest, drops every other
character, trims a leading or trailing dash, and truncates to 32 characters. An empty result
falls back to `agent`. Then:

- Worktree directory: `<git-common-parent>/.rho/worktrees/<slug>-<agent_id>-<yyyymmdd-hhmmss>`
- Branch: `rho/agent/<slug>-<agent_id>-<yyyymmdd-hhmmss>`

The `agent_id` and the timestamp make each name unique, so the slug is a label and never the
identity. A name of `../../../../tmp/x` therefore becomes `tmp-x`, inside the intended directory.
See decision D-a-path-slug-never-trusts-an-agent-name.

The worktree directory lives outside the parent session root. So a confined child cannot
reach it, and a confined parent cannot reach it either. The `rho/agent/` prefix makes an
orphan easy to find:

```sh
git worktree list
git branch --list 'rho/agent/*'
```

## 11. The interaction with cancellation and the timeout

A cancelled child leaves a worktree. A timed-out child leaves a worktree. Both may hold
uncommitted work. Losing that work silently is the worse failure. So rho decides on purpose.

**Reclaim always keeps the child's changes.** On cancel or timeout, reclaim still runs. It
commits whatever the child wrote to the branch. It returns the branch name in the report. The
outcome stays `Canceled` or `OutOfTurns`. `IsolationReport.kept_at` and `IsolationReport.root`
point at the kept work.

**When a commit is not safe, rho leaves the tree on disk.** It sets
`IsolationReport.left_on_disk` to the worktree path, and `kept_at` stays `None`. The summary
names the leftover tree. A human recovers it.

**Why.** A cancelled child may have done useful work. A silent delete would waste that work
and hide it. A named branch or a named tree lets the parent inspect the result. This matches
rho's rule that a child failure is a result, not a loss. See
D-worktree-keeps-a-cancelled-childs-work.

## 12. What this contract forbids

- The model may not grant isolation, and the schema offers no value that would.
- A definition may not grant isolation. Its type holds one variant, `Off`.
- `rho-core` may not depend on git.
- A git call may not run a hook, and it may not trust the worktree `.git` pointer.
- A git subprocess may not inherit a credential.
- A raw agent name may not reach a path or a git ref.
- The gate may not check a worktree artifact against the parent root.
- A child may not set its own session root, and it may not write `.git` inside it.
- An isolation failure may not fall back to the shared tree without consent.
- Cancel may not discard the child's changes. A timeout may not discard the child's changes.
- Reclaim may not delete a path git does not list as a worktree.
- A later run may not auto-delete an orphan, and orphans may not grow without a count.
- A nested repository may not be dropped in silence.

## 13. The extension point a third party uses

A third party writes one `impl rho_core::Workspace`. It installs the implementation in
`SpawnEnv.workspace`. It adds a container, a copy-on-write clone, or an overlay this way. It
edits nothing in rho. `rho-tools` ships `GitWorktreeWorkspace` as the default implementation.

## Test cases

Every test named here is for code that is not written yet. No test below exists today. Every
test uses a scripted fake provider. No test uses the network. No test uses `sleep`. A git
test uses a `tempfile` repository, never the real tree.

Grant authority, in `rho-tools`:
- `only_a_caller_grants_isolation` — a definition asking to isolate is ignored without a `Workspace`.
- `a_definition_may_refuse_isolation` — `isolation: off` keeps the child in the parent tree.
- `the_model_may_refuse_isolation` — `isolate: false` keeps the child in the parent tree.
- `a_refused_child_keeps_the_parent_policy_and_sandbox` — refusing removes no restriction.
- `an_ignored_isolation_request_is_reported` — the caller sees a warning, not a silent drop.

The session root, in `rho-core` and `rho-tools`:
- `a_child_cannot_change_the_session_root` — the existing test, and it stays true and unchanged.
- `an_isolated_child_uses_the_worktree_root` — the parent hands the child the worktree path.
- `a_confined_child_cannot_reach_the_parent_tree_through_parent_dir` — `confine` rejects the escape.
- `an_isolated_child_root_lives_outside_the_parent_root` — the worktree is a separate subtree.

The extension point, in `rho-core`:
- `rho_core_has_no_git_dependency` — a manifest guard proves the crate never links git.
- `a_third_party_workspace_needs_no_rho_edit` — a fake `Workspace` drives a spawn end to end.

Ordering with the gate, in `rho-tools`:
- `the_gate_runs_before_the_worktree_is_reclaimed` — a deleted tree would hide the artifacts.
- `the_gate_checks_the_worktree_root_not_the_parent_root` — an isolated artifact is checked in place.
- `an_artifact_written_in_the_worktree_passes_the_gate` — the child root reaches the gate context.

The persisted contract, in `rho-core`:
- `a_report_without_the_isolation_field_reads_as_not_isolated` — an older record fills `None`.
- `a_newer_report_round_trips_the_isolation_report` — every field serialises and reads back.
- `an_older_reader_drops_the_isolation_field` — an unknown field never fails the read.
- `the_isolation_report_defaults_every_optional_field` — a partial record still reads.

The hardened git call, in `rho-tools`:
- `a_rewritten_worktree_git_pointer_cannot_redirect_rho` — the recorded git directory wins, and
  the commit lands on the real branch.
- `a_child_hook_never_runs_during_reclaim` — hooks are off, proved with a hook that would write
  a file.
- `the_git_subprocess_environment_holds_no_credential` — the same scrub `bash` applies.
- `a_commit_succeeds_with_no_configured_git_identity` — rho passes its own identity.
- `a_child_cannot_write_dot_git_inside_its_root` — the write path denies it.
- `a_nested_repository_is_named_in_the_report` — its content is not kept, and the report says so.

The naming slug, in `rho-tools`:
- `an_agent_name_with_a_path_traversal_becomes_a_safe_slug` — `../../../../tmp/x` stays inside.
- `an_agent_name_with_a_leading_dash_is_not_a_git_option` — no ref injection.
- `an_unusable_name_falls_back_and_still_spawns` — the id carries the identity.

Orphans and removal, in `rho-tools`:
- `reclaim_removes_a_tree_through_git_and_not_by_deleting_a_path`
- `reclaim_refuses_a_path_git_does_not_list` — a symlink swap changes nothing.
- `too_many_orphans_refuses_a_new_worktree_and_names_the_directory`

The failure set, in `rho-tools`:
- `a_non_git_root_fails_the_isolated_spawn_as_a_result` — `NotAGitRepo`, and the parent continues.
- `a_repo_with_no_commit_fails_with_a_named_reason` — `NoCommitYet`.
- `a_taken_branch_name_fails_with_a_named_reason` — `BranchExists`.
- `a_failed_worktree_add_is_a_result_not_a_crash` — `CreateFailed`, and the parent continues.
- `an_isolation_failure_never_falls_back_to_the_shared_tree` — no silent widening.
- `a_commit_failure_after_the_child_keeps_the_tree_on_disk` — `CommitFailed`, and `isolation_root` is set.
- `a_reclaim_failure_names_the_leftover_tree` — `ReclaimFailed`, and the work is not lost.

Concurrency, in `rho-tools`:
- `two_concurrent_worktree_adds_are_serialised` — a mutex serialises the git call.
- `four_children_get_four_distinct_worktrees` — no two children share a tree or a branch.

A dirty parent tree, in `rho-tools`:
- `a_dirty_parent_tree_warns_the_parent` — the child sees `HEAD`, not the uncommitted edits.
- `a_dirty_parent_tree_sets_parent_was_dirty` — the fact is a field a caller can assert.
- `an_isolated_child_does_not_see_the_parent_uncommitted_work` — the worktree is based on `HEAD`.

The repository lock, in `rho-tools`:
- `two_sessions_on_one_repository_share_one_lock` — the lock is keyed by the git common directory.
- `two_repositories_do_not_wait_for_each_other` — a per-instance mutex would fail this.

Cleanup and orphans, in `rho-tools`:
- `a_finished_child_worktree_is_reclaimed` — reclaim runs on the normal path.
- `a_failed_child_worktree_is_reclaimed` — the drop guard runs reclaim on failure.
- `a_later_run_reports_an_orphan_and_never_deletes_it` — a crash leftover is listed, not removed.
- `the_worktree_and_branch_names_never_collide` — the agent id and the timestamp make each name unique.

Cancellation and the timeout, in `rho-tools`:
- `a_cancelled_child_keeps_its_changes_on_a_branch` — cancel commits the work and names the branch.
- `a_timed_out_child_keeps_its_changes` — a timeout commits the work and names the branch.
- `a_child_with_no_change_leaves_no_branch` — an empty tree is deleted, and `branch` is `None`.

## Out of scope

- Merging a child's branch back into the parent branch.
- Conflict resolution between two children's branches.
- A pull request, or any code-forge integration.
- Work across more than one repository.
- Rebasing a child's branch onto the parent's later state.
- Copying the parent's uncommitted edits into a worktree.
- Automatic deletion of an orphan left by a crash.
- Any non-git `Workspace` implementation shipped by rho. rho ships the trait and the git one.
- Cross-machine or cross-process isolation. A host that needs a process per child runs rho as a library.
