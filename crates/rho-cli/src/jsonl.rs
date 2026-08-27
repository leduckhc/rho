//! The `rho jsonl` frontend: it wires `rho-jsonl` onto the real session builder.
//!
//! `rho-jsonl` links no provider, so it cannot build a session. This module is the host
//! side of that seam. It implements `SessionFactory` over the same config merge, provider
//! set, tool registry, and skill loader that `rho run` uses. See decision
//! D-rho-jsonl-asks-a-factory-for-a-session.

use std::sync::Arc;

use async_trait::async_trait;
use rho_core::Session;
use rho_jsonl::{Asker, FactoryError, SessionFactory, SessionRequest};

use crate::cli::{Cli, build_config_with_notices, build_session, load_config};
use crate::provider;

/// The host side of the frontend seam.
///
/// It holds the parsed flags, so every session it builds sees the same configuration a
/// `rho run` would. It also holds what a session needs kept alive: the task registry and
/// the extras. Dropping either kills background tasks and stops MCP servers, so the
/// factory keeps the newest set for as long as the session lives.
struct CliFactory {
    cli: Cli,
    /// Keeps the current session's task registry and extras alive.
    ///
    /// `set_model` and `new_session` replace a session, and the old set drops then. That
    /// is correct: the old session is gone, and its background work goes with it.
    held: tokio::sync::Mutex<Option<Held>>,
}

/// What a live session needs kept alive beside it.
struct Held {
    #[allow(dead_code)]
    tasks: Arc<rho_core::TaskRegistry>,
    #[allow(dead_code)]
    extras: crate::cli::SessionExtras,
}

#[async_trait]
impl SessionFactory for CliFactory {
    async fn build(
        &self,
        request: &SessionRequest,
        asker: Arc<dyn Asker>,
    ) -> Result<Session, FactoryError> {
        // Refuse an unknown provider by name, before any credential work. A client then
        // learns which of the three failures happened.
        if provider::build_provider(&request.provider).is_err() {
            return Err(FactoryError::UnknownProvider {
                name: request.provider.clone(),
            });
        }

        // Take the merged configuration, then state this request's provider and model.
        // Every other layer still applies, so a config file and a profile reach a JSONL
        // session exactly as they reach `rho run`.
        let mut cli = self.cli.clone();
        cli.provider = Some(request.provider.clone());
        cli.model = Some(request.model_id.clone());
        let loaded =
            load_config(&cli).map_err(|error| FactoryError::Internal(error.to_string()))?;

        // `approval = "ask"` has no answer in a one-shot headless run, so
        // `build_config_with_notices` refuses it. Over this protocol it does have an
        // answer, because the client can be asked. So ask for a permissive config here
        // and swap the real policy in below.
        let wants_ask = matches!(loaded.approval, Some(rho_config::ApprovalMode::Ask));
        let mut for_build = loaded.clone();
        if wants_ask {
            for_build.approval = Some(rho_config::ApprovalMode::AllowAll);
        }

        let mut notices = Vec::new();
        let mut config = build_config_with_notices(&for_build, &mut notices).map_err(|error| {
            // A missing model for a provider with no default lands here, and it is a
            // bad argument rather than an internal fault.
            FactoryError::RefusedModel {
                provider: request.provider.clone(),
                model_id: request.model_id.clone(),
                reason: error.to_string(),
            }
        })?;

        if wants_ask {
            // The dialog gate is fail-closed: only an explicit yes allows a call, and a
            // silent client denies after the timeout. This is the first rho frontend
            // that can honour `approval = "ask"`.
            config.approval = Arc::new(rho_jsonl::DialogApproval::new(asker));
        }

        let (session, tasks, extras) = build_session(&cli, &loaded, config)
            .await
            .map_err(|error| classify_build(&request.provider, error))?;

        // Notices go to stderr. stdout carries the protocol, so a notice there would be
        // a line the client cannot parse.
        for notice in &extras.notices {
            eprintln!("rho: {notice}");
        }
        *self.held.lock().await = Some(Held { tasks, extras });
        Ok(session)
    }
}

/// Turn a session-build failure into the named wire case.
///
/// A missing credential is the common one. The provider's own message already names the
/// variable to set, so this lifts that name out of it rather than keeping a second table of
/// provider variables. It is a heuristic: it looks for one shouting token, such as
/// `OPENROUTER_API_KEY`. When it finds none, the structured field says `unnamed`.
///
/// The provider's message goes to stderr before the reply, because `FactoryError`'s own
/// display text replaces it and a dropped diagnostic is the hardest kind to debug. No
/// branch here ever reads a credential value.
fn classify_build(provider_name: &str, error: anyhow::Error) -> FactoryError {
    let text = error.to_string();
    let looks_like_a_credential =
        text.contains("credential") || text.contains("key") || text.contains("Set ");
    if looks_like_a_credential {
        eprintln!("rho: {text}");
        return FactoryError::MissingCredential {
            provider: provider_name.to_string(),
            variable: shouting_token(&text).unwrap_or_else(|| "unnamed".to_string()),
        };
    }
    FactoryError::Internal(text)
}

/// The first token that looks like an environment variable name.
fn shouting_token(text: &str) -> Option<String> {
    text.split(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
        .find(|token| token.len() >= 5 && token.contains('_'))
        .map(str::to_string)
}

/// Serve the JSONL protocol on stdin and stdout until stdin closes.
///
/// Returns the process exit code.
pub async fn run_jsonl(cli: &Cli) -> i32 {
    // Resolve the starting provider and model from the merge, so `rho jsonl` needs no
    // flags when a config file already says which model to use.
    let loaded = match load_config(cli) {
        Ok(loaded) => loaded,
        Err(error) => {
            eprintln!("rho: {error}");
            return 1;
        }
    };
    let provider_name = match provider::resolve_provider_name(loaded.provider.as_deref(), None) {
        Ok(name) => name,
        Err(error) => {
            eprintln!("rho: {error}");
            return 1;
        }
    };
    // Refuse an unresolved model. `unwrap_or_default` here sent an empty model id to the
    // provider, which then failed with a message about the request rather than about the
    // missing setting. `rho run` refuses this case, and so must this frontend. Azure has
    // no default model, so it is the common way to reach this.
    let model = match loaded
        .model
        .clone()
        .or_else(|| provider::default_model(&provider_name).map(str::to_string))
    {
        Some(model) => model,
        None => {
            eprintln!(
                "rho: no model was chosen, and {provider_name} has no default. \
                 Set --model or the RHO_MODEL variable. For azure, the value is your \
                 deployment name."
            );
            return 1;
        }
    };

    let factory: Arc<dyn SessionFactory> = Arc::new(CliFactory {
        cli: cli.clone(),
        held: tokio::sync::Mutex::new(None),
    });

    match rho_jsonl::serve(
        factory,
        SessionRequest::new(provider_name, model),
        tokio::io::stdin(),
        tokio::io::stdout(),
    )
    .await
    {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("rho: the jsonl frontend stopped: {error}");
            1
        }
    }
}
