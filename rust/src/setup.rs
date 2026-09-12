//! `config` subcommand: print, or apply, the MCP client configuration for this
//! executable.
//!
//! The binary knows its own absolute path, so a client config never has to be
//! hand-written - and it cannot go stale when the skill directory moves. That is
//! the whole point: the executable ships inside a skill folder, and every agent on
//! the machine should be able to point at it without a human editing JSON.
//!
//! There is no `es.exe` path to configure. This server talks to Everything's HTTP
//! API and to nothing else, so the only knobs are the HTTP URL and two limits.

use crate::everything::{format_unix, Client, Config as HttpConfig, Query};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

const DSH_CLIENT: &str = "@deepseek-ai/dsh-mcp-client";
const DEFAULT_NAME: &str = "everything";

const USAGE: &str = "\
everything-search-mcp config [options]      print or apply MCP client configuration

Targets - default is every target whose config file already exists on this machine
  dsh             DeepSeek Harness   %USERPROFILE%\\.dsh\\cordis.patch.yml
  claude          Claude Code        %USERPROFILE%\\.claude.json
  claude-desktop  Claude Desktop     %APPDATA%\\Claude\\claude_desktop_config.json
  codex           Codex CLI          %USERPROFILE%\\.codex\\config.toml
  gemini          Gemini CLI         %USERPROFILE%\\.gemini\\settings.json
  cursor          Cursor             %USERPROFILE%\\.cursor\\mcp.json
  vscode          VS Code            %APPDATA%\\Code\\User\\mcp.json
  json            no file - just print a generic mcpServers block

Options
  --target <t>          target name, repeatable or comma separated
  --file <path>         use this file instead of the target's own; with no
                        --target the format is inferred from the extension
  --name <server>       name to register the server under (default: everything)
  --http-url <url>      pin EVERYTHING_HTTP_URL (the default is already
                        http://127.0.0.1:23333, so this is rarely needed)
  --timeout <seconds>   pin EVERYTHING_TIMEOUT (default 30)
  --max-results-cap <n> pin EVERYTHING_MAX_RESULTS_CAP (default 1000)
  --write               apply the change; every existing file is backed up first.
                        Without it nothing is touched and the snippet is printed
  --no-check            skip the liveness probe of the Everything HTTP server
  --json                machine readable report instead of prose

Printing instead of writing is the default on purpose: this reads your agent
config files, and a tool that silently rewrites them is a tool you cannot trust.
";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// DeepSeek Harness loader patch entry (YAML).
    Dsh,
    /// `mcpServers` entry carrying an explicit `"type": "stdio"`.
    JsonStdio,
    /// `mcpServers` entry without `type` - Claude Desktop, Gemini, Cursor.
    JsonPlain,
    /// VS Code user `mcp.json`, which nests under `servers`.
    VsCode,
    /// Codex CLI `config.toml`.
    Codex,
    /// No config file at all; the snippet goes to stdout.
    Stdout,
}

struct Target {
    id: &'static str,
    label: &'static str,
    file: Option<PathBuf>,
    kind: Kind,
    note: &'static str,
}

#[derive(Default)]
struct Opts {
    targets: Vec<String>,
    file: Option<PathBuf>,
    name: String,
    env: Vec<(String, String)>,
    write: bool,
    check: bool,
    json: bool,
}

