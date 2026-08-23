//! Layered configuration for rho.
//!
//! The public API is stated verbatim in `docs/specs/20260818-014343-SPEC-config.md`. This crate
//! reads configuration from files, the environment, and the command line. It merges
//! those sources in one fixed order, and it resolves a credential to a `Secret`.
//!
//! The crate fails closed. A malformed file, an unknown key, or an unreadable file
//! returns a typed error. It never falls back to a default that grants more access.

use std::collections::BTreeMap;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::time::{Duration, Instant};

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
    /// A merged value is not valid. The merge keeps a winning value and drops which layer
    /// held it, so this error names the key and the value, and never a file.
    ///
    /// It exists because `Parse` needs a path, and four parsers passed the literal "the
    /// merged configuration" as one. That printed "cannot parse the config file the merged
    /// configuration", which a live run found twice. See
    /// `D-the-merge-cannot-name-a-values-source`.
    #[error("the {key} value \"{value}\" is not valid: {message}")]
    Value {
        key: &'static str,
        value: String,
        message: String,
    },
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
    /// Whether the TUI captures the mouse. Off by default, so the terminal keeps
    /// drag-select and its own wheel. See `D-native-selection-is-the-default`.
    pub tui_mouse: Option<bool>,
    /// How the TUI draws reasoning. It parses through `ReasoningDisplay::from_str`.
    /// Values are `off`, `summary`, `full`, and `live`. Default is `summary`.
    pub tui_reasoning: Option<String>,
    /// How hard the model should think. It parses through `ReasoningEffort::from_str`.
    /// Values are `off`, `low`, `medium`, `high`, and `xhigh`. Unset means the provider's
    /// own default, so rho sends no field. See `SPEC-reasoning-across-providers` section 9.
    pub reasoning_effort: Option<String>,
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
    /// `AskPolicy` in `SPEC-approval`.
    Ask,
}

impl FromStr for ApprovalMode {
    type Err = String;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "allow-all" => Ok(ApprovalMode::AllowAll),
            "read-only" => Ok(ApprovalMode::ReadOnly),
            "ask" => Ok(ApprovalMode::Ask),
            other => Err(format!(
                "unknown approval mode \"{other}\": the valid names are \
                 read-only, ask, and allow-all"
            )),
        }
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
    /// A credential the project file asked to run as a command, without trust.
    ///
    /// It is a variant and not a dropped value, because dropping it would hand the
    /// provider an empty key and a 401, which reads as a broken account rather than a
    /// refusal. It fails when it is resolved, and the message names `--trust-project`.
    /// See `SPEC-config-call-site` section 5.
    RefusedProjectCommand { path: PathBuf },
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
        std::env::var(name).ok()
    }
}

impl EnvLookup for BTreeMap<String, String> {
    fn get(&self, name: &str) -> Option<String> {
        self.get(name).cloned()
    }
}

/// Where rho looks for its two config files.
///
/// A path is returned whether or not the file exists, because discovery is pure and
/// `Config::read_file` already answers `Ok(None)` for a file that is not there.
/// See `SPEC-config-call-site` section 2.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigPaths {
    /// `$XDG_CONFIG_HOME/rho/config.toml`, else `$HOME/.config/rho/config.toml`.
    /// `None` when neither variable is set. Discovery does not fail, and the caller
    /// reports the absence, because a lost global file loses a hardened setting.
    pub global: Option<PathBuf>,
    /// `<bootstrap_root>/.rho/config.toml`.
    pub project: Option<PathBuf>,
}

impl ConfigPaths {
    /// Discover both paths. `env` supplies `XDG_CONFIG_HOME` and `HOME`, so a test never
    /// reads the real home directory.
    pub fn discover(env: &dyn EnvLookup, bootstrap_root: &Path) -> ConfigPaths {
        // `XDG_CONFIG_HOME` is the stated override, so it wins. An empty value counts as
        // unset, because an exported-but-empty variable is a common shell accident and
        // `/rho/config.toml` at the filesystem root is never what the user meant.
        let global = non_empty(env.get("XDG_CONFIG_HOME"))
            .map(|base| PathBuf::from(base).join("rho").join("config.toml"))
            .or_else(|| {
                non_empty(env.get("HOME")).map(|home| {
                    PathBuf::from(home)
                        .join(".config")
                        .join("rho")
                        .join("config.toml")
                })
            });
        ConfigPaths {
            global,
            project: Some(bootstrap_root.join(".rho").join("config.toml")),
        }
    }
}

