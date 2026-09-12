//! The five MCP tools, their published (flat) input schemas, and result formatting.

use crate::everything::{
    self, Client, Item, Query, FILE_TYPE_NAMES, PERIOD_NAMES, SORT_NAMES,
};
use crate::jsonrpc::Handler;
use serde_json::{json, Value};
use std::path::Path;

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

fn tool(name: &str, description: &str, input: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input,
    })
}

fn p_string(desc: &str, default: Option<&str>) -> Value {
    match default {
        Some(d) => json!({"type": "string", "description": desc, "default": d}),
        None => json!({"type": "string", "description": desc}),
    }
}

fn p_int(desc: &str, default: i64, min: i64, max: i64) -> Value {
    json!({"type": "integer", "description": desc, "default": default, "minimum": min, "maximum": max})
}

fn p_bool(desc: &str, default: bool) -> Value {
    json!({"type": "boolean", "description": desc, "default": default})
}

// ---------------------------------------------------------------- Handler

impl Handler for Tools {
    fn list_tools(&self) -> Value {
        let sort_desc = format!("Sort order. One of: {}", SORT_NAMES.join(", "));
        let search_schema = schema(
            json!({
                "query": p_string("Search query using Everything syntax. Examples: '*.py', 'ext:py;js', 'size:>10mb ext:log', 'dm:today ext:py'. Space = AND, | = OR, ! excludes. Prefer the 'path' parameter over embedding path: in the query.", None),
                "path": p_string("Restrict search to this directory. Prefer this over embedding path: in the query string.", Some("")),
                "max_results": p_int("Maximum results to return (1-500)", 50, 1, 500),
                "offset": p_int("Skip N results (pagination)", 0, 0, i64::MAX),
                "sort": p_string(&sort_desc, Some("date-modified-desc")),
                "match_case": p_bool("Case-sensitive search", false),
                "match_whole_word": p_bool("Match whole words only", false),
                "match_regex": p_bool("Treat query as a regex", false),
                "match_path": p_bool("Match against the full path, not just the filename", false),
                "include_total": p_bool("Also report the total number of matches. Default false to keep searches fast.", false),
            }),
            &["query"],
        );

        let by_type_schema = schema(
            json!({
                "file_type": p_string(&format!("File type category. One of: {}", FILE_TYPE_NAMES.join(", ")), None),
                "query": p_string("Additional search filter", Some("")),
                "path": p_string("Restrict search to this directory", Some("")),
                "max_results": p_int("Maximum results to return (1-500)", 50, 1, 500),
                "sort": p_string(&sort_desc, Some("date-modified-desc")),
            }),
            &["file_type"],
        );

        let recent_schema = schema(
            json!({
                "period": p_string(&format!("How recent. Options: {}. Or raw Everything syntax like 'last2hours'.", PERIOD_NAMES.join(", ")), Some("1day")),
                "path": p_string("Restrict to this directory path", Some("")),
                "extensions": p_string("Filter by extensions, e.g. 'py,js,ts' or 'py;js;ts'", Some("")),
                "query": p_string("Additional search filter", Some("")),
                "max_results": p_int("Maximum results to return (1-500)", 50, 1, 500),
                "auto_expand": p_bool("When the time period yields fewer than max_results hits, retry without the time restriction (all time). Set false to keep a strict period.", true),
            }),
            &[],
        );

        let details_schema = schema(
            json!({
                "paths": json!({
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "File/folder paths to inspect (1-20). Must be absolute: the server working directory is not predictable.",
                    "minItems": 1,
                    "maxItems": 20
                }),
                "preview_lines": p_int("Lines of text content to preview (0 = none, max 200)", 0, 0, 200),
            }),
            &["paths"],
        );

        let stats_schema = schema(
            json!({
                "query": p_string("Search query to count/measure. Same syntax as everything_search.", None),
                "path": p_string("Restrict counting to this directory", Some("")),
                "include_size": p_bool("Also calculate total size of all matching files", true),
                "breakdown_by_extension": p_bool("Break down count and size by file extension (samples up to 500 results)", false),
                "sample_sort": p_string(&sort_desc, Some("date-modified-desc")),
            }),
            &["query"],
        );

        json!({"tools": [
            tool("everything_search",
                 "Search for files and folders instantly using voidtools Everything. Leverages Everything's real-time NTFS index for sub-millisecond search across all local and mapped drives. Supports wildcards, regex, size/date filters, extension filters, path restrictions, and content search.",
                 search_schema),
            tool("everything_search_by_type",
                 "Search for files by category (audio, video, image, document, code, archive, executable, font, 3d, data) without hand-writing extension lists.",
                 by_type_schema),
            tool("everything_find_recent",
                 "Find files modified within a recent time period. Ideal for discovering what changed in a project, tracking recent downloads, or finding today's logs. Sorted newest-first.",
                 recent_schema),
            tool("everything_file_details",
                 "Get detailed metadata and optional content preview for specific files. Returns full path, size, dates, type. For directories: entries. For text files with preview_lines > 0: first N lines.",
                 details_schema),
            tool("everything_count_stats",
                 "Get count and size statistics for files matching a query without listing every file. Optionally breaks down by extension.",
                 stats_schema),
        ]})
    }

