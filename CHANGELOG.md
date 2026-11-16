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
  15312 → 1834.
- `format: "compact"` on `everything_search` and `everything_find_recent`: one path
  per line instead of path + size + date.
- `format` and `noise_filter` fields in the structured result.
- The installer reads Everything's HTTP port from `Plugins.ini` and records it in
  `%APPDATA%\everything-search-mcp\config.json`; the server resolves
  environment variable → that file → default 23333.

### Fixed

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
