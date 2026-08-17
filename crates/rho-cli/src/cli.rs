//! The `rho` command line interface.
//!
//! Three entry points must work: `rho --help`, `rho run <prompt>` for a headless
//! answer on stdout, and `rho` for the interactive TUI. This module parses the
//! arguments, builds a `SessionConfig` explicitly, and runs the chosen mode.
//!
//! The session config is stated out loud here. Decision D-013 deleted a
//! convenience constructor because it hid a fake model id, an accidental session
//! root, and a policy that approved every tool call. So this module names the
//! model, the session root, and the approval policy in the calling code.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use futures::StreamExt;
use rho_core::{
    AgentEvent, AllowAllPolicy, ApprovalPolicy, CancelToken, ContentBlock, Context, ReadOnlyPolicy,
    Session, SessionConfig, StreamEvent,
};

use crate::provider::{self, MODEL_ENV, PROVIDER_ENV};

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

    /// The log filter, for example "info" or "rho_core=debug". Overrides RHO_LOG.
    #[arg(long, global = true, env = "RHO_LOG")]
    pub log: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
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
    let model = cli.model.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "no model was chosen. Set --model or the {MODEL_ENV} variable to a model id."
        )
    })?;

    // The session root confines every tool path. Choose the current directory by
    // default, and state that choice here. A --root flag overrides it.
    let root = match &cli.root {
        Some(path) => path.clone(),
        None => std::env::current_dir()
            .map_err(|error| anyhow::anyhow!("cannot read the current directory: {error}"))?,
    };

    // State the approval policy out loud. See decision D-013, which deleted a
    // constructor that hid this choice.
    //
    // The default approves every tool call, so a headless run never stops for a
    // prompt. `--read-only` swaps in a policy that denies every mutating tool. That
    // policy is fail-closed: it allows only a kind it names, so a tool with an
    // undeclared kind is denied. See decision D-012, and D-017 for plugin tools,
    // which always count as mutating.
    //
    // A future release adds an interactive approval gate for the TUI. Until then
    // `--read-only` is the way to run rho against a repository you do not trust.
    let approval: Arc<dyn ApprovalPolicy> = if cli.read_only {
        Arc::new(ReadOnlyPolicy)
    } else {
        Arc::new(AllowAllPolicy)
    };

    Ok(SessionConfig::new(model, root, approval))
}

/// Build a session from the config and the chosen provider.
fn build_session(cli: &Cli, config: SessionConfig) -> anyhow::Result<Session> {
    let name = provider::resolve_provider_name(cli.provider.as_deref(), None)?;
    let provider = provider::build_provider(&name)?;
    let tools = Arc::new(rho_tools::builtin_registry());
    let hooks = Arc::new(rho_core::HookChain::default());
    let context = Context::new(Some(system_prompt()), tools.specs());
    Ok(Session::with_config(
        config, provider, tools, hooks, context,
    ))
}

/// The short system prompt. A short prompt keeps the prefix small. See F-64.
fn system_prompt() -> String {
    "You are rho, a coding agent. You use the tools to read and change files. \
     You keep answers short."
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
    let session = match build_session(cli, config) {
        Ok(session) => session,
        Err(error) => return fail(error),
    };

    let cancel = CancelToken::new();
    let mut events = session.prompt(vec![ContentBlock::Text { text: prompt }], cancel);
    let mut stdout = std::io::stdout();
    let mut failed = false;

    while let Some(item) = events.next().await {
        match item {
            Ok(AgentEvent::Stream(StreamEvent::TextDelta { delta, .. })) => {
                let _ = write!(stdout, "{delta}");
                let _ = stdout.flush();
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

/// Run the interactive TUI. Return a non-zero code on failure.
#[cfg(feature = "tui")]
async fn run_interactive(cli: &Cli) -> i32 {
    let config = match build_config(cli) {
        Ok(config) => config,
        Err(error) => return fail(error),
    };
    let model = config.model.clone();
    let session = match build_session(cli, config) {
        Ok(session) => session,
        Err(error) => return fail(error),
    };

    let mut app = rho_tui::App::new(session, model);
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

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn build_config_fails_without_a_model() {
        // Parse real argv instead of building the struct by hand. A hand-built
        // literal breaks whenever a flag is added, and it skips the parser.
        let cli = Cli::try_parse_from(["rho"]).unwrap();
        let error = match build_config(&cli) {
            Ok(_) => panic!("expected an error"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains(MODEL_ENV),
            "message must name the model variable: {error}"
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
        // constructor. See decision D-013.
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
            // An undeclared kind counts as mutating, so it is denied too. See D-012.
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
}
