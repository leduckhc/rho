//! One launched plugin subprocess and its JSON-RPC engine.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use rho_core::{CancelToken, ContentBlock, ToolOutput};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{mpsc, oneshot};

use crate::cache::PluginToolSpec;
use crate::host::PluginError;

/// The largest line the host reads from a plugin, in bytes. A longer line is
/// dropped, so a plugin cannot exhaust host memory with one enormous line.
const MAX_LINE_BYTES: usize = 1_000_000;

/// The default per-call timeout.
pub(crate) const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_millis(120_000);

/// The handshake timeout. It is fixed and generous, so a short per-call timeout
/// set for a test never makes a valid handshake fail under load.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(10_000);

/// Shared routing state between the reader task and the caller.
struct Shared {
    /// Pending requests, keyed by JSON-RPC id. A response wakes the waiter.
    pending: Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>,
    /// Active call update sinks, keyed by call id. A `tool_update` routes here.
    updates: Mutex<HashMap<String, mpsc::Sender<String>>>,
    /// False once the plugin process is gone. A later call fails fast.
    available: AtomicBool,
}

/// A launched plugin subprocess.
pub struct PluginProcess {
    name: String,
    tools: Vec<PluginToolSpec>,
    stdin: tokio::sync::Mutex<ChildStdin>,
    child: tokio::sync::Mutex<Child>,
    shared: Arc<Shared>,
    next_id: AtomicU64,
    call_timeout: Duration,
}

impl PluginProcess {
    /// Launch a plugin, run the handshake, and read its tool list.
    pub(crate) async fn launch(
        command: &str,
        args: &[String],
        call_timeout: Duration,
    ) -> Result<Arc<Self>, PluginError> {
        let mut child = tokio::process::Command::new(command)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| PluginError::Launch(error.to_string()))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| PluginError::Launch("the plugin has no stdin pipe".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| PluginError::Launch("the plugin has no stdout pipe".to_string()))?;

        let shared = Arc::new(Shared {
            pending: Mutex::new(HashMap::new()),
            updates: Mutex::new(HashMap::new()),
            available: AtomicBool::new(true),
        });

        // The reader task routes every line from the plugin. It ends when the
        // plugin closes stdout, which happens when the process exits.
        tokio::spawn(read_loop(BufReader::new(stdout), Arc::clone(&shared)));

        let stdin = tokio::sync::Mutex::new(stdin);
        let next_id = AtomicU64::new(1);

        // The handshake. Send `initialize`, then read the plugin and tool list.
        let id = next_id.fetch_add(1, Ordering::SeqCst);
        let result = send_request(
            &stdin,
            &shared,
            id,
            "initialize",
            serde_json::json!({
                "protocolVersion": 1,
                "host": { "name": "rho", "version": "0.1.0" }
            }),
            HANDSHAKE_TIMEOUT,
        )
        .await
        .map_err(|error| PluginError::Handshake(error.to_string()))?;

        let name = result["plugin"]["name"]
            .as_str()
            .ok_or_else(|| PluginError::Handshake("the plugin sent no name".to_string()))?
            .to_string();
        let tools: Vec<PluginToolSpec> =
            serde_json::from_value(result["tools"].clone()).map_err(|error| {
                PluginError::Handshake(format!("the tool list is not valid: {error}"))
            })?;

        Ok(Arc::new(Self {
            name,
            tools,
            stdin,
            child: tokio::sync::Mutex::new(child),
            shared,
            next_id,
            call_timeout,
        }))
    }

    /// The plugin name from the handshake.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The tools the plugin advertised in the handshake.
    pub fn tools(&self) -> &[PluginToolSpec] {
        &self.tools
    }

    /// True while the plugin process is usable.
    pub fn is_available(&self) -> bool {
        self.shared.available.load(Ordering::SeqCst)
    }

    /// Call a plugin tool. Forward update lines to `updates`. Select against
    /// `cancel`. A crash, a timeout, or a closed pipe returns a `PluginError`.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
        updates: mpsc::Sender<String>,
        cancel: CancelToken,
    ) -> Result<ToolOutput, PluginError> {
        if !self.is_available() {
            return Err(PluginError::Unavailable);
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let call_id = format!("call_{id}");
        let (tx, rx) = oneshot::channel();
        self.shared.pending.lock().unwrap().insert(id, tx);
        self.shared
            .updates
            .lock()
            .unwrap()
            .insert(call_id.clone(), updates);

        let message = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "call_tool",
            "params": { "callId": call_id, "name": name, "arguments": arguments }
        });
        if let Err(error) = write_line(&self.stdin, &message).await {
            self.cleanup_call(id, &call_id);
            return Err(error);
        }

        let outcome = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                // Tell the plugin to stop, then stop waiting.
                let _ = write_line(
                    &self.stdin,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "cancel",
                        "params": { "callId": call_id }
                    }),
                )
                .await;
                self.cleanup_call(id, &call_id);
                return Err(PluginError::Canceled);
            }
            result = tokio::time::timeout(self.call_timeout, rx) => result,
        };

        self.shared.updates.lock().unwrap().remove(&call_id);
        match outcome {
            Ok(Ok(value)) => parse_tool_output(&value),
            Ok(Err(_)) => Err(PluginError::Unavailable),
            Err(_) => {
                self.shared.pending.lock().unwrap().remove(&id);
                Err(PluginError::Timeout)
            }
        }
    }

    /// Shut down the plugin. Send `shutdown`, close stdin, wait a grace period,
    /// then kill the process. No orphan is left behind.
    pub async fn shutdown(&self) {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let _ = send_request(
            &self.stdin,
            &self.shared,
            id,
            "shutdown",
            serde_json::json!({}),
            Duration::from_millis(2000),
        )
        .await;
        {
            // Drop stdin so the plugin sees end of input.
            let mut stdin = self.stdin.lock().await;
            let _ = stdin.shutdown().await;
        }
        let mut child = self.child.lock().await;
        // Wait a short grace period for a clean exit, then kill.
        if tokio::time::timeout(Duration::from_millis(2000), child.wait())
            .await
            .is_err()
        {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
        self.shared.available.store(false, Ordering::SeqCst);
    }

    fn cleanup_call(&self, id: u64, call_id: &str) {
        self.shared.pending.lock().unwrap().remove(&id);
        self.shared.updates.lock().unwrap().remove(call_id);
    }
}

