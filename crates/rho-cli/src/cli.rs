//! The `rho` command line interface.
//!
//! Three entry points must work: `rho --help`, `rho run <prompt>` for a headless
//! answer on stdout, and `rho` for the interactive TUI. This module parses the
//! arguments, builds a `SessionConfig` explicitly, and runs the chosen mode.
//!
//! The session config is stated out loud here. Decision D-no-four-argument-session-new deleted a
//! convenience constructor because it hid a fake model id, an accidental session
//! root, and a policy that approved every tool call. So this module names the
//! model, the session root, and the approval policy in the calling code.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use clap::ValueEnum;
use clap::{Parser, Subcommand};
use futures::StreamExt;
use rho_core::{
    AgentEvent, AllowAllPolicy, ApprovalPolicy, CancelToken, ContentBlock, Context, ReadOnlyPolicy,
    SandboxMode, Session, SessionConfig, StreamEvent,
};

use crate::extensions;
use crate::provider::{self, MODEL_ENV};
use crate::subagents;

/// The exit code for a run that failed.
const EXIT_FAILURE: i32 = 1;

/// rho is a composable coding agent harness.
#[derive(Debug, Parser)]
#[command(name = "rho", version, about = "A composable coding agent harness.")]
pub struct Cli {
    /// The provider to use. It beats the RHO_PROVIDER variable, through the merge.
    ///
    /// No `env` attribute here. Layer 5 belongs to the merge, and a clap `env` would be a
    /// second precedence beside it. See `SPEC-config` section 2.
    #[arg(long, global = true)]
    pub provider: Option<String>,

    /// The model id to send. It beats the RHO_MODEL variable, through the merge.
    #[arg(long, global = true)]
    pub model: Option<String>,

    /// The profile to apply. A profile is a named block in a config file.
    ///
    /// Layer 4 selects a profile, so without this flag no file profile is reachable.
    #[arg(long, global = true, value_name = "NAME")]
    pub profile: Option<String>,

    /// The session root. Tools cannot touch a path outside it. Defaults to the
    /// current directory.
    #[arg(long, global = true)]
    pub root: Option<PathBuf>,

    /// Deny every tool that can change state. rho then reads, searches, and
    /// thinks, but it does not write, edit, or run a command.
    ///
    /// It is `Option<bool>` so that an unpassed flag writes nothing into layer 6. A plain
    /// `bool` would send `false` and beat an `approval` key in a file. It maps onto
    /// `approval = "read-only"`, so the flag wins by the merge order and needs no
    /// special case.
    #[arg(long, global = true, num_args = 0..=1, default_missing_value = "true")]
    pub read_only: Option<bool>,

    /// The `bash` confinement mode. `off` runs a command unconfined, which is the
    /// default. `confined` limits writes to the session root and the scratch
    /// directory. `strict` also denies the network. When confinement is asked for
    /// and no OS sandbox is available, `bash` refuses the command. See SPEC-bash-sandbox.
    ///
    /// It carries no `default_value_t`. A clap default would always yield `Off`, so layer 6
    /// would beat a file asking for `sandbox = "strict"`. That is a fail-open inside the
    /// merge built to prevent one. The default now lives in `Config::defaults`.
    #[arg(long, global = true, value_enum)]
    pub sandbox: Option<SandboxArg>,

    /// Let the TUI capture the mouse, so the wheel scrolls the band and a click
    /// selects a list row.
    ///
    /// Off by default, because capture takes drag-select away from the terminal. With
    /// capture off, the wheel, a drag, and the terminal search all work on the
    /// transcript. See decision D-native-selection-is-the-default.
    #[arg(long, global = true, num_args = 0..=1, default_missing_value = "true")]
    pub mouse: Option<bool>,

    /// How the TUI draws reasoning: `off`, `summary`, `full`, or `live`.
    ///
    /// Default is `summary`, a one-row `∴ thought for 2.4s`. `full` also draws the
    /// reasoning text, dimmed. `live` draws the text while it streams, then collapses it.
    /// `off` hides reasoning entirely. See `SPEC-reasoning-across-providers`.
    #[arg(long, global = true)]
    pub reasoning: Option<String>,

    /// How hard the model thinks: `off`, `low`, `medium`, `high`, or `xhigh`.
    ///
    /// Unset means the provider's own default, so rho sends no field. The level is the
    /// user's word: a Claude model gets a token budget, and an OpenAI-compatible host gets
    /// the word. A model that cannot think is asked for nothing. See
    /// `SPEC-reasoning-across-providers` section 9.
    #[arg(long, global = true)]
    pub reasoning_effort: Option<String>,

    /// Load skills that live in this repository.
    ///
    /// A skill can instruct the model and can carry scripts, so a skill from the
    /// repository under edit is off by default. See decision D-project-skill-needs-trust.
    #[arg(long, global = true)]
    pub trust_project: bool,

    /// Load a skill from this path. Repeatable. It loads even with --no-skills.
    #[arg(long = "skill", global = true, value_name = "PATH")]
    pub skills: Vec<PathBuf>,

    /// Do not search the skill directories. An explicit --skill still loads.
    #[arg(long, global = true, num_args = 0..=1, default_missing_value = "true")]
    pub no_skills: Option<bool>,

    /// Read MCP servers from this file instead of ~/.rho/mcp.json.
    #[arg(long, global = true, value_name = "PATH")]
    pub mcp_config: Option<PathBuf>,

    /// The log filter, for example "info" or "rho_core=debug". It beats RHO_LOG.
    #[arg(long, global = true)]
    pub log: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// The `--sandbox` value. This mirrors `rho_core::SandboxMode` for clap, because
/// `rho-core` carries no clap dependency. The default is `Off`, stated here.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum SandboxArg {
    /// No confinement. The default.
    Off,
    /// Writes limited to the session root and the scratch directory.
    Confined,
    /// `Confined`, plus no network.
    Strict,
}

impl From<SandboxArg> for SandboxMode {
    fn from(arg: SandboxArg) -> Self {
        match arg {
            SandboxArg::Off => SandboxMode::Off,
            SandboxArg::Confined => SandboxMode::Confined,
            SandboxArg::Strict => SandboxMode::Strict,
        }
    }
}

/// The subcommands of `rho`.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run one prompt without the terminal UI. Print the answer to stdout.
    Run {
        /// The prompt text.
        prompt: String,
    },
}

/// Build a `SessionConfig` from the loaded configuration. State every choice.
fn build_config(config: &rho_config::Config) -> anyhow::Result<SessionConfig> {
    // A model comes from the merge, then the provider's default. Every source the user can
    // set now arrives through `Config`, so this reads one value instead of three.
    //
    // The default is a convenience, not a security choice. Decision D-no-four-argument-session-new removed hidden
    // defaults for the session root and the approval policy, because a wrong value there is
    // a breach. A wrong model id is a bad answer and a small bill.
    //
    // The choice is still reported, so it is never silent.
    let provider_name = provider::resolve_provider_name(config.provider.as_deref(), None)?;
    let model = match config.model.clone() {
        Some(model) => model,
        None => match provider::default_model(&provider_name) {
            Some(model) => {
                eprintln!(
                    "rho: no model given, so using the default for {provider_name}: {model}. \
                     Set --model or {MODEL_ENV} to choose another."
                );
                model.to_string()
            }
            None => {
                return Err(anyhow::anyhow!(
                    "no model was chosen, and {provider_name} has no default. \
                     Set --model or the {MODEL_ENV} variable. For azure, the value is your \
                     deployment name."
                ));
            }
        },
    };

    // The session root confines every tool path. It comes from the merge, and the current
    // directory is the stated default.
    let root = match config.session_root.clone() {
        Some(path) => path,
        None => std::env::current_dir()
            .map_err(|error| anyhow::anyhow!("cannot read the current directory: {error}"))?,
    };

    // State the approval policy out loud. See decision D-no-four-argument-session-new, which deleted a
    // constructor that hid this choice.
    //
    // The default approves every tool call, so a headless run never stops for a
    // prompt. `--read-only` swaps in a policy that denies every mutating tool. That
    // policy is fail-closed: it allows only a kind it names, so a tool with an
    // undeclared kind is denied. See decision D-todo-in-a-green-stage, and D-plugin-does-not-classify-itself for plugin tools,
    // which always count as mutating.
    //
    // A future release adds an interactive approval gate for the TUI. Until then
    // `--read-only` is the way to run rho against a repository you do not trust.
    // The approval policy comes from the merge. `None` means the user stated none, and the
    // stated default here is permissive so a headless run never stops for a prompt. See
    // decision D-no-four-argument-session-new, which deleted a constructor that hid this choice.
    //
    // `Ask` is refused rather than downgraded. There is no interactive gate in this path, so
    // honouring it would either deny every write in silence or approve every write in
    // silence. Both are worse than saying so. `SPEC-approval` owns the real gate.
    let approval: Arc<dyn ApprovalPolicy> = match config.approval {
        Some(rho_config::ApprovalMode::ReadOnly) => Arc::new(ReadOnlyPolicy),
        Some(rho_config::ApprovalMode::AllowAll) | None => Arc::new(AllowAllPolicy),
        Some(rho_config::ApprovalMode::Ask) => {
            return Err(anyhow::anyhow!(
                "approval = \"ask\" needs an interactive frontend, which this build does not \
                 have here. Use read-only or allow-all, or pass --read-only."
            ));
        }
    };

    Ok(SessionConfig::new(model, root, approval)
        .with_sandbox(config.sandbox)
        .with_reasoning_effort(config.reasoning_effort))
}

