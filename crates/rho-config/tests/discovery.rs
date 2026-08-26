//! Config file discovery, and the one constructor that carries every source.
//!
//! Discovery is pure. It returns a path whether or not the file exists, because
//! `Config::read_file` already answers `Ok(None)` for an absent file. So a test never
//! touches the real home directory, and CI can unset `HOME` without a panic.
//!
//! See `SPEC-config-call-site` section 2 and section 4.

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;

use common::{env_vars, temp_dir, write_file};
use rho_config::{Config, ConfigLayer, ConfigPaths, Sources};

/// An `EnvLookup` that holds only what a test states. It never reads the real
/// environment, so the result does not change per machine.
fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
        .collect()
}

#[test]
fn xdg_config_home_wins_over_home() {
    let paths = ConfigPaths::discover(
        &env(&[("XDG_CONFIG_HOME", "/xdg"), ("HOME", "/home/le")]),
        &PathBuf::from("/work"),
    );
    assert_eq!(
        paths.global,
        Some(PathBuf::from("/xdg/rho/config.toml")),
        "XDG_CONFIG_HOME is the stated override, so it wins over HOME"
    );
}

#[test]
fn home_supplies_the_global_path() {
    let paths = ConfigPaths::discover(&env(&[("HOME", "/home/le")]), &PathBuf::from("/work"));
    assert_eq!(
        paths.global,
        Some(PathBuf::from("/home/le/.config/rho/config.toml")),
        "with no XDG variable the global path sits under HOME"
    );
}

#[test]
fn no_home_yields_no_global_path() {
    // CI can unset HOME. Discovery must answer `None` and never panic, because the
    // caller reports the absence: a lost global file loses a hardened setting.
    let paths = ConfigPaths::discover(&env(&[]), &PathBuf::from("/work"));
    assert_eq!(
        paths.global, None,
        "no HOME and no XDG_CONFIG_HOME means no global path, and no failure"
    );
}

#[test]
fn an_empty_home_value_yields_no_global_path() {
    // An exported-but-empty variable is a common shell accident. Joining from "" would
    // name /rho/config.toml at the filesystem root, which is never what the user meant.
    // A deliberate break that deleted the empty check passed `no_home_yields_no_global_path`,
    // so that test alone did not prove this.
    for pairs in [
        vec![("HOME", "")],
        vec![("XDG_CONFIG_HOME", "")],
        vec![("XDG_CONFIG_HOME", "   ")],
        vec![("XDG_CONFIG_HOME", ""), ("HOME", "")],
    ] {
        let paths = ConfigPaths::discover(&env(&pairs), &PathBuf::from("/work"));
        assert_eq!(
            paths.global, None,
            "an empty value counts as unset, for {pairs:?}"
        );
    }
}

#[test]
fn an_empty_xdg_value_falls_back_to_home() {
    // The empty check must not throw away a usable HOME beside an empty XDG variable.
    let paths = ConfigPaths::discover(
        &env(&[("XDG_CONFIG_HOME", ""), ("HOME", "/home/le")]),
        &PathBuf::from("/work"),
    );
    assert_eq!(
        paths.global,
        Some(PathBuf::from("/home/le/.config/rho/config.toml")),
        "an empty XDG value is unset, so HOME still supplies the path"
    );
}

#[test]
fn the_project_path_sits_under_the_bootstrap_root() {
    let paths = ConfigPaths::discover(&env(&[("HOME", "/home/le")]), &PathBuf::from("/work/repo"));
    assert_eq!(
        paths.project,
        Some(PathBuf::from("/work/repo/.rho/config.toml")),
        "the project file is .rho/config.toml under the bootstrap root"
    );
}

#[test]
fn discovery_names_a_path_that_does_not_exist() {
    // Discovery is pure, so it does no filesystem test. `read_file` answers for absence.
    let paths = ConfigPaths::discover(
        &env(&[("HOME", "/no/such/home")]),
        &PathBuf::from("/no/such/root"),
    );
    assert_eq!(
        paths.global,
        Some(PathBuf::from("/no/such/home/.config/rho/config.toml"))
    );
    assert_eq!(
        paths.project,
        Some(PathBuf::from("/no/such/root/.rho/config.toml"))
    );
}

#[test]
fn from_paths_maps_the_two_paths() {
    // The one constructor the whole contract rests on. A misplaced path sends the project
    // file into the global slot and silently reverses the merge order.
    //
    // Both files must set the **same** key, or the swap hides. An earlier version of this
    // test gave each file a different key, and a deliberate swap of the two slots passed
    // it, because both values still reached the config. See AGENTS.md step 7.
    let dir = temp_dir();
    let global = write_file(&dir, "global.toml", "model = \"from-global\"\n");
    let project = write_file(&dir, "project.toml", "model = \"from-project\"\n");
    let sources = Sources::from_paths(ConfigPaths {
        global: Some(global),
        project: Some(project),
    });

    let config = Config::load(&sources).expect("both files load");
    assert_eq!(
        config.model.as_deref(),
        Some("from-project"),
        "the project file is the stronger layer, so a swap of the two slots fails here"
    );
}

#[test]
fn the_builder_carries_env_profile_and_flags() {
    // Each source arrives by its own method, so a new source is a new method and never
    // a longer argument list. See D-no-four-argument-session-new.
    let dir = temp_dir();
    let project = write_file(
        &dir,
        "project.toml",
        "model = \"from-file\"\n\
         [profiles.work]\n\
         model = \"from-profile\"\n",
    );
    let sources = Sources::from_paths(ConfigPaths {
        global: None,
        project: Some(project),
    })
    .with_env(env_vars(&[("RHO_SESSION_FILE", "/tmp/from-env.jsonl")]))
    // It states trust, because `session-file` is a path an untrusted source may not set.
    // `session-root` and `session-file` move the confinement boundary of every file tool,
    // which a probe proved, so both are powerful. See
    // `D-trust-is-provenance-not-a-field-list`.
    .with_project_trust(rho_config::ProjectTrust::Trusted)
    .with_profile(Some("work".to_string()))
    .with_flags(ConfigLayer {
        provider: Some("from-flag".to_string()),
        ..ConfigLayer::default()
    });

    let config = Config::load(&sources).expect("every source resolves");
    assert_eq!(
        config.model.as_deref(),
        Some("from-profile"),
        "the profile beats the plain file key"
    );
    assert_eq!(
        config.session_file,
        Some(PathBuf::from("/tmp/from-env.jsonl")),
        "with_env reached the merge"
    );
    assert_eq!(
        config.provider.as_deref(),
        Some("from-flag"),
        "with_flags reached the merge"
    );
}
