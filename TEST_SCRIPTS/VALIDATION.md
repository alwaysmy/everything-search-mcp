# Result correctness and bounded HTTP regression coverage

Validated on Linux with Rust 1.85.0 and Cargo 1.85.0, using the unchanged locked
runtime dependencies. No real Everything index or private user files were used.

Before implementation, the original 31 unit tests passed. Four additional
loopback HTTP regression tests failed on the original implementation:

- A 1-byte file, a folder, and a 3-byte file summed to 7 bytes instead of 4:
  the cursor advanced by known sizes rather than consumed rows
- A file with an unknown size still produced a size labelled exact
- Diversification displayed two rows after consuming three, but returned cursor 2
- Probe results replaced an actual extended count of 100 with the number of
  locally merged sample rows and restarted the extra searches at offset zero

After the changes, `cargo test --locked` passes 54 tests. New coverage includes
short/empty/changing size scans, missing totals and file sizes, folder rows,
per-source cursors and actual follow-up requests, partial/all-failed backends,
single-source labels, unknown remote existence, truncated index lookups,
batch output budgets and state preservation, plus HTTP framing, size limits,
chunk decoding, hostname resolution and a continuously dripping peer deadline.

`cargo build --release --locked` builds the release binary. Then:

```sh
python3 TEST_SCRIPTS/mock-http-smoke.py --binary target/release/everything-search-mcp --validate-schema
```

This checks the compiled CLI and MCP server against an isolated HTTP fixture,
compares their search JSON apart from elapsed time, and validates six structured
responses against the five published tool schemas. The optional schema check
uses Python jsonschema 4.26.0 in this validation environment; omit
`--validate-schema` to run the standard-library-only smoke checks.

Not executed here: the live Windows/Everything PowerShell baseline comparison,
Windows installation/deployment, real authenticated Everything, DNS blackhole or
unreachable-network timing, or Windows-specific runtime behavior. Existing
Windows CI remains the next platform check. The mock server verifies emitted
probe grouping/scoping and pagination contracts; it does not emulate Everything's
search engine. OR/group syntax was checked against the official voidtools manual.

Stdio dispatch is intentionally still synchronous and cannot process cancellation
while a tool call runs. The deadline is per HTTP request, not an atomic snapshot
or a whole multi-page tool deadline. Source labels, scope/accuracy fields, null
cursors when no common continuation exists, and the explicit probe union are
intentional response-contract changes; cached tool schemas must be refreshed.