pub fn dispatch(rest: &[String]) -> i32 {
    let mut opts = Opts { name: DEFAULT_NAME.to_string(), check: true, ..Opts::default() };
    let need = |i: usize, flag: &str| -> Result<String, String> {
        rest.get(i + 1).cloned().ok_or_else(|| format!("{flag} needs a value"))
    };
    let mut i = 0;
    while i < rest.len() {
        let a = rest[i].as_str();
        let r: Result<(), String> = match a {
            "--target" | "--targets" | "-t" => need(i, a).map(|v| {
                for part in v.split(',') {
                    let p = part.trim();
                    if !p.is_empty() {
                        opts.targets.push(p.to_string());
                    }
                }
                i += 2;
            }),
            "--file" | "--out" => need(i, a).map(|v| {
                opts.file = Some(PathBuf::from(v));
                i += 2;
            }),
            "--name" => need(i, a).map(|v| {
                opts.name = v;
                i += 2;
            }),
            "--http-url" => need(i, a).map(|v| {
                opts.env.push(("EVERYTHING_HTTP_URL".into(), v));
                i += 2;
            }),
            "--timeout" => need(i, a).and_then(|v| {
                numeric(&v, a)?;
                opts.env.push(("EVERYTHING_TIMEOUT".into(), v));
                i += 2;
                Ok(())
            }),
            "--max-results-cap" => need(i, a).and_then(|v| {
                numeric(&v, a)?;
                opts.env.push(("EVERYTHING_MAX_RESULTS_CAP".into(), v));
                i += 2;
                Ok(())
            }),
            "--write" => {
                opts.write = true;
                i += 1;
                Ok(())
            }
            "--no-check" => {
                opts.check = false;
                i += 1;
                Ok(())
            }
            "--json" => {
                opts.json = true;
                i += 1;
                Ok(())
            }
            "--help" | "-h" => {
                print!("{USAGE}");
                return 0;
            }
            other => Err(format!("unknown flag '{other}'")),
        };
        if let Err(e) = r {
            eprintln!("everything-search-mcp: {e}\n");
            eprint!("{USAGE}");
            return 2;
        }
    }

    let exe = match exe_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("everything-search-mcp: {e}");
            return 1;
        }
    };
    run(&opts, &exe)
}

fn numeric(v: &str, flag: &str) -> Result<(), String> {
    if v.is_empty() || !v.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("{flag} needs a number, got '{v}'"));
    }
    Ok(())
}

fn exe_path() -> Result<String, String> {
    let p = std::env::current_exe().map_err(|e| format!("cannot locate this executable: {e}"))?;
    let p = fs::canonicalize(&p).unwrap_or(p);
    let s = p.to_string_lossy().into_owned();
    Ok(match s.strip_prefix(r"\\?\") {
        Some(rest) => rest.to_string(),
        None => s,
    })
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn all_targets() -> Vec<Target> {
    let home = home();
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    vec![
        Target {
            id: "dsh",
            label: "DeepSeek Harness",
            file: Some(home.join(".dsh").join("cordis.patch.yml")),
            kind: Kind::Dsh,
            note: "a top-level loader patch entry; DSH reads it at startup, so restart DSH",
        },
        Target {
            id: "claude",
            label: "Claude Code",
            file: Some(home.join(".claude.json")),
            kind: Kind::JsonStdio,
            note: "restart Claude Code",
        },
        Target {
            id: "claude-desktop",
            label: "Claude Desktop",
            file: appdata.as_ref().map(|a| a.join("Claude").join("claude_desktop_config.json")),
            kind: Kind::JsonPlain,
            note: "quit Claude Desktop from the tray and start it again",
        },
        Target {
            id: "codex",
            label: "Codex CLI",
            file: Some(home.join(".codex").join("config.toml")),
            kind: Kind::Codex,
            note: "restart Codex",
        },
        Target {
            id: "gemini",
            label: "Gemini CLI",
            file: Some(home.join(".gemini").join("settings.json")),
            kind: Kind::JsonPlain,
            note: "restart Gemini CLI",
        },
        Target {
            id: "cursor",
            label: "Cursor",
            file: Some(home.join(".cursor").join("mcp.json")),
            kind: Kind::JsonPlain,
            note: "restart Cursor",
        },
        Target {
            id: "vscode",
            label: "VS Code",
            file: appdata.as_ref().map(|a| a.join("Code").join("User").join("mcp.json")),
            kind: Kind::VsCode,
            note: "run 'Developer: Reload Window'",
        },
        Target {
            id: "json",
            label: "generic mcpServers block",
            file: None,
            kind: Kind::Stdout,
            note: "paste into your client's own MCP configuration",
        },
    ]
}

fn infer_kind(file: &Path) -> Kind {
    let n = file
        .file_name()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if n.ends_with(".yml") || n.ends_with(".yaml") {
        Kind::Dsh
    } else if n.ends_with(".toml") {
        Kind::Codex
    } else {
        Kind::JsonPlain
    }
}

fn select<'a>(all: &'a [Target], opts: &Opts) -> Result<Vec<&'a Target>, String> {
    if opts.targets.is_empty() {
        let existing: Vec<&Target> =
            all.iter().filter(|t| t.file.as_ref().is_some_and(|f| f.exists())).collect();
        return Ok(if existing.is_empty() { all.iter().collect() } else { existing });
    }
    let mut out: Vec<&Target> = Vec::new();
    for want in &opts.targets {
        let exact: Vec<&Target> = all.iter().filter(|t| t.id == want.as_str()).collect();
        let hits: Vec<&Target> = if exact.is_empty() {
            all.iter().filter(|t| t.id.starts_with(want.as_str())).collect()
        } else {
            exact
        };
        if hits.is_empty() {
            return Err(format!(
                "unknown target '{want}'. Known targets: {}",
                all.iter().map(|t| t.id).collect::<Vec<_>>().join(", ")
            ));
        }
        for h in hits {
            if !out.iter().any(|o| o.id == h.id) {
                out.push(h);
            }
        }
    }
    Ok(out)
}

