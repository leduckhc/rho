//! The JSON-RPC 2.0 engine for one server connection.
//!
//! The engine spawns a reader task that routes each line to a pending waiter. A
//! request awaits its response, bounded by a timeout. A crash, a closed pipe, or
//! a timeout returns a clear error, never a panic or a hang. The pattern follows
//! `rho-plugin`'s process engine; the shared shape is a genuine extraction
//! candidate, recorded in the stage report. See `SPEC-mcp` sections 3 and 4.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use rho_core::{ContentBlock, ToolOutput};
use tokio::sync::oneshot;

use crate::config::{McpServerConfig, McpToolDef};
use crate::error::McpError;
use crate::limits::McpLimits;
use crate::line::LineOutcome;
use crate::naming::validate_tool_name;
use crate::sanitize::{sanitize_output, truncate_bytes};
use crate::transport::{
    DefaultTransportFactory, TransportFactory, TransportPair, TransportReader, TransportWriter,
};

/// The protocol version rho sends in `initialize`.
const CLIENT_PROTOCOL_VERSION: &str = "2025-06-18";

/// The protocol versions rho accepts from a server.
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Routing state shared between the reader task and the callers.
struct Shared {
    /// Pending requests, keyed by JSON-RPC id. A response wakes the waiter.
    pending: Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>,
    /// False once the server process is gone. A later call fails fast.
    available: AtomicBool,
}

/// One live server connection.
pub struct McpConnection {
    server: String,
    writer: Arc<dyn TransportWriter>,
    shared: Arc<Shared>,
    next_id: AtomicU64,
    call_timeout: Duration,
    max_output_bytes: usize,
    // The process guard. Dropping the connection drops the guard, which stops the
    // server. A `Mutex` keeps the connection `Sync` while it holds a `Send` box.
    _guard: Mutex<Box<dyn Send>>,
}

impl McpConnection {
    /// True while the server process is usable.
    pub fn is_available(&self) -> bool {
        self.shared.available.load(Ordering::SeqCst)
    }

    /// Call a tool and return its output.
    ///
    /// A server error becomes an error `ToolOutput`, not an `Err`, so a failing
    /// tool does not abort the session. A timeout, a crash, or a closed pipe
    /// returns an `Err`, so the caller can turn it into an error result.
    pub async fn call(&self, tool: &str, args: serde_json::Value) -> Result<ToolOutput, McpError> {
        let value = self
            .request(
                "tools/call",
                serde_json::json!({ "name": tool, "arguments": args }),
                self.call_timeout,
                || McpError::CallTimeout {
                    server: self.server.clone(),
                },
            )
            .await?;
        self.parse_tool_output(&value)
    }

    /// Send a request and await its response, bounded by `timeout`.
    async fn request(
        &self,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
        on_timeout: impl Fn() -> McpError,
    ) -> Result<serde_json::Value, McpError> {
        if !self.is_available() {
            return Err(McpError::Unavailable {
                server: self.server.clone(),
            });
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.shared.pending.lock().unwrap().insert(id, tx);

        let message = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        let line = serde_json::to_string(&message).map_err(|error| McpError::Protocol {
            server: self.server.clone(),
            reason: error.to_string(),
        })?;
        if let Err(error) = self.writer.send_line(&line).await {
            self.shared.pending.lock().unwrap().remove(&id);
            return Err(error);
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(value)) => {
                if let Some(error) = value.get("error") {
                    // A JSON-RPC error object. Turn it into an error tool result
                    // upstream. Here it is returned as the value so `call` maps it.
                    return Ok(serde_json::json!({ "__mcp_error": error.clone() }));
                }
                Ok(value.get("result").cloned().unwrap_or(value))
            }
            // The sender dropped. The reader task ended, so the server is gone.
            Ok(Err(_)) => Err(McpError::Unavailable {
                server: self.server.clone(),
            }),
            Err(_) => {
                self.shared.pending.lock().unwrap().remove(&id);
                Err(on_timeout())
            }
        }
    }

