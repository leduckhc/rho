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
    /// A base url was set for a provider that names its endpoint its own way. This is a
    /// conflict between two stated choices, not a missing value, so it is its own variant.
    /// `MissingConfig` once carried it, and a caller could not tell "you set two things that
    /// disagree" from "you set nothing". See D11.
    #[error(
        "base-url is set to {url}, and the {name} provider names its endpoint its own way. Unset base-url, or use --provider openrouter."
    )]
    IncompatibleBaseUrl { name: String, url: String },
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
/// **Each default is verified through rho, not taken from a table.**
///
/// The Bedrock entry is the latest Haiku, on the **global** inference profile. Three ids were
/// run through `rho run` against live Bedrock, and the result decided the choice:
///
/// | id | result |
/// | --- | --- |
/// | `anthropic.claude-haiku-4-5-20251001-v1:0` | 400. A Claude 4.5 model has no on-demand throughput |
/// | `us.anthropic.claude-haiku-4-5-20251001-v1:0` | works in `us-east-1`, **400 in `eu-west-1`** |
/// | `global.anthropic.claude-haiku-4-5-20251001-v1:0` | works in both |
///
/// So a Claude 4.5 model needs an inference profile, and the profile has to be the global one. A
/// `us.` default would fail for every caller outside a US region.
///
/// The previous default was `amazon.nova-micro-v1:0`, chosen when it was the smallest model that
/// called a tool four times out of four. It is still alive, and it was not replaced because it
/// broke: a coding agent wants a model that handles tools and long context well, and Haiku 4.5
/// is that. See `docs/verification/models.md` and `D-bedrock-default-is-the-global-haiku`.
///
/// **A default model is a convenience, not a security choice.** Decision D-no-four-argument-session-new removed
/// hidden defaults for the session root and the approval policy, because a wrong value there
/// is a breach. A wrong model id is a bad answer and a small bill, so a default is safe here.
/// It is still reported, so the choice is never silent.
///
/// A caller overrides it with `--model`, or with the `RHO_MODEL` variable.
pub fn default_model(provider: &str) -> Option<&'static str> {
    match provider {
        // The latest Haiku, on the global inference profile so it works in any region.
        "bedrock" => Some("global.anthropic.claude-haiku-4-5-20251001-v1:0"),
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
pub fn build_provider(
    name: &str,
    base_url: Option<&str>,
) -> Result<Arc<dyn Provider>, ProviderError> {
    // A base url belongs to the OpenAI-compatible client. Bedrock and Azure name their
    // endpoint their own way, so a base url with either is a mistake rho reports rather
    // than ignores. Silence here would be a fresh dead switch.
    //
    // Each provider builder answers for itself whether it can use a base url, so the choice
    // is not welded to the literal `"openrouter"` in this shared match. A new
    // OpenAI-compatible provider adds its own builder and either uses the base url or calls
    // `refuse_base_url`; it never edits this match. See D10, and the trait-method design
    // noted in `refuse_base_url`.
    match name {
        "openrouter" => build_openrouter(base_url),
        "bedrock" => build_bedrock(base_url),
        "azure" => build_azure(base_url),
        other if KNOWN_PROVIDERS.contains(&other) => Err(ProviderError::NotCompiled {
            name: other.to_string(),
        }),
        other => Err(ProviderError::Unknown {
            name: other.to_string(),
        }),
    }
}

/// Refuse a base url for a provider that names its endpoint its own way.
///
/// A provider builder calls this when it cannot use a base url. A provider that can use one,
/// such as OpenRouter, never calls it. The decision therefore lives with each provider, not
/// in a shared `match` keyed on a provider name. See D10.
///
/// The clean end state is a `fn accepts_base_url(&self) -> bool` on `rho_core::Provider`,
/// with a default of `false` that OpenRouter overrides to `true`. That lives in `rho-core`,
/// which this agent does not own, so it is described here and applied locally: each builder
/// answers for itself.
fn refuse_base_url(name: &str, base_url: Option<&str>) -> Result<(), ProviderError> {
    if let Some(url) = base_url {
        return Err(ProviderError::IncompatibleBaseUrl {
            name: name.to_string(),
            url: url.to_string(),
        });
    }
    Ok(())
}

/// True when the provider is the OpenAI-compatible one that reads a base url and sends its
/// key there. Only such a provider redirects the credential, so only it earns the base-url
/// wiring notice. Bedrock and azure refuse a base url (`refuse_base_url`), so the key never
/// travels and a notice naming a key would name a secret that stays home.
///
/// This mirrors the per-provider answer in `build_provider`: OpenRouter uses a base url and
/// the others refuse it. A new OpenAI-compatible provider adds its name here, the same edit
/// it already makes to `build_provider`. The clean end state is a `Provider` trait method, as
/// `refuse_base_url` notes, and that lives in `rho-core`, which this agent does not own.
pub fn accepts_base_url(name: &str) -> bool {
    matches!(name, "openrouter")
}

#[cfg(feature = "openrouter")]
fn build_openrouter(base_url: Option<&str>) -> Result<Arc<dyn Provider>, ProviderError> {
    let key = std::env::var(OPENROUTER_KEY_ENV).unwrap_or_default();
    openrouter_from_key(&key, base_url)
}

/// Build the OpenRouter provider from a key value. A separate function, so a test
/// can check the missing-key path without changing the process environment.
#[cfg(feature = "openrouter")]
fn openrouter_from_key(
    key: &str,
    base_url: Option<&str>,
) -> Result<Arc<dyn Provider>, ProviderError> {
    use rho_provider_openrouter::OpenRouterProvider;
    Ok(Arc::new(OpenRouterProvider::new(
        openrouter_config_from_key(key, base_url)?,
    )))
}

/// Build the OpenRouter config from a key and an optional base url.
///
/// This is the seam that applies the base url. It returns the config, not the boxed
/// `Provider`, so a test can read the endpoint back and prove the base url reached it. A
/// test that could only see `Arc<dyn Provider>` could not tell an applied host from a
/// dropped one, which is the exact gap a critic found: deleting the two lines that apply
/// the host sent the bearer token to `openrouter.ai` and no test failed. See D2.
#[cfg(feature = "openrouter")]
fn openrouter_config_from_key(
    key: &str,
    base_url: Option<&str>,
) -> Result<rho_provider_openrouter::OpenRouterConfig, ProviderError> {
    use rho_core::Secret;
    use rho_provider_openrouter::OpenRouterConfig;

    let secret = Secret::new(key);
    if secret.is_empty() {
        return Err(missing(OPENROUTER_KEY_ENV, "your OpenRouter API key"));
    }
    let mut config = OpenRouterConfig::new(secret);
    // The client already carried a settable endpoint, and nothing reached it. That is the
    // whole defect. See `D-a-provider-base-url-is-a-config-key`.
    if let Some(url) = base_url {
        config = config.with_openai_host(url);
    }
    Ok(config)
}

#[cfg(not(feature = "openrouter"))]
fn build_openrouter(_base_url: Option<&str>) -> Result<Arc<dyn Provider>, ProviderError> {
    Err(ProviderError::NotCompiled {
        name: "openrouter".to_string(),
    })
}

#[cfg(feature = "bedrock")]
fn build_bedrock(base_url: Option<&str>) -> Result<Arc<dyn Provider>, ProviderError> {
    use rho_provider_bedrock::{BedrockConfig, BedrockProvider};

    // Bedrock names its endpoint through the AWS region, so a base url is a conflict.
    refuse_base_url("bedrock", base_url)?;
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
fn build_bedrock(base_url: Option<&str>) -> Result<Arc<dyn Provider>, ProviderError> {
    refuse_base_url("bedrock", base_url)?;
    Err(ProviderError::NotCompiled {
        name: "bedrock".to_string(),
    })
}

#[cfg(feature = "azure")]
fn build_azure(base_url: Option<&str>) -> Result<Arc<dyn Provider>, ProviderError> {
    use rho_core::Secret;
    use rho_provider_azure::{AzureAuth, AzureConfig, AzureProvider};

    // Azure names its endpoint and deployment its own way, so a base url is a conflict.
    refuse_base_url("azure", base_url)?;
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
fn build_azure(base_url: Option<&str>) -> Result<Arc<dyn Provider>, ProviderError> {
    refuse_base_url("azure", base_url)?;
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
        let message = expect_err(build_provider("nope", None)).to_string();
        assert!(message.contains("openrouter"), "message was: {message}");
        assert!(message.contains("--provider"), "message was: {message}");
    }

    #[test]
    fn the_bedrock_default_is_the_latest_haiku_on_a_global_profile() {
        // Measured, not chosen from a table. Three ids were run through `rho run` against live
        // Bedrock:
        //
        //   anthropic.claude-haiku-4-5-20251001-v1:0       -> 400, no on-demand throughput
        //   us.anthropic.claude-haiku-4-5-20251001-v1:0    -> works in us-east-1, 400 in eu-west-1
        //   global.anthropic.claude-haiku-4-5-20251001-v1:0 -> works in both
        //
        // So a Claude 4.5 model needs an inference profile, and the profile must be the global
        // one. A `us.` default would be a default that fails for anyone outside a US region.
        let model = default_model("bedrock").expect("bedrock has a default");
        assert!(
            model.contains("haiku-4-5"),
            "the default is the latest haiku: {model}"
        );
        assert!(
            model.starts_with("global."),
            "and it is region portable, so it must be the global inference profile: {model}"
        );
    }

    #[test]
    fn no_default_names_a_bare_claude_45_model() {
        // A bare Claude 4.5 foundation id has no on-demand throughput and returns 400. Any
        // default that names one would break on the first prompt.
        for provider in ["bedrock", "openrouter", "azure"] {
            if let Some(model) = default_model(provider) {
                let bare_claude_45 =
                    model.starts_with("anthropic.claude") && model.contains("-4-5-");
                assert!(
                    !bare_claude_45,
                    "{provider} default {model} is a bare Claude 4.5 id, which Bedrock rejects"
                );
            }
        }
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
        let message = expect_err(openrouter_from_key("", None)).to_string();
        assert!(
            message.contains(OPENROUTER_KEY_ENV),
            "message must name the variable: {message}"
        );
    }

    #[cfg(feature = "openrouter")]
    #[test]
    fn a_present_openrouter_key_builds_a_provider() {
        assert!(openrouter_from_key("sk-test-key", None).is_ok());
    }

    // ---- D2: the base-url seam, tested where it can fail ----

    #[cfg(feature = "openrouter")]
    #[test]
    fn a_base_url_reaches_the_openrouter_endpoint() {
        // The seam, not the helper beside it. `openrouter_from_key` returns `Arc<dyn
        // Provider>`, which hides the endpoint, so a test that only saw the boxed provider
        // could not tell an applied host from a dropped one. That is exactly why a critic
        // could delete the two lines that apply the host and watch a green suite. This reads
        // the config the seam produces, so deleting the apply lines fails it.
        let config =
            openrouter_config_from_key("sk-test-key", Some("https://models.example.com/v1"))
                .expect("a present key builds a config");
        assert!(
            config.chat_url().contains("models.example.com"),
            "the base url must reach the endpoint, or the bearer token goes to openrouter.ai; \
             endpoint was: {}",
            config.chat_url()
        );
    }

    #[cfg(feature = "openrouter")]
    #[test]
    fn no_base_url_keeps_the_openrouter_default_endpoint() {
        // The other half: without a base url the default host stands. So the test above
        // proves the base url moved the host, not merely that a host exists.
        let config = openrouter_config_from_key("sk-test-key", None).expect("a config");
        assert!(
            config.chat_url().contains("openrouter.ai"),
            "the default endpoint is OpenRouter's own: {}",
            config.chat_url()
        );
    }

    #[test]
    fn a_base_url_is_refused_for_a_provider_that_names_its_endpoint_its_own_way() {
        // The refusal branch every production call skipped, because every call passed `None`.
        // Deleting the refusal makes rho silently ignore base-url for bedrock. This reaches
        // it, and it also pins D11: the error is the conflict variant, not `MissingConfig`.
        let error = expect_err(build_provider(
            "bedrock",
            Some("https://models.example.com/v1"),
        ));
        assert!(
            matches!(error, ProviderError::IncompatibleBaseUrl { .. }),
            "a base url with bedrock is a conflict, not a missing value: {error:?}"
        );
        let message = error.to_string();
        assert!(
            message.contains("bedrock"),
            "the message names the provider: {message}"
        );
        assert!(
            message.contains("models.example.com"),
            "and it names the url the user set: {message}"
        );
    }

    #[test]
    fn a_base_url_is_refused_for_azure_too() {
        // Every provider that names its endpoint its own way must refuse, not just bedrock.
        let error = expect_err(build_provider(
            "azure",
            Some("https://models.example.com/v1"),
        ));
        assert!(
            matches!(error, ProviderError::IncompatibleBaseUrl { ref name, .. } if name == "azure"),
            "azure must refuse a base url with the conflict variant: {error:?}"
        );
    }
}