struct Row<'a> {
    target: &'a Target,
    file: Option<PathBuf>,
    report: Report,
    snippet: String,
}

fn run(opts: &Opts, exe: &str) -> i32 {
    let mut all = all_targets();
    // `--file` on its own means "configure this file", with the format guessed
    // from its extension; `--file` together with `--target` just redirects the
    // target's own path.
    let standalone = opts.file.is_some() && opts.targets.is_empty();
    if let Some(f) = opts.file.as_ref().filter(|_| standalone) {
        all.push(Target {
            id: "file",
            label: "explicit --file",
            kind: infer_kind(f),
            file: Some(f.clone()),
            note: "restart the client",
        });
    }
    let selected: Vec<&Target> = if standalone {
        vec![all.last().expect("just pushed")]
    } else {
        match select(&all, opts) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("everything-search-mcp: {e}");
                return 2;
            }
        }
    };
    if opts.file.is_some() && selected.len() > 1 {
        eprintln!("everything-search-mcp: --file needs exactly one target");
        return 2;
    }

    let http = HttpConfig::from_env();
    let ready = if opts.check { Some(probe()) } else { None };

    let mut rows: Vec<Row> = Vec::new();
    let mut failed = false;
    for &t in &selected {
        let snippet = render(t, exe, &opts.name, &opts.env);
        let file = opts.file.clone().or_else(|| t.file.clone());
        let outcome = if opts.write {
            match file.clone() {
                None => Err("this target has no config file; copy the snippet by hand".to_string()),
                Some(f) => apply(t, &f, exe, &opts.name, &opts.env),
            }
        } else {
            Ok(Report::Printed)
        };
        let report = match outcome {
            Ok(r) => r,
            Err(e) => {
                failed = true;
                Report::Failed(e)
            }
        };
        rows.push(Row { target: t, file, report, snippet });
    }

    if opts.json {
        print_json(exe, &http, &ready, &rows, opts);
    } else {
        print_prose(exe, &http, &ready, &rows, opts);
    }
    if failed {
        1
    } else {
        0
    }
}

/// One empty search. Everything answers from its index, so the total is the
/// number of objects it currently has indexed - a cheap liveness check.
fn probe() -> Result<u64, String> {
    let client = Client::new(HttpConfig::from_env());
    let q = Query {
        search: "",
        count: 1,
        offset: 0,
        sort: "date-modified-desc",
        ascending: false,
        case: false,
        whole_word: false,
        regex: false,
        match_path: false,
        path: None,
    };
    client.query(&q).map(|r| r.total_results)
}

// ------------------------------------------------------------------ snippets

fn render(t: &Target, exe: &str, name: &str, env: &[(String, String)]) -> String {
    match t.kind {
        Kind::Dsh => format!("- insert:\n{}", dsh_block(exe, name, env, "    ")),
        Kind::Codex => codex_block(exe, name, env),
        kind => json_block(kind, exe, name, env),
    }
}

