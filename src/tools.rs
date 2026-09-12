//! The MCP tools: published schemas, dispatch, and the text + structured output
//! that every tool returns.
//!
//! Design notes (settled in a design review):
//!  - `category` replaces the old `everything_search_by_type` tool. The category
//!    table is real value, but exposing it as a second search entry point made the
//!    model choose between two tools that do the same thing, which hurts tool
//!    selection. It is now a preset parameter of `everything_search`.
//!  - `everything_count_stats` stays a separate tool: it returns an aggregate, not
//!    a result list, and folding it into search behind a flag would turn the output
//!    schema into a conditional union.
//!  - Every tool carries `annotations` and an `outputSchema`, and returns
//!    `structuredContent` plus text that carries the same information.

use crate::everything::{self, Client, Item, Query, PERIOD_NAMES, SORT_NAMES};
use crate::filetype;
use crate::jsonrpc::{Handler, ToolOutput};
use serde_json::{json, Value};
use std::path::Path;
use std::time::Instant;

pub struct Tools {
    client: Client,
}

impl Tools {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

// ---------------------------------------------------------------- schema helpers

fn schema(props: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": props,
        "required": required,
        "additionalProperties": false,
    })
}

fn p_string(desc: &str, default: Option<&str>) -> Value {
    match default {
        Some(d) => json!({"type": "string", "description": desc, "default": d}),
        None => json!({"type": "string", "description": desc}),
    }
}

fn p_enum(desc: &str, values: &[&str], default: Option<&str>) -> Value {
    let mut v = json!({"type": "string", "description": desc, "enum": values});
    if let Some(d) = default {
        v["default"] = Value::String(d.to_string());
    }
    v
}

fn p_int(desc: &str, default: i64, min: i64, max: i64) -> Value {
    json!({"type": "integer", "description": desc, "default": default,
           "minimum": min, "maximum": max})
}

fn p_bool(desc: &str, default: bool) -> Value {
    json!({"type": "boolean", "description": desc, "default": default})
}

/// Schema for one search hit, shared by every outputSchema that returns results.
fn item_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "path": {"type": "string", "description": "parent directory"},
            "full_path": {"type": "string"},
            "type": {"type": "string", "enum": ["file", "folder"]},
            "size": {"type": "integer", "description": "bytes; absent for folders"},
            "modified": {"type": "string", "description": "local time, YYYY-MM-DD HH:MM:SS"},
            "kind": {"type": "string",
                "description": "file category, the same vocabulary as the category parameter, plus text and unknown. Feed it straight back into category."},
            "content_mode": {"type": "string", "enum": ["text", "binary", "unknown"],
                "description": "whether the file is worth reading as text. From the extension alone, so unknown is common for ambiguous types; everything_file_details sniffs the header to resolve it."},
            "format": {"type": "string", "description": "friendly format name, e.g. rust, pdf, png"},
            "type_source": {"type": "string", "enum": ["extension", "magic", "heuristic"]}
        },
        "required": ["name", "path", "full_path", "type", "kind", "content_mode", "type_source"],
        "additionalProperties": false
    })
}

fn results_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": {"type": "string", "description": "the query as requested"},
            "effective_query": {"type": "string",
                "description": "the Everything expression actually executed, after path/category/entry_type/period expansion"},
            "match_modes": {"type": "array", "items": {"type": "string"},
                "description": "match modifiers that were in effect. These travel as HTTP parameters rather than as part of the expression, so effective_query cannot show them; empty means none were set"},
            "probes": {"type": "array", "items": {"type": "string"},
                "description": "extra queries executed by multi-probe; empty when probe was not requested"},
            "total": {"type": "integer", "description": "matching objects in the index"},
            "total_accuracy": {"type": "string", "enum": ["exact"]},
            "returned": {"type": "integer"},
            "offset": {"type": "integer"},
            "next_offset": {"type": "integer"},
            "has_more": {"type": "boolean"},
            "results": {"type": "array", "items": item_schema()},
            "elapsed_ms": {"type": "number"}
        },
        "required": ["query", "effective_query", "match_modes", "total", "total_accuracy",
                     "returned", "offset", "has_more", "results", "elapsed_ms"],
        "additionalProperties": false
    })
}

fn recent_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": {"type": "string"},
            "effective_query": {"type": "string"},
            "requested_period": {"type": "string"},
            "effective_period": {"type": "string",
                "description": "differs from requested_period only when auto_expand widened the window"},
            "expanded": {"type": "boolean",
                "description": "true when the requested window held too few results and the search was retried across all time"},
            "total": {"type": "integer"},
            "total_accuracy": {"type": "string", "enum": ["exact"]},
            "returned": {"type": "integer"},
            "has_more": {"type": "boolean"},
            "results": {"type": "array", "items": item_schema()},
            "elapsed_ms": {"type": "number"}
        },
        "required": ["query", "effective_query", "requested_period", "effective_period",
                     "expanded", "total", "total_accuracy", "returned", "has_more",
                     "results", "elapsed_ms"],
        "additionalProperties": false
    })
}

