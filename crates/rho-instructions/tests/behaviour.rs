//! Failure paths, constructors, notices, and prefix stability.
//! SPEC-project-instructions section 7.

mod common;

use common::*;
use rho_instructions::{
    InstructionConfig, InstructionLimits, InstructionSet, OmissionReason, gather, home_dir,
    prompt_block, user_instruction_path,
};
use tempfile::TempDir;

fn tree() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let home = real(tmp.path()).join("home");
    let root = home.join("repo");
    std::fs::create_dir_all(&root).unwrap();
    (tmp, home, root)
}

// ---------------------------------------------------------------- failure paths

#[tokio::test]
async fn an_unreadable_file_records_an_omission() {
    use std::os::unix::fs::PermissionsExt;
    let (_tmp, home, root) = tree();
    let file = root.join("AGENTS.md");
    write_file(&file, "SECRET RULES");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();

    let set = gather(&config(&home, &root)).await;

    // Restore, so the temporary directory can be removed.
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

    assert!(set.delivered.is_empty());
    assert!(has_reason(&set, OmissionReason::Unreadable));
}

#[tokio::test]
async fn home_unavailable_records_an_omission() {
    let (_tmp, _home, root) = tree();
    let mut cfg = config(&root, &root);
    cfg.home = None;
    // An empty user_dir plus no home means rho cannot resolve either boundary.
    cfg.user_dir = None;

    // The test supplies no home, and it must not read the real one.
    let set = gather_without_home(&cfg).await;

    assert!(has_reason(&set, OmissionReason::HomeUnavailable));
}

#[tokio::test]
async fn home_unavailable_walks_zero_ancestors() {
    let (_tmp, home, root) = tree();
    let deeper = root.join("inner");
    std::fs::create_dir_all(&deeper).unwrap();
    write_instructions(&home, "ABOVE");
    write_instructions(&root, "MIDDLE");
    write_instructions(&deeper, "HERE");

    let mut cfg = config(&home, &deeper);
    cfg.home = None;
    let set = gather_without_home(&cfg).await;

    // The ancestor cap must never stand in for the home boundary.
    assert!(
        !bodies(&set).contains(&"MIDDLE".to_string()),
        "no ancestor may be read with an unknown boundary: {:?}",
        bodies(&set)
    );
    assert!(
        !bodies(&set).contains(&"ABOVE".to_string()),
        "no ancestor may be read with an unknown boundary: {:?}",
        bodies(&set)
    );
    assert!(has_reason(&set, OmissionReason::HomeUnavailable));
}

/// Run a gather with `HOME` and `USERPROFILE` cleared, serialised against other tests
/// that touch the environment.
///
/// The restore runs from `Drop`, so a panicking gather still puts the environment back. A
/// poisoned lock is recovered rather than propagated, because a poisoned lock would hide
/// the real failure behind a `PoisonError` in every later test.
async fn gather_without_home(cfg: &InstructionConfig) -> InstructionSet {
    let _guard = ClearedHome::new();
    gather(cfg).await
}

