//! Everything access over its built-in HTTP server.
//!
//! Why HTTP and not IPC/SDK: measured on Windows 10/11 with Everything 1.5,
//! a loopback HTTP round trip is ~0.7 ms versus ~17 ms for WM_COPYDATA IPC and
//! ~60 ms for spawning es.exe. The server answers from Everything's in-memory
//! index and needs no subprocess, no DLL and no window message pump.
//!
//! The server does not have to be on this machine. A [`Backend`] is one Everything
//! instance - the name an answer is attributed to plus the address that reaches it -
//! and a search can run against several at once. The paths in a result are paths on
//! the machine that answered, so every result carries its backend's name; without
//! that, a remote hit reads as a local file and the caller opens the wrong thing.
//!
//! The HTTP server must be reachable; see `Config::from_env` for the URL knob and
//! `crate::servers` for the registry of remote instances.

use serde::Deserialize;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

// ---------------------------------------------------------------- constants

/// Friendly sort name -> (Everything sort key, ascending)
pub fn resolve_sort(sort: &str) -> (&'static str, bool) {
    match sort {
        "name" => ("name", true),
        "name-desc" => ("name", false),
        "path" => ("path", true),
        "path-desc" => ("path", false),
        "size" | "size-asc" => ("size", true),
        "size-desc" => ("size", false),
        "date-created" | "date-created-asc" => ("date-created", true),
        "date-created-desc" => ("date-created", false),
        "extension" => ("extension", true),
        // Everything's own default
        "date-modified" | "date-modified-asc" => ("date-modified", true),
        _ => ("date-modified", false),
    }
}

pub const SORT_NAMES: &[&str] = &[
    "name", "name-desc", "path", "path-desc", "size", "size-asc", "size-desc",
    "date-modified", "date-modified-asc", "date-modified-desc",
    "date-created", "date-created-asc", "date-created-desc", "extension",
];

/// Category table: the single source of truth for the `category` parameter, the
/// published enum, and the `kind` field on results (which is deliberately the same
/// vocabulary, so a `kind` can be fed straight back into `category`).
pub const FILE_TYPES: &[(&str, &str)] = &[
    ("audio", "ext:mp3;wav;flac;aac;ogg;wma;m4a;opus;aiff;alac"),
    ("video", "ext:mp4;avi;mkv;mov;wmv;flv;webm;m4v;mpeg;mpg;3gp;ts"),
    ("image", "ext:jpg;jpeg;png;gif;bmp;svg;webp;tiff;tif;ico;raw;heic;heif;avif;psd"),
    (
        "document",
        "ext:pdf;doc;docx;xls;xlsx;ppt;pptx;odt;ods;odp;rtf;txt;md;epub;pages;numbers;key",
    ),
    (
        "code",
        "ext:py;js;ts;jsx;tsx;c;cpp;h;hpp;cs;java;go;rs;rb;php;swift;kt;scala;r;lua;sh;bash;\
         ps1;bat;cmd;sql;html;css;scss;sass;less;vue;svelte;dart;zig;nim;hx;ex;exs;erl;hs;ml;\
         fs;clj;lisp;asm;toml;yaml;yml;json;xml;ini;cfg;conf;env;dockerfile;makefile;cmake;\
         gradle;sbt;proto;graphql;tf;hcl",
    ),
    ("archive", "ext:zip;rar;7z;tar;gz;bz2;xz;tgz;zst;lz4;cab;iso;dmg"),
    ("executable", "ext:exe;msi;dll;sys;com;scr;appx;msix"),
    ("font", "ext:ttf;otf;woff;woff2;eot;fon"),
    ("3d", "ext:obj;fbx;stl;blend;dae;3ds;gltf;glb;usd;usda;usdz;step;iges"),
    (
        "data",
        "ext:csv;tsv;json;jsonl;ndjson;xml;sqlite;db;mdb;accdb;parquet;arrow;avro;hdf5;feather",
    ),
];

/// `file_type` category -> `ext:` clause (mirrors the Python server exactly).
pub fn file_type_query(kind: &str) -> Option<&'static str> {
    FILE_TYPES.iter().find(|(k, _)| *k == kind).map(|(_, v)| *v)
}