fn details_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "entries": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "exists": {"type": "boolean"},
                        "type": {"type": "string", "enum": ["file", "folder", "other"]},
                        "size": {"type": "integer"},
                        "modified": {"type": "string"},
                        "entries": {"type": "integer", "description": "for folders: number listed"},
                        "kind": {"type": "string", "description": "file category, same vocabulary as the category parameter"},
                        "content_mode": {"type": "string", "enum": ["text", "binary", "unknown"],
                            "description": "resolved by sniffing the header when the extension cannot say"},
                        "format": {"type": "string"},
                        "type_source": {"type": "string", "enum": ["extension", "magic", "heuristic"]},
                        "preview": {"type": "string", "description": "triage preview, text files only"},
                        "preview_bytes": {"type": "integer"},
                        "preview_truncated": {"type": "boolean"},
                        "preview_skipped": {"type": "string", "description": "why no preview was produced"},
                        "error": {"type": "string"}
                    },
                    "required": ["path", "exists"],
                    "additionalProperties": false
                }
            },
            "elapsed_ms": {"type": "number"}
        },
        "required": ["entries", "elapsed_ms"],
        "additionalProperties": false
    })
}

fn stats_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": {"type": "string"},
            "effective_query": {"type": "string"},
            "count": {"type": "integer"},
            "count_accuracy": {"type": "string", "enum": ["exact"],
                "description": "always exact: Everything reports the true total independently of how many rows are fetched"},
            "total_size": {"type": "integer",
                "description": "absent when no exact sum was requested or the row cap was hit"},
            "total_size_accuracy": {"type": "string", "enum": ["exact", "unavailable"]},
            "total_size_note": {"type": "string",
                "description": "why the size is unavailable, when it is"},
            "rows_summed": {"type": "integer"},
            "sampled": {"type": "integer"},
            "breakdown": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "extension": {"type": "string"},
                        "count": {"type": "integer"},
                        "size": {"type": "integer"}
                    },
                    "required": ["extension", "count", "size"],
                    "additionalProperties": false
                }
            },
            "breakdown_accuracy": {"type": "string", "enum": ["exact", "sampled"]},
            "elapsed_ms": {"type": "number"}
        },
        "required": ["query", "effective_query", "count", "count_accuracy", "elapsed_ms"],
        "additionalProperties": false
    })
}

fn batch_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "queries_requested": {"type": "integer"},
            "queries_executed": {"type": "array", "items": {"type": "string"}},
            "total_returned": {"type": "integer"},
            "results": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"},
                        "effective_query": {"type": "string"},
                        "total": {"type": "integer"},
                        "returned": {"type": "integer"},
                        "results": {"type": "array", "items": item_schema()},
                        "error": {"type": "string"}
                    },
                    "additionalProperties": false
                }
            },
            "elapsed_ms": {"type": "number"}
        },
        "required": ["queries_requested", "queries_executed", "total_returned",
                     "results", "elapsed_ms"],
        "additionalProperties": false
    })
}

fn tool(name: &str, description: &str, input: Value, output: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input,
        // Every tool here is a read-only query against a local index: no writes,
        // no network, no side effects.
        "annotations": {
            "title": name,
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        },
        "outputSchema": output,
    })
}

// ---------------------------------------------------------------- query building

/// Filters that compile into the Everything expression actually executed.
struct Filters<'a> {
    raw: &'a str,
    category: Option<&'a str>,
    entry_type: &'a str,
    path: Option<&'a str>,
    regex: bool,
}

/// Compile the typed arguments into one Everything expression.
///
/// Order matters: `regex:` is a search *function* that applies to everything after
/// it, so it must come last, immediately before the caller's pattern. The path is
/// expressed as the `path:"..."` function for the same reason — an injected
/// literal would otherwise become part of the regex pattern.
fn compile(f: &Filters) -> Result<String, String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(p) = f.path.filter(|p| !p.trim().is_empty()) {
        let p = p.trim().trim_end_matches(|c| c == '\\' || c == '/');
        parts.push(format!("path:\"{p}\""));
    }
    match f.entry_type {
        "file" => parts.push("file:".to_string()),
        "folder" => parts.push("folder:".to_string()),
        "any" | "" => {}
        other => return Err(format!("invalid entry_type '{other}'. Valid: file, folder, any")),
    }
    if let Some(c) = f.category.filter(|c| !c.trim().is_empty()) {
        let clause = everything::file_type_query(c)
            .ok_or_else(|| format!("invalid category '{c}'. Valid: {}", everything::file_type_names().join(", ")))?;
        parts.push(clause.to_string());
    }
    let tail = f.raw.trim();
    if f.regex {
        // `regex:` must sit immediately before the pattern; a separating space makes
        // it part of the pattern and the search stops matching (verified).
        parts.push(format!("regex:{tail}"));
    } else if !tail.is_empty() {
        parts.push(tail.to_string());
    }
    Ok(parts.join(" "))
}

// ---------------------------------------------------------------- Handler

impl Handler for Tools {
    fn instructions(&self) -> Option<String> {
        Some(
            "Everything file search for Windows, backed by voidtools Everything's real-time \
             NTFS index. Prefer these tools over shell commands (dir /s, Get-ChildItem \
             -Recurse, glob) for ANY filename lookup outside the current project: a query \
             costs well under a millisecond. All tools are read-only. Results carry an \
             `effective_query` field showing the exact Everything expression that ran, so \
             an unexpected result set can be diagnosed without guessing."
                .to_string(),
        )
    }