/// Build a session from the config and the chosen provider.
///
/// Returns the session and its task registry. A caller keeps the registry alive for
/// as long as the session, because dropping it kills every background task.
async fn build_session(
    cli: &Cli,
    loaded: &rho_config::Config,
    config: SessionConfig,
) -> anyhow::Result<(Session, Arc<rho_core::TaskRegistry>, SessionExtras)> {
    let name = provider::resolve_provider_name(loaded.provider.as_deref(), None)?;
    let provider = provider::build_provider(&name)?;
    let tasks = Arc::new(rho_core::TaskRegistry::new(rho_core::TaskLimits::default()));

    // Skills and MCP are optional. A failure in either degrades one capability and
    // never stops the session, so this call cannot fail.
    //
    // The skill list, the discovery switch, and the MCP path all come from the merge, so a
    // config file reaches them. `--trust-project` stays a flag, because it is the trust
    // decision itself and a file cannot grant itself trust.
    let extensions = extensions::load(
        &config.session_root,
        cli.trust_project,
        &loaded.skill_paths,
        loaded.discover_skills,
        loaded.mcp_config.as_deref(),
    )
    .await;

    let mut registry =
        rho_tools::builtin_registry_with_tasks_and_sandbox(Arc::clone(&tasks), config.sandbox);
    for tool in &extensions.mcp_tools {
        registry.register(Arc::clone(tool));
    }
    let hooks = Arc::new(rho_core::HookChain::default());

    // Subagents. The parent's tool set is captured **before** `spawn_agent` joins it, so a
    // child can never receive `spawn_agent` through the intersection. Depth is enforced too,
    // and this makes the common case structural rather than a check.
    let parent_tools: Vec<Arc<dyn rho_core::Tool>> =
        rho_tools::builtin_tools_with_tasks_and_sandbox(Arc::clone(&tasks), config.sandbox)
            .into_iter()
            .chain(extensions.mcp_tools.iter().map(Arc::clone))
            .collect();
    let (spawn_tool, subagents) = subagents::load(subagents::LoadRequest {
        session_root: config.session_root.clone(),
        trust_project: cli.trust_project,
        discover: loaded.discover_skills,
        parent_config: config.clone(),
        provider: Arc::clone(&provider),
        hooks: Arc::clone(&hooks),
        parent_tools,
        limits: rho_core::SubagentLimits::default(),
    })
    .await;
    if let Some(tool) = spawn_tool {
        registry.register(tool);
    }
    let tools = Arc::new(registry);

    // The skills block joins the stable prefix, never the dynamic part. The skill set
    // is fixed for a session, so the prefix stays byte-identical and the provider
    // prompt cache survives. See SPEC-core-runtime section 1 and SPEC-skills section 6.
    let mut prompt = system_prompt();
    if !extensions.skills_prompt.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(&extensions.skills_prompt);
    }

    let context = Context::new(Some(prompt), tools.specs());
    Ok((
        Session::with_config(config, provider, tools, hooks, context),
        tasks,
        SessionExtras {
            notices: extensions
                .notices
                .into_iter()
                .chain(subagents.notices)
                .collect(),
            agents: subagents.registry,
            agent_definitions: subagents.loaded,
            mcp_pool: extensions.mcp_pool,
        },
    ))
}

/// What a caller must hold, and what it should show the user.
struct SessionExtras {
    /// Lines to print once, before the session starts.
    notices: Vec<String>,
    /// The subagent registry. Holding it keeps the process-wide live cap in force for as
    /// long as the session, and the spawn tree shares this handle.
    #[allow(dead_code)]
    agents: rho_core::AgentRegistry,
    /// How many agent definitions loaded. Zero means `spawn_agent` is not registered, so a
    /// status line can say why the tool is absent.
    #[allow(dead_code)]
    agent_definitions: usize,
    /// The MCP pool. Holding it keeps the servers alive for the session.
    #[allow(dead_code)]
    mcp_pool: Option<std::sync::Arc<rho_mcp::McpPool>>,
}

/// The short system prompt. A short prompt keeps the prefix small. See F-short-system-prompt.
fn system_prompt() -> String {
    // The prompt stays short on purpose. See F-short-system-prompt. It says only what the model cannot
    // work out from the tool schemas, and background behaviour is exactly that: the
    // model needs to know that a long command returns a task id, and that it should
    // wait on an event rather than sleep.
    "You are rho, a coding agent. You use the tools to read and change files. \
     You keep answers short.\n\
     A long command runs in the background and returns a task id at once. \
     Use the task tool with the wait action to be woken when it finishes or \
     reports progress. Never sleep and poll."
        .to_string()
}

/// Run the CLI. Return the process exit code.
pub async fn run(cli: Cli) -> i32 {
    match &cli.command {
        Some(Command::Run { prompt }) => run_headless(&cli, prompt.clone()).await,
        None => run_interactive(&cli).await,
    }
}

/// Run one prompt headless. Print the answer to stdout. Print diagnostics to
/// stderr. Return a non-zero code on failure.
async fn run_headless(cli: &Cli, prompt: String) -> i32 {
    // The configuration loads once, here, before a session exists.
    let loaded = match load_config(cli) {
        Ok(loaded) => loaded,
        Err(error) => return fail(error),
    };
    let config = match build_config(&loaded) {
        Ok(config) => config,
        Err(error) => return fail(error),
    };
    // Hold `_tasks` and `_extras` for the whole run. Dropping the task registry kills
    // every background task, and dropping the MCP pool stops every server, so an early
    // drop would end work the model is still waiting on.
    let (session, _tasks, extras) = match build_session(cli, &loaded, config).await {
        Ok(triple) => triple,
        Err(error) => return fail(error),
    };
    for notice in &extras.notices {
        eprintln!("rho: {notice}");
    }
    let _extras = extras;

    let cancel = CancelToken::new();
    let mut events = session.prompt(vec![ContentBlock::Text { text: prompt }], cancel);
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    print_run(
        &mut events,
        &mut stdout,
        &mut stderr,
        reasoning_is_shown(loaded.reasoning),
    )
    .await
}

