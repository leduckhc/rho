//! The shared provider contract, run against Azure.

mod common;

use common::AzureHarness;
use rho_provider_testkit::run_all;

#[tokio::test]
async fn provider_contract_emits_message_start_first() {
    rho_provider_testkit::provider_contract_emits_message_start_first(&AzureHarness).await;
}

#[tokio::test]
async fn provider_contract_emits_done_last() {
    rho_provider_testkit::provider_contract_emits_done_last(&AzureHarness).await;
}

#[tokio::test]
async fn provider_contract_yields_first_event_before_stream_end() {
    rho_provider_testkit::provider_contract_yields_first_event_before_stream_end(&AzureHarness)
        .await;
}

#[tokio::test]
async fn provider_contract_text_deltas_in_order() {
    rho_provider_testkit::provider_contract_text_deltas_in_order(&AzureHarness).await;
}

#[tokio::test]
async fn provider_contract_tool_call_end_has_parsed_arguments() {
    rho_provider_testkit::provider_contract_tool_call_end_has_parsed_arguments(&AzureHarness).await;
}

#[tokio::test]
async fn provider_contract_run_all() {
    run_all(&AzureHarness).await;
}
