//! Provider selection for the `rho` binary.
//!
//! A provider is a cargo feature. A build without a provider must fail with a
//! clear message, not a panic. A missing credential must name what to set. Every function
//! here states its choices, so no credential or session boundary is set by accident. See
//! `SPEC-core-runtime` decisions D-session-config and D-no-four-argument-session-new.
//!
//! **No function here reads the real process environment directly.** A credential comes from
//! the merged configuration, through `Config::resolve_credential_or_env`, and every other
//! value comes through an injected `&dyn EnvLookup`. So a test never depends on the machine it
//! runs on, and `unwrap_or_default()` can never turn an absent key into an empty string again.
//! See `SPEC-config-call-site` section 7.

use std::sync::Arc;

use rho_config::{Config, EnvLookup};
use rho_core::Provider;

/// The environment variable that holds the model id.
pub const MODEL_ENV: &str = "RHO_MODEL";
/// The environment variable that names the provider.
pub const PROVIDER_ENV: &str = "RHO_PROVIDER";

/// The provider-specific suggestion list for the model picker. These ids are not a claim
/// that every id works for every task; they are the small set a first-time user should see
/// when the starred file is empty. See
/// `D-the-picker-seeds-from-a-per-provider-suggestion-list`.
#[cfg(feature = "tui")]
pub fn provider_suggestions(name_or_protocol: &str) -> Vec<String> {
    match name_or_protocol {
        "openrouter" => vec![
            "anthropic/claude-haiku-4.5".to_string(),
            "anthropic/claude-sonnet-4.5".to_string(),
            "openai/gpt-5".to_string(),
            "openai/gpt-4o-mini".to_string(),
            "google/gemini-2.5-pro".to_string(),
        ],
        "bedrock" => vec![
            "amazon.nova-micro-v1:0".to_string(),
            "amazon.nova-pro-v1:0".to_string(),
            "anthropic.claude-3-5-sonnet-20241022-v2:0".to_string(),
        ],
        "anthropic" => vec![
            "claude-haiku-4-5".to_string(),
            "claude-sonnet-4-5".to_string(),
            "claude-opus-4-5".to_string(),
        ],
        // Azure names deployments, not models, and only the account owner knows the
        // deployment names. So we suggest nothing. See `docs/verification/models.md`.
        _ => Vec::new(),
    }
}

/// The Anthropic API key variable.
#[cfg(feature = "anthropic")]
pub const ANTHROPIC_KEY_ENV: &str = "ANTHROPIC_API_KEY";
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
        "the provider \"{name}\" is not known. Choose one of: openrouter, bedrock, azure, anthropic. Set it with --provider or the {PROVIDER_ENV} variable."
    )]
    Unknown { name: String },
    /// The provider name is valid, but this build does not include it.
    #[error(
        "the provider \"{name}\" is not in this build. Rebuild rho with the feature: cargo build --features {name}."
    )]
    NotCompiled { name: String },
    /// A required environment variable is missing or empty.
    ///
    /// Only a provider that reads a plain environment variable builds this, and the
    /// `minimal` build has no provider at all. So the variant is gated: `-D warnings` in CI
    /// makes an unconstructed variant an error, and that job is the one guard this project
    /// has against a feature combination nobody builds by hand.
    #[cfg(any(feature = "bedrock", feature = "azure"))]
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
    /// A credential could not be resolved.
    ///
    /// `rho-config` names the reason, and the reason may be a refusal rather than an absence,
    /// so the message is passed through whole. No `ConfigError::Credential` message holds a
    /// credential value, and `a_credential_error_never_holds_the_resolved_value` pins that.
    #[error("{message}")]
    Credential { message: String },
}

/// Build the message for a missing environment variable.
///
/// Gated with the variant it builds. Every caller sits inside a `bedrock` or `azure` block.
#[cfg(any(feature = "bedrock", feature = "azure"))]
fn missing(var: &str, purpose: &str) -> ProviderError {
    ProviderError::MissingConfig {
        message: format!("set the {var} environment variable to {purpose}."),
    }
}

