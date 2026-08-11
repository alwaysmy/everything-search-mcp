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

All five tools take their arguments inside a single `params` object - the
MCP input schema for every tool is `{"params": {...}}`. For example:

```json
{"params": {"query": "*.py", "path": "C:\\Projects"}}
```

The `tool(args...)` shorthand used below is equivalent to that JSON; a client
that reads the schema builds the `params` object automatically, and calls that
omit it fail with `params Field required`.

## Query syntax essentials

Space between terms = AND. Key operators:

```
report.pdf                    name contains "report.pdf"
*.py                          extension wildcard
ext:py;js;ts                  multiple extensions
path:C:\Projects ext:py       restrict to a directory tree (or use the path argument)
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
regex:^test_.*\.py$           regex (or pass match_regex=true)
```

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