/// Category names in table order, so the published enum cannot drift from the table.
pub fn file_type_names() -> Vec<&'static str> {
    FILE_TYPES.iter().map(|(k, _)| *k).collect()
}

/// Accepted `period` values for `everything_find_recent` -> a complete Everything
/// expression.
///
/// The `dm:` prefix is part of the value on purpose. Everything has no notion of
/// a bare `last1week`: as a search term it matches nothing at all (measured:
/// 0 results, against 497,396 for `dm:last1week`). Returning the prefix-less
/// value and relying on the caller to add it is exactly how this was broken, so
/// the value is no longer separable from its prefix. `file_type_query` returns
/// `ext:...` for the same reason.
pub fn period_query(period: &str) -> Option<&'static str> {
    Some(match period {
        "1min" => "dm:last1min",
        "5min" => "dm:last5mins",
        "10min" => "dm:last10mins",
        "15min" => "dm:last15mins",
        "30min" => "dm:last30mins",
        "1hour" => "dm:last1hour",
        "2hours" => "dm:last2hours",
        "6hours" => "dm:last6hours",
        "12hours" => "dm:last12hours",
        "today" => "dm:today",
        "yesterday" => "dm:yesterday",
        "1day" => "dm:last1day",
        "3days" => "dm:last3days",
        "1week" => "dm:last1week",
        "2weeks" => "dm:last2weeks",
        "1month" => "dm:last1month",
        "3months" => "dm:last3months",
        "6months" => "dm:last6months",
        "1year" => "dm:last1year",
        _ => return None,
    })
}

pub const PERIOD_NAMES: &[&str] = &[
    "1min", "5min", "10min", "15min", "30min", "1hour", "2hours", "6hours", "12hours", "today",
    "yesterday", "1day", "3days", "1week", "2weeks", "1month", "3months", "6months", "1year",
];

/// Translate the numeric spellings Everything itself understands.
///
/// Everything accepts any `last<N><unit>` — `last7days`, `last24hours`,
/// `last90days` all work — and those are the obvious synonyms for the named
/// periods. An agent that asks for `7days` is right and this tool was wrong to
/// refuse it: the named list only exists for convenience, and it was being
/// enforced as a closed set. `1week` and `7days` are the same window.
pub fn numeric_period(period: &str) -> Option<String> {
    let digits: String = period.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let unit = match &period[digits.len()..] {
        "min" | "mins" | "minute" | "minutes" => "mins",
        "h" | "hr" | "hrs" | "hour" | "hours" => "hours",
        "d" | "day" | "days" => "days",
        "w" | "week" | "weeks" => "weeks",
        "mo" | "month" | "months" => "months",
        _ => return None,
    };
    let n: u64 = digits.parse().ok()?;
    if n == 0 {
        return None; // `last0days` is accepted by Everything and means nothing
    }
    // Everything spells the singular without the trailing s: last1day, last1hour.
    let unit = if n == 1 { unit.trim_end_matches('s') } else { unit };
    Some(format!("dm:last{n}{unit}"))
}

// ---------------------------------------------------------------- response

#[derive(Debug, Deserialize)]
pub struct RawResult {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub size: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub date_modified: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: String,
}

/// Everything returns these columns as JSON strings, but accept numbers too so a
/// format change cannot fail the whole response.
fn de_opt_string<'de, D>(d: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(match v {
        Some(serde_json::Value::String(s)) => Some(s),
        Some(serde_json::Value::Number(n)) => Some(n.to_string()),
        _ => None,
    })
}

#[derive(Debug, Deserialize)]
pub struct RawResponse {
    #[serde(rename = "totalResults", default)]
    pub total_results: u64,
    #[serde(default)]
    pub results: Vec<RawResult>,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub name: String,
    pub path: String,
    pub size: Option<u64>,
    pub modified: Option<String>,
    pub is_dir: bool,
}

impl Item {
    pub fn full_path(&self) -> String {
        if self.path.is_empty() {
            self.name.clone()
        } else {
            let sep = if self.path.ends_with('\\') { "" } else { "\\" };
            format!("{}{}{}", self.path, sep, self.name)
        }
    }
}

