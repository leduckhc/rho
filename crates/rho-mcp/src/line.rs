//! A bounded line reader.
//!
//! A line from a server is untrusted input. An uncapped reader grows its buffer
//! to hold one whole line, so a hostile server can exhaust host memory with one
//! enormous line. Decision D-016 records a case where an uncapped reader reached
//! 805 MB of resident memory. So the reader caps one line and reads in buffered
//! chunks, not one byte at a time.

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

/// The result of one bounded line read.
pub enum LineOutcome {
    /// A complete line, within the cap.
    Line(String),
    /// A line longer than the cap. It is discarded, so memory stays bounded.
    TooLong,
    /// The stream ended.
    Eof,
}

/// A reader that yields one bounded line at a time from an async source.
pub struct BoundedLineReader<R> {
    reader: BufReader<R>,
    max: usize,
}

impl<R: AsyncRead + Unpin> BoundedLineReader<R> {
    /// Build a reader that caps one line at `max` bytes.
    pub fn new(inner: R, max: usize) -> Self {
        Self {
            reader: BufReader::new(inner),
            max,
        }
    }

    /// Read one line, up to `max` bytes.
    ///
    /// A longer line is read to its newline and reported as `TooLong`, so the
    /// host never buffers an enormous line. The reader scans a filled buffer
    /// rather than one byte at a time, so a large line does not cost one syscall
    /// per byte.
    pub async fn next_line(&mut self) -> LineOutcome {
        let mut buffer: Vec<u8> = Vec::new();
        let mut overflow = false;
        loop {
            let available = match self.reader.fill_buf().await {
                Ok(bytes) => bytes,
                Err(_) => return LineOutcome::Eof,
            };
            if available.is_empty() {
                if buffer.is_empty() && !overflow {
                    return LineOutcome::Eof;
                }
                if overflow {
                    return LineOutcome::TooLong;
                }
                return LineOutcome::Line(String::from_utf8_lossy(&buffer).into_owned());
            }
            match available.iter().position(|&b| b == b'\n') {
                Some(pos) => {
                    if !overflow {
                        append_capped(&mut buffer, &available[..pos], self.max, &mut overflow);
                    }
                    self.reader.consume(pos + 1);
                    if overflow {
                        return LineOutcome::TooLong;
                    }
                    if buffer.last() == Some(&b'\r') {
                        buffer.pop();
                    }
                    return LineOutcome::Line(String::from_utf8_lossy(&buffer).into_owned());
                }
                None => {
                    let len = available.len();
                    if !overflow {
                        let chunk = available.to_vec();
                        append_capped(&mut buffer, &chunk, self.max, &mut overflow);
                    }
                    self.reader.consume(len);
                }
            }
        }
    }
}

/// Append `chunk` to `buffer` up to `max` bytes. Set `overflow` and free the
/// buffer when the cap is crossed, so the host never holds an enormous line.
fn append_capped(buffer: &mut Vec<u8>, chunk: &[u8], max: usize, overflow: &mut bool) {
    if buffer.len() + chunk.len() <= max {
        buffer.extend_from_slice(chunk);
    } else {
        *overflow = true;
        *buffer = Vec::new();
    }
}
