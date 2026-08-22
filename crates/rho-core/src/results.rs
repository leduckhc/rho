//! Cap a large tool result, and keep a handle back to the whole of it.
//!
//! See `docs/specs/20260822-103220-SPEC-tool-result-handle.md`.
//!
//! Two rules govern this module, and section 2 of the spec explains why.
//!
//! 1. **The cap always runs.** Every tool result meets `max_result_bytes` before it reaches
//!    the context, whatever the caller configured. A cap a caller can switch off is a cap a
//!    peer can escape. See D-cap-at-one-choke-point.
//! 2. **The store is optional, and it makes the cap kind.** With a store, the context keeps a
//!    small preview and a handle, because the tail is readable. So a store makes the context
//!    smaller, never larger.

use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Why a store operation failed.
#[derive(Debug, thiserror::Error)]
pub enum ResultStoreError {
    /// The handle does not match the documented shape. It was never used as a path.
    #[error("handle {0:?} is not a valid result handle")]
    MalformedHandle(String),
    /// No stored result has that handle in this store.
    #[error("no stored result has handle {0}")]
    NotFound(String),
    /// The store could not be read or written.
    #[error("result store input or output failed: {0}")]
    Io(String),
}

/// One slice of a stored result.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredSlice {
    /// The text in the requested range, cut on a character boundary.
    pub text: String,
    /// The zero-based byte offset the slice starts at.
    pub start_byte: usize,
    /// The byte offset one past the end of the slice.
    pub end_byte: usize,
    /// The whole payload size, so the model can plan the next read. This is always the whole
    /// size, even for an empty slice past the end, or the model cannot learn it is done.
    pub total_bytes: usize,
}

/// One literal match inside a stored result.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredMatch {
    /// The one-based line number the match is on.
    pub line_number: usize,
    /// The byte offset of the match within the payload.
    pub start_byte: usize,
    /// The whole line, bounded. A very long line is cut on a character boundary.
    pub line: String,
}

/// The most bytes one line of a search match carries into the context.
pub const MAX_MATCH_LINE_BYTES: usize = 512;

/// The hard ceiling on one `read_range`, whatever a caller asks for.
///
/// `ResultLimits::read_max_bytes` is the policy a caller may lower. This is the ceiling the
/// store enforces itself, so a caller cannot raise it by passing a larger number.
pub const READ_CEILING_BYTES: usize = 64 * 1024;

/// The most bytes one pending line may hold while a search scans.
///
/// A payload with no newline at all must not grow the scan buffer without bound.
const SEARCH_PENDING_CEILING_BYTES: usize = 1024 * 1024;

/// Where a large tool result is kept so the model can read it later.
///
/// A store belongs to one session. An implementation must refuse a malformed handle before it
/// builds any path from it.
#[async_trait]
pub trait ResultStore: Send + Sync {
    /// Store `text` and return its handle. The handle is unique within this store, even when
    /// two callers put at the same time.
    async fn put(&self, text: &str) -> Result<String, ResultStoreError>;

    /// Read at most `max_bytes` from `start_byte`.
    ///
    /// A range past the end returns an empty slice with the true `total_bytes`, not an error,
    /// so a model learns it has read everything.
    async fn read_range(
        &self,
        handle: &str,
        start_byte: usize,
        max_bytes: usize,
    ) -> Result<StoredSlice, ResultStoreError>;

    /// Find a literal string. Returns at most `max_matches` matches.
    ///
    /// The default scans the payload through `read_range`, in this process. It never puts a
    /// slice into the context, so the scan costs no tokens. A store whose backend can search
    /// on the server overrides this and saves the transfer.
    async fn search(
        &self,
        handle: &str,
        needle: &str,
        max_matches: usize,
    ) -> Result<Vec<StoredMatch>, ResultStoreError> {
        default_search(self, handle, needle, max_matches).await
    }
}

/// Chooses what part of an oversize result the context keeps, and how it reads.
///
/// This is the extension point for the shape of the cap. A caller that wants the tail, or
/// both ends, or a different block format, supplies an impl. It does not edit `rho-core`.
pub trait ResultPreview: Send + Sync {
    /// The bytes to keep from `text`, at most `max_bytes`, cut on a character boundary.
    fn select(&self, text: &str, max_bytes: usize) -> String;

