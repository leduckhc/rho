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
    #[error("client error: status {status}: {message}")]
    Client { status: u16, message: String },
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

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Tool(#[from] ToolError),
    #[error("canceled")]
    Canceled,
}
