# Developer guide

Native MCP server for [voidtools Everything](https://www.voidtools.com/): five
tools that answer file-search questions from Everything's live NTFS index.

## Layout

```
src/
  main.rs       process entry: no args -> MCP stdio server, subcommand -> CLI
  jsonrpc.rs    MCP stdio protocol, tool schemas, the params compat shim
  everything.rs HTTP transport, query constants, FILETIME and timezone handling
  filetype.rs   name-level type classification and 16 KB header sniffing
  tools.rs      the five tools: validation, query building, result formatting
  cli.rs        one-shot search/recent/count/details
  setup.rs      `config`: generate or apply MCP client configuration
skills/
  everything-search/   the agent-facing skill; ships alongside the binary
```

Two front ends over one implementation. `main.rs` decides, everything below it is
shared, so the CLI can never drift from the MCP server.

## Commands

```powershell
cargo build --release      # target\release\everything-search-mcp.exe
cargo test                 # unit tests: filetype classification, time conversion
.\deploy.ps1               # build, install into the skill dir, hard-link onto PATH
.\deploy.ps1 -NoBuild      # deploy what is already built
```

## Invariants

These are load-bearing. Changing one silently breaks behaviour that is hard to
notice.

- **The search path never touches the filesystem.** `everything_search`
  classifies from the name alone. Opening files per result turns one index lookup
  into N filesystem operations with wildly different cost on spinning disks, SMB
  shares and cloud placeholders. Only `everything_file_details` reads, and only
  when the extension cannot decide.
- **Everything's HTTP API ignores `path=` and `folder=`.** Scope is the
  `path:"..."` search *function*, and it must be a function rather than a quoted
  literal or it becomes part of the regex pattern (measured: 0 results).
  `match_regex` compiles to `regex:` for the same reason — sending `&regex=1` too
  would double-apply the pattern.
- **`date_modified` is a raw FILETIME in UTC.** It is converted to local time
  before it is shown; Everything's own UI and Explorer both show local time, and
  `modified` is the field a "what changed recently" answer rests on.
- **`count`/`total` are exact; sizes are not.** Everything reports the true total
  regardless of the page size. It reports no aggregate at all, so `total_size`
  is omitted unless `exact_size: true` asks for a real sum — extrapolating from a
  top-N slice produced self-contradictory numbers, and a wrong number is worse
  than no number.
- **Text and `structuredContent` carry the same information.** Clients have
  disagreed about which channel reaches the model, so neither is a stub. Type
  classification is only visible in structured form, which is why the text form
  flags `<kind/content_mode>` whenever `content_mode != "text"`.
- **Input schemas are flat**, and the server also accepts a `params`-wrapped
  object so clients holding a stale cached schema keep working.
- **`config` writes nothing without `--write`.** It rewrites other programs'
  configuration files; the writers are line-based and conservative on purpose
  (DSH YAML keeps its comments, an unparsable JSON file is left alone with an
  error rather than reformatted).

## Gotchas

- The crate has exactly one `unsafe`: `GetTimeZoneInformation` in
  `everything.rs`. That is deliberate — a whole date/time dependency for one
  integer is not worth it, and the registry is not reachable without one either.
- Everything returns no `Content-Length`, so responses are read to EOF.
- Error text should say what to do, not just what failed. The connection error
  names the Everything option to check, because "it does not work" is the most
  common report.
- `deploy.ps1` renames a running binary aside instead of deleting it: the MCP
  server is mapped from the file it is replacing, and DSH respawns it within a
  second of being killed.

## History

`main` is the native implementation. The original Python server is frozen on the
`legacy` branch and is not a supported fallback; it is kept for reference only.