    /// Render the block the context keeps for a stored result.
    ///
    /// Must be byte-identical for the same input, because a tool result joins the append-only
    /// log and is never rewritten.
    fn render_stored(&self, handle: &str, preview: &str, stored_bytes: usize) -> String;

    /// Render the block the context keeps when there is no store and the tail is lost.
    fn render_cut(&self, kept: &str, kept_bytes: usize, whole_bytes: usize) -> String;
}

/// Keeps the head of a result. The default, because a command's first lines say what happened.
pub struct HeadPreview;

impl ResultPreview for HeadPreview {
    fn select(&self, text: &str, max_bytes: usize) -> String {
        cut_on_boundary(text, max_bytes).to_string()
    }

    fn render_stored(&self, handle: &str, preview: &str, stored_bytes: usize) -> String {
        format!(
            "<tool_result_preview handle=\"{handle}\" stored_bytes=\"{stored_bytes}\" \
             preview_bytes=\"{}\">\n{preview}\n</tool_result_preview>\n\
             The full result is stored outside the context. Use read_tool_result with this \
             exact handle to read a byte range, or to search it for a literal string.",
            preview.len()
        )
    }

    fn render_cut(&self, kept: &str, kept_bytes: usize, whole_bytes: usize) -> String {
        format!(
            "{kept}\n[rho cut this result at {kept_bytes} bytes of {whole_bytes}. The rest is \
             not available, because this session has no result store.]"
        )
    }
}

/// The bounds from spec section 5.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResultLimits {
    /// The most bytes one tool result may contribute to the context. Always enforced.
    pub max_result_bytes: usize,
    /// A result at or above this size is stored, when a store exists.
    pub store_threshold_bytes: usize,
    /// The preview kept in the context when the result is stored.
    pub preview_bytes: usize,
    /// The default size of one `read_tool_result` read.
    pub read_default_bytes: usize,
    /// The largest size one `read_tool_result` read may ask for.
    pub read_max_bytes: usize,
    /// The most matches one search returns.
    pub max_matches: usize,
}

impl Default for ResultLimits {
    fn default() -> Self {
        Self {
            max_result_bytes: 64 * 1024,
            store_threshold_bytes: 16 * 1024,
            preview_bytes: 4 * 1024,
            read_default_bytes: 8 * 1024,
            read_max_bytes: 64 * 1024,
            max_matches: 20,
        }
    }
}

/// How a session caps a tool result, and where the tail goes.
///
/// `limits` and `preview` always apply. `store` is what makes the tail readable.
#[derive(Clone)]
pub struct ResultPolicy {
    pub limits: ResultLimits,
    pub preview: Arc<dyn ResultPreview>,
    pub store: Option<Arc<dyn ResultStore>>,
}

impl Default for ResultPolicy {
    /// The cap runs with no store. So a default session is bounded, and its tail is lost.
    fn default() -> Self {
        Self {
            limits: ResultLimits::default(),
            preview: Arc::new(HeadPreview),
            store: None,
        }
    }
}

impl std::fmt::Debug for ResultPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResultPolicy")
            .field("limits", &self.limits)
            .field("has_store", &self.store.is_some())
            .finish()
    }
}

/// True when `handle` matches `^tr-[0-9a-f]{16}-[0-9]{6,}$`.
///
/// Every store calls this before it builds a path.
pub fn is_valid_handle(handle: &str) -> bool {
    let Some(rest) = handle.strip_prefix("tr-") else {
        return false;
    };
    let Some((nonce, sequence)) = rest.split_once('-') else {
        return false;
    };
    nonce.len() == 16
        && nonce
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        && sequence.len() >= 6
        && sequence.bytes().all(|b| b.is_ascii_digit())
}

