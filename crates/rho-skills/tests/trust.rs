//! The security core from SPEC-08 section 3. Trust, symlinks, and sanitisation.

mod common;

use common::{config, write_skill};
use rho_skills::{SkillOrigin, discover};
use tempfile::tempdir;

/// Build a project skill under `.rho/skills` in the session root.
fn write_project_skill(root: &std::path::Path, name: &str) {
    write_skill(
        &root.join(".rho").join("skills").join(name),
        &format!("name: {name}\ndescription: A project skill."),
        "# body",
    );
}

#[tokio::test]
async fn a_project_skill_is_withheld_when_the_project_is_not_trusted() {
    let root = tempdir().unwrap();
    write_project_skill(root.path(), "proj");

    let mut cfg = config(vec![], Some(root.path()));
    cfg.project_trusted = false;

    let set = discover(&cfg).await;

    assert!(
        set.loaded.is_empty(),
        "an untrusted project skill does not load"
    );
    assert_eq!(set.withheld.len(), 1);
    assert_eq!(set.withheld[0].origin, SkillOrigin::Project);
}

#[tokio::test]
async fn a_project_skill_loads_when_the_project_is_trusted() {
    let root = tempdir().unwrap();
    write_project_skill(root.path(), "proj");

    let mut cfg = config(vec![], Some(root.path()));
    cfg.project_trusted = true;

    let set = discover(&cfg).await;

    assert_eq!(set.loaded.len(), 1, "a trusted project skill loads");
    assert!(set.withheld.is_empty());
    assert_eq!(set.loaded[0].origin, SkillOrigin::Project);
}

#[tokio::test]
async fn a_withheld_skill_is_still_listed_so_the_user_can_decide() {
    let root = tempdir().unwrap();
    write_project_skill(root.path(), "proj");

    let set = discover(&config(vec![], Some(root.path()))).await;

    assert_eq!(set.withheld.len(), 1, "a withheld skill is still listed");
    assert_eq!(set.withheld[0].name, "proj");
}

#[tokio::test]
async fn a_user_skill_loads_without_project_trust() {
    let user = tempdir().unwrap();
    let root = tempdir().unwrap();
    write_skill(
        &user.path().join("user-skill"),
        "name: user-skill\ndescription: A user skill.",
        "# body",
    );

    let mut cfg = config(vec![user.path().to_path_buf()], Some(root.path()));
    cfg.project_trusted = false;

    let set = discover(&cfg).await;

    assert_eq!(set.loaded.len(), 1, "a user skill needs no project trust");
    assert_eq!(set.loaded[0].origin, SkillOrigin::User);
}

#[tokio::test]
async fn an_explicit_path_inside_the_session_root_is_trusted_because_the_user_named_it() {
    let root = tempdir().unwrap();
    let skill_dir = root.path().join("inside");
    write_skill(
        &skill_dir,
        "name: inside\ndescription: The user named this path.",
        "# body",
    );

    let mut cfg = config(vec![], Some(root.path()));
    cfg.explicit = vec![skill_dir];

    let set = discover(&cfg).await;

    assert_eq!(set.loaded.len(), 1, "an explicit path is trusted");
    assert_eq!(set.loaded[0].origin, SkillOrigin::User);
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_from_a_user_dir_into_the_session_root_is_treated_as_project() {
    // A trusted user directory must not become a hole into the repository. A
    // link that resolves inside the session root is a project skill.
    let user = tempdir().unwrap();
    let root = tempdir().unwrap();

    // The real skill lives inside the session root.
    let real = root.path().join("secret-skill");
    write_skill(
        &real,
        "name: secret\ndescription: A skill inside the repository.",
        "# body",
    );

    // A link in the user directory points at it.
    std::os::unix::fs::symlink(&real, user.path().join("link")).unwrap();

    let mut cfg = config(vec![user.path().to_path_buf()], Some(root.path()));
    cfg.project_trusted = false;

    let set = discover(&cfg).await;

    assert!(
        set.loaded.is_empty(),
        "a symlink into the repository must not load as trusted"
    );
    assert_eq!(set.withheld.len(), 1);
    assert_eq!(set.withheld[0].origin, SkillOrigin::Project);
}

#[tokio::test]
async fn allowed_tools_is_ignored_and_warns() {
    let user = tempdir().unwrap();
    write_skill(
        &user.path().join("tooled"),
        "name: tooled\ndescription: Tries to pre-approve tools.\nallowed-tools: bash read",
        "# body",
    );

    let set = discover(&config(vec![user.path().to_path_buf()], None)).await;

    assert_eq!(set.loaded.len(), 1, "the skill still loads");
    assert!(
        set.loaded[0]
            .warnings
            .iter()
            .any(|w| w.contains("allowed-tools")),
        "the ignored field raises a warning"
    );
}

#[tokio::test]
async fn a_skill_name_with_a_control_sequence_is_sanitised() {
    let user = tempdir().unwrap();
    write_skill(
        &user.path().join("evil"),
        "name: \"danger\\u0007bell\"\ndescription: Carries a control byte.",
        "# body",
    );

    let set = discover(&config(vec![user.path().to_path_buf()], None)).await;

    assert_eq!(set.loaded.len(), 1);
    assert!(
        !set.loaded[0].name.chars().any(|c| c.is_control()),
        "the name holds no control character"
    );
}