/// Print one run onto two streams, and return the exit code.
///
/// stdout carries the answer alone, so a pipe stays clean. stderr carries the reasoning and
/// any error. Both are parameters, so a test drives the real loop over a scripted event
/// stream and reads both streams back. The previous version was inline in `run_headless`,
/// where the only guard possible was a grep of this file, and two reviews found that a grep
/// cannot tell a live call from a comment.
async fn print_run<S, O: Write, E: Write>(
    events: &mut S,
    stdout: &mut O,
    stderr: &mut E,
    show_reasoning: bool,
) -> i32
where
    S: futures::Stream<Item = Result<AgentEvent, rho_core::Error>> + Unpin,
{
    let mut failed = false;
    // A run may span several turns, because a turn can call tools. Each turn is a
    // separate block of prose, so separate them. Without this, the last word of one
    // turn runs into the first word of the next.
    let mut wrote_text = false;
    let mut turn_pending = false;
    // A leading `<thinking>` tag is the model's reasoning, not the answer. Without this
    // the headless path printed the tag as the answer, while the TUI did not. The splitter
    // starts fresh for each text block, exactly as the TUI does.
    let mut splitter = rho_core::ThinkingSplitter::new();

    while let Some(item) = events.next().await {
        match item {
            Ok(AgentEvent::Stream(StreamEvent::TextStart { .. })) => {
                splitter = rho_core::ThinkingSplitter::new();
            }
            Ok(AgentEvent::Stream(StreamEvent::TextDelta { delta, .. })) => {
                let split = split_run_delta(&mut splitter, &delta);
                if show_reasoning && !split.reasoning.is_empty() {
                    let _ = write!(stderr, "{}", split.reasoning);
                }
                if split.answer.is_empty() {
                    continue;
                }
                if turn_pending {
                    let _ = writeln!(stdout);
                    turn_pending = false;
                }
                let _ = write!(stdout, "{}", split.answer);
                let _ = stdout.flush();
                wrote_text = true;
            }
            // A structured reasoning block, from a provider rho asked to think.
            Ok(AgentEvent::Stream(StreamEvent::ThinkingDelta { delta, .. })) => {
                if show_reasoning {
                    let _ = write!(stderr, "{delta}");
                }
            }
            Ok(AgentEvent::TurnEnd { .. }) => {
                // Mark a break, but write it only when more prose actually follows.
                // So a run never ends with a stray blank line.
                if wrote_text {
                    turn_pending = true;
                }
            }
            Ok(AgentEvent::AgentEnd { stop_reason }) => {
                tracing::info!(?stop_reason, "run ended");
            }
            Ok(_) => {}
            Err(error) => {
                let _ = writeln!(stderr, "rho: {error}");
                failed = true;
                break;
            }
        }
    }
    let _ = writeln!(stdout);

    if failed { EXIT_FAILURE } else { 0 }
}

/// The working directory, with the home directory shortened to `~`.
fn display_cwd() -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    let text = cwd.to_string_lossy().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && text.starts_with(&home) => text.replacen(&home, "~", 1),
        _ => text,
    }
}

/// The current git branch, or an empty string outside a repository.
///
/// A failed command is not an error here. The banner simply omits the field, because a
/// session outside a repository is normal.
fn git_branch() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default()
}

/// The `RHO_*` variables, as the config layer wants them.
///
/// This is the one place the process environment is read for the interface. A test never
/// reads the real environment, because `resolve_mouse` takes the list as data.
fn rho_env_vars() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(name, _)| name.starts_with("RHO_"))
        .collect()
}

/// Turn the parsed flags into layer 6, the strongest layer.
///
/// A flag the user did not pass must not write a value, so every field stays `None`
/// unless the user passed it. That covers a `bool` and an enum with a clap default, which
/// is the harder half. See `SPEC-config-call-site` rule 6.
///
/// `--read-only` maps onto `approval = "read-only"`. The flag then beats a file's
/// `approval` key by the merge order, and no special case is needed.
fn flag_layer(cli: &Cli) -> rho_config::ConfigLayer {
    rho_config::ConfigLayer {
        provider: cli.provider.clone(),
        model: cli.model.clone(),
        session_root: cli.root.clone(),
        // `--read-only` is one bool over a three-valued enum, so only the true case has a
        // single target. `--read-only=false` says "do not force read-only", and it does not
        // name which of `ask` or `allow-all` the user wants, so it writes nothing.
        approval: match cli.read_only {
            Some(true) => Some("read-only".to_string()),
            Some(false) | None => None,
        },
        // The name comes from `SandboxMode`, so the flag and the file key cannot drift.
        sandbox: cli
            .sandbox
            .map(|arg| SandboxMode::from(arg).as_str().to_string()),
        tui_mouse: cli.mouse,
        tui_reasoning: cli.reasoning.clone(),
        reasoning_effort: cli.reasoning_effort.clone(),
        no_skills: cli.no_skills,
        mcp_config: cli.mcp_config.clone(),
        // An empty `--skill` list is no request at all, so it writes nothing.
        skill_paths: if cli.skills.is_empty() {
            None
        } else {
            Some(cli.skills.clone())
        },
        ..rho_config::ConfigLayer::default()
    }
}

/// The root that locates the project file: `--root`, then `RHO_SESSION_ROOT`, then the
/// working directory.
///
/// A `session-root` key inside a file sets the root for tools. It never moves the project
/// file that was already read, because that would be circular.
fn bootstrap_root(cli: &Cli, env: &[(String, String)]) -> anyhow::Result<PathBuf> {
    if let Some(path) = &cli.root {
        return Ok(path.clone());
    }
    if let Some((_, value)) = env.iter().find(|(name, _)| name == "RHO_SESSION_ROOT")
        && !value.trim().is_empty()
    {
        return Ok(PathBuf::from(value));
    }
    std::env::current_dir()
        .map_err(|error| anyhow::anyhow!("cannot read the current directory: {error}"))
}

/// What one assistant text delta becomes on the two streams of `rho run`.
///
/// stdout carries the answer alone, so a pipe stays clean. Reasoning goes to stderr.
#[derive(Debug, Default, PartialEq, Eq)]
struct RunDelta {
    answer: String,
    reasoning: String,
}

/// Split one text delta into answer text and reasoning text.
///
/// `rho run` printed a leading `<thinking>` tag as the answer, because the splitter lived
/// in the TUI alone. See `SPEC-reasoning-across-providers` section 9.
fn split_run_delta(splitter: &mut rho_core::ThinkingSplitter, delta: &str) -> RunDelta {
    let mut out = RunDelta::default();
    for piece in splitter.push(delta) {
        match piece {
            rho_core::ThinkingPiece::Text(text) => out.answer.push_str(&text),
            rho_core::ThinkingPiece::Reasoning(text) => out.reasoning.push_str(&text),
        }
    }
    out
}

/// Does this display mode print the reasoning text on this path?
///
/// `summary` draws a one-row summary in the TUI. There are no rows here, so `summary` and
/// `off` both print nothing, and the two modes that ask for the text get it.
fn reasoning_is_shown(display: rho_core::ReasoningDisplay) -> bool {
    matches!(
        display,
        rho_core::ReasoningDisplay::Full | rho_core::ReasoningDisplay::Live
    )
}

/// Refuse a bad reasoning mode at its own source, before the merge.
///
/// `merge` keeps a winning value and drops where it came from, so a refusal raised after the
/// merge can only say "the merged configuration". The flag and the variable are checked here
/// so that each refusal still names its own source. `Config::load` keeps its own check for a
/// value that came from a file. See `D-the-merge-cannot-name-a-values-source` and
/// `D-a-bad-reasoning-mode-is-refused`.
fn validate_reasoning_sources(cli: &Cli, env: &[(String, String)]) -> anyhow::Result<()> {
    use std::str::FromStr;
    if let Some(name) = cli.reasoning.as_deref() {
        rho_core::ReasoningDisplay::from_str(name)
            .map_err(|error| anyhow::anyhow!("the --reasoning flag is wrong: {error}"))?;
    }
    if let Some((_, value)) = env.iter().find(|(name, _)| name == "RHO_TUI_REASONING") {
        rho_core::ReasoningDisplay::from_str(value)
            .map_err(|error| anyhow::anyhow!("the RHO_TUI_REASONING variable is wrong: {error}"))?;
    }
    // The effort level follows the same rule as the display mode, and for the same reason:
    // the merge cannot name a value's source, so each source is checked here.
    if let Some(level) = cli.reasoning_effort.as_deref() {
        rho_core::ReasoningEffort::from_str(level)
            .map_err(|error| anyhow::anyhow!("the --reasoning-effort flag is wrong: {error}"))?;
    }
    if let Some((_, value)) = env.iter().find(|(name, _)| name == "RHO_REASONING_EFFORT") {
        rho_core::ReasoningEffort::from_str(value).map_err(|error| {
            anyhow::anyhow!("the RHO_REASONING_EFFORT variable is wrong: {error}")
        })?;
    }
    Ok(())
}