    /// Send a notification. A notification has no id and no response.
    async fn notify(&self, method: &str, params: serde_json::Value) -> Result<(), McpError> {
        let message = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        let line = serde_json::to_string(&message).map_err(|error| McpError::Protocol {
            server: self.server.clone(),
            reason: error.to_string(),
        })?;
        self.writer.send_line(&line).await
    }

    /// Map a JSON-RPC `tools/call` result to a `ToolOutput`.
    ///
    /// The text is sanitised and capped, because a server output is untrusted.
    fn parse_tool_output(&self, value: &serde_json::Value) -> Result<ToolOutput, McpError> {
        if let Some(error) = value.get("__mcp_error") {
            let message = error["message"].as_str().unwrap_or("unknown error");
            return Ok(ToolOutput {
                content: vec![ContentBlock::Text {
                    text: sanitize_output(&format!(
                        "the MCP server {} returned an error: {message}",
                        self.server
                    )),
                }],
                is_error: true,
            });
        }
        let content = value["content"]
            .as_array()
            .ok_or_else(|| McpError::Protocol {
                server: self.server.clone(),
                reason: "a tool result has no content array".to_string(),
            })?;
        let mut blocks = Vec::new();
        for item in content {
            if item["type"] == "text" {
                let raw = item["text"].as_str().unwrap_or_default();
                let clean = truncate_bytes(&sanitize_output(raw), self.max_output_bytes);
                blocks.push(ContentBlock::Text { text: clean });
            }
        }
        Ok(ToolOutput {
            content: blocks,
            is_error: value["isError"].as_bool().unwrap_or(false),
        })
    }

    /// Shut the connection down. Close the writer, then drop the guard on drop.
    pub async fn shutdown(&self) {
        self.writer.close().await;
        self.shared.available.store(false, Ordering::SeqCst);
    }
}

/// Connect over an already-open transport, run the handshake, and list the tools.
///
/// The handshake is `initialize`, then `notifications/initialized`, then
/// `tools/list` with cursor paging. The tool list is capped and each schema is
/// checked against its cap. Each tool name is validated before it is returned.
pub async fn handshake(
    pair: TransportPair,
    config: &McpServerConfig,
    limits: &McpLimits,
) -> Result<(Arc<McpConnection>, Vec<McpToolDef>), McpError> {
    let server = config.name.clone();
    let shared = Arc::new(Shared {
        pending: Mutex::new(HashMap::new()),
        available: AtomicBool::new(true),
    });

    // The reader task routes every line. It ends when the server closes stdout.
    tokio::spawn(read_loop(pair.reader, Arc::clone(&shared)));

    let call_timeout =
        Duration::from_millis(config.call_timeout_ms.unwrap_or(limits.call_timeout_ms));
    let connection = Arc::new(McpConnection {
        server: server.clone(),
        writer: pair.writer,
        shared,
        next_id: AtomicU64::new(1),
        call_timeout,
        max_output_bytes: limits.max_output_bytes,
        _guard: Mutex::new(pair.guard),
    });

    let connect_timeout = Duration::from_millis(limits.connect_timeout_ms);

    // Step 1: initialize.
    let init = connection
        .request(
            "initialize",
            serde_json::json!({
                "protocolVersion": CLIENT_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "rho", "version": "0.1.0" }
            }),
            connect_timeout,
            || McpError::ConnectTimeout {
                server: server.clone(),
            },
        )
        .await?;

    let version = init["protocolVersion"].as_str().unwrap_or_default();
    if !SUPPORTED_PROTOCOL_VERSIONS.contains(&version) {
        return Err(McpError::UnknownProtocolVersion {
            server: server.clone(),
            version: version.to_string(),
        });
    }

    // Step 2: notifications/initialized.
    connection
        .notify("notifications/initialized", serde_json::json!({}))
        .await?;

    // Step 3: tools/list, with cursor paging.
    let mut tools: Vec<McpToolDef> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let params = match &cursor {
            Some(c) => serde_json::json!({ "cursor": c }),
            None => serde_json::json!({}),
        };
        let page = connection
            .request("tools/list", params, connect_timeout, || {
                McpError::ConnectTimeout {
                    server: server.clone(),
                }
            })
            .await?;
        let page_tools: Vec<McpToolDef> =
            serde_json::from_value(page["tools"].clone()).map_err(|error| McpError::Handshake {
                server: server.clone(),
                reason: format!("the tool list is not valid: {error}"),
            })?;
        for tool in page_tools {
            tools.push(tool);
        }
        match page["nextCursor"].as_str() {
            Some(next) if !next.is_empty() => cursor = Some(next.to_string()),
            _ => break,
        }
        if tools.len() > limits.max_tools_per_server {
            // Stop paging once past the cap. The list is truncated below.
            break;
        }
    }

    // Validate each name and each schema. Cap the number of tools.
    for tool in &tools {
        validate_tool_name(&server, &tool.name)?;
        let schema_len = serde_json::to_vec(&tool.input_schema)
            .map(|bytes| bytes.len())
            .unwrap_or(0);
        if schema_len > limits.max_schema_bytes {
            return Err(McpError::SchemaTooLarge {
                server: server.clone(),
                tool: tool.name.clone(),
                limit: limits.max_schema_bytes,
            });
        }
    }
    if tools.len() > limits.max_tools_per_server {
        tools.truncate(limits.max_tools_per_server);
    }

    Ok((connection, tools))
}

