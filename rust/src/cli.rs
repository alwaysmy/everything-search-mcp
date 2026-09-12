//! One-shot command line mode.
//!
//! The binary is an MCP stdio server when it is started with no arguments, which
//! is how a client spawns it. Given a subcommand it instead runs a single query
//! and prints the result, so the same executable is usable as a plain CLI - in a
//! skill, a script or a terminal - with no MCP wiring at all.
//!
//! Both paths run the identical tool code, so there is no second implementation to
//! drift.

use crate::everything::{Client, Config};
use crate::jsonrpc::Handler;
use crate::tools::Tools;
use serde_json::{json, Map, Value};

const USAGE: &str = "\
everything-search-mcp - Everything file search, as an MCP server or a one-shot CLI

  everything-search-mcp                              run as an MCP stdio server
  everything-search-mcp search <query> [flags]       find files and folders
  everything-search-mcp recent [flags]               files changed recently
  everything-search-mcp count <query> [flags]        exact count, optional size
  everything-search-mcp details <path>... [flags]    metadata and a triage preview
  everything-search-mcp config [flags]               print or apply the MCP client config
                                                     for this exe (see `config --help`)
  everything-search-mcp --version | --help

Flags
  --path <dir>        restrict to a directory tree
  --category <name>   audio video image document code archive executable font 3d data
  --type <t>          file | folder | any
  --max <n>           max results, 1-500 (default 50 for search, 20 for the CLI)
  --offset <n>        skip n results
  --sort <name>       any of the 14 sort names, default date-modified-desc
  --per-parent <n>    keep at most n hits per parent directory
  --regex             treat the query as a regular expression
  --case              case sensitive
  --whole-word        whole words only
  --match-path        match against the full path
  --total             also print the total match count
  --probe             also look for a bare term in paths and folder names
  --period <p>        recent: 1min 5min 10min 15min 30min 1hour 2hours 6hours 12hours
                      today yesterday 1day 3days 1week 2weeks 1month 3months 6months
                      1year, or raw syntax such as last2hours
  --ext <list>        recent: extensions, e.g. py,js or py;js
  --preview <n>       details: triage preview lines, 0-200
  --exact-size        count: sum every match for an exact total size
  --breakdown         count: break the result set down by extension
  --json              print structuredContent instead of the human-readable text
";

/// Returns the process exit code when a subcommand was handled, or `None` when
/// this is an MCP session.
pub fn dispatch(argv: &[String]) -> Option<i32> {
    let cmd = argv.first().map(|s| s.as_str()).unwrap_or("");
    if cmd.is_empty() {
        return None;
    }
    match cmd {
        "--version" | "-V" => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Some(0)
        }
        "--help" | "-h" | "help" => {
            print!("{USAGE}");
            Some(0)
        }
        "search" | "recent" | "count" | "details" => Some(run(cmd, &argv[1..])),
        "config" | "mcp-config" => Some(crate::setup::dispatch(&argv[1..])),
        other => {
            eprintln!("everything-search-mcp: unknown command '{other}'\n");
            eprint!("{USAGE}");
            Some(2)
        }
    }
}