/// Treat an empty environment value as unset. An exported-but-empty `HOME` is a common
/// shell accident, and joining from `""` would name a path at the filesystem root.
fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty())
}

/// Whether the user trusts the project file's powerful keys. `--trust-project` sets it.
///
/// The default is `Untrusted`, because a project file arrives with a clone. This reuses
/// the flag and the reason of `D-project-skill-needs-trust`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProjectTrust {
    #[default]
    Untrusted,
    Trusted,
}

/// The sources that feed one merge. The strongest source is last.
///
/// The fields are `pub(crate)` on purpose. They were `pub`, and
/// `Sources { ..Default::default() }` then walked around any constructor rule written as
/// prose. A rule the compiler does not hold is a comment. Build one through
/// `Sources::from_paths`, then add one source per call.
#[derive(Clone, Debug, Default)]
pub struct Sources {
    pub(crate) global_file: Option<PathBuf>,
    pub(crate) project_file: Option<PathBuf>,
    pub(crate) profile: Option<String>,
    /// The `RHO_*` variables, by name and value. The CLI collects them.
    pub(crate) env: Vec<(String, String)>,
    /// The command-line flags, already turned into a layer.
    pub(crate) flags: ConfigLayer,
    /// Whether the project file's powerful keys are trusted.
    pub(crate) project_trust: ProjectTrust,
}

impl Sources {
    /// Start from the discovered paths. Each later call adds one source, so a new source
    /// is a new method and never a longer argument list. See
    /// `D-no-four-argument-session-new`.
    pub fn from_paths(paths: ConfigPaths) -> Sources {
        Sources {
            global_file: paths.global,
            project_file: paths.project,
            profile: None,
            env: Vec::new(),
            flags: ConfigLayer::default(),
            project_trust: ProjectTrust::default(),
        }
    }

    /// Add the `RHO_*` variables, by name and value.
    pub fn with_env(mut self, env: Vec<(String, String)>) -> Sources {
        self.env = env;
        self
    }

    /// Add the profile the user named, if any.
    pub fn with_profile(mut self, profile: Option<String>) -> Sources {
        self.profile = profile;
        self
    }

    /// Add the command-line flags, already turned into a layer.
    pub fn with_flags(mut self, flags: ConfigLayer) -> Sources {
        self.flags = flags;
        self
    }

    /// State whether the project file's powerful keys are trusted.
    pub fn with_project_trust(mut self, trust: ProjectTrust) -> Sources {
        self.project_trust = trust;
        self
    }
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
    /// `None` means the frontend resolves the mode, per `SPEC-approval` section 4.
    pub approval: Option<ApprovalMode>,
    pub skill_paths: Vec<PathBuf>,
    pub discover_skills: bool,
    /// Whether the TUI captures the mouse. False by default.
    pub tui_mouse: bool,
    /// How the TUI draws reasoning. `Summary` by default.
    pub reasoning: rho_core::ReasoningDisplay,
    /// How hard the model should think. `None` means the provider's own default, so rho
    /// sends no field at all. See `SPEC-reasoning-across-providers` section 9.
    pub reasoning_effort: Option<rho_core::ReasoningEffort>,
    pub mcp_config: Option<PathBuf>,
    pub subagents: rho_core::SubagentLimits,
    /// Credential sources, by name. A value resolves through `resolve_credential`.
    pub credentials: BTreeMap<String, CredentialSource>,
}