/// Send a request and await its response, bounded by `timeout`.
async fn send_request(
    stdin: &tokio::sync::Mutex<ChildStdin>,
    shared: &Shared,
    id: u64,
    method: &str,
    params: serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value, PluginError> {
    if !shared.available.load(Ordering::SeqCst) {
        return Err(PluginError::Unavailable);
    }
    let (tx, rx) = oneshot::channel();
    shared.pending.lock().unwrap().insert(id, tx);

    let message = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    });
    write_line(stdin, &message).await?;

    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(value)) => Ok(value),
        // The sender dropped. The reader task ended, so the plugin is gone.
        Ok(Err(_)) => Err(PluginError::Unavailable),
        Err(_) => {
            shared.pending.lock().unwrap().remove(&id);
            Err(PluginError::Timeout)
        }
    }
}

/// Write one JSON message as an LF-framed line to the plugin stdin.
async fn write_line(
    stdin: &tokio::sync::Mutex<ChildStdin>,
    message: &serde_json::Value,
) -> Result<(), PluginError> {
    let mut line =
        serde_json::to_string(message).map_err(|error| PluginError::Protocol(error.to_string()))?;
    line.push('\n');
    let mut guard = stdin.lock().await;
    guard
        .write_all(line.as_bytes())
        .await
        .map_err(|_| PluginError::Unavailable)?;
    guard.flush().await.map_err(|_| PluginError::Unavailable)?;
    Ok(())
}

/// Map a JSON-RPC result value to a `ToolOutput`.
fn parse_tool_output(value: &serde_json::Value) -> Result<ToolOutput, PluginError> {
    let content = value["content"]
        .as_array()
        .ok_or_else(|| PluginError::Protocol("the result has no content array".to_string()))?;
    let mut blocks = Vec::new();
    for item in content {
        if item["type"] == "text" {
            blocks.push(ContentBlock::Text {
                text: item["text"].as_str().unwrap_or_default().to_string(),
            });
        }
    }
    Ok(ToolOutput {
        content: blocks,
        is_error: value["isError"].as_bool().unwrap_or(false),
    })
}

/// The reader task. It reads one bounded line at a time and routes it. It never
/// panics on bad input. It ends when the plugin closes stdout.
async fn read_loop<R>(mut reader: BufReader<R>, shared: Arc<Shared>)
where
    R: tokio::io::AsyncRead + Unpin,
{
    loop {
        match next_line(&mut reader, MAX_LINE_BYTES).await {
            LineOutcome::Line(line) => route_line(&line, &shared),
            // An enormous line is dropped, not fatal.
            LineOutcome::TooLong => continue,
            LineOutcome::Eof => break,
        }
    }
    // The plugin is gone. Mark it unavailable and drop every pending waiter, so a
    // blocked call fails fast instead of hanging.
    shared.available.store(false, Ordering::SeqCst);
    shared.pending.lock().unwrap().clear();
    shared.updates.lock().unwrap().clear();
}

/// The result of a bounded line read.
enum LineOutcome {
    Line(String),
    TooLong,
    Eof,
}

/// Read one line, up to `max` bytes. A longer line is discarded to its newline
/// and reported as `TooLong`, so the host never buffers an enormous line. It
/// reads in buffered chunks, so a large line does not cost one syscall per byte.
async fn next_line<R>(reader: &mut BufReader<R>, max: usize) -> LineOutcome
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buffer: Vec<u8> = Vec::new();
    let mut overflow = false;
    loop {
        let available = match reader.fill_buf().await {
            Ok(bytes) => bytes,
            Err(_) => return LineOutcome::Eof,
        };
        if available.is_empty() {
            // End of input.
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
                    append_capped(&mut buffer, &available[..pos], max, &mut overflow);
                }
                reader.consume(pos + 1);
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
                    append_capped(&mut buffer, &chunk, max, &mut overflow);
                }
                reader.consume(len);
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

/// Route one parsed line to a pending waiter or an update sink. A malformed line
/// is dropped, not fatal.
fn route_line(line: &str, shared: &Shared) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }
    let Ok(message) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        // A non-JSON line is logged and dropped. The host stays usable.
        tracing::debug!("dropping a non-JSON plugin line");
        return;
    };

    // A response carries an id and a `result` or `error`.
    if let Some(id) = message["id"].as_u64()
        && (message.get("result").is_some() || message.get("error").is_some())
    {
        if let Some(tx) = shared.pending.lock().unwrap().remove(&id) {
            let value = message
                .get("result")
                .cloned()
                .unwrap_or_else(|| message["error"].clone());
            let _ = tx.send(value);
        }
        return;
    }

    // A `tool_update` notification carries a call id and an output line.
    if message["method"] == "tool_update" {
        let call_id = message["params"]["callId"].as_str().unwrap_or_default();
        let output = message["params"]["output"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if let Some(sink) = shared.updates.lock().unwrap().get(call_id) {
            let _ = sink.try_send(output);
        }
    }
}
