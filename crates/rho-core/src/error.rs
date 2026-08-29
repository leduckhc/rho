//! The error taxonomy.
//!
//! `ProviderError` is raised by a provider crate. `Error` is the top-level agent
//! error. Retryable transport errors are separate from permanent client errors.

use crate::ToolError;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// A network fault. Retryable.
    #[error("transport error: {0}")]
    Transport(String),
    /// HTTP 429. Retryable. `retry_after_ms` mirrors the `Retry-After` header.
    #[error("rate limited")]
    RateLimited { retry_after_ms: Option<u64> },
    /// HTTP 5xx. Retryable.
    #[error("server error: status {status}")]
    Server { status: u16 },
    /// HTTP 4xx other than 429. Permanent. Never retried.
    ///
    /// `advice` is **rho's own sentence, and never the peer's response body.** Its type is
    /// `&'static str`, so a body read from the network cannot be placed here: a response body
    /// is a `String` built at run time and it does not coerce to `&'static str`. The compiler
    /// holds the no-secret rule, rather than a comment asking an author to remember it.
    ///
    /// It carried the body, and rho prints this error, so a host that reflects the
    /// `Authorization` header put a resolved credential on a user's stderr. A loopback host is
    /// allowed by the base-url safety gate, so nothing warned. See
    /// `D-a-client-error-carries-no-peer-body`.
    #[error("client error: status {status}: {advice}")]
    Client { status: u16, advice: &'static str },
    /// The response body could not be decoded. Permanent.
    #[error("stream decode error: {0}")]
    Decode(String),
    /// Credential resolution or signing failed. Permanent.
    #[error("authentication failed: {0}")]
    Auth(String),
    /// The caller cancelled the request.
    #[error("canceled")]
    Canceled,
}

impl ProviderError {
    /// True for faults a retry may fix. False for permanent client faults.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ProviderError::Transport(_)
                | ProviderError::RateLimited { .. }
                | ProviderError::Server { .. }
        )
    }
}

/// A compile-time guard that `Client::advice` cannot hold a run-time string.
///
/// A `const` item can only be built from values that exist at compile time. A `&'static str`
/// does; a `String` does not. So widening the field back to `String`, which is what carried a
/// peer's response body and leaked a credential onto a user's stderr, fails to build **here**,
/// with a message pointing at this comment.
///
/// A mutation proved this is the load-bearing half: re-adding the body read alone does not
/// compile, and the leak needed the type widened first. See
/// `D-a-client-error-carries-no-peer-body`.
const _ADVICE_IS_NEVER_A_RUNTIME_STRING: ProviderError = ProviderError::Client {
    status: 400,
    advice: "a const item can hold no String, so a peer's body can never reach this field",
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Tool(#[from] ToolError),
    #[error("canceled")]
    Canceled,
}