impl ConfigLayer {
    /// Merge `over` onto `self`. A value in `over` wins. `self` fills a gap.
    pub fn merge(mut self, over: ConfigLayer) -> ConfigLayer {
        self.provider = over.provider.or(self.provider);
        self.model = over.model.or(self.model);
        self.session_root = over.session_root.or(self.session_root);
        self.session_file = over.session_file.or(self.session_file);
        self.ephemeral = over.ephemeral.or(self.ephemeral);
        self.sandbox = over.sandbox.or(self.sandbox);
        self.approval = over.approval.or(self.approval);
        self.skill_paths = over.skill_paths.or(self.skill_paths);
        self.no_skills = over.no_skills.or(self.no_skills);
        self.tui_mouse = over.tui_mouse.or(self.tui_mouse);
        self.tui_reasoning = over.tui_reasoning.or(self.tui_reasoning);
        self.reasoning_effort = over.reasoning_effort.or(self.reasoning_effort);
        self.mcp_config = over.mcp_config.or(self.mcp_config);
        self.subagents = over.subagents.or(self.subagents);
        self.credentials = over.credentials.or(self.credentials);
        // A profile is a named block, not a merged value. Keep the union, so a
        // profile defined in either file is reachable by name.
        self.profiles.extend(over.profiles);
        self
    }

    /// Build a layer from the `RHO_*` variables. This maps every scalar config key,
    /// per SPEC-config section 2 layer 5 and F-environment-variable-override. It is infallible: an unaccepted boolean
    /// value is omitted here, and `Config::load` is the fail-closed authority that
    /// rejects it. The two security keys (`sandbox`, `approval`) pass through as
    /// strings, and the merged-layer parser fails closed on a bad value.
    pub fn from_env(vars: &[(String, String)]) -> ConfigLayer {
        let mut layer = ConfigLayer::default();
        for (name, value) in vars {
            match name.as_str() {
                "RHO_PROVIDER" => layer.provider = Some(value.clone()),
                "RHO_MODEL" => layer.model = Some(value.clone()),
                "RHO_SESSION_ROOT" => layer.session_root = Some(PathBuf::from(value)),
                "RHO_SESSION_FILE" => layer.session_file = Some(PathBuf::from(value)),
                "RHO_EPHEMERAL" => layer.ephemeral = parse_env_bool("ephemeral", value).ok(),
                "RHO_SANDBOX" => layer.sandbox = Some(value.clone()),
                "RHO_APPROVAL" => layer.approval = Some(value.clone()),
                "RHO_SKILL_PATHS" => {
                    layer.skill_paths = Some(std::env::split_paths(value).collect())
                }
                "RHO_NO_SKILLS" => layer.no_skills = parse_env_bool("no-skills", value).ok(),
                "RHO_TUI_MOUSE" => layer.tui_mouse = parse_env_bool("tui-mouse", value).ok(),
                "RHO_TUI_REASONING" => layer.tui_reasoning = Some(value.clone()),
                "RHO_REASONING_EFFORT" => layer.reasoning_effort = Some(value.clone()),
                "RHO_MCP_CONFIG" => layer.mcp_config = Some(PathBuf::from(value)),
                _ => {}
            }
        }
        layer
    }
}

/// Parse a boolean from an environment value. It accepts `1`, `true`, and `yes` as
/// true, and `0`, `false`, and `no` as false. The match trims space and ignores case.
/// Any other value fails closed, so an unknown string never reads as false.
fn parse_env_bool(key: &str, value: &str) -> Result<bool, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" => Ok(true),
        "0" | "false" | "no" => Ok(false),
        _ => Err(ConfigError::Parse {
            path: PathBuf::from(ENV_SOURCE_LABEL),
            message: format!(
                "the {key} key value \"{value}\" is not a boolean: \
                 use 1, true, yes, 0, false, or no"
            ),
        }),
    }
}

