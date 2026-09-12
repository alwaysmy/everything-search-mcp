//! Everything access over its built-in HTTP server on loopback.
//!
//! Why HTTP and not IPC/SDK: measured on Windows 10/11 with Everything 1.5,
//! a loopback HTTP round trip is ~0.7 ms versus ~17 ms for WM_COPYDATA IPC and
//! ~60 ms for spawning es.exe. The server answers from Everything's in-memory
//! index and needs no subprocess, no DLL and no window message pump.
//!
//! The HTTP server must be reachable; see `Config::from_env` for the URL knob.

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

/// Accepted `period` values for `everything_find_recent` -> Everything `dm:` syntax.
pub fn period_query(period: &str) -> Option<&'static str> {
    Some(match period {
        "1min" => "last1min",
        "5min" => "last5mins",
        "10min" => "last10mins",
        "15min" => "last15mins",
        "30min" => "last30mins",
        "1hour" => "last1hour",
        "2hours" => "last2hours",
        "6hours" => "last6hours",
        "12hours" => "last12hours",
        "today" => "today",
        "yesterday" => "yesterday",
        "1day" => "last1day",
        "3days" => "last3days",
        "1week" => "last1week",
        "2weeks" => "last2weeks",
        "1month" => "last1month",
        "3months" => "last3months",
        "6months" => "last6months",
        "1year" => "last1year",
        _ => return None,
    })
}

pub const PERIOD_NAMES: &[&str] = &[
    "1min", "5min", "10min", "15min", "30min", "1hour", "2hours", "6hours", "12hours", "today",
    "yesterday", "1day", "3days", "1week", "2weeks", "1month", "3months", "6months", "1year",
];

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

// ---------------------------------------------------------------- config

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub timeout: Duration,
    pub max_results_cap: usize,
}

impl Config {
    pub fn from_env() -> Self {
        let url = std::env::var("EVERYTHING_HTTP_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:23333".to_string());
        let stripped = url.trim_end_matches('/');
        let rest = stripped.strip_prefix("http://").unwrap_or(stripped);
        let (host, port) = match rest.split_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().unwrap_or(23333)),
            None => (rest.to_string(), 23333),
        };
        let timeout_ms = std::env::var("EVERYTHING_TIMEOUT")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(|s| s * 1000)
            .unwrap_or(30_000);
        let cap = std::env::var("EVERYTHING_MAX_RESULTS_CAP")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(1000);
        Config { host, port, timeout: Duration::from_millis(timeout_ms), max_results_cap: cap }
    }
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
        let addr = format!("{}:{}", self.cfg.host, self.cfg.port);
        let mut stream = TcpStream::connect(&addr).map_err(|e| {
            format!(
                "cannot reach Everything's HTTP server at http://{addr} ({e}).\n\
                 Enable it in Everything: Tools > Options > HTTP Server, and restrict it to \
                 localhost by setting bindings=127.0.0.1 in Everything's Plugins.ini."
            )
        })?;
        stream.set_read_timeout(Some(self.cfg.timeout)).ok();
        stream.set_write_timeout(Some(self.cfg.timeout)).ok();
        let req = format!(
            "GET {path_and_query} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: application/json\r\n\r\n",
            self.cfg.host
        );
        stream.write_all(req.as_bytes()).map_err(|e| format!("write failed: {e}"))?;

        let mut reader = BufReader::new(stream);
        let mut status = String::new();
        reader.read_line(&mut status).map_err(|e| format!("read failed: {e}"))?;
        if !status.contains(" 200") {
            return Err(format!("Everything HTTP server returned: {}", status.trim()));
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
}
