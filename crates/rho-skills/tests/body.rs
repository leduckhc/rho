//! Loading a skill body on demand, from SPEC-08 section 6.

mod common;

use common::write_skill;
use rho_skills::{discover, load_body};
use tempfile::tempdir;

use rho_skills::SkillConfig;

/// A config that scans one user directory.
fn user_config(dir: &std::path::Path) -> SkillConfig {
    SkillConfig {
        user_dirs: vec![dir.to_path_buf()],
        session_root: None,
        explicit: Vec::new(),
        project_trusted: false,
        discover: true,
    }
}

#[tokio::test]
async fn load_body_returns_the_text_after_the_frontmatter() {
    let user = tempdir().unwrap();
    write_skill(
        &user.path().join("with-body"),
        "name: with-body\ndescription: Has a body.",
        "# Heading\n\nThe body text.",
    );

    let set = discover(&user_config(user.path())).await;
    let body = load_body(&set.loaded[0]).await.unwrap();

    assert_eq!(body, "# Heading\n\nThe body text.");
    assert!(!body.contains("name: with-body"), "the frontmatter is gone");
}

#[tokio::test]
async fn load_body_on_a_deleted_skill_is_an_error() {
    let user = tempdir().unwrap();
    write_skill(
        &user.path().join("doomed"),
        "name: doomed\ndescription: About to be removed.",
        "# body",
    );

    let set = discover(&user_config(user.path())).await;
    let skill = set.loaded[0].clone();

    std::fs::remove_file(&skill.path).unwrap();

    assert!(
        load_body(&skill).await.is_err(),
        "a deleted skill is an error"
    );
}
