//! everything-search-mcp - fast Everything file search over MCP, no Python.
//!
//! Speaks the MCP stdio protocol directly and answers from voidtools Everything's
//! index over its loopback HTTP server. No `es.exe` subprocess, no SDK DLL,
//! no Python runtime.

mod everything;
mod jsonrpc;
mod tools;

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("everything-search-mcp {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "everything-search-mcp {}\n\n\
             An MCP stdio server for voidtools Everything (Windows).\n\n\
             Environment:\n  \
             EVERYTHING_HTTP_URL       Everything HTTP server base URL (default http://127.0.0.1:23333)\n  \
             EVERYTHING_TIMEOUT        request timeout in seconds (default 30)\n  \
             EVERYTHING_MAX_RESULTS_CAP  hard cap on results per search (default 1000)\n",
            env!("CARGO_PKG_VERSION")
        );
        return;
    }

    let cfg = everything::Config::from_env();
    let client = everything::Client::new(cfg);
    let handler = tools::Tools::new(client);

    if let Err(e) = jsonrpc::serve(handler) {
        let _ = writeln!(std::io::stderr(), "everything-search-mcp: {e}");
        std::process::exit(1);
    }
}