/// Load the configuration once for this process. Every later reader takes `&Config`.
fn load_config(cli: &Cli) -> anyhow::Result<rho_config::Config> {
    let env = rho_env_vars();
    let root = bootstrap_root(cli, &env)?;
    load_config_from(cli, env, &root, &rho_config::SystemEnv)
}

/// The testable core of `load_config`. `home` supplies `XDG_CONFIG_HOME` and `HOME`, so a
/// test never reads the real home directory and its result cannot change per machine.
fn load_config_from(
    cli: &Cli,
    env: Vec<(String, String)>,
    root: &std::path::Path,
    home: &dyn rho_config::EnvLookup,
) -> anyhow::Result<rho_config::Config> {
    validate_reasoning_sources(cli, &env)?;
    let paths = rho_config::ConfigPaths::discover(home, root);
    let sources = rho_config::Sources::from_paths(paths)
        .with_env(env)
        .with_profile(cli.profile.clone())
        .with_flags(flag_layer(cli))
        .with_project_trust(if cli.trust_project {
            rho_config::ProjectTrust::Trusted
        } else {
            rho_config::ProjectTrust::Untrusted
        });
    Ok(rho_config::Config::load(&sources)?)
}

/// Run the interactive TUI. Return a non-zero code on failure.
#[cfg(feature = "tui")]
async fn run_interactive(cli: &Cli) -> i32 {
    // The configuration loads once, here, before a session exists.
    let loaded = match load_config(cli) {
        Ok(loaded) => loaded,
        Err(error) => return fail(error),
    };
    let config = match build_config(&loaded) {
        Ok(config) => config,
        Err(error) => return fail(error),
    };
    let model = config.model.clone();
    let provider_name = provider::resolve_provider_name(loaded.provider.as_deref(), None)
        .unwrap_or_else(|_| String::new());
    // The interface reads the merged configuration, so a config file reaches both switches.
    let mouse = loaded.tui_mouse;
    let reasoning = loaded.reasoning;
    // Hold `_tasks` and `_extras` for the whole run. Dropping the task registry kills
    // every background task, and dropping the MCP pool stops every server, so an early
    // drop would end work the model is still waiting on.
    let (session, _tasks, extras) = match build_session(cli, &loaded, config).await {
        Ok(triple) => triple,
        Err(error) => return fail(error),
    };
    for notice in &extras.notices {
        eprintln!("rho: {notice}");
    }
    let _extras = extras;

    // The banner names where this session runs. Without it the banner drew separators
    // around three empty fields, because nothing ever wrote them.
    let cwd = display_cwd();
    let branch = git_branch();
    let mut app = rho_tui::App::new(session, model)
        .with_mouse(mouse)
        .with_reasoning(reasoning)
        .with_context(cwd, branch, provider_name);
    match app.run().await {
        Ok(()) => 0,
        Err(error) => fail(anyhow::anyhow!(error)),
    }
}

/// The interactive mode needs the `tui` feature. Report a clear error otherwise.
#[cfg(not(feature = "tui"))]
async fn run_interactive(_cli: &Cli) -> i32 {
    eprintln!(
        "rho: this build has no terminal UI. Use \"rho run <prompt>\", or rebuild with the tui feature: cargo build --features tui."
    );
    EXIT_FAILURE
}

