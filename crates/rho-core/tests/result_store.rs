//! The result store: round trips, bounds, concurrency, and resume isolation.
//! SPEC-tool-result-handle section 8.

use rho_core::{FileResultStore, ResultStore, ResultStoreError};
use tempfile::TempDir;

async fn store() -> (TempDir, FileResultStore) {
    let dir = TempDir::new().unwrap();
    let store = FileResultStore::open(dir.path()).await.unwrap();
    (dir, store)
}

#[tokio::test]
async fn put_then_read_returns_the_text() {
    let (_dir, store) = store().await;
    let handle = store.put("hello evidence").await.unwrap();

    let slice = store.read_range(&handle, 0, 1024).await.unwrap();

    assert_eq!(slice.text, "hello evidence");
    assert_eq!(slice.total_bytes, 14);
}

#[tokio::test]
async fn handles_are_unique_within_a_store() {
    let (_dir, store) = store().await;

    let first = store.put("one").await.unwrap();
    let second = store.put("two").await.unwrap();

    assert_ne!(first, second);
    assert_eq!(store.read_range(&first, 0, 64).await.unwrap().text, "one");
    assert_eq!(store.read_range(&second, 0, 64).await.unwrap().text, "two");
}

/// Tool dispatch can run in parallel, so two puts can ask for a sequence number at once. A
/// store that read a "highest handle" from the directory would hand out the same number twice
/// and lose a payload.
#[tokio::test]
async fn concurrent_puts_never_share_a_handle() {
    let dir = TempDir::new().unwrap();
    let store = std::sync::Arc::new(FileResultStore::open(dir.path()).await.unwrap());

    let mut tasks = Vec::new();
    for i in 0..32 {
        let store = std::sync::Arc::clone(&store);
        tasks.push(tokio::spawn(async move {
            let text = format!("payload-{i}");
            let handle = store.put(&text).await.unwrap();
            (handle, text)
        }));
    }

    let mut seen = std::collections::BTreeMap::new();
    for task in tasks {
        let (handle, text) = task.await.unwrap();
        assert!(
            seen.insert(handle.clone(), text.clone()).is_none(),
            "handle {handle} was handed out twice"
        );
    }
    assert_eq!(seen.len(), 32);

    // Every payload must still be its own. A collision would have overwritten one.
    for (handle, expected) in seen {
        let slice = store.read_range(&handle, 0, 128).await.unwrap();
        assert_eq!(
            slice.text, expected,
            "handle {handle} holds the wrong payload"
        );
    }
}

#[tokio::test]
async fn read_range_reports_the_total_size() {
    let (_dir, store) = store().await;
    let handle = store.put(&"a".repeat(5000)).await.unwrap();

    let slice = store.read_range(&handle, 0, 100).await.unwrap();

    assert_eq!(slice.text.len(), 100);
    assert_eq!(slice.total_bytes, 5000);
}

/// A model that reads past the end must learn it is done. An error, or a wrong total, would
/// let it loop.
#[tokio::test]
async fn read_range_past_the_end_returns_empty_with_the_true_total() {
    let (_dir, store) = store().await;
    let handle = store.put("short").await.unwrap();

    let slice = store.read_range(&handle, 9000, 100).await.unwrap();

    assert_eq!(slice.text, "");
    assert_eq!(
        slice.total_bytes, 5,
        "the true size, so the model can tell it is done"
    );
}

#[tokio::test]
async fn read_range_reports_its_own_offsets() {
    let (_dir, store) = store().await;
    let handle = store.put(&"b".repeat(1000)).await.unwrap();

    let slice = store.read_range(&handle, 100, 50).await.unwrap();

    assert_eq!(slice.start_byte, 100);
    assert_eq!(
        slice.end_byte, 150,
        "the offsets describe the slice returned"
    );
    assert_eq!(slice.text.len(), 50);
}

#[tokio::test]
async fn read_range_clamps_to_the_maximum() {
    let (_dir, store) = store().await;
    let handle = store.put(&"c".repeat(200_000)).await.unwrap();

    let slice = store.read_range(&handle, 0, 10_000_000).await.unwrap();

    assert!(
        slice.text.len() <= 64 * 1024,
        "a read must never exceed the maximum, got {}",
        slice.text.len()
    );
}

#[tokio::test]
async fn read_range_cuts_on_a_character_boundary() {
    let (_dir, store) = store().await;
    // Each 'é' is two bytes, so an odd length lands inside a character.
    let handle = store.put(&"é".repeat(500)).await.unwrap();

    let slice = store.read_range(&handle, 0, 101).await.unwrap();

    assert!(slice.text.chars().all(|c| c == 'é'));
    assert_eq!(slice.text.len() % 2, 0, "the cut landed inside a character");
}