/// A store that writes one file per result, beside the session file.
///
/// See D-stored-result-inherits-session-trust. The directory inherits the session file's
/// privacy and its lifetime.
pub struct FileResultStore {
    directory: PathBuf,
    /// Stamped into every handle this store makes. A fresh value on every open is what stops
    /// a resumed session from reading or clobbering an earlier run. See spec section 4.
    nonce: String,
    /// The sequence source. Atomic, because tool dispatch can put in parallel.
    next: AtomicUsize,
}

impl FileResultStore {
    /// Open or create a store in `directory`.
    ///
    /// Each open takes a fresh nonce, so a resumed session can neither read nor overwrite an
    /// earlier run's results.
    pub async fn open(directory: impl Into<PathBuf>) -> Result<Self, ResultStoreError> {
        let directory = directory.into();
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(|e| ResultStoreError::Io(format!("{}: {e}", directory.display())))?;
        let nonce = fresh_nonce(&directory);
        Ok(Self {
            directory,
            nonce,
            // The sequence starts at one and never reads the directory. A fresh nonce already
            // separates this run from any earlier one, so there is nothing to continue past.
            next: AtomicUsize::new(1),
        })
    }

    /// The nonce this store stamps into every handle it makes.
    pub fn nonce(&self) -> &str {
        &self.nonce
    }

    /// The path for one handle. The handle is validated before this runs.
    fn path_for(&self, handle: &str) -> PathBuf {
        self.directory.join(format!("{handle}.result"))
    }
}

#[async_trait]
impl ResultStore for FileResultStore {
    async fn put(&self, text: &str) -> Result<String, ResultStoreError> {
        // Up to a bounded number of attempts. An attempt only repeats when another task took
        // the same name, which `create_new` detects rather than overwriting.
        for _ in 0..64 {
            let sequence = self.next.fetch_add(1, Ordering::Relaxed);
            let handle = format!("{}-{:06}", self.handle_prefix(), sequence);
            debug_assert!(
                is_valid_handle(&handle),
                "the store made a handle it would refuse"
            );
            let final_path = self.path_for(&handle);

            // Write a temporary file, then rename. A rename inside one directory is atomic, so
            // a concurrent read sees the whole payload or no file at all. It never sees a
            // prefix with a wrong total size.
            let temporary = self.directory.join(format!(".{handle}.partial"));
            match tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .await
            {
                Ok(mut file) => {
                    use tokio::io::AsyncWriteExt;
                    let write = async {
                        file.write_all(text.as_bytes()).await?;
                        file.flush().await
                    };
                    if let Err(error) = write.await {
                        let _ = tokio::fs::remove_file(&temporary).await;
                        return Err(ResultStoreError::Io(format!(
                            "{}: {error}",
                            temporary.display()
                        )));
                    }
                    drop(file);
                    if let Err(error) = tokio::fs::rename(&temporary, &final_path).await {
                        // Do not leave the temporary file behind. A later scan of the directory
                        // would see a name nothing can read.
                        let _ = tokio::fs::remove_file(&temporary).await;
                        return Err(ResultStoreError::Io(format!(
                            "{}: {error}",
                            final_path.display()
                        )));
                    }
                    return Ok(handle);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(ResultStoreError::Io(format!(
                        "{}: {error}",
                        temporary.display()
                    )));
                }
            }
        }
        Err(ResultStoreError::Io(
            "could not allocate a free result handle".to_string(),
        ))
    }

    async fn read_range(
        &self,
        handle: &str,
        start_byte: usize,
        max_bytes: usize,
    ) -> Result<StoredSlice, ResultStoreError> {
        // The shape check runs before any path is built, so an untrusted string never becomes
        // a path. See spec section 4.
        if !is_valid_handle(handle) {
            return Err(ResultStoreError::MalformedHandle(handle.to_string()));
        }
        // A handle must carry this store's nonce. Without this check the nonce would only stop
        // a new run from overwriting an old file, and a resumed session could still read every
        // result of the previous run by name. A well-formed handle from another run is not
        // malformed, so it reports `NotFound` and leaks nothing about what exists on disk.
        if !handle.starts_with(&self.handle_prefix()) {
            return Err(ResultStoreError::NotFound(handle.to_string()));
        }
        let path = self.path_for(handle);

        let mut file = match tokio::fs::File::open(&path).await {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(ResultStoreError::NotFound(handle.to_string()));
            }
            Err(error) => {
                return Err(ResultStoreError::Io(format!("{}: {error}", path.display())));
            }
        };
        let total_bytes = file
            .metadata()
            .await
            .map_err(|e| ResultStoreError::Io(format!("{}: {e}", path.display())))?
            .len() as usize;