    fn list_tools(&self) -> Value {
        let sort_desc = format!("Sort order. One of: {}", SORT_NAMES.join(", "));
        let cats = everything::file_type_names();
        let cats_desc = format!("One of: {}", cats.join(", "));

        let search_schema = schema(
            json!({
                "query": p_string("Search query in Everything syntax, e.g. '*.rs', 'ext:py;js', 'size:>10mb', 'dm:today', or a regex when match_regex is set. Space = AND, | = OR, ! excludes. May be empty when a category or entry_type filter alone expresses the intent. Matching is case-insensitive by default; use the search FUNCTIONS instead of the match_* parameters where you can, because functions work in every tool: 'case:README' (case-sensitive), 'wholeword:read' (whole words only), 'exact:name' (whole name). The match_* parameters exist only because functions are unreliable inside a regex.", Some("")),
                "category": p_enum(&format!("Restrict to a file category, so extension lists do not have to be written by hand. Adds an ext: clause. {cats_desc}."), &cats, Some("")),
                "entry_type": p_enum("Restrict to files or folders. Use 'folder' to find directories (projects, install dirs) instead of guessing from results.", &["any", "file", "folder"], Some("any")),
                "path": p_string("Restrict search to this directory tree. Prefer this over writing path: in the query.", Some("")),
                "max_results": p_int("Maximum results to return (1-500)", 50, 1, 500),
                "offset": p_int("Skip N results (pagination)", 0, 0, 2147483647),
                "sort": p_string(&sort_desc, Some("date-modified-desc")),
                "match_case": p_bool("Case-sensitive search. Prefer the `case:` search function inside query - it works in every tool, including everything_count_stats and everything_find_recent, which have no match_* parameters. This parameter is for regex searches, where functions are unreliable.", false),
                "match_whole_word": p_bool("Match whole words only. Prefer the `wholeword:` (or `ww:`) search function inside query, which works in every tool. This parameter is for regex searches.", false),
                "match_regex": p_bool("Treat query as a regular expression", false),
                "match_path": p_bool("Match the query against the full path instead of the filename", false),
                "max_per_parent": p_int("Diversify results: keep at most N hits per parent directory, so one directory tree cannot fill the whole list. 0 disables it.", 0, 0, 100),
                "probe": p_bool("Multi-probe: when a single bare term returns few hits, also look for it in the path and among folders (useful when the thing you are naming is really a directory). Every extra query executed is reported in `probes`.", false),
                "include_total": p_bool("Also report the total match count (it is already exact and costs nothing, so this only controls whether the text form mentions it).", false),
            }),
            &["query"],
        );

        let recent_schema = schema(
            json!({
                "period": p_string(&format!("How recent. One of: {}. Raw Everything syntax such as 'last2hours' is also accepted.", PERIOD_NAMES.join(", ")), Some("1day")),
                "path": p_string("Restrict to this directory path", Some("")),
                "extensions": p_string("Filter by extensions, e.g. 'py,js,ts' or 'py;js;ts'", Some("")),
                "query": p_string("Additional search filter", Some("")),
                "max_results": p_int("Maximum results to return (1-500)", 50, 1, 500),
                "auto_expand": p_bool("When the period yields fewer than max_results hits, retry across all time. The response reports whether this happened via `expanded` and `effective_period`.", true),
            }),
            &[],
        );

        let details_schema = schema(
            json!({
                "paths": json!({
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Absolute file or folder paths to inspect (1-20).",
                    "minItems": 1,
                    "maxItems": 20
                }),
                "preview_lines": p_int("Triage preview: at most this many lines, and only for files whose content resolves to text. Use your own file-reading tool for full contents.", 0, 0, 200),
            }),
            &["paths"],
        );

        let stats_schema = schema(
            json!({
                "query": p_string("Search query to count. Same syntax as everything_search.", None),
                "category": p_enum(&format!("Restrict to a file category. {cats_desc}."), &cats, Some("")),
                "entry_type": p_enum("Restrict to files or folders.", &["any", "file", "folder"], Some("any")),
                "path": p_string("Restrict counting to this directory", Some("")),
                "include_size": p_bool("Report total size of matching files. Without exact_size the figure is omitted entirely rather than estimated from a biased sample.", true),
                "exact_size": p_bool("Sum every match to get an exact total size. Costs one paged pass over the result set, so it is opt-in; refused above 200000 matches. Also makes the extension breakdown exact.", false),
                "breakdown_by_extension": p_bool("Break the result set down by extension", false),
                "sample_sort": p_string("Sort used when sampling for the breakdown. Name sorts are rejected when a breakdown is requested, because filename sort correlates with extension and biases the sample.", Some("date-modified-desc")),
            }),
            &["query"],
        );

        json!({"tools": [
            tool("everything_search",
                 "Search files and folders by name, extension, size or date using voidtools Everything's real-time NTFS index. Supports a category preset, file/folder filtering, path scoping, paging, sorting, regex and result diversification.",
                 search_schema, results_output_schema()),
            tool("everything_find_recent",
                 "Find files modified within a recent time period - useful for what changed in a project, recent downloads, or today's logs. Sorted newest-first, with optional widening when the window is too narrow.",
                 recent_schema, recent_output_schema()),
            tool("everything_file_details",
                 "Inspect specific paths: metadata plus a resolved type (kind, text-or-binary, format), sniffing the file header only when the extension cannot say. Use it to decide whether a file is worth reading; it is not a replacement for your own file-reading tool.",
                 details_schema, details_output_schema()),
            tool("everything_count_stats",
                 "Count and measure files matching a query without listing them. The count is exact; size and per-extension figures are sampled and labelled as such.",
                 stats_schema, stats_output_schema()),
            tool("everything_search_batch",
                 "Run up to 8 searches in one call and get all the answers back together. Use it when several independent lookups are needed at once (does this project have a Cargo.toml / package.json / pyproject.toml?), to avoid paying an agent round trip per lookup. Each entry takes the same arguments as everything_search.",
                 schema(
                     json!({
                         "queries": json!({
                             "type": "array",
                             "description": "1-8 search argument objects; each accepts the everything_search arguments.",
                             "minItems": 1,
                             "maxItems": 8,
                             "items": {"type": "object"}
                         }),
                         "max_total_results": p_int("Stop early once this many results have been returned across all queries.", 200, 1, 2000),
                     }),
                     &["queries"],
                 ),
                 batch_output_schema()),
        ]})
    }

