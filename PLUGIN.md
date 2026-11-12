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
| Administrator | only to write the plugin allowlist in `HKLM` |

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
.\install-plugin.ps1 -NoBuild            # install the DLL already in target\release
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

Connections are kept alive: a client that reuses one socket skips the per-call
handshake, which was the single largest piece of this endpoint's overhead
(~0.47 ms of a ~0.62 ms console round trip). `Connection: close` is honored, and
HTTP/1.0 defaults to closing, both per spec.

That has a cost worth stating, because this thread runs inside `Everything.exe`:

| Bound | Value | Why |
|---|---|---|
| idle close | 5 s after a response | a kept-alive socket pins a thread; 100 idle sockets were measured holding 100 threads in the host |
| concurrent connections | 64 | past it, new ones get `503` immediately instead of queueing |
| first-request timeout | 30 s | a client that connects and never speaks still holds a slot, but only for that long |

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
Remove-Item "D:\MyPrograms\1_Tools\Everything\Plugins\everything_search_mcp64.dll"

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
