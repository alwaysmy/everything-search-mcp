"""Live smoke test against a real Everything instance (not collected by pytest).

Run manually on a Windows machine with Everything running, or via the
live-smoke GitHub Actions workflow:

    python tests/live_smoke.py

Exercises the real es.exe paths that unit tests can only mock: multi-term
AND queries in search(), count() and get_total_size() (issues #2 and #4).
"""

from __future__ import annotations

import asyncio
import sys

from everything_mcp.backend import EverythingBackend
from everything_mcp.config import EverythingConfig

MULTI_TERM_QUERY = r"ext:exe path:C:\Windows"


async def main() -> int:
    config = EverythingConfig.auto_detect()
    print(f"es_path={config.es_path!r} instance={config.instance!r}")
    if not config.is_valid:
        print(f"FAIL: config invalid: {config.errors}")
        return 1
    print(f"version: {config.version_info}")

    backend = EverythingBackend(config)
    failures = 0

    results = await backend.search("explorer.exe", max_results=10)
    print(f"single-term search: {len(results)} results")
    if not results:
        print("FAIL: single-term search returned nothing")
        failures += 1

    results = await backend.search(MULTI_TERM_QUERY, max_results=10)
    print(f"multi-term search:  {len(results)} results")
    if not results:
        print("FAIL: multi-term AND search returned nothing (issue #2 regression)")
        failures += 1

    count = await backend.count(MULTI_TERM_QUERY)
    print(f"multi-term count:   {count}")
    if count <= 0:
        print("FAIL: multi-term count not positive (issue #4 regression)")
        failures += 1

    size = await backend.get_total_size(MULTI_TERM_QUERY)
    print(f"multi-term size:    {size}")
    if size <= 0 or size >= 2**63:
        print("FAIL: multi-term total size implausible (issue #4 regression)")
        failures += 1

    print("OK" if failures == 0 else f"{failures} check(s) failed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