    fn call_tool(&mut self, name: &str, args: &Value) -> Result<ToolOutput, String> {
        match name {
            "everything_search" => self.search(args),
            "everything_find_recent" => self.find_recent(args),
            "everything_file_details" => self.file_details(args),
            "everything_count_stats" => self.count_stats(args),
            "everything_search_batch" => self.batch(args),
            other => Err(format!(
                "unknown tool: {other}. Available: everything_search, everything_find_recent, \
                 everything_file_details, everything_count_stats, everything_search_batch"
            )),
        }
    }
}

// ---------------------------------------------------------------- arg helpers

fn s(args: &Value, key: &str) -> String {
    args.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}
fn s_or(args: &Value, key: &str, default: &str) -> String {
    let v = s(args, key);
    if v.trim().is_empty() {
        default.to_string()
    } else {
        v
    }
}
fn u(args: &Value, key: &str, default: usize) -> usize {
    args.get(key).and_then(|v| v.as_u64()).map(|v| v as usize).unwrap_or(default)
}
fn b(args: &Value, key: &str, default: bool) -> bool {
    args.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}
fn opt(args: &Value, key: &str) -> Option<String> {
    let v = s(args, key);
    if v.trim().is_empty() {
        None
    } else {
        Some(v)
    }
}

fn cap(cfg: &everything::Config, n: usize) -> usize {
    n.clamp(1, cfg.max_results_cap.max(1))
}

/// Convert hits to the structured form (null fields are omitted, not sent as null).
///
/// Classification here is extension-only on purpose: touching the filesystem per
/// result would turn one index lookup into N file operations.
fn item_json(it: &Item) -> Value {
    let mut o = json!({
        "name": it.name,
        "path": it.path,
        "full_path": it.full_path(),
        "type": if it.is_dir { "folder" } else { "file" },
    });
    if it.is_dir {
        o["kind"] = json!("folder");
        o["content_mode"] = json!("unknown");
        o["type_source"] = json!("extension");
    } else {
        let t = filetype::classify(&it.name);
        o["kind"] = json!(t.kind);
        o["content_mode"] = json!(t.content_mode);
        o["type_source"] = json!(t.type_source);
        if !t.format.is_empty() {
            o["format"] = json!(t.format);
        }
    }
    if let Some(sz) = it.size {
        o["size"] = json!(sz);
    }
    if let Some(m) = &it.modified {
        o["modified"] = json!(m);
    }
    o
}

/// Keep at most `n` hits per parent directory, preserving order.
fn diversify(items: Vec<Item>, n: usize) -> Vec<Item> {
    if n == 0 {
        return items;
    }
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        let key = it.path.to_lowercase();
        let c = seen.entry(key).or_insert(0);
        if *c < n {
            *c += 1;
            out.push(it);
        }
    }
    out
}

fn format_items(items: &[Item], offset: usize, total: u64) -> String {
    if items.is_empty() {
        return "No results.\n".to_string();
    }
    let mut out = String::new();
    for it in items {
        let tag = if it.is_dir { "DIR " } else { "FILE" };
        let mut meta: Vec<String> = Vec::new();
        if let Some(sz) = it.size {
            meta.push(everything::human_size(sz));
        }
        if let Some(m) = &it.modified {
            meta.push(m.clone());
        }
        // The structured channel always carries the full classification. The text
        // channel flags only what changes what a reader should do - not plain text,
        // or not determinable from the name - because some clients forward only the
        // text, and annotating every ordinary source file would just cost tokens.
        let mut note = String::new();
        if !it.is_dir {
            let t = filetype::classify(&it.name);
            if t.content_mode != "text" {
                note = format!("  <{}/{}>", t.kind, t.content_mode);
            }
        }
        if meta.is_empty() {
            out.push_str(&format!("  [{tag}] {}{note}\n", it.full_path()));
        } else {
            out.push_str(&format!(
                "  [{tag}] {}  ({}){note}\n",
                it.full_path(),
                meta.join(", ")
            ));
        }
    }
    out.push_str(&format!(
        "\n{} result(s) from offset {} of {} matching (exact total).",
        items.len(),
        offset,
        total
    ));
    out
}

// ---------------------------------------------------------------- tools

/// Everything's HTTP API returns sizes per row but has no aggregate, so summing
/// every match is the only route to an exact total. Bounded so a huge query cannot
/// run away, and the caller is told when the bound is hit rather than given a guess.
const EXACT_SIZE_MAX_ROWS: u64 = 200_000;

