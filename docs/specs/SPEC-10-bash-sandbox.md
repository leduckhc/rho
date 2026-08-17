# SPEC-10 — bash OS sandbox

Status: draft for sprint 1.
Owning crates: `rho-core` (the `SandboxMode` type and the `SessionConfig` field),
`rho-tools` (the sandbox planner and the `bash` wiring), `rho-cli` (the flag).

Feature: F-31 (bash OS sandbox).

## 1. The problem this solves

`SPEC-03` section 7 states the truth: path confinement does not apply to `bash`.
A command reaches any path with `cd` or with an absolute path. So the only real
boundary for `bash` is the approval policy, which is all or nothing.

Every comparable harness has the same hole. jcode pattern-matches dangerous
commands. Claude Code uses allow and deny patterns like `Bash(git log *)`. Both
are heuristics over a string. A shell has many routes to the same effect: a
variable, a here-document, `base64 -d`, an alias, a script file, an `$IFS` trick.
So a pattern list is not a boundary.

The operating system already has the mechanism. This spec adds a real confinement
mode for `bash`, built on the operating system, not on string matching.

## 2. The three modes

| Mode | Behaviour |
| --- | --- |
| `off` | No confinement. Today's behaviour. The default. |
| `confined` | Writes limited to the session root and the scratch directory. Reads allowed. |
| `strict` | `confined`, plus no network. |

`off` is the default on purpose. A sandbox that breaks a build is worse than no
sandbox. The default is stated in the code, not hidden. See decision D-013.

`confined` still allows reads, because a coding agent must read a toolchain that
lives outside the session root. `strict` adds network denial.

## 3. The API, verbatim

In `rho-core`:

```rust
use std::str::FromStr;

/// The OS-level confinement mode for the `bash` tool. See SPEC-10.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SandboxMode {
    /// No confinement. Today's behaviour, and the default.
    #[default]
    Off,
    /// Writes limited to the session root and the scratch directory. Reads allowed.
    Confined,
    /// `Confined`, plus no network.
    Strict,
}

impl SandboxMode {
    /// The lowercase name used on the command line and in messages.
    pub fn as_str(&self) -> &'static str;
    /// True when the mode confines writes to the session root and scratch.
    pub fn confines_writes(&self) -> bool;
    /// True when the mode denies network access.
    pub fn denies_network(&self) -> bool;
}

impl FromStr for SandboxMode {
    type Err = String;
    fn from_str(text: &str) -> Result<Self, Self::Err>;
}
```

`SessionConfig` gains one field:

```rust
pub struct SessionConfig {
    pub model: String,
    pub session_root: PathBuf,
    pub approval: Arc<dyn ApprovalPolicy>,
    pub max_turns: u32,
    /// The `bash` confinement mode. `SessionConfig::new` sets `Off`, so the
    /// default is stated, not hidden. Use `with_sandbox` to change it.
    pub sandbox: SandboxMode,
}

impl SessionConfig {
    /// Set the sandbox mode. `new` leaves it `Off`.
    pub fn with_sandbox(self, sandbox: SandboxMode) -> Self;
}
```

In `rho-tools`:

```rust
use rho_core::SandboxMode;
use std::path::Path;

/// A confinement backend the host can use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// macOS `sandbox-exec` with a generated profile.
    SandboxExec,
    /// Linux `bwrap` (bubblewrap), which needs no privilege.
    Bwrap,
}

/// The program and arguments to spawn for a command.
pub struct CommandPlan {
    pub program: String,
    pub args: Vec<String>,
}

/// Returned when confinement is requested but no backend is available.
#[derive(Debug)]
pub struct SandboxUnavailable {
    pub mode: SandboxMode,
}

/// Detect a confinement backend on this host. `None` means none is available.
pub fn detect_backend() -> Option<Backend>;

/// Build the run plan for a command. Fail closed: when `mode` needs confinement
/// and no backend is available, return `Err`. Never run unconfined in that case.
pub fn plan(
    mode: SandboxMode,
    root: &Path,
    scratch: Option<&Path>,
    command: &str,
) -> Result<CommandPlan, SandboxUnavailable>;
```

`BashTool` gains a builder:

```rust
impl BashTool {
    /// Set the confinement mode. The default is `SandboxMode::Off`.
    pub fn sandbox(self, mode: SandboxMode) -> Self;
}
```

Registry builders that carry the mode:

```rust
pub fn builtin_registry_with_tasks_and_sandbox(
    tasks: Arc<TaskRegistry>,
    sandbox: SandboxMode,
) -> ToolRegistry;

pub fn builtin_tools_with_tasks_and_sandbox(
    tasks: Arc<TaskRegistry>,
    sandbox: SandboxMode,
) -> Vec<Arc<dyn Tool>>;
```

The existing `builtin_registry_with_tasks` and `builtin_tools_with_tasks` stay,
and they delegate with `SandboxMode::Off`, so no existing caller changes.

CLI flag:

```
--sandbox <off|confined|strict>   Default: off.
```

## 4. How each backend works

### macOS: `sandbox-exec`

The planner generates a Sandbox Profile Language (SBPL) profile and runs
`sandbox-exec -p <profile> sh -c <command>`. The profile:

```
(version 1)
(allow default)
(deny file-write*)
(allow file-write*
    (subpath "<canonical session root>")
    (subpath "<canonical scratch dir>")
    (subpath "/dev"))
```

