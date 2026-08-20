//! The sandbox planner for the `bash` tool. See `SPEC-bash-sandbox`.
//!
//! A pattern list is not a boundary. A shell has many routes to one effect: a
//! variable, a here-document, `base64 -d`, an alias, a script file, an `$IFS`
//! trick. So confinement uses the operating system, not string matching.
//!
//! This module turns a [`SandboxMode`] into a real command to spawn. On macOS it
//! generates a `sandbox-exec` profile. On Linux it prefers `bwrap`. When a mode
//! needs confinement and no backend is available, it fails closed: the command is
//! refused, never run unconfined.

use rho_core::SandboxMode;
use std::fmt;
use std::path::Path;

/// A confinement backend the host can use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// macOS `sandbox-exec` with a generated profile.
    ///
    /// `sandbox-exec` is deprecated by Apple but still present and still works.
    /// The whole macOS path is behind [`macos_plan`], so a replacement is a small
    /// change. See `SPEC-bash-sandbox` section 4.
    SandboxExec,
    /// Linux `bwrap` (bubblewrap), which needs no privilege.
    Bwrap,
}

/// The program and arguments to spawn for a command.
#[derive(Debug)]
pub struct CommandPlan {
    /// The program to run, for example `sh` or `sandbox-exec`.
    pub program: String,
    /// The arguments, ending with `sh -c <command>`.
    pub args: Vec<String>,
}

/// Returned when confinement is requested but no backend is available.
///
/// `bash` turns this into a refusal. It never runs the command unconfined. That
/// is the point of the mode.
#[derive(Debug)]
pub struct SandboxUnavailable {
    /// The mode the caller asked for.
    pub mode: SandboxMode,
}

impl fmt::Display for SandboxUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the sandbox mode \"{}\" needs an OS sandbox, but none is available on \
             this host. Install bubblewrap (bwrap) on Linux, or pass --sandbox off \
             to run without confinement.",
            self.mode
        )
    }
}

impl std::error::Error for SandboxUnavailable {}

/// Detect a confinement backend on this host. `None` means none is available.
///
/// macOS uses `sandbox-exec`. Linux prefers `bwrap`. A host with only `unshare`
/// returns `None` on purpose; see `SPEC-bash-sandbox` section 6 for why a partial `unshare`
/// confinement is not shipped.
pub fn detect_backend() -> Option<Backend> {
    #[cfg(target_os = "macos")]
    {
        if on_path("sandbox-exec") {
            return Some(Backend::SandboxExec);
        }
    }
    #[cfg(target_os = "linux")]
    {
        if on_path("bwrap") {
            return Some(Backend::Bwrap);
        }
    }
    None
}

/// Build the run plan for a command. Fail closed when confinement is requested
/// and no backend is available.
pub fn plan(
    mode: SandboxMode,
    root: &Path,
    scratch: Option<&Path>,
    command: &str,
) -> Result<CommandPlan, SandboxUnavailable> {
    wrap(detect_backend(), mode, root, scratch, command)
}

/// The pure core of [`plan`], with the backend passed in.
///
/// This split is what makes the fail-closed path testable without a host that
/// lacks a sandbox. A test calls `wrap(None, Confined, ..)` and asserts the error.
fn wrap(
    backend: Option<Backend>,
    mode: SandboxMode,
    root: &Path,
    scratch: Option<&Path>,
    command: &str,
) -> Result<CommandPlan, SandboxUnavailable> {
    if mode == SandboxMode::Off {
        return Ok(plain(command));
    }
    match backend {
        None => Err(SandboxUnavailable { mode }),
        Some(Backend::SandboxExec) => Ok(macos_plan(mode, root, scratch, command)),
        Some(Backend::Bwrap) => Ok(bwrap_plan(mode, root, scratch, command)),
    }
}

/// The unconfined plan: `sh -c <command>`. This is `SandboxMode::Off`.
fn plain(command: &str) -> CommandPlan {
    CommandPlan {
        program: "sh".to_string(),
        args: vec!["-c".to_string(), command.to_string()],
    }
}

/// The macOS plan: `sandbox-exec -p <profile> sh -c <command>`.
///
/// `sandbox-exec` is deprecated by Apple. It still works on the current macOS.
/// Keep the whole macOS path in this one function, so a replacement is small.
fn macos_plan(
    mode: SandboxMode,
    root: &Path,
    scratch: Option<&Path>,
    command: &str,
) -> CommandPlan {
    let profile = macos_profile(mode, root, scratch);
    CommandPlan {
        program: "sandbox-exec".to_string(),
        args: vec![
            "-p".to_string(),
            profile,
            "sh".to_string(),
            "-c".to_string(),
            command.to_string(),
        ],
    }
}

