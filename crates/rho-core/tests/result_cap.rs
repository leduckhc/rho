//! The cap at the choke point. SPEC-tool-result-handle section 8.
//!
//! These tests drive `cap_result_text`, which is what `Session::finish_tool` calls for every
//! text block of every tool result. See D-cap-at-one-choke-point.

use rho_core::{
    FileResultStore, ResultLimits, ResultPolicy, ResultStore, ResultStoreError, StoredSlice,
    cap_result_text,
};
use std::sync::Arc;
use tempfile::TempDir;

/// A policy with a real file store.
async fn with_store() -> (TempDir, ResultPolicy) {
    let dir = TempDir::new().unwrap();
    let store = FileResultStore::open(dir.path()).await.unwrap();
    let policy = ResultPolicy {
        store: Some(Arc::new(store)),
        ..ResultPolicy::default()
    };
    (dir, policy)
}

// ------------------------------------------------------- the cap always runs

#[tokio::test]
async fn a_small_result_is_unchanged() {
    let policy = ResultPolicy::default();

    let capped = cap_result_text(&policy, "a short result".to_string()).await;

    assert_eq!(
        capped.text, "a short result",
        "a small result costs no extra byte"
    );
    assert!(!capped.capped);
    assert!(capped.handle.is_none());
}

/// The regression test for the defect this feature exists to fix. With the default policy and
/// no store, a huge result must still be bounded.
#[tokio::test]
async fn a_large_result_is_cut_with_no_store() {
    let policy = ResultPolicy::default();
    let huge = "x".repeat(10 * 1024 * 1024);

    let capped = cap_result_text(&policy, huge).await;

    assert!(
        capped.text.len() < 70 * 1024,
        "ten megabytes reached the context: {} bytes",
        capped.text.len()
    );
    assert!(capped.capped);
}

#[tokio::test]
async fn the_cut_note_states_the_whole_size() {
    let policy = ResultPolicy::default();

    let capped = cap_result_text(&policy, "y".repeat(200_000)).await;

    assert!(
        capped.text.contains("200000"),
        "the note must name the whole size"
    );
    assert!(capped.text.contains("65536"), "and the bytes kept");
    assert!(
        capped.text.contains("not available"),
        "a cut that says nothing lets the model treat a partial result as whole"
    );
}

/// A tool that bounds nothing must not be able to fill the window. `bash` self-caps; an MCP or
/// plugin tool does not, and this is the boundary that covers them.
#[tokio::test]
async fn the_cap_applies_to_a_tool_that_does_not_bound_itself() {
    let policy = ResultPolicy::default();
    // Ten megabytes, as a peer rho does not ship would return.
    let from_a_peer = "P".repeat(10 * 1024 * 1024);

    let capped = cap_result_text(&policy, from_a_peer).await;

    assert!(capped.text.len() <= policy.limits.max_result_bytes + 200);
}

#[tokio::test]
async fn an_error_result_is_capped_too() {
    // `cap_result_text` runs on the text of a result, and `finish_tool` calls it whatever
    // `is_error` says. A huge failure message is bounded like any other.
    let policy = ResultPolicy::default();

    let capped = cap_result_text(&policy, "E".repeat(500_000)).await;

    assert!(capped.capped);
    assert!(capped.text.len() < 70 * 1024);
}

// ------------------------------------------------------------- the store path

#[tokio::test]
async fn a_large_result_is_replaced_by_a_preview() {
    let (_dir, policy) = with_store().await;
    // A distinctive marker in the middle. It must not reach the context.
    let mut payload = "a".repeat(100_000);
    payload.push_str("UNIQUE-MIDDLE-MARKER");
    payload.push_str(&"b".repeat(100_000));

    let capped = cap_result_text(&policy, payload).await;

    assert!(capped.text.contains("<tool_result_preview"));
    assert!(
        !capped.text.contains("UNIQUE-MIDDLE-MARKER"),
        "the payload must not reach the context, only a preview of it"
    );
    assert!(capped.handle.is_some());
}