impl Tools {
    fn sum_all_sizes(
        &self,
        effective: &str,
        sort: &str,
        already_have: usize,
        running: u64,
        total: u64,
    ) -> Result<u64, String> {
        if total > EXACT_SIZE_MAX_ROWS {
            return Err(format!(
                "{total} matches exceeds the {EXACT_SIZE_MAX_ROWS}-row cap for an exact sum; \
                 narrow the query with path or category"
            ));
        }
        let mut sum = running;
        let mut offset = already_have;
        while (offset as u64) < total {
            let page = self.client.query(&Query {
                search: effective,
                count: 500,
                offset,
                sort,
                ascending: !sort.ends_with("-desc"),
                case: false,
                whole_word: false,
                regex: false,
                match_path: false,
                path: None,
            })?;
            if page.results.is_empty() {
                break;
            }
            for r in &page.results {
                if let Some(sz) = r.size.as_deref().and_then(|s| s.parse::<u64>().ok()) {
                    sum += sz;
                }
            }
            offset += page.results.len();
            if page.results.len() < 500 {
                break;
            }
        }
        Ok(sum)
    }

    fn search(&mut self, args: &Value) -> Result<ToolOutput, String> {
        let started = Instant::now();
        let query = s(args, "query");
        let category = opt(args, "category");
        let entry_type = s_or(args, "entry_type", "any");
        let path = opt(args, "path");
        if query.trim().is_empty() && category.is_none() && entry_type == "any" {
            return Err("query is required (or provide category / entry_type to express the filter)".into());
        }
        let max = cap(self.client.config(), u(args, "max_results", 50));
        let offset = u(args, "offset", 0);
        let sort = s_or(args, "sort", "date-modified-desc");
        everything::validate_sort(&sort)?;
        let regex = b(args, "match_regex", false);
        let match_case = b(args, "match_case", false);
        let match_whole_word = b(args, "match_whole_word", false);
        let match_path = b(args, "match_path", false);
        let max_per_parent = u(args, "max_per_parent", 0);
        let include_total = b(args, "include_total", false);

        // A malformed pattern is not a search that found nothing, and Everything
        // reports both the same way. Refuse the typo instead of answering it.
        if regex {
            if let Some(problem) = everything::regex_syntax_problem(&query) {
                return Err(format!(
                    "match_regex is set but the pattern is not well-formed: {problem}. \
                     Everything returns zero matches for a broken pattern rather than an \
                     error, so this would have looked like an empty result."
                ));
            }
        }

        // These three travel as HTTP parameters, not as part of the search
        // expression, so `effective_query` cannot show them. Reported back
        // explicitly: otherwise the only way to tell whether a flag took effect is
        // to compare match counts and guess.
        let mut match_modes: Vec<&str> = Vec::new();
        if match_case {
            match_modes.push("case-sensitive");
        }
        if match_whole_word {
            match_modes.push("whole-word");
        }
        if match_path {
            match_modes.push("match full path");
        }

        let effective = compile(&Filters {
            raw: &query,
            category: category.as_deref(),
            entry_type: &entry_type,
            path: path.as_deref(),
            regex,
        })?;

        // Diversification needs a wider net than the page actually returned.
        let fetch = if max_per_parent > 0 { (max * 5).min(500) } else { max };

        let resp = self.client.query(&Query {
            search: &effective,
            count: fetch,
            offset,
            sort: &sort,
            ascending: !sort.ends_with("-desc"),
            case: match_case,
            whole_word: match_whole_word,
            regex: false, // already compiled into the expression as the regex: function
            match_path,
            path: None, // already compiled into the expression as the path: function
        })?;

        let total_primary = resp.total_results;
        let mut probes: Vec<String> = Vec::new();
        let mut all: Vec<Item> = resp.results.into_iter().map(Item::from).collect();

        // Optional multi-probe. A bare term usually names a file, but agents often
        // mean "the thing called X" when X is really a directory (a project folder).
        // When the primary result set is thin, look for the term in the path and
        // among folders too. Opt-in, and every extra query is reported back, so the
        // widening is never silent.
        let bare_term = !query.trim().is_empty()
            && !query.contains(' ')
            && !query.contains(':')
            && !query.contains('*')
            && !query.contains('?')
            && !regex;
        if b(args, "probe", false) && bare_term && all.len() < max {
            for variant in [format!("path:{}", query.trim()), format!("folder: {}", query.trim())] {
                if let Ok(r2) = self.client.query(&Query {
                    search: &variant,
                    count: max,
                    offset: 0,
                    sort: &sort,
                    ascending: !sort.ends_with("-desc"),
                    case: false,
                    whole_word: false,
                    regex: false,
                    match_path: false,
                    path: None,
                }) {
                    probes.push(variant);
                    for it in r2.results.into_iter().map(Item::from) {
                        all.push(it);
                    }
                }
            }
            let mut seen = std::collections::HashSet::new();
            all.retain(|i| seen.insert(i.full_path().to_lowercase()));
        }
        let total = total_primary.max(all.len() as u64);

        let diversified = max_per_parent > 0;
        let mut items = diversify(all, max_per_parent);
        items.truncate(max);
        let has_more = (offset as u64 + items.len() as u64) < total;

        let mut text = if items.is_empty() {
            "No results.\n".to_string()
        } else {
            let mut head = String::new();
            if !query.trim().is_empty() {
                head.push_str(&format!("query: {query}\n"));
            }
            if diversified {
                head.push_str(&format!("diversified: at most {max_per_parent} per parent directory\n"));
            }
            head.push_str(&format!("effective query: {effective}\n"));
            if !match_modes.is_empty() {
                head.push_str(&format!("match modes: {}\n", match_modes.join(", ")));
            }
            head.push('\n');
            head.push_str(&format_items(&items, offset, total));
            head
        };
        if include_total && items.is_empty() {
            text.push_str(&format!("Total matches: {total} (exact)\n"));
        }

        if !probes.is_empty() {
            text.push_str(&format!("extra probes executed: {}\n", probes.join("  |  ")));
        }
        let structured = json!({
            "query": query,
            "effective_query": effective,
            "match_modes": match_modes,
            "probes": probes,
            "total": total,
            "total_accuracy": "exact",
            "returned": items.len(),
            "offset": offset,
            "next_offset": offset + items.len(),
            "has_more": has_more,
            "results": items.iter().map(item_json).collect::<Vec<_>>(),
            "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
        });
        Ok(ToolOutput::new(text, structured))
    }

