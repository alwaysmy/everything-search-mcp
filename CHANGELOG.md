# Changelog

Behaviour a user can notice. The reasoning behind each change belongs here, not in
the docs — a reader installing the tool does not need to know what it used to do.

## 0.3.11-dev.1 — plugin-only (v3)

### Breaking

- This branch ships **no executable**. v3 is one DLL that Everything 1.5 loads into
  its own process, serving MCP at `http://127.0.0.1:8286/mcp`.
- `everything-search-mcp.exe` and its command line (`search`, `recent`, `count`,
  `details`, `servers`, `config`) were removed. The stdio server and the CLI stay on
  the `v2` branch.
  Why: two surfaces for one query drifted apart — the tools gained `exclude_noise`
  and `format` while the CLI still answered `unknown flag '--format'`. A regression
  run found that, not review.
- Clients that only speak stdio need v2. The fallback here is Everything's own HTTP
  API, documented in four lines.
- Client configuration is a URL now, not a `command` + path.

### Added

- `exclude_noise` on `everything_search` and `everything_count_stats`: appends
  `!path:` exclusions for `node_modules`, `.git`, `.venv`, `venv`, `dist`, `build`,
  `target`, `__pycache__`, `site-packages`. Measured on `D:\WorkDesigns`, `*.js`:
  15353 → 1905.
- `format: "compact"` on `everything_search` and `everything_find_recent`: one path
  per line instead of path + size + date.
- `format` and `noise_filter` fields in the structured result.
- The installer reads Everything's HTTP port from `Plugins.ini` and records it in
  `%APPDATA%\everything-search-mcp\config.json`; the server resolves
  environment variable → that file → default 23333.

### Changed

- A zero-result search now always ends with `Total matches: 0 (exact)`. `No results.`
  alone could not be told apart from a query that never ran — the ambiguity
  `include_total` was added for in 0.3.x, when the total cost a second request. It
  comes back with the search response now, so the number is printed unconditionally
  and `include_total` is gone; passing it produces the unknown-argument warning.

### Fixed

- `exclude_noise` excluded by unanchored substring: `!path:dist` also removed
  `dist-info` and `distribution` (24,426 folders → 9,745 on a real tree, `*dist-info*`
  130 → 0). The clauses are now `!path:"\name\"`, which removes the directory and its
  contents only. Residual: the excluded directory's own row survives (its path has no
  trailing separator), which only matters for `entry_type: folder`.
- An empty result scoped to a remote machine reported `path not found` when the path
  existed there (`Y:\安装包` had 58,708 matches on ts-always). The filesystem check now
  runs only when every backend is local, and with a 200 ms budget so a dead mapped
  drive cannot stall the call. "Local" is decided by the address, not by the registry
  name: `url: "http://127.0.0.1:23333"` is this machine even though it is labelled
  `127.0.0.1:23333`, and a remote registered under the name `local` is not.
- `everything_find_recent` accepted `format: "bogus"` silently while
  `everything_search` rejected it. It validates now, echoes `format`, and does it before
  the search runs rather than after — a rejected call no longer pays for the query.
- `call_tool` rebuilt all five tool schemas on every successful call just to read the
  argument names — ~830 µs of a ~1.5 ms request, invisible in `elapsed_ms`. The
  definitions are built once now: server-side overhead 1,022 → 191 µs, `tools/list`
  1,487 → 682 µs.
- Output schemas declare `additionalProperties: false` but omitted the new fields,
  so every search and count call failed with
  `-32602 ... data must NOT have additional properties`.
- `-NoRestart` installs skipped port recording: the block sat after the verification
  step that branch returns before.
- `servers::local_url` re-derived `23333` instead of using the resolved config, which
  made the recorded port invisible to searches.
- The raw-HTTP fallback snippet returned nothing useful: the API's `path=` parameter
  does not filter, rows carry only `name`/`type` without `path_column=1`, and the
  count field is `totalResults`.
