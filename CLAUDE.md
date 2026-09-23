# Developer guide

Native MCP server for [voidtools Everything](https://www.voidtools.com/): five
tools that answer file-search questions from Everything's live NTFS index.

## Layout

```
src/
  main.rs       process entry: no args -> MCP stdio server, subcommand -> CLI
  jsonrpc.rs    MCP stdio protocol, tool schemas, the params compat shim
  everything.rs HTTP transport, addresses, query constants, FILETIME and timezone
  servers.rs    the instance registry: named Everything servers and their switches
  filetype.rs   name-level type classification and 16 KB header sniffing
  tools.rs      the five tools: validation, query building, result formatting
  cli.rs        one-shot search/recent/count/details/servers
  setup.rs      `config`: generate or apply MCP client configuration
skills/
  everything-search/   the agent-facing skill; ships alongside the binary
TEST_SCRIPTS/
  compare-single-backend.ps1   single-instance regression against a git baseline
```

Two front ends over one implementation. `main.rs` decides, everything below it is
shared, so the CLI can never drift from the MCP server.

## Commands

```powershell
cargo build --release      # target\release\everything-search-mcp.exe
cargo test                 # unit tests: classification, time, addresses, registry
.\deploy.ps1               # build, install into the skill dir, hard-link onto PATH
.\deploy.ps1 -NoBuild      # deploy what is already built
```

`TEST_SCRIPTS\compare-single-backend.ps1 -BaselineExe <exe>` replays a batch of CLI
calls against the current build and one compiled from a git worktree, comparing text
byte for byte and JSON field for field. Run it after any change to the query path:
"searching one machine is unchanged" is otherwise an assertion, and the multi-instance
code paths are exactly the kind that quietly alter the single-instance answer.

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
- **`url` replaces the default instance set; it never adds to it.** Naming one
  machine must not also return another machine's rows, and the caller cannot tell
  from the result that it happened.
- **Every hit carries `source`, and several instances are grouped rather than
  merged.** A path from another computer is textually indistinguishable from a
  local one, so an unlabelled merged list turns a remote hit into a file the
  caller tries to open here.
- **`offset` is per instance; `total` is the sum of exact per-instance counts.**
  Paging a merged list with one cursor repeats or skips rows. Both facts are stated
  in the response (`offset_scope`, `backends[]`) rather than left to be inferred.
- **A failure is an error only when there is one instance.** With one there is no
  partial success to report, so it stays an ordinary tool error — the behaviour
  single-machine callers already depend on. With several, one machine being down
  must not withhold the others' answers.
- **The registry is read on every call, not cached at startup.** An MCP server
  lives for days; needing a restart to pick up a new machine would make the
  registry useless in practice.

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
- Credentials in an address (`http://user:pass@host:port`) are parsed and sent as
  Basic auth, but **no authenticated Everything instance was available to test
  against** — the base64 encoding is unit-tested, the round trip is not.
- A remote path's absence from an index is not evidence the file is gone:
  Everything indexes the NTFS volumes it was pointed at, so network drives and
  excluded folders never appear. `index_details` says this instead of reporting
  "not found".

## History

`main` is the native implementation. The original Python server is frozen on the
`legacy` branch and is not a supported fallback; it is kept for reference only.
