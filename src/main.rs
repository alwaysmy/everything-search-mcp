//! everything-search-mcp - fast Everything file search over MCP, no Python.
//!
//! One front end: an MCP stdio server that a client spawns. There is no CLI —
//! the previous release had one, and it was removed because two ways to ask the
//! same question drifted apart (flags the tools accepted, the binary did not).
//! For a script or a client without MCP, ask Everything's own HTTP API directly;
//! `REFERENCE.md` in the skill has the four-line version.
//!
//! Answers come from voidtools Everything's index over its loopback HTTP server.
//! No `es.exe` subprocess, no SDK DLL, no Python runtime.
//!
//! The same implementation also ships as an Everything 1.5 plugin (`src/plugin/`),
//! where Everything loads it in-process and the MCP endpoint is served from there.

fn main() {
    // No CLI: refuse arguments instead of starting an MCP session and appearing to
    // hang. A client spawns this with no arguments, so this only catches a person or
    // an old script that still expects `search` / `config`.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if !argv.is_empty() {
        eprintln!(
            "everything-search-mcp: this build has no command line (got '{}').\n\
             It is an MCP stdio server; a client spawns it with no arguments.\n\
             For a one-off search, ask Everything directly:\n\
             \x20 http://127.0.0.1:23333/?search=ext:py&json=1&count=20&path_column=1",
            argv.join(" ")
        );
        std::process::exit(2);
    }

    let client = everything_mcp_core::everything::Client::new(
        everything_mcp_core::everything::Config::from_env(),
    );
    if let Err(e) =
        everything_mcp_core::jsonrpc::serve(everything_mcp_core::tools::Tools::new(client))
    {
        eprintln!("everything-search-mcp: {e}");
        std::process::exit(1);
    }
}
