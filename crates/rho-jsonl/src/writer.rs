//! The single writer for stdout.
//!
//! Every reply and every event goes through one `Writer`. A clone shares the same
//! stream behind a lock, so two lines never interleave and a reader on the far side
//! may assume every line is whole. See SPEC-jsonl-frontend section 5.

use std::sync::Arc;

use serde::Serialize;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::Mutex;

use crate::protocol::{Event, Reply};

/// Writes one JSON line at a time to the output stream.
///
/// A lock guards the stream, not a channel. A channel would need a task, and a task
/// that dies leaves a writer that silently drops lines. A lock cannot do that.
pub struct Writer<W> {
    inner: Arc<Mutex<W>>,
}

impl<W> Clone for Writer<W> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<W: AsyncWrite + Unpin> Writer<W> {
    /// Take ownership of the output stream.
    pub fn new(output: W) -> Self {
        Self {
            inner: Arc::new(Mutex::new(output)),
        }
    }

    /// Write one value as a JSON line, then flush.
    ///
    /// It flushes every line. A frontend that buffered would leave a client waiting
    /// for a reply that is already written, which reads as a hang.
    pub async fn line<T: Serialize>(&self, value: &T) -> std::io::Result<()> {
        // Serialise before taking the lock, so a serialisation failure cannot hold it.
        let mut bytes = serde_json::to_vec(value).map_err(std::io::Error::other)?;
        // Exactly one LF, and never a CR. See SPEC-jsonl-frontend section 4.
        bytes.push(b'\n');
        let mut guard = self.inner.lock().await;
        guard.write_all(&bytes).await?;
        guard.flush().await
    }

    /// Write one reply.
    pub async fn reply(&self, reply: &Reply) -> std::io::Result<()> {
        self.line(reply).await
    }

    /// Write one event.
    pub async fn event(&self, event: &Event) -> std::io::Result<()> {
        self.line(event).await
    }
}