        // A read past the end is not an error. The model learns it is done from the empty
        // text plus the true total.
        if start_byte >= total_bytes {
            return Ok(StoredSlice {
                text: String::new(),
                start_byte: total_bytes,
                end_byte: total_bytes,
                total_bytes,
            });
        }

        let want = max_bytes.min(READ_CEILING_BYTES);
        let end = (start_byte + want).min(total_bytes);
        let mut buffer = vec![0u8; end - start_byte];
        {
            use tokio::io::{AsyncReadExt, AsyncSeekExt};
            file.seek(std::io::SeekFrom::Start(start_byte as u64))
                .await
                .map_err(|e| ResultStoreError::Io(format!("{}: {e}", path.display())))?;
            file.read_exact(&mut buffer)
                .await
                .map_err(|e| ResultStoreError::Io(format!("{}: {e}", path.display())))?;
        }

        let (text, leading, trailing) = decode_window(&buffer);
        Ok(StoredSlice {
            text,
            start_byte: start_byte + leading,
            end_byte: end - trailing,
            total_bytes,
        })
    }
}

impl FileResultStore {
    /// The `tr-<nonce>` part every handle from this store begins with.
    fn handle_prefix(&self) -> String {
        format!("tr-{}", self.nonce)
    }
}

/// Decode a byte window into text that starts and ends on character boundaries.
///
/// Returns the text, the bytes skipped at the front, and the bytes dropped at the end. A window
/// can begin or end inside a multi-byte character, and invalid UTF-8 in a request would fail at
/// the provider.
fn decode_window(buffer: &[u8]) -> (String, usize, usize) {
    // Skip any continuation bytes at the front. There are at most three.
    let mut leading = 0usize;
    while leading < buffer.len() && leading < 4 && (buffer[leading] & 0b1100_0000) == 0b1000_0000 {
        leading += 1;
    }
    let rest = &buffer[leading..];

    match std::str::from_utf8(rest) {
        Ok(text) => (text.to_string(), leading, 0),
        Err(error) => {
            let valid = error.valid_up_to();
            let text = String::from_utf8_lossy(&rest[..valid]).into_owned();
            (text, leading, rest.len() - valid)
        }
    }
}

/// Scan a stored payload for a literal, through `read_range`.
///
/// This is the default body of `ResultStore::search`. It reads in bounded slices, in this
/// process, so the scan never touches the context.
pub async fn default_search<S>(
    store: &S,
    handle: &str,
    needle: &str,
    max_matches: usize,
) -> Result<Vec<StoredMatch>, ResultStoreError>
where
    S: ResultStore + ?Sized,
{
    let mut matches = Vec::new();
    if needle.is_empty() || max_matches == 0 {
        return Ok(matches);
    }

    // Scan whole lines, so a needle that straddles a slice boundary is still found. The pending
    // buffer holds one partial line, and it is capped so a payload with no newline cannot grow
    // it without bound.
    let mut pending = String::new();
    let mut pending_start = 0usize;
    let mut line_number = 1usize;
    let mut offset = 0usize;

    loop {
        let slice = store.read_range(handle, offset, READ_CEILING_BYTES).await?;
        if slice.text.is_empty() {
            break;
        }
        offset = slice.end_byte;
        pending.push_str(&slice.text);

        while let Some(position) = pending.find('\n') {
            let line: String = pending.drain(..position).collect();
            pending.drain(..1);
            record_match(
                &mut matches,
                &line,
                needle,
                line_number,
                pending_start,
                max_matches,
            );
            pending_start += line.len() + 1;
            line_number += 1;
            if matches.len() >= max_matches {
                return Ok(matches);
            }
        }

        // A single line longer than the ceiling is scanned in place, so memory stays bounded.
        // The line number does not advance, because this is still the same logical line.
        if pending.len() > SEARCH_PENDING_CEILING_BYTES {
            record_match(
                &mut matches,
                &pending,
                needle,
                line_number,
                pending_start,
                max_matches,
            );
            pending_start += pending.len();
            pending.clear();
            if matches.len() >= max_matches {
                return Ok(matches);
            }
        }

        if offset >= slice.total_bytes {
            break;
        }
    }

    if !pending.is_empty() {
        record_match(
            &mut matches,
            &pending,
            needle,
            line_number,
            pending_start,
            max_matches,
        );
    }
    Ok(matches)
}