/// Print an error to stderr and return the failure code.
fn fail(error: anyhow::Error) -> i32 {
    eprintln!("rho: {error}");
    EXIT_FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use std::collections::BTreeMap;

    /// Load a config with no files, no environment, and an empty temporary root.
    ///
    /// A test never reads the real home directory or the real `RHO_*` variables, because a
    /// result that changes per machine is not a test.
    pub(super) fn loaded(cli: &Cli) -> rho_config::Config {
        try_load(cli, &[], &[]).expect("the config must load")
    }

    /// Load with an explicit `RHO_*` list and an explicit home lookup.
    fn try_load(
        cli: &Cli,
        env: &[(&str, &str)],
        home: &[(&str, &str)],
    ) -> anyhow::Result<rho_config::Config> {
        let root = tempfile::tempdir().expect("a temporary root");
        try_load_in(cli, env, home, root.path())
    }

    /// Load against a named root, so a test can place a project file.
    fn try_load_in(
        cli: &Cli,
        env: &[(&str, &str)],
        home: &[(&str, &str)],
        root: &std::path::Path,
    ) -> anyhow::Result<rho_config::Config> {
        let env = env
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        let home: BTreeMap<String, String> = home
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        load_config_from(cli, env, root, &home)
    }

    /// Write a config file at `<dir>/rho/config.toml`, the global layout.
    fn write_global(dir: &std::path::Path, body: &str) {
        let home = dir.join("rho");
        std::fs::create_dir_all(&home).expect("the global directory");
        std::fs::write(home.join("config.toml"), body).expect("the global file");
    }

    /// Write a config file at `<root>/.rho/config.toml`, the project layout.
    fn write_project(root: &std::path::Path, body: &str) {
        let dir = root.join(".rho");
        std::fs::create_dir_all(&dir).expect("the project directory");
        std::fs::write(dir.join("config.toml"), body).expect("the project file");
    }

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn build_config_uses_the_provider_default_when_no_model_is_given() {
        // The behaviour changed on purpose. This test used to assert that a missing model
        // is an error. A provider now supplies a default, which is a convenience and not a
        // security choice, so the error is gone for a provider that has one.
        //
        // The old test is not deleted, it is split: the case below covers the provider
        // that still has no honest default.
        let cli = Cli::try_parse_from(["rho", "--provider", "openrouter"]).unwrap();
        let config = build_config(&loaded(&cli)).expect("a default model");
        assert_eq!(config.model, "anthropic/claude-haiku-4.5");
    }

    #[test]
    fn build_config_fails_for_a_provider_with_no_default() {
        // Azure names a deployment, not a model, and only the account owner knows the
        // deployment names. So there is no honest default, and the message must say what
        // to set. See docs/verification/models.md.
        let cli = Cli::try_parse_from(["rho", "--provider", "azure"]).unwrap();
        let error = match build_config(&loaded(&cli)) {
            Ok(_) => panic!("azure must not invent a default"),
            Err(error) => error,
        };
        let text = error.to_string();
        assert!(
            text.contains(MODEL_ENV),
            "the message must name the model variable: {text}"
        );
        assert!(
            text.contains("deployment"),
            "and it must say the azure value is a deployment name: {text}"
        );
    }

    #[test]
    fn build_config_uses_an_explicit_root() {
        let cli =
            Cli::try_parse_from(["rho", "--model", "openai/gpt-4o", "--root", "/tmp"]).unwrap();
        let config = build_config(&loaded(&cli)).unwrap();
        assert_eq!(config.session_root, PathBuf::from("/tmp"));
        assert_eq!(config.model, "openai/gpt-4o");
    }

    #[test]
    fn build_config_defaults_to_approving_every_tool() {
        // The default is permissive on purpose, so a headless run never stops for a
        // prompt. The choice is stated in `build_config`, not hidden in a
        // constructor. See decision D-no-four-argument-session-new.
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        assert_eq!(cli.read_only, None, "an unpassed flag must write nothing");
        let config = build_config(&loaded(&cli)).unwrap();
        let decision = futures::executor::block_on(config.approval.approve(
            "write",
            rho_core::ToolKind::Edit,
            &serde_json::json!({}),
        ));
        assert_eq!(decision, rho_core::ApprovalDecision::Allow);
    }

    #[test]
    fn read_only_flag_denies_a_mutating_tool() {
        // `--read-only` is the way to run rho against a repository you do not trust.
        let cli = Cli::try_parse_from(["rho", "--model", "m", "--read-only"]).unwrap();
        assert_eq!(cli.read_only, Some(true));
        let config = build_config(&loaded(&cli)).unwrap();
        for kind in [
            rho_core::ToolKind::Edit,
            rho_core::ToolKind::Delete,
            rho_core::ToolKind::Execute,
            // An undeclared kind counts as mutating, so it is denied too. See D-todo-in-a-green-stage.
            rho_core::ToolKind::Other,
        ] {
            let decision = futures::executor::block_on(config.approval.approve(
                "any",
                kind,
                &serde_json::json!({}),
            ));
            assert_eq!(
                decision,
                rho_core::ApprovalDecision::Deny,
                "{kind:?} must be denied under --read-only"
            );
        }
    }

    #[test]
    fn read_only_flag_still_allows_reading() {
        let cli = Cli::try_parse_from(["rho", "--model", "m", "--read-only"]).unwrap();
        let config = build_config(&loaded(&cli)).unwrap();
        let decision = futures::executor::block_on(config.approval.approve(
            "read",
            rho_core::ToolKind::Read,
            &serde_json::json!({}),
        ));
        assert_eq!(decision, rho_core::ApprovalDecision::Allow);
    }

    #[test]
    fn run_subcommand_parses() {
        let cli = Cli::try_parse_from(["rho", "run", "hello"]).unwrap();
        match cli.command {
            Some(Command::Run { prompt }) => assert_eq!(prompt, "hello"),
            other => panic!("expected a run command, got {other:?}"),
        }
    }

    #[test]
    fn sandbox_flag_defaults_to_off() {
        // The flag itself now writes nothing, because a clap default would beat a file.
        // The effective default is still `Off`, and it comes from the merge.
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        assert_eq!(
            cli.sandbox, None,
            "an unpassed enum flag must write nothing"
        );
        let config = build_config(&loaded(&cli)).unwrap();
        assert_eq!(config.sandbox, rho_core::SandboxMode::Off);
    }

    #[test]
    fn an_unset_flag_does_not_beat_a_file() {
        // Rule 6. A `bool` flag the user never passed must leave layer 6 empty, so a file
        // value survives the merge. A plain `bool` would send `false` and win.
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        let layer = flag_layer(&cli);
        assert_eq!(layer.approval, None, "--read-only was not passed");
        assert_eq!(layer.tui_mouse, None, "--mouse was not passed");
        assert_eq!(layer.no_skills, None, "--no-skills was not passed");
    }

    #[test]
    fn the_sandbox_flag_default_does_not_beat_a_file() {
        // The enum half of rule 6, and the fail-open the review found. `--sandbox` used to
        // carry `default_value_t`, so layer 6 always held `off` and a file asking for
        // `strict` could never win.
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        assert_eq!(flag_layer(&cli).sandbox, None);
    }

    #[test]
    fn flag_layer_maps_each_passed_flag() {
        // A wrong mapping is otherwise silent, because both fields are `Option<String>`.
        let cli = Cli::try_parse_from([
            "rho",
            "--provider",
            "bedrock",
            "--model",
            "claude",
            "--sandbox",
            "strict",
            "--reasoning",
            "full",
            "--read-only",
            "--mouse",
            "--no-skills",
            "--root",
            "/tmp/root",
            "--mcp-config",
            "/tmp/mcp.json",
            "--skill",
            "/tmp/skill-one",
        ])
        .unwrap();
        let layer = flag_layer(&cli);
        assert_eq!(layer.provider.as_deref(), Some("bedrock"));
        assert_eq!(layer.model.as_deref(), Some("claude"));
        assert_eq!(layer.sandbox.as_deref(), Some("strict"));
        assert_eq!(layer.tui_reasoning.as_deref(), Some("full"));
        assert_eq!(
            layer.approval.as_deref(),
            Some("read-only"),
            "--read-only maps onto the approval key"
        );
        assert_eq!(layer.tui_mouse, Some(true));
        assert_eq!(layer.no_skills, Some(true));
        assert_eq!(layer.session_root, Some(PathBuf::from("/tmp/root")));
        assert_eq!(layer.mcp_config, Some(PathBuf::from("/tmp/mcp.json")));
        assert_eq!(
            layer.skill_paths.as_deref(),
            Some([PathBuf::from("/tmp/skill-one")].as_slice())
        );
    }

    #[test]
    fn a_negated_read_only_flag_writes_nothing() {
        // `--read-only=false` does not name which of `ask` or `allow-all` the user wants,
        // so it must not weaken an `approval` key that a file set.
        let cli = Cli::try_parse_from(["rho", "--model", "m", "--read-only=false"]).unwrap();
        assert_eq!(cli.read_only, Some(false));
        assert_eq!(flag_layer(&cli).approval, None);
    }

    #[test]
    fn an_empty_skill_list_writes_nothing() {
        // An absent `--skill` must not send an empty list, because an empty list would beat
        // a file's `skill-paths` and drop every skill in silence.
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        assert_eq!(flag_layer(&cli).skill_paths, None);
    }

    #[test]
    fn no_clap_env_attribute_remains() {
        // A source guard. A clap `env` is a second precedence beside layer 5, which
        // `SPEC-config` section 2 forbids. The needle is built at run time, so this test
        // does not match its own source.
        let needle = ["env", "="].join(" ");
        let offenders: Vec<&str> = include_str!("cli.rs")
            .lines()
            .filter(|line| line.contains("#[arg(") && line.contains(&needle))
            .collect();
        assert!(
            offenders.is_empty(),
            "a clap env attribute is a second precedence: {offenders:?}"
        );
    }

    #[test]
    fn sandbox_flag_sets_confined() {
        let cli = Cli::try_parse_from(["rho", "--model", "m", "--sandbox", "confined"]).unwrap();
        let config = build_config(&loaded(&cli)).unwrap();
        assert_eq!(config.sandbox, rho_core::SandboxMode::Confined);
    }

    #[test]
    fn sandbox_flag_sets_strict() {
        let cli = Cli::try_parse_from(["rho", "--model", "m", "--sandbox", "strict"]).unwrap();
        let config = build_config(&loaded(&cli)).unwrap();
        assert_eq!(config.sandbox, rho_core::SandboxMode::Strict);
    }

    #[test]
    fn sandbox_flag_rejects_an_unknown_mode() {
        let result = Cli::try_parse_from(["rho", "--model", "m", "--sandbox", "loose"]);
        assert!(result.is_err(), "an unknown mode must be rejected");
    }

    // ---- The call site. These replace the resolve_mouse and resolve_reasoning modules,
    // ---- because a reader now takes `&Config` and never a `ConfigLayer`.

    #[test]
    fn the_mouse_is_off_by_default() {
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        assert!(!loaded(&cli).tui_mouse);
    }

    #[test]
    fn the_flag_turns_the_mouse_on() {
        let cli = Cli::try_parse_from(["rho", "--model", "m", "--mouse"]).unwrap();
        assert!(loaded(&cli).tui_mouse);
    }

    #[test]
    fn the_env_var_turns_the_mouse_on() {
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        let config = try_load(&cli, &[("RHO_TUI_MOUSE", "true")], &[]).unwrap();
        assert!(config.tui_mouse);
    }

    #[test]
    fn a_bad_env_value_leaves_the_mouse_off() {
        // The layer omits an unaccepted boolean, so the resolution fails closed.
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        let config = try_load(&cli, &[("RHO_TUI_MOUSE", "yes please")], &[]).unwrap();
        assert!(!config.tui_mouse);
    }

    #[test]
    fn a_config_file_alone_changes_the_mouse_capture() {
        // The R7 defect, for the second key that D-the-layered-config-has-no-caller names.
        // No flag and no variable: the file must reach the product on its own.
        let home = tempfile::tempdir().unwrap();
        write_global(home.path(), "tui-mouse = true\n");
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        let config = try_load(
            &cli,
            &[],
            &[("XDG_CONFIG_HOME", home.path().to_str().unwrap())],
        )
        .unwrap();
        assert!(config.tui_mouse, "a config file alone must turn it on");
    }

    #[test]
    fn reasoning_defaults_to_summary() {
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        assert_eq!(loaded(&cli).reasoning, rho_core::ReasoningDisplay::Summary);
    }

    #[test]
    fn the_flag_sets_the_mode() {
        let cli = Cli::try_parse_from(["rho", "--model", "m", "--reasoning", "full"]).unwrap();
        assert_eq!(loaded(&cli).reasoning, rho_core::ReasoningDisplay::Full);
    }

    #[test]
    fn the_env_var_sets_the_mode() {
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        let config = try_load(&cli, &[("RHO_TUI_REASONING", "live")], &[]).unwrap();
        assert_eq!(config.reasoning, rho_core::ReasoningDisplay::Live);
    }

    #[test]
    fn a_config_file_alone_changes_the_reasoning_mode() {
        // The R7 defect that started this branch. `tui-reasoning = "full"` in a file used to
        // parse, validate, and then draw nothing, because nobody called `Config::load`.
        let home = tempfile::tempdir().unwrap();
        write_global(home.path(), "tui-reasoning = \"full\"\n");
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        let config = try_load(
            &cli,
            &[],
            &[("XDG_CONFIG_HOME", home.path().to_str().unwrap())],
        )
        .unwrap();
        assert_eq!(config.reasoning, rho_core::ReasoningDisplay::Full);
    }

    #[test]
    fn the_flag_beats_the_config_and_the_environment() {
        // The full precedence chain, layer 6 over layer 5 over layer 3. This replaces
        // the_flag_wins_over_the_env_var, which only proved two of the three.
        let home = tempfile::tempdir().unwrap();
        write_global(home.path(), "tui-reasoning = \"full\"\n");
        let cli = Cli::try_parse_from(["rho", "--model", "m", "--reasoning", "off"]).unwrap();
        let config = try_load(
            &cli,
            &[("RHO_TUI_REASONING", "live")],
            &[("XDG_CONFIG_HOME", home.path().to_str().unwrap())],
        )
        .unwrap();
        assert_eq!(config.reasoning, rho_core::ReasoningDisplay::Off);
    }

    #[test]
    fn the_environment_beats_the_config_file() {
        // Layer 5 over layer 3, with no flag. Without this, the chain above could pass while
        // the environment was ignored entirely.
        let home = tempfile::tempdir().unwrap();
        write_global(home.path(), "tui-reasoning = \"full\"\n");
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        let config = try_load(
            &cli,
            &[("RHO_TUI_REASONING", "live")],
            &[("XDG_CONFIG_HOME", home.path().to_str().unwrap())],
        )
        .unwrap();
        assert_eq!(config.reasoning, rho_core::ReasoningDisplay::Live);
    }

    #[test]
    fn an_unknown_reasoning_mode_is_refused() {
        // The owner ruled that a wrong value is an error, never a silent default. The message
        // still names the flag, because the check runs at the source. After the merge it could
        // only say "the merged configuration". See D-the-merge-cannot-name-a-values-source.
        let cli = Cli::try_parse_from(["rho", "--model", "m", "--reasoning", "loud"]).unwrap();
        let error = try_load(&cli, &[], &[])
            .expect_err("an unknown mode name must not resolve")
            .to_string();
        assert!(error.contains("--reasoning"), "names the source: {error}");
        assert!(error.contains("loud"), "names the bad value: {error}");
        assert!(
            error.contains("off") && error.contains("live"),
            "names the valid modes: {error}"
        );
    }

    #[test]
    fn an_unknown_mode_in_the_environment_is_refused() {
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        let error = try_load(&cli, &[("RHO_TUI_REASONING", "loud")], &[])
            .expect_err("an unknown mode name must not resolve")
            .to_string();
        assert!(
            error.contains("RHO_TUI_REASONING"),
            "names the source: {error}"
        );
        assert!(error.contains("loud"), "names the bad value: {error}");
    }

    #[test]
    fn an_unknown_mode_in_a_file_is_refused_and_names_the_key() {
        // The third source. The merge owns this one, and it names the key, not a flag.
        let home = tempfile::tempdir().unwrap();
        write_global(home.path(), "tui-reasoning = \"loud\"\n");
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        let error = try_load(
            &cli,
            &[],
            &[("XDG_CONFIG_HOME", home.path().to_str().unwrap())],
        )
        .expect_err("an unknown mode name must not resolve")
        .to_string();
        assert!(error.contains("tui-reasoning"), "names the key: {error}");
        assert!(error.contains("loud"), "names the bad value: {error}");
    }

    #[test]
    fn every_reasoning_mode_still_resolves() {
        for (name, want) in [
            ("off", rho_core::ReasoningDisplay::Off),
            ("summary", rho_core::ReasoningDisplay::Summary),
            ("full", rho_core::ReasoningDisplay::Full),
            ("live", rho_core::ReasoningDisplay::Live),
        ] {
            let cli = Cli::try_parse_from(["rho", "--model", "m", "--reasoning", name]).unwrap();
            assert_eq!(loaded(&cli).reasoning, want, "mode {name}");
        }
    }

    #[test]
    fn the_bootstrap_root_reads_the_session_root_variable() {
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        let env = vec![(
            "RHO_SESSION_ROOT".to_string(),
            "/tmp/from-the-var".to_string(),
        )];
        assert_eq!(
            bootstrap_root(&cli, &env).unwrap(),
            PathBuf::from("/tmp/from-the-var")
        );
    }

    #[test]
    fn the_root_flag_beats_the_session_root_variable() {
        let cli =
            Cli::try_parse_from(["rho", "--model", "m", "--root", "/tmp/from-the-flag"]).unwrap();
        let env = vec![(
            "RHO_SESSION_ROOT".to_string(),
            "/tmp/from-the-var".to_string(),
        )];
        assert_eq!(
            bootstrap_root(&cli, &env).unwrap(),
            PathBuf::from("/tmp/from-the-flag")
        );
    }

    #[test]
    fn an_empty_session_root_variable_is_ignored() {
        // An exported-but-empty variable is a shell accident, and it must not name the root.
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        let env = vec![("RHO_SESSION_ROOT".to_string(), "   ".to_string())];
        assert_eq!(
            bootstrap_root(&cli, &env).unwrap(),
            std::env::current_dir().unwrap()
        );
    }

    #[test]
    fn a_missing_config_file_is_not_an_error() {
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        let config = try_load(&cli, &[], &[]).expect("a missing file is normal");
        assert_eq!(config.reasoning, rho_core::ReasoningDisplay::Summary);
    }

    #[test]
    fn a_broken_project_file_stops_the_run() {
        let root = tempfile::tempdir().unwrap();
        write_project(root.path(), "this is not toml =\n");
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        let error = try_load_in(&cli, &[], &[], root.path())
            .expect_err("a broken file must stop the run")
            .to_string();
        assert!(
            error.contains("config.toml"),
            "the message names the path: {error}"
        );
    }

    #[test]
    fn loading_twice_gives_the_same_config() {
        // Rule 5. Two loads with the same inputs must agree, or a second reader would see a
        // different product than the first.
        let root = tempfile::tempdir().unwrap();
        write_project(root.path(), "tui-reasoning = \"live\"\nmodel = \"m2\"\n");
        let cli = Cli::try_parse_from(["rho"]).unwrap();
        let first = try_load_in(&cli, &[], &[], root.path()).unwrap();
        let second = try_load_in(&cli, &[], &[], root.path()).unwrap();
        assert_eq!(first.reasoning, second.reasoning);
        assert_eq!(first.model, second.model);
        assert_eq!(first.sandbox, second.sandbox);
    }

    #[test]
    fn an_unknown_profile_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        write_project(root.path(), "model = \"m\"\n");
        let cli = Cli::try_parse_from(["rho", "--profile", "nope"]).unwrap();
        let error = try_load_in(&cli, &[], &[], root.path())
            .expect_err("an undefined profile must not be ignored")
            .to_string();
        assert!(error.contains("nope"), "the message names it: {error}");
    }

    #[test]
    fn a_profile_key_beats_a_plain_file_key() {
        // Layer 4 over layer 3, reached through the --profile flag that did not exist before.
        let root = tempfile::tempdir().unwrap();
        write_project(
            root.path(),
            "tui-reasoning = \"summary\"\n\n[profiles.deep]\ntui-reasoning = \"full\"\n",
        );
        let cli = Cli::try_parse_from(["rho", "--model", "m", "--profile", "deep"]).unwrap();
        let config = try_load_in(&cli, &[], &[], root.path()).unwrap();
        assert_eq!(config.reasoning, rho_core::ReasoningDisplay::Full);
    }

    #[test]
    fn the_session_root_key_does_not_move_the_project_file() {
        // No circular read. The file under the bootstrap root is the one that was read, and a
        // session-root key only moves the root that tools are confined to.
        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        write_project(
            root.path(),
            &format!(
                "session-root = \"{}\"\ntui-reasoning = \"live\"\n",
                elsewhere.path().to_str().unwrap()
            ),
        );
        write_project(elsewhere.path(), "tui-reasoning = \"off\"\n");
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        let config = try_load_in(&cli, &[], &[], root.path()).unwrap();
        assert_eq!(
            config.reasoning,
            rho_core::ReasoningDisplay::Live,
            "the other file must never be read"
        );
        assert_eq!(config.session_root.as_deref(), Some(elsewhere.path()));
    }

    #[test]
    fn a_project_file_reaches_the_product() {
        let root = tempfile::tempdir().unwrap();
        write_project(root.path(), "model = \"from-the-project\"\n");
        let cli = Cli::try_parse_from(["rho"]).unwrap();
        let config = try_load_in(&cli, &[], &[], root.path()).unwrap();
        assert_eq!(config.model.as_deref(), Some("from-the-project"));
    }

    #[test]
    fn an_untrusted_project_file_loses_skill_paths() {
        // Rule 8, now reachable through the call site. Without --trust-project the key drops.
        let root = tempfile::tempdir().unwrap();
        write_project(root.path(), "skill-paths = [\"/tmp/attacker-skills\"]\n");
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        let config = try_load_in(&cli, &[], &[], root.path()).unwrap();
        assert!(
            config.skill_paths.is_empty(),
            "an untrusted project file must not add a skill path"
        );

        let trusted = Cli::try_parse_from(["rho", "--model", "m", "--trust-project"]).unwrap();
        let config = try_load_in(&trusted, &[], &[], root.path()).unwrap();
        assert_eq!(
            config.skill_paths,
            vec![PathBuf::from("/tmp/attacker-skills")],
            "the flag restores it, so the gate is a gate and not a wall"
        );
    }

    #[test]
    fn the_model_variable_still_chooses_the_model() {
        // A regression guard. `--model` carried a clap `env` attribute, and dropping it made
        // RHO_MODEL dead while every one of 874 tests still passed. Layer 5 now carries it.
        let cli = Cli::try_parse_from(["rho"]).unwrap();
        let config = try_load(&cli, &[("RHO_MODEL", "from-the-var")], &[]).unwrap();
        assert_eq!(config.model.as_deref(), Some("from-the-var"));
    }

    #[test]
    fn the_provider_variable_still_chooses_the_provider() {
        // The same regression, for the provider. `resolve_provider_name` takes an env
        // argument that every production caller passes as `None`, so this variable reached
        // the product only through clap.
        let cli = Cli::try_parse_from(["rho"]).unwrap();
        let config = try_load(&cli, &[("RHO_PROVIDER", "bedrock")], &[]).unwrap();
        assert_eq!(config.provider.as_deref(), Some("bedrock"));
    }

    #[test]
    fn the_model_flag_beats_the_model_variable() {
        let cli = Cli::try_parse_from(["rho", "--model", "from-the-flag"]).unwrap();
        let config = try_load(&cli, &[("RHO_MODEL", "from-the-var")], &[]).unwrap();
        assert_eq!(config.model.as_deref(), Some("from-the-flag"));
    }

    #[test]
    fn an_ask_approval_mode_is_refused_here() {
        // `Ask` has no interactive gate in this path. Honouring it would either deny every
        // write in silence or approve every write in silence, and both are worse than saying
        // so. `SPEC-approval` owns the real gate.
        let root = tempfile::tempdir().unwrap();
        write_project(root.path(), "approval = \"ask\"\nmodel = \"m\"\n");
        let cli = Cli::try_parse_from(["rho"]).unwrap();
        let config = try_load_in(&cli, &[], &[], root.path()).unwrap();
        let error = match build_config(&config) {
            Ok(_) => panic!("ask must not resolve to a silent policy"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("ask"), "names the mode: {error}");
        assert!(
            error.contains("read-only") || error.contains("allow-all"),
            "names a way out: {error}"
        );
    }

    #[test]
    fn a_config_file_can_deny_a_mutating_tool() {
        // `approval = "read-only"` from a file must reach the policy, not merely parse. The
        // flag path was already proven, and this is the file path that had no caller.
        let root = tempfile::tempdir().unwrap();
        write_project(root.path(), "approval = \"read-only\"\nmodel = \"m\"\n");
        let cli = Cli::try_parse_from(["rho"]).unwrap();
        let config = build_config(&try_load_in(&cli, &[], &[], root.path()).unwrap()).unwrap();
        let decision = futures::executor::block_on(config.approval.approve(
            "write",
            rho_core::ToolKind::Edit,
            &serde_json::json!({}),
        ));
        assert_eq!(decision, rho_core::ApprovalDecision::Deny);
    }

    #[test]
    fn the_read_only_flag_beats_an_allow_all_file() {
        // The U3 ruling, end to end. The flag lands in layer 6 as approval = "read-only", so
        // it wins over a file with no special case anywhere in the merge.
        let root = tempfile::tempdir().unwrap();
        write_project(root.path(), "approval = \"allow-all\"\nmodel = \"m\"\n");
        let cli = Cli::try_parse_from(["rho", "--read-only"]).unwrap();
        let config = build_config(&try_load_in(&cli, &[], &[], root.path()).unwrap()).unwrap();
        let decision = futures::executor::block_on(config.approval.approve(
            "write",
            rho_core::ToolKind::Edit,
            &serde_json::json!({}),
        ));
        assert_eq!(decision, rho_core::ApprovalDecision::Deny);
    }
}

