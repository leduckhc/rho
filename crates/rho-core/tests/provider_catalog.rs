//! Tests for the `Provider::catalog()` contract.
//!
//! These tests do not reach any provider crate. They pin the shape of the
//! `ModelCatalog` trait and the `ModelDescriptor` data model.

use rho_core::{MAX_MODELS, ModelCatalog, ModelDescriptor, Provider};

struct NoCatalogProvider;

#[async_trait::async_trait]
impl Provider for NoCatalogProvider {
    fn id(&self) -> &str {
        "no-catalog"
    }

    fn catalog(&self) -> Option<&dyn ModelCatalog> {
        None
    }

    async fn stream(
        &self,
        _request: rho_core::CompletionRequest,
        _cancel: rho_core::CancelToken,
    ) -> Result<rho_core::ProviderStream, rho_core::ProviderError> {
        unreachable!("test does not stream")
    }
}

#[test]
fn catalog_none_is_not_empty_list() {
    let provider = NoCatalogProvider;
    // A provider that cannot list returns `None`. That is distinct from a provider
    // that lists and returns an empty `Ok(vec)`, which the OpenRouter tests cover.
    assert!(provider.catalog().is_none());
}

#[test]
fn descriptor_has_only_id_and_label() {
    // Construct with the two public fields. If a third field is ever added,
    // this compile-time construction breaks, which is the point of the test.
    let descriptor = ModelDescriptor {
        id: "x".to_string(),
        display_name: Some("X".to_string()),
    };
    assert_eq!(descriptor.id, "x");
    assert_eq!(descriptor.display_name.as_deref(), Some("X"));
}

// The cap is a positive constant small enough to refuse a hostile list.
const _: () = assert!(MAX_MODELS > 0 && MAX_MODELS <= 10_000);