impl From<RawResult> for Item {
    fn from(r: RawResult) -> Self {
        Item {
            size: r.size.and_then(|s| s.parse::<u64>().ok()),
            modified: r.date_modified.as_deref().and_then(format_filetime),
            is_dir: r.kind == "folder",
            name: r.name,
            path: r.path,
        }
    }
}

/// Everything reports mtime as a Windows FILETIME - 100 ns ticks since
/// 1601-01-01 **UTC**. Everything's own UI shows local time and so does Explorer,
/// so the value is shifted into local time here; reported raw it was 8 hours off
/// on a UTC+8 machine, which is exactly the kind of error a "what changed
/// recently" answer must not have.
pub fn format_filetime(raw: &str) -> Option<String> {
    let ticks: u64 = raw.parse().ok()?;
    let unix = (ticks / 10_000_000) as i64 - 11_644_473_600;
    Some(format_unix(unix + local_offset_minutes() * 60))
}

/// Unix seconds to `YYYY-MM-DD HH:MM:SS`, with no timezone applied.
pub fn format_unix(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // civil_from_days (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as i64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        y,
        m,
        d,
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// Minutes to add to UTC to get local time, daylight saving included.
///
/// This is the crate's only `unsafe`: a single `GetTimeZoneInformation` call.
/// The alternatives were a date/time crate (a whole dependency tree for one
/// integer) or reading the registry from Rust (not possible without one either).
#[cfg(windows)]
pub fn local_offset_minutes() -> i64 {
    #[repr(C)]
    struct SystemTime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }
    /// `TIME_ZONE_INFORMATION`: i32 / [u16;32] / SYSTEMTIME / i32 / [u16;32] /
    /// SYSTEMTIME / i32, which is what `#[repr(C)]` lays out here.
    #[repr(C)]
    struct TimeZoneInformation {
        bias: i32,
        standard_name: [u16; 32],
        standard_date: SystemTime,
        standard_bias: i32,
        daylight_name: [u16; 32],
        daylight_date: SystemTime,
        daylight_bias: i32,
    }
    unsafe extern "system" {
        fn GetTimeZoneInformation(tzi: *mut TimeZoneInformation) -> u32;
    }
    const TIME_ZONE_ID_DAYLIGHT: u32 = 3;
    // SAFETY: the struct mirrors the documented Win32 layout, so the call writes
    // nothing outside this local; every field is a plain integer array.
    let bias = unsafe {
        let mut tzi = std::mem::zeroed::<TimeZoneInformation>();
        let id = GetTimeZoneInformation(&mut tzi);
        if id == TIME_ZONE_ID_DAYLIGHT {
            tzi.bias + tzi.daylight_bias
        } else {
            tzi.bias + tzi.standard_bias
        }
    };
    // `Bias` counts minutes *west* of UTC.
    -(bias as i64)
}

#[cfg(not(windows))]
pub fn local_offset_minutes() -> i64 {
    0
}

pub fn human_size(bytes: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{:.1} {}", v, U[i])
    }
}

/// Report an unambiguously malformed regular expression, or `None`.
///
/// Everything answers a broken pattern with zero matches rather than an error
/// (measured: `regex:^(unclosed[(` returns HTTP 200 with totalResults 0), so a
/// typo is indistinguishable from "nothing matched" — the worst failure mode a
/// search tool can have, because the caller concludes the file is not there.
///
/// Only faults that are wrong in every dialect are reported here: unbalanced
/// parentheses and character classes, and a trailing backslash. Anything subtler
/// is left to Everything rather than guessed at.
pub fn regex_syntax_problem(pattern: &str) -> Option<String> {
    let mut depth: i32 = 0;
    let mut class_at: Option<usize> = None; // 1-based character position of '['
    let mut escaped = false;
    for (pos, c) in pattern.chars().enumerate() {
        let at = pos + 1;
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '[' if class_at.is_none() => class_at = Some(at),
            ']' => {
                if let Some(start) = class_at {
                    // `[]]` and `[^]]`: a `]` in first position is a literal.
                    let body: String = pattern.chars().skip(start).take(at - start - 1).collect();
                    let body = body.strip_prefix('^').unwrap_or(&body);
                    if !body.is_empty() {
                        class_at = None;
                    }
                }
            }
            '(' if class_at.is_none() => depth += 1,
            ')' if class_at.is_none() => {
                depth -= 1;
                if depth < 0 {
                    return Some(format!("unmatched ')' at character {at}"));
                }
            }
            _ => {}
        }
    }
    if escaped {
        return Some("the pattern ends with a lone backslash".into());
    }
    if let Some(at) = class_at {
        return Some(format!(
            "unclosed character class: the '[' at character {at} is never closed"
        ));
    }
    if depth > 0 {
        return Some(format!("{depth} unclosed '('"));
    }
    None
}