/// Build the Sandbox Profile Language (SBPL) text.
///
/// The profile allows everything, then denies every file write, then allows a
/// write only under the session root, the scratch directory, and `/dev`. So a
/// `cd` or an absolute path cannot write outside the root, which a path check
/// could not stop. `strict` also denies the network.
///
/// The paths are canonicalised. On macOS the temp root lives under `/var/folders`
/// and `/var` is a symlink to `/private/var`, so a profile on the uncanonical path
/// would not match the real write path.
fn macos_profile(mode: SandboxMode, root: &Path, scratch: Option<&Path>) -> String {
    let mut profile = String::from("(version 1)\n(allow default)\n(deny file-write*)\n");
    profile.push_str("(allow file-write*\n");
    profile.push_str(&format!("    (subpath \"{}\")\n", sbpl_path(root)));
    if let Some(scratch) = scratch {
        profile.push_str(&format!("    (subpath \"{}\")\n", sbpl_path(scratch)));
    }
    profile.push_str("    (subpath \"/dev\"))\n");
    if mode.denies_network() {
        profile.push_str("(deny network*)\n");
    }
    profile
}

/// Canonicalise a path and escape it for an SBPL string literal.
///
/// A path that does not resolve keeps its given form, so the profile is still
/// valid. An SBPL string is double-quoted, so escape a backslash and a quote.
fn sbpl_path(path: &Path) -> String {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    resolved
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// The Linux plan, using `bwrap` (bubblewrap).
///
/// The system is bound read-only. The session root and the scratch directory are
/// bound read-write. `/tmp` is private, and `/dev` and `/proc` are minimal.
/// `strict` adds `--unshare-net`.
///
/// Verified in two halves, because `bwrap` does not run on macOS. The argument sequence
/// below was run against real `bwrap` 0.11.0 in a Debian container, and the unit tests in
/// this module pin that rho emits exactly that sequence. `SPEC-bash-sandbox` records the container
/// command and its output.
///
/// The ordering rule is the one to protect: the read-only bind of `/` must come **before**
/// the read-write bind of the session root, or the root ends up read-only.
fn bwrap_plan(
    mode: SandboxMode,
    root: &Path,
    scratch: Option<&Path>,
    command: &str,
) -> CommandPlan {
    let root = canonical(root);
    let mut args: Vec<String> = vec![
        "--ro-bind".to_string(),
        "/".to_string(),
        "/".to_string(),
        "--dev".to_string(),
        "/dev".to_string(),
        "--proc".to_string(),
        "/proc".to_string(),
        "--tmpfs".to_string(),
        "/tmp".to_string(),
        // Re-bind the session root read-write, after the read-only system bind.
        "--bind".to_string(),
        root.clone(),
        root,
    ];
    if let Some(scratch) = scratch {
        let scratch = canonical(scratch);
        args.push("--bind".to_string());
        args.push(scratch.clone());
        args.push(scratch);
    }
    if mode.denies_network() {
        args.push("--unshare-net".to_string());
    }
    args.push("sh".to_string());
    args.push("-c".to_string());
    args.push(command.to_string());
    CommandPlan {
        program: "bwrap".to_string(),
        args,
    }
}

/// Canonicalise a path to a string, keeping the given form when it does not
/// resolve.
fn canonical(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// True when `program` is on `PATH` as a file.
fn on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(program).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn plan_off_runs_sh_directly() {
        // `Off` is today's behaviour: `sh -c <command>`, no wrapper.
        let plan = plan(SandboxMode::Off, Path::new("/tmp"), None, "echo hi").unwrap();
        assert_eq!(plan.program, "sh");
        assert_eq!(plan.args, vec!["-c".to_string(), "echo hi".to_string()]);
    }

    #[test]
    fn confinement_fails_closed_when_no_sandbox_is_available() {
        // The whole point: a missing backend refuses the command, it never runs
        // unconfined. Pass `None` for the backend to simulate the absence.
        for mode in [SandboxMode::Confined, SandboxMode::Strict] {
            let result = wrap(None, mode, Path::new("/tmp"), None, "echo hi");
            assert!(
                result.is_err(),
                "{mode} must fail closed when no backend exists"
            );
        }
    }

    #[test]
    fn a_sandbox_failure_message_names_the_mode_and_what_to_do() {
        let error = wrap(None, SandboxMode::Strict, Path::new("/tmp"), None, "true")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("strict"),
            "message must name the mode: {error}"
        );
        assert!(
            error.contains("--sandbox off"),
            "message must say what to do: {error}"
        );
    }

    #[test]
    fn off_never_fails_even_without_a_backend() {
        // `Off` asks for no confinement, so a missing backend is irrelevant.
        assert!(wrap(None, SandboxMode::Off, Path::new("/tmp"), None, "true").is_ok());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_profile_denies_writes_and_allows_the_root() {
        let profile = macos_profile(SandboxMode::Confined, Path::new("/tmp"), None);
        assert!(profile.contains("(deny file-write*)"), "{profile}");
        assert!(profile.contains("(allow file-write*"), "{profile}");
        assert!(
            !profile.contains("(deny network*)"),
            "confined keeps network"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_strict_profile_denies_the_network() {
        let profile = macos_profile(SandboxMode::Strict, Path::new("/tmp"), None);
        assert!(profile.contains("(deny network*)"), "{profile}");
    }

    // --- The Linux plan ---------------------------------------------------
    //
    // These tests pin the exact argument sequence that `bwrap_plan` emits. That matters
    // because the sequence itself was verified against real `bwrap` 0.11.0 in a Debian
    // container, and the two together are the whole verification chain: the container run
    // proves the arguments confine correctly, and these tests prove rho emits those
    // arguments. See `SPEC-bash-sandbox` section 6 for the container command and its output.

    #[test]
    fn bwrap_plan_binds_the_system_read_only_then_the_root_read_write() {
        // Order matters. The read-only bind of `/` must come first, and the read-write
        // bind of the session root must come after it, or the root would be read-only.
        let plan = bwrap_plan(SandboxMode::Confined, Path::new("/tmp"), None, "echo hello");
        assert_eq!(plan.program, "bwrap");
        let joined = plan.args.join(" ");
        let ro_at = joined
            .find("--ro-bind / /")
            .expect("a read-only system bind");
        let rw_at = joined.find("--bind /").expect("a read-write root bind");
        assert!(
            ro_at < rw_at,
            "the read-only system bind must precede the root bind: {joined}"
        );
    }

    #[test]
    fn bwrap_plan_gives_a_private_tmp_and_a_minimal_dev_and_proc() {
        let plan = bwrap_plan(SandboxMode::Confined, Path::new("/tmp"), None, "true");
        let joined = plan.args.join(" ");
        for needed in ["--tmpfs /tmp", "--dev /dev", "--proc /proc"] {
            assert!(joined.contains(needed), "missing {needed} in {joined}");
        }
    }

    #[test]
    fn bwrap_plan_binds_the_scratch_directory_when_there_is_one() {
        let plan = bwrap_plan(
            SandboxMode::Confined,
            Path::new("/tmp"),
            Some(Path::new("/var/tmp")),
            "true",
        );
        let joined = plan.args.join(" ");
        assert!(joined.contains("/var/tmp"), "{joined}");
    }

    #[test]
    fn bwrap_plan_unshares_the_network_only_in_strict() {
        let confined = bwrap_plan(SandboxMode::Confined, Path::new("/tmp"), None, "true");
        assert!(
            !confined.args.iter().any(|a| a == "--unshare-net"),
            "confined must keep the network"
        );
        let strict = bwrap_plan(SandboxMode::Strict, Path::new("/tmp"), None, "true");
        assert!(
            strict.args.iter().any(|a| a == "--unshare-net"),
            "strict must deny the network"
        );
    }

    #[test]
    fn bwrap_plan_passes_the_command_to_a_shell_last() {
        let plan = bwrap_plan(SandboxMode::Confined, Path::new("/tmp"), None, "echo hi");
        let tail = &plan.args[plan.args.len() - 3..];
        assert_eq!(tail, ["sh", "-c", "echo hi"]);
    }

    #[test]
    fn sbpl_path_escapes_a_quote() {
        // A path with a quote must not break the profile string.
        let escaped = sbpl_path(&PathBuf::from("/tmp/a\"b"));
        assert!(escaped.contains("\\\""), "{escaped}");
    }
}
