//! Tests for `ModelCatalog` on the Bedrock provider.
//!
//! Live listing against AWS is verified separately. These tests cover the
//! contract shape without credentials.

use rho_core::Provider;
use rho_provider_bedrock::{BedrockConfig, BedrockProvider};

#[test]
fn catalog_reports_some_for_bedrock() {
    let provider = BedrockProvider::new(BedrockConfig::new("us-east-1"));
    assert!(provider.catalog().is_some());
}
