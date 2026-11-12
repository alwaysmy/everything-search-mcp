# Running the MCP endpoint inside Everything

The default way to use this project is a separate process: a client spawns
`everything-search-mcp.exe` and speaks MCP over stdio. This page covers the other
option — a plugin DLL that Everything loads into **its own process**, where it
serves the same five tools over loopback HTTP.

```
client ──HTTP──► 127.0.0.1:8286 (inside Everything.exe)  ──HTTP──► 127.0.0.1:23333
```

Nothing is spawned, nothing to supervise, and the endpoint disappears exactly
when Everything exits.

## What you need

| | |
|---|---|
| Everything | 1.5 x64 (the 1.4 plugin API is different) |
| HTTP server | enabled in Everything → Tools → Options → HTTP Server, port `23333` |
| Administrator | only to write the plugin allowlist in `HKLM` (the installer checks for elevation unconditionally today) |

The plugin is unsigned, and **Everything refuses unsigned plugins that are not on
its allowlist without logging anything** — the module simply never appears in the
process. That is what `install-plugin.ps1` writes:

| Location | Value |
|---|---|
| `Plugins\everything_search_mcp64.dll` | the module itself |
| `Everything.ini` → `plugin_filename` / `plugin_enabled` | load list and one flag per entry, same order |
| `Plugins.ini` → `[everything_search_mcp64.dll] enabled=1` | start it |
| `HKLM\SOFTWARE\voidtools\Everything` → `AllowedPlugins` | third-party allowlist, **semicolon**-separated |

A comma-joined `AllowedPlugins` silently disables every entry in it, including
ones that used to work. That is the single most common reason a plugin "does not
load".

## Install

From the repository root, in an elevated PowerShell:

```powershell
.\install-plugin.ps1                     # build, install, restart Everything, verify
.\install-plugin.ps1 -NoBuild            # install the DLL already in target\release (automatic in the release archive)
.\install-plugin.ps1 -NoRestart          # register only; takes effect next restart
.\install-plugin.ps1 -EverythingDir D:\Tools\Everything   # non-standard install
```

Everything loads plugins **only at startup**, so installing costs one exit/start
cycle. The restart is issued with `-startup -WindowStyle Hidden` so the window
does not jump to the foreground; your existing window does get closed.

## Point a client at it

Register it as an HTTP MCP server:

```json
{
  "mcpServers": {
    "everything-search": { "url": "http://127.0.0.1:8286/" }
  }
}
```

Clients post to `/` or `/mcp`. The endpoint is stateless (no session id, no SSE
stream), so `GET` returns `405` on purpose — that is legal for a tool-only server.

Per-client shapes. The rows marked in the notes were checked by making the client
itself connect, not by reading the schema:

| Client | Where | Entry |
|---|---|---|
| DeepSeek Harness | `~/.dsh/profiles/<p>/cordis.patch.yml` | `transport: streamable-http` + `url:` |
| opencode | `~/.config/opencode/opencode.json` | `"type": "remote"` + `"url"` |
| Codex CLI | `~/.codex/config.toml` | `[mcp_servers.<name>]` + `url = "..."` |
| pi / omp | `~/.omp/agent/mcp.json` | `mcpServers.<name>.url` |
| Kimi Code | `~/.kimi-code/mcp.json` | `mcpServers.<name>.url` |

Two gotchas worth knowing: Kimi Code's `config.toml` silently ignores a
`[mcp_servers]` table (it reads `mcp.json` instead — `kimi doctor` reports the key
as "ignored by the v2 engine"), and `opencode mcp add --url` appends a nested
`mcp.servers` object instead of replacing an existing `mcp.<name>` entry, which
leaves two servers with the same name.

What was confirmed by running the client rather than reading its docs: opencode
(`mcp list` reports connected, and no stdio child is spawned), pi/omp and Kimi
Code (each answered one search through the endpoint). Codex's own reader accepts
the entry (`codex mcp list`). DeepSeek Harness was not running here, so its
profiles are config-only. ZCode stays on stdio: its `mcp.servers` map has no HTTP
shape anywhere in this install, and it ships no CLI to add one safely.