#[cfg(test)]
mod effort_tests {
    use super::*;
    use clap::Parser;

    fn cli(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).expect("the arguments parse")
    }

    /// The flag reaches layer 6, so a file cannot beat it.
    #[test]
    fn the_effort_flag_writes_layer_six() {
        let cli = cli(&["rho", "--reasoning-effort", "high"]);
        let layer = flag_layer(&cli);
        assert_eq!(layer.reasoning_effort.as_deref(), Some("high"));
    }

    /// An absent flag writes nothing, so a file key survives the merge.
    #[test]
    fn an_absent_effort_flag_writes_nothing() {
        let cli = cli(&["rho"]);
        assert_eq!(flag_layer(&cli).reasoning_effort, None);
    }

    /// A bad flag value is refused, and the error names the flag.
    #[test]
    fn an_unknown_effort_flag_is_refused() {
        let cli = cli(&["rho", "--reasoning-effort", "ludicrous"]);
        let error = validate_reasoning_sources(&cli, &[]).expect_err("a bad level is refused");
        let text = error.to_string();
        assert!(
            text.contains("--reasoning-effort") && text.contains("ludicrous"),
            "the error names the flag and the value: {text}"
        );
    }

    /// A bad variable value is refused, and the error names the variable.
    #[test]
    fn an_unknown_effort_variable_is_refused() {
        let cli = cli(&["rho"]);
        let env = vec![("RHO_REASONING_EFFORT".to_string(), "ludicrous".to_string())];
        let error = validate_reasoning_sources(&cli, &env).expect_err("a bad level is refused");
        assert!(
            error.to_string().contains("RHO_REASONING_EFFORT"),
            "the error names the variable: {error}"
        );
    }

    /// The level reaches the session, or the whole setting changes nothing. This is the
    /// wiring step, and the one a green suite hid twice before on this branch.
    #[test]
    fn the_session_config_carries_the_effort() {
        let cli = cli(&["rho", "--model", "m", "--reasoning-effort", "xhigh"]);
        let loaded = super::tests::loaded(&cli);
        assert_eq!(
            loaded.reasoning_effort,
            Some(rho_core::ReasoningEffort::XHigh),
            "the flag reaches the merged config"
        );
        let built = build_config(&loaded).expect("the session config builds");
        assert_eq!(
            built.reasoning_effort,
            Some(rho_core::ReasoningEffort::XHigh),
            "the merged config reaches the session"
        );
    }
}

