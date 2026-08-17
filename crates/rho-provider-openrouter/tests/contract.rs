//! The shared provider contract, run against OpenRouter.

mod common;

use common::OpenRouterHarness;
use rho_provider_testkit::run_all;

#[tokio::test]
async fn provider_contract_emits_message_start_first() {
    rho_provider_testkit::provider_contract_emits_message_start_first(&OpenRouterHarness).await;
}

#[tokio::test]
async fn provider_contract_emits_done_last() {
    rho_provider_testkit::provider_contract_emits_done_last(&OpenRouterHarness).await;
}

#[tokio::test]
async fn provider_contract_yields_first_event_before_stream_end() {
    rho_provider_testkit::provider_contract_yields_first_event_before_stream_end(
        &OpenRouterHarness,
    )
    .await;
}

#[tokio::test]
async fn provider_contract_text_deltas_in_order() {
    rho_provider_testkit::provider_contract_text_deltas_in_order(&OpenRouterHarness).await;
}

#[tokio::test]
async fn provider_contract_tool_call_end_has_parsed_arguments() {
    rho_provider_testkit::provider_contract_tool_call_end_has_parsed_arguments(&OpenRouterHarness)
        .await;
}

#[tokio::test]
async fn provider_contract_run_all() {
    run_all(&OpenRouterHarness).await;
}
