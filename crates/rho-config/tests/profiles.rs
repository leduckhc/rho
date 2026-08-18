//! Named profiles from `SPEC-config` section 2 and section 7.
//!
//! A profile is a named block inside a file, not a seventh source. The merge applies
//! the profile after both files. A profile the user names but no file defines is an
//! error.

mod common;

use common::{temp_dir, write_file};
use rho_config::{Config, ConfigError, Sources};

#[test]
fn a_named_profile_overrides_the_base_keys() {
    // The profile's keys win over the base file keys.
    let dir = temp_dir();
    let path = write_file(
        &dir,
        "config.toml",
        "model = \"base-model\"\n\
         provider = \"openrouter\"\n\
         \n\
         [profiles.fast]\n\
         model = \"fast-model\"\n",
    );
    let sources = Sources {
        project_file: Some(path),
        profile: Some("fast".to_string()),
        ..Sources::default()
    };
    let config = Config::load(&sources).expect("the profile is defined");
    assert_eq!(config.model.as_deref(), Some("fast-model"));
    // The base value the profile omits still survives.
    assert_eq!(config.provider.as_deref(), Some("openrouter"));
}

#[test]
fn an_unknown_profile_name_is_an_error() {
    // A profile the user names but no file defines is `ConfigError::UnknownProfile`.
    let dir = temp_dir();
    let path = write_file(&dir, "config.toml", "model = \"base-model\"\n");
    let sources = Sources {
        project_file: Some(path),
        profile: Some("missing".to_string()),
        ..Sources::default()
    };
    match Config::load(&sources) {
        Err(ConfigError::UnknownProfile { name }) => assert_eq!(name, "missing"),
        other => panic!("an unknown profile must be UnknownProfile, got {other:?}"),
    }
}
