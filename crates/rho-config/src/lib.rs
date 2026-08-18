//! Layered configuration for rho.
//!
//! The public API is stated verbatim in `docs/specs/SPEC-13-config.md`. The bodies
//! are `todo!()` until stage T3 makes the T2 tests pass.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rho_core::{SandboxMode, Secret};
use serde::Deserialize;

/// A typed configuration error. Every failure is typed. Every failure is closed.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file exists but could not be read, for example a permission fault.
    #[error("cannot read the config file {path}: {message}")]
    Read { path: PathBuf, message: String },
    /// The file is not valid TOML, or it names an unknown key.
    #[error("cannot parse the config file {path}: {message}")]
    Parse { path: PathBuf, message: String },
    /// The user asked for a profile that no file defines.
    #[error("the profile \"{name}\" is not defined")]
    UnknownProfile { name: String },
    /// A credential source failed to resolve.
    #[error("cannot resolve the credential \"{name}\": {message}")]
    Credential { name: String, message: String },
}

/// One layer of configuration. Every field is optional. A layer states only what
/// it overrides. `serde(deny_unknown_fields)` makes an unknown key an error.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct ConfigLayer {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub session_root: Option<PathBuf>,
    pub session_file: Option<PathBuf>,
    pub ephemeral: Option<bool>,
    /// A sandbox name. It parses through `SandboxMode::from_str`.
    pub sandbox: Option<String>,
    /// An approval mode name. It parses through `ApprovalMode::from_str`.
    pub approval: Option<String>,
    pub skill_paths: Option<Vec<PathBuf>>,
    pub no_skills: Option<bool>,
    /// A path to the MCP server file. See section 6.
    pub mcp_config: Option<PathBuf>,
    pub subagents: Option<SubagentLimitsLayer>,
    /// Credential sources, by name. Each value is one string, parsed in section 5.
    pub credentials: Option<BTreeMap<String, String>>,
    /// Named profiles. A profile is a nested layer.
    #[serde(default)]
    pub profiles: BTreeMap<String, ConfigLayer>,
}

/// The subagent limits, as optional file fields. It mirrors `rho_core::SubagentLimits`,
/// which is not itself `Deserialize` and carries a `Duration`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct SubagentLimitsLayer {
    pub max_depth: Option<u32>,
    pub max_children_per_parent: Option<usize>,
    pub max_live_total: Option<usize>,
    pub child_timeout_secs: Option<u64>,
}

/// How the agent approves a tool call. It maps onto the built-in policies in
/// `rho-core`. This type is new in `rho-config`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalMode {
    /// Approve every tool call. It maps to `rho_core::AllowAllPolicy`.
    AllowAll,
    /// Deny every mutating tool call. It maps to `rho_core::ReadOnlyPolicy`.
    ReadOnly,
    /// Ask a frontend before a mutating tool call. It maps to the interactive
    /// `AskPolicy` in `SPEC-16`.
    Ask,
}

impl std::str::FromStr for ApprovalMode {
    type Err = String;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let _ = text;
        todo!()
    }
}

/// One credential source. A literal value is a `Secret` from the moment it is read.
#[derive(Clone, Debug)]
pub enum CredentialSource {
    /// A value written in the file. It is a `Secret` at once.
    Literal(Secret),
    /// The whole value of one environment variable, by name.
    Env(String),
    /// A template with `${NAME}` spans, filled from the environment.
    Interpolate(String),
    /// A command and its arguments. The trimmed standard output is the credential.
    /// `pass_env` is the only allowlist of variable names the child inherits.
    Command {
        argv: Vec<String>,
        pass_env: Vec<String>,
    },
}

/// A source of environment values. A test passes a map. Production passes the real
/// environment. So a test never reads the real environment.
pub trait EnvLookup {
    fn get(&self, name: &str) -> Option<String>;
}

/// The real process environment.
pub struct SystemEnv;

impl EnvLookup for SystemEnv {
    fn get(&self, name: &str) -> Option<String> {
        let _ = name;
        todo!()
    }
}

impl EnvLookup for BTreeMap<String, String> {
    fn get(&self, name: &str) -> Option<String> {
        let _ = name;
        todo!()
    }
}

/// The sources that feed one merge. The strongest source is last.
#[derive(Clone, Debug, Default)]
pub struct Sources {
    pub global_file: Option<PathBuf>,
    pub project_file: Option<PathBuf>,
    pub profile: Option<String>,
    /// The `RHO_*` variables, by name and value. The CLI collects them.
    pub env: Vec<(String, String)>,
    /// The command-line flags, already turned into a layer.
    pub flags: ConfigLayer,
}

/// The merged and resolved configuration for one run.
#[derive(Clone, Debug)]
pub struct Config {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub session_root: Option<PathBuf>,
    pub session_file: Option<PathBuf>,
    pub ephemeral: bool,
    pub sandbox: SandboxMode,
    /// The approval mode the user stated, or `None` when the user stated none.
    /// `None` means the frontend resolves the mode, per `SPEC-16` section 4.
    pub approval: Option<ApprovalMode>,
    pub skill_paths: Vec<PathBuf>,
    pub discover_skills: bool,
    pub mcp_config: Option<PathBuf>,
    pub subagents: rho_core::SubagentLimits,
    /// Credential sources, by name. A value resolves through `resolve_credential`.
    pub credentials: BTreeMap<String, CredentialSource>,
}

impl ConfigLayer {
    /// Merge `over` onto `self`. A value in `over` wins. `self` fills a gap.
    pub fn merge(self, over: ConfigLayer) -> ConfigLayer {
        let _ = over;
        todo!()
    }

    /// Build a layer from the `RHO_*` variables.
    pub fn from_env(vars: &[(String, String)]) -> ConfigLayer {
        let _ = vars;
        todo!()
    }
}

impl CredentialSource {
    /// Parse one credential value from the config file. See section 5.
    pub fn parse(raw: &str) -> CredentialSource {
        let _ = raw;
        todo!()
    }

    /// Resolve to a `Secret`. Never log the result. The child of a command source
    /// inherits only the `pass_env` names.
    pub fn resolve(&self, name: &str, env: &dyn EnvLookup) -> Result<Secret, ConfigError> {
        let _ = (name, env);
        todo!()
    }
}

impl Config {
    /// The built-in defaults. The weakest layer.
    pub fn defaults() -> ConfigLayer {
        todo!()
    }

    /// Read one layer from a TOML file. A missing file is `Ok(None)`. An unreadable
    /// file, a malformed file, or an unknown key is an `Err`.
    pub fn read_file(path: &Path) -> Result<Option<ConfigLayer>, ConfigError> {
        let _ = path;
        todo!()
    }

    /// Load, merge, and resolve. This is the one entry point.
    pub fn load(sources: &Sources) -> Result<Config, ConfigError> {
        let _ = sources;
        todo!()
    }

    /// Resolve one named credential to a `Secret`.
    pub fn resolve_credential(
        &self,
        name: &str,
        env: &dyn EnvLookup,
    ) -> Result<Secret, ConfigError> {
        let _ = (name, env);
        todo!()
    }
}
