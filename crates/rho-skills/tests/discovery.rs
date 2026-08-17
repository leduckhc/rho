//! Discovery rules from SPEC-08 section 2.

mod common;

use common::{config, write_file, write_skill};
use rho_skills::discover;
use tempfile::tempdir;

#[tokio::test]
async fn discovers_a_directory_skill_with_frontmatter() {
    let user = tempdir().unwrap();
    write_skill(
        &user.path().join("pdf-tools"),
        "name: pdf-tools\ndescription: Works with PDF files.",
        "# body",
    );

    let set = discover(&config(vec![user.path().to_path_buf()], None)).await;

    assert_eq!(set.loaded.len(), 1);
    assert_eq!(set.loaded[0].name, "pdf-tools");
    assert_eq!(set.loaded[0].description, "Works with PDF files.");
}

#[tokio::test]
async fn discovers_a_bare_md_file_in_the_rho_dir() {
    let base = tempdir().unwrap();
    let rho_dir = base.path().join(".rho").join("skills");
    write_file(
        &rho_dir.join("quick.md"),
        "---\nname: quick\ndescription: A one-file skill.\n---\n# body",
    );

    let set = discover(&config(vec![rho_dir], None)).await;

    assert_eq!(set.loaded.len(), 1);
    assert_eq!(set.loaded[0].name, "quick");
}

#[tokio::test]
async fn ignores_a_bare_md_file_in_the_agents_dir() {
    let base = tempdir().unwrap();
    let agents_dir = base.path().join(".agents").join("skills");
    write_file(
        &agents_dir.join("README.md"),
        "---\nname: readme\ndescription: Not a skill here.\n---\n# body",
    );

    let set = discover(&config(vec![agents_dir], None)).await;

    assert!(
        set.loaded.is_empty(),
        "a bare md in an agents dir is ignored"
    );
}

#[tokio::test]
async fn discovers_recursively() {
    let user = tempdir().unwrap();
    write_skill(
        &user.path().join("group").join("nested"),
        "name: nested\ndescription: A deep skill.",
        "# body",
    );

    let set = discover(&config(vec![user.path().to_path_buf()], None)).await;

    assert_eq!(set.loaded.len(), 1);
    assert_eq!(set.loaded[0].name, "nested");
}

#[tokio::test]
async fn a_name_collision_keeps_the_first_and_warns() {
    let first = tempdir().unwrap();
    let second = tempdir().unwrap();
    write_skill(
        &first.path().join("dup"),
        "name: same\ndescription: The first one.",
        "# first",
    );
    write_skill(
        &second.path().join("dup"),
        "name: same\ndescription: The second one.",
        "# second",
    );

    let set = discover(&config(
        vec![first.path().to_path_buf(), second.path().to_path_buf()],
        None,
    ))
    .await;

    assert_eq!(set.loaded.len(), 1, "only the first skill is kept");
    assert_eq!(set.loaded[0].description, "The first one.");
    assert!(
        set.loaded[0].warnings.iter().any(|w| w.contains("same")),
        "the kept skill warns about the collision"
    );
}

#[tokio::test]
async fn no_discover_flag_still_loads_an_explicit_path() {
    let user = tempdir().unwrap();
    let skill_dir = user.path().join("explicit-one");
    write_skill(
        &skill_dir,
        "name: explicit-one\ndescription: Loaded by an explicit path.",
        "# body",
    );

    let mut cfg = config(vec![user.path().to_path_buf()], None);
    cfg.discover = false;
    cfg.explicit = vec![skill_dir];

    let set = discover(&cfg).await;

    assert_eq!(set.loaded.len(), 1);
    assert_eq!(set.loaded[0].name, "explicit-one");
}