/// Record every occurrence of `needle` in one line, up to the match cap.
fn record_match(
    matches: &mut Vec<StoredMatch>,
    line: &str,
    needle: &str,
    line_number: usize,
    line_start: usize,
    max_matches: usize,
) {
    let mut from = 0usize;
    while let Some(position) = line[from..].find(needle) {
        if matches.len() >= max_matches {
            return;
        }
        let at = from + position;
        matches.push(StoredMatch {
            line_number,
            start_byte: line_start + at,
            line: cut_on_boundary(line, MAX_MATCH_LINE_BYTES).to_string(),
        });
        from = at + needle.len();
        if from >= line.len() {
            return;
        }
    }
}

/// What the cap decided for one tool result.
#[derive(Clone, Debug, PartialEq)]
pub struct CappedText {
    /// The text the context keeps.
    pub text: String,
    /// The handle, when the result was stored.
    pub handle: Option<String>,
    /// True when the context holds less than the whole result.
    pub capped: bool,
    /// A line for the user when the store failed. The model never sees it.
    pub warning: Option<String>,
}

/// Apply the policy to one tool result text.
///
/// This is the one choke point. See D-cap-at-one-choke-point.
pub async fn cap_result_text(policy: &ResultPolicy, text: String) -> CappedText {
    let limits = &policy.limits;
    let whole_bytes = text.len();

    // A store makes the context smaller, so it is tried first for anything at the threshold.
    if let Some(store) = policy
        .store
        .as_ref()
        .filter(|_| whole_bytes >= limits.store_threshold_bytes)
    {
        {
            match store.put(&text).await {
                Ok(handle) => {
                    let preview = policy.preview.select(&text, limits.preview_bytes);
                    return CappedText {
                        text: policy.preview.render_stored(&handle, &preview, whole_bytes),
                        handle: Some(handle),
                        capped: true,
                        warning: None,
                    };
                }
                Err(error) => {
                    // The cap still runs. Appending the whole payload here would defeat the cap
                    // exactly when a large write is what broke the store. See the spec's
                    // `a_store_failure_keeps_the_cap`.
                    let mut capped = cut_with_note(policy, text, whole_bytes);
                    capped.warning = Some(format!(
                        "a large tool result could not be stored: {error}. The result was cut                          to {} bytes and the rest is not available.",
                        limits.max_result_bytes
                    ));
                    return capped;
                }
            }
        }
    }

    if whole_bytes <= limits.max_result_bytes {
        return CappedText {
            text,
            handle: None,
            capped: false,
            warning: None,
        };
    }
    cut_with_note(policy, text, whole_bytes)
}

/// Cut a result at `max_result_bytes` and say so.
///
/// A cut that says nothing lets the model treat a partial result as whole.
fn cut_with_note(policy: &ResultPolicy, text: String, whole_bytes: usize) -> CappedText {
    let kept = policy.preview.select(&text, policy.limits.max_result_bytes);
    let kept_bytes = kept.len();
    CappedText {
        text: policy.preview.render_cut(&kept, kept_bytes, whole_bytes),
        handle: None,
        capped: true,
        warning: None,
    }
}

/// The largest prefix of `text` that is at most `max_bytes` and ends on a character boundary.
fn cut_on_boundary(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// A fresh nonce for one store.
///
/// This is not a security token. It stops a later run from reaching an earlier run's handles,
/// so it needs to be unpredictable across opens, not cryptographically strong.
fn fresh_nonce(directory: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    directory.hash(&mut hasher);
    // A second sample, so two opens in the same nanosecond still differ.
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed).hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
