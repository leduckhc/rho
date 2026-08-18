//! The `rho` binary entry point.
//!
//! It parses the arguments, sets up logging, and runs the chosen mode. Logging
//! goes through `tracing-subscriber`, controlled by --log or the RHO_LOG
//! variable. A secret never reaches a log, because the `Secret` type masks
//! itself by construction. See `SPEC-core-runtime` section 12a.

mod cli;
mod extensions;
mod provider;
mod subagents;

use clap::Parser;
use cli::Cli;
use tracing_subscriber::EnvFilter;

fn main() {
    let parsed = Cli::parse();
    init_logging(parsed.log.as_deref());

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("rho: cannot start the async runtime: {error}");
            std::process::exit(1);
        }
    };
    let code = runtime.block_on(cli::run(parsed));
    std::process::exit(code);
}

/// Set up logging. The filter comes from the flag, then the RHO_LOG variable,
/// then a quiet default. Diagnostics go to stderr, so stdout stays clean for a
/// piped answer.
fn init_logging(filter: Option<&str>) {
    let env_filter = match filter {
        Some(value) => EnvFilter::new(value),
        None => EnvFilter::try_from_env("RHO_LOG").unwrap_or_else(|_| EnvFilter::new("warn")),
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(std::io::stderr)
        .try_init();
}
