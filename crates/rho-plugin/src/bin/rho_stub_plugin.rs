//! A stub plugin for the `rho-plugin` host tests.
//!
//! It speaks the SPEC-04 stdio JSON-RPC protocol, one JSON object per line. The
//! first command-line argument selects a behaviour, so one binary drives every
//! host test: the normal path, a crash, a hang, a malformed line, and an
//! enormous line. No test reaches the network.

use std::io::{BufRead, Write};

fn main() {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "normal".to_string());
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let method = message["method"].as_str().unwrap_or("");
        let id = message["id"].clone();

        match method {
            "initialize" => {
                if mode == "garbage" {
                    // A non-JSON line before the handshake. The host must drop it.
                    writeln!(out, "this line is not json at all").unwrap();
                    out.flush().unwrap();
                }
                let response = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": 1,
                        "plugin": { "name": "stub", "version": "0.1.0" },
                        "tools": [{
                            "name": "echo",
                            "description": "Echo the text argument back.",
                            "kind": "other",
                            "inputSchema": {
                                "type": "object",
                                "properties": { "text": { "type": "string" } },
                                "required": ["text"]
                            }
                        }]
                    }
                });
                writeln!(out, "{response}").unwrap();
                out.flush().unwrap();
            }
            "call_tool" => {
                let call_id = message["params"]["callId"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                match mode.as_str() {
                    // Die in the middle of the call. The host must not panic.
                    "crash" => std::process::exit(1),
                    // Never answer. The host must time out or cancel.
                    "hang" => loop {
                        std::thread::sleep(std::time::Duration::from_secs(3600));
                    },
                    // Write an enormous line, then the real result. The host must
                    // cap the line and still read the result.
                    "bigline" => {
                        let big = "z".repeat(20_000_000);
                        writeln!(out, "{big}").unwrap();
                        out.flush().unwrap();
                        write_result(&mut out, &id, &call_id, &message);
                    }
                    _ => {
                        // Stream one update, then return the result.
                        let update = serde_json::json!({
                            "jsonrpc": "2.0",
                            "method": "tool_update",
                            "params": { "callId": call_id, "output": "working" }
                        });
                        writeln!(out, "{update}").unwrap();
                        out.flush().unwrap();
                        write_result(&mut out, &id, &call_id, &message);
                    }
                }
            }
            "cancel" => {
                // A real plugin stops the call here. The stub just acknowledges by
                // doing nothing, so the host drives the outcome.
            }
            "shutdown" => {
                let response = serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": {} });
                writeln!(out, "{response}").unwrap();
                out.flush().unwrap();
                break;
            }
            _ => {}
        }
    }
}

/// Write a normal tool result that echoes the `text` argument.
fn write_result<W: Write>(
    out: &mut W,
    id: &serde_json::Value,
    _call_id: &str,
    request: &serde_json::Value,
) {
    let text = request["params"]["arguments"]["text"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let response = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{ "type": "text", "text": text }],
            "isError": false
        }
    });
    writeln!(out, "{response}").unwrap();
    out.flush().unwrap();
}
