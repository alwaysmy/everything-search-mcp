---
name: everything-search
description: Find files and folders on Windows instantly using the Everything MCP tools (everything_search, everything_find_recent, everything_search_by_type, everything_file_details, everything_count_stats). Use whenever the user asks to locate, list, count, or size files anywhere on a Windows machine - dramatically faster than dir, Get-ChildItem, glob, or recursive directory walks.
---

# Everything File Search

The `everything_*` MCP tools query voidtools Everything's real-time NTFS index.
A search over millions of files returns in milliseconds, so prefer these tools
over shell commands (`dir /s`, `Get-ChildItem -Recurse`, glob) for ANY
filename-based lookup outside the current project directory.

## Picking the right tool

| Task | Tool |
|---|---|
| Find files/folders by name, extension, size, date | `everything_search` |
| "What changed in the last hour/day/week?" | `everything_find_recent` |
| All videos / documents / code / archives somewhere | `everything_search_by_type` |
| Metadata or first N lines of specific files | `everything_file_details` |
| "How many?" / "How much disk space?" | `everything_count_stats` |

Use `everything_count_stats` BEFORE listing when a query might match
thousands of files - check the scope first, then narrow.

## Calling the tools

All five tools take flat, named arguments that match their input schema
exactly.  For example:

```json
{"query": "*.py", "path": "C:\\Projects"}
```

Do not wrap those arguments in a `params` object: a nested payload is rejected
with `params.<field> Extra inputs are not permitted`.  This matters because
everything-search-mcp 1.0.x did nest its arguments, so older examples circulating for
this server may show the wrong shape.

The `tool(args...)` shorthand used below means one JSON argument per key;
a client that reads the schema builds these automatically.

## Query syntax essentials

Space between terms = AND. Key operators:

```
report.pdf                    name contains "report.pdf"
*.py                          extension wildcard
ext:py;js;ts                  multiple extensions
path:C:\Projects ext:py       restrict to a directory tree (or use the path argument)
parent:C:\Projects ext:py     direct children of one folder
size:>10mb  size:1kb..1mb     size filters
dm:today  dm:last1week        modified date
dc:2024                       created date (SLOW: creation time is not
                              indexed by default; may time out - prefer dm:)
"exact name.txt"              phrase with spaces (quote it)
a | b                         OR
!node_modules                 exclude
folder:                       folders only
file:                         files only
dupe:                         duplicate names
empty:                        empty folders
content:TODO ext:py           file contents (SLOW, needs content indexing)
regex:^test_.*\.py$           regex (or pass match_regex=true)
```

## Tool reference

### `everything_search`

| Argument | Default | Notes |
|---|---|---|
| `query` | required | Everything syntax above, 1-2000 chars |
| `path` | `""` | Restrict to one directory; prefer this over `path:` in the query |
| `max_results` | `50` | 1-500 |
| `offset` | `0` | Skip N results, for paging |
| `sort` | `date-modified-desc` | One of the 14 values below |
| `match_case` | `false` | Case-sensitive |
| `match_whole_word` | `false` | Whole words only |
| `match_regex` | `false` | Treat `query` as a regex |
| `match_path` | `false` | Match the full path instead of the filename |
| `include_total` | `false` | Also return the total match count (extra call) |

### `everything_find_recent`

`period` (default `1day`), `path`, `extensions` (`py,js,ts` or `py;js;ts`),
`query` (extra filter), `max_results` (default `50`), `auto_expand` (default
`true`). Valid `period` values: `1min`, `5min`, `10min`, `15min`, `30min`,
`1hour`, `2hours`, `6hours`, `12hours`, `today`, `yesterday`, `1day`, `3days`,
`1week`, `2weeks`, `1month`, `3months`, `6months`, `1year`, or raw Everything
syntax like `last2hours`. Anything else is rejected rather than passed through.

### `everything_search_by_type`

`file_type` (required - see the categories below), plus `query`, `path`,
`max_results`, `sort`. Builds the `ext:` clause for you.

### `everything_file_details`

`paths` (required, 1-20 entries, **must be absolute**), `preview_lines`
(0-200, default `0`).

### `everything_count_stats`

