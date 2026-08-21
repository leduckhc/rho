//! Discovery and order. SPEC-project-instructions section 2 and section 7.

mod common;

use common::*;
use rho_instructions::{InstructionOrigin, OmissionReason, gather};
use tempfile::TempDir;

/// A tree with a home, and a session root two directories below it.
fn tree() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let home = real(tmp.path()).join("home");
    let root = home.join("work").join("repo");
    std::fs::create_dir_all(&root).unwrap();
    (tmp, home, root)
}

#[tokio::test]
async fn gathers_the_root_file() {
    let (_tmp, home, root) = tree();
    write_instructions(&root, "ROOT RULES");

    let set = gather(&config(&home, &root)).await;

    assert_eq!(bodies(&set), vec!["ROOT RULES".to_string()]);
    assert_eq!(set.delivered[0].origin, InstructionOrigin::Project);
}

#[tokio::test]
async fn gathers_nothing_when_no_file_exists() {
    let (_tmp, home, root) = tree();

    let set = gather(&config(&home, &root)).await;

    assert!(set.delivered.is_empty(), "delivered: {:?}", set.delivered);
    assert!(set.omissions.is_empty(), "omissions: {:?}", set.omissions);
    assert!(set.is_empty());
}

#[tokio::test]
async fn walks_ancestors_up_to_home() {
    let (_tmp, home, root) = tree();
    write_instructions(home.join("work").as_path(), "MID");
    write_instructions(&root, "ROOT");

    let set = gather(&config(&home, &root)).await;

    assert_eq!(bodies(&set), vec!["MID".to_string(), "ROOT".to_string()]);
}

#[tokio::test]
async fn stops_at_home() {
    let (_tmp, home, root) = tree();
    // The home directory itself must never be read as a project file. The user file at
    // `<home>/.config/rho/AGENTS.md` is the only home source.
    write_instructions(&home, "HOME FILE");
    write_instructions(&root, "ROOT");

    let set = gather(&config(&home, &root)).await;

    assert_eq!(bodies(&set), vec!["ROOT".to_string()]);
}

#[tokio::test]
async fn orders_broad_to_narrow() {
    let (_tmp, home, root) = tree();
    write_instructions(home.join("work").as_path(), "MID");
    write_instructions(&root, "ROOT");

    let set = gather(&config(&home, &root)).await;

    let last = set.delivered.last().unwrap();
    assert_eq!(
        last.body, "ROOT",
        "the narrowest file must be last, so it wins"
    );
}

#[tokio::test]
async fn user_file_comes_first() {
    let (_tmp, home, root) = tree();
    write_file(
        &home.join(".config").join("rho").join("AGENTS.md"),
        "USER RULES",
    );
    write_instructions(&root, "ROOT");

    let set = gather(&config(&home, &root)).await;

    assert_eq!(
        bodies(&set),
        vec!["USER RULES".to_string(), "ROOT".to_string()]
    );
    assert_eq!(set.delivered[0].origin, InstructionOrigin::User);
}

#[tokio::test]
async fn one_directory_yields_one_file() {
    let (_tmp, home, root) = tree();
    write_file(&root.join("AGENTS.md"), "FIRST");
    write_file(&root.join("CLAUDE.md"), "SECOND");
    let mut cfg = config(&home, &root);
    cfg.filenames = vec!["AGENTS.md".to_string(), "CLAUDE.md".to_string()];

    let set = gather(&cfg).await;

    assert_eq!(bodies(&set), vec!["FIRST".to_string()]);
}

#[tokio::test]
async fn root_outside_home_records_an_omission() {
    let tmp = TempDir::new().unwrap();
    let home = real(tmp.path()).join("home");
    let root = real(tmp.path()).join("elsewhere");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    write_instructions(&root, "OUTSIDE");

    let set = gather(&config(&home, &root)).await;

    assert!(has_reason(&set, OmissionReason::RootOutsideHome));
    // The spec refuses the ancestor walk, not the root's own file. A user who opens a
    // directory outside home still chose that directory, and its file is the narrowest
    // one there is. Dropping it would rebuild the defect this feature exists to fix.
    assert_eq!(bodies(&set), vec!["OUTSIDE".to_string()]);
}

#[tokio::test]
async fn discover_false_keeps_the_user_file() {
    let (_tmp, home, root) = tree();
    write_file(
        &home.join(".config").join("rho").join("AGENTS.md"),
        "USER RULES",
    );
    write_instructions(&root, "ROOT");
    let mut cfg = config(&home, &root);
    cfg.discover = false;

    let set = gather(&cfg).await;

    assert_eq!(bodies(&set), vec!["USER RULES".to_string()]);
}

#[tokio::test]
async fn ancestor_cap_stops_the_walk() {
    let tmp = TempDir::new().unwrap();
    let home = real(tmp.path()).join("home");
    let mut root = home.clone();
    for i in 0..6 {
        root = root.join(format!("d{i}"));
    }
    std::fs::create_dir_all(&root).unwrap();
    write_instructions(&home.join("d0"), "TOP");
    write_instructions(&root, "DEEP");

    let mut cfg = config(&home, &root);
    cfg.limits.instruction_ancestor_cap = 2;

    let set = gather(&cfg).await;

    assert!(has_reason(&set, OmissionReason::AncestorCap));
    assert!(
        !bodies(&set).contains(&"TOP".to_string()),
        "the cap must stop the walk before the top file"
    );
    assert!(
        bodies(&set).contains(&"DEEP".to_string()),
        "the narrowest file is always read first, so the cap never drops it"
    );
}

/// The notice for a walk refusal must not name the session root file.
///
/// A live run printed "skipped <root> because the session root is not below the home
/// directory" while rho had in fact read that file. A notice that contradicts the behaviour
/// is worse than no notice, because the user acts on it.
#[tokio::test]
async fn a_walk_refusal_never_claims_the_root_file_was_skipped() {
    let tmp = TempDir::new().unwrap();
    let home = real(tmp.path()).join("home");
    let root = real(tmp.path()).join("elsewhere");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    write_instructions(&root, "OUTSIDE");

    let set = gather(&config(&home, &root)).await;
    let notices = set.notices();

    assert_eq!(
        bodies(&set),
        vec!["OUTSIDE".to_string()],
        "the root file is read"
    );
    assert_eq!(notices.len(), 1);
    assert!(
        !notices[0].contains("AGENTS.md"),
        "the notice must not name the file rho read: {}",
        notices[0]
    );
    assert!(
        notices[0].contains("directories above the session root"),
        "the notice must name the walk: {}",
        notices[0]
    );
}