#[tokio::test]
async fn a_large_result_keeps_the_head() {
    let (_dir, policy) = with_store().await;
    let payload = format!("HEAD-MARKER{}TAIL-MARKER", "m".repeat(100_000));

    let capped = cap_result_text(&policy, payload).await;

    assert!(
        capped.text.contains("HEAD-MARKER"),
        "the head says what happened"
    );
    assert!(!capped.text.contains("TAIL-MARKER"));
}

#[tokio::test]
async fn the_preview_states_the_stored_size() {
    let (_dir, policy) = with_store().await;

    let capped = cap_result_text(&policy, "s".repeat(123_456)).await;

    assert!(
        capped.text.contains("stored_bytes=\"123456\""),
        "the model must know how much there is: {}",
        &capped.text[..200.min(capped.text.len())]
    );
}

#[tokio::test]
async fn a_capped_preview_cuts_on_a_character_boundary() {
    let (_dir, policy) = with_store().await;
    // The character must be three bytes wide. The default preview is 4096 bytes, and 4096 is
    // even, so a two-byte character never straddles it and the test would prove nothing. A
    // first draft used 'é' and passed even with the boundary walk deleted. 4096 is not a
    // multiple of three, so '€' does straddle the cut.
    let payload = "€".repeat(50_000);
    assert_eq!("€".len(), 3);
    assert_ne!(
        policy.limits.preview_bytes % 3,
        0,
        "the cut must fall inside a character"
    );

    let capped = cap_result_text(&policy, payload).await;

    // The preview is inside the block. Extract it and check it decoded whole.
    let start = capped.text.find(">\n").unwrap() + 2;
    let end = capped.text.find("\n</tool_result_preview>").unwrap();
    let preview = &capped.text[start..end];
    assert!(preview.chars().all(|c| c == '€'), "a cut split a character");
    assert_eq!(preview.len() % 3, 0, "the cut landed inside a character");
}

/// Configuring a store must never cost context. It replaces a 64 KiB cut with a 4 KiB preview,
/// so it makes the context smaller.
#[tokio::test]
async fn a_store_keeps_less_context_than_no_store() {
    let payload = "z".repeat(500_000);

    let without = cap_result_text(&ResultPolicy::default(), payload.clone()).await;
    let (_dir, policy) = with_store().await;
    let with = cap_result_text(&policy, payload).await;

    assert!(
        with.text.len() < without.text.len(),
        "a store must shrink the context: with={} without={}",
        with.text.len(),
        without.text.len()
    );
}

/// A store that cannot write must not make the cap worse. Appending the whole payload here would
/// defeat the cap exactly when a large write is what broke the store.
#[tokio::test]
async fn a_store_failure_keeps_the_cap() {
    struct BrokenStore;

    #[async_trait::async_trait]
    impl ResultStore for BrokenStore {
        async fn put(&self, _text: &str) -> Result<String, ResultStoreError> {
            Err(ResultStoreError::Io("disk full".to_string()))
        }
        async fn read_range(
            &self,
            handle: &str,
            _start_byte: usize,
            _max_bytes: usize,
        ) -> Result<StoredSlice, ResultStoreError> {
            Err(ResultStoreError::NotFound(handle.to_string()))
        }
    }

    let policy = ResultPolicy {
        store: Some(Arc::new(BrokenStore)),
        ..ResultPolicy::default()
    };
    let payload = "f".repeat(1024 * 1024);

    let capped = cap_result_text(&policy, payload).await;

    assert!(
        capped.text.len() < 70 * 1024,
        "a broken store must not release the cap: {} bytes",
        capped.text.len()
    );
    assert!(capped.capped);
    assert!(
        capped.warning.is_some(),
        "the user must hear that the tail was lost"
    );
    assert!(capped.handle.is_none());
}

/// A caller may lower the limits. The threshold decides when a store is used.
#[tokio::test]
async fn a_result_below_the_store_threshold_is_not_stored() {
    let (_dir, mut policy) = with_store().await;
    policy.limits = ResultLimits {
        store_threshold_bytes: 100_000,
        ..ResultLimits::default()
    };

    let capped = cap_result_text(&policy, "u".repeat(20_000)).await;

    assert!(
        capped.handle.is_none(),
        "below the threshold, storing costs a file"
    );
    assert_eq!(capped.text.len(), 20_000, "and the result is unchanged");
}