/// Clears the home environment variables, and restores them on drop.
struct ClearedHome {
    _lock: std::sync::MutexGuard<'static, ()>,
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl ClearedHome {
    fn new() -> Self {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        let lock = LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let saved: Vec<(&'static str, Option<std::ffi::OsString>)> = ["HOME", "USERPROFILE"]
            .into_iter()
            .map(|key| (key, std::env::var_os(key)))
            .collect();
        for (key, _) in &saved {
            unsafe { std::env::remove_var(key) };
        }
        Self { _lock: lock, saved }
    }
}

impl Drop for ClearedHome {
    fn drop(&mut self) {
        for (key, value) in &self.saved {
            match value {
                Some(v) => unsafe { std::env::set_var(key, v) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
    }
}

#[tokio::test]
async fn no_session_root_records_an_omission() {
    let (_tmp, home, _root) = tree();
    let mut cfg = config(&home, &home);
    cfg.session_root = None;

    let set = gather(&cfg).await;

    assert!(
        has_reason(&set, OmissionReason::NoSessionRoot),
        "a missing root must never look like a repository with no file: {:?}",
        set.omissions
    );
    assert!(
        !set.is_empty(),
        "the caller must be able to see the problem"
    );
}

#[tokio::test]
async fn gather_is_deterministic() {
    let (_tmp, home, root) = tree();
    write_instructions(&home.join("repo"), "ROOT");
    write_file(&home.join(".config").join("rho").join("AGENTS.md"), "USER");

    let first = gather(&config(&home, &root)).await;
    let second = gather(&config(&home, &root)).await;

    assert_eq!(first, second);
}

// ------------------------------------------------------- constructors and paths

#[tokio::test]
async fn for_session_root_reads_the_user_file() {
    let (_tmp, home, root) = tree();
    write_file(&home.join(".config").join("rho").join("AGENTS.md"), "USER");

    // `user_dir: None` must resolve to `<home>/.config/rho`, never mean "skip it".
    let mut cfg = InstructionConfig::for_session_root(&root);
    cfg.home = Some(home.clone());

    let set = gather(&cfg).await;

    assert_eq!(bodies(&set), vec!["USER".to_string()]);
}

#[tokio::test]
async fn for_session_root_bounds_the_walk_at_home() {
    let cfg = InstructionConfig::for_session_root("/some/root");

    assert!(cfg.discover);
    assert_eq!(cfg.filenames, vec!["AGENTS.md".to_string()]);
    assert!(
        cfg.home.is_none(),
        "None means read the environment; see the field documentation"
    );
    // The environment is what `None` resolves to, and this host has a home.
    assert!(home_dir().is_some(), "the test host sets HOME");
}

#[test]
fn user_instruction_path_is_under_config_rho() {
    let path = user_instruction_path(std::path::Path::new("/home/u"), "AGENTS.md");

    assert_eq!(path, std::path::Path::new("/home/u/.config/rho/AGENTS.md"));
}

// --------------------------------------------------------------------- notices

#[tokio::test]
async fn notices_names_every_omission() {
    let (_tmp, home, root) = tree();
    let target = home.join("t.md");
    write_file(&target, "X");
    std::os::unix::fs::symlink(&target, root.join("AGENTS.md")).unwrap();

    let set = gather(&config(&home, &root)).await;
    let notices = set.notices();

    assert_eq!(notices.len(), set.omissions.len());
    assert!(
        notices[0].contains("symlink"),
        "a notice must name the reason: {:?}",
        notices
    );
    assert!(
        notices[0].contains("AGENTS.md"),
        "a notice must name the source: {:?}",
        notices
    );
}

#[tokio::test]
async fn notices_is_empty_for_a_clean_gather() {
    let (_tmp, home, root) = tree();
    write_instructions(&root, "ROOT");

    let set = gather(&config(&home, &root)).await;

    assert!(set.notices().is_empty());
}

#[tokio::test]
async fn notices_never_reach_the_model() {
    let (_tmp, home, root) = tree();
    let target = home.join("t.md");
    write_file(&target, "X");
    std::os::unix::fs::symlink(&target, root.join("AGENTS.md")).unwrap();
    write_instructions(&home.join("repo2"), "OTHER");

    let set = gather(&config(&home, &root)).await;
    let block = prompt_block(&set);

    assert!(
        !block.contains("skipped"),
        "an omission is the user's problem, not the model's; block was:\n{block}"
    );
    assert!(!block.contains("symlink"), "block was:\n{block}");
}

// ------------------------------------------------------------ prefix stability

#[tokio::test]
async fn prompt_block_is_byte_identical_across_calls() {
    let (_tmp, home, root) = tree();
    write_instructions(&root, "ROOT");
    write_file(&home.join(".config").join("rho").join("AGENTS.md"), "USER");

    let set = gather(&config(&home, &root)).await;

    assert_eq!(prompt_block(&set), prompt_block(&set));
}

#[test]
fn prompt_block_is_empty_for_an_empty_set() {
    assert_eq!(prompt_block(&InstructionSet::default()), "");
}

// ---------------------------------------------------------- pinned invariants

/// The spec states these defaults in section 4. A silent change to one of them changes
/// every request rho sends, and no other test would notice.
#[test]
fn default_limits_match_the_spec() {
    let limits = InstructionLimits::default();

    assert_eq!(limits.instruction_file_bytes, 64 * 1024);
    assert_eq!(limits.instructions_total_bytes, 128 * 1024);
    assert_eq!(limits.instruction_ancestor_cap, 32);
    assert_eq!(rho_instructions::MAX_OMISSION_RECORDS, 32);
}

/// Every reason must explain itself to the user. A new variant with an empty label would
/// print a notice that names no cause, which is the silence D-rho-reads-agents-md forbids.
#[test]
fn every_omission_reason_has_a_label() {
    use OmissionReason::*;
    // Listing the variants makes this fail to compile when a variant is added, so nobody
    // can add one without deciding what it says.
    let all = [
        HomeUnavailable,
        RootOutsideHome,
        UnsafePath,
        Unreadable,
        Symlink,
        NonRegular,
        TotalBudget,
        AncestorCap,
        NoSessionRoot,
    ];
    for reason in all {
        assert!(!reason.label().is_empty(), "{reason:?} has no label");
        let set = InstructionSet {
            delivered: Vec::new(),
            omissions: vec![rho_instructions::Omission {
                source: "x".to_string(),
                reason,
            }],
        };
        assert_eq!(set.notices().len(), 1, "{reason:?} produced no notice");
    }
}

#[test]
fn is_empty_separates_a_clean_gather_from_a_refusal() {
    let clean = InstructionSet::default();
    assert!(clean.is_empty());

    let refused = InstructionSet {
        delivered: Vec::new(),
        omissions: vec![rho_instructions::Omission {
            source: "x".to_string(),
            reason: OmissionReason::Symlink,
        }],
    };
    assert!(
        !refused.is_empty(),
        "a refusal is not an empty repository, and a caller must be able to tell them apart"
    );
}

#[test]
fn home_dir_reads_the_environment() {
    // The host running this test has a home directory. The point is that the function reads
    // the environment rather than guessing a path.
    let home = home_dir().expect("the test host sets HOME");
    assert!(home.is_absolute(), "home was {home:?}");
}
