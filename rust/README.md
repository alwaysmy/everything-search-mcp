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

Three notes that are easy to get wrong:

- Everything's HTTP API **ignores its `path=` and `folder=` parameters**. A
  directory scope must be expressed with the `path:"..."` search **function**.
  It has to be a function rather than a quoted literal, because under regex
  matching the whole search text becomes the pattern and an injected literal
  path would be read as part of it — that combination silently returned zero
  results until it was fixed.
- For the same reason `match_regex` compiles to the `regex:` **function**, not to
  Everything's `&regex=1` flag, so that it composes with `path:`.
- `match_path` maps to Everything's `p=1` switch. That parameter is absent from
  the HTTP documentation; it was confirmed live (274 → 25686 results for a term
  that occurs only in paths).
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

| Tool | Purpose |
|---|---|
| `everything_search` | Query by name, extension, size, date; supports a `category` preset, `entry_type` (file/folder), `path` scoping, paging, sort, regex, `max_per_parent` diversification and an opt-in multi-`probe` |
| `everything_find_recent` | Files modified within a period, with auto-expand to all time and explicit requested-vs-effective window reporting |
| `everything_file_details` | Metadata and optional text preview for specific paths |
| `everything_count_stats` | Count/size without listing, optional per-extension breakdown |
| `everything_search_batch` | 1-8 searches in one call, to save agent round trips |

`category` replaces the former `everything_search_by_type` tool. The category
table is worth keeping, but a second search entry point made the model choose
between two tools that do the same thing.

Every tool carries `annotations` (`readOnlyHint`, `idempotentHint`,
`destructiveHint: false`, `openWorldHint: false`) and an `outputSchema`, and
returns `structuredContent` alongside text that carries the **same** information.
Clients have disagreed about which channel reaches the model, so neither is a
stub. Input schemas are **flat**; the server also **accepts a `params`-wrapped
argument object** so clients holding a stale cached schema keep working.

### Diagnosing a result set: `effective_query`

Every search result reports the exact Everything expression that ran, after
`path` / `category` / `entry_type` expansion:

```json
{
  "query": "*.rs",
  "effective_query": "path:\"D:\\proj\" file: ext:rs;...;hcl *.rs",
  "total": 6, "total_accuracy": "exact",
  "returned": 3, "offset": 0, "next_offset": 3, "has_more": true,
  "results": [{"name": "tools.rs", "path": "D:\\proj\\src",
               "full_path": "D:\\proj\\src\\tools.rs", "type": "file",
               "size": 42300, "modified": "2026-09-12 07:27:55"}],
  "elapsed_ms": 0.81
}
```

### Accuracy is labelled

`count` / `total` are **exact** — Everything reports the true total independently
of how many rows are fetched, so `count_stats` asks for a single row when size is
not needed. `total_size` and the per-extension breakdown are **sampled**: the HTTP
API has no aggregate, so they are extrapolated from the fetched sample and
reported as `"sampled"` rather than passed off as exact.

### Multi-probe

`probe: true` widens a thin result set for a bare term by also looking for it in
the path and among folders, because agents often name something that is really a
directory rather than a file. It is opt-in, only fires for bare terms, and every
extra query is reported in `probes` so the widening is never silent.


## Differences from the Python implementation

A 34-shape comparison against the Python server shows 30 identical outcomes.
The remaining four are deliberate or immaterial:

- **Errors are reported with `isError: true`.** The Python server catches
  exceptions and returns them as ordinary result text with `isError: false`.
  Both reject the same inputs; Rust follows the MCP specification instead of a
  text convention. A client that greps for `"Error:"` in the body would need
  updating.
- Tie-breaking among equal sort keys can differ; the result *sets* match.
- Two Python-era dependencies are gone by construction: `EVERYTHING_ES_PATH`
  (no `es.exe`) and `EVERYTHING_INSTANCE` (the HTTP server targets the default
  instance). `EVERYTHING_HTTP_URL` replaces them.

Everything else — `path` scoping, paging via `offset`, all 14 sort values,
`match_case`, `match_whole_word`, `match_regex`, `match_path`, `include_total`,
all 10 `file_type` categories, all 19 `period` values plus raw `last…` syntax,
`extensions` with either separator, `auto_expand`, and the
`sample_sort`/`file_type`/`period` validation rules — behaves the same.

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