fn run(cmd: &str, rest: &[String]) -> i32 {
    let mut args = Map::new();
    let mut positional: Vec<String> = Vec::new();
    let mut i = 0;
    let need = |i: usize, rest: &[String], flag: &str| -> Result<String, String> {
        rest.get(i + 1)
            .cloned()
            .ok_or_else(|| format!("{flag} needs a value"))
    };
    while i < rest.len() {
        let a = rest[i].as_str();
        let r = match a {
            "--path" => need(i, rest, a).map(|v| { args.insert("path".into(), json!(v)); i += 2; }),
            "--category" => need(i, rest, a).map(|v| { args.insert("category".into(), json!(v)); i += 2; }),
            "--type" => need(i, rest, a).map(|v| { args.insert("entry_type".into(), json!(v)); i += 2; }),
            "--sort" => need(i, rest, a).map(|v| { args.insert("sort".into(), json!(v)); i += 2; }),
            "--period" => need(i, rest, a).map(|v| { args.insert("period".into(), json!(v)); i += 2; }),
            "--ext" | "--extensions" => need(i, rest, a).map(|v| { args.insert("extensions".into(), json!(v)); i += 2; }),
            "--max" | "--max-results" => need(i, rest, a).and_then(|v| {
                v.parse::<u64>().map(|n| { args.insert("max_results".into(), json!(n)); i += 2; })
                    .map_err(|_| format!("{a} needs a number, got '{v}'"))
            }),
            "--offset" => need(i, rest, a).and_then(|v| {
                v.parse::<u64>().map(|n| { args.insert("offset".into(), json!(n)); i += 2; })
                    .map_err(|_| format!("{a} needs a number, got '{v}'"))
            }),
            "--per-parent" => need(i, rest, a).and_then(|v| {
                v.parse::<u64>().map(|n| { args.insert("max_per_parent".into(), json!(n)); i += 2; })
                    .map_err(|_| format!("{a} needs a number, got '{v}'"))
            }),
            "--preview" | "--preview-lines" => need(i, rest, a).and_then(|v| {
                v.parse::<u64>().map(|n| { args.insert("preview_lines".into(), json!(n)); i += 2; })
                    .map_err(|_| format!("{a} needs a number, got '{v}'"))
            }),
            "--regex" => { args.insert("match_regex".into(), json!(true)); i += 1; Ok(()) }
            "--case" => { args.insert("match_case".into(), json!(true)); i += 1; Ok(()) }
            "--whole-word" => { args.insert("match_whole_word".into(), json!(true)); i += 1; Ok(()) }
            "--match-path" => { args.insert("match_path".into(), json!(true)); i += 1; Ok(()) }
            "--total" => { args.insert("include_total".into(), json!(true)); i += 1; Ok(()) }
            "--probe" => { args.insert("probe".into(), json!(true)); i += 1; Ok(()) }
            "--exact-size" => { args.insert("exact_size".into(), json!(true)); i += 1; Ok(()) }
            "--breakdown" => { args.insert("breakdown_by_extension".into(), json!(true)); i += 1; Ok(()) }
            "--json" => { args.insert("__json".into(), json!(true)); i += 1; Ok(()) }
            "--no-expand" => { args.insert("auto_expand".into(), json!(false)); i += 1; Ok(()) }
            other if other.starts_with("--") => Err(format!("unknown flag '{other}'")),
            _ => { positional.push(rest[i].clone()); i += 1; Ok(()) }
        };
        if let Err(e) = r {
            eprintln!("everything-search-mcp: {e}");
            return 2;
        }
    }

    let want_json = args.remove("__json").is_some();
    let joined = positional.join(" ");
    let (tool, default_max) = match cmd {
        "search" => {
            if joined.trim().is_empty() {
                args.entry("query").or_insert(json!(""));
            } else {
                args.insert("query".into(), json!(joined));
            }
            ("everything_search", 20)
        }
        "recent" => {
            if !joined.trim().is_empty() {
                args.insert("query".into(), json!(joined));
            }
            ("everything_find_recent", 20)
        }
        "count" => {
            // an empty query is legitimate when a category or entry_type is given
            args.insert("query".into(), json!(joined));
            ("everything_count_stats", 1)
        }
        "details" => {
            if positional.is_empty() {
                eprintln!("everything-search-mcp: details needs at least one path");
                return 2;
            }
            args.insert("paths".into(), json!(positional));
            ("everything_file_details", 0)
        }
        _ => unreachable!(),
    };
    args.entry("max_results").or_insert(json!(default_max));

    let mut tools = Tools::new(Client::new(Config::from_env()));
    match tools.call_tool(tool, &Value::Object(args)) {
        Ok(out) => {
            if want_json {
                println!("{}", serde_json::to_string_pretty(&out.structured).unwrap_or_default());
            } else {
                print!("{}", out.text);
            }
            0
        }
        Err(e) => {
            eprintln!("everything-search-mcp: {e}");
            1
        }
    }
}
