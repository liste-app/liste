# Using Liste from an agent

Liste ships a small MCP server, `liste-mcp`, that lets an agent capture, search, and complete tasks on this device. It talks to the running Liste app over a local socket and never to the network; the only bytes that leave the machine are the app's own encrypted sync traffic (Section 13 of `docs/ARCHITECTURE.md`).

The binary lives beside `liste` in the app bundle (`Liste.app/Contents/Helpers/` on macOS) or on `PATH` after "Install command line tools" in Settings. It speaks MCP over stdio only; there is no port to open.

## Registering it

Point your MCP client at the binary with no arguments. For a client that reads a JSON configuration:

```json
{
  "mcpServers": {
    "liste": {
      "command": "/Applications/Liste.app/Contents/Helpers/liste-mcp"
    }
  }
}
```

With a development build, use the path `cargo` produces, for example `target/debug/liste-mcp`, or run `just mcp`.

If Liste is not running, the server starts it in the background the first time a tool is used, and waits up to three seconds for it. If the app's store is locked, tools answer with a `locked` error until the app is unlocked; the server never asks for credentials.

## What the agent sees

On the first connection from a new agent client, the server's instructions carry a one-time notice: the agent sees task titles, notes, tags, and dates in plaintext, and if it is a cloud model, that content goes to the model's provider. End-to-end encryption protects tasks from Liste's servers, not from software the person runs on their own device (Section 7).

## Tools

| Tool | What it does |
|---|---|
| `capture` | Create a task from one natural-language line, with the same syntax as the app. |
| `search` | Full-text search over titles and notes; each word is a prefix. |
| `list_today` | Tasks due today or overdue. |
| `list_upcoming` | Tasks due in the coming days. |
| `get_task` | One task by id, with every field. |
| `update_task` | Change title, notes, due, priority, list, or tags by id. |
| `complete` | Mark a task complete; recurring tasks get their next instance. |
| `uncomplete` | Reopen a task. |
| `list_lists` | The lists a task can be filed under. |

Every tool returns a short text block and structured JSON. Errors are tool errors with a `code` of `locked`, `not_running`, `version_mismatch`, `not_found`, `invalid`, or `internal`, matching the CLI's exit codes.
