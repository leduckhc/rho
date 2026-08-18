//! Provider selection for the `rho` binary.
//!
//! A provider is a cargo feature. A build without a provider must fail with a
//! clear message, not a panic. A missing API key must name the environment
//! variable to set. Every function here states its choices, so no credential or
//! session boundary is set by accident. See `SPEC-core-runtime` decisions D-session-config and D-no-four-argument-session-new.

use std::sync::Arc;

use rho_core::Provider;

/// The environment variable that holds the model id.
pub const MODEL_ENV: &str = "RHO_MODEL";
/// The environment variable that names the provider.
pub const PROVIDER_ENV: &str = "RHO_PROVIDER";

/// The OpenRouter API key variable.
#[cfg(feature = "openrouter")]
pub const OPENROUTER_KEY_ENV: &str = "OPENROUTER_API_KEY";
/// The Azure OpenAI API key variable.
#[cfg(feature = "azure")]
pub const AZURE_KEY_ENV: &str = "AZURE_OPENAI_API_KEY";
/// The Azure OpenAI endpoint variable.
#[cfg(feature = "azure")]
pub const AZURE_ENDPOINT_ENV: &str = "AZURE_OPENAI_ENDPOINT";
/// The Azure OpenAI deployment variable.
#[cfg(feature = "azure")]
pub const AZURE_DEPLOYMENT_ENV: &str = "AZURE_OPENAI_DEPLOYMENT";
/// The AWS region variable that Bedrock reads.
#[cfg(feature = "bedrock")]
pub const AWS_REGION_ENV: &str = "AWS_REGION";

/// A provider build fault. Every message tells the user what to do.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// The provider name is not known.
    #[error(
        "the provider \"{name}\" is not known. Choose one of: openrouter, bedrock, azure. Set it with --provider or the {PROVIDER_ENV} variable."
    )]
    Unknown { name: String },
    /// The provider name is valid, but this build does not include it.
    #[error(
        "the provider \"{name}\" is not in this build. Rebuild rho with the feature: cargo build --features {name}."
    )]
    NotCompiled { name: String },
    /// A required environment variable is missing or empty.
    #[error("{message}")]
    MissingConfig { message: String },
    /// No provider name was given and none could be chosen.
    #[error(
        "no provider was chosen. Set --provider or the {PROVIDER_ENV} variable to one of: openrouter, bedrock, azure."
    )]
    NoneChosen,
}

/// Build the message for a missing environment variable.
fn missing(var: &str, purpose: &str) -> ProviderError {
    ProviderError::MissingConfig {
        message: format!("set the {var} environment variable to {purpose}."),
    }
}

/// The set of known provider names, whether compiled in or not.
const KNOWN_PROVIDERS: [&str; 3] = ["openrouter", "bedrock", "azure"];

/// The default provider name for this build, or `None` when no provider feature
/// is present. OpenRouter wins when it is compiled in.
pub fn default_provider() -> Option<&'static str> {
    if cfg!(feature = "openrouter") {
        Some("openrouter")
    } else if cfg!(feature = "bedrock") {
        Some("bedrock")
    } else if cfg!(feature = "azure") {
        Some("azure")
    } else {
        None
    }
}

/// Resolve the provider name from the flag, then the environment, then the build
/// default.
/// The default model for a provider, when the caller names no model.
///
/// **Each default is verified through rho, not taken from a table.** The Bedrock entry was
/// chosen by sweeping the 54 models that a separate harness reported as working, and running
/// each candidate through `rho run` for a plain answer, a single tool call, and two tool
/// calls in one turn. `amazon.nova-micro-v1:0` passed a tool call four times out of four and
/// is the smallest that did. See `docs/verification/models.md`.
///
/// **A default model is a convenience, not a security choice.** Decision D-no-four-argument-session-new removed
/// hidden defaults for the session root and the approval policy, because a wrong value there
/// is a breach. A wrong model id is a bad answer and a small bill, so a default is safe here.
/// It is still reported, so the choice is never silent.
///
/// A caller overrides it with `--model`, or with the `RHO_MODEL` variable.
pub fn default_model(provider: &str) -> Option<&'static str> {
    match provider {
        // Smallest Bedrock model that reliably calls tools.
        "bedrock" => Some("amazon.nova-micro-v1:0"),
        // A small, cheap, widely available model on OpenRouter.
        "openrouter" => Some("anthropic/claude-haiku-4.5"),
        // Azure names a deployment, not a model, and only the account owner knows the
        // deployment names. So there is no honest default here.
        "azure" => None,
        _ => None,
    }
}

pub fn resolve_provider_name(
    flag: Option<&str>,
    env: Option<&str>,
) -> Result<String, ProviderError> {
    if let Some(name) = flag.or(env) {
        return Ok(name.to_string());
    }
    default_provider()
        .map(str::to_string)
        .ok_or(ProviderError::NoneChosen)
}

/// Build a provider by name. This reads the environment for credentials. It
/// fails with a clear message when a credential is missing.
pub fn build_provider(name: &str) -> Result<Arc<dyn Provider>, ProviderError> {
    match name {
        "openrouter" => build_openrouter(),
        "bedrock" => build_bedrock(),
        "azure" => build_azure(),
        other if KNOWN_PROVIDERS.contains(&other) => Err(ProviderError::NotCompiled {
            name: other.to_string(),
        }),
        other => Err(ProviderError::Unknown {
            name: other.to_string(),
        }),
    }
}

