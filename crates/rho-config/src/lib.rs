//! Layered configuration for rho.
//!
//! The public API is stated verbatim in `docs/specs/20260818-014343-SPEC-config.md`. This crate
//! reads configuration from files, the environment, and the command line. It merges
//! those sources in one fixed order, and it resolves a credential to a `Secret`.
//!
//! The crate fails closed. A malformed file, an unknown key, or an unreadable file
//! returns a typed error. It never falls back to a default that grants more access.

use std::collections::{BTreeMap, BTreeSet};
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

/// What an untrusted layer lost, so the caller can tell the user rather than stay silent.
///
/// Silence was the defect a security review named: a user who sets `RHO_SKILL_PATHS` in their
/// shell profile is locked out in an untrusted checkout with no hint that `--trust-project`
/// exists. See `D-trust-is-provenance-not-a-field-list`.
#[derive(Clone, Debug, Default)]
struct Stripped {
    /// Every `!command` credential found, by name, so each becomes a refusal.
    refused: BTreeMap<String, BTreeSet<String>>,
    /// The keys that held a value and were cleared.
    cleared: Vec<&'static str>,
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
    /// The provider endpoint. Unset means the provider's own default. It is powerful: it
    /// redirects the credential, so an untrusted source may not set it.
    pub base_url: Option<String>,
    /// False stops the terminal's sweep animation.
    pub tui_motion: Option<bool>,
    /// True stops the agent-definition search. It is separate from `no_skills`, because a
    /// skill and a worker are two capabilities.
    pub no_agents: Option<bool>,
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
    /// Whether rho searches for agent definitions. Separate from `discover_skills`, so one
    /// switch never removes two capabilities. See `D-skills-and-agents-are-two-switches`.
    pub discover_agents: bool,
    /// The provider endpoint, when a user chose one. `None` keeps the provider's default.
    pub base_url: Option<String>,
    /// Whether the terminal animates the working word. True by default.
    pub tui_motion: bool,
    /// Every powerful key an untrusted source set and lost, named for a notice.
    ///
    /// A drop with no message is the shape a security review named: a user who set
    /// `RHO_SKILL_PATHS` once is locked out in an untrusted checkout and told nothing.
    pub dropped_keys: Vec<String>,
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
        self.base_url = over.base_url.or(self.base_url);
        self.tui_motion = over.tui_motion.or(self.tui_motion);
        self.no_agents = over.no_agents.or(self.no_agents);
        self.subagents = over.subagents.or(self.subagents);
        self.credentials = over.credentials.or(self.credentials);
        // A profile is a named block, not a merged value. Keep the union, so a
        // profile defined in either file is reachable by name.
        self.profiles.extend(over.profiles);
        self
    }

    /// Clear every key an untrusted source may not contribute, at every depth.
    ///
    /// The gate used to null two fields on one layer. `merge` carries `profiles` across
    /// untouched and a profile is applied after the gate, so the same key one level down
    /// never met it. A live probe put an attacker skill in a project profile and the model
    /// received it with no `--trust-project`. See `D-trust-is-provenance-not-a-field-list`
    /// and `docs/verification/profile-trust-bypass.md`.
    ///
    /// It returns the `!command` credentials it found, by name, so the caller can refuse
    /// them rather than drop them. A dropped credential would read as "no such name",
    /// which teaches the user nothing.
    fn strip_powerful_keys(&mut self) -> Stripped {
        self.strip_powerful_keys_to_depth(0)
    }

    /// The recursion, with its own bound.
    ///
    /// A profile may hold profiles, so a config file controls the depth. The `toml` parser
    /// bounds its own nesting today, and a guard that leans on a dependency's behaviour is
    /// not a guard: a parser change would remove it in silence. A review named this, so the
    /// bound lives here.
    fn strip_powerful_keys_to_depth(&mut self, depth: usize) -> Stripped {
        /// Deeper than any real config, and shallow enough that no stack is at risk.
        const MAX_PROFILE_DEPTH: usize = 16;

        // **An exhaustive destructure, with no `..`.** The compiler fails when `ConfigLayer`
        // gains a field, so a new field cannot be trusted by default: somebody must classify
        // it here to make the crate build again.
        //
        // That is the whole point. The first version of this function was a remembered list,
        // and `session_root` was missing from it, so an untrusted project file could move the
        // boundary every file tool confines to. A probe proved it. A test then claimed to
        // catch that class and asserted four hard-coded names instead, which two reviews
        // called decorative. A rule the compiler holds is not a rule anybody can forget.
        //
        // See `D-trust-is-provenance-not-a-field-list` and
        // `docs/verification/profile-trust-bypass.md`.
        let Self {
            // Powerful. An untrusted source may not contribute any of these.
            session_root,
            session_file,
            skill_paths,
            mcp_config,
            base_url,
            credentials,
            profiles,
            // Harmless. Each chooses a model, a display, or a limit, and grants nothing.
            provider: _,
            model: _,
            ephemeral: _,
            sandbox: _,
            approval: _,
            no_skills: _,
            no_agents: _,
            tui_mouse: _,
            tui_motion: _,
            tui_reasoning: _,
            reasoning_effort: _,
            subagents: _,
        } = self;

        let mut cleared: Vec<&'static str> = Vec::new();
        for (name, was_set) in [
            ("session-root", session_root.is_some()),
            ("session-file", session_file.is_some()),
            ("skill-paths", skill_paths.is_some()),
            ("mcp-config", mcp_config.is_some()),
            ("base-url", base_url.is_some()),
        ] {
            if was_set {
                cleared.push(name);
            }
        }
        *session_root = None;
        *session_file = None;
        *skill_paths = None;
        *mcp_config = None;
        *base_url = None;

        // A credential is kept as a refusal rather than dropped, so the user meets a message
        // naming `--trust-project` instead of "no such name".
        let mut refused: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (name, raw) in credentials.iter().flatten() {
            if raw.starts_with('!') {
                refused.entry(name.clone()).or_default().insert(raw.clone());
            }
        }

        // Recurse, so a nesting level a later format adds inherits the rule instead of
        // defeating it. Every refused value for a name is kept, because two profiles may use
        // one name while the merge keeps only the winner, and recording one value let the
        // other slip past the check at the call site.
        for profile in profiles.values_mut() {
            if depth >= MAX_PROFILE_DEPTH {
                // Past the bound the profiles are cleared wholesale, so nothing deeper can
                // carry a powerful value past the gate. The bound lives here rather than in
                // the parser, because a guard that leans on a dependency is not a guard.
                profile.profiles.clear();
                continue;
            }
            let deeper = profile.strip_powerful_keys_to_depth(depth + 1);
            for (name, values) in deeper.refused {
                refused.entry(name).or_default().extend(values);
            }
            for name in deeper.cleared {
                if !cleared.contains(&name) {
                    cleared.push(name);
                }
            }
        }
        Stripped { refused, cleared }
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
                "RHO_BASE_URL" => layer.base_url = Some(value.clone()),
                "RHO_TUI_MOTION" => layer.tui_motion = parse_env_bool("tui-motion", value).ok(),
                "RHO_NO_AGENTS" => layer.no_agents = parse_env_bool("no-agents", value).ok(),
                // The code already documented this name, so a user who sets it once for
                // every tool is obeyed. `1` stops the sweep, as the convention expects.
                "RHO_REDUCE_MOTION"
                    if parse_env_bool("reduce-motion", value).ok() == Some(true) =>
                {
                    layer.tui_motion = Some(false);
                }
                _ => {}
            }
        }
        // `RHO_REDUCE_MOTION` is read after the loop, so it always wins over
        // `RHO_TUI_MOTION`. Both write one field, and the winner used to depend on the order
        // the variables arrived in, which an external review caught. A reduced-motion
        // request is an accessibility signal, so it is the one that decides.
        for (name, value) in vars {
            if name == "RHO_REDUCE_MOTION"
                && parse_env_bool("reduce-motion", value).ok() == Some(true)
            {
                layer.tui_motion = Some(false);
            }
        }
        layer
    }
}