#[tokio::test]
async fn search_finds_a_literal_with_its_line_number() {
    let (_dir, store) = store().await;
    let handle = store
        .put("first line\nsecond line\nthird NEEDLE here\nfourth\n")
        .await
        .unwrap();

    let matches = store.search(&handle, "NEEDLE", 20).await.unwrap();

    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].line_number, 3);
    assert_eq!(matches[0].line, "third NEEDLE here");
    // "first line\n" is 11 bytes, "second line\n" is 12, so line three starts at 23. "third "
    // is 6 more, so the needle starts at 29. The first draft of this test said 28, and the
    // implementation was right.
    assert_eq!(matches[0].start_byte, 29);
}

#[tokio::test]
async fn search_returns_nothing_for_no_match() {
    let (_dir, store) = store().await;
    let handle = store.put("nothing to find here").await.unwrap();

    let matches = store.search(&handle, "ABSENT", 20).await.unwrap();

    assert!(matches.is_empty());
}

#[tokio::test]
async fn search_caps_the_match_count() {
    let (_dir, store) = store().await;
    let body: String = (0..100).map(|i| format!("line {i} HIT\n")).collect();
    let handle = store.put(&body).await.unwrap();

    let matches = store.search(&handle, "HIT", 20).await.unwrap();

    assert_eq!(
        matches.len(),
        20,
        "a search must not return the payload again"
    );
}

/// The required surface is `put` and `read_range`. A store that implements only those two must
/// still search, or the trait is larger than it needs to be.
#[tokio::test]
async fn the_default_search_scans_through_read_range() {
    struct MinimalStore {
        text: String,
    }

    #[async_trait::async_trait]
    impl ResultStore for MinimalStore {
        async fn put(&self, _text: &str) -> Result<String, ResultStoreError> {
            Ok("tr-00000000000000ff-000001".to_string())
        }
        async fn read_range(
            &self,
            _handle: &str,
            start_byte: usize,
            max_bytes: usize,
        ) -> Result<rho_core::StoredSlice, ResultStoreError> {
            let total = self.text.len();
            let start = start_byte.min(total);
            let end = (start + max_bytes).min(total);
            Ok(rho_core::StoredSlice {
                text: self.text[start..end].to_string(),
                start_byte: start,
                end_byte: end,
                total_bytes: total,
            })
        }
    }

    let store = MinimalStore {
        text: "alpha\nbeta TARGET\ngamma\n".to_string(),
    };

    let matches = store
        .search("tr-00000000000000ff-000001", "TARGET", 5)
        .await
        .unwrap();

    assert_eq!(matches.len(), 1, "the default body must really scan");
    assert_eq!(matches[0].line_number, 2);
}

#[tokio::test]
async fn an_unknown_handle_is_not_found() {
    let (_dir, store) = store().await;

    let error = store
        .read_range("tr-0123456789abcdef-999999", 0, 64)
        .await
        .expect_err("an absent handle is an error");

    assert!(matches!(error, ResultStoreError::NotFound(_)), "{error:?}");
}

#[tokio::test]
async fn opening_a_store_creates_its_directory() {
    let dir = TempDir::new().unwrap();
    let nested = dir.path().join("results").join("deeper");

    let store = FileResultStore::open(&nested).await.unwrap();
    let handle = store.put("made it").await.unwrap();

    assert!(nested.is_dir(), "open must create the directory");
    assert_eq!(
        store.read_range(&handle, 0, 64).await.unwrap().text,
        "made it"
    );
}

/// A reader must see the whole payload or nothing.
///
/// `put` writes a temporary file and renames it into place, and a rename inside one directory is
/// atomic. A first draft of this test only checked that no temporary file was left behind, which
/// would also pass if `put` wrote the payload directly. So it read a handle repeatedly while a
/// large put ran: with a direct write a reader can observe a growing file and a total size that
/// is a lie.
#[tokio::test]
async fn a_partial_write_is_never_read() {
    let dir = TempDir::new().unwrap();
    let store = std::sync::Arc::new(FileResultStore::open(dir.path()).await.unwrap());
    // Large enough that a direct write takes several observable steps.
    let big = "z".repeat(8 * 1024 * 1024);
    let expected = big.len();

    // Guess the handle the next put will take, so a reader can race it.
    let handle = format!("tr-{}-000001", store.nonce());

    let reader = {
        let store = std::sync::Arc::clone(&store);
        let handle = handle.clone();
        tokio::spawn(async move {
            let mut seen_sizes = Vec::new();
            for _ in 0..2000 {
                // A NotFound before the rename is expected, and it is not a partial read.
                if let Ok(slice) = store.read_range(&handle, 0, 64).await {
                    seen_sizes.push(slice.total_bytes);
                }
                tokio::task::yield_now().await;
            }
            seen_sizes
        })
    };

    let written = store.put(&big).await.unwrap();
    assert_eq!(
        written, handle,
        "the handle guess must match, or the race proves nothing"
    );
    let seen_sizes = reader.await.unwrap();

    // Every size a reader ever saw must be the whole size. A partial file would report less.
    for size in &seen_sizes {
        assert_eq!(
            *size, expected,
            "a reader saw a partial payload of {size} bytes; put is not atomic"
        );
    }

    // And no temporary file is left behind for a later read to pick up.
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| !name.ends_with(".result"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temporary files left behind: {leftovers:?}"
    );
}
