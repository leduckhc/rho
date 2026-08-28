# MCP, driven for real — the client works and the tools never arrive

> **Fixed on branch `feat/wire-the-dead-switches`.**
> `drain_connects` now awaits every connect task before the process exits, so the
> cache write completes on a short run. The notice no longer repeats.
> See `docs/verification/wiring-sprint.md` for the re-probe.
> The record below describes the pre-fix state.

Date: 2026-08-23. Binary: `target/release/rho`, version 0.1.0, default features.

`docs/verification/sprint-1.md` had checked one MCP case: a server that does not start does
not stop the session. Nobody had ever watched rho talk to a **working** server. Writing
`docs/guide/mcp.md` needed that, so this is it.

## The probe server

A minimal stdio MCP server in Python, at `/tmp/mcp-probe/server.py` for this run. It logs
every message it receives. It offers one tool, `echo_upper`. It echoes back whichever
protocol version the client asks for, so a version mismatch cannot explain a failure.

```json
{
  "servers": [
    {
      "name": "probe",
      "transport": { "type": "stdio", "command": "python3", "args": ["/tmp/mcp-probe/server.py"] }
    }
  ]
}
```

## Run 1

```sh
rho run "Use the echo_upper tool on the word hello. Report only its output." \
  --mcp-config /tmp/mcp-probe/mcp.json --no-skills
```

rho printed:

```
rho: 1 MCP server(s) are configured, and no tool schema is cached yet. Their tools appear in the next session.
```

The model answered that it had no such tool, listed the built-in set, and used `bash` instead.

The server log shows the handshake completed:

```
IN  {"id":1,"method":"initialize","params":{"capabilities":{},"clientInfo":{"name":"rho","version":"0.1.0"},"protocolVersion":"2025-06-18"}}
OUT {"id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"probe","version":"0.1.0"}}}
IN  {"method":"notifications/initialized","params":{}}
IN  {"id":2,"method":"tools/list","params":{}}
OUT {"id":2,"result":{"tools":[{"name":"echo_upper", ...}]}}
```

So rho starts the process, sends a correct `initialize`, sends `notifications/initialized`,
and asks for `tools/list`. **The client half works.** That is new information, and it is
worth having.

## Run 2

The same command again. The notice was identical, and the model again reported no such tool.
The server log holds **zero** `tools/call` entries across both runs.

## The cause

`tools_for` advertises tools from `McpSchemaCache`, and the cache is empty, so it advertises
nothing. `McpSchemaCache::save` exists in `crates/rho-mcp/src/cache.rs:74` and **no code
calls it**. A grep across `crates/*/src` finds the definition and no caller. So
`~/.rho/mcp-schema-cache.json` is never written, and `~/.rho/` held only `scratch/` after
both runs.

The notice therefore promises a next session that cannot differ.

## What this means

An MCP tool cannot reach the model through the `rho` binary today. The transport, the
handshake, the config parsing, the naming rule, and the pool are all real. The last hop is
missing.

`docs/guide/mcp.md` opens with this, and `docs/guide/status.md` lists it under what is not
built.

## What was not covered

- The HTTP transport. Only stdio was driven.
- A tool call. It could not be reached, since no tool was ever advertised.
- The name collision path, and the `shared` flag.