#[cfg(test)]
mod run_output_tests {
    use super::*;

    /// `rho run` printed a `<thinking>` tag as the answer, because the splitter was wired
    /// into the TUI alone. Defect 1 of the spec reaches this path too.
    #[test]
    fn the_run_path_strips_a_leading_thinking_tag() {
        let mut splitter = rho_core::ThinkingSplitter::new();
        let out = split_run_delta(&mut splitter, "<thinking>plan</thinking>the answer");
        assert_eq!(out.answer, "the answer");
        assert_eq!(out.reasoning, "plan");
    }

    /// A tag in the middle of a sentence is prose about tags, so it stays in the answer.
    #[test]
    fn the_run_path_keeps_a_tag_in_the_middle() {
        let mut splitter = rho_core::ThinkingSplitter::new();
        let out = split_run_delta(&mut splitter, "here is a <thinking> tag");
        assert_eq!(out.answer, "here is a <thinking> tag");
        assert!(out.reasoning.is_empty());
    }

    /// An opening tag split across deltas still matches, and nothing leaks before the
    /// splitter decides.
    #[test]
    fn the_run_path_holds_a_split_tag_until_it_decides() {
        let mut splitter = rho_core::ThinkingSplitter::new();
        let first = split_run_delta(&mut splitter, "<thin");
        assert!(first.answer.is_empty(), "no text before the decision");
        let second = split_run_delta(&mut splitter, "king>plan</thinking>done");
        assert_eq!(second.reasoning, "plan");
        assert_eq!(second.answer, "done");
    }

