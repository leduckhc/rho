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

// ---- The effort level. See SPEC-reasoning-across-providers section 9. ----

/// The file key reaches the merged config.
#[test]
fn the_effort_key_reaches_the_config() {
    let config = load("reasoning-effort = \"high\"\n", &[]);
    assert_eq!(
        config.reasoning_effort,
        Some(rho_core::ReasoningEffort::High)
    );
}

/// An absent key stays unset, so the provider keeps its own default.
#[test]
fn an_absent_effort_key_stays_unset() {
    let config = load("", &[]);
    assert_eq!(config.reasoning_effort, None);
}

/// A bad value stops the run, and the message names the key and the value.
#[test]
fn a_bad_reasoning_effort_value_fails_closed() {
    let dir = temp_dir();
    let project = write_file(&dir, "project.toml", "reasoning-effort = \"ludicrous\"\n");
    let sources = project_sources(project);
    let error = Config::load(&sources).expect_err("a bad level fails closed");
    let text = error.to_string();
    assert!(
        text.contains("reasoning-effort") && text.contains("ludicrous"),
        "the error names the key and the value: {text}"
    );
}

/// The environment sets the level too, and layer 5 beats layer 3.
#[test]
fn the_effort_variable_beats_the_file() {
    let config = load(
        "reasoning-effort = \"low\"\n",
        &[("RHO_REASONING_EFFORT", "xhigh")],
    );
    assert_eq!(
        config.reasoning_effort,
        Some(rho_core::ReasoningEffort::XHigh)
    );
}