    fn call_tool(&mut self, name: &str, args: &Value) -> Result<String, String> {
        match name {
            "everything_search" => self.search(args),
            "everything_search_by_type" => self.search_by_type(args),
            "everything_find_recent" => self.find_recent(args),
            "everything_file_details" => self.file_details(args),
            "everything_count_stats" => self.count_stats(args),
            other => Err(format!("unknown tool: {other}")),
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

fn cap(cfg: &everything::Config, n: usize) -> usize {
    n.clamp(1, cfg.max_results_cap.max(1))
}

// ---------------------------------------------------------------- tools

impl Tools {
    fn run(&self, q: &Query) -> Result<everything::RawResponse, String> {
        self.client.query(q)
    }

    fn search(&mut self, args: &Value) -> Result<String, String> {
        let query = s(args, "query");
        if query.trim().is_empty() {
            return Err("query is required and must not be empty".into());
        }
        let max = cap(self.client.config(), u(args, "max_results", 50));
        let offset = u(args, "offset", 0);
        let sort = s_or(args, "sort", "date-modified-desc");
        let path = s(args, "path");
        let include_total = b(args, "include_total", false);

        if b(args, "match_path", false) {
            // Everything's HTTP API has no match-path switch; the equivalent is the
            // `path:` search function, which callers can write directly in `query`.
        }

        let resp = self.run(&Query {
            search: &query,
            count: max,
            offset,
            sort: &sort,
            ascending: !sort.ends_with("-desc"),
            case: b(args, "match_case", false),
            whole_word: b(args, "match_whole_word", false),
            regex: b(args, "match_regex", false),
            path: if path.trim().is_empty() { None } else { Some(path.as_str()) },
        })?;

        let items: Vec<Item> = resp.results.into_iter().map(Item::from).collect();
        // format_results already renders the empty case; keep the total regardless.
        let mut out = format_results(&query, &items, offset);
        if include_total {
            out.push_str(&format!("\nTotal matches: {}", resp.total_results));
        }
        Ok(out)
    }

    fn search_by_type(&mut self, args: &Value) -> Result<String, String> {
        let kind = s(args, "file_type");
        let ext_clause = everything::file_type_query(&kind).ok_or_else(|| {
            format!(
                "invalid file_type '{kind}'. Valid values: {}",
                FILE_TYPE_NAMES.join(", ")
            )
        })?;
        let extra = s(args, "query");
        let search = if extra.trim().is_empty() {
            ext_clause.to_string()
        } else {
            format!("{ext_clause} {extra}")
        };
        let max = cap(self.client.config(), u(args, "max_results", 50));
        let sort = s_or(args, "sort", "date-modified-desc");
        let path = s(args, "path");
        let resp = self.run(&Query {
            search: &search,
            count: max,
            offset: 0,
            sort: &sort,
            ascending: !sort.ends_with("-desc"),
            case: false,
            whole_word: false,
            regex: false,
            path: if path.trim().is_empty() { None } else { Some(path.as_str()) },
        })?;
        let label = if extra.trim().is_empty() {
            format!("type:{kind}")
        } else {
            format!("type:{kind} {extra}")
        };
        let items: Vec<Item> = resp.results.into_iter().map(Item::from).collect();
        Ok(format_results(&label, &items, 0))
    }

    fn find_recent(&mut self, args: &Value) -> Result<String, String> {
        let period = s_or(args, "period", "1day");
        let dm = everything::period_query(&period).ok_or_else(|| {
            format!(
                "invalid period '{period}'. Valid values: {} (or raw Everything syntax like 'last2hours')",
                PERIOD_NAMES.join(", ")
            )
        })?;
        let max = cap(self.client.config(), u(args, "max_results", 50));
        let path = s(args, "path");
        let path_ref = if path.trim().is_empty() { None } else { Some(path.as_str()) };
        let extra = s(args, "query");
        let extensions = s(args, "extensions");

        let build = |with_time: bool| {
            let mut parts: Vec<String> = Vec::new();
            if with_time {
                parts.push(dm.to_string());
            }
            if !extensions.trim().is_empty() {
                let list = extensions.replace(',', ";");
                parts.push(format!("ext:{list}"));
            }
            if !extra.trim().is_empty() {
                parts.push(extra.clone());
            }
            parts.join(" ")
        };

        let mut resp = self.run(&Query {
            search: &build(true),
            count: max,
            offset: 0,
            sort: "date-modified-desc",
            ascending: false,
            case: false,
            whole_word: false,
            regex: false,
            path: path_ref,
        })?;

        // auto_expand: the period produced fewer than requested -> retry across all time.
        let auto_expand = b(args, "auto_expand", true);
        let expanded = auto_expand && resp.results.len() < max && resp.total_results < max as u64;
        if expanded {
            resp = self.run(&Query {
                search: &build(false),
                count: max,
                offset: 0,
                sort: "date-modified-desc",
                ascending: false,
                case: false,
                whole_word: false,
                regex: false,
                path: path_ref,
            })?;
        }

        let items: Vec<Item> = resp.results.into_iter().map(Item::from).collect();
        let mut out = format_results(&format!("recent ({period})"), &items, 0);
        if expanded {
            out.push_str("\n(no more results in that period - expanded to all time)");
        }
        Ok(out)
    }

    fn file_details(&mut self, args: &Value) -> Result<String, String> {
        let paths = args
            .get("paths")
            .and_then(|v| v.as_array())
            .ok_or("paths is required (array of absolute paths)")?;
        if paths.is_empty() || paths.len() > 20 {
            return Err("paths must contain 1-20 entries".into());
        }
        let preview = u(args, "preview_lines", 0).min(200);
        let mut out = String::new();
        for pv in paths {
            let p = pv.as_str().unwrap_or("");
            let path = Path::new(p);
            if !path.is_absolute() {
                out.push_str(&format!("{p}\n  error: path must be absolute\n\n"));
                continue;
            }
            match std::fs::metadata(path) {
                Ok(md) => {
                    out.push_str(&format!("{p}\n"));
                    if md.is_dir() {
                        out.push_str("  type: directory\n");
                        let mut n = 0usize;
                        if let Ok(rd) = std::fs::read_dir(path) {
                            let mut names: Vec<String> = rd
                                .flatten()
                                .take(50)
                                .map(|e| e.file_name().to_string_lossy().to_string())
                                .collect();
                            names.sort();
                            n = names.len();
                            for name in names.iter().take(20) {
                                out.push_str(&format!("    - {name}\n"));
                            }
                        }
                        out.push_str(&format!("  entries (first {n}): {n}\n"));
                    } else {
                        out.push_str(&format!("  size: {} ({} bytes)\n", everything::human_size(md.len()), md.len()));
                        out.push_str(&format!("  type: {}\n", if md.is_file() { "file" } else { "other" }));
                        if preview > 0 {
                            match std::fs::read(path) {
                                Ok(bytes) => {
                                    let text = String::from_utf8_lossy(&bytes);
                                    let lines: Vec<&str> =
                                        text.lines().take(preview).collect();
                                    out.push_str("  preview:\n");
                                    for l in lines {
                                        out.push_str(&format!("    {l}\n"));
                                    }
                                }
                                Err(e) => out.push_str(&format!("  preview unavailable: {e}\n")),
                            }
                        }
                    }
                    out.push('\n');
                }
                Err(e) => {
                    out.push_str(&format!("{p}\n  error: {e}\n\n"));
                }
            }
        }
        Ok(out)
    }

    fn count_stats(&mut self, args: &Value) -> Result<String, String> {
        let query = s(args, "query");
        if query.trim().is_empty() {
            return Err("query is required".into());
        }
        let path = s(args, "path");
        let include_size = b(args, "include_size", true);
        let breakdown = b(args, "breakdown_by_extension", false);
        let sample_sort = s_or(args, "sample_sort", "date-modified-desc");
        if sample_sort == "name" || sample_sort == "name-desc" {
            return Err(
                "sample_sort 'name' is rejected: file-name sort correlates with extension and biases the sample. Use a date or size sort.".into(),
            );
        }
        let resp = self.run(&Query {
            search: &query,
            count: 500,
            offset: 0,
            sort: &sample_sort,
            ascending: !sample_sort.ends_with("-desc"),
            case: false,
            whole_word: false,
            regex: false,
            path: if path.trim().is_empty() { None } else { Some(path.as_str()) },
        })?;

        let total = resp.total_results;
        let items: Vec<Item> = resp.results.into_iter().map(Item::from).collect();
        let mut out = format!("query: {query}\n");
        if !path.trim().is_empty() {
            out.push_str(&format!("path: {path}\n"));
        }
        out.push_str(&format!("total_count: {total}\n"));
        if include_size {
            let known: Vec<u64> = items.iter().filter_map(|i| i.size).collect();
            let sum: u64 = known.iter().sum();
            out.push_str(&format!(
                "sampled_size: {} ({} bytes across {} sampled results)\n",
                everything::human_size(sum),
                sum,
                known.len()
            ));
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
            out.push_str("breakdown (sampled):\n");
            for (ext, (n, sz)) in by_ext.iter().take(30) {
                out.push_str(&format!("  {ext:<12} {n:>5}  {}\n", everything::human_size(*sz)));
            }
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- formatting

fn format_results(label: &str, items: &[Item], offset: usize) -> String {
    if items.is_empty() {
        return format!("No results for: {label}\n");
    }
    let mut out = format!("Found {} results for: {label}\n\n", items.len());
    for it in items {
        let tag = if it.is_dir { "DIR " } else { "FILE" };
        let mut meta: Vec<String> = Vec::new();
        if let Some(sz) = it.size {
            meta.push(everything::human_size(sz));
        }
        if let Some(m) = &it.modified {
            meta.push(m.clone());
        }
        if meta.is_empty() {
            out.push_str(&format!("  [{tag}] {}\n", it.full_path()));
        } else {
            out.push_str(&format!("  [{tag}] {}  ({})\n", it.full_path(), meta.join(", ")));
        }
    }
    out.push_str(&format!(
        "\nShowing {} results from offset {}. Use 'offset' to paginate or refine the query.",
        items.len(),
        offset
    ));
    out
}