    fn find_recent(&mut self, args: &Value) -> Result<ToolOutput, String> {
        let started = Instant::now();
        let period = s_or(args, "period", "1day");
        // Accept the documented raw Everything syntax (last2hours, last30mins, ...).
        // Either branch produces a full `dm:` expression; see `period_query`.
        let dm: String = match everything::period_query(&period) {
            Some(v) => v.to_string(),
            None if period.starts_with("last") && period.len() > 4 => format!("dm:{period}"),
            None => {
                return Err(format!(
                    "invalid period '{period}'. Valid: {} (or raw Everything syntax such as 'last2hours')",
                    PERIOD_NAMES.join(", ")
                ))
            }
        };
        let max = cap(self.client.config(), u(args, "max_results", 50));
        let path = opt(args, "path");
        let extra = s(args, "query");
        let extensions = s(args, "extensions");

        let build = |with_time: bool| {
            let mut parts: Vec<String> = Vec::new();
            if let Some(p) = path.as_deref() {
                let p = p.trim().trim_end_matches(|c| c == '\\' || c == '/');
                parts.push(format!("path:\"{p}\""));
            }
            if with_time {
                parts.push(dm.clone());
            }
            if !extensions.trim().is_empty() {
                parts.push(format!("ext:{}", extensions.replace(',', ";")));
            }
            if !extra.trim().is_empty() {
                parts.push(extra.clone());
            }
            parts.join(" ")
        };

        let requested_period = period.clone();
        let mut effective = build(true);
        let mut resp = self.client.query(&Query {
            search: &effective,
            count: max,
            offset: 0,
            sort: "date-modified-desc",
            ascending: false,
            case: false,
            whole_word: false,
            regex: false,
            match_path: false,
            path: None,
        })?;

        let auto_expand = b(args, "auto_expand", true);
        let expanded = auto_expand && resp.results.len() < max && resp.total_results < max as u64;
        if expanded {
            effective = build(false);
            resp = self.client.query(&Query {
                search: &effective,
                count: max,
                offset: 0,
                sort: "date-modified-desc",
                ascending: false,
                case: false,
                whole_word: false,
                regex: false,
                match_path: false,
                path: None,
            })?;
        }

        let total = resp.total_results;
        let items: Vec<Item> = resp.results.into_iter().map(Item::from).collect();
        let effective_period = if expanded { "all time".to_string() } else { requested_period.clone() };

        let mut text = format!(
            "recent: requested {requested_period}, effective {effective_period}{}\n",
            if expanded { "  (window too narrow - widened to all time)" } else { "" }
        );
        text.push_str(&format!("effective query: {effective}\n\n"));
        text.push_str(&format_items(&items, 0, total));

        let structured = json!({
            "query": extra,
            "effective_query": effective,
            "requested_period": requested_period,
            "effective_period": effective_period,
            "expanded": expanded,
            "total": total,
            "total_accuracy": "exact",
            "returned": items.len(),
            "has_more": (items.len() as u64) < total,
            "results": items.iter().map(item_json).collect::<Vec<_>>(),
            "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
        });
        Ok(ToolOutput::new(text, structured))
    }

