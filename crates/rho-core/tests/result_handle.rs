//! Handle validation, resume isolation, and the pinned defaults.
//! SPEC-tool-result-handle section 8.
//!
//! Every case here is a refusal or a boundary. A handle is a string the model copies, so it is
//! untrusted input and must never become an arbitrary path.

use rho_core::{
    FileResultStore, HeadPreview, ResultLimits, ResultPolicy, ResultPreview, ResultStore,
    ResultStoreError, is_valid_handle,
};
use tempfile::TempDir;

// ------------------------------------------------------------------ handle shape

#[tokio::test]
async fn a_handle_with_a_separator_is_refused() {
    let dir = TempDir::new().unwrap();
    let store = FileResultStore::open(dir.path()).await.unwrap();

    let error = store
        .read_range("tr-0123456789abcdef-000001/../../etc/passwd", 0, 64)
        .await
        .expect_err("a separator must be refused");

    assert!(
        matches!(error, ResultStoreError::MalformedHandle(_)),
        "a path must never be built from this handle, got {error:?}"
    );
}

#[tokio::test]
async fn a_handle_with_a_parent_reference_is_refused() {
    let dir = TempDir::new().unwrap();
    let store = FileResultStore::open(dir.path()).await.unwrap();

    for handle in ["tr-..0123456789abcd-000001", "tr-0123456789abcdef-..0001"] {
        let error = store
            .read_range(handle, 0, 64)
            .await
            .expect_err("a parent reference must be refused");
        assert!(
            matches!(error, ResultStoreError::MalformedHandle(_)),
            "{handle} was not refused: {error:?}"
        );
    }
}

#[test]
fn is_valid_handle_accepts_only_the_documented_shape() {
    // The shape is `tr-<16 lowercase hex>-<at least 6 digits>`.
    assert!(is_valid_handle("tr-0123456789abcdef-000001"));
    assert!(is_valid_handle("tr-ffffffffffffffff-1234567"));

    assert!(!is_valid_handle(""), "empty");
    assert!(!is_valid_handle("000001"), "bare number");
    assert!(!is_valid_handle("tr-000001"), "no nonce");
    assert!(
        !is_valid_handle("tr-0123456789ABCDEF-000001"),
        "uppercase nonce"
    );
    assert!(!is_valid_handle("tr-0123456789abcde-000001"), "short nonce");
    assert!(
        !is_valid_handle("tr-0123456789abcdef0-000001"),
        "long nonce"
    );
    assert!(
        !is_valid_handle("tr-0123456789abcdef-00001"),
        "short sequence"
    );
    assert!(
        !is_valid_handle("tr-0123456789abcdef-00000a"),
        "sequence letters"
    );
    assert!(
        !is_valid_handle("tr-0123456789abcdef-000001.result"),
        "suffix"
    );
    assert!(
        !is_valid_handle("../tr-0123456789abcdef-000001"),
        "prefix path"
    );
}

/// The check must run before any path is built. A store whose directory is gone still refuses a
/// malformed handle, which proves the order.
#[tokio::test]
async fn a_malformed_handle_never_reaches_the_filesystem() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_path_buf();
    let store = FileResultStore::open(&path).await.unwrap();
    std::fs::remove_dir_all(&path).unwrap();

    let error = store
        .read_range("not-a-handle", 0, 64)
        .await
        .expect_err("a malformed handle is refused");

    assert!(
        matches!(error, ResultStoreError::MalformedHandle(_)),
        "the shape check must run before any filesystem call, got {error:?}"
    );
}

// --------------------------------------------------------------- resume isolation

/// A resumed session must not be able to enumerate an earlier run's handles.
///
/// An earlier draft of the spec used a bare sequence number and argued guessing was harmless,
/// because the model had seen a preview of every result in its session. A review broke that on
/// resume: the resumed model never saw the earlier run's previews. The nonce is the fix.
#[tokio::test]
async fn a_reopened_store_cannot_read_an_earlier_handle() {
    let dir = TempDir::new().unwrap();

    let first = FileResultStore::open(dir.path()).await.unwrap();
    let old_handle = first.put("SECRET FROM THE EARLIER RUN").await.unwrap();

    let second = FileResultStore::open(dir.path()).await.unwrap();

    assert_ne!(
        first.nonce(),
        second.nonce(),
        "each open takes a fresh nonce"
    );
    let error = second
        .read_range(&old_handle, 0, 1024)
        .await
        .expect_err("the earlier run's handle must be out of reach");
    assert!(matches!(error, ResultStoreError::NotFound(_)), "{error:?}");
}