// ---------------------------------------------------------------- config

pub const DEFAULT_URL: &str = "http://127.0.0.1:23333";
pub const DEFAULT_PORT: u16 = 23333;

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub timeout: Duration,
    pub max_results_cap: usize,
    /// Basic credentials taken from the URL's userinfo, when it carried any.
    pub auth: Option<(String, String)>,
}

impl Config {
    /// The local instance, from the environment.
    ///
    /// Deliberately lenient: this runs at startup where there is no error channel,
    /// and a malformed `EVERYTHING_HTTP_URL` must not stop the server from coming
    /// up. The strict parser is for addresses that arrive later, where a typo
    /// silently meaning "the local machine" would answer with the wrong files.
    pub fn from_env() -> Self {
        let url = std::env::var("EVERYTHING_HTTP_URL")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_URL.to_string());
        Self::from_url(&url).unwrap_or_else(|_| {
            Self::from_url(DEFAULT_URL).expect("the built-in default address parses")
        })
    }

    /// Parse one Everything address. `host`, `host:port`, `http://host:port` and
    /// `http://user:pass@host:port` are all accepted.
    pub fn from_url(url: &str) -> Result<Self, String> {
        let (host, port, auth) = parse_url(url)?;
        let timeout_ms = std::env::var("EVERYTHING_TIMEOUT")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(|s| s * 1000)
            .unwrap_or(30_000);
        let cap = std::env::var("EVERYTHING_MAX_RESULTS_CAP")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(1000);
        Ok(Config {
            host,
            port,
            timeout: Duration::from_millis(timeout_ms),
            max_results_cap: cap,
            auth,
        })
    }

    /// The same timeouts and caps, aimed at another instance. A per-call address
    /// should not quietly reset the limits the operator configured.
    pub fn retarget(&self, url: &str) -> Result<Config, String> {
        let mut c = Config::from_url(url)?;
        c.timeout = self.timeout;
        c.max_results_cap = self.max_results_cap;
        Ok(c)
    }

    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Split an Everything address into host, port and optional credentials.
///
/// Everything's HTTP server is plain HTTP only - there is no TLS option to enable -
/// so an `https://` address is refused rather than attempted and failed opaquely.
pub fn parse_url(raw: &str) -> Result<(String, u16, Option<(String, String)>), String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("the address is empty".into());
    }
    let rest = match s.split_once("://") {
        Some((scheme, r)) => {
            if !scheme.eq_ignore_ascii_case("http") {
                return Err(format!(
                    "unsupported scheme '{scheme}://' in '{raw}'. Everything's HTTP server \
                     serves plain HTTP; terminate TLS in a reverse proxy if you need it."
                ));
            }
            r
        }
        None => s,
    };
    // Credentials come off first, and with rsplit: an unencoded '@' in a password
    // is common, and splitting on the first one would then eat part of the host.
    let (auth, hostport) = match rest.rsplit_once('@') {
        Some((creds, hp)) => {
            let (u, p) = creds.split_once(':').unwrap_or((creds, ""));
            (Some((u.to_string(), p.to_string())), hp)
        }
        None => (None, rest),
    };
    let hostport = hostport.trim_end_matches('/');
    if hostport.is_empty() {
        return Err(format!("'{raw}' has no host"));
    }
    // A bracketed IPv6 literal, where the colons belong to the address and not to
    // the port separator.
    let (host, port, bracketed) = if let Some(after) = hostport.strip_prefix('[') {
        match after.split_once(']') {
            Some((h, tail)) => {
                let p = match tail.strip_prefix(':') {
                    Some(v) => Some(v),
                    None if tail.is_empty() => None,
                    None => {
                        return Err(format!("'{raw}' has trailing text after the IPv6 address"))
                    }
                };
                (h.to_string(), p, true)
            }
            None => return Err(format!("'{raw}' has an unclosed '[' in the host")),
        }
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), Some(p), false),
            None => (hostport.to_string(), None, false),
        }
    };
    if host.is_empty() {
        return Err(format!("'{raw}' has no host"));
    }
    // Only an unbracketed host is wrong for containing a colon; inside brackets the
    // colons are the address itself.
    if !bracketed && host.contains(':') {
        return Err(format!(
            "'{raw}' has an unbracketed IPv6 address. Write it as [{}]:{DEFAULT_PORT}.",
            host
        ));
    }
    let port = match port {
        None | Some("") => DEFAULT_PORT,
        Some(p) => p
            .parse::<u16>()
            .ok()
            .filter(|p| *p > 0)
            .ok_or_else(|| format!("'{p}' in '{raw}' is not a port number (expected 1-65535)"))?,
    };
    Ok((host, port, auth))
}