fn entry_json(kind: Kind, exe: &str, env: &[(String, String)]) -> Value {
    let mut m = Map::new();
    if matches!(kind, Kind::JsonStdio | Kind::VsCode) {
        m.insert("type".into(), json!("stdio"));
    }
    m.insert("command".into(), json!(exe));
    m.insert("args".into(), json!([]));
    if !env.is_empty() {
        let mut e = Map::new();
        for (k, v) in env {
            e.insert(k.clone(), json!(v));
        }
        m.insert("env".into(), Value::Object(e));
    }
    Value::Object(m)
}

fn json_block(kind: Kind, exe: &str, name: &str, env: &[(String, String)]) -> String {
    let container = if kind == Kind::VsCode { "servers" } else { "mcpServers" };
    let mut inner = Map::new();
    inner.insert(name.to_string(), entry_json(kind, exe, env));
    let mut root = Map::new();
    root.insert(container.to_string(), Value::Object(inner));
    let mut s = serde_json::to_string_pretty(&Value::Object(root)).unwrap_or_default();
    s.push('\n');
    s
}

/// A DSH loader patch entry. `base` is the indentation of the `- id:` line, so
/// the same generator serves both a standalone snippet (base is four spaces,
/// nested under a fresh `- insert:`) and an in-place rewrite of an existing entry.
fn dsh_block(exe: &str, name: &str, env: &[(String, String)], base: &str) -> String {
    let exe = exe.replace('\\', "/");
    let mut s = String::new();
    s.push_str(&format!("{base}- id: mcp-{name}\n"));
    s.push_str(&format!("{base}  name: '{DSH_CLIENT}'\n"));
    s.push_str(&format!("{base}  config:\n"));
    s.push_str(&format!("{base}    serverName: {name}\n"));
    s.push_str(&format!("{base}    transport: stdio\n"));
    s.push_str(&format!("{base}    command: {exe}\n"));
    if !env.is_empty() {
        s.push_str(&format!("{base}    env:\n"));
        for (k, v) in env {
            s.push_str(&format!("{base}      {k}: {v}\n"));
        }
    }
    s
}

fn codex_block(exe: &str, name: &str, env: &[(String, String)]) -> String {
    // TOML literal strings: single quotes keep the backslashes literal.
    let mut s = format!("[mcp_servers.{name}]\ncommand = '{exe}'\nargs = []\n");
    if !env.is_empty() {
        s.push_str(&format!("\n[mcp_servers.{name}.env]\n"));
        for (k, v) in env {
            s.push_str(&format!("{k} = '{v}'\n"));
        }
    }
    s
}

// ------------------------------------------------------------------- writing

enum Report {
    Printed,
    Created,
    Updated(Option<PathBuf>),
    Unchanged,
    Failed(String),
}

impl Report {
    fn verb(&self) -> &'static str {
        match self {
            Report::Printed => "printed",
            Report::Created => "created",
            Report::Updated(_) => "updated",
            Report::Unchanged => "unchanged",
            Report::Failed(_) => "FAILED",
        }
    }

    fn backup(&self) -> Option<&Path> {
        match self {
            Report::Updated(b) => b.as_deref(),
            _ => None,
        }
    }

    fn error(&self) -> Option<&str> {
        match self {
            Report::Failed(e) => Some(e),
            _ => None,
        }
    }
}

fn apply(
    t: &Target,
    file: &Path,
    exe: &str,
    name: &str,
    env: &[(String, String)],
) -> Result<Report, String> {
    match t.kind {
        Kind::Dsh => {
            let new = dsh_replacement(file, exe, name, env)?;
            write_if_changed(file, &new)
        }
        Kind::Codex => {
            let new = codex_replacement(file, exe, name, env)?;
            write_if_changed(file, &new)
        }
        Kind::Stdout => Err("this target has no config file".into()),
        kind => apply_json(kind, file, exe, name, env),
    }
}

fn read_or_empty(file: &Path) -> Result<String, String> {
    match fs::read_to_string(file) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("cannot read {}: {e}", file.display())),
    }
}

fn write_if_changed(file: &Path, new: &str) -> Result<Report, String> {
    let old = read_or_empty(file)?;
    if old == new {
        return Ok(Report::Unchanged);
    }
    let existed = file.exists();
    let backup = if existed { Some(backup(file)?) } else { None };
    write_file(file, new)?;
    Ok(if existed { Report::Updated(backup) } else { Report::Created })
}