    fn file_details(&mut self, args: &Value) -> Result<ToolOutput, String> {
        let started = Instant::now();
        let paths = args
            .get("paths")
            .and_then(|v| v.as_array())
            .ok_or("paths is required (array of absolute paths)")?;
        if paths.is_empty() || paths.len() > 20 {
            return Err(format!(
                "paths must contain 1-20 entries, got {}",
                paths.len()
            ));
        }
        let preview = u(args, "preview_lines", 0).min(200);
        let mut entries: Vec<Value> = Vec::new();
        let mut text = String::new();

        for pv in paths {
            let p = pv.as_str().unwrap_or("");
            let pathref = Path::new(p);
            text.push_str(&format!("{p}\n"));
            if !pathref.is_absolute() {
                text.push_str("  error: path must be absolute\n\n");
                entries.push(json!({"path": p, "exists": false,
                                    "error": "path must be absolute"}));
                continue;
            }
            match std::fs::metadata(pathref) {
                Ok(md) => {
                    let mut e = json!({"path": p, "exists": true});
                    if md.is_dir() {
                        e["type"] = json!("folder");
                        text.push_str("  type: directory\n");
                        let mut names: Vec<String> = Vec::new();
                        if let Ok(rd) = std::fs::read_dir(pathref) {
                            for ent in rd.flatten().take(500) {
                                names.push(ent.file_name().to_string_lossy().to_string());
                            }
                        }
                        names.sort();
                        e["entries"] = json!(names.len());
                        text.push_str(&format!("  entries: {}\n", names.len()));
                        for n in names.iter().take(20) {
                            text.push_str(&format!("    - {n}\n"));
                        }
                    } else {
                        e["type"] = json!(if md.is_file() { "file" } else { "other" });
                        e["size"] = json!(md.len());

                        // Level 2 classification. Only sniff when the name cannot tell
                        // us, and only read a small header to do it.
                        let name = pathref
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let mut t = filetype::classify(&name);
                        if t.content_mode == "unknown" && md.is_file() {
                            if let Ok(mut f) = std::fs::File::open(pathref) {
                                use std::io::Read;
                                let mut head = vec![0u8; filetype::SNIFF_BYTES];
                                if let Ok(n) = f.read(&mut head) {
                                    head.truncate(n);
                                    t = filetype::refine(t, &head);
                                }
                            }
                        }
                        e["kind"] = json!(t.kind);
                        e["content_mode"] = json!(t.content_mode);
                        e["type_source"] = json!(t.type_source);
                        if !t.format.is_empty() {
                            e["format"] = json!(t.format);
                        }

                        text.push_str(&format!(
                            "  size: {} ({} bytes)\n",
                            everything::human_size(md.len()),
                            md.len()
                        ));
                        text.push_str(&format!(
                            "  type: {} / {} (via {})\n",
                            t.kind, t.content_mode, t.type_source
                        ));

                        if preview > 0 {
                            if t.content_mode == "text" {
                                // A triage preview: enough to judge relevance, not a
                                // substitute for the agent's own read tool.
                                match std::fs::read(pathref) {
                                    Ok(bytes) => {
                                        let decoded = filetype::decode_text(&bytes);
                                        let mut it = decoded.lines();
                                        let lines: Vec<&str> = it.by_ref().take(preview).collect();
                                        let truncated = it.next().is_some();
                                        e["preview"] = json!(lines.join("\n"));
                                        e["preview_bytes"] = json!(bytes.len());
                                        e["preview_truncated"] = json!(truncated);
                                        let enc = filetype::encoding_of(&bytes);
                                        if enc != "utf-8" {
                                            e["encoding"] = json!(enc);
                                        }
                                        text.push_str("  preview:\n");
                                        for l in &lines {
                                            text.push_str(&format!("    {l}\n"));
                                        }
                                        if truncated {
                                            text.push_str("  (truncated)\n");
                                        }
                                    }
                                    Err(err) => {
                                        e["error"] = json!(err.to_string());
                                        text.push_str(&format!("  preview unavailable: {err}\n"));
                                    }
                                }
                            } else {
                                text.push_str(&format!(
                                    "  preview: skipped (content_mode is {})\n",
                                    t.content_mode
                                ));
                                e["preview_skipped"] = json!(t.content_mode);
                            }
                        }
                    }
                    entries.push(e);
                    text.push('\n');
                }
                Err(err) => {
                    text.push_str(&format!("  error: {err}\n\n"));
                    entries.push(json!({"path": p, "exists": false, "error": err.to_string()}));
                }
            }
        }
        let structured = json!({
            "entries": entries,
            "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
        });
        Ok(ToolOutput::new(text, structured))
    }

