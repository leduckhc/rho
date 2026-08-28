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
use std::path::{Path, PathBuf};
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
use crate::recording::{self, Recording, RecordingRequest, SessionSelector};
use crate::sessions_command;
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

    /// Let the TUI capture the mouse, so the wheel scrolls the transcript and a click
    /// selects a list row.
    ///
    /// On by default. rho owns the alternate screen, which has no scrollback, so the wheel
    /// is the only way to scroll. Pass `--no-mouse` to give the mouse back to the terminal.
    /// Option and drag still selects text in Ghostty and in iTerm2. See decision
    /// D-the-wheel-needs-capture.
    #[arg(long, global = true)]
    pub mouse: bool,

    /// Give the mouse back to the terminal, so a drag selects text without a modifier.
    ///
    /// It wins over `--mouse`, over the config, and over the environment. The wheel then
    /// does nothing, because the alternate screen has no scrollback. See decision
    /// D-the-wheel-needs-capture.
    #[arg(long = "no-mouse", global = true, conflicts_with = "mouse")]
    pub no_mouse: bool,

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

    /// How long a child may wait for a slot before rho refuses it.
    ///
    /// It defaults to the child timeout, so a waiter gets one whole sibling run of
    /// patience. `0` refuses any child that has to wait. There is no off switch, because
    /// one blocking spawn used to hold a turn for about forty minutes. A very large value
    /// comes close to one, and that choice belongs to the host.
    #[arg(long, global = true, value_name = "SECONDS")]
    pub queue_wait_secs: Option<u64>,

    /// The largest steering message a subagent queue accepts, in bytes. Defaults to 16384.
    ///
    /// A message count is not a memory bound, because one message can be any size. A
    /// model writes a steer to a child, and 160 child queues may exist at once.
    ///
    /// **Raising this raises the memory ceiling with it.** The ceiling is this value times
    /// 32 messages, times `--max-queued-total` plus `--max-live-agents`. At the defaults
    /// that is 80 MiB. rho does not clamp the value, because the host owns the machine.
    #[arg(long, global = true, value_name = "BYTES")]
    pub max_agent_steer_bytes: Option<usize>,

    /// How many children one parent may queue for a slot. Defaults to 16.
    ///
    /// Over the per-parent child cap, rho queues a child instead of refusing it. This
    /// bounds that line, because a waiting child holds a cancel token and a queue.
    #[arg(long, global = true, value_name = "COUNT")]
    pub max_queued_per_parent: Option<usize>,

    /// How many children may wait for a slot in the whole process. Defaults to 128.
    ///
    /// A session root holds no live-child slot, so --max-live-agents bounds neither
    /// the number of sessions nor the number of wait lines. This bounds the total.
    #[arg(long, global = true, value_name = "COUNT")]
    pub max_queued_total: Option<usize>,

    /// Turns of warning before a subagent's turn cap. `0` turns the warning off.
    ///
    /// A child that runs out of turns has nobody to ask, so rho tells it to write
    /// its summary this many turns early. The default is 5.
    #[arg(long, global = true, value_name = "TURNS")]
    pub agent_grace_turns: Option<u32>,

    /// How many tool calls one subagent may make. Defaults to 64.
    ///
    /// A turn cap counts provider round trips. It does not bound a child that makes
    /// forty tool calls inside one turn. This does.
    #[arg(long, global = true, value_name = "COUNT")]
    pub max_agent_tool_calls: Option<u32>,
    /// Do not search the skill directories. An explicit --skill still loads.
    #[arg(long, global = true, num_args = 0..=1, default_missing_value = "true")]
    pub no_skills: Option<bool>,
    /// Do not search for agent definitions, so rho offers no subagent.
    ///
    /// It is separate from `--no-skills`. One flag used to stop both loaders, and it said
    /// nothing about either. See decision D-skills-and-agents-are-two-switches.
    #[arg(long, global = true, num_args = 0..=1, default_missing_value = "true")]
    pub no_agents: Option<bool>,
    /// Stop the animation that sweeps the working word.
    ///
    /// The footer still names the state in words, so nothing is lost but the movement.
    ///
    /// It is `Option<bool>` with `num_args = 0..=1`, the same shape as `--no-skills` and
    /// `--no-agents`. A bare `bool` could not parse `--no-motion false`, and a global
    /// `tui-motion = false` could then never be turned back on from the command line. See D9.
    #[arg(long, global = true, num_args = 0..=1, default_missing_value = "true")]
    pub no_motion: Option<bool>,
    /// The provider endpoint. Use it for a local model host, such as Ollama or vLLM.
    ///
    /// It redirects the credential, so a project file and the environment need
    /// `--trust-project` to set it. A remote plain-http url is refused.
    #[arg(long, global = true, value_name = "URL")]
    pub base_url: Option<String>,

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

        /// Continue a session. Bare it takes the newest one, and `--continue=<id>` takes that
        /// one.
        ///
        /// `--resume` is an alias, so the two spellings are one argument and can never
        /// disagree. `--continue --resume=<id>` is then refused by clap itself, with no custom
        /// conflict rule.
        ///
        /// **The value needs an equals sign.** The prompt is a positional argument, so without
        /// `require_equals` clap takes the prompt as this flag's value and the prompt goes
        /// missing. That was measured, not guessed. See `SPEC-session-store-wiring` section 8c.
        #[arg(
            long = "continue",
            short = 'c',
            alias = "resume",
            num_args = 0..=1,
            require_equals = true,
            default_missing_value = "",
            value_name = "ID"
        )]
        continue_session: Option<String>,

        /// Write no session file at all.
        #[arg(long)]
        ephemeral: bool,

        /// Allow a resume that widens the approval mode or the sandbox mode.
        ///
        /// A stored mode may only tighten a run. This flag is the only way to widen one, and it
        /// is an error on its own. See `D-resume-never-widens`.
        #[arg(long, requires = "continue_session")]
        allow_widen: bool,
    },
    /// Read, name, fork, and delete the sessions of this project.
    Sessions {
        #[command(subcommand)]
        action: SessionsAction,
    },
}

/// What `rho sessions` does.
#[derive(Debug, Subcommand)]
pub enum SessionsAction {
    /// Print one row per session, newest first.
    List {
        /// Also print the working directory and the fork origin.
        #[arg(long)]
        long: bool,
    },
    /// Print one line per record, so a user can read a session and copy a record id.
    ///
    /// It sends nothing to a model, and it starts no session. So looking costs nothing.
    Show {
        /// The session id, or a prefix of one.
        id: String,
        /// Print the whole text of each record instead of one line.
        #[arg(long)]
        full: bool,
    },
    /// Delete one session file, and every sidecar beside it.
    Delete {
        /// The session id, or a prefix of one.
        id: String,
    },
    /// Copy a session into a new one, from a chosen record.
    Fork {
        /// The session id, or a prefix of one.
        id: String,
        /// The record to fork from. `rho sessions show` prints it in the first column.
        #[arg(long, value_name = "RECORD-ID")]
        at: String,
    },
    /// Write an explicit title for one session.
    Name {
        /// The session id, or a prefix of one.
        id: String,
        /// The title. An empty title is refused.
        title: String,
    },
}

/// Build a `SessionConfig` from the loaded configuration, and print any notice.
///
/// This is the wrapper the non-interactive paths use, where a print is the right channel.
/// The interactive path calls `build_config_with_notices`, because a print there lands on the
/// primary screen and rho then opens the alternate screen over it.
///
/// It takes the loaded `Config`, not the flags. Every source the user can set arrives through
/// the merge, so this reads one value instead of three. That is the call-site contract of
/// `SPEC-config-call-site`, and this merge kept it while adopting the notices of
/// `D-a-notice-reaches-the-transcript`.
fn build_config(config: &rho_config::Config) -> anyhow::Result<SessionConfig> {
    let mut notices = Vec::new();
    let built = build_config_with_notices(config, &mut notices)?;
    for notice in notices {
        eprintln!("rho: {notice}");
    }
    Ok(built)
}

