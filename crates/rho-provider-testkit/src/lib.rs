//! `rho-provider-testkit` proves that a `Provider` conforms to the rho contract.
//!
//! A third party writes a provider without forking rho. This crate lets that
//! author run the same conformance checks that rho itself uses. The checks are
//! public functions. Each one drives a `Provider` through a scripted response
//! and asserts one rule from `SPEC-02` section 7.
//!
//! An outside author implements [`ProviderHarness`] for their provider. The
//! harness turns an abstract [`Script`] into a live event stream, with no
//! network. The author then calls [`run_all`] to run every contract check, or
//! calls a single `provider_contract_*` function.
//!
//! The crate stays provider-agnostic. It never depends on a concrete provider.

mod contract;
mod harness;
mod script;
mod staged_server;

pub use contract::{
    provider_contract_emits_done_last, provider_contract_emits_message_start_first,
    provider_contract_text_deltas_in_order, provider_contract_tool_call_end_has_parsed_arguments,
    provider_contract_yields_first_event_before_stream_end, run_all,
};
pub use harness::{HarnessRun, ProviderHarness};
pub use script::{SCRIPT_TEXT, SCRIPT_TOOL_NAME, Script, script_tool_arguments};
pub use staged_server::StagedHttpServer;
