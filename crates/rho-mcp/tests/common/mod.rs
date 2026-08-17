//! Shared test helpers for `rho-mcp` integration tests.
//!
//! No test reaches the network. A fake transport answers the handshake in
//! memory, so a background connect finishes without a process.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use rho_mcp::{
    LineOutcome, McpError, McpLimits, McpServerConfig, McpTransport, TransportFactory,
    TransportPair, TransportReader, TransportWriter,
};
use tokio::sync::mpsc;

/// The stub MCP server binary path, provided by cargo for this crate.
pub const STUB: &str = env!("CARGO_BIN_EXE_rho_stub_mcp_server");

/// Short limits, so a hang fails the test fast instead of stalling the suite.
pub fn test_limits() -> McpLimits {
    McpLimits {
        call_timeout_ms: 800,
        connect_timeout_ms: 2000,
        max_line_bytes: 1_000_000,
        max_output_bytes: 1_000_000,
        max_schema_bytes: 100_000,
        max_tools_per_server: 256,
    }
}

/// A stdio config that runs the stub in `mode`.
pub fn stub_config(name: &str, mode: &str) -> McpServerConfig {
    McpServerConfig {
        name: name.to_string(),
        transport: McpTransport::Stdio {
            command: STUB.to_string(),
            args: vec![mode.to_string()],
        },
        env: BTreeMap::new(),
        shared: true,
        call_timeout_ms: None,
    }
}

/// A fake transport factory. It answers the handshake in memory and lists no
/// tools, so a background connect finishes at once.
pub struct FakeFactory;

#[async_trait]
impl TransportFactory for FakeFactory {
    async fn open(
        &self,
        _config: &McpServerConfig,
        _limits: &McpLimits,
    ) -> Result<TransportPair, McpError> {
        let (tx, rx) = mpsc::unbounded_channel();
        Ok(TransportPair {
            reader: Box::new(FakeReader { rx }),
            writer: Arc::new(FakeWriter { tx }),
            guard: Box::new(()),
        })
    }
}

struct FakeReader {
    rx: mpsc::UnboundedReceiver<String>,
}

#[async_trait]
impl TransportReader for FakeReader {
    async fn next_line(&mut self) -> LineOutcome {
        match self.rx.recv().await {
            Some(line) => LineOutcome::Line(line),
            None => LineOutcome::Eof,
        }
    }
}

struct FakeWriter {
    tx: mpsc::UnboundedSender<String>,
}

#[async_trait]
impl TransportWriter for FakeWriter {
    async fn send_line(&self, line: &str) -> Result<(), McpError> {
        let message: serde_json::Value = serde_json::from_str(line).unwrap();
        let Some(id) = message.get("id").cloned() else {
            return Ok(());
        };
        let result = match message["method"].as_str().unwrap_or("") {
            "initialize" => serde_json::json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "serverInfo": { "name": "fake", "version": "0.1.0" }
            }),
            "tools/list" => serde_json::json!({ "tools": [] }),
            _ => serde_json::json!({ "content": [], "isError": false }),
        };
        let response = serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
        let _ = self.tx.send(response.to_string());
        Ok(())
    }

    async fn close(&self) {}
}

/// A factory whose call is recorded, so a test can prove a call never reached the
/// server. It answers the handshake, then records every `tools/call`.
pub struct RecordingFactory {
    pub calls: Arc<AtomicUsize>,
}

#[async_trait]
impl TransportFactory for RecordingFactory {
    async fn open(
        &self,
        _config: &McpServerConfig,
        _limits: &McpLimits,
    ) -> Result<TransportPair, McpError> {
        let (tx, rx) = mpsc::unbounded_channel();
        Ok(TransportPair {
            reader: Box::new(FakeReader { rx }),
            writer: Arc::new(RecordingWriter {
                tx,
                calls: Arc::clone(&self.calls),
            }),
            guard: Box::new(()),
        })
    }
}

struct RecordingWriter {
    tx: mpsc::UnboundedSender<String>,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl TransportWriter for RecordingWriter {
    async fn send_line(&self, line: &str) -> Result<(), McpError> {
        let message: serde_json::Value = serde_json::from_str(line).unwrap();
        if message["method"] == "tools/call" {
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
        let Some(id) = message.get("id").cloned() else {
            return Ok(());
        };
        let result = match message["method"].as_str().unwrap_or("") {
            "initialize" => serde_json::json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "serverInfo": { "name": "fake", "version": "0.1.0" }
            }),
            "tools/list" => serde_json::json!({ "tools": [] }),
            _ => serde_json::json!({ "content": [], "isError": false }),
        };
        let response = serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
        let _ = self.tx.send(response.to_string());
        Ok(())
    }

    async fn close(&self) {}
}
