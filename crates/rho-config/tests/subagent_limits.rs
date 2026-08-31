//! The `[subagents]` table reaches the agent, and a project file may only lower a limit.
//!
//! A limit is a bound, so the stricter value is the smaller one. A project file may lower a
//! cap. It may never raise one. A home directory is not a clone, so the user's own global
//! file may set any value.
//!
//! See `SPEC-subagent-limits-are-a-floor`, `D-a-project-file-only-lowers-a-limit`, and
//! `D-your-settings-are-a-floor`.
//!
//! No test reads a real config path or the real process environment.

mod common;

use std::time::Duration;

use common::{temp_dir, write_file};
use rho_config::{Config, ConfigPaths, ProjectTrust, Sources};
use rho_core::SubagentLimits;
use tempfile::TempDir;

/// Load a config from a global file and a project file. Either may be empty.
///
/// The `TempDir` comes back so the caller keeps the files alive.
fn load(global: &str, project: &str, trust: ProjectTrust) -> (Config, TempDir) {
    let dir = temp_dir();
    let global = write_file(&dir, "global.toml", global);
    let project = write_file(&dir, "project.toml", project);
    let sources = Sources::from_paths(ConfigPaths {
        global: Some(global),
        project: Some(project),
        ..Default::default()
    })
    .with_project_trust(trust);
    let config = Config::load(&sources).expect("both files are valid TOML");
    (config, dir)
}

/// Load with a profile selected, so a nested `[subagents]` table is reachable.
fn load_with_profile(global: &str, project: &str, profile: &str) -> (Config, TempDir) {
    let dir = temp_dir();
    let global = write_file(&dir, "global.toml", global);
    let project = write_file(&dir, "project.toml", project);
    let sources = Sources::from_paths(ConfigPaths {
        global: Some(global),
        project: Some(project),
        ..Default::default()
    })
    .with_profile(Some(profile.to_string()));
    let config = Config::load(&sources).expect("both files are valid TOML");
    (config, dir)
}

// ---- the merge: a table that names one limit states one limit ----

#[test]
fn a_project_subagents_table_does_not_erase_a_global_limit() {
    // `merge` replaced the whole table with `over.subagents.or(self.subagents)`, so one
    // project table dropped every limit the global file set. That is defect C4's shape one
    // type down. Both limits below must survive.
    let (config, _dir) = load(
        "[subagents]\nmax-live-total = 7\n",
        "[subagents]\nmax-children-per-parent = 2\n",
        ProjectTrust::Untrusted,
    );
    assert_eq!(
        config.subagents.max_live_total, 7,
        "the global limit must survive a project table that names another limit"
    );
    assert_eq!(
        config.subagents.max_children_per_parent, 2,
        "and the project limit applies, because it is stricter"
    );
}

// ---- a config file lowers each limit. One test per limit, not one for the set ----

#[test]
fn a_config_file_lowers_the_depth_limit() {
    let (config, _dir) = load("", "[subagents]\nmax-depth = 0\n", ProjectTrust::Untrusted);
    assert_eq!(config.subagents.max_depth, 0);
}

#[test]
fn a_config_file_lowers_the_children_limit() {
    let (config, _dir) = load(
        "",
        "[subagents]\nmax-children-per-parent = 2\n",
        ProjectTrust::Untrusted,
    );
    assert_eq!(config.subagents.max_children_per_parent, 2);
}

#[test]
fn a_config_file_lowers_the_live_limit() {
    let (config, _dir) = load(
        "",
        "[subagents]\nmax-live-total = 3\n",
        ProjectTrust::Untrusted,
    );
    assert_eq!(config.subagents.max_live_total, 3);
}

#[test]
fn a_config_file_lowers_the_child_timeout() {
    let (config, _dir) = load(
        "",
        "[subagents]\nchild-timeout-secs = 30\n",
        ProjectTrust::Untrusted,
    );
    assert_eq!(config.subagents.child_timeout, Duration::from_secs(30));
}

// ---- a project file cannot raise any limit. One test per limit ----

#[test]
fn a_project_file_cannot_raise_the_depth_limit() {
    let stated = SubagentLimits::new();
    let (config, _dir) = load("", "[subagents]\nmax-depth = 9\n", ProjectTrust::Untrusted);
    assert_eq!(
        config.subagents.max_depth, stated.max_depth,
        "a project file may lower a cap and never raise one"
    );
    assert!(
        config
            .lowered_limits
            .iter()
            .any(|line| line.contains("max-depth")),
        "and the refusal is named, not silent: {:?}",
        config.lowered_limits
    );
}

