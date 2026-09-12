# everything-search-mcp

Instant whole-disk file search for AI agents on Windows, exposed as five MCP
tools. A single native executable — **no Python runtime, no `es.exe` subprocess,
no SDK DLL** — that answers from
[voidtools Everything](https://www.voidtools.com/)'s live NTFS index in about a
millisecond.

```
mcp__everything-search__everything_search
mcp__everything-search__everything_find_recent
mcp__everything-search__everything_file_details
mcp__everything-search__everything_count_stats
mcp__everything-search__everything_search_batch
```

## Install

**1. Get the binary.** Download `everything-search-mcp.exe` from
[Releases](https://github.com/alwaysmy/everything-search-mcp/releases), or grab
`everything-search-skill.zip`, which is the binary plus the agent-facing skill
that explains how to use the tools well.

**2. Point your clients at it.** The executable knows its own absolute path, so
it writes the configuration itself — no hand-copied paths, and nothing breaks
when the folder moves:

```powershell
.\everything-search-mcp.exe config          # show what each client on this machine needs
.\everything-search-mcp.exe config --write  # apply it (every file backed up first)
```

It covers DeepSeek Harness, Claude Code, Claude Desktop, Codex CLI, Gemini CLI,
Cursor, VS Code and opencode, each in its own format. With no `--target` it acts
on exactly those clients whose config file already exists; with no `--write` it
changes nothing and just prints the snippet.

**3. Requirements.** Windows, Everything installed **and running**, and
Everything's HTTP server enabled on port `23333`
(Tools → Options → HTTP Server).

> The server binds `0.0.0.0` and serves file contents by default, so anyone on
> your LAN can query the whole index. Restrict it in Everything's `Plugins.ini`:
> ```ini
> [http_server64.dll]
> bindings=127.0.0.1
> allow_file_download=0
> ```

## Why it is fast

Everything is reached over its built-in HTTP server rather than through `es.exe`,
IPC or the SDK DLL. Measured on one machine (Everything 1.5.0.1396, warm, same
query):

| Approach | Median |
|---|---:|
| **HTTP on loopback** | **0.70 ms** |
| IPC `WM_COPYDATA` | 17.44 ms |
| SDK3 `Everything3_x64.dll` | 17.34 ms |
| SDK2 `Everything64.dll` | 27.77 ms |
| `es.exe` subprocess | 60.24 ms |

Against the original Python implementation, same query and machine:

| | Python | This | |
|---|---:|---:|---|
| MCP `tools/call` round trip | 134.0 ms | **0.53 ms** | 251× |
| Cold start (spawn + `initialize`) | 1681 ms | **5 ms** | 336× |
| Distribution size | interpreter + deps | **~580 KB** | — |

Dropping the interpreter buys one order of magnitude; the transport change buys
the rest.

## Tools

| Tool | Purpose |
|---|---|
| `everything_search` | Query by name, extension, size or date. `category` presets, `entry_type` (file/folder), `path` scoping, paging, sorting, regex, `max_per_parent` diversification, opt-in multi-`probe` |
| `everything_find_recent` | Files modified within a period, with auto-expand to all time and explicit requested-vs-effective window reporting |
| `everything_file_details` | Metadata and an optional triage preview for specific paths |
| `everything_count_stats` | Count and size without listing; optional per-extension breakdown |
| `everything_search_batch` | 1–8 searches per call, to save agent round trips |

Every result reports the Everything expression that actually ran, so a surprise
is diagnosable instead of guessable:

```json
{
  "query": "*.rs",
  "effective_query": "path:\"D:\\proj\" file: ext:rs;...;hcl *.rs",
  "total": 6, "total_accuracy": "exact",
  "returned": 3, "offset": 0, "next_offset": 3, "has_more": true,
  "results": [{"name": "tools.rs", "path": "D:\\proj\\src",
               "full_path": "D:\\proj\\src\\tools.rs", "type": "file",
               "size": 42300, "modified": "2026-09-12 16:27:55"}],
  "elapsed_ms": 0.81
}
```

## The same executable is also a CLI

Started with no arguments it is an MCP stdio server, which is how a client spawns
it. Started with a subcommand it is a one-shot tool — usable from a script, a CI
job, or an agent that has no MCP support at all:

```powershell
everything-search-mcp search "*.py" --path D:\Projects --max 20
everything-search-mcp recent --period 1week --path D:\Projects
everything-search-mcp count  "ext:pdf" --exact-size
everything-search-mcp details D:\a\b.rs --preview 20
everything-search-mcp config --write
```

Exit codes: `0` success, `1` query failed (message on stderr), `2` bad arguments.
`--json` prints the structured form instead of text.

## Accuracy is labelled

| Quantity | Accuracy |
|---|---|
| `count` / `total` | **exact** — Everything reports the true total regardless of how many rows are fetched, so `count_stats` asks for a single row when size is not needed |
| `total_size`, extension breakdown | **not estimated** by default. File sizes are heavily skewed and a top-N slice is not a random sample, so extrapolating produced figures that contradicted each other (23.6 GB for a set whose PDF members alone came to 84 GB). Without `exact_size` the field is omitted and `total_size_accuracy` is `"unavailable"` |
| `exact_size: true` | sums every match, one paged pass, capped at 200,000 rows — above the cap it says so rather than guessing. 185,892 `.txt` files summed in ~620 ms |
| `modified` | **local time**, matching Explorer. Everything returns a raw FILETIME (UTC); rendered as-is it was 8 hours behind on a UTC+8 machine, which is exactly the field a "what changed recently" answer is built on |

## Type classification, in two levels

`everything_search` classifies from the **name only and never opens a file**:
reading a header per result would turn one index lookup into N filesystem
operations, which behaves completely differently on spinning disks, SMB shares,
cloud placeholders and machines with aggressive antivirus. Each result carries:

```json
{"kind": "image", "content_mode": "text", "format": "svg", "type_source": "extension"}
```

`kind` uses the same vocabulary as the `category` parameter, so a `kind` can be
fed straight back into `category`. `content_mode` is a separate axis on purpose:
`.svg` is image **and** text, `.docx` is document and binary, `.rs` is code and
text.

`everything_file_details` is the level that may read, and only when the extension
cannot tell — missing, unknown or ambiguous (`.dat`, `.bin`). It reads 16 KB and
resolves BOM → magic number → heuristic, so UTF-16 text (which is full of NUL
bytes) is not misread as binary. The preview is a **triage** preview: text files
only, and not a substitute for the agent's own file-reading tool.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `EVERYTHING_HTTP_URL` | `http://127.0.0.1:23333` | Base URL of Everything's HTTP server |
| `EVERYTHING_TIMEOUT` | `30` | Request timeout, seconds |
| `EVERYTHING_MAX_RESULTS_CAP` | `1000` | Hard cap on results per search |

`EVERYTHING_ES_PATH` and `EVERYTHING_INSTANCE` no longer exist: there is no
`es.exe` to point at, and the HTTP server targets the default instance.

## Design notes

Four things that are easy to get wrong, all of them found the hard way:

- Everything's HTTP API **ignores its `path=` and `folder=` parameters**. A
  directory scope has to be the `path:"..."` search **function**, not a quoted
  literal, because under regex matching the whole search text becomes the pattern
  and an injected literal would be read as part of it — that combination silently
  returned zero results.
- For the same reason `match_regex` compiles to the `regex:` function rather than
  Everything's `&regex=1` flag, so that it composes with `path:`.
- `match_path` maps to Everything's `p=1` switch, which is absent from the HTTP
  documentation; confirmed live (274 → 25,686 results for a term that occurs only
  in paths).
- Responses are UTF-8 and carry no `Content-Length`, so the body is read to EOF
  (`Connection: Close`). The lack of keep-alive is irrelevant on loopback: a
  fresh connection still costs well under a millisecond.

Unknown `sort` and `period` values are rejected with a clear error rather than
being passed through, and `sample_sort=name` is refused when a breakdown is
requested, because filename sort correlates with extension and biases the sample.

Errors are returned with `isError: true` per the MCP specification. The original
Python server caught exceptions and returned them as ordinary result text; a
client that greps the body for `"Error:"` would need updating.

## Development

```powershell
cargo build --release      # -> target\release\everything-search-mcp.exe
cargo test                 # 16 unit tests
.\deploy.ps1               # build, then install into the skill dir + hard-link onto PATH
.\deploy.ps1 -NoBuild      # deploy what is already built
```

`deploy.ps1` treats the skill directory as the one real copy and makes the names
on `PATH` **hard links** to it, so they cannot drift. A running MCP server holds
its own image open, so a redeploy renames the old binary aside rather than
deleting it (Windows allows renaming a running executable).

```
Cargo.toml      2 dependencies (serde, serde_json); lto + strip + panic=abort
deploy.ps1      build and install
src/
  main.rs       entry point: no args = MCP stdio server, else the CLI
  jsonrpc.rs    MCP stdio protocol, flat-schema tools, params compat shim
  everything.rs HTTP transport, query constants, FILETIME conversion, timezone
  filetype.rs   name-level classification and header sniffing
  tools.rs      the five tools, schemas, result formatting
  cli.rs        one-shot search/recent/count/details, so the exe works with no MCP
  setup.rs      `config`: emit or apply the client configuration for this exe
skills/
  everything-search/   the agent-facing skill (SKILL.md + INSTALL.md)
```

The crate's only `unsafe` is a single `GetTimeZoneInformation` call, used to
render `modified` in local time without pulling in a date/time dependency.

## The Python version

This project started as a Python MCP server for Everything. That implementation
is **frozen and unmaintained**, and it is not a fallback — it lives on the
`legacy` branch. `main` is the native one.

The tool set and the category table come from
[elis132/everything-mcp](https://github.com/elis132/everything-mcp), which is
where this work started.

## License

MIT — see [LICENSE](LICENSE).
