//! Shared test support for the Bedrock provider.
//!
//! It loads a recorded `ConverseStream` fixture into the serde mirror. It then
//! feeds the events to `events_to_stream`. No AWS client runs. No network runs.
//!
//! Each integration-test binary uses only part of this module. The allow below
//! silences the false dead-code warning that pattern creates.
#![allow(dead_code)]

use async_trait::async_trait;
use rho_core::{CancelToken, ProviderStream};
use rho_provider_bedrock::{ConverseStreamEvent, events_to_stream};
use rho_provider_testkit::{HarnessRun, ProviderHarness, Script};

/// Load a fixture file into the recorded-event mirror.
///
/// The path resolves against the crate root, so the test works from any
/// working directory.
pub fn load_fixture(name: &str) -> Vec<ConverseStreamEvent> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {path}: {error}"));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("parse {path}: {error}"))
}

/// Build a provider stream from a fixture, with no network.
pub fn stream_fixture(name: &str) -> ProviderStream {
    events_to_stream(load_fixture(name), CancelToken::new())
}

/// The testkit harness for Bedrock. It replays recorded events.
pub struct BedrockHarness;

#[async_trait]
impl ProviderHarness for BedrockHarness {
    async fn run(&self, script: Script) -> HarnessRun {
        // The text fixture answers "Hello" and reports usage. The tool fixture
        // splits the tool input across several deltas.
        let name = match script {
            Script::Text | Script::Usage | Script::Gated => "converse_text.json",
            Script::ToolCall => "converse_tool_call.json",
        };
        HarnessRun {
            stream: stream_fixture(name),
            guard: Box::new(()),
        }
    }
}
