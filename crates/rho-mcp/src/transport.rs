//! The transport layer.
//!
//! JSON-RPC 2.0 runs over a transport. rho-mcp implements the stdio transport,
//! where rho spawns the server. rho-mcp must not link an HTTP client, because
//! `rho-core` stays HTTP-free and rho-mcp sits beside it. So the HTTP transport
//! is a trait a caller supplies, and `rho-cli` passes an implementation backed
//! by the `reqwest` already present in the provider crates. See `SPEC-09`
//! section 3.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::process::ChildStdin;

use crate::config::{McpServerConfig, McpTransport};
use crate::error::McpError;
use crate::limits::McpLimits;
use crate::line::{BoundedLineReader, LineOutcome};

/// The reading half of a transport. It yields one bounded line at a time.
#[async_trait]
pub trait TransportReader: Send {
    /// Read the next line from the server.
    async fn next_line(&mut self) -> LineOutcome;
}

/// The writing half of a transport. It sends one line at a time.
#[async_trait]
pub trait TransportWriter: Send + Sync {
    /// Send one line to the server. The caller adds no newline.
    async fn send_line(&self, line: &str) -> Result<(), McpError>;
    /// Close the writing half, so the server sees the end of input.
    async fn close(&self);
}

/// An open transport, split into its halves plus a process guard.
///
/// The connection owns `guard`. When the connection drops, the guard drops. For
/// a stdio server the guard is the child process with kill on drop, so the last
/// reference stops the server and leaves no orphan. See `SPEC-09` section 6.
pub struct TransportPair {
    pub reader: Box<dyn TransportReader>,
    pub writer: Arc<dyn TransportWriter>,
    pub guard: Box<dyn Send>,
}

/// A factory that opens a transport for one server config.
///
/// The default factory opens a stdio server. A caller supplies its own factory
/// to add the HTTP transport, because rho-mcp links no HTTP client.
#[async_trait]
pub trait TransportFactory: Send + Sync {
    async fn open(
        &self,
        config: &McpServerConfig,
        limits: &McpLimits,
    ) -> Result<TransportPair, McpError>;
}

/// The default factory. It opens a stdio server, and refuses HTTP with a clear
/// message that names the server and says how to wire HTTP.
pub struct DefaultTransportFactory;

#[async_trait]
impl TransportFactory for DefaultTransportFactory {
    async fn open(
        &self,
        config: &McpServerConfig,
        limits: &McpLimits,
    ) -> Result<TransportPair, McpError> {
        match &config.transport {
            McpTransport::Stdio { command, args } => {
                open_stdio(&config.name, command, args, config, limits)
            }
            McpTransport::Http { .. } => Err(McpError::HttpTransportNotWired {
                server: config.name.clone(),
            }),
        }
    }
}

/// Spawn a stdio server and build its transport pair.
fn open_stdio(
    server: &str,
    command: &str,
    args: &[String],
    config: &McpServerConfig,
    limits: &McpLimits,
) -> Result<TransportPair, McpError> {
    let mut cmd = tokio::process::Command::new(command);
    cmd.args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);

    // Scrub every variable whose name looks like a credential, then add the
    // configured environment back. A configured server often needs a token, so
    // the config names it. Inheriting the whole environment is what we refuse.
    scrub_credentials(&mut cmd);
    for (key, value) in &config.env {
        cmd.env(key, value);
    }

    // Run the server in its own process group, so releasing the last reference
    // kills the group and leaves no orphan.
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }

    let mut child = cmd.spawn().map_err(|error| McpError::Launch {
        server: server.to_string(),
        reason: error.to_string(),
    })?;

    let stdin = child.stdin.take().ok_or_else(|| McpError::Launch {
        server: server.to_string(),
        reason: "the server has no stdin pipe".to_string(),
    })?;
    let stdout = child.stdout.take().ok_or_else(|| McpError::Launch {
        server: server.to_string(),
        reason: "the server has no stdout pipe".to_string(),
    })?;

    let reader = Box::new(StdioReader {
        inner: BoundedLineReader::new(stdout, limits.max_line_bytes),
    });
    let writer: Arc<dyn TransportWriter> = Arc::new(StdioWriter {
        stdin: tokio::sync::Mutex::new(Some(stdin)),
        server: server.to_string(),
    });
    Ok(TransportPair {
        reader,
        writer,
        guard: Box::new(child),
    })
}

/// The stdio reading half.
struct StdioReader {
    inner: BoundedLineReader<tokio::process::ChildStdout>,
}

#[async_trait]
impl TransportReader for StdioReader {
    async fn next_line(&mut self) -> LineOutcome {
        self.inner.next_line().await
    }
}

/// The stdio writing half.
struct StdioWriter {
    stdin: tokio::sync::Mutex<Option<ChildStdin>>,
    server: String,
}

#[async_trait]
impl TransportWriter for StdioWriter {
    async fn send_line(&self, line: &str) -> Result<(), McpError> {
        let mut guard = self.stdin.lock().await;
        let stdin = guard.as_mut().ok_or_else(|| McpError::Unavailable {
            server: self.server.clone(),
        })?;
        let unavailable = || McpError::Unavailable {
            server: self.server.clone(),
        };
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|_| unavailable())?;
        stdin.write_all(b"\n").await.map_err(|_| unavailable())?;
        stdin.flush().await.map_err(|_| unavailable())?;
        Ok(())
    }

    async fn close(&self) {
        let mut guard = self.stdin.lock().await;
        if let Some(mut stdin) = guard.take() {
            let _ = stdin.shutdown().await;
        }
    }
}

/// Remove every variable whose name looks like a credential from `cmd`.
///
/// The filter reads the name, not the value, because a value cannot be
/// recognised reliably. It removes the name as well, so the presence of a key
/// leaks nothing. This is defence in depth, not a boundary: a server can still
/// read a credential file the user can read. See decision D-019.
fn scrub_credentials(cmd: &mut tokio::process::Command) {
    for (name, _) in std::env::vars_os() {
        let text = name.to_string_lossy().to_ascii_uppercase();
        if rho_redact::looks_like_a_secret(&text) {
            cmd.env_remove(&name);
        }
    }
}
