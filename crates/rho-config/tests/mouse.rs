//! The one interface key in the config: whether rho captures the mouse.
//!
//! Capture is off by default, so the terminal keeps drag-select and its own wheel. See
//! `D-native-selection-is-the-default` and `SPEC-tui-inline-and-composer` section 5.

mod common;

use common::{env_vars, project_sources, temp_dir, write_file};
use rho_config::{Config, ConfigLayer};

/// Load a config from one project file and one environment list.
fn load(file: &str, env: &[(&str, &str)]) -> Config {
    let dir = temp_dir();
    let project = write_file(&dir, "project.toml", file);
    let sources = project_sources(project).with_env(env_vars(env));
    Config::load(&sources).expect("the sources resolve")
}

#[test]
fn the_config_key_defaults_to_true() {
    // It asserted `false` under `D-native-selection-is-the-default`, when the interface drew an
    // inline band and the terminal kept the scrollback. The merge with the alternate-screen
    // renderer reversed it: that screen has no scrollback, so with capture off the wheel does
    // nothing at all. See `D-the-wheel-needs-capture`.
    let config = load("", &[]);
    assert!(
        config.tui_mouse,
        "capture is on unless the user turns it off"
    );
}

#[test]
fn the_config_file_can_turn_the_mouse_on() {
    let config = load("tui-mouse = true\n", &[]);
    assert!(
        config.tui_mouse,
        "the file key must reach the resolved value"
    );
}

#[test]
fn the_env_var_sets_the_mouse_key() {
    let config = load("", &[("RHO_TUI_MOUSE", "true")]);
    assert!(config.tui_mouse, "every scalar key has an environment form");
}

#[test]
fn the_env_var_wins_over_the_file() {
    let config = load("tui-mouse = true\n", &[("RHO_TUI_MOUSE", "false")]);
    assert!(
        !config.tui_mouse,
        "an environment value always overrides a file value"
    );
}

#[test]
fn a_bad_mouse_value_is_omitted_from_the_env_layer() {
    let layer = ConfigLayer::from_env(&env_vars(&[("RHO_TUI_MOUSE", "yes please")]));
    assert!(
        layer.tui_mouse.is_none(),
        "an unaccepted boolean is omitted here, so the merged parser stays the authority"
    );
}