Connections are kept alive: a client that reuses one socket skips the per-call
handshake, which was the single largest piece of this endpoint's overhead
(~0.47 ms of a ~0.62 ms console round trip). `Connection: close` is honored, and
HTTP/1.0 defaults to closing, both per spec.

That has a cost worth stating, because this thread runs inside `Everything.exe`:

| Bound | Value | Why |
|---|---|---|
| idle close | 5 s after a response | a kept-alive socket pins a thread; 100 idle sockets were measured holding 100 threads in the host |
| concurrent connections | 256 | past it, new ones get `503` immediately instead of queueing |
| first-request timeout | 30 s of read inactivity | a client that connects and never speaks holds a slot for that long (it is a per-read timeout, not a total deadline) |

The cap costs nothing while it is not reached — it is a counter, not a
preallocation. What costs memory is the connections themselves: measured at
256 open sockets, `Everything.exe` grew by 256 threads and 9 MB RSS (≈35 KB per
connection, plus 2 MB of reserved-but-uncommitted stack each), and returned to
its baseline within 5 s of them going idle. Raise it with
`EVERYTHING_MCP_MAX_CONNECTIONS` if you really run hundreds of agents at once.

Two framing details, both deliberately refused rather than guessed at:
`Transfer-Encoding: chunked` gets `411` (a Content-Length-only parser would read
it as an empty body), and `Expect: 100-continue` is answered before the body is
awaited, because a client that sends it waits for that line.

## Verify by hand

```powershell
# 1) the endpoint answers a real MCP handshake
$body = '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"probe","version":"1"}}}'
Invoke-WebRequest -Uri http://127.0.0.1:8286/mcp -Method Post -ContentType 'application/json' `
  -Headers @{Accept='application/json'} -Body $body -UseBasicParsing | Select-Object -Expand Content

# 2) what the plugin itself saw
Get-Content "$env:LOCALAPPDATA\everything-search-mcp\plugin.log" -Tail 5
```

`-UseBasicParsing` is not optional on Windows PowerShell 5.1: without it the call
throws a bare `NullReferenceException` on a request that actually succeeded.

The log should show `started: MCP on http://127.0.0.1:8286/ — plugin <version> on
Everything <version>`.

## Port

`8286` by default. `23333` belongs to Everything's own HTTP server and `8285` is
used by another Everything MCP plugin, so neither is a safe default. Override with
`EVERYTHING_MCP_PORT` in the environment Everything inherits:

```powershell
[Environment]::SetEnvironmentVariable('EVERYTHING_MCP_PORT', '9000', 'User')
# then restart Everything
```

`install-plugin.ps1 -Port` only changes where the installer *checks*; the plugin
reads the environment variable.

## Uninstall

```powershell
# 1) stop Everything
Stop-Process -Name Everything

# 2) drop the DLL
Remove-Item "$env:ProgramFiles\Everything\Plugins\everything_search_mcp64.dll"

# 3) remove it from the load list, the per-plugin settings and the allowlist
#    (plugin_filename / plugin_enabled must stay aligned; AllowedPlugins is
#     semicolon-separated)
```

There is no uninstall switch: the installer only ever adds this entry, and a
half-removed plugin is worse than a disabled one (`plugin_enabled` must stay
aligned with `plugin_filename`). Remove all three by hand, or set that entry's
flag to `0` and leave the DLL in place if you only want it off.

## Security

- Binds `127.0.0.1` only; nothing off-machine can reach it.
- Runs inside `Everything.exe`, so a crash here takes the file manager with it.
  The plugin therefore avoids Everything's `db_*` API entirely (it has main-thread
  affinity), never panics in an entry point, and does all queries over the HTTP
  server the same way the stdio build does.
- Tool calls are read-only; the plugin adds a transport, not a second query path.

## Troubleshooting

| Symptom | Cause |
|---|---|
| Port never opens, no log file | DLL not loaded: check `AllowedPlugins` is semicolon-separated and contains the file name |
| `start failed on port N` in the log | something else owns the port; pick another with `EVERYTHING_MCP_PORT` |
| Endpoint up but queries fail | Everything's own HTTP server (23333) is off |
| Everything window jumps to the foreground on install | old installer; the current one restarts with `-startup` |
| `tools/list` returns 5 tools | correct — the "14 tools" figure in older notes was stale |
