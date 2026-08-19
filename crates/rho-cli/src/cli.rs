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
use crate::provider::{self, MODEL_ENV, PROVIDER_ENV};
use crate::subagents;

/// The exit code for a run that failed.
const EXIT_FAILURE: i32 = 1;

/// rho is a composable coding agent harness.
#[derive(Debug, Parser)]
#[command(name = "rho", version, about = "A composable coding agent harness.")]
pub struct Cli {
    /// The provider to use. Overrides the RHO_PROVIDER variable.
    #[arg(long, global = true, env = PROVIDER_ENV)]
    pub provider: Option<String>,

    /// The model id to send. Overrides the RHO_MODEL variable.
    #[arg(long, global = true, env = MODEL_ENV)]
    pub model: Option<String>,

    /// The session root. Tools cannot touch a path outside it. Defaults to the
    /// current directory.
    #[arg(long, global = true)]
    pub root: Option<PathBuf>,

    /// Deny every tool that can change state. rho then reads, searches, and
    /// thinks, but it does not write, edit, or run a command.
    #[arg(long, global = true)]
    pub read_only: bool,

    /// The `bash` confinement mode. `off` runs a command unconfined, which is the
    /// default. `confined` limits writes to the session root and the scratch
    /// directory. `strict` also denies the network. When confinement is asked for
    /// and no OS sandbox is available, `bash` refuses the command. See SPEC-bash-sandbox.
    #[arg(long, global = true, value_enum, default_value_t = SandboxArg::Off)]
    pub sandbox: SandboxArg,

    /// Let the TUI capture the mouse, so the wheel scrolls the band and a click
    /// selects a list row.
    ///
    /// Off by default, because capture takes drag-select away from the terminal. With
    /// capture off, the wheel, a drag, and the terminal search all work on the
    /// transcript. See decision D-native-selection-is-the-default.
    #[arg(long, global = true)]
    pub mouse: bool,

    /// Load skills that live in this repository.
    ///
    /// A skill can instruct the model and can carry scripts, so a skill from the
    /// repository under edit is off by default. See decision D-project-skill-needs-trust.
    #[arg(long, global = true)]
    pub trust_project: bool,

    /// Load a skill from this path. Repeatable. It loads even with --no-skills.
    #[arg(long = "skill", global = true, value_name = "PATH")]
    pub skills: Vec<PathBuf>,

    /// How many children one agent may run at once. Defaults to 4.
    ///
    /// A refusal names this flag, so it has to exist.
    #[arg(long, global = true, value_name = "COUNT")]
    pub max_children_per_parent: Option<usize>,

    /// How many agents may be live in the whole process. Defaults to 32.
    ///
    /// This protects the machine, where --max-children-per-parent protects one run.
    #[arg(long, global = true, value_name = "COUNT")]
    pub max_live_agents: Option<usize>,

    /// How long a child may run before rho cancels it. Defaults to 600 seconds.
    #[arg(long, global = true, value_name = "SECONDS")]
    pub child_timeout_secs: Option<u64>,

    /// How many tool calls one subagent may make. Defaults to 64.
    ///
    /// A turn cap counts provider round trips. It does not bound a child that makes
    /// forty tool calls inside one turn. This does.
    #[arg(long, global = true, value_name = "COUNT")]
    pub max_agent_tool_calls: Option<u32>,
    /// Do not search the skill directories. An explicit --skill still loads.
    #[arg(long, global = true)]
    pub no_skills: bool,

    /// Read MCP servers from this file instead of ~/.rho/mcp.json.
    #[arg(long, global = true, value_name = "PATH")]
    pub mcp_config: Option<PathBuf>,