#[test]
fn a_project_file_cannot_raise_the_children_limit() {
    let stated = SubagentLimits::new();
    let (config, _dir) = load(
        "",
        "[subagents]\nmax-children-per-parent = 64\n",
        ProjectTrust::Untrusted,
    );
    assert_eq!(
        config.subagents.max_children_per_parent,
        stated.max_children_per_parent
    );
    assert!(
        config
            .lowered_limits
            .iter()
            .any(|line| line.contains("max-children-per-parent")),
        "the refusal is named: {:?}",
        config.lowered_limits
    );
}

#[test]
fn a_project_file_cannot_raise_the_live_limit() {
    let stated = SubagentLimits::new();
    let (config, _dir) = load(
        "",
        "[subagents]\nmax-live-total = 4096\n",
        ProjectTrust::Untrusted,
    );
    assert_eq!(config.subagents.max_live_total, stated.max_live_total);
    assert!(
        config
            .lowered_limits
            .iter()
            .any(|line| line.contains("max-live-total")),
        "the refusal is named: {:?}",
        config.lowered_limits
    );
}

#[test]
fn a_project_file_cannot_raise_the_child_timeout() {
    let stated = SubagentLimits::new();
    let (config, _dir) = load(
        "",
        "[subagents]\nchild-timeout-secs = 86400\n",
        ProjectTrust::Untrusted,
    );
    assert_eq!(
        config.subagents.child_timeout, stated.child_timeout,
        "a day-long child would hold a slot until the process dies"
    );
    assert!(
        config
            .lowered_limits
            .iter()
            .any(|line| line.contains("child-timeout-secs")),
        "the refusal is named: {:?}",
        config.lowered_limits
    );
}

// ---- the ceiling: who may raise ----

#[test]
fn a_global_file_may_raise_a_limit() {
    // A home directory is not a clone. The user's own file is their own choice.
    let (config, _dir) = load(
        "[subagents]\nmax-live-total = 64\n",
        "",
        ProjectTrust::Untrusted,
    );
    assert_eq!(config.subagents.max_live_total, 64);
    assert!(
        config.lowered_limits.is_empty(),
        "the user's own file is never refused: {:?}",
        config.lowered_limits
    );
}

#[test]
fn a_project_file_may_lower_a_limit_the_global_file_raised() {
    // The two rules compose. The ceiling is the global value, so a project file may sit
    // anywhere at or below it.
    let (config, _dir) = load(
        "[subagents]\nmax-live-total = 64\n",
        "[subagents]\nmax-live-total = 40\n",
        ProjectTrust::Untrusted,
    );
    assert_eq!(config.subagents.max_live_total, 40);
    assert!(config.lowered_limits.is_empty(), "40 is below 64");
}

#[test]
fn a_project_file_cannot_raise_a_limit_above_a_raised_ceiling() {
    // The ceiling is the global value, and not the built-in default. A project value above
    // it is still lowered, so the comparison is against the real ceiling.
    let (config, _dir) = load(
        "[subagents]\nmax-live-total = 64\n",
        "[subagents]\nmax-live-total = 999\n",
        ProjectTrust::Untrusted,
    );
    assert_eq!(config.subagents.max_live_total, 64);
}

#[test]
fn trust_does_not_let_a_project_file_raise_a_limit() {
    // `--trust-project` loads a capability, and a limit is not a capability a file adds. It
    // is a bound a file relaxes, and `D-your-settings-are-a-floor` keeps the two apart.
    let stated = SubagentLimits::new();
    let (config, _dir) = load(
        "",
        "[subagents]\nmax-live-total = 4096\n",
        ProjectTrust::Trusted,
    );
    assert_eq!(
        config.subagents.max_live_total, stated.max_live_total,
        "trust loads a capability; it does not lift the floor"
    );
}

// ---- the recursion into a profile ----

#[test]
fn a_project_profile_cannot_raise_a_limit() {
    // A project profile carried a powerful key past a gate that read only the top layer, and
    // a live probe proved it. See `docs/verification/profile-trust-bypass.md`. The limit rule
    // recurses for the same reason.
    let stated = SubagentLimits::new();
    let (config, _dir) = load_with_profile(
        "",
        "[profiles.fast.subagents]\nmax-live-total = 4096\n",
        "fast",
    );
    assert_eq!(
        config.subagents.max_live_total, stated.max_live_total,
        "a nested table must not walk around the floor"
    );
}

