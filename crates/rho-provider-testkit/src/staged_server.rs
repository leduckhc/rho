//! A staged HTTP server that proves a client streams and does not buffer.
//!
//! The server sends the response head, flushes it, waits, then sends the tail.
//! A streaming client yields the first event before the tail arrives. A client
//! that buffers the whole body blocks until the tail arrives. A bounded timeout
//! in the test then fails the buffering client.
//!
//! The server speaks minimal HTTP/1.1.
//!
//! **It serves every connection, not one.** A provider may retry, and a retry needs a
//! peer. The first version accepted one connection and then ended its task, so a retry
//! landed in the kernel backlog with nobody to accept it. The client then waited for its
//! own timeout, and the test failed at the point where the stream starts. That made the
//! provider contract suite fail about three runs in eight. A test double that cannot
//! answer twice is a test double that lies about the client.
//!
//! **It reads the whole request before it answers.** One `read` returns whatever one
//! segment carried. A client that writes its head and its body separately could still be
//! writing when the server answered and closed, and the client then saw a reset instead
//! of a response.

use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// A staged HTTP/1.1 server for the buffering test.
pub struct StagedHttpServer {
    /// The base URL a provider must target, for example `http://127.0.0.1:PORT`.
    base_url: String,
    handle: JoinHandle<()>,
}

impl StagedHttpServer {
    /// Start the server. Every connection gets `head`, a flush, a `delay`, then `tail`.
    ///
    /// The body is a `text/event-stream`. The connection closes at the end, so the client
    /// reads to end of file.
    ///
    /// Keep `delay` well above the window the test asserts, and well below the client
    /// timeout. A delay longer than the client timeout turns a failure into a long wait.
    pub async fn start(head: Vec<u8>, tail: Vec<u8>, delay: Duration) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let base_url = format!("http://{addr}");
        let handle = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    return;
                };
                let head = head.clone();
                let tail = tail.clone();
                // One task per connection, so a retry never waits behind a sleeping peer.
                tokio::spawn(async move {
                    serve(socket, head, tail, delay).await;
                });
            }
        });
        Ok(Self { base_url, handle })
    }

    /// The base URL to target.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

/// Read one request in full, then send the head, wait, and send the tail.
async fn serve(mut socket: TcpStream, head: Vec<u8>, tail: Vec<u8>, delay: Duration) {
    if read_request(&mut socket).await.is_err() {
        return;
    }
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
}

/// Read the request head, and the body that `Content-Length` names.
///
/// A single `read` returns one segment, and a request can span several. So this reads
/// until the head terminator arrives, then reads exactly the body length. A server that
/// answers early can reset a client that is still writing.
async fn read_request(socket: &mut TcpStream) -> std::io::Result<()> {
    let mut buffer: Vec<u8> = Vec::with_capacity(4096);
    let mut chunk = [0u8; 1024];
    let head_end = loop {
        if let Some(position) = find_head_end(&buffer) {
            break position;
        }
        let read = socket.read(&mut chunk).await?;
        if read == 0 {
            // The peer closed before it finished the head. Nothing to answer.
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        }
        buffer.extend_from_slice(&chunk[..read]);
        // A head this large is not a real request from a provider under test.
        if buffer.len() > 64 * 1024 {
            return Err(std::io::Error::from(std::io::ErrorKind::InvalidData));
        }
    };

    let length = content_length(&buffer[..head_end]).unwrap_or(0);
    let mut body = buffer.len().saturating_sub(head_end);
    while body < length {
        let read = socket.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        body += read;
    }
    Ok(())
}

/// The index just past the `\r\n\r\n` that ends a request head.
fn find_head_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
}

/// The `Content-Length` value in a request head, when it states one.
fn content_length(head: &[u8]) -> Option<usize> {
    let text = String::from_utf8_lossy(head);
    for line in text.lines() {
        let mut parts = line.splitn(2, ':');
        let name = parts.next()?.trim();
        if name.eq_ignore_ascii_case("content-length") {
            return parts.next()?.trim().parse().ok();
        }
    }
    None
}

impl Drop for StagedHttpServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}
