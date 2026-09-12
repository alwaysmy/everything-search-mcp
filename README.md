<div align="center">
  <h1>⚡ Everything MCP</h1>
  <p>
    <strong>MCP server for <a href="https://www.voidtools.com/">voidtools Everything</a> - search millions of Windows files in milliseconds from any AI agent.</strong>
  </p>
  <p>
    <a href="https://github.com/alwaysmy/everything-search-mcp"><img alt="Repository" src="https://img.shields.io/badge/repo-alwaysmy%2Feverything--search--mcp-111827"></a>
    <a href="LICENSE"><img alt="License" src="https://img.shields.io/github/license/alwaysmy/everything-search-mcp.svg?cacheSeconds=300&v=20260204"></a>
  </p>
</div>

---

> **Personal fork notice** — this is a detached, self-maintained fork of
> [`elis132/everything-mcp`](https://github.com/elis132/everything-mcp) (MIT).
> The upstream remote was removed on 2026-09-12 and **upstream changes are no
> longer tracked or merged**. The last absorbed upstream commit is pinned to the
> local tag `upstream-base-elis132`, so history stays traceable without any
> remote. Copyright remains with the original author — see [LICENSE](LICENSE).

## Quick start

```
/plugin marketplace add alwaysmy/everything-search-mcp
/plugin install everything-search-mcp@everything-search-mcp
```

That's it for Claude Code - the plugin bundles the MCP server and a skill that teaches the query syntax. For every other client, see [Installation](#installation) below.

## Why this one

|  | **everything-search-mcp** (this) | [mamertofabian](https://github.com/mamertofabian/mcp-everything-search) (342⭐) | [Josephur](https://github.com/Josephur/everything-mcp) (26⭐) | essovius |
|---|---|---|---|---|
| Tools | 5 | 1 | 1 | 16 |
| Setup | Auto-detects es.exe | Manual SDK DLL path | Manual HTTP server + host/port | Manual es.exe in PATH |
| Everything 1.5 | Auto-detects instance | Not supported | Untested | Manual flag |
| Talks to Everything via | `es.exe` subprocess | Everything SDK (DLL) | Everything's HTTP server plugin (unauthenticated) | `es.exe` subprocess |
| Tests / CI | pytest, GitHub Actions | None visible | None visible | None visible |

## Performance

`es.exe` (Everything's real-time NTFS index) vs. a naive filesystem walk, same query:

- **everything-search-mcp**: 220 ms avg (5 runs)
- **Naive walk of `C:\`**: 66,539 ms
- **~300x faster**

<details>
<summary>Reproduce this benchmark</summary>

```powershell
@'
import os, subprocess, time, statistics

ES = os.path.expandvars(r"%LOCALAPPDATA%\Everything\es.exe")
QUERY = "everything.exe"

es_runs = []
for _ in range(5):
    t0 = time.perf_counter()
    subprocess.run([ES, "-n", "100", QUERY], capture_output=True, text=True)
    es_runs.append((time.perf_counter() - t0) * 1000)

t0 = time.perf_counter()
matches = []
for dirpath, _, filenames in os.walk(r"C:\\"):
    for name in filenames:
        if name.lower() == QUERY:
            matches.append(os.path.join(dirpath, name))
walk_ms = (time.perf_counter() - t0) * 1000

es_avg = statistics.mean(es_runs)
print("ES avg ms:", round(es_avg, 2))
print("Walk ms:", round(walk_ms, 2))
print("Speedup x:", round(walk_ms / es_avg, 1))
print("Matches:", len(matches))
'@ | python -
```
</details>

---

## Installation

### Prerequisites

1. **Windows** with [Everything](https://www.voidtools.com/) installed and **running**
2. **es.exe** (Everything's command-line interface) - included with Everything 1.5 alpha, or install separately:
   - `winget install voidtools.Everything.Cli`
   - `scoop install everything-cli`
   - `choco install es`
   - or download from [github.com/voidtools/es](https://github.com/voidtools/es/releases) and place it in your PATH
3. **Python 3.10+** or **uv**

### Run the server

This fork is **not published to PyPI**, so install it straight from the repository:

```bash
uv tool install --from git+https://github.com/alwaysmy/everything-search-mcp everything-search-mcp  # installs the command
uvx --from git+https://github.com/alwaysmy/everything-search-mcp everything-search-mcp             # or run once, without installing
```

Either way the executable is named `everything-search-mcp`; the client configs
below assume it is on your `PATH`.

From a local checkout:

```bash
git clone https://github.com/alwaysmy/everything-search-mcp.git
cd everything-search-mcp && pip install -e ".[dev]"
```

### Add it to your client

Every client below uses the same MCP server definition:

```json
{
  "mcpServers": {
    "everything": {
      "command": "everything-search-mcp"
    }
  }
}
```

| Client | How to add it |
|---|---|
| **Claude Code** | `/plugin install everything-search-mcp@everything-search-mcp` (see [Quick start](#quick-start)), or `claude mcp add everything -- everything-search-mcp` |
| **Claude Desktop** | Paste the JSON above into `%APPDATA%\Claude\claude_desktop_config.json` |
| **Codex CLI** | `codex mcp add everything -- everything-search-mcp` |
| **Gemini CLI** | `gemini mcp add -s user everything everything-search-mcp` |
| **Kimi CLI** | `kimi mcp add --transport stdio everything -- everything-search-mcp` |
| **Qwen CLI** | `qwen mcp add -s user everything everything-search-mcp` |
| **Cursor** | Paste the JSON above into Cursor's MCP settings UI |
| **Windsurf** | Paste the JSON above into `%USERPROFILE%\.codeium\windsurf\mcp_config.json` |
| **Any other MCP client** | Use the JSON above verbatim |

<details>
<summary>Running a local checkout</summary>

```json
{ "mcpServers": { "everything": { "command": "everything-search-mcp" } } }
```

Or with explicit Python: `{"command": "python", "args": ["-m", "everything_search_mcp"]}`
</details>

### Environment variables (optional)

Everything MCP auto-detects your setup, but you can override:

| Variable | Description | Example |
|---|---|---|
| `EVERYTHING_ES_PATH` | Path to es.exe | `C:\Program Files\Everything\es.exe` |
| `EVERYTHING_INSTANCE` | Named Everything instance | `1.5a` |
| `EVERYTHING_MAX_RESULTS_CAP` | Hard cap on results per search (default `1000`) | `200` |
| `EVERYTHING_TIMEOUT` | es.exe call timeout in seconds (default `30`) | `60` |

> Only set `EVERYTHING_INSTANCE` if you explicitly configured a named instance
> in Everything (Tools → Options → General → Instance). Most installs -
> including most Everything 1.5 installs - run on the **default** instance;
> setting this unnecessarily breaks the connection. If in doubt, leave it out.

```json
{
  "mcpServers": {
    "everything": {
      "command": "everything-search-mcp",
      "env": { "EVERYTHING_INSTANCE": "1.5a" }
    }
  }
}
```

---

## Tools

### 1. `everything_search` - the workhorse

| Parameter | Default | Description |
|---|---|---|
| `query` | *(required)* | Everything search query |
| `path` | `""` | Restrict to a directory (preferred over `path:` in the query) |
| `max_results` | 50 | 1-500 |
| `sort` | `date-modified-desc` | name, path, size, date-modified, date-created, extension (+ `-desc` variants) |
| `match_case` / `match_whole_word` / `match_regex` / `match_path` | false | Match modifiers |
| `offset` | 0 | Pagination offset |
| `include_total` | false | Also report the total number of matches (`-get-result-count`) |

**Query syntax:**

```
*.py                          all Python files
ext:py;js;ts                  multiple extensions
ext:py path:C:\Projects       Python files under a path
size:>10mb                    larger than 10 MB
size:1kb..1mb                 between 1 KB and 1 MB
dm:today / dm:last1week       modified today / in the last week
dc:2024                       created in 2024 (SLOW: creation time is not
                              indexed by default; may time out - prefer dm:)
"exact name.txt"              exact filename match
project1 | project2           OR search
!node_modules                 exclude a term
content:TODO                  files containing TODO (needs content indexing)
regex:^test_.*\.py$           regex search
parent:C:\src ext:py          files directly inside 'src' folders (full path)
dupe:  /  empty:               duplicate filenames / empty folders
```

### 2. `everything_search_by_type` - category search

Categories: `audio`, `video`, `image`, `document`, `code`, `archive`, `executable`, `font`, `3d`, `data`

Parameters: `file_type` *(required)*, `query`, `path`, `max_results`, `sort`

### 3. `everything_find_recent` - what changed?

Periods: `1min` … `12hours`, `today`, `yesterday`, `1day` … `1year`

Parameters: `period` (default `1day`), `path`, `extensions`, `query`, `max_results`, `auto_expand` (default true)

When the period yields fewer results than `max_results`, `auto_expand` retries without the time restriction (all time) so the call still returns a useful set; the result label notes `[auto-expanded to all time]`. Pass `auto_expand=false` for a strict period.

### 4. `everything_file_details` - deep inspection

Parameters: `paths` *(required, 1-20)*, `preview_lines` (0-200)

Returns full metadata; for directories, item count and listing; for text files with a preview, the first N lines.

### 5. `everything_count_stats` - quick analytics

Parameters: `query` *(required)*, `path`, `include_size` (default true), `breakdown_by_extension`, `sample_sort` (default `date-modified-desc`)

Count and size stats without listing every file - check scope before a big search.

`sample_sort` controls how files are sampled for the extension breakdown. The default (`date-modified-desc`) is less biased than `name`, because file-name order correlates with file extensions.

---

## Examples

| Ask | Call |
|---|---|
| Python files modified today in my project | `everything_find_recent(period="today", extensions="py", path="C:\Projects\myapp")` |
| How much space do my log files use? | `everything_count_stats(query="ext:log", include_size=true, breakdown_by_extension=true)` |
| First 50 lines of a config file | `everything_file_details(paths=["C:\Projects\app\config.yaml"], preview_lines=50)` |
| Duplicate filenames in Documents | `everything_search(query='dupe: path:"C:\Users\me\Documents"')` |
| Images larger than 5MB | `everything_search(query="ext:jpg;png;gif size:>5mb")` |
| How many .py files exist? | `everything_search(query="ext:py", max_results=1, include_total=true)` |

---

## Troubleshooting

**"es.exe not found"** - Install [Everything](https://www.voidtools.com/) and [es.exe](https://github.com/voidtools/es/releases), or set `EVERYTHING_ES_PATH`.

**"Everything IPC window not found"** - Make sure Everything is running (check the system tray). If you set `EVERYTHING_INSTANCE`, try removing it - most installs don't need it. Everything Lite doesn't support IPC.

**No results for valid queries** - Confirm Everything's index has finished building, try the same query in Everything's GUI, and check the drive/path is included in Everything's index settings.

**Debugging:**

```bash
everything-search-mcp 2>everything-search-mcp.log                        # server logs
npx @modelcontextprotocol/inspector everything-search-mcp          # MCP Inspector
```

---

## Development

```bash
pip install -e ".[dev]"   # install with dev dependencies
pytest                    # run tests
ruff check src/ tests/    # lint
```

Contributions welcome - see [CLAUDE.md](CLAUDE.md) for the architecture and design decisions. Areas of interest: direct named-pipe IPC, Everything SDK3 for 1.5, content search, file-watching, bookmark/tag support.

## License

MIT - see [LICENSE](LICENSE)

## Acknowledgments

[voidtools](https://www.voidtools.com/) for Everything, [Anthropic](https://anthropic.com/) for the Model Context Protocol, and the MCP community.

<!-- mcp-name: io.github.alwaysmy/everything-search-mcp -->