fn write_file(file: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = file.parent() {
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
    }
    fs::write(file, text).map_err(|e| format!("cannot write {}: {e}", file.display()))
}

fn backup(file: &Path) -> Result<PathBuf, String> {
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".bak-{}", utc_stamp()));
    let dest = file.with_file_name(name);
    fs::copy(file, &dest).map_err(|e| format!("cannot back up {}: {e}", file.display()))?;
    Ok(dest)
}

fn utc_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let s = format_unix(secs);
    if s.len() >= 19 {
        format!(
            "{}{}{}-{}{}{}Z",
            &s[0..4],
            &s[5..7],
            &s[8..10],
            &s[11..13],
            &s[14..16],
            &s[17..19]
        )
    } else {
        format!("{secs}Z")
    }
}

fn apply_json(
    kind: Kind,
    file: &Path,
    exe: &str,
    name: &str,
    env: &[(String, String)],
) -> Result<Report, String> {
    let container = if kind == Kind::VsCode { "servers" } else { "mcpServers" };
    let old = read_or_empty(file)?;
    let existed = file.exists();
    // Windows tools happily write a UTF-8 BOM; `serde_json` would reject it, so
    // it is peeled off here and put back on the way out.
    let (bom, body) = match old.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", old.as_str()),
    };
    let mut root: Value = if body.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(body)
            .map_err(|e| format!("{} is not valid JSON, leaving it alone: {e}", file.display()))?
    };
    let entry = entry_json(kind, exe, env);
    if existed && root.get(container).and_then(|c| c.get(name)) == Some(&entry) {
        return Ok(Report::Unchanged);
    }
    {
        let obj = root
            .as_object_mut()
            .ok_or_else(|| format!("{}: the top level is not a JSON object", file.display()))?;
        if !obj.contains_key(container) {
            obj.insert(container.to_string(), json!({}));
        }
        let slot = obj.get_mut(container).expect("just inserted");
        let map = slot
            .as_object_mut()
            .ok_or_else(|| format!("{}: '{container}' is not a JSON object", file.display()))?;
        map.insert(name.to_string(), entry);
    }
    let text = format!(
        "{bom}{}\n",
        serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?
    );
    let backup = if existed { Some(backup(file)?) } else { None };
    write_file(file, &text)?;
    Ok(if existed { Report::Updated(backup) } else { Report::Created })
}

/// Replace the loader entry for this server in place, or append a new
/// `- insert:` block. Line based on purpose: this file is hand-maintained YAML
/// full of comments, and re-serialising it would throw all of that away.
fn dsh_replacement(
    file: &Path,
    exe: &str,
    name: &str,
    env: &[(String, String)],
) -> Result<String, String> {
    let text = read_or_empty(file)?;
    let lines: Vec<&str> = text.lines().collect();
    let own = [format!("- id: mcp-{name}"), format!("- id: {name}")];
    let found = lines
        .iter()
        .position(|l| own.iter().any(|o| o.as_str() == l.trim()));
    if let Some(start) = found {
        let indent = lines[start].len() - lines[start].trim_start().len();
        // Cut at the last line that is genuinely part of the entry, so the blank
        // line separating it from the next top-level entry survives.
        let mut cut = start + 1;
        let mut scan = start + 1;
        while scan < lines.len() {
            let l = lines[scan];
            if !l.trim().is_empty() {
                if l.len() - l.trim_start().len() <= indent {
                    break;
                }
                cut = scan + 1;
            }
            scan += 1;
        }
        let block = dsh_block(exe, name, env, &" ".repeat(indent));
        let mut out = String::new();
        for l in &lines[..start] {
            out.push_str(l);
            out.push('\n');
        }
        out.push_str(&block);
        for l in &lines[cut..] {
            out.push_str(l);
            out.push('\n');
        }
        return Ok(out);
    }
    let mut out = text;
    if !out.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str(
        "# everything-search-mcp - added by `everything-search-mcp config --target dsh --write`\n",
    );
    out.push_str(&format!("- insert:\n{}", dsh_block(exe, name, env, "    ")));
    Ok(out)
}