`strict` appends `(deny network*)`.

The planner canonicalises the root and the scratch directory first. On macOS the
temporary directory lives under `/var/folders`, and `/var` is a symlink to
`/private/var`. A profile built on the uncanonical path would not match the real
write path, so the canonical form is required.

`sandbox-exec` is deprecated by Apple. It is still present and still works on the
current macOS. The whole macOS path is behind one function, `macos_plan`, so a
replacement is a small change.

### Linux: `bwrap` (bubblewrap)

The planner runs `bwrap` with the system bound read-only, the session root and
the scratch directory bound read-write, a private `/tmp`, and a `/dev` and
`/proc`. `strict` adds `--unshare-net`. `bwrap` needs no privilege.

### Neither available: fail closed

When the mode needs confinement and no backend is available, `plan` returns
`SandboxUnavailable`. `bash` turns that into a `ToolError::Io` whose message names
the mode and says what to do. `bash` never runs the command unconfined. That is
the whole point.

## 5. Measured overhead

Measured on macOS 25.5.0 (arm64), 20 runs of `sh -c true` each:

| Path | Average per call |
| --- | --- |
| plain `sh -c` | ~3.8 ms |
| `sandbox-exec -p ... sh -c` | ~8.5 ms |

So `sandbox-exec` adds about 4.7 ms per `bash` call on this machine. The command
producing the number is in `docs/verification/`. The overhead is small against a
build or a test run, but it is real, and it is why `off` stays the default until a
later measurement on more machines.

## 6. What this sandbox does not stop

State this plainly, because a guard that oversells itself is worse than none.

- **`confined` allows reads.** A command can read any file the user can read,
  including `~/.aws/credentials` or a shell profile. So exfiltration through a
  read plus a network call is still possible in `confined`. Use `strict` to deny
  the network half of that path.
- **`strict` denies the network, not the read.** A `strict` command still reads
  a secret file. It cannot send it over a socket, but it can still write it into
  the session root, where a later un-`strict` command could send it.
- **The sandbox does not replace the approval policy.** A read-only policy still
  denies `bash` outright. The sandbox narrows what an approved `bash` call can do.
  It is defence in depth for an approved command, not a substitute for approval.
- **`sandbox-exec` is deprecated.** Apple may remove it. The macOS path is behind
  one function so a replacement is small, but the risk is real and named here.
- **The Linux `bwrap` path is not verified in this environment.** The
  implementation was written and reviewed on macOS, where `bwrap` does not run.
  The macOS `sandbox-exec` path is verified with live tests. See the honesty note
  in the developer report.
- **Unprivileged `unshare` is not a backend.** A robust write confinement with
  `unshare` needs a mount-namespace helper. A partial check would read as a
  boundary while a single path walked through it, which decision D-021 forbids. So
  a host with only `unshare` fails closed. This is a deliberate gap, not a bug.

## 7. Named tests, with the assertion each proves

In `crates/rho-core/src/sandbox.rs`:
- `sandbox_mode_default_is_off` — `SandboxMode::default()` is `Off`. Guards the
  stated default.
- `sandbox_mode_parses_each_name` — `"off"`, `"confined"`, `"strict"` parse.
- `sandbox_mode_rejects_an_unknown_name` — an unknown name returns `Err`.

In `crates/rho-tools/src/sandbox.rs` (unit tests, no binary needed):
- `plan_off_runs_sh_directly` — `Off` returns `sh -c <command>`, unchanged.
- `confinement_fails_closed_when_no_sandbox_is_available` — `wrap(None, Confined,
  ..)` returns `Err`, so a missing backend never runs unconfined.
- `a_sandbox_failure_message_names_the_mode_and_what_to_do` — the
  `SandboxUnavailable` message contains the mode name and an instruction.

In `crates/rho-tools/tests/sandbox.rs` (need a real backend; skip and print when
absent):
- `sandbox_off_runs_a_command_unchanged` — `Off` runs a command and returns its
  output. Needs no backend.
- `confined_allows_a_write_inside_the_session_root` — a write under the root
  succeeds and the file exists.
- `confined_refuses_a_write_outside_the_session_root` — a write to
  `$HOME/rho-sandbox-probe-*` is refused. Asserts the file does not exist, not the
  error text.
- `confined_refuses_a_write_even_after_cd` — `cd / && touch /rho-probe-*` is
  refused. Asserts the file does not exist.
- `confined_refuses_an_absolute_path_write` — an absolute path outside the root is
  refused. Asserts the file does not exist.
- `confined_still_allows_reading_a_system_file` — a read of `/etc/hosts` succeeds.
- `strict_refuses_a_network_call` — a connection to a local listener succeeds
  under `confined` and fails under `strict`. The listener is on `127.0.0.1`, so no
  external network is used.

In `crates/rho-cli/src/cli.rs`:
- `sandbox_flag_defaults_to_off` — with no flag, the config mode is `Off`.
- `sandbox_flag_sets_confined` — `--sandbox confined` sets the config mode.

## 8. Out of scope

- An `unshare`-based backend. Recorded as a deliberate gap in section 6.
- A per-command or per-path allow list finer than the session root. That is the
  `rho-guard` extension, F-140.
- A network allow list. `strict` is all or nothing.
- A read confinement. `confined` and `strict` both allow reads on purpose.
- A Windows backend. rho targets Unix hosts for the sandbox in sprint 1.