/// One Everything instance a search can run against.
#[derive(Debug, Clone)]
pub struct Backend {
    /// How this instance is named in results. `local` for this machine.
    pub name: String,
    /// The address as configured, echoed back so a caller can see what was queried.
    pub url: String,
    pub config: Config,
}

/// What one backend answered, or why it did not.
pub struct BackendOutcome {
    pub backend: Backend,
    pub result: Result<RawResponse, String>,
}

/// Run one query against several instances at once.
///
/// Parallel because the backends are independent and a remote one is tens of
/// milliseconds away rather than sub-millisecond like loopback; serial would add
/// their latencies together for no reason.
///
/// A backend that fails does not fail the search: its error is reported beside the
/// answers that did come back. One machine being switched off is not a reason to
/// withhold the files found on the others.
pub fn query_all(backends: &[Backend], q: &Query) -> Vec<BackendOutcome> {
    let run = |b: &Backend| Client::new(b.config.clone()).query(q);
    if backends.len() <= 1 {
        return backends
            .iter()
            .map(|b| BackendOutcome { backend: b.clone(), result: run(b) })
            .collect();
    }
    std::thread::scope(|scope| {
        let handles: Vec<_> = backends.iter().map(|b| scope.spawn(move || run(b))).collect();
        handles
            .into_iter()
            .zip(backends)
            .map(|(h, b)| BackendOutcome {
                backend: b.clone(),
                result: h
                    .join()
                    .unwrap_or_else(|_| Err("the query thread did not finish".to_string())),
            })
            .collect()
    })
}

/// Standard base64, for the Basic authorization header. Hand-rolled because one
/// header is not worth a dependency, and the alternative - refusing credentials in
/// the URL - would fail on the deployments that need them.
///
/// NOT verified against a live authenticated Everything instance: none was
/// available. The encoding itself is covered by unit tests.
fn base64(s: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let b = s.as_bytes();
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

// ---------------------------------------------------------------- transport

pub struct Client {
    cfg: Config,
}

#[derive(Debug)]
pub struct Query<'a> {
    pub search: &'a str,
    pub count: usize,
    pub offset: usize,
    pub sort: &'a str,
    pub ascending: bool,
    pub case: bool,
    pub whole_word: bool,
    pub regex: bool,
    pub match_path: bool,
    pub path: Option<&'a str>,
}

/// Reject unknown sort names instead of silently falling back to the default.
pub fn validate_sort(sort: &str) -> Result<(), String> {
    if SORT_NAMES.contains(&sort) {
        Ok(())
    } else {
        Err(format!(
            "invalid sort '{sort}'. Valid: {}",
            SORT_NAMES.join(", ")
        ))
    }
}