/// Build a `SessionConfig`, and collect every notice instead of printing it. State every
/// choice. A notice is data here, so a frontend can draw it where the user is looking.
/// See `D-a-notice-reaches-the-transcript`.
fn build_config_with_notices(
    config: &rho_config::Config,
    notices: &mut Vec<String>,
) -> anyhow::Result<SessionConfig> {
    // A model comes from the flag, then the environment, then the provider's default.
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
                notices.push(format!(
                    "no model given, so using the default for {provider_name}: {model}. \
                     Set --model or {MODEL_ENV} to choose another."
                ));
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
    mut config: SessionConfig,
) -> anyhow::Result<(Session, Arc<rho_core::TaskRegistry>, SessionExtras)> {
    let name = provider::resolve_provider_name(loaded.provider.as_deref(), None)?;
    // Compute the wiring notices before building the provider, and surface them even when the
    // build fails. A user whose untrusted `base-url` was dropped, and who has no credential,
    // otherwise saw only the credential error and never learned the base-url was ignored. The
    // notice is the security-relevant half, so it must not be lost to an unrelated failure.
    // See D6.
    let wiring_notices = wiring_notices(loaded, &name);
    let provider = match provider::build_provider(&name, loaded.base_url.as_deref()) {
        Ok(provider) => provider,
        Err(error) => {
            // Surface what rho already decided before the failure, so a user whose untrusted
            // `base-url` was dropped, and who has no credential, learns it instead of seeing
            // only the credential error. The notice is the security-relevant half, so it
            // rides along with the fatal error rather than being lost. See D6.
            let mut message = format!("{error}");
            for notice in &wiring_notices {
                message.push_str(&format!("\nrho: {notice}"));
            }
            return Err(anyhow::anyhow!(message));
        }
    };
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

    // The result store. The cap already runs without it; the store is what makes the tail
    // readable and shrinks the preview from 64 KiB to 4 KiB. See SPEC-tool-result-handle.
    let (results_dir, result_notices) = open_result_store().await;
    if let Some((store, _)) = &results_dir {
        config = config.with_result_policy(rho_core::ResultPolicy {
            store: Some(Arc::clone(store)),
            ..rho_core::ResultPolicy::default()
        });
    }

    let mut registry =
        rho_tools::builtin_registry_with_tasks_and_sandbox(Arc::clone(&tasks), config.sandbox);
    for tool in &extensions.mcp_tools {
        registry.register(Arc::clone(tool));
    }
    // Registered only when a store exists. A tool that always fails would spend schema bytes
    // every turn and teach the model a capability rho does not have.
    if let Some((store, _)) = &results_dir {
        registry.register(Arc::new(rho_tools::ReadToolResultTool::new(
            Arc::clone(store),
            config.results.limits,
        )));
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
        agents: {
            // The caller states where to look, so `subagents::load` reads no environment.
            let mut agents =
                rho_skills::AgentConfig::with_default_user_dirs(config.session_root.clone());
            agents.project_trusted = cli.trust_project;
            // An agent definition answers to its own switch. `--no-skills` stops the skill
            // search only, because it used to remove `spawn_agent` in silence. See
            // `D-skills-and-agents-are-two-switches`.
            agents.discover = loaded.discover_agents;
            agents
        },
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
    let prompt = assemble_prompt(
        &system_prompt(),
        &extensions.instructions_prompt,
        &extensions.skills_prompt,
    );

    let context = Context::new(Some(prompt), tools.specs());
    Ok((
        Session::with_config(config, provider, tools, hooks, context),
        tasks,
        SessionExtras {
            notices: wiring_notices
                .into_iter()
                .chain(extensions.notices)
                .chain(subagents.notices)
                .chain(result_notices)
                .collect(),
            // Holding the guard keeps the directory alive for the session, and removes it when
            // the session ends. See D-stored-result-inherits-session-trust.
            results_dir: results_dir.map(|(_, guard)| guard),
            agents: subagents.registry,
            agent_definitions: subagents.loaded,
            mcp_pool: extensions.mcp_pool,
        },
    ))
}

/// What a caller must hold, and what it should show the user.
struct SessionExtras {
    /// The result store directory. Dropping it removes the stored results.
    #[allow(dead_code)]
    results_dir: Option<tempfile::TempDir>,
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

/// Open a private directory for this session's stored tool results.
///
/// `rho-cli` writes no session file yet, so there is nothing to sit beside. A temporary
/// directory with owner-only permissions has the same privacy and the same lifetime, and it is
/// removed when the session ends. See D-stored-result-inherits-session-trust.
///
/// A failure is never fatal. Without a store the cap still runs, so the model still cannot
/// flood the context; it only loses the ability to read a tail.
async fn open_result_store() -> (
    Option<(Arc<dyn rho_core::ResultStore>, tempfile::TempDir)>,
    Vec<String>,
) {
    let guard = match tempfile::Builder::new().prefix("rho-results-").tempdir() {
        Ok(guard) => guard,
        Err(error) => {
            return (
                None,
                vec![format!(
                    "cannot open a result store: {error}. A large tool result will be cut \
                     instead of stored."
                )],
            );
        }
    };
    // Owner only. A stored result can hold whatever a tool read.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) =
            std::fs::set_permissions(guard.path(), std::fs::Permissions::from_mode(0o700))
        {
            return (
                None,
                vec![format!(
                    "cannot make the result store private: {error}. No result will be stored."
                )],
            );
        }
    }
    match rho_core::FileResultStore::open(guard.path()).await {
        Ok(store) => (
            Some((Arc::new(store) as Arc<dyn rho_core::ResultStore>, guard)),
            Vec::new(),
        ),
        Err(error) => (
            None,
            vec![format!(
                "cannot open a result store: {error}. A large tool result will be cut instead \
                 of stored."
            )],
        ),
    }
}

/// Join the three parts of the stable prefix, in the one fixed order.
///
/// The order is the rho prompt, then project instructions, then skills. An instruction is a
/// rule that governs a choice, and a skill is a capability the model may choose, so the rule
/// comes first. The order is fixed because reordering changes the prefix bytes and costs the
/// provider prompt cache. See SPEC-project-instructions section 6 and
/// F-stable-prefix-for-kv-cache.
///
/// An empty part contributes nothing, not even a blank line, so a session with no
/// instructions sends the same bytes it sent before this feature existed.
fn assemble_prompt(system: &str, instructions: &str, skills: &str) -> String {
    let mut prompt = String::from(system);
    for block in [instructions, skills] {
        if !block.is_empty() {
            prompt.push_str("\n\n");
            prompt.push_str(block);
        }
    }
    prompt
}

/// Run the CLI. Return the process exit code.
pub async fn run(cli: Cli) -> i32 {
    match &cli.command {
        Some(Command::Run {
            prompt,
            continue_session,
            ephemeral,
            allow_widen,
        }) => {
            let selector = SessionSelector::from_flag(continue_session.as_deref());
            // A space instead of an equals sign continued the wrong session and sent the id to
            // the model. It was measured. See `SPEC-session-store-wiring` section 8c.
            if let Err(error) = recording::refuse_an_id_shaped_prompt(prompt, &selector) {
                return fail(error);
            }
            let request = RunRequest {
                prompt: prompt.clone(),
                selector,
                ephemeral: *ephemeral,
                allow_widen: *allow_widen,
            };
            run_headless(&cli, request).await
        }
        Some(Command::Sessions { action }) => run_sessions(&cli, action),
        None => run_interactive(&cli).await,
    }
}

/// What one headless run needs beyond the flags every mode shares.
struct RunRequest {
    prompt: String,
    selector: SessionSelector,
    ephemeral: bool,
    allow_widen: bool,
}

/// The wall clock, as epoch milliseconds.
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or(0)
}

/// The home directory the session store sits under.
///
/// It reads `HOME`, and falls back to the working directory, so a run without `HOME` still
/// records rather than losing the session in silence.
fn home_dir() -> PathBuf {
    match std::env::var("HOME") {
        Ok(home) if !home.trim().is_empty() => PathBuf::from(home),
        _ => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    }
}

/// Open the session this run records into.
///
/// A failure to open a **new** session degrades to ephemeral with a warning, because a session
/// file is not worth ending a run for. See `D-write-failure-degrades`. A failure to open a
/// session the user **named** is fatal, because rho would otherwise answer from a blank
/// conversation while the user believes it continued theirs.
fn open_recording(
    loaded: &rho_config::Config,
    config: &SessionConfig,
    provider_name: &str,
    request: &RunRequest,
    home: &Path,
) -> Result<Recording, anyhow::Error> {
    let approval = approval_name(loaded);
    let sandbox = config.sandbox.as_str().to_string();
    let opened = recording::open(RecordingRequest {
        project_root: &config.session_root,
        home,
        session_file: loaded.session_file.as_deref(),
        ephemeral: request.ephemeral || loaded.ephemeral,
        selector: request.selector.clone(),
        allow_widen: request.allow_widen,
        approval: &approval,
        sandbox: &sandbox,
        provider: provider_name,
        model: &config.model,
        now_millis: now_millis(),
    });
    match opened {
        Ok(recording) => Ok(recording),
        // **Only a write failure degrades.** A session file is not worth ending a run for, and that
        // is what `D-write-failure-degrades` says: a *write failure*. Everything else on this path
        // is a refusal, and a refusal exists to protect the user.
        //
        // The rule names what degrades, and not what stops. A list of errors that stop the run is a
        // fail-open shape, because the next variant joins the degrade by default. A first version
        // was that list, and a live drive then showed a widen refusal becoming a warning:
        //
        //     rho: cannot open a session file: a resume would widen approval from read-only to
        //     allow-all; pass --allow-widen to allow it. This run is ephemeral.
        //
        // A resume never degrades at all, because a user who asked for their conversation must not
        // get a blank one instead.
        Err(error) if request.selector.resumes() => Err(error),
        Err(error) => match error.downcast_ref::<rho_core::SessionError>() {
            Some(rho_core::SessionError::Io(_)) => Ok(Recording::degraded(format!(
                "cannot open a session file: {error}. This run is ephemeral."
            ))),
            _ => Err(error),
        },
    }
}

/// The approval mode name this run resolved to.
///
/// It goes into a new header, and a resume compares it against the stored one. The name must
/// match `StoredApproval::parse`, so it comes from the config enum and never from a literal at
/// the call site.
fn approval_name(loaded: &rho_config::Config) -> String {
    match loaded.approval {
        Some(rho_config::ApprovalMode::ReadOnly) => "read-only",
        Some(rho_config::ApprovalMode::Ask) => "ask",
        Some(rho_config::ApprovalMode::AllowAll) | None => "allow-all",
    }
    .to_string()
}

