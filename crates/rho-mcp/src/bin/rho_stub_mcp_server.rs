//! A stub MCP server for the `rho-mcp` tests.
//!
//! It speaks JSON-RPC 2.0 over stdio, one JSON object per line. The first
//! command-line argument selects a behaviour, so one binary drives every test:
//! the normal path, a hang, a crash, a garbage line, an over-long line, an
//! over-large schema, too many tools, paging, an unknown version, an environment
//! dump, and an escape sequence. No test reaches the network.

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
            "initialize" => handle_initialize(&mut out, &mode, &id),
            "notifications/initialized" => {}
            "tools/list" => handle_tools_list(&mut out, &mode, &id, &message),
            "tools/call" => {
                let keep_running = handle_tools_call(&mut out, &mode, &id, &message);
                if !keep_running {
                    break;
                }
            }
            _ => {}
        }
    }
}

/// Answer `initialize`. The `badversion` mode reports a version rho rejects. The
/// `garbage` mode emits a non-JSON line first; the client must drop it.
fn handle_initialize<W: Write>(out: &mut W, mode: &str, id: &serde_json::Value) {
    if mode == "garbage" {
        writeln!(out, "this line is not json at all").unwrap();
        out.flush().unwrap();
    }
    let version = if mode == "badversion" {
        "1999-01-01"
    } else {
        "2025-06-18"
    };
    let response = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocolVersion": version,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "stub", "version": "0.1.0" }
        }
    });
    writeln!(out, "{response}").unwrap();
    out.flush().unwrap();
}

/// Answer `tools/list`. The mode selects the tool set and the paging shape.
fn handle_tools_list<W: Write>(
    out: &mut W,
    mode: &str,
    id: &serde_json::Value,
    request: &serde_json::Value,
) {
    let echo_schema = serde_json::json!({
        "type": "object",
        "properties": { "text": { "type": "string" } },
        "required": ["text"]
    });

    let result = match mode {
        "bigschema" => {
            // A schema far past the cap. The client must refuse it.
            let big = "x".repeat(500_000);
            serde_json::json!({
                "tools": [{
                    "name": "echo",
                    "description": "Echo the text.",
                    "inputSchema": { "type": "object", "properties": { "text": { "type": "string", "pad": big } } }
                }]
            })
        }
        "manytools" => {
            // More tools than the cap. The client must cap the list.
            let tools: Vec<serde_json::Value> = (0..1000)
                .map(|index| {
                    serde_json::json!({
                        "name": format!("tool{index}"),
                        "description": "A tool.",
                        "inputSchema": echo_schema
                    })
                })
                .collect();
            serde_json::json!({ "tools": tools })
        }
        "paged" => {
            let cursor = request["params"]["cursor"].as_str().unwrap_or("");
            if cursor == "page2" {
                serde_json::json!({
                    "tools": [{ "name": "second", "description": "Second.", "inputSchema": echo_schema }]
                })
            } else {
                serde_json::json!({
                    "tools": [{ "name": "first", "description": "First.", "inputSchema": echo_schema }],
                    "nextCursor": "page2"
                })
            }
        }
        _ => serde_json::json!({
            "tools": [{ "name": "echo", "description": "Echo the text argument back.", "inputSchema": echo_schema }]
        }),
    };

    let response = serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
    writeln!(out, "{response}").unwrap();
    out.flush().unwrap();
}

/// Answer `tools/call`. Return `false` when the process must stop after this.
fn handle_tools_call<W: Write>(
    out: &mut W,
    mode: &str,
    id: &serde_json::Value,
    request: &serde_json::Value,
) -> bool {
    let name = request["params"]["name"].as_str().unwrap_or("");
    let args = &request["params"]["arguments"];

    match mode {
        // Die in the middle of the call. The client must not panic, and the
        // session must survive.
        "crash" => std::process::exit(1),
        // Never answer. The client must time out.
        "hang" => true,
        // Write an over-long line, then the real result. The client must refuse
        // the line and still read the result.
        "bigline" => {
            let big = "z".repeat(5_000_000);
            writeln!(out, "{big}").unwrap();
            out.flush().unwrap();
            write_echo_result(out, id, args);
            true
        }
        // Return the value of the requested environment variable, so a test can
        // check the credential scrub and the configured environment.
        "printenv" => {
            let var = args["var"].as_str().unwrap_or("");
            let value = std::env::var(var).unwrap_or_default();
            write_text_result(out, id, &value);
            true
        }
        // Return text with an escape sequence, so a test can check sanitising.
        "escape" => {
            write_text_result(out, id, "\u{1b}[31mred\u{1b}[0m");
            true
        }
        _ => {
            if name == "echo" {
                write_echo_result(out, id, args);
            } else {
                // An unknown tool is a clear JSON-RPC error, so a cached tool that
                // no longer exists produces a clear error, not a panic.
                let response = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("no such tool: {name}") }
                });
                writeln!(out, "{response}").unwrap();
                out.flush().unwrap();
            }
            true
        }
    }
}

/// Write a result that echoes the `text` argument.
fn write_echo_result<W: Write>(out: &mut W, id: &serde_json::Value, args: &serde_json::Value) {
    let text = args["text"].as_str().unwrap_or("").to_string();
    write_text_result(out, id, &text);
}

/// Write a text tool result.
fn write_text_result<W: Write>(out: &mut W, id: &serde_json::Value, text: &str) {
    let response = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "content": [{ "type": "text", "text": text }], "isError": false }
    });
    writeln!(out, "{response}").unwrap();
    out.flush().unwrap();
}
