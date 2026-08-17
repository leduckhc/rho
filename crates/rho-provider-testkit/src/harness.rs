//! The harness an outside author implements.
//!
//! The harness is the only provider-specific part of a conformance run. It
//! turns an abstract [`Script`] into a live event stream, with no network. An
//! author wires their own fake transport inside `run`. For an HTTP provider,
//! use `wiremock` for a normal script and [`crate::StagedHttpServer`] for the
//! gated script. For an SDK provider, replay recorded events.

use crate::script::Script;
use async_trait::async_trait;
use rho_core::ProviderStream;
use std::any::Any;

/// One scripted run: the event stream, plus a guard that keeps the fake
/// transport alive while the stream is read.
pub struct HarnessRun {
    /// The provider event stream, produced with no network.
    pub stream: ProviderStream,
    /// A guard for the fake transport, for example a started mock server.
    /// The contract check holds it until the stream ends.
    pub guard: Box<dyn Any + Send>,
}

/// A provider-specific bridge from a [`Script`] to a live event stream.
///
/// An author implements this once for their provider. The contract functions
/// then run against any implementation.
#[async_trait]
pub trait ProviderHarness: Send + Sync {
    /// Build the event stream for `script`, with no network call.
    async fn run(&self, script: Script) -> HarnessRun;
}