    /// A source guard, kept as a cheap tripwire beside the behavioural test in
    /// `headless_loop_tests`. It proves the call exists; the other proves it works.
    #[test]
    fn the_headless_loop_splits_its_text() {
        // The production half, with comments removed. A guard that searches its own text
        // passes against the deletion it exists to catch, and so does one that accepts the
        // literal surviving in a comment. Two reviews found those in turn.
        let whole = include_str!("cli.rs");
        let source: String = whole
            .split("#[cfg(test)]")
            .next()
            .expect("a source file has a first part")
            .lines()
            // Every comment goes, including a trailing one. A break that deleted the call
            // and left its words in a trailing comment passed the first version of this.
            .map(|line| match line.split_once("//") {
                Some((code, _)) => code,
                None => line,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            source.contains("split_run_delta(&mut splitter, &delta)"),
            "the headless loop must split its text deltas"
        );
        assert!(
            source.contains("write!(stdout, \"{}\", split.answer)"),
            "stdout must carry the split answer, not the raw delta"
        );
    }

    /// stdout carries the answer alone, so a pipe stays clean. Reasoning goes to stderr,
    /// and only when the user asked to see it.
    #[test]
    fn reasoning_prints_only_in_full_and_live() {
        use rho_core::ReasoningDisplay;
        assert!(!reasoning_is_shown(ReasoningDisplay::Off));
        assert!(!reasoning_is_shown(ReasoningDisplay::Summary));
        assert!(reasoning_is_shown(ReasoningDisplay::Full));
        assert!(reasoning_is_shown(ReasoningDisplay::Live));
    }
}

#[cfg(test)]
mod headless_loop_tests {
    //! The headless loop, driven for real over a scripted event stream.
    //!
    //! `print_run` takes both streams as parameters, so a test reads back exactly what a user
    //! would see. This replaces a grep of this file, which two reviews showed could pass
    //! against a deleted call whose words survived in a comment.

    use super::*;
    use rho_core::{AgentEvent, StopReason, StreamEvent};

    /// Run the real loop over `events`, and return what landed on each stream.
    fn run(events: Vec<AgentEvent>, show_reasoning: bool) -> (String, String, i32) {
        drive(events.into_iter().map(Ok).collect(), show_reasoning)
    }

    /// The same, for a list that may hold an error.
    fn drive(
        items: Vec<Result<AgentEvent, rho_core::Error>>,
        show_reasoning: bool,
    ) -> (String, String, i32) {
        let mut stream = futures::stream::iter(items);
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let code = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(print_run(&mut stream, &mut out, &mut err, show_reasoning));
        (
            String::from_utf8(out).expect("utf8 on stdout"),
            String::from_utf8(err).expect("utf8 on stderr"),
            code,
        )
    }

    fn text(delta: &str) -> Vec<AgentEvent> {
        vec![
            AgentEvent::Stream(StreamEvent::TextStart { index: 0 }),
            AgentEvent::Stream(StreamEvent::TextDelta {
                index: 0,
                delta: delta.to_string(),
            }),
            AgentEvent::TurnEnd {
                stop_reason: StopReason::EndTurn,
            },
        ]
    }

    /// The defect that opened this branch, on the headless path, proved end to end.
    #[test]
    fn a_leading_tag_never_reaches_stdout() {
        let (out, err, code) = run(text("<thinking>a plan</thinking>the answer"), true);
        assert_eq!(out.trim(), "the answer", "stdout carries the answer alone");
        assert_eq!(err, "a plan", "the reasoning goes to stderr");
        assert_eq!(code, 0);
    }

    /// A tag in the middle is prose about tags, so it stays in the answer.
    #[test]
    fn a_tag_in_the_middle_stays_on_stdout() {
        let (out, err, _) = run(text("here is a <thinking> tag"), true);
        assert_eq!(out.trim(), "here is a <thinking> tag");
        assert!(err.is_empty());
    }

    /// A structured reasoning block goes to stderr, and only when asked for.
    #[test]
    fn structured_reasoning_obeys_the_display_mode() {
        let events = vec![
            AgentEvent::Stream(StreamEvent::ThinkingStart { index: 0 }),
            AgentEvent::Stream(StreamEvent::ThinkingDelta {
                index: 0,
                delta: "private".to_string(),
            }),
            AgentEvent::Stream(StreamEvent::ThinkingEnd {
                index: 0,
                state: None,
            }),
            AgentEvent::Stream(StreamEvent::TextStart { index: 1 }),
            AgentEvent::Stream(StreamEvent::TextDelta {
                index: 1,
                delta: "the answer".to_string(),
            }),
        ];
        let (out, err, _) = run(events.clone(), true);
        assert_eq!(out.trim(), "the answer");
        assert_eq!(err, "private");

        let (out, err, _) = run(events, false);
        assert_eq!(
            out.trim(),
            "the answer",
            "the answer never depends on the mode"
        );
        assert!(err.is_empty(), "no reasoning without the mode: {err}");
    }

    /// An error goes to stderr and sets a non-zero code, so a script can see it.
    #[test]
    fn an_error_exits_non_zero_and_names_itself() {
        let (out, err, code) = drive(
            vec![Err(rho_core::Error::Provider(
                rho_core::ProviderError::Decode("a broken chunk".to_string()),
            ))],
            false,
        );
        assert_eq!(code, EXIT_FAILURE);
        assert!(
            err.contains("a broken chunk"),
            "the error names itself: {err}"
        );
        assert!(out.trim().is_empty(), "no answer on stdout: {out}");
    }
}
