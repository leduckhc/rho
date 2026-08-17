//! Measure the resident memory of one idle session.
//!
//! This is the number the whole project exists to improve. A harness that costs
//! more than 100 MB per session cannot run 50 sessions on one machine.
//!
//! The example builds everything a real session holds: the provider client with
//! its TLS stack, the built-in tool registry, the hook chain, and the agent
//! session itself. Then it idles. It opens no socket, so the number describes the
//! steady state rather than a request in flight.
//!
//! A process cannot read its own resident set portably, so run it under a reporter:
//!
//! ```sh
//! cargo build --release -p rho-cli --example idle_session
//! /usr/bin/time -l ./target/release/examples/idle_session
//! ```
//!
//! On macOS read `maximum resident set size`, which is in bytes. On Linux read
//! `Maximum resident set size`, which is in kilobytes.

use std::sync::Arc;

use rho_core::{AllowAllPolicy, Context, HookChain, Session, SessionConfig};
use rho_provider_openrouter::{OpenRouterConfig, OpenRouterProvider, Secret};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    // The key is never sent anywhere. The client still builds its TLS stack, and
    // that is the part which costs memory.
    let provider = Arc::new(OpenRouterProvider::new(OpenRouterConfig::new(Secret::new(
        "sk-not-a-real-key",
    ))));
    let tools = Arc::new(rho_tools::builtin_registry());
    let hooks = Arc::new(HookChain::new());
    let config = SessionConfig::new(
        "anthropic/claude-sonnet-4",
        std::env::current_dir().expect("a working directory"),
        Arc::new(AllowAllPolicy),
    );
    let session = Session::with_config(
        config,
        provider,
        tools,
        hooks,
        Context::new(Some("You are rho.".to_string()), Vec::new()),
    );

    // Hold the session, so nothing is dropped before the reporter samples the peak.
    println!(
        "idle session built, {} messages",
        session.messages().await.len()
    );
    std::hint::black_box(&session);
}
