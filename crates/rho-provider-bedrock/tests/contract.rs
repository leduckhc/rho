//! The shared provider contract, run against Bedrock.
//!
//! The harness replays a recorded `ConverseStream` fixture through the pure
//! mapping. No AWS client runs. No network runs.

mod common;

use common::BedrockHarness;
use rho_provider_testkit::run_all;

#[tokio::test]
async fn provider_contract_emits_message_start_first() {
    rho_provider_testkit::provider_contract_emits_message_start_first(&BedrockHarness).await;
}

#[tokio::test]
async fn provider_contract_emits_done_last() {
    rho_provider_testkit::provider_contract_emits_done_last(&BedrockHarness).await;
}

#[tokio::test]
async fn provider_contract_yields_first_event_before_stream_end() {
    rho_provider_testkit::provider_contract_yields_first_event_before_stream_end(&BedrockHarness)
        .await;
}

#[tokio::test]
async fn provider_contract_text_deltas_in_order() {
    rho_provider_testkit::provider_contract_text_deltas_in_order(&BedrockHarness).await;
}

#[tokio::test]
async fn provider_contract_tool_call_end_has_parsed_arguments() {
    rho_provider_testkit::provider_contract_tool_call_end_has_parsed_arguments(&BedrockHarness)
        .await;
}

#[tokio::test]
async fn provider_contract_run_all() {
    run_all(&BedrockHarness).await;
}