/// Fail closed on an unaccepted boolean in the environment layer. `from_env` omits
/// such a value, so this guard runs before the merge to keep the run from a soft
/// default. It names the key and the value.
fn validate_env_booleans(vars: &[(String, String)]) -> Result<(), ConfigError> {
    for (name, value) in vars {
        match name.as_str() {
            "RHO_EPHEMERAL" => {
                parse_env_bool("ephemeral", value)?;
            }
            "RHO_NO_SKILLS" => {
                parse_env_bool("no-skills", value)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Replace each `${NAME}` span in `template` with the environment value. A span with
/// no value fails closed.
fn interpolate(template: &str, name: &str, env: &dyn EnvLookup) -> Result<String, ConfigError> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after.find('}').ok_or_else(|| ConfigError::Credential {
            name: name.to_string(),
            message: "an interpolation span has no closing brace".to_string(),
        })?;
        let var = &after[..end];
        let value = env.get(var).ok_or_else(|| ConfigError::Credential {
            name: name.to_string(),
            message: format!("the environment variable \"{var}\" is not set"),
        })?;
        out.push_str(&value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

impl CredentialSource {
    /// Parse one credential value from the config file. See section 5.
    pub fn parse(raw: &str) -> CredentialSource {
        if let Some(var) = raw.strip_prefix("env:") {
            return CredentialSource::Env(var.to_string());
        }
        if let Some(command) = raw.strip_prefix('!') {
            let argv = command.split_whitespace().map(|s| s.to_string()).collect();
            return CredentialSource::Command {
                argv,
                pass_env: Vec::new(),
            };
        }
        if raw.contains("${") {
            return CredentialSource::Interpolate(raw.to_string());
        }
        CredentialSource::Literal(Secret::new(raw))
    }

    /// Resolve to a `Secret`. Never log the result. The child of a command source
    /// inherits only the `pass_env` names.
    pub fn resolve(&self, name: &str, env: &dyn EnvLookup) -> Result<Secret, ConfigError> {
        self.resolve_with_timeout(name, env, DEFAULT_CREDENTIAL_TIMEOUT)
    }

    /// Resolve to a `Secret` with an explicit command timeout. A hung command source
    /// fails after `timeout`. Other sources ignore the timeout.
    pub fn resolve_with_timeout(
        &self,
        name: &str,
        env: &dyn EnvLookup,
        timeout: Duration,
    ) -> Result<Secret, ConfigError> {
        match self {
            CredentialSource::Literal(secret) => Ok(secret.clone()),
            CredentialSource::RefusedProjectCommand { path } => Err(ConfigError::Credential {
                name: name.to_string(),
                message: format!(
                    "the project file {} asks to run a command for this credential, and \
                     the project is not trusted. Pass --trust-project to allow it.",
                    path.display()
                ),
            }),
            CredentialSource::Env(var) => {
                let value = env.get(var).ok_or_else(|| ConfigError::Credential {
                    name: name.to_string(),
                    message: format!("the environment variable \"{var}\" is not set"),
                })?;
                Ok(Secret::new(value))
            }
            CredentialSource::Interpolate(template) => {
                Ok(Secret::new(interpolate(template, name, env)?))
            }
            CredentialSource::Command { argv, pass_env } => {
                resolve_command(name, argv, pass_env, env, timeout)
            }
        }
    }
}

/// The default timeout for a credential command. A hung helper fails after this.
const DEFAULT_CREDENTIAL_TIMEOUT: Duration = Duration::from_secs(30);

/// The poll interval while a credential command runs. It bounds the wait latency
/// without a busy loop.
const CREDENTIAL_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// The source label for an environment parse error. It names the layer, not a file.
const ENV_SOURCE_LABEL: &str = "the environment";

/// Run a credential command with a cleared environment. The child inherits only
/// `PATH`, `HOME`, and the names in `pass_env`, per decision D-credential-command-allowlist. The child never
/// inherits the whole process environment, so a variable the allowlist does not name
/// cannot reach the helper.
fn resolve_command(
    name: &str,
    argv: &[String],
    pass_env: &[String],
    env: &dyn EnvLookup,
    timeout: Duration,
) -> Result<Secret, ConfigError> {
    let (program, args) = argv.split_first().ok_or_else(|| ConfigError::Credential {
        name: name.to_string(),
        message: "the credential command is empty".to_string(),
    })?;

    let mut command = Command::new(program);
    command.args(args);
    command.env_clear();
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());

    // The base names a command needs to run, plus the explicit allowlist.
    for var in ["PATH", "HOME"]
        .iter()
        .map(|s| s.to_string())
        .chain(pass_env.iter().cloned())
    {
        if let Some(value) = env.get(&var) {
            command.env(&var, value);
        }
    }

    let mut child = command.spawn().map_err(|error| ConfigError::Credential {
        name: name.to_string(),
        message: format!("the credential command did not run: {error}"),
    })?;

    // Wait for the child, but never longer than the timeout. A hung helper is killed
    // and reaped, so it leaks no process and leaves no zombie. See SPEC-config section 5.
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ConfigError::Credential {
                        name: name.to_string(),
                        message: format!("the credential command timed out after {timeout:?}"),
                    });
                }
                std::thread::sleep(CREDENTIAL_POLL_INTERVAL);
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ConfigError::Credential {
                    name: name.to_string(),
                    message: format!("the credential command could not be waited on: {error}"),
                });
            }
        }
    };
    if !status.success() {
        return Err(ConfigError::Credential {
            name: name.to_string(),
            message: format!("the credential command failed with status {status}"),
        });
    }
    let mut stdout = String::new();
    if let Some(mut handle) = child.stdout.take() {
        handle
            .read_to_string(&mut stdout)
            .map_err(|_| ConfigError::Credential {
                name: name.to_string(),
                message: "the credential command wrote invalid UTF-8".to_string(),
            })?;
    }
    Ok(Secret::new(stdout.trim()))
}

