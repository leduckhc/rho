//! The bounded JSONL line reader.
//!
//! See `docs/specs/20260819-102749-SPEC-jsonl-frontend.md` section 4.
//!
//! LF is the only record delimiter. The reader strips a trailing CR. It never
//! splits on U+2028 or U+2029, because those code points are legal inside a JSON
//! string and a reader that splits on them corrupts real data.
//!
//! One line is capped. This project shipped an unbounded reader twice, and an
//! unbounded `bash` reader once turned 8 MB of output into 805 MB of memory. See
//! decisions D-bash-line-cap, D-reader-line-cap, and D-a-command-line-is-capped.

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

/// The byte cap for one command line. 1 MiB.
///
/// A prompt is text, and 1 MiB of text is more than most context windows hold. So
/// the cap cannot refuse a real prompt, and it still bounds the reader. It is a
/// constant and not a config key, because a cap a caller can raise is a cap a peer
/// can escape. See D-cap-at-one-choke-point.
pub const MAX_COMMAND_LINE_BYTES: usize = 1024 * 1024;

/// One record read from the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// A whole record, with any trailing CR removed.
    ///
    /// It carries bytes, not a `String`. Invalid UTF-8 must reach the caller as a
    /// parse error with a reply, and never as a reader error that ends the session.
    Record(Vec<u8>),
    /// A run of bytes with no newline that reached the cap.
    ///
    /// `bytes` is how many bytes the reader consumed for it, so a test asserts the
    /// bytes **read** and not the bytes kept. A memory-cap test here once passed
    /// against the very bug it was written for, because it measured what was kept
    /// while the read buffer still grew. See D-bash-line-cap.
    ///
    /// One enormous line yields this more than once, once per cap-sized run. That is
    /// deliberate. `next_line` must always return, because a peer that opens the pipe
    /// and never writes a newline would otherwise hang the reader for ever. A caller
    /// that replies to the client collapses a run of these into one reply.
    TooLong { bytes: usize },
}

/// A reader that yields one JSONL record at a time, with a byte cap per line.
///
/// Two guarantees hold for every call to [`LineReader::next_line`]. It consumes no
/// more than the cap plus one read buffer. It always returns.
pub struct LineReader<R> {
    inner: BufReader<R>,
    cap: usize,
    /// True while the reader is throwing away the tail of an over-long line. It
    /// stops when the next newline arrives.
    discarding: bool,
}

impl<R: AsyncRead + Unpin> LineReader<R> {
    /// A reader with the standard cap.
    pub fn new(inner: R) -> Self {
        Self::with_cap(inner, MAX_COMMAND_LINE_BYTES)
    }

    /// A reader with a stated cap. A test uses a small cap, so it never allocates a
    /// megabyte to prove a rule about one.
    pub fn with_cap(inner: R, cap: usize) -> Self {
        Self {
            inner: BufReader::new(inner),
            cap,
            discarding: false,
        }
    }

    /// Read the next record. `Ok(None)` means the input reached end of file.
    ///
    /// A line that reaches the cap before its newline is reported as
    /// [`Line::TooLong`]. The reader then throws away bytes to the next newline, and
    /// reports `TooLong` again for each further cap-sized run. The record after the
    /// newline reads normally.
    pub async fn next_line(&mut self) -> std::io::Result<Option<Line>> {
        if self.discarding {
            // Finish throwing away the tail of the previous over-long line. This is
            // bounded by the cap as well, so an endless stream with no newline
            // returns instead of looping.
            match self.skip_to_newline().await? {
                Skip::Eof => return Ok(None),
                Skip::StillTooLong { bytes } => return Ok(Some(Line::TooLong { bytes })),
                Skip::Found => self.discarding = false,
            }
        }

        let mut kept: Vec<u8> = Vec::new();
        // Bytes consumed for this line so far. It is the read count, not the keep
        // count.
        let mut consumed = 0usize;

        loop {
            let available = self.inner.fill_buf().await?;
            if available.is_empty() {
                // End of file. A last line with no newline is still a record.
                if consumed == 0 {
                    return Ok(None);
                }
                return Ok(Some(Line::Record(strip_cr(kept))));
            }

            match available.iter().position(|byte| *byte == b'\n') {
                Some(at) => {
                    // Copy before `consume`, because `available` borrows the buffer.
                    let line: Vec<u8> = available[..at].to_vec();
                    self.inner.consume(at + 1);
                    consumed += at + 1;
                    if kept.len() + at > self.cap {
                        // The cap applies here too, not only when the newline is
                        // absent. A whole over-long line often arrives inside one read
                        // buffer, and checking only the no-newline branch let it
                        // through. The newline is already consumed, so the reader is
                        // lined up on the next record and needs no discard.
                        return Ok(Some(Line::TooLong { bytes: consumed }));
                    }
                    kept.extend_from_slice(&line);
                    return Ok(Some(Line::Record(strip_cr(kept))));
                }
                None => {
                    let taken = available.len();
                    kept.extend_from_slice(available);
                    consumed += taken;
                    self.inner.consume(taken);
                    if kept.len() > self.cap {
                        // Return now. Waiting for a newline that may never arrive is
                        // the hang this branch exists to prevent. Drop the bytes
                        // before returning, so the cap bounds memory as well as time.
                        self.discarding = true;
                        drop(kept);
                        return Ok(Some(Line::TooLong { bytes: consumed }));
                    }
                }
            }
        }
    }

    /// Throw away bytes up to and including the next newline, for at most one cap.
    async fn skip_to_newline(&mut self) -> std::io::Result<Skip> {
        let mut skipped = 0usize;
        loop {
            let available = self.inner.fill_buf().await?;
            if available.is_empty() {
                return Ok(Skip::Eof);
            }
            match available.iter().position(|byte| *byte == b'\n') {
                Some(at) => {
                    self.inner.consume(at + 1);
                    return Ok(Skip::Found);
                }
                None => {
                    let taken = available.len();
                    self.inner.consume(taken);
                    skipped += taken;
                    if skipped > self.cap {
                        return Ok(Skip::StillTooLong { bytes: skipped });
                    }
                }
            }
        }
    }
}

/// The outcome of throwing away the tail of an over-long line.
enum Skip {
    /// The newline arrived. The next record starts after it.
    Found,
    /// One cap of bytes went by with no newline.
    StillTooLong { bytes: usize },
    /// The input ended inside the over-long line.
    Eof,
}

/// Remove one trailing CR. A writer must not send one, and a reader must cope.
fn strip_cr(mut line: Vec<u8>) -> Vec<u8> {
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    line
}