#[test]
fn a_project_profile_shadowing_a_global_profile_name_cannot_raise_a_limit() {
    // The likely vector, because the user types `--profile prod` from habit. `merge` ends
    // with `profiles.extend(over.profiles)`, so a project profile of the same name replaces
    // the user's own. The limit must still be bounded by the ceiling.
    let (config, _dir) = load_with_profile(
        "[subagents]\nmax-live-total = 10\n\
         [profiles.prod.subagents]\nmax-live-total = 10\n",
        "[profiles.prod.subagents]\nmax-live-total = 4096\n",
        "prod",
    );
    assert_eq!(
        config.subagents.max_live_total, 10,
        "a shadowed profile name must not become a way to raise a cap"
    );
}

#[test]
fn a_global_profile_may_raise_a_limit() {
    // The ceiling excludes the applied profile, so the user's own profile is still their own
    // choice. Without this the floor would refuse the user their own settings.
    let (config, _dir) =
        load_with_profile("[profiles.big.subagents]\nmax-live-total = 64\n", "", "big");
    assert_eq!(config.subagents.max_live_total, 64);
    assert!(
        config.lowered_limits.is_empty(),
        "the user's own profile is never refused: {:?}",
        config.lowered_limits
    );
}

// ---- the shape of the narrowing ----

#[test]
fn only_the_limit_a_file_raised_is_named() {
    // A notice must name the limit the file raised, and no other. A notice that names a limit
    // the user never set teaches the user to distrust every notice.
    //
    // **This test does not prove that an unset field stays unset.** Two versions tried, and a
    // deliberate break passed both: `build_subagents` fills every gap with the same default the
    // ceiling holds, so a pinned field and an unset field give the identical `Config` today.
    // `narrow_to_leaves_an_unset_field_unset`, a unit test in `crates/rho-config/src/lib.rs`,
    // is where that invariant is observable. A review found this, and the honest split is the
    // fix.
    //
    // The global file raises two limits, so the ceiling is not the built-in default and the
    // value assertions below can fail.
    let (config, _dir) = load(
        "[subagents]\nmax-children-per-parent = 9\nchild-timeout-secs = 900\n",
        "[subagents]\nmax-live-total = 4096\n",
        ProjectTrust::Untrusted,
    );
    assert_eq!(
        config.lowered_limits.len(),
        1,
        "only the limit the file raised may be named: {:?}",
        config.lowered_limits
    );
    // The project file states nothing about these two, so the global file's values must stand.
    // A narrowing that pinned the ceiling into the project layer would write 9 and 900 there
    // too, and the merge would then read them from the project layer instead. That is still 9
    // and 900 today, so the load is checked from the other side as well: the project layer
    // must remain silent about a field it never named.
    assert_eq!(config.subagents.max_children_per_parent, 9);
    assert_eq!(config.subagents.child_timeout, Duration::from_secs(900));
    assert_eq!(
        config.subagents.max_depth,
        SubagentLimits::new().max_depth,
        "a field no layer set keeps the built-in default"
    );
}

#[test]
fn the_refusal_report_holds_exactly_one_line_per_raised_limit() {
    // The report is what the notice prints, so a duplicate line or an extra line is a defect a
    // user sees. This asserts the whole vector, not its length, so an extra name fails it.
    let (config, _dir) = load_with_profile(
        "[subagents]\nmax-children-per-parent = 9\n",
        "[subagents]\nmax-live-total = 4096\n\
         [profiles.fast]\nmodel = \"some-model\"\n",
        "fast",
    );
    assert_eq!(
        config.lowered_limits,
        vec![format!(
            "subagents.max-live-total (from {})",
            config_project_path(&config)
        )],
        "exactly one limit asked for more, so exactly one is named"
    );
    assert_eq!(
        config.subagents.max_children_per_parent, 9,
        "the user's own raised value stands, because no project layer touched it"
    );
}

/// The project path a `lowered_limits` line names. It is read back out of the report, so the
/// assertion above does not have to thread the temp path through.
fn config_project_path(config: &Config) -> String {
    config
        .lowered_limits
        .first()
        .and_then(|line| line.split_once("(from "))
        .map(|(_, tail)| tail.trim_end_matches(')').to_string())
        .unwrap_or_default()
}

#[test]
fn a_refused_raise_is_named_in_the_config() {
    // The report holds the config key and the file that asked, so a notice can say both.
    let dir = temp_dir();
    let project = write_file(&dir, "project.toml", "[subagents]\nmax-live-total = 4096\n");
    let sources = Sources::from_paths(ConfigPaths {
        global: None,
        project: Some(project.clone()),
        ..Default::default()
    });
    let config = Config::load(&sources).expect("the file is valid TOML");
    let named = config.lowered_limits.join(", ");
    assert!(
        named.contains("subagents.max-live-total"),
        "the report names the config key: {named}"
    );
    assert!(
        named.contains(&project.display().to_string()),
        "and the file that asked: {named}"
    );
}