/// The reader task. It reads one bounded line at a time and routes it. It never
/// panics on bad input. It ends when the server closes stdout.
async fn read_loop(mut reader: Box<dyn TransportReader>, shared: Arc<Shared>) {
    loop {
        match reader.next_line().await {
            LineOutcome::Line(line) => route_line(&line, &shared),
            // An over-long line is refused, not fatal. Memory stays bounded.
            LineOutcome::TooLong => continue,
            LineOutcome::Eof => break,
        }
    }
    // The server is gone. Mark it unavailable and drop every pending waiter, so a
    // blocked call fails fast instead of hanging.
    shared.available.store(false, Ordering::SeqCst);
    shared.pending.lock().unwrap().clear();
}

/// Route one parsed line to a pending waiter. A malformed line is dropped.
fn route_line(line: &str, shared: &Shared) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }
    let Ok(message) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        // A non-JSON line is dropped. The client stays usable.
        tracing::debug!("dropping a non-JSON MCP line");
        return;
    };
    if let Some(id) = message["id"].as_u64()
        && (message.get("result").is_some() || message.get("error").is_some())
        && let Some(tx) = shared.pending.lock().unwrap().remove(&id)
    {
        let _ = tx.send(message);
    }
}

/// The client for one server. It owns one connection.
pub struct McpClient {
    connection: Arc<McpConnection>,
}

impl McpClient {
    /// Connect and handshake over the default (stdio) transport. Return the tool
    /// list. An HTTP server needs a supplied factory; see [`McpClient::connect_with`].
    pub async fn connect(
        config: &McpServerConfig,
        limits: McpLimits,
    ) -> Result<(Self, Vec<McpToolDef>), McpError> {
        Self::connect_with(config, limits, &DefaultTransportFactory).await
    }

    /// Connect and handshake over a supplied transport factory.
    ///
    /// A caller passes an HTTP transport factory here, because rho-mcp links no
    /// HTTP client.
    pub async fn connect_with(
        config: &McpServerConfig,
        limits: McpLimits,
        factory: &dyn TransportFactory,
    ) -> Result<(Self, Vec<McpToolDef>), McpError> {
        let pair = factory.open(config, &limits).await?;
        let (connection, tools) = handshake(pair, config, &limits).await?;
        Ok((Self { connection }, tools))
    }

    /// Call a tool by its server-side name.
    pub async fn call(&self, tool: &str, args: serde_json::Value) -> Result<ToolOutput, McpError> {
        self.connection.call(tool, args).await
    }

    /// Shut the client down. No orphan process is left behind.
    pub async fn shutdown(&self) {
        self.connection.shutdown().await;
    }
}
