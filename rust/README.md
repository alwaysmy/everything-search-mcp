# everything-search-mcp — Rust implementation

A native MCP stdio server for [voidtools Everything](https://www.voidtools.com/).
**No Python runtime, no `es.exe` subprocess, no SDK DLL.** Single ~360 KB executable.

This is the **active** implementation. The Python package at the repository root
is retained for reference only and is **no longer maintained** — it is not a
supported fallback.

## Build

```bash
cd rust
cargo build --release
# -> target/release/everything-search-mcp.exe
```

Requires a Rust toolchain (MSVC target) and, for linking, the MSVC build tools.
On a machine where another `link.exe` (e.g. Cygwin's) comes first on `PATH`,
rustc still finds the MSVC linker through its own detection; no linker override
is needed or shipped.

To run it as a drop-in replacement for the Python server, put the binary on
`PATH` as `everything-search-mcp.exe` (the MCP server name) and optionally also
as `everything-mcp.exe` (the pre-rename command name) so that existing client
configuration keeps working unchanged.

## Measured performance

Same query (`*.py`, 50 results, warm), same machine, Everything 1.5.0.1396:

| | Python | Rust | |
|---|---:|---:|---|
| MCP `tools/call` round trip | 134.0 ms | **0.53 ms** | **251x** |
| Cold start (spawn + initialize) | 1681 ms | **5 ms** | **336x** |
| Executable size | multi-MB + interpreter | **0.35 MB** | — |

The Python figure decomposes as `es.exe` 60.2 ms + Python/MCP layer 73.7 ms.
Removing the interpreter is what buys the order of magnitude; the transport
change below removes the rest.

## Transport: HTTP on loopback

The server queries Everything over its built-in HTTP server
(`http://127.0.0.1:23333`) rather than spawning `es.exe`, using IPC, or loading
the SDK DLL. Measured on this machine:

| Approach | Median |
|---|---:|
| HTTP loopback | **0.70 ms** |
| IPC `WM_COPYDATA` | 17.44 ms |
| SDK3 `Everything3_x64.dll` (1.5 pipe) | 17.34 ms |
| SDK2 `Everything64.dll` | 27.77 ms |
| `es.exe` subprocess | 60.24 ms |

Two notes that are easy to get wrong:

- Everything's HTTP API **ignores its `path=` and `folder=` parameters**. To
  scope a search to a directory, the quoted literal path must be prefixed to the
  search string (`"D:\dir" *.py`). This is what the `path` argument compiles to.
- Responses are UTF-8 and **carry no `Content-Length`**, so the body is read to
  EOF (`Connection: Close`). The lack of keep-alive is irrelevant on loopback:
  a fresh connection still costs well under a millisecond.

Everything's HTTP server must be enabled. For a local-only client it should be
restricted to loopback, since by default it can bind all interfaces and serve
file contents:

```ini
; Everything -> Plugins.ini -> [http_server64.dll]
bindings=127.0.0.1
allow_file_download=0
```

## Tools

Five tools, matching the Python implementation's surface:

| Tool | Purpose |
|---|---|
| `everything_search` | Query by name, extension, size, date; supports `path`, paging, sort, regex |
| `everything_search_by_type` | One of 10 categories (`audio`…`data`) instead of hand-written `ext:` lists |
| `everything_find_recent` | Files modified within a period, with auto-expand to all time |
| `everything_file_details` | Metadata and optional text preview for specific paths |
| `everything_count_stats` | Count/size without listing, optional per-extension breakdown |

Input schemas are **flat** (as the Python 1.1.0 release intended). The server
additionally **accepts a `params`-wrapped argument object** so that clients
holding a stale cached schema keep working.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `EVERYTHING_HTTP_URL` | `http://127.0.0.1:23333` | Base URL of Everything's HTTP server |
| `EVERYTHING_TIMEOUT` | `30` | Request timeout, seconds |
| `EVERYTHING_MAX_RESULTS_CAP` | `1000` | Hard cap on results per search |

Note that `EVERYTHING_ES_PATH`, `EVERYTHING_INSTANCE`, `EVERYTHING_TIMEOUT` and
`EVERYTHING_MAX_RESULTS_CAP` are named after voidtools Everything, not after this
package; `EVERYTHING_ES_PATH` is meaningless here because no `es.exe` is used.

## Layout

```
rust/
  Cargo.toml        2 dependencies (serde, serde_json); lto + strip + panic=abort
  src/
    main.rs         entry point, --help/--version, env config
    jsonrpc.rs      MCP stdio protocol, flat-schema tools, params compat shim
    everything.rs   HTTP transport, query constants, FILETIME conversion
    tools.rs        the five tools, schemas, result formatting
```

Unknown `period` and `file_type` values are rejected with a clear error rather
than being passed through, and `sample_sort=name` is refused because filename
sort correlates with extension and biases the breakdown sample — both carried
over from the Python implementation's fixes.
