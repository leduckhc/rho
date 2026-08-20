//! The reasoning display key in the config: how the TUI draws a model's reasoning.
//!
//! `Summary` is the default. See `SPEC-reasoning-across-providers` section 5.

mod common;

use common::{env_vars, project_sources, temp_dir, write_file};
use rho_config::Config;
use rho_core::ReasoningDisplay;

/// Load a config from one project file and one environment list.
fn load(file: &str, env: &[(&str, &str)]) -> Config {
    let dir = temp_dir();
    let project = write_file(&dir, "project.toml", file);
    let sources = project_sources(project).with_env(env_vars(env));
    Config::load(&sources).expect("the sources resolve")
}

#[test]
fn the_reasoning_key_defaults_to_summary() {
    let config = load("", &[]);
    assert_eq!(config.reasoning, ReasoningDisplay::Summary);
}

#[test]
fn the_config_file_sets_the_reasoning_mode() {
    let config = load("tui-reasoning = \"full\"\n", &[]);
    assert_eq!(config.reasoning, ReasoningDisplay::Full);
}

#[test]
fn the_env_var_sets_the_reasoning_mode() {
    let config = load("", &[("RHO_TUI_REASONING", "live")]);
    assert_eq!(config.reasoning, ReasoningDisplay::Live);
}

#[test]
fn the_env_var_wins_over_the_file() {
    let config = load(
        "tui-reasoning = \"full\"\n",
        &[("RHO_TUI_REASONING", "off")],
    );
    assert_eq!(config.reasoning, ReasoningDisplay::Off);
}

#[test]
fn a_bad_reasoning_value_fails_closed() {
    let dir = temp_dir();
    let project = write_file(&dir, "project.toml", "tui-reasoning = \"loud\"\n");
    let sources = project_sources(project);
    assert!(
        Config::load(&sources).is_err(),
        "an unknown mode is a typed error, never a silent default"
    );
}