/// Run one `rho sessions` action. It sends nothing to a model.
fn run_sessions(cli: &Cli, action: &SessionsAction) -> i32 {
    let loaded = match load_config(cli) {
        Ok(loaded) => loaded,
        Err(error) => return fail(error),
    };
    let root = match loaded.session_root.clone() {
        Some(path) => path,
        None => match std::env::current_dir() {
            Ok(path) => path,
            Err(error) => {
                return fail(anyhow::anyhow!(
                    "cannot read the current directory: {error}"
                ));
            }
        },
    };
    match sessions_command::run(&home_dir(), &root, action, now_millis()) {
        Ok(text) => {
            print!("{text}");
            0
        }
        Err(error) => fail(error),
    }
}

/// Run one prompt headless. Print the answer to stdout. Print diagnostics to
/// stderr. Return a non-zero code on failure.
async fn run_headless(cli: &Cli, request: RunRequest) -> i32 {
    // The configuration loads once, here, before a session exists.
    let loaded = match load_config(cli) {
        Ok(loaded) => loaded,
        Err(error) => return fail(error),
    };
    let config = match build_config(&loaded) {
        Ok(config) => config,
        Err(error) => return fail(error),
    };
    let provider_name = match provider::resolve_provider_name(loaded.provider.as_deref(), None) {
        Ok(name) => name,
        Err(error) => return fail(anyhow::anyhow!(error)),
    };
    // The session file opens **before** the provider runs, so a resume that would widen a
    // permission stops before a single token is spent.
    let mut recording =
        match open_recording(&loaded, &config, &provider_name, &request, &home_dir()) {
            Ok(recording) => recording,
            Err(error) => return fail(error),
        };
    for notice in &recording.notices {
        eprintln!("rho: {notice}");
    }
    // Name the session and its file. A user needs the id for `--resume=<id>` and the path for
    // anything else, and a run that printed neither left the feature invisible.
    if let (Some(id), Some(path)) = (&recording.id, &recording.path) {
        eprintln!("rho: session {} at {}", id.as_str(), path.display());
    }
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
    // A resume replays the rebuilt conversation into the context before the prompt goes out.
    if !recording.messages.is_empty() {
        session
            .replay(std::mem::take(&mut recording.messages))
            .await;
    }

    let input = vec![ContentBlock::Text {
        text: request.prompt,
    }];
    let cancel = CancelToken::new();
    let mut events = session.prompt(input.clone(), cancel);
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    // `record_and_print` owns the recording lifecycle: it records the prompt, folds every event,
    // and closes the session. `main` added the MCP drain after the run, and it has to stay after,
    // because dropping `extras` stops every server. So the close happens first and the drain second;
    // the two touch different files and neither waits on the other.
    let code = record_and_print(
        &mut events,
        &mut recording,
        &input,
        &mut stdout,
        &mut stderr,
        reasoning_is_shown(loaded.reasoning),
    )
    .await;

    // Drain the MCP connect tasks before the process exits. A connect writes the schema cache
    // on a detached task, and a fast `rho run` finished its turn and exited before that task
    // ran, so the cache was never written and every run told the user the tools arrive next
    // session, forever. The drain awaits the write, bounded so a stuck server cannot hang the
    // exit. It runs before `extras` drops, because dropping the pool stops every server. See
    // A1 and `docs/verification/mcp-live-probe.md`.
    drain_mcp(&extras).await;
    code
}

/// The bound on the MCP connect drain at shutdown.
///
/// A connect that has not finished within this window is left detached, and a lapsed timeout
/// is not an error, so a slow or dead server never hangs the exit. See A1 and
/// [`rho_mcp::McpPool::drain_connects`].
const MCP_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Await the outstanding MCP connect tasks, then surface any cache notice.
///
/// A cache write happens on a background task, so a caller must await it before exit or the
/// write races the process end. A write that still failed reaches the user here, because the
/// handshake already returned and could not report it inline. See A1 and A6. The alternate
/// screen is closed by the time this runs on the interactive path, so stderr is visible.
async fn drain_mcp(extras: &SessionExtras) {
    if let Some(pool) = &extras.mcp_pool {
        pool.drain_connects(MCP_DRAIN_TIMEOUT).await;
        for notice in pool.take_cache_notices() {
            eprintln!("rho: {notice}");
        }
    }
}