impl Client {
    pub fn new(cfg: Config) -> Self {
        Self { cfg }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// One HTTP/1.0 request on a fresh loopback connection (~0.7 ms measured).
    pub fn query(&self, q: &Query) -> Result<RawResponse, String> {
        let mut search = String::new();
        // Everything's HTTP API ignores its `path=`/`folder=` parameters, so the
        // directory is expressed with the `path:` search FUNCTION. It has to be a
        // function rather than a quoted literal: under regex matching the whole
        // search text becomes the pattern, and an injected literal path would be
        // read as part of that pattern (measured: 0 results).
        if let Some(p) = q.path.filter(|p| !p.trim().is_empty()) {
            let p = p.trim().trim_end_matches(|c| c == '\\' || c == '/');
            search.push_str("path:\"");
            search.push_str(p);
            search.push_str("\" ");
        }
        // Same reason: `regex:` is used as a function rather than the HTTP regex
        // flag, so that it composes with the `path:` function above (measured:
        // `path:"..." regex:^main\.rs$` returns exactly the expected hit).
        if q.regex {
            search.push_str("regex:");
        }
        search.push_str(q.search);

        let mut url = String::from("/?json=1&path_column=1&size_column=1&date_modified_column=1");
        url.push_str("&search=");
        url.push_str(&encode(&search));
        url.push_str(&format!("&count={}&offset={}", q.count, q.offset));
        let (sort, asc) = resolve_sort(q.sort);
        url.push_str("&sort=");
        url.push_str(&encode(sort));
        url.push_str(if q.ascending && asc { "&ascending=1" } else { "&ascending=0" });
        if q.case {
            url.push_str("&case=1");
        }
        if q.whole_word {
            url.push_str("&wholeword=1");
        }
        // `regex` is deliberately NOT sent as `&regex=1`; see the `regex:` function
        // above. Sending both would double-apply the pattern.
        //
        // `p=1` is the switch that makes the term match the full path instead of
        // the filename. It is absent from Everything's HTTP documentation; verified
        // live (274 -> 25686 results for a term that only occurs in paths).
        if q.match_path {
            url.push_str("&p=1");
        }

        let body = self.get(&url)?;
        serde_json::from_str::<RawResponse>(&body)
            .map_err(|e| format!("could not parse Everything's HTTP response: {e}"))
    }

    fn get(&self, path_and_query: &str) -> Result<String, String> {
        let addr = self.cfg.address();
        let mut stream = TcpStream::connect(&addr).map_err(|e| {
            // Two different situations, two different fixes. The local hint is the
            // one that answers "it does not work" on a fresh machine; the remote one
            // is about reachability, which the local hint would misdirect.
            if matches!(self.cfg.host.as_str(), "127.0.0.1" | "localhost" | "::1") {
                format!(
                    "cannot reach Everything's HTTP server at http://{addr} ({e}).\n\
                     Enable it in Everything: Tools > Options > HTTP Server, and restrict it to \
                     localhost by setting bindings=127.0.0.1 in Everything's Plugins.ini."
                )
            } else {
                format!(
                    "cannot reach the Everything HTTP server at http://{addr} ({e}).\n\
                     Check that the machine is up, that Everything is running on it, and that \
                     its HTTP server is enabled (Tools > Options > HTTP Server) on a binding \
                     that covers this network."
                )
            }
        })?;
        stream.set_read_timeout(Some(self.cfg.timeout)).ok();
        stream.set_write_timeout(Some(self.cfg.timeout)).ok();
        let mut req = format!(
            "GET {path_and_query} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: application/json\r\n",
            self.cfg.host
        );
        if let Some((user, pass)) = &self.cfg.auth {
            req.push_str(&format!(
                "Authorization: Basic {}\r\n",
                base64(&format!("{user}:{pass}"))
            ));
        }
        req.push_str("\r\n");
        stream.write_all(req.as_bytes()).map_err(|e| format!("write failed: {e}"))?;

        let mut reader = BufReader::new(stream);
        let mut status = String::new();
        reader.read_line(&mut status).map_err(|e| format!("read failed: {e}"))?;
        if !status.contains(" 200") {
            let line = status.trim();
            // A refusal is not a broken query: say which one it is, or the caller
            // spends the next hour rewriting a search that was never the problem.
            if line.contains(" 401") || line.contains(" 403") {
                return Err(format!(
                    "the Everything HTTP server at http://{addr} refused the request ({line}). \
                     It requires credentials - put them in the address as \
                     http://user:password@host:port."
                ));
            }
            return Err(format!("Everything HTTP server returned: {line}"));
        }
        let mut content_length: Option<usize> = None;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).map_err(|e| format!("read failed: {e}"))?;
            let t = line.trim_end();
            if t.is_empty() {
                break;
            }
            if let Some(v) = t.to_ascii_lowercase().strip_prefix("content-length:") {
                content_length = v.trim().parse().ok();
            }
        }
        let mut body = String::new();
        if let Some(n) = content_length {
            use std::io::Read;
            let mut buf = vec![0u8; n];
            reader.read_exact(&mut buf).map_err(|e| format!("read failed: {e}"))?;
            body.push_str(&String::from_utf8_lossy(&buf));
        } else {
            use std::io::Read;
            let mut s = String::new();
            reader.read_to_string(&mut s).map_err(|e| format!("read failed: {e}"))?;
            body = s;
        }
        Ok(body)
    }
}

