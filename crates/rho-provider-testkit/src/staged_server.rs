//! A staged HTTP server that proves a client streams and does not buffer.
//!
//! The server sends the response head, flushes it, waits, then sends the tail.
//! A streaming client yields the first event before the tail arrives. A client
//! that buffers the whole body blocks until the tail arrives. A bounded timeout
//! in the test then fails the buffering client.
//!
//! The server speaks minimal HTTP/1.1. It answers one request, then closes.

use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// A one-shot HTTP/1.1 server for the buffering test.
pub struct StagedHttpServer {
    /// The base URL a provider must target, for example `http://127.0.0.1:PORT`.
    base_url: String,
    handle: JoinHandle<()>,
}

impl StagedHttpServer {
    /// Start the server. It sends `head`, flushes, waits `delay`, sends `tail`.
    ///
    /// The body is an `text/event-stream`. The connection closes at the end, so
    /// the client reads to end of file.
    pub async fn start(head: Vec<u8>, tail: Vec<u8>, delay: Duration) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let base_url = format!("http://{addr}");
        let handle = tokio::spawn(async move {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            // Drain the request head. The content does not matter here.
            let mut buf = [0u8; 2048];
            let _ = socket.read(&mut buf).await;
            let response_head =
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
            if socket.write_all(response_head).await.is_err() {
                return;
            }
            if socket.write_all(&head).await.is_err() {
                return;
            }
            let _ = socket.flush().await;
            tokio::time::sleep(delay).await;
            let _ = socket.write_all(&tail).await;
            let _ = socket.flush().await;
        });
        Ok(Self { base_url, handle })
    }

    /// The base URL to target.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

impl Drop for StagedHttpServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}