    fn count_stats(&mut self, args: &Value) -> Result<ToolOutput, String> {
        let started = Instant::now();
        let query = s(args, "query");
        let category = opt(args, "category");
        let entry_type = s_or(args, "entry_type", "any");
        if query.trim().is_empty() && category.is_none() {
            return Err("query is required".into());
        }
        let path = opt(args, "path");
        let include_size = b(args, "include_size", true);
        let want_exact = b(args, "exact_size", false);
        let breakdown = b(args, "breakdown_by_extension", false);
        let sample_sort = s_or(args, "sample_sort", "date-modified-desc");
        // Only meaningful when a breakdown is actually sampled.
        if breakdown && (sample_sort == "name" || sample_sort == "name-desc") {
            return Err(
                "sample_sort 'name' is rejected: filename sort correlates with extension and \
                 biases the sample. Use a date or size sort."
                    .into(),
            );
        }
        everything::validate_sort(&sample_sort)?;

        let effective = compile(&Filters {
            raw: &query,
            category: category.as_deref(),
            entry_type: &entry_type,
            path: path.as_deref(),
            regex: false,
        })?;

        // count=1 is enough for an exact total; a wider sample is only needed when
        // size or a per-extension breakdown is requested.
        let sample = if include_size || breakdown { 500 } else { 1 };
        let resp = self.client.query(&Query {
            search: &effective,
            count: sample,
            offset: 0,
            sort: &sample_sort,
            ascending: !sample_sort.ends_with("-desc"),
            case: false,
            whole_word: false,
            regex: false,
            match_path: false,
            path: None,
        })?;

        let total = resp.total_results;
        let items: Vec<Item> = resp.results.into_iter().map(Item::from).collect();

        let mut structured = json!({
            "query": query,
            "effective_query": effective,
            "count": total,
            "count_accuracy": "exact",
            "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
        });
        let mut text = format!("count: {total} (exact)\neffective query: {effective}\n");

        if include_size {
            let known: Vec<u64> = items.iter().filter_map(|i| i.size).collect();
            let partial: u64 = known.iter().sum();
            if known.len() as u64 >= total {
                // the sample already covers every match, so this really is exact
                structured["total_size"] = json!(partial);
                structured["total_size_accuracy"] = json!("exact");
                text.push_str(&format!(
                    "total size: {} ({} bytes, exact)\n",
                    everything::human_size(partial),
                    partial
                ));
            } else if want_exact {
                match self.sum_all_sizes(&effective, &sample_sort, known.len(), partial, total) {
                    Ok(sum) => {
                        structured["total_size"] = json!(sum);
                        structured["total_size_accuracy"] = json!("exact");
                        structured["rows_summed"] = json!(total);
                        text.push_str(&format!(
                            "total size: {} ({} bytes, exact - summed all {} matches)\n",
                            everything::human_size(sum),
                            sum,
                            total
                        ));
                    }
                    Err(why) => {
                        // Field omitted rather than sent as null, matching how size
                        // and modified are omitted when unknown.
                        structured["total_size_accuracy"] = json!("unavailable");
                        structured["total_size_note"] = json!(why);
                        text.push_str(&format!("total size: unavailable ({why})\n"));
                    }
                }
            } else {
                // Deliberately NOT estimated. File sizes are heavily skewed and a
                // top-N slice is not a random sample: extrapolating from it produced
                // figures that contradicted each other (23.6 GB combined versus
                // 84 GB for one member of that same set). A wrong number is worse
                // than no number.
                let note = format!(
                    "not estimated: file sizes are heavily skewed and {} of {} rows is not a \
                     random sample, so extrapolation is not meaningful. Pass exact_size=true to \
                     sum every match.",
                    known.len(),
                    total
                );
                structured["total_size_accuracy"] = json!("unavailable");
                structured["total_size_note"] = json!(note);
                text.push_str(&format!("total size: unavailable - {note}\n"));
            }
        }

        if breakdown {
            use std::collections::BTreeMap;
            let mut by_ext: BTreeMap<String, (usize, u64)> = BTreeMap::new();
            for it in &items {
                let ext = Path::new(&it.name)
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_else(|| "(none)".into());
                let e = by_ext.entry(ext).or_insert((0, 0));
                e.0 += 1;
                e.1 += it.size.unwrap_or(0);
            }
            let list: Vec<Value> = by_ext
                .iter()
                .map(|(ext, (n, sz))| json!({"extension": ext, "count": n, "size": sz}))
                .collect();
            let covers_all = items.len() as u64 >= total;
            let accuracy = if covers_all { "exact" } else { "sampled" };
            text.push_str(&format!(
                "breakdown ({accuracy}, from {} of {} matches):\n",
                items.len(),
                total
            ));
            // The number under `rows` is how many files of that extension are in
            // the sample, not how many exist. Without a column header that reads as
            // a total — the reported confusion — so the sample size is spelled out
            // and, when the sample is partial, so is the word used for the column.
            let column = if covers_all { "count" } else { "in sample" };
            text.push_str(&format!("  {:<12} {:>9}  {}\n", "extension", column, "size"));
            for (ext, (n, sz)) in by_ext.iter().take(40) {
                text.push_str(&format!(
                    "  {ext:<12} {n:>9}  {}\n",
                    everything::human_size(*sz)
                ));
            }
            if !covers_all {
                text.push_str(&format!(
                    "  (counts are rows within the {}-row sample, not totals in the result set)\n",
                    items.len()
                ));
            }
            structured["breakdown"] = json!(list);
            structured["breakdown_accuracy"] = json!(accuracy);
            structured["sampled"] = json!(items.len());
        }

        Ok(ToolOutput::new(text, structured))
    }

    /// Several independent searches in one call. The win is agent round trips, not
    /// server time: one search costs about a millisecond, one agent turn costs
    /// seconds.
    fn batch(&mut self, args: &Value) -> Result<ToolOutput, String> {
        let started = Instant::now();
        let queries = args
            .get("queries")
            .and_then(|v| v.as_array())
            .ok_or("queries is required (array of everything_search argument objects)")?;
        if queries.is_empty() || queries.len() > 8 {
            // Report the count that was actually received. Without it a transient
            // client-side serialisation problem and a genuinely oversized payload
            // produce the same message, and the caller cannot tell which happened.
            return Err(format!(
                "queries must contain 1-8 entries, got {}",
                queries.len()
            ));
        }
        let max_total = u(args, "max_total_results", 200).clamp(1, 2000);

        let mut text = String::new();
        let mut per_query: Vec<Value> = Vec::new();
        let mut executed: Vec<Value> = Vec::new();
        let mut total_returned = 0usize;

        for (i, q) in queries.iter().enumerate() {
            match self.search(q) {
                Ok(out) => {
                    let sc = out.structured;
                    let n = sc["returned"].as_u64().unwrap_or(0) as usize;
                    total_returned += n;
                    executed.push(sc["effective_query"].clone());
                    text.push_str(&format!(
                        "===== query {} of {} : {} hit(s) of {} matching\n",
                        i + 1,
                        queries.len(),
                        n,
                        sc["total"]
                    ));
                    text.push_str(&out.text);
                    text.push('\n');
                    per_query.push(json!({
                        "query": sc["query"],
                        "effective_query": sc["effective_query"],
                        "total": sc["total"],
                        "returned": sc["returned"],
                        "results": sc["results"],
                    }));
                }
                Err(e) => {
                    text.push_str(&format!("===== query {} : error: {e}\n", i + 1));
                    per_query.push(json!({"query": q.get("query").cloned().unwrap_or(json!("")),
                                          "error": e}));
                }
            }
            if total_returned >= max_total {
                text.push_str(&format!(
                    "\nstopped early: {total_returned} results reached max_total_results={max_total}\n"
                ));
                break;
            }
        }

        let structured = json!({
            "queries_requested": queries.len(),
            "queries_executed": executed,
            "total_returned": total_returned,
            "results": per_query,
            "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
        });
        Ok(ToolOutput::new(text, structured))
    }
}