/// Percent-encode everything outside the RFC 3986 unreserved set.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_epoch_formats_as_utc() {
        assert_eq!(format_unix(0), "1970-01-01 00:00:00");
        // 2026-09-12T08:40:37Z, the instant a real prefetch file was written
        assert_eq!(format_unix(1_789_202_437), "2026-09-12 08:40:37");
    }

    #[test]
    fn filetime_is_converted_to_local_not_left_in_utc() {
        // 2026-09-12T08:40:37Z as a FILETIME
        let got = format_filetime("134336760370000000").expect("parses");
        let local = format_unix(1_789_202_437 + local_offset_minutes() * 60);
        assert_eq!(got, local);
        // The regression this guards: a raw UTC render was exactly the offset off.
        if local_offset_minutes() != 0 {
            assert_ne!(got, "2026-09-12 08:40:37");
        }
    }

    #[test]
    fn local_offset_is_a_plausible_timezone() {
        let m = local_offset_minutes();
        assert!((-720..=840).contains(&m), "offset {m} minutes is not a real timezone");
        assert_eq!(m % 15, 0, "offsets are always a multiple of 15 minutes");
    }

    #[test]
    fn malformed_regexes_are_caught_and_valid_ones_are_not() {
        // The reported failure: Everything answers this with 0 matches and no
        // error, so the tool said "No results" for a typo.
        assert!(regex_syntax_problem("^(unclosed[(").is_some());
        assert!(regex_syntax_problem("(a|b").is_some());
        assert!(regex_syntax_problem("a)b").is_some());
        assert!(regex_syntax_problem("abc\\").is_some());
        assert!(regex_syntax_problem("[abc").is_some());
        assert!(regex_syntax_problem("[").is_some());

        // Valid patterns must pass, including the two constructs that look wrong
        // to a naive balance check.
        for ok in [
            "^main\\.rs$",
            "[]]",
            "[^]]",
            "a\\)b",
            "[a-z]+(\\d{2,3})?",
            "^(foo|bar)$",
            "\\(literal\\)",
            "x[[]y",
        ] {
            assert_eq!(regex_syntax_problem(ok), None, "should accept {ok:?}");
        }
    }

    #[test]
    fn numeric_periods_are_accepted_like_the_engine_does() {
        // Everything takes any `last<N><unit>`; refusing the numeric spelling was
        // this tool being stricter than the engine, and it cost an agent a failed
        // call for asking "7days" instead of "1week".
        assert_eq!(numeric_period("7days").as_deref(), Some("dm:last7days"));
        assert_eq!(numeric_period("1week").as_deref(), Some("dm:last1week"));
        assert_eq!(numeric_period("24hours").as_deref(), Some("dm:last24hours"));
        assert_eq!(numeric_period("90days").as_deref(), Some("dm:last90days"));
        assert_eq!(numeric_period("1d").as_deref(), Some("dm:last1day"));
        assert_eq!(numeric_period("30min").as_deref(), Some("dm:last30mins"));
        assert_eq!(numeric_period("1hour").as_deref(), Some("dm:last1hour"));
        assert_eq!(numeric_period("2mo").as_deref(), Some("dm:last2months"));

        // Not numeric periods: leave them to the named table or the error path.
        assert_eq!(numeric_period("7"), None);
        assert_eq!(numeric_period("7fortnights"), None);
        assert_eq!(numeric_period("days"), None);
        assert_eq!(numeric_period("0days"), None);
        assert_eq!(numeric_period("last7days"), None);

        // The singular spelling the engine wants must match the named table's.
        for p in PERIOD_NAMES {
            if let (Some(named), Some(numeric)) = (period_query(p), numeric_period(p)) {
                assert_eq!(named, numeric, "{p} disagreed between the two spellings");
            }
        }
    }

    #[test]
    fn every_period_carries_its_dm_prefix() {        // Everything has no bare `last1week`: as a term it matches nothing (0
        // results, versus 497,396 for `dm:last1week`). A period value without the
        // prefix therefore turns every recent-file search into an empty result,
        // which the auto-expand fallback then quietly answers from all time.
        for p in PERIOD_NAMES {
            let q = period_query(p).expect("every listed period must resolve");
            assert!(q.starts_with("dm:"), "period '{p}' resolved to '{q}' without dm:");
        }
        assert_eq!(period_query("1week"), Some("dm:last1week"));
        assert_eq!(period_query("today"), Some("dm:today"));
        assert_eq!(period_query("nonsense"), None);
    }

    #[test]
    fn addresses_parse_in_every_form_a_caller_writes() {
        let (h, p, a) = parse_url("http://10.0.0.2:23333").expect("parses");
        assert_eq!((h.as_str(), p, a), ("10.0.0.2", 23333, None));
        // Bare host and bare host:port, because that is how people write an address
        // in a config file.
        assert_eq!(parse_url("10.0.0.2").expect("parses").1, 23333);
        assert_eq!(parse_url("127.0.0.1:8080").expect("parses").1, 8080);
        assert_eq!(parse_url("http://host:80/").expect("parses").1, 80);
        // IPv6 has to be bracketed, or its colons read as the port separator.
        let (h, p, _) = parse_url("http://[::1]:23333").expect("parses");
        assert_eq!((h.as_str(), p), ("::1", 23333));

        // Credentials, including a password containing '@' unencoded.
        let (h, p, a) = parse_url("http://user:pass@10.0.0.2:23333").expect("parses");
        assert_eq!((h.as_str(), p), ("10.0.0.2", 23333));
        assert_eq!(a, Some(("user".into(), "pass".into())));
        let (_, _, a) = parse_url("http://user:p@ss@10.0.0.2:23333").expect("parses");
        assert_eq!(a, Some(("user".into(), "p@ss".into())));
    }

    #[test]
    fn a_malformed_address_is_refused_rather_than_quietly_becoming_local() {
        // The failure this guards: a typo silently falling back to 127.0.0.1 would
        // answer a question about another machine with this machine's files.
        assert!(parse_url("").is_err());
        assert!(parse_url("http://").is_err());
        assert!(parse_url("https://10.0.0.2:23333").is_err(), "no TLS on that server");
        assert!(parse_url("10.0.0.2:notaport").is_err());
        assert!(parse_url("10.0.0.2:0").is_err());
        assert!(parse_url("10.0.0.2:99999").is_err());
        assert!(parse_url("http://::1:23333").is_err(), "unbracketed IPv6");
        assert!(parse_url("http://[::1:23333").is_err());
    }

    #[test]
    fn base64_matches_the_rfc_vectors() {
        // RFC 4648 section 10, plus the one string this actually encodes.
        for (input, want) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
            ("user:pass", "dXNlcjpwYXNz"),
        ] {
            assert_eq!(base64(input), want, "base64({input:?})");
        }
    }

    #[test]
    fn a_per_call_address_keeps_the_operators_timeouts_and_caps() {
        let base = Config::from_url("http://127.0.0.1:23333").expect("parses");
        let other = base.retarget("http://10.0.0.2:23333").expect("parses");
        assert_eq!(other.host, "10.0.0.2");
        assert_eq!(other.timeout, base.timeout);
        assert_eq!(other.max_results_cap, base.max_results_cap);
        assert!(base.retarget("nonsense:port").is_err());
    }
}