/// Parse the two security keys of a merged layer, and fail closed on a bad value.
fn parse_sandbox(layer: &ConfigLayer) -> Result<SandboxMode, ConfigError> {
    match &layer.sandbox {
        Some(value) => SandboxMode::from_str(value).map_err(|error| ConfigError::Value {
            key: "sandbox",
            value: value.clone(),
            message: error.to_string(),
        }),
        None => Ok(SandboxMode::default()),
    }
}

fn parse_approval(layer: &ConfigLayer) -> Result<Option<ApprovalMode>, ConfigError> {
    match &layer.approval {
        Some(value) => {
            ApprovalMode::from_str(value)
                .map(Some)
                .map_err(|error| ConfigError::Value {
                    key: "approval",
                    value: value.clone(),
                    message: error.to_string(),
                })
        }
        None => Ok(None),
    }
}

/// Parse the reasoning effort of a merged layer, and fail closed on a bad value.
///
/// An absent key is `None`, and `None` means the provider's own default. So an unset key
/// and `off` are different answers, and the type keeps them apart.
fn parse_reasoning_effort(
    layer: &ConfigLayer,
) -> Result<Option<rho_core::ReasoningEffort>, ConfigError> {
    match &layer.reasoning_effort {
        None => Ok(None),
        Some(value) => value
            .parse::<rho_core::ReasoningEffort>()
            .map(Some)
            .map_err(|error| ConfigError::Value {
                key: "reasoning-effort",
                value: value.clone(),
                message: error,
            }),
    }
}

/// Parse the reasoning display mode of a merged layer, and fail closed on a bad value.
fn parse_reasoning(layer: &ConfigLayer) -> Result<rho_core::ReasoningDisplay, ConfigError> {
    match &layer.tui_reasoning {
        Some(value) => {
            rho_core::ReasoningDisplay::from_str(value).map_err(|error| ConfigError::Value {
                key: "tui-reasoning",
                value: value.clone(),
                message: error,
            })
        }
        None => Ok(rho_core::ReasoningDisplay::default()),
    }
}

fn build_subagents(layer: Option<&SubagentLimitsLayer>) -> rho_core::SubagentLimits {
    let mut limits = rho_core::SubagentLimits::default();
    if let Some(source) = layer {
        if let Some(value) = source.max_depth {
            limits.max_depth = value;
        }
        if let Some(value) = source.max_children_per_parent {
            limits.max_children_per_parent = value;
        }
        if let Some(value) = source.max_live_total {
            limits.max_live_total = value;
        }
        if let Some(secs) = source.child_timeout_secs {
            limits.child_timeout = std::time::Duration::from_secs(secs);
        }
    }
    limits
}

impl Config {
    /// The built-in defaults. The weakest layer.
    pub fn defaults() -> ConfigLayer {
        ConfigLayer {
            // The sandbox default is stated, not hidden. See section 4 and D-bash-os-sandbox.
            sandbox: Some(SandboxMode::default().as_str().to_string()),
            // The approval default stays unset, so the frontend resolves it. See D-approval-option-not-enum.
            ..ConfigLayer::default()
        }
    }

