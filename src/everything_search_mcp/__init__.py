"""
Everything MCP - The definitive MCP server for voidtools Everything.

Lightning-fast file search for AI agents.
"""

__version__ = "1.1.0"


def main() -> None:
    """Entry point for the ``everything-search-mcp`` command."""
    from everything_search_mcp.server import mcp

    mcp.run()
