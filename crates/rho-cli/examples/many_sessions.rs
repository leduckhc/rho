//! Measure the incremental cost of one more session.
//!
//! This answers the question the project was started for: can one machine hold 50
//! concurrent sessions? A total for one process hides the answer, because a large
//! part of that total is the binary, the allocator, and the TLS stack, and those
//! are paid once. What matters is the slope, not the intercept.
//!
//! So this example builds `RHO_SESSIONS` sessions in one process and idles. Run it
//! twice, with 1 and with 51, and subtract. The difference divided by 50 is the
//! honest incremental cost of a session.
//!
//! ```sh
//! cargo build --release -p rho-cli --example many_sessions
//! RHO_SESSIONS=1  /usr/bin/time -l ./target/release/examples/many_sessions
//! RHO_SESSIONS=51 /usr/bin/time -l ./target/release/examples/many_sessions
//! ```
//!
//! Every session gets its own provider client, tool registry, hook chain, and
//! context, because a real host does not share those between users.

use std::sync::Arc;

use rho_core::{AllowAllPolicy, Context, HookChain, Session, SessionConfig};
use rho_provider_openrouter::{OpenRouterConfig, OpenRouterProvider, Secret};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let count: usize = std::env::var("RHO_SESSIONS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);

    let root = std::env::current_dir().expect("a working directory");
    let mut sessions = Vec::with_capacity(count);
    for index in 0..count {
        let provider = Arc::new(OpenRouterProvider::new(OpenRouterConfig::new(Secret::new(
            "sk-not-a-real-key",
        ))));
        let config = SessionConfig::new(
            "anthropic/claude-sonnet-4",
            root.clone(),
            Arc::new(AllowAllPolicy),
        );
        sessions.push(Session::with_config(
            config,
            provider,
            Arc::new(rho_tools::builtin_registry()),
            Arc::new(HookChain::new()),
            Context::new(Some(format!("You are rho. Session {index}.")), Vec::new()),
        ));
    }

    println!("{count} idle sessions built");
    std::hint::black_box(&sessions);
}
