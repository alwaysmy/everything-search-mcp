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

## Concurrency: no head-of-line blocking

The in-process endpoint used to take one global `Arc<Mutex<Server>>` per request,
so tool calls were serialized. Measured on the live index: with a 730 ms
`everything_count_stats` in flight, an `everything_search` that answers in 25 ms
waited 829 ms. Everything's own HTTP server one layer down is fully parallel
(1-2 ms for the same probe while a 259 ms regex search ran), so the lock was a
regression introduced by hosting, not a property of the backend.

`cargo test` now covers this twice, and neither check uses a stopwatch:

- `concurrent_requests_do_not_queue_behind_each_other` is a barrier: request A's
  handler blocks until request B's handler releases it. Reintroducing a shared
  lock makes B unreachable, A times out, and the test fails in 10 s with
  `peer request never arrived`. Verified by putting the lock back before keeping
  the test: it failed, and passed again once removed.
- `every_request_builds_its_own_server` counts handler constructions, so the
  "one server per request" shape cannot be swapped for a lock that still looks
  parallel from the outside.

`TEST_SCRIPTS/multi-client-stress.ps1` covers the other half of the claim - that
concurrent callers each get *their own* answer rather than a neighbour's. Ten
independent clients run their own query in parallel and every returned row must
carry that client's extension; a total may drift by up to 5% because Everything
re-indexes while the test runs, but a total from somebody else's query fails.
Verified by deliberately expecting the wrong extension: the run exits 1 with
`expected .png, got SKILL.md`. A clean run of 200 calls holds p95 under 90 ms.

`TEST_SCRIPTS/parallel-probe.ps1` repeats the measurement against a live index
(slow solo, fast solo, fast while slow) and fails if a concurrent fast call
exceeds `max(150 ms, 5x solo)`. It is the release check, not CI: it needs Windows
and a running Everything.

Stdio dispatch is intentionally still synchronous and cannot process cancellation
while a tool call runs. The deadline is per HTTP request, not an atomic snapshot
or a whole multi-page tool deadline. Source labels, scope/accuracy fields, null
cursors when no common continuation exists, and the explicit probe union are
intentional response-contract changes; cached tool schemas must be refreshed.