fn codex_replacement(
    file: &Path,
    exe: &str,
    name: &str,
    env: &[(String, String)],
) -> Result<String, String> {
    let text = read_or_empty(file)?;
    let lines: Vec<&str> = text.lines().collect();
    let head = format!("[mcp_servers.{name}]");
    let env_head = format!("[mcp_servers.{name}.env]");
    let block = codex_block(exe, name, env);
    let found = lines.iter().position(|l| l.trim() == head || l.trim() == env_head);
    if let Some(start) = found {
        // In TOML every line up to the next table header belongs to this table,
        // so the replacement is the run of lines before that header.
        let mut end = start + 1;
        while end < lines.len() {
            let t = lines[end].trim();
            if t.starts_with('[') && t != head && t != env_head {
                break;
            }
            end += 1;
        }
        let mut out = String::new();
        for l in &lines[..start] {
            out.push_str(l);
            out.push('\n');
        }
        out.push_str(&block);
        if end < lines.len() {
            out.push('\n');
        }
        for l in &lines[end..] {
            out.push_str(l);
            out.push('\n');
        }
        return Ok(out);
    }
    let mut out = text;
    if !out.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str(&block);
    Ok(out)
}

// ------------------------------------------------------------------- reports

fn indent_block(s: &str, pad: &str) -> String {
    s.lines().map(|l| format!("{pad}{l}")).collect::<Vec<_>>().join("\n")
}

fn print_prose(
    exe: &str,
    http: &HttpConfig,
    ready: &Option<Result<u64, String>>,
    rows: &[Row],
    opts: &Opts,
) {
    println!("everything-search-mcp {}", env!("CARGO_PKG_VERSION"));
    println!("  executable  {exe}");
    println!("  transport   http://{}:{}", http.host, http.port);
    match ready {
        Some(Ok(n)) => println!("  Everything  reachable, {n} objects indexed"),
        Some(Err(e)) => {
            println!("  Everything  NOT reachable - {}", e.lines().next().unwrap_or(""))
        }
        None => {}
    }
    println!(
        "  mode        {}",
        if opts.write { "write" } else { "print only (add --write to apply)" }
    );
    for row in rows {
        let t = row.target;
        println!();
        println!("[{}] {}", t.id, t.label);
        if let Some(f) = &row.file {
            println!(
                "  file     {}{}",
                f.display(),
                if f.exists() { "" } else { "   (does not exist yet)" }
            );
        }
        println!("  action   {}", row.report.verb());
        if let Some(b) = row.report.backup() {
            println!("  backup   {}", b.display());
        }
        if let Some(e) = row.report.error() {
            println!("  error    {e}");
        }
        println!("  note     {}", t.note);
        if opts.write {
            continue;
        }
        println!("  --- begin snippet ---");
        println!("{}", indent_block(&row.snippet, "  "));
        println!("  --- end snippet ---");
    }
}

fn print_json(
    exe: &str,
    http: &HttpConfig,
    ready: &Option<Result<u64, String>>,
    rows: &[Row],
    opts: &Opts,
) {
    let targets: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "target": row.target.id,
                "label": row.target.label,
                "file": row.file.as_ref().map(|f| f.display().to_string()),
                "exists": row.file.as_ref().is_some_and(|f| f.exists()),
                "action": row.report.verb(),
                "backup": row.report.backup().map(|b| b.display().to_string()),
                "error": row.report.error(),
                "note": row.target.note,
                "snippet": row.snippet,
            })
        })
        .collect();
    let everything = match ready {
        Some(Ok(n)) => json!({ "reachable": true, "objects": n }),
        Some(Err(e)) => json!({ "reachable": false, "error": e.lines().next().unwrap_or("") }),
        None => json!({ "checked": false }),
    };
    let doc = json!({
        "executable": exe,
        "version": env!("CARGO_PKG_VERSION"),
        "transport": format!("http://{}:{}", http.host, http.port),
        "write": opts.write,
        "everything": everything,
        "targets": targets,
    });
    println!("{}", serde_json::to_string_pretty(&doc).unwrap_or_default());
}