    /// Read one layer from a TOML file. A missing file is `Ok(None)`. An unreadable
    /// file, a malformed file, or an unknown key is an `Err`.
    pub fn read_file(path: &Path) -> Result<Option<ConfigLayer>, ConfigError> {
        let contents = match std::fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(ConfigError::Read {
                    path: path.to_path_buf(),
                    message: error.to_string(),
                });
            }
        };
        let layer = toml::from_str(&contents).map_err(|error| ConfigError::Parse {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        Ok(Some(layer))
    }

    /// Load, merge, and resolve. This is the one entry point.
    pub fn load(sources: &Sources) -> Result<Config, ConfigError> {
        let mut merged = Config::defaults();
        if let Some(path) = &sources.global_file {
            // A global file sits in the user's own home directory. A home directory is
            // not a clone, so it is never gated.
            if let Some(layer) = Config::read_file(path)? {
                merged = merged.merge(layer);
            }
        } // The project file arrives with a clone, so three keys need `--trust-project`.
        // See SPEC-config-call-site section 5, and the probe that proved the command path.
        let mut refused_commands: Option<(PathBuf, BTreeMap<String, String>)> = None;
        if let Some(path) = &sources.project_file
            && let Some(mut layer) = Config::read_file(path)?
        {
            if sources.project_trust == ProjectTrust::Untrusted {
                // `skill-paths` would load attacker skills, and that walks around
                // `D-project-skill-needs-trust`, a gate this repository already ships.
                // `mcp-config` would launch attacker server processes at startup.
                layer.skill_paths = None;
                layer.mcp_config = None;
                let commands = layer
                    .credentials
                    .iter()
                    .flatten()
                    .filter(|(_, raw)| raw.starts_with('!'))
                    .map(|(name, raw)| (name.clone(), raw.clone()))
                    .collect::<BTreeMap<_, _>>();
                if !commands.is_empty() {
                    refused_commands = Some((path.clone(), commands));
                }
            }
            merged = merged.merge(layer);
        }
        // A profile is applied after both files, so a profile value beats a plain
        // file value. A profile the user names but no file defines is an error.
        if let Some(name) = &sources.profile {
            let profile = merged
                .profiles
                .get(name)
                .cloned()
                .ok_or_else(|| ConfigError::UnknownProfile { name: name.clone() })?;
            merged = merged.merge(profile);
        }
        merged = merged.merge(ConfigLayer::from_env(&sources.env));
        merged = merged.merge(sources.flags.clone());

        // The environment layer fails closed on an unaccepted boolean, before use.
        validate_env_booleans(&sources.env)?;

        let sandbox = parse_sandbox(&merged)?;
        let approval = parse_approval(&merged)?;
        let reasoning = parse_reasoning(&merged)?;
        let reasoning_effort = parse_reasoning_effort(&merged)?;
        let credentials = merged
            .credentials
            .unwrap_or_default()
            .into_iter()
            .map(|(name, raw)| {
                // Refuse only when this exact value came from the untrusted project file.
                // A later layer, such as a profile, may have replaced it, and that value
                // is not the one the gate refused.
                if let Some((path, commands)) = &refused_commands
                    && commands.get(&name) == Some(&raw)
                {
                    return (
                        name,
                        CredentialSource::RefusedProjectCommand { path: path.clone() },
                    );
                }
                (name, CredentialSource::parse(&raw))
            })
            .collect();

        Ok(Config {
            provider: merged.provider,
            model: merged.model,
            session_root: merged.session_root,
            session_file: merged.session_file,
            ephemeral: merged.ephemeral.unwrap_or(false),
            sandbox,
            approval,
            skill_paths: merged.skill_paths.unwrap_or_default(),
            // `no-skills = true` disables discovery. The default is discovery on.
            discover_skills: !merged.no_skills.unwrap_or(false),
            // On by default. rho owns the alternate screen, which has no scrollback, so with
            // capture off the wheel does nothing at all. The default flipped with
            // `D-the-wheel-needs-capture`, and this merge adopts it, because that renderer won.
            tui_mouse: merged.tui_mouse.unwrap_or(true),
            reasoning,
            reasoning_effort,
            mcp_config: merged.mcp_config,
            subagents: build_subagents(merged.subagents.as_ref()),
            credentials,
        })
    }

    /// Resolve one named credential to a `Secret`.
    pub fn resolve_credential(
        &self,
        name: &str,
        env: &dyn EnvLookup,
    ) -> Result<Secret, ConfigError> {
        let source = self
            .credentials
            .get(name)
            .ok_or_else(|| ConfigError::Credential {
                name: name.to_string(),
                message: "no credential source is defined by that name".to_string(),
            })?;
        source.resolve(name, env)
    }
}