/// A base url must be `https`, or plain `http` to a loopback literal.
///
/// A base url redirects the credential. `https` anywhere is allowed, because the transport
/// is encrypted. Plain `http` puts the key on the wire in clear text, so it is allowed only
/// where the traffic never leaves the machine.
///
/// The host is read from a parser, never from the string. `http://127.0.0.1@evil.example` has
/// host `evil.example`, and a substring check would send the key there.
///
/// An encoded address is **decoded by the parser and then judged on the result**, which an
/// earlier version of this comment denied. `http://2130706433` is 127.0.0.1, so it is allowed,
/// and `http://3627734734` is not loopback, so it is refused. A live probe found the comment
/// wrong while the behaviour was right. A name is never resolved, because a DNS lookup here
/// would be a TOCTOU of its own, so only `localhost` is accepted by name.
fn check_base_url(value: &str) -> Result<(), ConfigError> {
    let refuse = |message: &str| {
        Err(ConfigError::Value {
            key: "base-url",
            value: value.to_string(),
            message: message.to_string(),
        })
    };
    let Ok(url) = url::Url::parse(value) else {
        return refuse("it is not a url with a scheme, for example https://host/v1");
    };
    if !url.username().is_empty() || url.password().is_some() {
        return refuse(
            "a url with a user or a password is refused, because the host is not what it looks like",
        );
    }
    match url.scheme() {
        "https" => Ok(()),
        "http" => match url.host() {
            Some(url::Host::Ipv4(address)) if address.is_loopback() => Ok(()),
            Some(url::Host::Ipv6(address)) if address.is_loopback() => Ok(()),
            Some(url::Host::Domain("localhost")) => Ok(()),
            _ => refuse(
                "plain http is allowed only to localhost, 127.0.0.0/8, or [::1], because the credential would travel in clear text",
            ),
        },
        other => refuse(&format!("the scheme {other} is not https or http")),
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
            "RHO_NO_AGENTS" => {
                parse_env_bool("no-agents", value)?;
            }
            "RHO_TUI_MOTION" => {
                parse_env_bool("tui-motion", value)?;
            }
            "RHO_REDUCE_MOTION" => {
                parse_env_bool("reduce-motion", value)?;
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
        let mut refused_commands: Option<(PathBuf, BTreeMap<String, BTreeSet<String>>)> = None;
        // Every powerful key an untrusted source lost. The caller names them, because a
        // silent drop leaves a user with no hint that `--trust-project` exists.
        let mut dropped_keys: Vec<String> = Vec::new();
        if let Some(path) = &sources.project_file
            && let Some(mut layer) = Config::read_file(path)?
        {
            if sources.project_trust == ProjectTrust::Untrusted {
                // `skill-paths` would load attacker skills, and that walks around
                // `D-project-skill-needs-trust`, a gate this repository already ships.
                // `mcp-config` would launch attacker server processes at startup, and
                // `base-url` would point the credential at a host the file chose.
                let stripped = layer.strip_powerful_keys();
                for name in &stripped.cleared {
                    dropped_keys.push(format!("{name} (from {})", path.display()));
                }
                if !stripped.refused.is_empty() {
                    refused_commands = Some((path.clone(), stripped.refused));
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
        // The environment is the wider door. A `.devcontainer` file, a CI `env:` block,
        // and a `.envrc` all arrive with the clone, so a powerful variable needs the same
        // trust as a powerful key in the file beside it. A display key needs none, because
        // it grants nothing. See `D-trust-is-provenance-not-a-field-list`.
        let mut env_layer = ConfigLayer::from_env(&sources.env);
        if sources.project_trust == ProjectTrust::Untrusted {
            let stripped = env_layer.strip_powerful_keys();
            for name in &stripped.cleared {
                dropped_keys.push(format!("{name} (from the environment)"));
            }
            if !stripped.refused.is_empty() && refused_commands.is_none() {
                refused_commands = Some((PathBuf::from("the environment"), stripped.refused));
            }
        }
        merged = merged.merge(env_layer);
        merged = merged.merge(sources.flags.clone());

        // The environment layer fails closed on an unaccepted boolean, before use.
        validate_env_booleans(&sources.env)?;

        if let Some(url) = &merged.base_url {
            check_base_url(url)?;
        }
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
                    && commands
                        .get(&name)
                        .is_some_and(|values| values.contains(&raw))
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
            discover_agents: !merged.no_agents.unwrap_or(false),
            base_url: merged.base_url.clone(),
            tui_motion: merged.tui_motion.unwrap_or(true),
            dropped_keys,
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