/// Record the prompt, print the run, and close the session.
///
/// **This is the seam.** `run_headless` builds the stream and calls this once, so the whole
/// recording lifecycle is one function a test can drive. A reviewer showed that the guard which
/// greps this file passes against a call moved into `if false`, and a grep cannot see reachability,
/// order, or an argument. This function is the behavioural answer, and
/// `the_whole_lifecycle_runs_in_order` drives it.
///
/// The order is the contract: the prompt is recorded before the answer arrives, every event is
/// folded, and the close is last. A run that ended on its own closes its session, so a crash offer
/// can tell a clean exit from a crash. A cancel does not reach here, per
/// `D-cancel-keeps-the-session-open`.
async fn record_and_print<S, O: Write, E: Write>(
    events: &mut S,
    recording: &mut Recording,
    input: &[ContentBlock],
    stdout: &mut O,
    stderr: &mut E,
    show_reasoning: bool,
) -> i32
where
    S: futures::Stream<Item = Result<AgentEvent, rho_core::Error>> + Unpin,
{
    recording.start(input);
    let code = print_run(events, stdout, stderr, show_reasoning, Some(recording)).await;
    recording.close();
    code
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
    mut recording: Option<&mut Recording>,
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
        // The recorder folds every event into records. It sees each one before the printer, so a
        // print that panicked could not lose a record it already had.
        if let (Some(recording), Ok(event)) = (recording.as_deref_mut(), item.as_ref()) {
            recording.observe(event);
        }
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
#[cfg(feature = "tui")]
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
#[cfg(feature = "tui")]
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
/// This is the one place the process environment is read. A test never reads the real
/// environment, because every caller takes the list as data.
///
/// It carried a `tui` gate, because only the interface read the environment. `load_config` reads
/// it now, on every path, so the gate broke the minimal build. The gate command in `AGENTS.md`
/// builds that profile and never tests it, which is why only a build caught this.
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
        // Two flags over one boolean, so an unset pair writes nothing and a file still
        // decides. `--no-mouse` wins, per `D-the-wheel-needs-capture`, and layer 6 must not
        // send a value the user never asked for, per `SPEC-config-call-site` rule 6.
        tui_mouse: match (cli.mouse, cli.no_mouse) {
            (_, true) => Some(false),
            (true, false) => Some(true),
            (false, false) => None,
        },
        tui_reasoning: cli.reasoning.clone(),
        reasoning_effort: cli.reasoning_effort.clone(),
        no_skills: cli.no_skills,
        no_agents: cli.no_agents,
        // `--no-motion` is the negation of `tui-motion`, so it flips the value. An unpassed
        // flag writes nothing, so a file still decides, per `SPEC-config-call-site` rule 6.
        // `--no-motion=false` turns motion back on, so a global `tui-motion = false` can be
        // overridden from the command line. See D9.
        tui_motion: cli.no_motion.map(|off| !off),
        base_url: cli.base_url.clone(),
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

/// The root that locates the project file: `--root`, then a **trusted** `RHO_SESSION_ROOT`,
/// then the working directory.
///
/// A `session-root` key inside a file sets the root for tools. It never moves the project
/// file that was already read, because that would be circular.
///
/// `RHO_SESSION_ROOT` chooses which directory's `.rho/config.toml` rho reads, so it is a
/// discovery input, not only a confinement input. An untrusted environment must not redirect
/// discovery: a critic set it to a directory holding a hostile `config.toml`, and that file's
/// `model` reached the provider while the trust notice claimed the key was ignored. The
/// config crate already clears the confinement effect of an untrusted `session_root`; this
/// closes the discovery half. So the variable moves the project file only with
/// `--trust-project`. See D1 and `D-project-skill-needs-trust`.
fn bootstrap_root(cli: &Cli, env: &[(String, String)]) -> anyhow::Result<PathBuf> {
    if let Some(path) = &cli.root {
        return Ok(path.clone());
    }
    if cli.trust_project
        && let Some((_, value)) = env.iter().find(|(name, _)| name == "RHO_SESSION_ROOT")
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
    rho_config::Config::load(&sources).map_err(explain_config_error)
}

/// Turn a config load failure into a user-facing error.
///
/// A base-url refusal carries a `BaseUrlRejection`, which separates a refusal that protects
/// the credential (a user or password in the url, or plain http to a remote host) from one
/// that corrects a typo (not a url, a query or fragment, an unknown scheme). rho-cli words
/// the two apart: a safety block says rho blocked the value to protect the key, and a typo
/// says the value is malformed. Without this, both read as one flat "is not valid" message
/// and a user cannot tell a deliberate block from a mistake. This is the one production
/// caller of `BaseUrlRejection::is_safety_block`; the config crate keeps the distinction, and
/// the caller is where it reaches the user. See comment 2.
fn explain_config_error(error: rho_config::ConfigError) -> anyhow::Error {
    match error {
        rho_config::ConfigError::BaseUrl { value, reason } if reason.is_safety_block() => {
            anyhow::anyhow!(
                "rho blocked the base-url value \"{value}\" to protect your credential: \
                 {reason}. Unset base-url, or use an endpoint rho can trust."
            )
        }
        rho_config::ConfigError::BaseUrl { value, reason } => anyhow::anyhow!(
            "the base-url value \"{value}\" is malformed: {reason}. Fix it, or unset base-url \
             to use the default endpoint."
        ),
        other => anyhow::Error::new(other),
    }
}

/// Whether the TUI captures the mouse.
///
/// The merge decides it now: the flag, then the variable, then the file, then off. The old
/// note here said a config file never reached this, which stopped being true when the call
/// site landed. See `SPEC-config-call-site`.
/// Whether the TUI captures the mouse. `--no-mouse` wins, then `--mouse`, then the
/// environment, then the default, which is on.
///
/// The default flipped with `D-the-wheel-needs-capture`. rho owns the alternate screen, and
/// Run the interactive TUI. Return a non-zero code on failure.
#[cfg(feature = "tui")]
async fn run_interactive(cli: &Cli) -> i32 {
    // The configuration loads once, here, before a session exists.
    let loaded = match load_config(cli) {
        Ok(loaded) => loaded,
        Err(error) => return fail(error),
    };
    // Collect the notices instead of printing them. A print here lands on the primary
    // screen, and rho opens the alternate screen over it a few milliseconds later.
    let mut notices: Vec<String> = Vec::new();
    let config = match build_config_with_notices(&loaded, &mut notices) {
        Ok(config) => config,
        Err(error) => return fail(error),
    };
    let model = config.model.clone();
    let provider_name = provider::resolve_provider_name(loaded.provider.as_deref(), None)
        .unwrap_or_else(|_| String::new());
    // The interface reads the merged configuration, so a config file reaches both switches.
    // The other branch read the flag and the variable here, through `resolve_mouse`. The
    // merge keeps the merged read, because a file must reach the interface as well. See
    // `SPEC-config-call-site`.
    let mouse = loaded.tui_mouse;
    let reasoning = loaded.reasoning;
    // A non-terminal stdout is the one condition the interface reads for itself. The flag,
    // the config key, and `RHO_REDUCE_MOTION` all arrive through the merge as `tui_motion`,
    // so there is one path and not two. A review found the second path was dead: production
    // hard-coded its input to false and only a test ever set it.
    let motion = rho_tui::motion_enabled(rho_tui::MotionInputs {
        tui_motion: loaded.tui_motion,
        stdout_is_terminal: std::io::IsTerminal::is_terminal(&std::io::stdout()),
    });
    // Hold `_tasks` and `_extras` for the whole run. Dropping the task registry kills
    // every background task, and dropping the MCP pool stops every server, so an early
    // drop would end work the model is still waiting on.
    let (session, _tasks, extras) = match build_session(cli, &loaded, config).await {
        Ok(triple) => triple,
        Err(error) => return fail(error),
    };
    // The notices go to the interface, not to stderr. rho used to print them here and then
    // open the alternate screen over them, so the user never read one. One of them says a
    // project skill stays unloaded until the user trusts it, which is a security notice.
    // See `D-a-notice-reaches-the-transcript`.
    notices.extend(extras.notices.iter().cloned());

    // The banner names where this session runs. Without it the banner drew separators
    // around three empty fields, because nothing ever wrote them.
    let cwd = display_cwd();
    let branch = git_branch();
    let mut app = rho_tui::App::new(session, model)
        .with_mouse(mouse)
        // The renderer read `state.animate` and nothing ever assigned it, so the sweep
        // never drew. See `D-motion-answers-to-one-switch`.
        .with_motion(motion)
        .with_reasoning(reasoning)
        .with_context(cwd, branch, provider_name)
        .with_notices(notices);
    let code = match app.run().await {
        Ok(()) => 0,
        Err(error) => fail(anyhow::anyhow!(error)),
    };
    // Drain the MCP connect tasks before the process exits, so the schema cache write is not
    // lost to a fast exit. The interface has closed the alternate screen by now, so a cache
    // notice on stderr is visible again. See A1 and `D-a-notice-reaches-the-transcript`.
    drain_mcp(&extras).await;
    code
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

/// What rho tells the user about a switch it obeyed.
///
/// A base url redirects the credential, so rho says where the key is going. A silent
/// redirect is the defect. See `D-a-provider-base-url-is-a-config-key`. Agent discovery
/// says so too, because a missing `spawn_agent` otherwise reads as a broken feature.
fn wiring_notices(loaded: &rho_config::Config, provider_name: &str) -> Vec<String> {
    let mut notices = Vec::new();
    if let Some(url) = &loaded.base_url {
        // A base url redirects the credential only for a provider that accepts one. Bedrock
        // and azure refuse a base url (`provider::refuse_base_url`), so the key never travels
        // and a notice naming a key would name the wrong secret. A security notice that names
        // a key that stays home teaches the user to distrust the notices. So this notice, and
        // the key it names, fire only for the OpenAI-compatible provider. See comment 1.
        if provider::accepts_base_url(provider_name) {
            let host = url::Url::parse(url)
                .ok()
                .and_then(|parsed| parsed.host_str().map(str::to_string))
                .unwrap_or_else(|| url.clone());
            notices.push(format!(
                "base-url is set, so {} goes to {host}. Unset base-url to use the default endpoint.",
                provider::OPENROUTER_KEY_ENV
            ));
        }
    }
    if !loaded.dropped_keys.is_empty() {
        notices.push(format!(
            "this project is not trusted, so rho ignored {}. Pass --trust-project to use them.",
            loaded.dropped_keys.join(", ")
        ));
    }
    if !loaded.discover_agents {
        notices.push(
            "agent discovery is off, so rho offers no subagent. Unset no-agents to use one."
                .to_string(),
        );
    }
    notices
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
    let child_timeout = cli
        .child_timeout_secs
        .map(std::time::Duration::from_secs)
        .unwrap_or(stated.child_timeout);
    rho_core::SubagentLimits {
        max_depth: 1,
        max_children_per_parent: cli
            .max_children_per_parent
            .unwrap_or(stated.max_children_per_parent),
        max_live_total: cli.max_live_agents.unwrap_or(stated.max_live_total),
        max_tool_calls: cli.max_agent_tool_calls.unwrap_or(stated.max_tool_calls),
        grace_turns: cli.agent_grace_turns.unwrap_or(stated.grace_turns),
        max_queued_per_parent: cli
            .max_queued_per_parent
            .unwrap_or(stated.max_queued_per_parent),
        max_queued_total: cli.max_queued_total.unwrap_or(stated.max_queued_total),
        // An unset deadline follows the child timeout, so a waiter gets one whole sibling
        // run of patience. A fixed default would time out every waiter of a longer child.
        queue_wait: cli
            .queue_wait_secs
            .map(std::time::Duration::from_secs)
            .unwrap_or(child_timeout),
        max_steer_message_bytes: cli
            .max_agent_steer_bytes
            .unwrap_or(stated.max_steer_message_bytes),
        child_timeout,
    }
}

#[cfg(test)]
mod tests {
    // ---- the result store, wired ----

    /// The store must really open, and it must be private. Without this the feature is a
    /// library nobody calls, which is the defect `F-layered-config` already has.
    #[tokio::test]
    async fn the_result_store_opens_and_is_owner_only() {
        let (opened, notices) = open_result_store().await;

        let (store, guard) = opened.expect("a store must open on a normal host");
        assert!(notices.is_empty(), "{notices:?}");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(guard.path())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o777,
                0o700,
                "a stored result can hold what a tool read"
            );
        }

        // It is a working store, not just a directory.
        let handle = store.put("evidence").await.unwrap();
        let slice = store.read_range(&handle, 0, 64).await.unwrap();
        assert_eq!(slice.text, "evidence");
    }

    /// Dropping the guard must remove the directory, so a stored result does not outlive its
    /// session.
    #[tokio::test]
    async fn the_result_store_is_removed_with_its_session() {
        let (opened, _) = open_result_store().await;
        let (store, guard) = opened.unwrap();
        let path = guard.path().to_path_buf();
        store.put("evidence").await.unwrap();
        assert!(path.is_dir());

        drop(guard);

        assert!(
            !path.exists(),
            "a stored result must not outlive its session"
        );
    }

    /// A session with a store must advertise `read_tool_result`, and one without must not.
    #[tokio::test]
    async fn read_tool_result_is_advertised_only_with_a_store() {
        let tasks = Arc::new(rho_core::TaskRegistry::new(rho_core::TaskLimits::default()));
        let mut with_store = rho_tools::builtin_registry_with_tasks_and_sandbox(
            Arc::clone(&tasks),
            rho_core::SandboxMode::Off,
        );
        let without: Vec<String> = with_store.specs().iter().map(|s| s.name.clone()).collect();
        assert!(
            !without.contains(&"read_tool_result".to_string()),
            "a tool that always fails must not be advertised"
        );

        // The same registration the session build does.
        let dir = tempfile::tempdir().unwrap();
        let store: Arc<dyn rho_core::ResultStore> =
            Arc::new(rho_core::FileResultStore::open(dir.path()).await.unwrap());
        with_store.register(Arc::new(rho_tools::ReadToolResultTool::new(
            store,
            rho_core::ResultLimits::default(),
        )));
        let names: Vec<String> = with_store.specs().iter().map(|s| s.name.clone()).collect();
        assert!(
            names.contains(&"read_tool_result".to_string()),
            "with a store the tool must reach the model: {names:?}"
        );
    }

    // ---- the stable prefix, and its fixed order ----

    #[test]
    fn project_instructions_precede_skills_in_the_prefix() {
        let prompt = assemble_prompt(
            "SYS",
            "<project_instructions>I</project_instructions>",
            "<available_skills>S</available_skills>",
        );

        let instructions_at = prompt
            .find("project_instructions")
            .expect("instructions present");
        let skills_at = prompt.find("available_skills").expect("skills present");
        assert!(
            instructions_at < skills_at,
            "a rule must precede a capability; prompt was:\n{prompt}"
        );
        assert!(prompt.starts_with("SYS"), "prompt was:\n{prompt}");
    }

    #[test]
    fn an_empty_block_adds_no_bytes_to_the_prefix() {
        // A session with no instructions and no skills must send exactly what it sent
        // before this feature existed. A stray blank line is a cache miss.
        assert_eq!(assemble_prompt("SYS", "", ""), "SYS");
        assert_eq!(assemble_prompt("SYS", "I", ""), "SYS\n\nI");
        assert_eq!(assemble_prompt("SYS", "", "S"), "SYS\n\nS");
    }

    #[test]
    fn the_prefix_is_byte_identical_across_calls() {
        let first = assemble_prompt("SYS", "I", "S");
        let second = assemble_prompt("SYS", "I", "S");
        assert_eq!(first, second);
    }

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
    pub(super) fn try_load(
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
            "--max-queued-per-parent",
            "5",
            "--max-queued-total",
            "11",
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
        // A full wait line tells the user to raise one of these two flags. A flag that
        // parses and changes nothing teaches a lie, which is the exact defect a live
        // sweep found in the older limits. See `SubagentError::QueueFull`.
        assert_eq!(
            limits.max_queued_per_parent, 5,
            "a wait line the caller cannot bound is not bounded by the caller"
        );
        assert_eq!(
            limits.max_queued_total, 11,
            "the process wait line must obey its flag too"
        );
    }

    #[test]
    fn the_grace_flag_reaches_the_limits() {
        let cli = Cli::parse_from(["rho", "--agent-grace-turns", "2"]);
        assert_eq!(subagent_limits(&cli).grace_turns, 2);
    }

    #[test]
    fn the_grace_window_defaults_to_the_stated_subagent_value() {
        let cli = Cli::parse_from(["rho"]);
        assert_eq!(
            subagent_limits(&cli).grace_turns,
            rho_core::DEFAULT_SUBAGENT_GRACE_TURNS,
            "a child is warned by default, because it has nobody to ask for more turns"
        );
    }

    #[test]
    fn the_queue_wait_flag_reaches_the_limits() {
        // A refusal names this flag, so the flag has to change the deadline.
        let cli = Cli::parse_from(["rho", "--queue-wait-secs", "30"]);
        assert_eq!(
            subagent_limits(&cli).queue_wait,
            std::time::Duration::from_secs(30)
        );
    }

    #[test]
    fn an_unset_queue_wait_follows_the_child_timeout() {
        // A waiter gets one whole sibling run of patience. So a host that lengthens a
        // child run lengthens the patience with it. See decision D-a-waiter-has-a-deadline.
        let cli = Cli::parse_from(["rho", "--child-timeout-secs", "900"]);
        let limits = subagent_limits(&cli);
        assert_eq!(
            limits.queue_wait,
            std::time::Duration::from_secs(900),
            "an unset deadline follows the child timeout, and never the stated default"
        );
        let plain = subagent_limits(&Cli::parse_from(["rho"]));
        assert_eq!(plain.queue_wait, plain.child_timeout);
    }

    #[test]
    fn the_agent_steer_byte_flag_reaches_the_limits() {
        // A cap only the default constructor applied would make this flag dead surface.
        let cli = Cli::parse_from(["rho", "--max-agent-steer-bytes", "4096"]);
        assert_eq!(subagent_limits(&cli).max_steer_message_bytes, 4096);
        let plain = subagent_limits(&Cli::parse_from(["rho"]));
        assert_eq!(
            plain.max_steer_message_bytes,
            rho_core::SubagentLimits::new().max_steer_message_bytes,
            "the default stays where SubagentLimits states it"
        );
    }

    #[test]
    fn the_grace_warning_can_be_turned_off_from_the_command_line() {
        let cli = Cli::parse_from(["rho", "--agent-grace-turns", "0"]);
        assert_eq!(subagent_limits(&cli).grace_turns, 0);
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
        let root = registry.new_tree();
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
    fn the_default_model_notice_is_data_and_not_a_print() {
        // The notice used to reach the user only through `eprintln!`, so the interactive
        // path printed it to the terminal and then opened the alternate screen over it.
        // A notice the interface can draw has to be a value the caller can carry.
        // See `D-a-notice-reaches-the-transcript`.
        let cli = Cli::try_parse_from(["rho", "--provider", "openrouter"]).unwrap();
        let mut notices = Vec::new();
        let config =
            build_config_with_notices(&loaded(&cli), &mut notices).expect("a default model");
        assert_eq!(config.model, "anthropic/claude-haiku-4.5");
        assert!(
            notices.iter().any(|line| line.contains("no model given")),
            "the default-model choice must arrive as data: {notices:?}"
        );
    }

    #[test]
    fn an_explicit_model_raises_no_notice() {
        // rho must not narrate a choice the user already made.
        let cli = Cli::try_parse_from(["rho", "--model", "openai/gpt-4o"]).unwrap();
        let mut notices = Vec::new();
        build_config_with_notices(&loaded(&cli), &mut notices).expect("an explicit model");
        assert!(notices.is_empty(), "no notice was needed: {notices:?}");
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
            Some(Command::Run { prompt, .. }) => assert_eq!(prompt, "hello"),
            other => panic!("expected a run command, got {other:?}"),
        }
    }

    #[test]
    fn sandbox_flag_defaults_to_off() {
        // The flag itself now writes nothing, because a clap default would beat a file.
        // The effective default is still `Off`, and it comes from the merge.
        // The default is stated in the flag definition, not hidden. See D-no-four-argument-session-new.
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

    /// A resolved config from one flag layer, so a notice test needs no file and no provider.
    fn resolved_with(flags: rho_config::ConfigLayer) -> rho_config::Config {
        let sources =
            rho_config::Sources::from_paths(rho_config::ConfigPaths::default()).with_flags(flags);
        rho_config::Config::load(&sources).expect("a flag layer resolves")
    }

    #[test]
    fn setting_a_base_url_names_the_host_in_a_notice() {
        // A silent redirect of the credential is the defect. The notice names the host and
        // the variable, so a user sees where the key is going.
        let loaded = resolved_with(rho_config::ConfigLayer {
            base_url: Some("https://models.example.com/v1".to_string()),
            ..Default::default()
        });
        let notices = wiring_notices(&loaded, "openrouter");
        assert_eq!(notices.len(), 1, "one notice: {notices:?}");
        assert!(notices[0].contains("models.example.com"), "{}", notices[0]);
        assert!(notices[0].contains("OPENROUTER_API_KEY"), "{}", notices[0]);
    }

    #[test]
    fn the_base_url_notice_matches_the_resolved_provider() {
        // Comment 1. `wiring_notices` named OPENROUTER_API_KEY for every provider. But
        // base-url is a hard error for bedrock and azure (`provider::refuse_base_url`), so the
        // credential never travels there. A security notice that names the wrong secret
        // teaches the user to distrust the notices. So the base-url notice fires only for the
        // OpenAI-compatible provider that accepts a base url, and names that provider's key.
        let loaded = resolved_with(rho_config::ConfigLayer {
            base_url: Some("https://models.example.com/v1".to_string()),
            ..Default::default()
        });

        let openrouter = wiring_notices(&loaded, "openrouter");
        assert!(
            openrouter
                .iter()
                .any(|line| line.contains("OPENROUTER_API_KEY")
                    && line.contains("models.example.com")),
            "the openai-compatible provider names its own key and the host: {openrouter:?}"
        );

        let bedrock = wiring_notices(&loaded, "bedrock");
        assert!(
            !bedrock
                .iter()
                .any(|line| line.contains("OPENROUTER_API_KEY")),
            "bedrock refuses base-url, so the credential never travels; do not name it: \
             {bedrock:?}"
        );
        assert!(
            !bedrock.iter().any(|line| line.contains("goes to")),
            "no base-url redirect notice for a provider that refuses base-url: {bedrock:?}"
        );
    }

    #[test]
    fn agent_discovery_off_is_reported() {
        // Renamed from `a_skipped_definition_is_reported`. That name promised a count of
        // skipped definitions and named a definition, and this notice does neither: when
        // `no-agents` is set, `discover_agents` returns before it scans, so there is nothing
        // to count. The honest promise is that rho says discovery is off and names the
        // switch, so a missing `spawn_agent` does not read as a broken feature. The count
        // promise in behaviour rule 6 and the spec is amended to match; see the report and
        // D7. The switch is named as `no-agents`, matching the base-url notice's phrasing
        // rather than a `--flag` a config key or `RHO_NO_AGENTS` did not use. See D8.
        let loaded = resolved_with(rho_config::ConfigLayer {
            no_agents: Some(true),
            ..Default::default()
        });
        let notices = wiring_notices(&loaded, "openrouter");
        assert!(
            notices.iter().any(|line| line.contains("no-agents")),
            "the notice names the switch: {notices:?}"
        );
        assert!(
            !notices.iter().any(|line| line.contains("--no-agents")),
            "the switch may be set by a config key or RHO_NO_AGENTS, so do not name a flag: {notices:?}"
        );
    }

    #[test]
    fn no_switch_means_no_wiring_notice() {
        // A notice a user did not ask for is noise, so the quiet path stays quiet.
        assert!(
            wiring_notices(
                &resolved_with(rho_config::ConfigLayer::default()),
                "openrouter"
            )
            .is_empty()
        );
    }

    #[tokio::test]
    async fn a_wiring_notice_survives_a_provider_build_failure() {
        // D6. `wiring_notices` used to run after `build_provider`, so a user whose provider
        // build failed never learned that rho had dropped a switch. The notice must reach
        // the user even on the failure path.
        //
        // The deterministic failure is a base-url conflict: bedrock refuses a base url with
        // no credential and no network. The base-url flag is trusted, so it survives to
        // `build_provider` and triggers the conflict. An untrusted project file also sets a
        // powerful `skill-paths` key, which the strip drops. The dropped-key notice is the
        // security-relevant half D6 protects, so it must ride along with the fatal error, not
        // be lost to it.
        //
        // This also pins comment 1: the surviving notice must not name OPENROUTER_API_KEY,
        // because bedrock refuses the base url and the openrouter key never travels.
        let root = tempfile::tempdir().unwrap();
        write_project(root.path(), "skill-paths = [\"/tmp/evil\"]\n");
        let cli = Cli::try_parse_from([
            "rho",
            "--provider",
            "bedrock",
            "--base-url",
            "https://models.example.com/v1",
            "--model",
            "m",
        ])
        .unwrap();
        let loaded = try_load_in(&cli, &[], &[], root.path()).unwrap();
        assert!(
            loaded
                .dropped_keys
                .iter()
                .any(|key| key.contains("skill-paths")),
            "an untrusted powerful key must be recorded as dropped: {:?}",
            loaded.dropped_keys
        );
        let config = build_config(&loaded).expect("bedrock has a default model");
        let error = match build_session(&cli, &loaded, config).await {
            Ok(_) => panic!("a base url with bedrock is a conflict"),
            Err(error) => error.to_string(),
        };
        assert!(
            error.contains("not trusted") && error.contains("skill-paths"),
            "the dropped-key wiring notice, not just the provider error, must reach the user on \
             a build failure: {error}"
        );
        assert!(
            !error.contains("OPENROUTER_API_KEY"),
            "bedrock refuses base-url, so the surviving notice must not name the openrouter \
             key: {error}"
        );
    }

    /// Load a config whose only fault is a refused base-url flag, and return the user-facing
    /// error text. The base-url flag is trusted, so it survives to validation. A temp root
    /// keeps the filesystem isolated.
    fn base_url_error(url: &str) -> String {
        let root = tempfile::tempdir().unwrap();
        let cli = Cli::try_parse_from(["rho", "--base-url", url, "--model", "m"]).unwrap();
        match try_load_in(&cli, &[], &[], root.path()) {
            Ok(_) => panic!("the base url {url} must be refused"),
            Err(error) => error.to_string(),
        }
    }

    #[test]
    fn a_base_url_safety_block_reads_differently_from_a_typo() {
        // Comment 2. `ConfigError::BaseUrl` carries a `BaseUrlRejection` that separates a
        // refusal which protects the credential (a user or password in the url, plain http to
        // a remote host) from one that corrects a typo (not a url, a query or fragment, an
        // unknown scheme). rho-cli must word the two apart: a safety block says rho blocked
        // the value to protect the key, and a typo says the value is malformed. Asserting
        // only that both mention "base-url" is the weak assertion a prior round criticised, so
        // this pins the distinct framing and that a typo never borrows the safety wording.
        let safety = base_url_error("http://user:pass@evil.example/v1");
        let typo = base_url_error("ftp://models.example.com/v1");

        assert!(
            safety.contains("to protect your credential"),
            "a safety block must say rho blocked it to protect the credential: {safety}"
        );
        assert!(
            typo.contains("malformed"),
            "a typo must say the value is malformed: {typo}"
        );
        assert!(
            !typo.contains("to protect your credential"),
            "a typo must not claim a safety block: {typo}"
        );
        assert_ne!(
            safety, typo,
            "a safety block and a typo must read differently"
        );
    }

    #[test]
    fn an_untrusted_dropped_key_is_named_in_a_notice() {
        // D5. The three original `wiring_notices` tests all used a flags layer through
        // `resolved_with`, and a flag is never stripped, so `dropped_keys` was always empty
        // and this branch had no test at all. A powerful key set by an untrusted project
        // file is the shape that populates it. rho must name the key it dropped, or a user
        // who set it once is locked out in an untrusted checkout and told nothing. That is
        // the shape a security review named. This uses `base-url`, so the dropped-key branch
        // fires without the base-url-is-set branch firing too.
        let root = tempfile::tempdir().unwrap();
        write_project(
            root.path(),
            "base-url = \"https://models.example.com/v1\"\n",
        );
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        let loaded = try_load_in(&cli, &[], &[], root.path()).unwrap();
        assert!(
            !loaded.dropped_keys.is_empty(),
            "an untrusted powerful key must be recorded as dropped"
        );
        let notices = wiring_notices(&loaded, "openrouter");
        assert!(
            notices
                .iter()
                .any(|line| line.contains("not trusted") && line.contains("base-url")),
            "the notice must say why and name the dropped key: {notices:?}"
        );
        assert!(
            !notices.iter().any(|line| line.contains("goes to")),
            "a dropped base-url must not also fire the base-url-is-set notice: {notices:?}"
        );
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
            // The three flags this sprint added. Without them here, dropping a mapping in
            // `flag_layer` would leave the flag silently doing nothing, which is the whole
            // defect class the sprint exists to kill. A test review found the omission.
            "--no-motion",
            "--no-agents",
            "--base-url",
            "http://localhost:11434/v1",
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
        assert_eq!(
            layer.tui_motion,
            Some(false),
            "--no-motion maps onto tui-motion"
        );
        assert_eq!(
            layer.no_agents,
            Some(true),
            "--no-agents maps onto no-agents"
        );
        assert_eq!(
            layer.base_url.as_deref(),
            Some("http://localhost:11434/v1"),
            "--base-url maps onto base-url"
        );
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

    // ---- D9: --no-motion takes an optional value, like --no-skills and --no-agents ----

    #[test]
    fn no_motion_bare_turns_motion_off() {
        let cli = Cli::try_parse_from(["rho", "--model", "m", "--no-motion"]).unwrap();
        assert_eq!(cli.no_motion, Some(true));
        assert_eq!(
            flag_layer(&cli).tui_motion,
            Some(false),
            "a bare --no-motion turns the animation off"
        );
    }

    #[test]
    fn no_motion_false_turns_motion_back_on() {
        // A bare `bool` could not parse this at all, and a global `tui-motion = false` could
        // then never be turned back on from the command line. See D9.
        let cli = Cli::try_parse_from(["rho", "--model", "m", "--no-motion", "false"]).unwrap();
        assert_eq!(cli.no_motion, Some(false));
        assert_eq!(
            flag_layer(&cli).tui_motion,
            Some(true),
            "--no-motion=false means animate"
        );
    }

    #[test]
    fn an_unpassed_no_motion_flag_writes_nothing() {
        // Rule 6: an absent flag leaves layer 6 empty, so a file still decides.
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        assert_eq!(flag_layer(&cli).tui_motion, None);
    }

    #[test]
    fn no_motion_false_overrides_a_file_that_turned_motion_off() {
        // The whole reason for the shape change: the command line can beat a global
        // `tui-motion = false`. This was impossible while `--no-motion` was a bare bool.
        let home = tempfile::tempdir().unwrap();
        write_global(home.path(), "tui-motion = false\n");
        let cli = Cli::try_parse_from(["rho", "--model", "m", "--no-motion", "false"]).unwrap();
        let config = try_load(
            &cli,
            &[],
            &[("XDG_CONFIG_HOME", home.path().to_str().unwrap())],
        )
        .unwrap();
        assert!(
            config.tui_motion,
            "the command line must be able to turn motion back on"
        );
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
    fn the_mouse_is_on_by_default_through_the_merge() {
        // This test asserted the opposite, under `D-native-selection-is-the-default`, when the
        // interface drew an inline band and the terminal kept the scrollback. The merge with the
        // alternate-screen renderer reversed the default: that screen has no scrollback, so with
        // capture off the wheel does nothing. See `D-the-wheel-needs-capture`.
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        assert!(loaded(&cli).tui_mouse);
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
    fn a_bad_env_value_leaves_the_mouse_at_its_default() {
        // The layer omits an unaccepted boolean, so the value falls through to the default
        // rather than to `false`. The default is now on, per `D-the-wheel-needs-capture`, and
        // this test moved with it rather than pinning the old answer.
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        let config = try_load(&cli, &[("RHO_TUI_MOUSE", "yes please")], &[]).unwrap();
        assert!(config.tui_mouse);
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
    fn a_trusted_session_root_variable_moves_the_project_root() {
        // With trust, the variable selects which directory's config.toml rho reads.
        let cli = Cli::try_parse_from(["rho", "--model", "m", "--trust-project"]).unwrap();
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
    fn an_untrusted_session_root_variable_does_not_move_the_project_root() {
        // The D1 fix. `RHO_SESSION_ROOT` chooses which `.rho/config.toml` rho reads, so an
        // untrusted environment that set it to a hostile directory could smuggle a `model`
        // and more into the run, while the trust notice claimed the key was ignored. Without
        // `--trust-project` the variable must not redirect discovery, so the root falls
        // through to the working directory. See D1 and the claims critic's live probe.
        let cli = Cli::try_parse_from(["rho", "--model", "m"]).unwrap();
        let env = vec![(
            "RHO_SESSION_ROOT".to_string(),
            "/tmp/attacker-dir".to_string(),
        )];
        assert_eq!(
            bootstrap_root(&cli, &env).unwrap(),
            std::env::current_dir().unwrap(),
            "an untrusted session-root variable must not choose the project file"
        );
    }

    #[test]
    fn an_untrusted_session_root_variable_cannot_smuggle_a_project_config() {
        // The claims critic's live scenario, end to end. An attacker directory holds a
        // hostile `config.toml`, and `RHO_SESSION_ROOT` points at it. Composing the two
        // functions `load_config` composes, the attacker's `model` must not reach the
        // product without `--trust-project`, and it must reach it with the flag, so the fix
        // is a gate and not a wall. See D1.
        let attacker = tempfile::tempdir().unwrap();
        write_project(attacker.path(), "model = \"attacker-chose-this-model\"\n");
        let env = vec![(
            "RHO_SESSION_ROOT".to_string(),
            attacker.path().to_str().unwrap().to_string(),
        )];

        let untrusted = Cli::try_parse_from(["rho"]).unwrap();
        let root = bootstrap_root(&untrusted, &env).unwrap();
        let home: BTreeMap<String, String> = BTreeMap::new();
        let config =
            load_config_from(&untrusted, env.clone(), &root, &home).expect("the config loads");
        assert_ne!(
            config.model.as_deref(),
            Some("attacker-chose-this-model"),
            "an untrusted environment must not select the project config"
        );

        let trusted = Cli::try_parse_from(["rho", "--trust-project"]).unwrap();
        let root = bootstrap_root(&trusted, &env).unwrap();
        let config = load_config_from(&trusted, env, &root, &home).expect("the config loads");
        assert_eq!(
            config.model.as_deref(),
            Some("attacker-chose-this-model"),
            "with trust the variable does select the project config, so the gate is a gate"
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
        // The root itself is not moved, because an untrusted project file may not move the
        // confinement boundary. That assertion used to read `Some(elsewhere)`, which
        // enshrined the escape a probe later proved: a cloned repository moved the boundary
        // and read a file outside itself. A security review named this test as the place the
        // vulnerability was written down as correct. See
        // `D-trust-is-provenance-not-a-field-list` and
        // `docs/verification/profile-trust-bypass.md`.
        assert_eq!(
            config.session_root, None,
            "an untrusted project file must not move the root"
        );
    }

    #[test]
    fn a_trusted_session_root_key_still_moves_the_root() {
        // The other half, so a break that drops the key unconditionally fails.
        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        write_project(
            root.path(),
            &format!(
                "session-root = \"{}\"\n",
                elsewhere.path().to_str().unwrap()
            ),
        );
        let cli = Cli::try_parse_from(["rho", "--model", "m", "--trust-project"]).unwrap();
        let config = try_load_in(&cli, &[], &[], root.path()).unwrap();
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
            .block_on(print_run(
                &mut stream,
                &mut out,
                &mut err,
                show_reasoning,
                None,
            ));
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

#[cfg(test)]
mod headless_recording_tests {
    //! The printer folds every event into the records, driven for real.
    //!
    //! `crates/rho-cli/tests/session_cli.rs` proves `Recording::start`, `observe` and `close` on a
    //! real file, and it greps this file to prove `run_headless` calls them. A grep cannot see a
    //! call moved into an unreachable branch, and a reviewer said so. **This test drives the real
    //! `print_run` with a real `Recording` over a real store**, so the fold is behaviour and not a
    //! string.
    //!
    //! The one thing still not driven in process is `run_headless` itself, because it builds a
    //! provider and `crates/rho-cli/src/provider.rs` belongs to another lane, so this crate cannot
    //! inject a stub. `docs/verification/session-store-wiring.md` drives it against live Bedrock.

    use super::*;
    use crate::recording::{RecordingRequest, SessionSelector};
    use rho_core::{AgentEvent, Record, Role, StopReason, StreamEvent};

    /// A session over a temporary home, and the file it writes.
    fn open_one(dir: &std::path::Path) -> (crate::recording::Recording, std::path::PathBuf) {
        let root = dir.join("project");
        std::fs::create_dir_all(root.join(".git")).expect("the project root");
        let recording = crate::recording::open(RecordingRequest {
            project_root: &root,
            home: dir,
            session_file: None,
            ephemeral: false,
            selector: SessionSelector::New,
            allow_widen: false,
            approval: "read-only",
            sandbox: "off",
            provider: "testkit",
            model: "test-model",
            now_millis: 1_756_000_000_000,
        })
        .expect("a session opens");
        let path = recording.path.clone().expect("a session file");
        (recording, path)
    }

    /// One scripted turn, in the order the agent loop really emits.
    fn one_turn() -> Vec<AgentEvent> {
        vec![
            AgentEvent::TurnStart,
            AgentEvent::Stream(StreamEvent::TextStart { index: 0 }),
            AgentEvent::Stream(StreamEvent::TextDelta {
                index: 0,
                delta: "the bug is on line 42".to_string(),
            }),
            AgentEvent::Stream(StreamEvent::TextEnd { index: 0 }),
            AgentEvent::TurnEnd {
                stop_reason: StopReason::EndTurn,
            },
            AgentEvent::AgentEnd {
                stop_reason: rho_core::AgentStopReason::EndTurn,
            },
        ]
    }

    /// The loaded config and the session config a run would build, with a chosen approval mode.
    fn built(
        root: &std::path::Path,
        approval: Option<rho_config::ApprovalMode>,
    ) -> (rho_config::Config, SessionConfig) {
        let mut loaded = rho_config::Config::load(&rho_config::Sources::default())
            .expect("the built-in defaults load");
        loaded.approval = approval;
        loaded.session_root = Some(root.to_path_buf());
        let config = SessionConfig::new(
            "test-model".to_string(),
            root.to_path_buf(),
            std::sync::Arc::new(rho_core::ReadOnlyPolicy),
        );
        (loaded, config)
    }

    fn a_new_run() -> RunRequest {
        RunRequest {
            prompt: "fix the parser".to_string(),
            selector: SessionSelector::New,
            ephemeral: false,
            allow_widen: false,
        }
    }

    #[test]
    fn a_write_failure_on_a_new_session_degrades_the_run() {
        // A session file is not worth ending a run for. The store root here is a file, so the
        // directory rho needs cannot be created, and that is an io error.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join(".git")).expect("the project root");
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".rho")).expect("the home");
        std::fs::write(home.join(".rho").join("sessions"), "not a directory").expect("the blocker");
        let (loaded, config) = built(&root, None);

        let opened = open_recording(&loaded, &config, "testkit", &a_new_run(), &home)
            .expect("a write failure must not end the run");

        assert!(opened.recorder.is_ephemeral(), "the run degrades");
        assert!(
            opened.notices.iter().any(|n| n.contains("ephemeral")),
            "the user is told, got {:?}",
            opened.notices
        );
    }

    #[test]
    fn a_widen_refusal_on_a_new_session_stops_the_run() {
        // A named session file is opened with a **new** selector, so this is the case a live drive
        // found degrading: the refusal became a warning and the run continued.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join(".git")).expect("the project root");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).expect("the home");
        let named = dir.path().join("notes").join("ro.jsonl");
        let mut loaded = built(&root, Some(rho_config::ApprovalMode::ReadOnly)).0;
        loaded.session_file = Some(named.clone());
        let config = built(&root, None).1;

        // A first run writes the file under read-only.
        let first = open_recording(&loaded, &config, "testkit", &a_new_run(), &home)
            .expect("the first run opens");
        drop(first);

        // The same file, now under allow-all, which is wider.
        loaded.approval = Some(rho_config::ApprovalMode::AllowAll);
        let error = open_recording(&loaded, &config, "testkit", &a_new_run(), &home)
            .map(|_| ())
            .expect_err("a widen refusal must stop the run, and never degrade it");

        assert!(
            error.to_string().contains("--allow-widen"),
            "the refusal must reach the user, got {error}"
        );
    }

    #[test]
    fn a_busy_session_stops_the_run() {
        // The other refusal that must never become a warning. Two runs on one named file.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join(".git")).expect("the project root");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).expect("the home");
        let named = dir.path().join("notes").join("live.jsonl");
        let (mut loaded, config) = built(&root, None);
        loaded.session_file = Some(named);

        let held = open_recording(&loaded, &config, "testkit", &a_new_run(), &home)
            .expect("the first run opens");
        let error = open_recording(&loaded, &config, "testkit", &a_new_run(), &home)
            .map(|_| ())
            .expect_err("a busy session must stop the run, and never degrade it");

        assert!(
            error.to_string().contains("open in another process"),
            "the refusal must reach the user, got {error}"
        );
        drop(held);
    }

    #[test]
    fn the_whole_lifecycle_runs_in_order() {
        // `run_headless` calls `record_and_print` once, so this drives the whole recording
        // lifecycle the real run uses: the prompt, then every folded event, then the close.
        //
        // A reviewer proved that the grep guard passes against `if false { recording.start(..) }`.
        // This test does not: it reads the file back and asserts the order on disk.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (mut recording, path) = open_one(dir.path());
        let input = vec![ContentBlock::Text {
            text: "fix the parser".to_string(),
        }];
        let mut stream = futures::stream::iter(one_turn().into_iter().map(Ok));
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();

        let code = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(record_and_print(
                &mut stream,
                &mut recording,
                &input,
                &mut out,
                &mut err,
                false,
            ));

        assert_eq!(code, 0);
        assert!(String::from_utf8_lossy(&out).contains("the bug is on line 42"));
        let read = rho_core::SessionReader::read(&path).expect("the file reads back");
        let kinds: Vec<&str> = read
            .entries
            .iter()
            .map(|entry| match &entry.record {
                Record::ModelChange { .. } => "model",
                Record::Message { message } => match message.role {
                    Role::User => "prompt",
                    Role::Assistant => "answer",
                    _ => "other",
                },
                Record::Usage { .. } => "usage",
                Record::Stop { .. } => "stop",
                Record::Closed => "closed",
                Record::Reopened => "reopened",
                Record::Name { .. } => "name",
                Record::Session { .. } => "header",
            })
            .collect();
        assert_eq!(
            kinds,
            vec!["model", "prompt", "answer", "stop", "closed"],
            "the lifecycle order on disk is the prompt, the answer, and the close"
        );
    }

    #[test]
    fn the_prompt_is_recorded_before_the_answer_even_when_the_run_fails() {
        // A run that errors mid-stream still leaves the prompt on disk, so a resume knows what the
        // user asked. A lifecycle that recorded the prompt after the stream would lose it.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (mut recording, path) = open_one(dir.path());
        let input = vec![ContentBlock::Text {
            text: "fix the parser".to_string(),
        }];
        let items: Vec<Result<AgentEvent, rho_core::Error>> = vec![Err(rho_core::Error::Provider(
            rho_core::ProviderError::Transport("the provider went away".to_string()),
        ))];
        let mut stream = futures::stream::iter(items);
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();

        let code = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(record_and_print(
                &mut stream,
                &mut recording,
                &input,
                &mut out,
                &mut err,
                false,
            ));

        assert_eq!(code, EXIT_FAILURE, "a failed run exits non-zero");
        let read = rho_core::SessionReader::read(&path).expect("the file reads back");
        let prompts = read
            .entries
            .iter()
            .filter(|entry| {
                matches!(&entry.record, Record::Message { message } if message.role == Role::User)
            })
            .count();
        assert_eq!(
            prompts, 1,
            "the prompt is on disk even though the run failed"
        );
        assert!(
            matches!(read.entries.last().map(|e| &e.record), Some(Record::Closed)),
            "a failed run still closes its session, so a crash offer means a real crash"
        );
    }

    #[test]
    fn the_printer_folds_every_event_into_the_records() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join(".git")).expect("the project root");
        let mut recording = crate::recording::open(RecordingRequest {
            project_root: &root,
            home: dir.path(),
            session_file: None,
            ephemeral: false,
            selector: SessionSelector::New,
            allow_widen: false,
            approval: "read-only",
            sandbox: "off",
            provider: "testkit",
            model: "test-model",
            now_millis: 1_756_000_000_000,
        })
        .expect("a session opens");
        let path = recording.path.clone().expect("a session file");

        let input = vec![ContentBlock::Text {
            text: "fix the parser".to_string(),
        }];
        recording.start(&input);
        let events = vec![
            AgentEvent::TurnStart,
            AgentEvent::Stream(StreamEvent::TextStart { index: 0 }),
            AgentEvent::Stream(StreamEvent::TextDelta {
                index: 0,
                delta: "the bug is on line 42".to_string(),
            }),
            AgentEvent::Stream(StreamEvent::TextEnd { index: 0 }),
            AgentEvent::TurnEnd {
                stop_reason: StopReason::EndTurn,
            },
            AgentEvent::AgentEnd {
                stop_reason: rho_core::AgentStopReason::EndTurn,
            },
        ];
        let mut stream = futures::stream::iter(events.into_iter().map(Ok));
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let code = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(print_run(
                &mut stream,
                &mut out,
                &mut err,
                false,
                Some(&mut recording),
            ));
        recording.close();

        assert_eq!(code, 0);
        assert!(
            String::from_utf8_lossy(&out).contains("the bug is on line 42"),
            "the answer still reaches stdout"
        );

        // And the same run is on disk, in order.
        let read = rho_core::SessionReader::read(&path).expect("the file reads back");
        let roles: Vec<Role> = read
            .entries
            .iter()
            .filter_map(|entry| match &entry.record {
                Record::Message { message } => Some(message.role),
                _ => None,
            })
            .collect();
        assert_eq!(
            roles,
            vec![Role::User, Role::Assistant],
            "the printer must fold the turn into a record, not only print it"
        );
        assert!(
            read.entries
                .iter()
                .any(|entry| matches!(entry.record, Record::Stop { .. })),
            "the agent end must reach the file"
        );
        assert!(
            matches!(read.entries.last().map(|e| &e.record), Some(Record::Closed)),
            "the run states its own close"
        );
    }
}

#[cfg(test)]
mod shutdown_tests {
    //! The MCP connect drain at shutdown, A1's CLI half.
    //!
    //! The write happens on a background task inside the pool, so a behavioural test of the
    //! mechanism lives in `extensions::tests` (a fake handshake, a real drain, a real file).
    //! What that test cannot see is whether the two run paths actually call the drain before
    //! the process exits. That is the exact defect A1 names: a fast `rho run` exited and
    //! killed the write. So this reads the production source with comments stripped and pins
    //! both call sites. A grep that accepts a call surviving only in a comment is the trap
    //! two earlier reviews found on this branch, so the comments go first.

    fn production_source() -> String {
        let whole = include_str!("cli.rs");
        whole
            .split("#[cfg(test)]")
            .next()
            .expect("a source file has a first part")
            .lines()
            .map(|line| match line.split_once("//") {
                Some((code, _)) => code,
                None => line,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn both_run_paths_drain_the_mcp_connects_before_exit() {
        // `run_headless` and `run_interactive` must each drain, or a fast run loses the cache
        // write. The call is `drain_mcp(&extras)`; the definition reads `drain_mcp(extras:`,
        // so counting the borrowed call form finds the two call sites and not the definition.
        let source = production_source();
        let call_sites = source.matches("drain_mcp(&extras)").count();
        assert_eq!(
            call_sites, 2,
            "both run_headless and run_interactive must call drain_mcp before exit; found {call_sites}"
        );
    }

    #[test]
    fn the_drain_awaits_connects_and_surfaces_cache_notices() {
        // The drain must both await the background write and surface a write failure, or A1
        // and A6 are half done. This pins the body of `drain_mcp` against a quiet deletion of
        // either call.
        let source = production_source();
        assert!(
            source.contains("pool.drain_connects(MCP_DRAIN_TIMEOUT)"),
            "drain_mcp must await the background connects"
        );
        assert!(
            source.contains("pool.take_cache_notices()"),
            "drain_mcp must surface a cache write failure the handshake could not report"
        );
    }

    #[test]
    fn build_session_passes_the_agent_discovery_switch_from_the_config() {
        // D4. `subagents::load` receives `discover`, and a critic hard-coded it to `true`
        // and watched 108 tests pass, so `--no-agents` could die in silence. The behavioural
        // half is `subagents::tests::no_agent_discovery_registers_no_spawn_tool`, which
        // cannot see this call site because it drives `load` directly. So this pins that
        // `build_session` forwards `loaded.discover_agents`, not a literal. Comments are
        // stripped first, because a call surviving only in a comment is the trap two earlier
        // reviews found on this branch.
        //
        // The spelling changed when `#5` moved the discovery fields into `AgentConfig`. The
        // invariant did not: the value comes from `loaded.discover_agents` and never from a
        // literal. Only the expected text moved with the merge.
        let source = production_source();
        assert!(
            source.contains("agents.discover = loaded.discover_agents"),
            "build_session must pass the config's discover_agents to subagents::load, not a literal"
        );
    }
}

// The mouse rules, asserted through the merge rather than through a helper.
//
// The other branch read the flag and the variable in `resolve_mouse`. This branch loads a
// config once and passes it, so a file reaches the interface as well. The rules are the same,
// and they are checked where they now live: `flag_layer` writes layer 6, and `Config` merges it
// over the variable, the file, and the default. See `D-the-wheel-needs-capture` and
// `SPEC-config-call-site`.
#[cfg(all(test, feature = "tui"))]
mod mouse_tests {
    use super::*;
    use clap::Parser;

    fn merged(args: &[&str], env: &[(&str, &str)]) -> bool {
        let cli = Cli::try_parse_from(args).expect("the arguments parse");
        tests::try_load(&cli, env, &[])
            .expect("the config loads")
            .tui_mouse
    }

    #[test]
    fn the_mouse_is_on_by_default() {
        // The alternate screen has no scrollback, so with capture off the wheel does nothing.
        assert!(merged(&["rho"], &[]));
    }

    #[test]
    fn the_no_mouse_flag_gives_the_mouse_back() {
        assert!(!merged(&["rho", "--no-mouse"], &[]));
    }

    #[test]
    fn the_no_mouse_flag_wins_over_the_env_var() {
        assert!(!merged(
            &["rho", "--no-mouse"],
            &[("RHO_TUI_MOUSE", "true")]
        ));
    }

    #[test]
    fn the_env_var_can_turn_the_mouse_off() {
        assert!(!merged(&["rho"], &[("RHO_TUI_MOUSE", "false")]));
    }

    #[test]
    fn an_unset_flag_writes_nothing_so_a_file_still_decides() {
        // Rule 6 of `SPEC-config-call-site`: a flag the user did not pass must not send a value.
        let cli = Cli::try_parse_from(["rho"]).expect("the arguments parse");
        assert_eq!(flag_layer(&cli).tui_mouse, None);
        let with_flag = Cli::try_parse_from(["rho", "--mouse"]).expect("the arguments parse");
        assert_eq!(flag_layer(&with_flag).tui_mouse, Some(true));
        let with_off = Cli::try_parse_from(["rho", "--no-mouse"]).expect("the arguments parse");
        assert_eq!(flag_layer(&with_off).tui_mouse, Some(false));
    }
}
