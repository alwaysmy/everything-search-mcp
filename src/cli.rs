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
use crate::servers;
use crate::tools::Tools;
use serde_json::{json, Map, Value};

const USAGE: &str = "\
everything-search-mcp - Everything file search, as an MCP server or a one-shot CLI

  everything-search-mcp                              run as an MCP stdio server
  everything-search-mcp search <query> [flags]       find files and folders
  everything-search-mcp recent [flags]               files changed recently
  everything-search-mcp count <query> [flags]        exact count, optional size
  everything-search-mcp details <path>... [flags]    metadata and a triage preview
  everything-search-mcp servers [list|add|remove|enable|disable]
                                                     manage the remote instances
  everything-search-mcp config [flags]               print or apply the MCP client config
                                                     for this exe (see `config --help`)
  everything-search-mcp --version | --help

Flags
  --url <name|address>  search this Everything instance instead of the default set.
                      Repeat the flag to search several at once; results are grouped
                      per machine. \"local\" is this machine. `servers list` shows the
                      registered names. Without it, every enabled instance is searched.
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
        "servers" | "server" => Some(servers_cmd(&argv[1..])),
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
            // Repeatable: each occurrence adds an instance, so
            // `--url local --url nas` searches both. Always an array, which is one of
            // the two shapes the argument accepts, so there is no special case.
            "--url" | "--server" => need(i, rest, a).map(|v| {
                match args.entry("url").or_insert_with(|| json!([])).as_array_mut() {
                    Some(list) => list.push(json!(v)),
                    None => unreachable!("url is only ever written as an array here"),
                }
                i += 2;
            }),
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

// ---------------------------------------------------------------- servers

const SERVERS_USAGE: &str = "\
Manage the Everything instances a search can run against.

  everything-search-mcp servers list                 show the registry and its switches
  everything-search-mcp servers path                 print the registry file location
  everything-search-mcp servers add <name> <url> [--disabled]
                                                     register an instance (on by default)
  everything-search-mcp servers remove <name>        unregister it
  everything-search-mcp servers enable <name>        switch it on
  everything-search-mcp servers disable <name>       switch it off, keeping its entry

\"local\" is built in and always available; registering it is how this machine gets
its own switch and address. Searches run against every enabled instance unless a
call names one with url / --url.
";

fn servers_cmd(rest: &[String]) -> i32 {
    let sub = rest.first().map(|s| s.as_str()).unwrap_or("list");
    match sub {
        "list" | "ls" => servers_list(),
        "path" | "where" => match servers::path() {
            Ok(p) => {
                println!("{}", p.display());
                0
            }
            Err(e) => fail(&e),
        },
        "add" => servers_add(rest),
        "remove" | "rm" | "delete" => servers_remove(rest),
        "enable" | "disable" => servers_toggle(rest, sub == "enable"),
        "--help" | "-h" | "help" => {
            print!("{SERVERS_USAGE}");
            0
        }
        other => {
            eprintln!("everything-search-mcp: unknown servers command '{other}'\n");
            eprint!("{SERVERS_USAGE}");
            2
        }
    }
}

fn fail(msg: &str) -> i32 {
    eprintln!("everything-search-mcp: {msg}");
    1
}

fn load_registry() -> Result<servers::Registry, i32> {
    servers::Registry::load().map_err(|e| {
        eprintln!("everything-search-mcp: {e}");
        1
    })
}

fn save_registry(reg: &servers::Registry) -> Result<(), i32> {
    match reg.save() {
        Ok(_) => Ok(()),
        Err(e) => Err(fail(&e)),
    }
}

fn servers_list() -> i32 {
    let p = match servers::path() {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    println!("registry: {}", p.display());
    let reg = match load_registry() {
        Ok(r) => r,
        Err(code) => return code,
    };
    let mut rows = 0;
    for s in &reg.servers {
        println!(
            "  {:<20} {:<44} {}",
            s.name,
            s.url,
            if s.enabled { "on" } else { "off" }
        );
        rows += 1;
    }
    // The built-in local instance is not a row in the file until it is registered, so
    // it is listed separately. Showing it only when unregistered keeps the list
    // honest: a registered `local` already appeared above with its own switch.
    if reg.get(servers::LOCAL).is_none() {
        println!(
            "  {:<20} {:<44} {}",
            servers::LOCAL,
            servers::local_url(),
            "on (built in)"
        );
        rows += 1;
    }
    if rows == 0 {
        println!("  (nothing registered)");
    }
    println!("\nSearches run against every instance that is on, unless a call names one.");
    0
}

fn servers_add(rest: &[String]) -> i32 {
    let name = rest.get(1).map(|s| s.trim()).unwrap_or("");
    let url = rest.get(2).map(|s| s.trim()).unwrap_or("");
    if name.is_empty() || url.is_empty() {
        eprintln!("everything-search-mcp: servers add needs a name and an address");
        eprint!("{SERVERS_USAGE}");
        return 2;
    }
    if let Err(e) = servers::validate_name(name) {
        return fail(&e);
    }
    // Validate the address before storing it: a registry entry that cannot be parsed
    // would break every later search, not just the one that used it.
    if let Err(e) = Config::from_url(url) {
        return fail(&e);
    }
    let disabled = rest.iter().any(|a| a == "--disabled" || a == "--off");
    let mut reg = match load_registry() {
        Ok(r) => r,
        Err(code) => return code,
    };
    // A new entry starts switched on, which is what registering it means. An existing
    // one keeps the switch it had, so re-pointing an address does not quietly revive a
    // machine that was turned off.
    let added = reg.upsert(name, url, if disabled { Some(false) } else { None });
    if let Err(code) = save_registry(&reg) {
        return code;
    }
    let state = reg.get(name).map(|e| e.enabled).unwrap_or(true);
    println!(
        "{} {name} -> {url} ({})",
        if added { "added" } else { "updated" },
        if state { "on" } else { "off" }
    );
    0
}

fn servers_remove(rest: &[String]) -> i32 {
    let name = rest.get(1).map(|s| s.trim()).unwrap_or("");
    if name.is_empty() {
        eprintln!("everything-search-mcp: servers remove needs a name");
        return 2;
    }
    let mut reg = match load_registry() {
        Ok(r) => r,
        Err(code) => return code,
    };
    if !reg.remove(name) {
        return fail(&format!(
            "no server named '{name}' is registered. `servers list` shows what is."
        ));
    }
    if let Err(code) = save_registry(&reg) {
        return code;
    }
    println!("removed {name}");
    0
}

fn servers_toggle(rest: &[String], on: bool) -> i32 {
    let name = rest.get(1).map(|s| s.trim()).unwrap_or("");
    if name.is_empty() {
        eprintln!("everything-search-mcp: servers {} needs a name", if on { "enable" } else { "disable" });
        return 2;
    }
    let mut reg = match load_registry() {
        Ok(r) => r,
        Err(code) => return code,
    };
    match reg.get_mut(name) {
        Some(e) => e.enabled = on,
        None => {
            // Registering it here would guess an address, so the caller is told what
            // is available instead.
            let known = reg.names();
            return fail(&format!(
                "no server named '{name}' is registered{}. Add it with `servers add \
                 {name} <address>` first.",
                if known.is_empty() {
                    String::new()
                } else {
                    format!(" (registered: {})", known.join(", "))
                }
            ));
        }
    }
    if let Err(code) = save_registry(&reg) {
        return code;
    }
    println!("{name}: {}", if on { "on" } else { "off" });
    0
}