`query` (required), `path`, `include_size` (default `true`),
`breakdown_by_extension` (default `false`, samples up to 500 hits),
`sample_sort` (default `date-modified-desc`).

## Sort options

`name`, `name-desc`, `path`, `path-desc`, `size`, `size-asc`, `size-desc`,
`date-modified`, `date-modified-asc`, `date-modified-desc`, `date-created`,
`date-created-asc`, `date-created-desc`, `extension`.

`size` and `size-asc` are aliases, as are `date-modified`/`date-modified-asc`
and `date-created`/`date-created-asc`.

## file_type categories

| Category | Extensions |
|---|---|
| `audio` | mp3 wav flac aac ogg wma m4a opus aiff alac |
| `video` | mp4 avi mkv mov wmv flv webm m4v mpeg mpg 3gp ts |
| `image` | jpg jpeg png gif bmp svg webp tiff tif ico raw heic heif avif psd |
| `document` | pdf doc docx xls xlsx ppt pptx odt ods odp rtf txt md epub pages numbers key |
| `code` | py js ts jsx tsx c cpp h hpp cs java go rs rb php swift kt scala r lua sh bash ps1 bat cmd sql html css scss sass less vue svelte dart zig nim hx ex exs erl hs ml fs clj lisp asm toml yaml yml json xml ini cfg conf env dockerfile makefile cmake gradle sbt proto graphql tf hcl |
| `archive` | zip rar 7z tar gz bz2 xz tgz zst lz4 cab iso dmg |
| `executable` | exe msi dll sys com scr appx msix |
| `font` | ttf otf woff woff2 eot fon |
| `3d` | obj fbx stl blend dae 3ds gltf glb usd usda usdz step iges |
| `data` | csv tsv json jsonl ndjson xml sqlite db mdb accdb parquet arrow avro hdf5 feather |

## Patterns that work well

- Locate a project someone mentioned: `everything_search(query="folder: myproject")`
- Find a config file of unknown location: `everything_search(query="wg0.conf | wireguard ext:conf")`
- Recently downloaded file: `everything_find_recent(period="1day", path="C:\\Users\\<user>\\Downloads")`
- `everything_find_recent` auto-expands to all time when the period returns
  fewer than `max_results` results (pass `auto_expand=false` for a strict window).
- Disk usage of build artifacts: `everything_count_stats(query="node_modules folder:", include_size=true, breakdown_by_extension=true)`
- Total number of matches without listing: `everything_search(query="ext:py", max_results=1, include_total=true)`
- Then inspect what you found: `everything_file_details(paths=[...], preview_lines=30)`

## Pitfalls

- The `everything_*` tools only exist when the Everything MCP server is
  connected. If this session has no `everything_*` tools, fall back to running
  the CLI directly (`es.exe -n 50 <query>`) if available, and offer to
  configure the MCP server instead of guessing at tool calls.
- `es.exe` ships with Everything 1.5a but NOT with stable 1.4. If the tools
  report "es.exe not found" on a custom install location, set the
  `EVERYTHING_ES_PATH` environment variable to the full es.exe path.
- **Prefer the `path` parameter over embedding `path:"..."` in the query.**
  A `path:"..."` clause inside the query string is extracted and routed to the
  es.exe `-path` switch, but paths containing spaces are safest passed via the
  dedicated `path` argument.
- `include_total` on `everything_search` adds one extra `-get-result-count`
  call - leave it off when searching hot paths repeatedly.
- The extension breakdown in `everything_count_stats` samples files; the
  default `sample_sort` (`date-modified-desc`) is less biased than `name`,
  which correlates with file extensions.
- Results reflect the index, not content: `content:` search only works if the
  user enabled content indexing in Everything (rare) - to search inside files,
  find candidates by name first, then read them.
- Everything must be running; if tools return connection errors, tell the user
  to start Everything (system tray). Do not suggest setting
  `EVERYTHING_INSTANCE` unless they configured a named instance.
- Search is across ALL indexed drives by default - add `path` to scope, and
  prefer `max_results`/`offset` paging over huge listings.
- `everything_file_details` rejects relative paths (the server's working
  directory is not predictable), so resolve them before calling.