/// Turn a credential failure into a provider failure, with the message intact.
fn credential_error(error: rho_config::ConfigError) -> ProviderError {
    ProviderError::Credential {
        message: error.to_string(),
    }
}

/// The set of known provider names, whether compiled in or not.
const KNOWN_PROVIDERS: [&str; 4] = ["openrouter", "bedrock", "azure", "anthropic"];

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

/// Whether this build can use a provider name, with no credential work.
///
/// A caller that only needs to know whether a name is usable must not run a credential
/// helper to find out. The JSONL frontend is that caller: it maps an unknown or absent
/// provider onto one wire error, and it does that before the merged configuration exists.
///
/// `check_provider_name_agrees_with_build_provider` pins the two name lists together, so they
/// cannot drift.
pub fn check_provider_name(name: &str) -> Result<(), ProviderError> {
    let compiled = match name {
        "openrouter" => cfg!(feature = "openrouter"),
        "bedrock" => cfg!(feature = "bedrock"),
        "azure" => cfg!(feature = "azure"),
        other => {
            return Err(ProviderError::Unknown {
                name: other.to_string(),
            });
        }
    };
    if compiled {
        Ok(())
    } else {
        Err(ProviderError::NotCompiled {
            name: name.to_string(),
        })
    }
}

/// Build a provider by name, and resolve its credential through the merged configuration.
///
/// It reads no environment variable directly. `env` is the lookup `rho-config` uses, so a
/// test never touches the real process environment. `base_url` is no longer an argument,
/// because it already lives in `Config`, so this call list got shorter and not longer. See
/// `D-no-four-argument-session-new`.
pub fn build_provider(
    name: &str,
    config: &Config,
    env: &dyn EnvLookup,
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
    // Built-ins first. A named entry cannot shadow a built-in; the parser rejects a
    // collision by name. See `D-a-built-in-is-never-shadowed`.
    match name {
        "openrouter" => return build_openrouter(config, env),
        "bedrock" => return build_bedrock(config, env),
        "azure" => return build_azure(config, env),
        "anthropic" => return build_anthropic(config, env),
        _ => {}
    }
    // A named entry from `[[providers]]`.
    if let Some(entry) = config.providers.iter().find(|entry| entry.id == name) {
        return build_named_provider(entry, config, env);
    }
    // A known name the build does not include.
    if KNOWN_PROVIDERS.contains(&name) {
        return Err(ProviderError::NotCompiled {
            name: name.to_string(),
        });
    }
    Err(ProviderError::Unknown {
        name: name.to_string(),
    })
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
fn build_openrouter(
    config: &Config,
    env: &dyn EnvLookup,
) -> Result<Arc<dyn Provider>, ProviderError> {
    use rho_provider_openrouter::OpenRouterProvider;

    Ok(Arc::new(OpenRouterProvider::new(openrouter_config_from(
        config, env,
    )?)))
}

/// Build the OpenRouter configuration from the merged config and the environment.
///
/// It returns the config, not the boxed `Provider`, so a test can read the key back and the
/// endpoint back. A test that could only see `Arc<dyn Provider>` could not tell an applied
/// host from a dropped one, and it could not tell a key from the file from a key from the
/// environment. Both gaps were found by a deliberate break. See D2.
///
/// This is the only OpenRouter seam. Two helpers, `openrouter_from_key` and
/// `openrouter_config_from_key`, used to sit beside it for the tests, and once the credential
/// came from `Config` they had no production caller and carried an unreachable empty check.
#[cfg(feature = "openrouter")]
fn openrouter_config_from(
    config: &Config,
    env: &dyn EnvLookup,
) -> Result<rho_provider_openrouter::OpenRouterConfig, ProviderError> {
    use rho_provider_openrouter::OpenRouterConfig;

    // The credential name is the provider name, because `credentials` is a map from a
    // provider name to a `CredentialSource`. The fallback variable is named here, in this
    // builder, so a fourth provider names its own and edits no shared code and no table in
    // `rho-config`. See `D-a-provider-names-its-own-credential`.
    //
    // `resolve_credential_or_env` refuses an absent and an empty credential, so no empty key
    // can reach the wire and no check here is needed.
    let secret = config
        .resolve_credential_or_env("openrouter", OPENROUTER_KEY_ENV, env)
        .map_err(credential_error)?;
    let mut built = OpenRouterConfig::new(secret);
    // The client already carried a settable endpoint, and nothing reached it. That is the
    // whole defect. See `D-a-provider-base-url-is-a-config-key`.
    if let Some(url) = &config.base_url {
        built = built.with_openai_host(url);
    }
    Ok(built)
}

#[cfg(not(feature = "openrouter"))]
fn build_openrouter(
    _config: &Config,
    _env: &dyn EnvLookup,
) -> Result<Arc<dyn Provider>, ProviderError> {
    Err(ProviderError::NotCompiled {
        name: "openrouter".to_string(),
    })
}

#[cfg(feature = "bedrock")]
fn build_bedrock(config: &Config, env: &dyn EnvLookup) -> Result<Arc<dyn Provider>, ProviderError> {
    use rho_provider_bedrock::{BedrockConfig, BedrockProvider};

    // Bedrock names its endpoint through the AWS region, so a base url is a conflict.
    refuse_base_url("bedrock", config.base_url.as_deref())?;

    // **Bedrock resolves no credential, on purpose.** The AWS SDK owns its own chain:
    // environment variables, a profile, the SSO cache, and IMDS. A region is not a secret,
    // and wrapping it in a `Secret` would break the chain `F-aws-bedrock-provider` states.
    // So this reads the region and nothing else. See `SPEC-config-call-site` section 7.
    let region = env.get(AWS_REGION_ENV).unwrap_or_default();
    if region.trim().is_empty() {
        return Err(missing(
            AWS_REGION_ENV,
            "your AWS region, for example us-east-1",
        ));
    }
    let config = BedrockConfig::new(region);
    Ok(Arc::new(BedrockProvider::new(config)))
}

#[cfg(not(feature = "bedrock"))]
fn build_bedrock(
    config: &Config,
    _env: &dyn EnvLookup,
) -> Result<Arc<dyn Provider>, ProviderError> {
    refuse_base_url("bedrock", config.base_url.as_deref())?;
    Err(ProviderError::NotCompiled {
        name: "bedrock".to_string(),
    })
}

/// Build a provider from a named `[[providers]]` entry. It looks up the credential in the
/// merged table and dispatches by protocol.
fn build_named_provider(
    entry: &rho_config::ProviderEntry,
    config: &Config,
    env: &dyn EnvLookup,
) -> Result<Arc<dyn Provider>, ProviderError> {
    // The credential is a name that resolves in the merged credentials table. No env
    // fallback here: a named entry must name a real credential. See
    // `SPEC-named-provider-profiles` amendment 2.
    let secret = config
        .resolve_credential(&entry.credential, env)
        .map_err(|error| ProviderError::Credential {
            message: format!(
                "named provider entry \"{}\" references credential \"{}\": {}",
                entry.id, entry.credential, error
            ),
        })?;

    match entry.protocol.as_str() {
        #[cfg(feature = "anthropic")]
        "anthropic" => {
            use rho_provider_anthropic::{AnthropicConfig, AnthropicProvider};
            Ok(Arc::new(AnthropicProvider::new(AnthropicConfig::new(
                entry.base_url.clone(),
                secret,
            ))))
        }
        #[cfg(feature = "openrouter")]
        "openai-chat" => {
            use rho_provider_openrouter::{OpenRouterConfig, OpenRouterProvider};
            // The OpenRouter crate speaks OpenAI Chat Completions against any base URL,
            // per `with_openai_host`. A `plain` OpenAI host and Ollama land here today; a
            // dedicated `rho-provider-openai-chat` lands in a later commit.
            let config = OpenRouterConfig::new(secret).with_openai_host(&entry.base_url);
            Ok(Arc::new(OpenRouterProvider::new(config)))
        }
        "openai-responses" => Err(ProviderError::Credential {
            message: format!(
                "named provider entry \"{}\" uses `openai-responses`, which is not yet \
                 available through a named entry. Use the built-in `azure` provider with \
                 `AZURE_OPENAI_ENDPOINT`, `AZURE_OPENAI_DEPLOYMENT`, and \
                 `AZURE_OPENAI_API_KEY` for now. A named-entry path lands in a follow-up.",
                entry.id
            ),
        }),
        protocol => Err(ProviderError::Credential {
            message: format!(
                "named provider entry \"{}\" uses unknown protocol \"{}\". \
                 Choose one of: anthropic, openai-chat, openai-responses.",
                entry.id, protocol
            ),
        }),
    }
}

/// Build the Anthropic provider.
///
/// A base URL is optional. The default is `https://api.anthropic.com`. The xdent tunnel and
/// any Anthropic-compatible proxy pass through `--base-url`. The credential name is the
/// provider name, so `[credentials.anthropic]` in the config sets it, and
/// `ANTHROPIC_API_KEY` is the env-var fallback. See
/// `D-a-provider-names-its-own-credential` and `SPEC-anthropic-messages-provider`.
#[cfg(feature = "anthropic")]
fn build_anthropic(
    config: &Config,
    env: &dyn EnvLookup,
) -> Result<Arc<dyn Provider>, ProviderError> {
    use rho_provider_anthropic::{AnthropicConfig, AnthropicProvider, DEFAULT_BASE_URL};

    let secret = config
        .resolve_credential_or_env("anthropic", ANTHROPIC_KEY_ENV, env)
        .map_err(credential_error)?;
    let base_url = config
        .base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
    Ok(Arc::new(AnthropicProvider::new(AnthropicConfig::new(
        base_url, secret,
    ))))
}

#[cfg(not(feature = "anthropic"))]
fn build_anthropic(
    _config: &Config,
    _env: &dyn EnvLookup,
) -> Result<Arc<dyn Provider>, ProviderError> {
    Err(ProviderError::NotCompiled {
        name: "anthropic".to_string(),
    })
}

#[cfg(feature = "azure")]
fn build_azure(config: &Config, env: &dyn EnvLookup) -> Result<Arc<dyn Provider>, ProviderError> {
    use rho_provider_azure::AzureProvider;

    Ok(Arc::new(AzureProvider::new(azure_config_from(
        config, env,
    )?)))
}

/// Build the Azure configuration from the merged config and the environment.
///
/// It returns the config, not the boxed `Provider`, so a test can read the key back and prove
/// it came from the config file. A test that could only see `Arc<dyn Provider>` could not tell
/// a key from the file from a key from the environment: `AzureConfig::new` accepts any value,
/// so a build succeeded either way. A mutation caught exactly that, and the same seam and the
/// same reason already exist for OpenRouter. See `a_base_url_reaches_the_openrouter_endpoint`.
#[cfg(feature = "azure")]
fn azure_config_from(
    config: &Config,
    env: &dyn EnvLookup,
) -> Result<rho_provider_azure::AzureConfig, ProviderError> {
    use rho_provider_azure::{AzureAuth, AzureConfig};

    // Azure names its endpoint and deployment its own way, so a base url is a conflict.
    refuse_base_url("azure", config.base_url.as_deref())?;

    // The endpoint and the deployment are not secrets, so they stay environment reads and
    // gain no config key. U1(a) of `.rho-work/i15-credential-expansion.md` chose that. They
    // now go through the same `&dyn EnvLookup`, so a test isolates them.
    let endpoint = env.get(AZURE_ENDPOINT_ENV).unwrap_or_default();
    if endpoint.trim().is_empty() {
        return Err(missing(
            AZURE_ENDPOINT_ENV,
            "your Azure OpenAI endpoint, for example https://my-resource.openai.azure.com",
        ));
    }
    let deployment = env.get(AZURE_DEPLOYMENT_ENV).unwrap_or_default();
    if deployment.trim().is_empty() {
        return Err(missing(
            AZURE_DEPLOYMENT_ENV,
            "your Azure OpenAI deployment name",
        ));
    }
    // The key is a credential, so it comes from the merged configuration. Azure spells its
    // variable `AZURE_OPENAI_API_KEY`, and this builder is the only place that knows it.
    //
    // There is no second empty check here. `resolve_credential_or_env` refuses an empty
    // credential, so a check here is unreachable: two deliberate breaks left it unreached by
    // any test, and step 8 says delete a branch no test reaches.
    let secret = config
        .resolve_credential_or_env("azure", AZURE_KEY_ENV, env)
        .map_err(credential_error)?;
    Ok(AzureConfig::new(
        endpoint,
        deployment,
        AzureAuth::ApiKey(secret),
    ))
}

#[cfg(not(feature = "azure"))]
fn build_azure(config: &Config, _env: &dyn EnvLookup) -> Result<Arc<dyn Provider>, ProviderError> {
    refuse_base_url("azure", config.base_url.as_deref())?;
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

    /// Load a `Config` from one global config file body.
    ///
    /// A global file is the user's own, so nothing is gated and a credential in it resolves.
    /// The file is under a `tempfile::TempDir`, so no test reads a real config path. The
    /// directory handle comes back, because dropping it removes the file.
    fn config_from_global(body: &str) -> (Config, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("global.toml");
        std::fs::write(&path, body).expect("write the global config");
        let sources = rho_config::Sources::from_paths(rho_config::ConfigPaths {
            global: Some(path),
            project: None,
            ..Default::default()
        });
        (
            rho_config::Config::load(&sources).expect("the file is valid TOML"),
            dir,
        )
    }

    /// Load a `Config` from one **project** config file body, untrusted. That is the layer a
    /// clone controls, so it is the one the trust gate has to stop.
    fn config_from_untrusted_project(body: &str) -> (Config, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("project.toml");
        std::fs::write(&path, body).expect("write the project config");
        let sources = rho_config::Sources::from_paths(rho_config::ConfigPaths {
            global: None,
            project: Some(path),
            ..Default::default()
        })
        .with_project_trust(rho_config::ProjectTrust::Untrusted);
        (
            rho_config::Config::load(&sources).expect("the file is valid TOML"),
            dir,
        )
    }

    /// An in-memory environment. No test mutates the process environment, so no test result
    /// depends on the machine it runs on.
    fn env(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn unknown_provider_names_the_choices() {
        let (config, _dir) = config_from_global("");
        let message = expect_err(build_provider("nope", &config, &env(&[]))).to_string();
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
        // It goes through the real seam now. The helper it used to call had no production
        // caller once the credential came from `Config`, and it carried an empty check that
        // two deliberate breaks left unreached.
        let (config, _dir) = config_from_global("");
        let message = expect_err(build_provider("openrouter", &config, &env(&[]))).to_string();
        assert!(
            message.contains(OPENROUTER_KEY_ENV),
            "message must name the variable: {message}"
        );
    }

    #[cfg(feature = "openrouter")]
    #[test]
    fn a_present_openrouter_key_builds_a_provider() {
        let (config, _dir) = config_from_global("");
        assert!(
            build_provider(
                "openrouter",
                &config,
                &env(&[(OPENROUTER_KEY_ENV, "sk-test-key")])
            )
            .is_ok()
        );
    }

    // ---- D2: the base-url seam, tested where it can fail ----

    #[cfg(feature = "openrouter")]
    #[test]
    fn a_base_url_reaches_the_openrouter_endpoint() {
        // The seam, not the boxed provider. `build_provider` returns `Arc<dyn Provider>`,
        // which hides the endpoint, so a test that only saw the boxed provider could not tell
        // an applied host from a dropped one. That is exactly why a critic could delete the two
        // lines that apply the host and watch a green suite. This reads the config the seam
        // produces, so deleting the apply lines fails it.
        let (config, _dir) = config_from_global("base-url = \"https://models.example.com/v1\"\n");
        let built = openrouter_config_from(&config, &env(&[(OPENROUTER_KEY_ENV, "sk-test-key")]))
            .expect("a present key builds a config");
        assert!(
            built.chat_url().contains("models.example.com"),
            "the base url must reach the endpoint, or the bearer token goes to openrouter.ai; \
             endpoint was: {}",
            built.chat_url()
        );
    }

    #[cfg(feature = "openrouter")]
    #[test]
    fn no_base_url_keeps_the_openrouter_default_endpoint() {
        // The other half: without a base url the default host stands. So the test above
        // proves the base url moved the host, not merely that a host exists.
        let (config, _dir) = config_from_global("");
        let built = openrouter_config_from(&config, &env(&[(OPENROUTER_KEY_ENV, "sk-test-key")]))
            .expect("a config");
        assert!(
            built.chat_url().contains("openrouter.ai"),
            "the default endpoint is OpenRouter's own: {}",
            built.chat_url()
        );
    }

    #[test]
    fn a_base_url_is_refused_for_a_provider_that_names_its_endpoint_its_own_way() {
        // The refusal branch every production call skipped, because every call passed `None`.
        // Deleting the refusal makes rho silently ignore base-url for bedrock. This reaches
        // it, and it also pins D11: the error is the conflict variant, not `MissingConfig`.
        let (config, _dir) = config_from_global("base-url = \"https://models.example.com/v1\"\n");
        let error = expect_err(build_provider("bedrock", &config, &env(&[])));
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
        let (config, _dir) = config_from_global("base-url = \"https://models.example.com/v1\"\n");
        let error = expect_err(build_provider("azure", &config, &env(&[])));
        assert!(
            matches!(error, ProviderError::IncompatibleBaseUrl { ref name, .. } if name == "azure"),
            "azure must refuse a base url with the conflict variant: {error:?}"
        );
    }

    // ---- I15: a provider gets its credential from the config ----

    #[cfg(feature = "openrouter")]
    #[test]
    fn the_openrouter_key_comes_from_the_config_file() {
        // The whole point of half one. A `[credentials]` entry named after the provider
        // reaches the provider, with no environment variable set at all.
        //
        // It reads the key back off the config, and does not merely check that a build
        // succeeded. A mutation proved why: a build that read the environment directly still
        // returned `Ok`, so an `is_ok()` assertion passed against the very bug it was
        // written for.
        let (config, _dir) =
            config_from_global("[credentials]\nopenrouter = \"sk-from-the-config-file\"\n");
        let built = openrouter_config_from(&config, &env(&[]))
            .expect("a config file must be able to supply the key");
        assert_eq!(
            built.api_key.expose(),
            "sk-from-the-config-file",
            "the key must come from the file, and not from the environment"
        );
    }

    #[cfg(feature = "azure")]
    #[test]
    fn the_azure_key_comes_from_the_config_file() {
        // One provider is not every provider. Azure keeps its endpoint and deployment in the
        // environment, because neither is a secret, and only the key comes from the config.
        //
        // The environment holds a **different** key here, so a build that read the
        // environment directly fails this test rather than passing it.
        let (config, _dir) =
            config_from_global("[credentials]\nazure = \"sk-azure-from-the-file\"\n");
        let vars = env(&[
            (AZURE_ENDPOINT_ENV, "https://my-resource.openai.azure.com"),
            (AZURE_DEPLOYMENT_ENV, "my-deployment"),
            (AZURE_KEY_ENV, "sk-azure-from-the-environment"),
        ]);
        let built = azure_config_from(&config, &vars)
            .expect("a config file must be able to supply the azure key");
        let (header, value) = built.auth.header();
        assert_eq!(header, "api-key");
        assert_eq!(
            value, "sk-azure-from-the-file",
            "the file beats the environment, and the key really reaches the request header"
        );
        assert_eq!(built.deployment, "my-deployment");
    }

    #[cfg(feature = "openrouter")]
    #[test]
    fn a_missing_openrouter_credential_names_what_to_set() {
        // `unwrap_or_default()` turned an absent key into an empty string, so the user read a
        // provider 401 and blamed their account. The message must name the variable instead.
        let (config, _dir) = config_from_global("");
        let error = expect_err(build_provider("openrouter", &config, &env(&[])));
        let message = error.to_string();
        assert!(
            matches!(error, ProviderError::Credential { .. }),
            "an absent credential is a credential failure: {error:?}"
        );
        assert!(
            message.contains(OPENROUTER_KEY_ENV),
            "the message must name the variable to set: {message}"
        );
    }

    #[cfg(feature = "azure")]
    #[test]
    fn a_missing_azure_credential_names_what_to_set() {
        let (config, _dir) = config_from_global("");
        let vars = env(&[
            (AZURE_ENDPOINT_ENV, "https://my-resource.openai.azure.com"),
            (AZURE_DEPLOYMENT_ENV, "my-deployment"),
        ]);
        let error = expect_err(build_provider("azure", &config, &vars));
        let message = error.to_string();
        assert!(
            message.contains(AZURE_KEY_ENV),
            "the message must name the variable to set: {message}"
        );
    }

    #[cfg(feature = "openrouter")]
    #[test]
    fn an_empty_openrouter_credential_is_refused() {
        // An exported-but-empty variable is not a key. It used to reach the provider and
        // return 401, which reads as a broken account rather than a missing key.
        let (config, _dir) = config_from_global("");
        let error = expect_err(build_provider(
            "openrouter",
            &config,
            &env(&[(OPENROUTER_KEY_ENV, "")]),
        ));
        assert!(
            matches!(error, ProviderError::Credential { .. }),
            "an empty key is a credential failure, not a provider 401: {error:?}"
        );
    }

    #[cfg(feature = "openrouter")]
    #[test]
    fn an_untrusted_project_command_credential_fails_the_provider_build() {
        // The gate finally runs at the call site. The fallback variable is **set** here, so an
        // implementation that fell back after the refusal would build a provider and pass.
        let (config, _dir) =
            config_from_untrusted_project("[credentials]\nopenrouter = \"!echo leaked\"\n");
        let error = expect_err(build_provider(
            "openrouter",
            &config,
            &env(&[(OPENROUTER_KEY_ENV, "sk-from-the-env")]),
        ));
        let message = error.to_string();
        assert!(
            message.contains("--trust-project"),
            "the refusal must name the flag that fixes it: {message}"
        );
    }

    #[cfg(feature = "bedrock")]
    #[test]
    fn bedrock_needs_no_credential_entry() {
        // The AWS SDK owns its own chain: environment variables, a profile, the SSO cache, and
        // IMDS. So an empty `[credentials]` table must not stop a Bedrock build.
        let (config, _dir) = config_from_global("[credentials]\n");
        assert!(
            build_provider("bedrock", &config, &env(&[(AWS_REGION_ENV, "us-east-1")])).is_ok(),
            "bedrock resolves no credential through rho"
        );
    }

    #[cfg(feature = "bedrock")]
    #[test]
    fn bedrock_still_reads_its_region_from_the_environment() {
        // A region is not a secret, so it is not a credential. It stays an environment read,
        // and an absent region still names the variable.
        let (config, _dir) = config_from_global("");
        let error = expect_err(build_provider("bedrock", &config, &env(&[])));
        let message = error.to_string();
        assert!(
            message.contains(AWS_REGION_ENV),
            "the message names the region variable: {message}"
        );
        assert!(
            !matches!(error, ProviderError::Credential { .. }),
            "a region is not a credential: {error:?}"
        );
    }

    #[test]
    fn no_provider_builder_reads_the_process_environment() {
        // A source guard. Five `std::env::var` calls here turned an absent key into an empty
        // string, and `SPEC-config-call-site` section 2 forbids a credential read that way. A
        // sixth must not come back.
        //
        // A grep cannot catch an aliased import, such as `use std::env as sys; sys::var(...)`,
        // so it is not the whole guard. `a_provider_builder_honours_the_injected_environment`
        // is the behavioural half, and it cannot be walked around. A review named the bypass.
        // The needle is built from two pieces, so this guard's own line does not match it.
        let needle = concat!("std::env", "::var");
        let source = include_str!("provider.rs");
        let reads: Vec<&str> = source
            .lines()
            .filter(|line| line.contains(needle))
            .filter(|line| !line.trim_start().starts_with("//"))
            // The behavioural test above reads the real environment on purpose, to prove the
            // value it injects is not already there.
            .filter(|line| !line.contains("OPENROUTER_KEY_ENV).is_ok_and("))
            .collect();
        assert!(
            reads.is_empty(),
            "every production read goes through &dyn EnvLookup, got: {reads:?}"
        );
    }

    #[cfg(feature = "openrouter")]
    #[test]
    fn a_provider_builder_honours_the_injected_environment() {
        // The behavioural half of the guard above, and the one that cannot be walked around.
        //
        // A source grep catches the literal spelling `std::env::var`. It does not catch
        // `use std::env as sys; sys::var(...)`, which reads the real process environment just
        // as well. So this injects a value the real process environment does not hold, and
        // asserts the builder used it. A builder that read the real environment sees nothing
        // and fails.
        let injected = "sk-only-in-the-injected-map";
        assert!(
            !std::env::var(OPENROUTER_KEY_ENV).is_ok_and(|value| value == injected),
            "the real environment must not hold the injected value, or this proves nothing"
        );
        let (config, _dir) = config_from_global("");
        let built = openrouter_config_from(&config, &env(&[(OPENROUTER_KEY_ENV, injected)]))
            .expect("the injected key resolves");
        assert_eq!(
            built.api_key.expose(),
            injected,
            "the builder must read the lookup it was given, and not the real environment"
        );
    }

    #[cfg(feature = "tui")]
    #[test]
    fn openrouter_suggestions_are_non_empty_and_unique() {
        let ids = provider_suggestions("openrouter");
        assert!(!ids.is_empty(), "a first-time picker needs seed rows");
        let unique: std::collections::HashSet<_> = ids.iter().cloned().collect();
        assert_eq!(
            ids.len(),
            unique.len(),
            "the suggestion list has no duplicates: {ids:?}"
        );
    }

    #[cfg(feature = "tui")]
    #[test]
    fn azure_has_no_suggestions_because_it_names_deployments() {
        assert!(
            provider_suggestions("azure").is_empty(),
            "azure suggestions stay empty"
        );
    }

    #[test]
    fn check_provider_name_agrees_with_build_provider() {
        // Two name lists exist: the `match` in `build_provider` and the `match` in
        // `check_provider_name`. A drift would make the JSONL frontend report an unknown
        // provider for a provider that works, or the reverse. This pins them together.
        let (config, _dir) = config_from_global("");
        for name in ["openrouter", "bedrock", "azure", "nope", ""] {
            let checked = check_provider_name(name);
            let built = build_provider(name, &config, &env(&[]));
            let class = |error: &ProviderError| match error {
                ProviderError::Unknown { .. } => "unknown",
                ProviderError::NotCompiled { .. } => "not-compiled",
                _ => "usable",
            };
            let checked_class = checked.as_ref().err().map_or("usable", class);
            let built_class = built.as_ref().err().map_or("usable", class);
            assert_eq!(
                checked_class, built_class,
                "the two name lists disagree about {name:?}"
            );
        }
    }
}