/// The earlier run's evidence must survive. A new run writes under a new nonce, so it cannot
/// overwrite a file the earlier run made.
#[tokio::test]
async fn a_reopened_store_does_not_clobber() {
    let dir = TempDir::new().unwrap();

    let first = FileResultStore::open(dir.path()).await.unwrap();
    let old_handle = first.put("EARLIER PAYLOAD").await.unwrap();

    let second = FileResultStore::open(dir.path()).await.unwrap();
    let new_handle = second.put("LATER PAYLOAD").await.unwrap();

    assert_ne!(old_handle, new_handle);
    // The first store can still read its own result, so nothing was overwritten.
    assert_eq!(
        first.read_range(&old_handle, 0, 1024).await.unwrap().text,
        "EARLIER PAYLOAD"
    );
    assert_eq!(
        second.read_range(&new_handle, 0, 1024).await.unwrap().text,
        "LATER PAYLOAD"
    );
}

// ------------------------------------------------------------- pinned invariants

/// The spec states these numbers in section 5. A silent change to one of them changes every
/// request rho sends, and no other test would notice.
#[test]
fn default_result_limits_match_the_spec() {
    let limits = ResultLimits::default();

    assert_eq!(limits.max_result_bytes, 64 * 1024);
    assert_eq!(limits.store_threshold_bytes, 16 * 1024);
    assert_eq!(limits.preview_bytes, 4 * 1024);
    assert_eq!(limits.read_default_bytes, 8 * 1024);
    assert_eq!(limits.read_max_bytes, 64 * 1024);
    assert_eq!(limits.max_matches, 20);
}

/// The default policy caps and has no store. So a plain session is bounded, and adding a store
/// is what makes the tail readable.
#[test]
fn the_default_policy_caps_and_has_no_store() {
    let policy = ResultPolicy::default();

    assert!(policy.store.is_none(), "no store by default");
    assert_eq!(policy.limits, ResultLimits::default());
}

/// A tool result joins the append-only log and is never rewritten, so the block must render the
/// same bytes for the same input.
#[test]
fn head_preview_renders_byte_identically() {
    let preview = HeadPreview;

    let first = preview.render_stored("tr-0123456789abcdef-000001", "body", 9000);
    let second = preview.render_stored("tr-0123456789abcdef-000001", "body", 9000);

    assert_eq!(first, second);
    assert!(first.contains("tr-0123456789abcdef-000001"));
    assert!(first.contains("stored_bytes=\"9000\""));
}

#[test]
fn head_preview_selects_the_head() {
    let preview = HeadPreview;

    let kept = preview.select("HEADmiddleTAIL", 4);

    assert_eq!(kept, "HEAD", "the head says what happened");
}

/// Every wrong shape is refused at the store boundary, not only by the predicate.
///
/// `is_valid_handle_accepts_only_the_documented_shape` tests the predicate. This tests that the
/// store really calls it, for each kind of wrong shape.
#[tokio::test]
async fn a_handle_of_the_wrong_shape_is_refused() {
    let dir = TempDir::new().unwrap();
    let store = FileResultStore::open(dir.path()).await.unwrap();

    for handle in [
        "",
        "000001",
        "tr-000001",
        "tr-0123456789ABCDEF-000001",
        "tr-0123456789abcde-000001",
        "tr-0123456789abcdef-00001",
        "tr-0123456789abcdef-00000a",
        "tr-0123456789abcdef-000001.result",
    ] {
        let error = store
            .read_range(handle, 0, 64)
            .await
            .expect_err("a wrong shape must be refused");
        assert!(
            matches!(error, ResultStoreError::MalformedHandle(_)),
            "{handle:?} was not refused as malformed: {error:?}"
        );
    }
}