    /// The log filter, for example "info" or "rho_core=debug". Overrides RHO_LOG.
    #[arg(long, global = true, env = "RHO_LOG")]
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

/// Build a `SessionConfig` from the parsed arguments. State every choice.
fn build_config(cli: &Cli) -> anyhow::Result<SessionConfig> {
    // A model comes from the flag, then the environment, then the provider's default.
    //
    // The default is a convenience, not a security choice. Decision D-no-four-argument-session-new removed hidden
    // defaults for the session root and the approval policy, because a wrong value there is
    // a breach. A wrong model id is a bad answer and a small bill.
    //
    // The choice is still reported, so it is never silent.
    let provider_name = provider::resolve_provider_name(cli.provider.as_deref(), None)?;
    let model = match cli.model.clone() {
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

    // The session root confines every tool path. Choose the current directory by
    // default, and state that choice here. A --root flag overrides it.
    let root = match &cli.root {
        Some(path) => path.clone(),
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
    let approval: Arc<dyn ApprovalPolicy> = if cli.read_only {
        Arc::new(ReadOnlyPolicy)
    } else {
        Arc::new(AllowAllPolicy)
    };

    Ok(SessionConfig::new(model, root, approval).with_sandbox(cli.sandbox.into()))
}

/// Build a session from the config and the chosen provider.
///
/// Returns the session and its task registry. A caller keeps the registry alive for
/// as long as the session, because dropping it kills every background task.
async fn build_session(
    cli: &Cli,
    config: SessionConfig,
) -> anyhow::Result<(Session, Arc<rho_core::TaskRegistry>, SessionExtras)> {
    let name = provider::resolve_provider_name(cli.provider.as_deref(), None)?;
    let provider = provider::build_provider(&name)?;
    let tasks = Arc::new(rho_core::TaskRegistry::new(rho_core::TaskLimits::default()));

    // Skills and MCP are optional. A failure in either degrades one capability and
    // never stops the session, so this call cannot fail.
    let extensions = extensions::load(
        &config.session_root,
        cli.trust_project,
        &cli.skills,
        !cli.no_skills,
        cli.mcp_config.as_deref(),
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
    let (spawn_tools, subagents) = subagents::load(subagents::LoadRequest {
        session_root: config.session_root.clone(),
        trust_project: cli.trust_project,
        discover: !cli.no_skills,
        parent_config: config.clone(),
        provider: Arc::clone(&provider),
        hooks: Arc::clone(&hooks),
        parent_tools,
        limits: subagent_limits(cli),
    })
    .await;
    for tool in spawn_tools {
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
    let config = match build_config(cli) {
        Ok(config) => config,
        Err(error) => return fail(error),
    };
    // Hold `_tasks` and `_extras` for the whole run. Dropping the task registry kills
    // every background task, and dropping the MCP pool stops every server, so an early
    // drop would end work the model is still waiting on.
    let (session, _tasks, extras) = match build_session(cli, config).await {
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
    let mut failed = false;
    // A run may span several turns, because a turn can call tools. Each turn is a
    // separate block of prose, so separate them. Without this, the last word of one
    // turn runs into the first word of the next.
    let mut wrote_text = false;
    let mut turn_pending = false;

    while let Some(item) = events.next().await {
        match item {
            Ok(AgentEvent::Stream(StreamEvent::TextDelta { delta, .. })) => {
                if turn_pending {
                    let _ = writeln!(stdout);
                    turn_pending = false;
                }
                let _ = write!(stdout, "{delta}");
                let _ = stdout.flush();
                wrote_text = true;
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
                eprintln!("rho: {error}");
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

/// Whether the TUI captures the mouse. The flag wins, then the environment, then off.
///
/// `--mouse` and `RHO_TUI_MOUSE` both reach this. The config file does not, because no
/// binary reads a config file yet. See decision D-the-layered-config-has-no-caller.
fn resolve_mouse(flag: bool, env: &[(String, String)]) -> bool {
    if flag {
        return true;
    }
    rho_config::ConfigLayer::from_env(env)
        .tui_mouse
        .unwrap_or(false)
}

/// Run the interactive TUI. Return a non-zero code on failure.
#[cfg(feature = "tui")]
async fn run_interactive(cli: &Cli) -> i32 {
    let config = match build_config(cli) {
        Ok(config) => config,
        Err(error) => return fail(error),
    };
    let model = config.model.clone();
    let provider_name = provider::resolve_provider_name(cli.provider.as_deref(), None)
        .unwrap_or_else(|_| String::new());
    // The interface reads one switch. The flag wins, then the environment, then off.
    let mouse = resolve_mouse(cli.mouse, &rho_env_vars());
    // Hold `_tasks` and `_extras` for the whole run. Dropping the task registry kills
    // every background task, and dropping the MCP pool stops every server, so an early
    // drop would end work the model is still waiting on.
    let (session, _tasks, extras) = match build_session(cli, config).await {
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

/// The subagent limits for this run, from the flags.
///
/// Every limit refusal in `rho-core` tells the user which flag to raise. The flags
/// did not exist, so a refusal named something impossible. A live sweep found it.
/// See `docs/verification/subagents-bedrock.md`.
///
/// `max_depth` is **not** a flag, and it is 1. `rho-cli` captures the parent tool
/// set before `spawn_agent` joins it, so a child never holds a spawn tool and a
/// grandchild cannot exist. Offering a depth flag would promise something the CLI
/// cannot do. See decision D-cli-depth-is-zero.
///
/// It is 1 and not 0, because 0 forbids spawning altogether. The root session is
/// depth 0, so a value of 0 refuses the very first child and the feature dies. A
/// live run caught that after the unit tests passed.
fn subagent_limits(cli: &Cli) -> rho_core::SubagentLimits {
    let stated = rho_core::SubagentLimits::new();
    rho_core::SubagentLimits {
        max_depth: 1,
        max_children_per_parent: cli
            .max_children_per_parent
            .unwrap_or(stated.max_children_per_parent),
        max_live_total: cli.max_live_agents.unwrap_or(stated.max_live_total),
        max_tool_calls: cli.max_agent_tool_calls.unwrap_or(stated.max_tool_calls),
        child_timeout: cli
            .child_timeout_secs
            .map(std::time::Duration::from_secs)
            .unwrap_or(stated.child_timeout),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn subagent_limits_come_from_the_flags() {
        // Every limit refusal in `rho-core` tells the user to raise a flag. Those
        // flags did not exist, so the refusal taught something impossible. A live
        // sweep found it. See docs/verification/subagents-bedrock.md.
        let cli = Cli::try_parse_from([
            "rho",
            "--provider",
            "openrouter",
            "--max-children-per-parent",
            "2",
            "--max-live-agents",
            "7",
            "--child-timeout-secs",
            "30",
            "--max-agent-tool-calls",
            "9",
        ])
        .unwrap();
        let limits = subagent_limits(&cli);
        assert_eq!(limits.max_children_per_parent, 2);
        assert_eq!(limits.max_live_total, 7);
        assert_eq!(limits.child_timeout, std::time::Duration::from_secs(30));
        assert_eq!(
            limits.max_tool_calls, 9,
            "a tool-call budget nobody can set is not a budget"
        );
    }

    #[test]
    fn subagent_limits_default_to_the_stated_values() {
        // The defaults stay where `SubagentLimits::new` states them, so a flag that
        // is absent changes nothing. See decision D-no-four-argument-session-new.
        let cli = Cli::try_parse_from(["rho", "--provider", "openrouter"]).unwrap();
        let limits = subagent_limits(&cli);
        let stated = rho_core::SubagentLimits::new();
        assert_eq!(
            limits.max_children_per_parent,
            stated.max_children_per_parent
        );
        assert_eq!(limits.max_live_total, stated.max_live_total);
        assert_eq!(limits.child_timeout, stated.child_timeout);
        assert_eq!(limits.max_tool_calls, stated.max_tool_calls);
    }

    #[test]
    fn a_cli_allows_one_level_of_delegation_and_no_more() {
        // The CLI depth is 1, so the root may spawn a child and the child may not
        // spawn a grandchild. It must never be 0: the root session is itself depth 0,
        // so 0 refuses the first child and the whole feature dies. A live run caught
        // exactly that, after these unit tests passed. See
        // docs/verification/subagents-bedrock.md and decision D-cli-depth-is-zero.
        let cli = Cli::try_parse_from(["rho", "--provider", "openrouter"]).unwrap();
        let limits = subagent_limits(&cli);
        assert_eq!(
            limits.max_depth, 1,
            "the root must be able to spawn a child"
        );

        // Prove the shape end to end on the real registry, not on the number alone.
        let registry = rho_core::AgentRegistry::new(limits);
        let root = registry.root();
        let spawn = root
            .spawn_child("scout", rho_core::CancelToken::new())
            .expect("the root must be able to spawn one child");
        assert!(
            spawn
                .node
                .spawn_child("scout", rho_core::CancelToken::new())
                .is_err(),
            "a CLI child must not spawn a grandchild"
        );
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
        let config = build_config(&cli).expect("a default model");
        assert_eq!(config.model, "anthropic/claude-haiku-4.5");
    }

    #[test]
    fn build_config_fails_for_a_provider_with_no_default() {
        // Azure names a deployment, not a model, and only the account owner knows the
        // deployment names. So there is no honest default, and the message must say what
        // to set. See docs/verification/models.md.
        let cli = Cli::try_parse_from(["rho", "--provider", "azure"]).unwrap();
        let error = match build_config(&cli) {
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
        let config = build_config(&cli).unwrap();
        assert_eq!(config.session_root, PathBuf::from("/tmp"));
        assert_eq!(config.model, "openai/gpt-4o");
    }

    #[test]
    fn build_config_defaults_to_approving_every_tool() {
        // The default is permissive on purpose, so a headless run never stops for a
        // prompt. The choice is stated in `build_config`, not hidden in a
        // constructor. See decision D-no-four-argument-session-new.
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        assert!(!cli.read_only);
        let config = build_config(&cli).unwrap();
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
        assert!(cli.read_only);
        let config = build_config(&cli).unwrap();
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
        let config = build_config(&cli).unwrap();
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
        // The default is stated in the flag definition, not hidden. See D-no-four-argument-session-new.
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        assert_eq!(cli.sandbox, SandboxArg::Off);
        let config = build_config(&cli).unwrap();
        assert_eq!(config.sandbox, rho_core::SandboxMode::Off);
    }

    #[test]
    fn sandbox_flag_sets_confined() {
        let cli = Cli::try_parse_from(["rho", "--model", "m", "--sandbox", "confined"]).unwrap();
        let config = build_config(&cli).unwrap();
        assert_eq!(config.sandbox, rho_core::SandboxMode::Confined);
    }

    #[test]
    fn sandbox_flag_sets_strict() {
        let cli = Cli::try_parse_from(["rho", "--model", "m", "--sandbox", "strict"]).unwrap();
        let config = build_config(&cli).unwrap();
        assert_eq!(config.sandbox, rho_core::SandboxMode::Strict);
    }

    #[test]
    fn sandbox_flag_rejects_an_unknown_mode() {
        let result = Cli::try_parse_from(["rho", "--model", "m", "--sandbox", "loose"]);
        assert!(result.is_err(), "an unknown mode must be rejected");
    }
}

#[cfg(test)]
mod mouse_tests {
    use super::resolve_mouse;

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn the_mouse_is_off_by_default() {
        assert!(!resolve_mouse(false, &[]));
    }

    #[test]
    fn the_flag_turns_the_mouse_on() {
        assert!(resolve_mouse(true, &[]));
    }

    #[test]
    fn the_env_var_turns_the_mouse_on() {
        assert!(resolve_mouse(false, &env(&[("RHO_TUI_MOUSE", "true")])));
    }

    #[test]
    fn a_bad_env_value_leaves_the_mouse_off() {
        // The layer omits an unaccepted boolean, so the resolution fails closed.
        assert!(!resolve_mouse(
            false,
            &env(&[("RHO_TUI_MOUSE", "yes please")])
        ));
    }
}
