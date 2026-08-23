# MCP servers

rho 0.1.0. An MCP server is a separate process that gives the model extra tools.

## Add a server

Create `~/.rho/mcp.json` and start rho:

```json
{
  "servers": [
    {
      "name": "brave-search",
      "transport": {
        "type": "stdio",
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-brave-search"]
      },
      "env": { "BRAVE_API_KEY": "your-key-here" }
    },
    {
      "name": "my-api",
      "transport": {
        "type": "http",
        "url": "https://tools.example.com/mcp",
        "headers": { "Authorization": "Bearer your-token" }
      }
    }
  ]
}
```

The first time you add a server with no cached schema, rho prints:

```
2 MCP server(s) are configured, and no tool schema is cached yet. Their tools appear in the next session.
```

Start rho again. The tools are now in the model's context.

### Field reference

JSON has no comments. This table names every field.

| Field | Required | Default | What it does |
|---|---|---|---|
| `name` | yes | — | Your label for the server. Prefixes every tool name. |
| `transport` | yes | — | How rho reaches the server. |
| `env` | no | `{}` | Extra environment variables added after credential scrubbing. |
| `shared` | no | `true` | Share one process across all sessions. |
| `call_timeout_ms` | no | `30000` | Milliseconds before a tool call times out. |

The `transport` object requires `type`. Type `stdio` also requires `command` and accepts optional `args`. Type `http` requires `url` and accepts optional `headers`.

## Tool names

Every tool reaches the model as `mcp__<server>__<tool>`. Hyphens become underscores. Server `my-search` with tool `search` becomes `mcp__my_search__search`.

MCP names start with `mcp__`. They never collide with built-in names like `read` or `bash`.

## Schema cache

rho caches tool schemas at `~/.rho/mcp-schema-cache.json`. The key is a fingerprint of the
connection: the transport, its command and arguments or its url and headers, the `env` map,
and the `shared` flag. Change one of those and rho ignores the old entry. Renaming a server
or changing `call_timeout_ms` reuses the cached schema. You never clear the cache by hand.

At startup rho connects to each server in the background. The cached schema is what the model sees on the first turn. The live list replaces it as soon as the handshake finishes.

## Environment for stdio servers

A stdio server does not inherit your full shell environment. rho removes every variable whose name looks like a credential before it spawns the process. It then adds the variables in your `env` map.

If your server needs an API key, put it in `env`. The key will not arrive from your shell. A server receives whatever non-credential variables survive the scrub, plus what you list in `env`.

## Shared servers

When `shared` is `true`, every concurrent session uses one process. That process stops when the last session ends.

When `shared` is `false`, each session spawns its own process. This adds one process per session.

## Config file path

The default is `~/.rho/mcp.json`. Three overrides exist:

- `--mcp-config <PATH>` on the command line
- `RHO_MCP_CONFIG=<PATH>` in the environment
- `mcp-config = "/path"` in a rho config file

A project config file drops `mcp-config` unless you pass `--trust-project`.

## Protocol

rho sends `2025-06-18` in `initialize`. It also accepts `2025-03-26` and `2024-11-05` from a server.

rho calls `tools/list` and `tools/call`. It does not call resources, prompts, or sampling endpoints. A server that offers those features still works. rho ignores anything outside tools.

rho validates every tool name a server reports. It rejects empty names, names with path separators, names with control characters, and names with a leading dot. It also rejects any schema larger than 100 KB. A server that sends an invalid name or an oversized schema fails at startup, not during a call.

## Troubleshooting

**The file does not parse.** rho prints:

```
cannot parse /Users/you/.rho/mcp.json: <error>. Expected {"servers": [...]}.
```

Check that the JSON is valid and the top-level key is `servers`.

**A server does not start.** rho prints:

```
failed to start the MCP server <name>: <reason>. Check the command path.
```

The session continues. The model sees an error result instead of a tool output.

**Two tool names collide.** rho prints:

```
the tool name <name> is used by both the MCP server <first> and the MCP server <second>. Rename one server so the final tool names differ.
```

Rename one server in `mcp.json` so the final names no longer match after hyphen-to-underscore conversion.

---

See [tools](tools.md) for the built-in tool list. See [permissions](permissions.md) before you add a server that can run code.
