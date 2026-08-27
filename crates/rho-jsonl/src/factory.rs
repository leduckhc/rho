//! The host seam: what a caller must provide so the frontend can build a session.
//!
//! This is the extension point of the crate. `rho-jsonl` builds no session itself,
//! because building one needs a provider, and a provider needs a credential and an
//! HTTP client. `rho-core` links neither. So the host builds the session and the
//! frontend asks for one. See decision D-rho-jsonl-asks-a-factory-for-a-session.

use std::sync::Arc;

use async_trait::async_trait;

use rho_core::Session;

use crate::dialog::Asker;

/// Which provider and model a session runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRequest {
    /// The provider name, for example `bedrock`.
    pub provider: String,
    /// The model id the provider expects.
    pub model_id: String,
}

impl SessionRequest {
    /// A request for one provider and model.
    pub fn new(provider: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model_id: model_id.into(),
        }
    }
}

/// Why a session could not be built.
///
/// Each case maps onto one named `ReplyError`, so a client learns which of them
/// happened and does not have to read the message. No variant carries a credential
/// value. `MissingCredential` names the variable, never the secret.
#[derive(Debug, thiserror::Error)]
pub enum FactoryError {
    /// This build has no provider with that name.
    #[error("no provider is called {name}")]
    UnknownProvider { name: String },
    /// The provider exists, and its credential is absent from the environment.
    #[error("{provider} has no credential: set {variable}")]
    MissingCredential { provider: String, variable: String },
    /// The provider refused the model id.
    #[error("{provider} refused the model {model_id}: {reason}")]
    RefusedModel {
        provider: String,
        model_id: String,
        reason: String,
    },
    /// Anything else. The session stays closed and the client is told.
    #[error("cannot build the session: {0}")]
    Internal(String),
}

/// What a host must provide so the frontend can build a session.
///
/// `rho-cli` implements this. So does any embedder that wants the protocol over its
/// own streams, with its own providers, tools, and approval policy. An implementor
/// edits no shared code and adds no enum variant.
///
/// It has exactly one method. An earlier draft also had `providers`, so `get_state`
/// could list the provider names a build has. It came out again, because `rho-cli` owns
/// the only list of provider names and this crate must not keep a second copy of it. A
/// client discovers a provider by trying `set_model` and reading the named error, which
/// is how it discovers a command too.
#[async_trait]
pub trait SessionFactory: Send + Sync {
    /// Build a session for this provider and model.
    ///
    /// The frontend calls it for the first session, and again for every `set_model`
    /// and every `new_session`. Each call must return a session with an empty
    /// context, because `new_session` means an empty conversation.
    ///
    /// `asker` reaches the client over the protocol. A host that wants a human in the
    /// approval loop wraps it in `DialogApproval` and puts that in the
    /// `SessionConfig`. A host that approves everything ignores it. Passing it here is
    /// what makes the dialog sub-protocol reachable, rather than a shape with no
    /// producer.
    async fn build(
        &self,
        request: &SessionRequest,
        asker: Arc<dyn Asker>,
    ) -> Result<Session, FactoryError>;
}