#[cfg(feature = "openrouter")]
fn build_openrouter() -> Result<Arc<dyn Provider>, ProviderError> {
    let key = std::env::var(OPENROUTER_KEY_ENV).unwrap_or_default();
    openrouter_from_key(&key)
}

/// Build the OpenRouter provider from a key value. A separate function, so a test
/// can check the missing-key path without changing the process environment.
#[cfg(feature = "openrouter")]
fn openrouter_from_key(key: &str) -> Result<Arc<dyn Provider>, ProviderError> {
    use rho_core::Secret;
    use rho_provider_openrouter::{OpenRouterConfig, OpenRouterProvider};

    let secret = Secret::new(key);
    if secret.is_empty() {
        return Err(missing(OPENROUTER_KEY_ENV, "your OpenRouter API key"));
    }
    let config = OpenRouterConfig::new(secret);
    Ok(Arc::new(OpenRouterProvider::new(config)))
}

#[cfg(not(feature = "openrouter"))]
fn build_openrouter() -> Result<Arc<dyn Provider>, ProviderError> {
    Err(ProviderError::NotCompiled {
        name: "openrouter".to_string(),
    })
}

#[cfg(feature = "bedrock")]
fn build_bedrock() -> Result<Arc<dyn Provider>, ProviderError> {
    use rho_provider_bedrock::{BedrockConfig, BedrockProvider};

    let region = std::env::var(AWS_REGION_ENV).unwrap_or_default();
    if region.is_empty() {
        return Err(missing(
            AWS_REGION_ENV,
            "your AWS region, for example us-east-1",
        ));
    }
    let config = BedrockConfig::new(region);
    Ok(Arc::new(BedrockProvider::new(config)))
}

#[cfg(not(feature = "bedrock"))]
fn build_bedrock() -> Result<Arc<dyn Provider>, ProviderError> {
    Err(ProviderError::NotCompiled {
        name: "bedrock".to_string(),
    })
}

#[cfg(feature = "azure")]
fn build_azure() -> Result<Arc<dyn Provider>, ProviderError> {
    use rho_core::Secret;
    use rho_provider_azure::{AzureAuth, AzureConfig, AzureProvider};

    let endpoint = std::env::var(AZURE_ENDPOINT_ENV).unwrap_or_default();
    if endpoint.is_empty() {
        return Err(missing(
            AZURE_ENDPOINT_ENV,
            "your Azure OpenAI endpoint, for example https://my-resource.openai.azure.com",
        ));
    }
    let deployment = std::env::var(AZURE_DEPLOYMENT_ENV).unwrap_or_default();
    if deployment.is_empty() {
        return Err(missing(
            AZURE_DEPLOYMENT_ENV,
            "your Azure OpenAI deployment name",
        ));
    }
    let key = std::env::var(AZURE_KEY_ENV).unwrap_or_default();
    let secret = Secret::new(key);
    if secret.is_empty() {
        return Err(missing(AZURE_KEY_ENV, "your Azure OpenAI API key"));
    }
    let config = AzureConfig::new(endpoint, deployment, AzureAuth::ApiKey(secret));
    Ok(Arc::new(AzureProvider::new(config)))
}

#[cfg(not(feature = "azure"))]
fn build_azure() -> Result<Arc<dyn Provider>, ProviderError> {
    Err(ProviderError::NotCompiled {
        name: "azure".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expect_err(result: Result<Arc<dyn Provider>, ProviderError>) -> ProviderError {
        match result {
            Ok(_) => panic!("expected an error"),
            Err(error) => error,
        }
    }

    #[test]
    fn unknown_provider_names_the_choices() {
        let message = expect_err(build_provider("nope")).to_string();
        assert!(message.contains("openrouter"), "message was: {message}");
        assert!(message.contains("--provider"), "message was: {message}");
    }

    #[test]
    fn resolve_prefers_the_flag_over_the_env() {
        let name = resolve_provider_name(Some("azure"), Some("bedrock")).unwrap();
        assert_eq!(name, "azure");
    }

    #[test]
    fn resolve_falls_back_to_the_env() {
        let name = resolve_provider_name(None, Some("bedrock")).unwrap();
        assert_eq!(name, "bedrock");
    }

    #[test]
    fn resolve_uses_the_build_default_when_nothing_is_set() {
        // The default build includes openrouter.
        let name = resolve_provider_name(None, None).unwrap();
        assert_eq!(name, "openrouter");
    }

    #[cfg(feature = "openrouter")]
    #[test]
    fn missing_openrouter_key_names_the_variable() {
        let message = expect_err(openrouter_from_key("")).to_string();
        assert!(
            message.contains(OPENROUTER_KEY_ENV),
            "message must name the variable: {message}"
        );
    }

    #[cfg(feature = "openrouter")]
    #[test]
    fn a_present_openrouter_key_builds_a_provider() {
        assert!(openrouter_from_key("sk-test-key").is_ok());
    }
}
