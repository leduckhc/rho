//! Byte and count bounds. SPEC-project-instructions section 4 and section 7.

mod common;

use common::*;
use rho_instructions::{MAX_OMISSION_RECORDS, OmissionReason, gather, prompt_block};
use tempfile::TempDir;

fn tree() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let home = real(tmp.path()).join("home");
    let root = home.join("repo");
    std::fs::create_dir_all(&root).unwrap();
    (tmp, home, root)
}

#[tokio::test]
async fn truncates_one_file_at_its_budget() {
    let (_tmp, home, root) = tree();
    write_instructions(&root, &"a".repeat(5000));
    let mut cfg = config(&home, &root);
    cfg.limits.instruction_file_bytes = 1000;

    let set = gather(&cfg).await;

    let kept = &set.delivered[0];
    assert!(
        kept.body.len() <= 1000,
        "kept {} bytes, budget was 1000",
        kept.body.len()
    );
    assert!(kept.truncated);
    assert_eq!(
        kept.observed_bytes, 5000,
        "the whole size is still reported"
    );
}

#[tokio::test]
async fn truncation_keeps_valid_utf8() {
    let (_tmp, home, root) = tree();
    // Each 'é' is two bytes, so an odd budget lands inside a character.
    write_instructions(&root, &"é".repeat(500));
    let mut cfg = config(&home, &root);
    cfg.limits.instruction_file_bytes = 101;

    let set = gather(&cfg).await;

    let body = &set.delivered[0].body;
    assert!(body.len() <= 101);
    assert!(
        body.chars().all(|c| c == 'é'),
        "a cut inside a character would corrupt the body"
    );
    // `String` cannot hold invalid UTF-8, so a byte cut must land on a boundary.
    assert_eq!(body.len() % 2, 0, "the cut landed inside a character");
}

#[tokio::test]
async fn renders_a_truncation_marker() {
    let (_tmp, home, root) = tree();
    write_instructions(&root, &"a".repeat(5000));
    let mut cfg = config(&home, &root);
    cfg.limits.instruction_file_bytes = 1000;

    let block = prompt_block(&gather(&cfg).await);

    assert!(
        block.contains("<instruction-truncated"),
        "the model must be told the file is partial; block was:\n{block}"
    );
    assert!(
        block.contains("observed_bytes=\"5000\""),
        "block was:\n{block}"
    );
    assert!(block.contains("kept_bytes="), "block was:\n{block}");
}

#[tokio::test]
async fn drops_a_file_over_the_total_budget() {
    let (_tmp, home, outer) = tree();
    let inner = outer.join("inner");
    std::fs::create_dir_all(&inner).unwrap();
    // The broad file is read first, so it fits. The narrow one overflows the total.
    write_instructions(&outer, &"n".repeat(400));
    write_instructions(&inner, &"i".repeat(400));

    let mut cfg = config(&home, &inner);
    cfg.limits.instructions_total_bytes = 500;

    let set = gather(&cfg).await;

    assert!(has_reason(&set, OmissionReason::TotalBudget));
    assert_eq!(
        bodies(&set),
        vec!["n".repeat(400)],
        "the file already read is kept, and the one that overflows is dropped"
    );
}

#[tokio::test]
async fn caps_the_omission_record_count() {
    let tmp = TempDir::new().unwrap();
    let home = real(tmp.path()).join("home");
    let mut root = home.clone();
    // Build a deep tree where every level holds a refused symlink.
    for i in 0..40 {
        root = root.join(format!("d{i}"));
    }
    std::fs::create_dir_all(&root).unwrap();
    let target = home.join("t.md");
    write_file(&target, "X");
    let mut dir = home.clone();
    for i in 0..40 {
        dir = dir.join(format!("d{i}"));
        std::os::unix::fs::symlink(&target, dir.join("AGENTS.md")).unwrap();
    }

    let mut cfg = config(&home, &root);
    cfg.limits.instruction_ancestor_cap = 64;

    let set = gather(&cfg).await;

    assert!(
        set.omissions.len() <= MAX_OMISSION_RECORDS,
        "kept {} records, cap is {MAX_OMISSION_RECORDS}",
        set.omissions.len()
    );
}
