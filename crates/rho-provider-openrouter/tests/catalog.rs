//! Tests for `ModelCatalog` on the OpenRouter provider.
//!
//! Every test uses a mock server. No test reaches the network.

use rho_core::{CancelToken, MAX_MODELS, ModelDescriptor, Provider};
use rho_provider_openrouter::{OpenRouterConfig, OpenRouterProvider, Secret};
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MODELS_PATH: &str = "/api/v1/models";

fn provider(server_uri: &str) -> OpenRouterProvider {
    let config = OpenRouterConfig::new(Secret::new("test-key")).with_base_url(server_uri);
    OpenRouterProvider::new(config)
}

fn models_response(data: Vec<ModelDescriptor>) -> serde_json::Value {
    let models: Vec<serde_json::Value> = data
        .into_iter()
        .map(|d| {
            let mut obj = serde_json::json!({"id": d.id});
            if let Some(name) = d.display_name {
                obj["name"] = serde_json::Value::String(name);
            }
            obj
        })
        .collect();
    serde_json::json!({ "data": models })
}

#[tokio::test]
async fn catalog_reports_some_for_openrouter() {
    let server = MockServer::start().await;
    let provider = provider(&server.uri());
    assert!(provider.catalog().is_some());
}

#[tokio::test]
async fn openrouter_lists_models_from_the_data_field() {
    let server = MockServer::start().await;
    let payload = models_response(vec![
        ModelDescriptor {
            id: "openai/gpt-4o".to_string(),
            display_name: Some("GPT-4o".to_string()),
        },
        ModelDescriptor {
            id: "anthropic/claude-sonnet-4-6".to_string(),
            display_name: None,
        },
    ]);
    Mock::given(method("GET"))
        .and(path(MODELS_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(payload))
        .mount(&server)
        .await;

    let provider = provider(&server.uri());
    let catalog = provider.catalog().expect("openrouter has a catalog");
    let models = catalog
        .list_models(CancelToken::new())
        .await
        .expect("list succeeded");
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "openai/gpt-4o");
    assert_eq!(models[0].display_name.as_deref(), Some("GPT-4o"));
    assert_eq!(models[1].id, "anthropic/claude-sonnet-4-6");
    assert!(models[1].display_name.is_none());
}

#[tokio::test]
async fn empty_list_is_ok_not_none() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(MODELS_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(models_response(vec![])))
        .mount(&server)
        .await;

    let provider = provider(&server.uri());
    let catalog = provider.catalog().expect("openrouter has a catalog");
    let models = catalog
        .list_models(CancelToken::new())
        .await
        .expect("empty list is still Ok");
    assert!(models.is_empty());
}

#[tokio::test]
async fn a_listing_over_the_cap_is_refused_and_names_both_numbers() {
    let server = MockServer::start().await;
    let too_many: Vec<ModelDescriptor> = (0..=MAX_MODELS)
        .map(|i| ModelDescriptor {
            id: format!("model-{i}"),
            display_name: None,
        })
        .collect();
    Mock::given(method("GET"))
        .and(path(MODELS_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(models_response(too_many)))
        .mount(&server)
        .await;

    let provider = provider(&server.uri());
    let catalog = provider.catalog().expect("openrouter has a catalog");
    let error = catalog
        .list_models(CancelToken::new())
        .await
        .expect_err("list over the cap must fail");
    let message = format!("{error}");
    assert!(
        message.contains(&format!("{}", MAX_MODELS + 1)),
        "error names the returned count: {message}"
    );
    assert!(
        message.contains(&format!("{}", MAX_MODELS)),
        "error names the cap: {message}"
    );
}

#[tokio::test]
async fn list_models_stops_on_cancel() {
    let server = MockServer::start().await;
    // Hold the response for longer than the test timeout so the cancel path is what returns.
    Mock::given(method("GET"))
        .and(path(MODELS_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(models_response(vec![]))
                .set_delay(Duration::from_secs(60)),
        )
        .mount(&server)
        .await;

    let provider = provider(&server.uri());
    let cancel = CancelToken::new();
    let catalog = provider
        .catalog()
        .expect("openrouter has a catalog")
        .list_models(cancel.clone());

    cancel.cancel();
    let result = catalog.await;
    assert!(
        matches!(result, Err(rho_core::ProviderError::Canceled)),
        "cancelled listing must return Canceled, got {result:?}"
    );
}
