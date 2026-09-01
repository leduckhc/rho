//! Tests for the starred-models file: `starred::load` and `starred::save`. The file lives
//! at `~/.rho/starred-models.toml`. See `D-starred-models-live-in-their-own-file`.

use rho_tui::starred;

#[test]
fn starred_read_missing_file_returns_an_empty_list() {
    let tmp = tempfile::tempdir().expect("a tempdir opens");
    let path = tmp.path().join("starred-models.toml");
    let list = starred::load(&path).expect("a missing file is not an error");
    assert!(list.is_empty(), "the empty list came back: {list:?}");
}

#[test]
fn starred_read_and_write_round_trip() {
    let tmp = tempfile::tempdir().expect("a tempdir opens");
    let path = tmp.path().join("starred-models.toml");
    let ids = vec![
        "anthropic/claude-sonnet-4-6".to_string(),
        "amazon.nova-micro-v1:0".to_string(),
    ];
    starred::save(&path, &ids).expect("the write goes through");
    // A second save overwrites, so a shrink cannot silently keep an id.
    let smaller = vec!["only-one".to_string()];
    starred::save(&path, &smaller).expect("the second write goes through");
    let read_back = starred::load(&path).expect("the file parses");
    assert_eq!(read_back, smaller, "the second write is the truth on disk");
}

#[test]
fn starred_read_of_a_bad_file_returns_a_notice_and_an_empty_list() {
    let tmp = tempfile::tempdir().expect("a tempdir opens");
    let path = tmp.path().join("starred-models.toml");
    // TOML that does not deserialise into the expected shape.
    std::fs::write(&path, "not = valid toml either =\n").expect("the write goes through");
    let error = starred::load(&path).expect_err("a bad file is an error");
    assert!(
        error.contains("starred-models"),
        "the error names the module: {error}"
    );
    assert!(
        error.contains(&path.display().to_string()),
        "the error names the path: {error}"
    );
    // A file that reads but does not parse the expected schema. `default` on `starred`
    // means an empty `[starred]` table succeeds, so use something that fails deserialisation
    // outright.
    let path2 = tmp.path().join("also-bad.toml");
    std::fs::write(&path2, "starred = \"not-an-array\"\n").expect("the write goes through");
    let error2 = starred::load(&path2).expect_err("a wrong-shape file is an error");
    assert!(
        error2.contains("does not parse"),
        "the error says why: {error2}"
    );
}