#[test]
fn loading_twice_gives_the_same_limits() {
    let dir = temp_dir();
    let global = write_file(&dir, "global.toml", "[subagents]\nmax-live-total = 20\n");
    let project = write_file(&dir, "project.toml", "[subagents]\nmax-live-total = 99\n");
    let build = || {
        Sources::from_paths(ConfigPaths {
            global: Some(global.clone()),
            project: Some(project.clone()),
            ..Default::default()
        })
    };
    let first = Config::load(&build()).expect("valid TOML");
    let second = Config::load(&build()).expect("valid TOML");
    assert_eq!(
        first.subagents.max_live_total,
        second.subagents.max_live_total
    );
    assert_eq!(
        first.subagents.max_live_total, 20,
        "the ceiling stands twice"
    );
    assert_eq!(first.lowered_limits, second.lowered_limits);
}

// ---- a limit too large is refused, not clamped, and never panics ----

#[test]
fn a_limit_above_the_runtime_maximum_is_refused_and_names_the_key() {
    // A config file panicked the binary. `max-live-total` reached `Semaphore::new` unclamped,
    // and tokio panics above `MAX_PERMITS`, so a typo aborted the process with a tokio
    // backtrace instead of a sentence. Driven live before the fix:
    //
    //   thread 'main' panicked at tokio-1.53.1/src/sync/batch_semaphore.rs:141:9:
    //   a semaphore may not have more than MAX_PERMITS permits (2305843009213693951)
    //
    // The floor rule does not help, because a ceiling bounds a project file and the user's own
    // global file is unbounded by design. See `D-a-limit-too-large-is-refused-not-clamped`.
    let dir = temp_dir();
    let global = write_file(
        &dir,
        "global.toml",
        "[subagents]\nmax-live-total = 18446744073709551615\n",
    );
    let sources = Sources::from_paths(ConfigPaths {
        global: Some(global),
        project: None,
        ..Default::default()
    });
    let error = Config::load(&sources).expect_err("a value the runtime cannot accept is refused");
    let message = error.to_string();
    assert!(
        message.contains("subagents.max-live-total"),
        "the message names the key: {message}"
    );
    assert!(
        message.contains(&SubagentLimits::MAX_COUNT.to_string()),
        "and it names the maximum: {message}"
    );
}

#[test]
fn a_children_limit_above_the_runtime_maximum_is_refused_too() {
    // The second semaphore count. One field is not the set.
    let dir = temp_dir();
    let global = write_file(
        &dir,
        "global.toml",
        "[subagents]\nmax-children-per-parent = 18446744073709551615\n",
    );
    let sources = Sources::from_paths(ConfigPaths {
        global: Some(global),
        project: None,
        ..Default::default()
    });
    let error = Config::load(&sources).expect_err("a value the runtime cannot accept is refused");
    assert!(
        error
            .to_string()
            .contains("subagents.max-children-per-parent"),
        "the message names the key: {error}"
    );
}

#[test]
fn a_limit_at_the_runtime_maximum_still_loads() {
    // The boundary. Refusing a value rho can accept would be its own defect.
    let dir = temp_dir();
    let global = write_file(
        &dir,
        "global.toml",
        &format!(
            "[subagents]\nmax-live-total = {}\n",
            SubagentLimits::MAX_COUNT
        ),
    );
    let sources = Sources::from_paths(ConfigPaths {
        global: Some(global),
        project: None,
        ..Default::default()
    });
    let config = Config::load(&sources).expect("a value at the maximum is accepted");
    assert_eq!(config.subagents.max_live_total, SubagentLimits::MAX_COUNT);
}

#[test]
fn a_refused_limit_is_not_silently_clamped() {
    // `D-your-settings-are-a-floor` argues against a value the user wrote and rho changed in
    // silence. So this is a refusal, and the load must fail rather than return a smaller value.
    let dir = temp_dir();
    let global = write_file(
        &dir,
        "global.toml",
        "[subagents]\nmax-live-total = 18446744073709551615\n",
    );
    let sources = Sources::from_paths(ConfigPaths {
        global: Some(global),
        project: None,
        ..Default::default()
    });
    assert!(
        Config::load(&sources).is_err(),
        "a clamp would return Ok with a value the user never wrote"
    );
}
