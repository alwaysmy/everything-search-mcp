//! everything-search-mcp - fast Everything file search over MCP, no Python.
//!
//! Two front ends over one implementation:
//!   * no arguments  -> MCP stdio server (how a client spawns it)
//!   * a subcommand  -> one-shot CLI (see `cli.rs`), usable with no MCP wiring
//!
//! Answers come from voidtools Everything's index over its loopback HTTP server.
//! No `es.exe` subprocess, no SDK DLL, no Python runtime.
//!
//! The same implementation also ships as an Everything 1.5 plugin (`src/plugin/`),
//! where Everything loads it in-process and the MCP endpoint is served from there.

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = everything_mcp_core::cli::dispatch(&argv) {
        std::process::exit(code);
    }

    // No arguments: an MCP session on stdin/stdout.
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
