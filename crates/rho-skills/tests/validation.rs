//! Frontmatter validation from SPEC-skills section 4.

mod common;

use common::{config, write_skill};
use rho_skills::discover;
use tempfile::tempdir;

#[tokio::test]
async fn a_skill_without_a_description_does_not_load() {
    let user = tempdir().unwrap();
    write_skill(&user.path().join("no-desc"), "name: no-desc", "# body");

    let set = discover(&config(vec![user.path().to_path_buf()], None)).await;

    assert!(
        set.loaded.is_empty(),
        "a skill with no description does not load"
    );
    assert!(set.withheld.is_empty());
}

#[tokio::test]
async fn a_bad_name_warns_and_still_loads() {
    let user = tempdir().unwrap();
    write_skill(
        &user.path().join("bad"),
        "name: Bad--Name-\ndescription: Still loads.",
        "# body",
    );

    let set = discover(&config(vec![user.path().to_path_buf()], None)).await;

    assert_eq!(set.loaded.len(), 1, "a bad name still loads");
    assert!(
        !set.loaded[0].warnings.is_empty(),
        "a bad name raises a warning"
    );
}

#[tokio::test]
async fn an_over_long_description_warns_and_truncates() {
    let user = tempdir().unwrap();
    let long = "a".repeat(2000);
    write_skill(
        &user.path().join("long"),
        &format!("name: long\ndescription: {long}"),
        "# body",
    );

    let set = discover(&config(vec![user.path().to_path_buf()], None)).await;

    assert_eq!(set.loaded.len(), 1);
    assert_eq!(
        set.loaded[0].description.chars().count(),
        1024,
        "the description is truncated to the limit"
    );
    assert!(
        set.loaded[0]
            .warnings
            .iter()
            .any(|w| w.contains("truncated")),
        "truncation raises a warning"
    );
}

#[tokio::test]
async fn an_unknown_frontmatter_field_is_ignored() {
    let user = tempdir().unwrap();
    write_skill(
        &user.path().join("extra"),
        "name: extra\ndescription: Has an unknown field.\nsurprise: yes",
        "# body",
    );

    let set = discover(&config(vec![user.path().to_path_buf()], None)).await;

    assert_eq!(set.loaded.len(), 1, "an unknown field is ignored");
    assert!(set.loaded[0].warnings.is_empty());
}

#[tokio::test]
async fn a_skill_with_no_frontmatter_does_not_load_and_warns() {
    let user = tempdir().unwrap();
    let dir = user.path().join("plain");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("SKILL.md"), "# Just a heading, no frontmatter").unwrap();

    let set = discover(&config(vec![user.path().to_path_buf()], None)).await;

    assert!(set.loaded.is_empty(), "no frontmatter does not load");
    assert!(set.withheld.is_empty());
}

#[tokio::test]
async fn malformed_yaml_does_not_load_and_warns() {
    let user = tempdir().unwrap();
    write_skill(
        &user.path().join("broken"),
        "name: broken\ndescription: [unterminated",
        "# body",
    );

    let set = discover(&config(vec![user.path().to_path_buf()], None)).await;

    assert!(set.loaded.is_empty(), "malformed YAML does not load");
    assert!(set.withheld.is_empty());
}
