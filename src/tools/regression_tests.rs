// Loopback fixtures exercise the actual shared CLI/MCP HTTP and tool paths.
// Each test uses a temporary registry; no user's index or filesystem is searched.
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

struct MockIndex {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl MockIndex {
    fn new(reply: impl Fn(&str) -> Value + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen, done) = (requests.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        socket
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let mut reader = BufReader::new(socket.try_clone().unwrap());
                        let mut line = String::new();
                        if reader.read_line(&mut line).is_err() {
                            continue;
                        }
                        let request = line.clone();
                        loop {
                            line.clear();
                            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                                break;
                            }
                        }
                        seen.lock().unwrap().push(request.clone());
                        let body = reply(&request).to_string();
                        let _ = write!(
                            socket,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2))
                    }
                    Err(e) => panic!("mock accept failed: {e}"),
                }
            }
        });
        Self {
            url,
            requests,
            stop,
            thread: Some(thread),
        }
    }
    fn tools(&self) -> Tools {
        let mut config = Config::from_url(&self.url).unwrap();
        config.timeout = Duration::from_secs(1);
        Tools::new(Client::new(config))
    }
}
impl Drop for MockIndex {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn request_number(request: &str, name: &str) -> usize {
    request
        .split(&format!("&{name}="))
        .nth(1)
        .unwrap()
        .split(['&', ' '])
        .next()
        .unwrap()
        .parse()
        .unwrap()
}
fn row(name: &str, path: &str, size: Option<u64>) -> Value {
    let mut r = json!({"name":name,"path":path,"type":"file"});
    if let Some(n) = size {
        r["size"] = json!(n);
    }
    r
}
fn paged(rows: &[Value], request: &str) -> Value {
    let offset = request_number(request, "offset");
    let count = request_number(request, "count");
    json!({"totalResults":rows.len(),"results":rows.iter().skip(offset).take(count).cloned().collect::<Vec<_>>()})
}

#[test]
fn exact_size_consumes_folders_without_repeating_files() {
    let _reg = TestRegistry::new("{}");
    let rows = vec![
        row("a.txt", "C:\\x", Some(1)),
        json!({"name":"dir","path":"C:\\x","type":"folder"}),
        row("c.txt", "C:\\x", Some(3)),
    ];
    let mock = MockIndex::new(move |r| paged(&rows, r));
    let out = mock
        .tools()
        .count_stats(&json!({"query":"*","url":mock.url,"exact_size":true}))
        .unwrap();
    assert_eq!(
        out.structured["total_size"], 4,
        "folder rows consume an offset but add no file bytes"
    );
    assert_eq!(
        mock.requests.lock().unwrap().len(),
        1,
        "all three rows were already read"
    );
}
#[test]
fn exact_size_does_not_treat_unknown_file_size_as_zero() {
    let _reg = TestRegistry::new("{}");
    let rows = vec![
        row("a.txt", "C:\\x", Some(1)),
        row("b.txt", "C:\\x", None),
        row("c.txt", "C:\\x", Some(3)),
    ];
    let mock = MockIndex::new(move |r| paged(&rows, r));
    let out = mock
        .tools()
        .count_stats(&json!({"query":"*","url":mock.url,"exact_size":true}))
        .unwrap();
    assert_eq!(out.structured["total_size_accuracy"], "unavailable");
    assert!(out.structured.get("total_size").is_none());
}
#[test]
fn diversification_cursor_tracks_consumed_raw_rows() {
    let _reg = TestRegistry::new("{}");
    let rows = vec![
        row("a", "C:\\A", Some(1)),
        row("b", "C:\\A", Some(2)),
        row("c", "C:\\B", Some(3)),
        row("d", "C:\\C", Some(4)),
    ];
    let mock = MockIndex::new(move |r| paged(&rows, r));
    let mut tools = mock.tools();
    let mut args = json!({"query":"*","url":mock.url,"max_results":2,"max_per_parent":1});
    let first = tools.search(&args).unwrap();
    assert_eq!(
        first.structured["next_offset"], 3,
        "two displayed hits consumed three backend rows"
    );
    args["offset"] = first.structured["next_offset"].clone();
    let second = tools.search(&args).unwrap();
    assert_eq!(
        second.structured["results"][0]["name"], "d",
        "the second page must not repeat c"
    );
}
#[test]
fn probe_pages_the_engine_union_and_reports_its_total() {
    let _reg = TestRegistry::new("{}");
    let mock = MockIndex::new(|r| {
        if r.contains("%7C") {
            // the engine pages the expanded OR expression
            let offset = request_number(r, "offset");
            json!({"totalResults":100,"results":[row(&format!("union-{offset}"),"C:\\scope",Some(1))]})
        } else if r.contains("search=needle&") {
            json!({"totalResults":1,"results":[]})
        } else {
            json!({"totalResults":100,"results":[row("extra","C:\\scope",Some(1))]})
        }
    });
    let out = mock
        .tools()
        .search(&json!({"query":"needle","url":mock.url,"probe":true,"offset":3,"max_results":2}))
        .unwrap();
    assert_eq!(
        out.structured["total"], 100,
        "a handful of merged probe rows is not an exact union count"
    );
    assert_eq!(
        out.structured["results"][0]["name"], "union-3",
        "probes must respect the requested offset"
    );
}

#[test]
fn size_scan_rejects_early_eof_and_changed_count() {
    let _reg = TestRegistry::new("{}");
    for changed in [false, true] {
        let mock = MockIndex::new(move |r| {
            if request_number(r, "offset") == 0 {
                json!({"totalResults":3,"results":[row("first","C:\\x",Some(10))]})
            } else if changed {
                json!({"totalResults":2,"results":[row("second","C:\\x",Some(20))]})
            } else {
                json!({"totalResults":3,"results":[]})
            }
        });
        let out = mock
            .tools()
            .count_stats(&json!({"query":"*","url":mock.url,"exact_size":true}))
            .unwrap();
        assert_eq!(out.structured["total_size_accuracy"], "unavailable");
        assert!(out.structured.get("total_size").is_none());
    }
}
#[test]
fn short_pages_continue_until_reported_count_and_label_live_scan() {
    let _reg = TestRegistry::new("{}");
    let mock = MockIndex::new(|r| {
        let n = request_number(r, "offset");
        json!({"totalResults":3,"results":[row(&format!("f{n}"),"C:\\x",Some(10))]})
    });
    let out = mock
        .tools()
        .count_stats(&json!({"query":"*","url":mock.url,"exact_size":true}))
        .unwrap();
    assert_eq!(out.structured["total_size"], 30);
    assert_eq!(out.structured["rows_summed"], 3);
    assert!(out.structured["total_size_note"]
        .as_str()
        .unwrap()
        .contains("not an atomic snapshot"));
    assert_eq!(
        mock.requests
            .lock()
            .unwrap()
            .iter()
            .map(|r| request_number(r, "offset"))
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
}
#[test]
fn source_is_present_for_single_search_and_recent() {
    let _reg = TestRegistry::new("{}");
    let mock =
        MockIndex::new(|_| json!({"totalResults":1,"results":[row("a.txt","D:\\remote",Some(5))]}));
    let mut tools = mock.tools();
    let search = tools.search(&json!({"query":"*","url":mock.url})).unwrap();
    let recent = tools
        .find_recent(&json!({"url":mock.url,"auto_expand":false}))
        .unwrap();
    for out in [search, recent] {
        assert_eq!(
            out.structured["results"][0]["source"],
            mock.url.trim_start_matches("http://")
        );
        assert!(out.text.contains("source: 127.0.0.1:"));
    }
}
fn unused_url() -> String {
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    format!("http://{}", socket.local_addr().unwrap())
}
#[test]
fn partial_and_all_failed_stats_do_not_claim_complete_size() {
    let _reg = TestRegistry::new("{}");
    let mock = MockIndex::new(|_| json!({"totalResults":1,"results":[row("a","C:\\x",Some(5))]}));
    let dead = unused_url();
    for urls in [json!([mock.url, dead]), json!([dead, unused_url()])] {
        let out = mock
            .tools()
            .count_stats(&json!({"query":"*","url":urls,"exact_size":true}))
            .unwrap();
        assert_eq!(out.structured["count_accuracy"], "partial");
        assert_eq!(out.structured["count_scope"], "reachable indexes");
        assert_eq!(out.structured["total_size_accuracy"], "unavailable");
        assert!(out.structured.get("total_size").is_none());
    }
}
#[test]
fn remote_absence_and_failure_leave_existence_unknown() {
    let mock = MockIndex::new(|_| json!({"totalResults":0,"results":[]}));
    let mut text = String::new();
    let backend = Backend {
        name: "remote".into(),
        url: mock.url.clone(),
        config: Config::from_url(&mock.url).unwrap(),
    };
    let out = mock
        .tools()
        .index_details(&backend, "X:\\excluded\\a.txt", 0, &mut text);
    assert!(out["exists"].is_null());
    assert_eq!(out["indexed"], false);
    assert_eq!(out["status"], "not_indexed");
    let dead = unused_url();
    let backend = Backend {
        name: "down".into(),
        url: dead.clone(),
        config: Config::from_url(&dead).unwrap(),
    };
    let out = mock
        .tools()
        .index_details(&backend, "X:\\a.txt", 0, &mut text);
    assert!(out["exists"].is_null());
    assert!(out["indexed"].is_null());
    assert_eq!(out["status"], "unreachable");
}
#[test]
fn distinct_backend_cursors_can_be_resumed_separately() {
    let _reg = TestRegistry::new("{}");
    let first = MockIndex::new(|r| {
        paged(
            &vec![
                row("a", "A", Some(1)),
                row("b", "A", Some(2)),
                row("c", "B", Some(3)),
                row("d", "C", Some(4)),
            ],
            r,
        )
    });
    let second = MockIndex::new(|r| {
        paged(
            &vec![
                row("w", "W", Some(1)),
                row("x", "X", Some(2)),
                row("y", "Y", Some(3)),
                row("z", "Z", Some(4)),
            ],
            r,
        )
    });
    let mut tools = first.tools();
    let mut args =
        json!({"query":"*","url":[first.url,second.url],"max_results":2,"max_per_parent":1});
    let out = tools.search(&args).unwrap();
    assert!(out.structured["next_offset"].is_null());
    for (g, next_name) in out.structured["backends"]
        .as_array()
        .unwrap()
        .iter()
        .zip(["d", "y"])
    {
        args["url"] = g["url"].clone();
        args["offset"] = g["next_offset"].clone();
        let next = tools.search(&args).unwrap();
        assert_eq!(next.structured["results"][0]["name"], next_name);
    }
}
#[test]
fn batch_budget_is_hard_and_keeps_per_backend_pagination() {
    let _reg = TestRegistry::new("{}");
    let first = MockIndex::new(|r| {
        paged(
            &(0..10)
                .map(|i| row(&format!("a{i}"), "A", Some(1)))
                .collect::<Vec<_>>(),
            r,
        )
    });
    let second = MockIndex::new(|r| {
        paged(
            &(0..10)
                .map(|i| row(&format!("b{i}"), "B", Some(1)))
                .collect::<Vec<_>>(),
            r,
        )
    });
    let out=first.tools().batch(&json!({"max_total_results":3,"queries":[{"query":"*","url":[first.url,second.url],"max_results":10},{"query":"later","url":first.url}]})).unwrap();
    assert_eq!(out.structured["total_returned"], 3);
    assert_eq!(out.structured["queries_skipped"], 1);
    let query = &out.structured["results"][0];
    assert_eq!(query["has_more"], true);
    assert!(query["next_offset"].is_null());
    assert_eq!(query["backends"][0]["next_offset"], 3);
    assert_eq!(query["backends"][1]["next_offset"], 0);
    assert_eq!(query["results"].as_array().unwrap().len(), 3);
    assert_eq!(first.requests.lock().unwrap().len(), 1);
    assert!(first.requests.lock().unwrap()[0].contains("&count=3&"));
}
#[test]
fn probe_union_keeps_scope_and_match_modifiers() {
    let _reg = TestRegistry::new("{}");
    let mock = MockIndex::new(|_| json!({"totalResults":0,"results":[]}));
    let out=mock.tools().search(&json!({"query":"needle","url":mock.url,"probe":true,"path":"C:\\scope","category":"code","entry_type":"file","match_case":true,"match_whole_word":true,"offset":7})).unwrap();
    let effective = out.structured["effective_query"].as_str().unwrap();
    assert!(effective.starts_with("path:\"C:\\scope\" file: ext:"));
    assert!(effective.ends_with("<needle | path:needle | <folder: needle>>"));
    let requests = mock.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].contains("&offset=7&"));
    assert!(requests[0].contains("&case=1"));
    assert!(requests[0].contains("&wholeword=1"));
}
#[test]
fn truncated_remote_lookup_does_not_claim_unindexed() {
    let mock = MockIndex::new(
        |_| json!({"totalResults":100,"results":[row("other.txt","X:\\x",Some(1))]}),
    );
    let backend = Backend {
        name: "remote".into(),
        url: mock.url.clone(),
        config: Config::from_url(&mock.url).unwrap(),
    };
    let out = mock
        .tools()
        .index_details(&backend, "X:\\x\\wanted.txt", 0, &mut String::new());
    assert!(out["exists"].is_null());
    assert!(out["indexed"].is_null());
    assert_eq!(out["status"], "incomplete");
}
#[test]
fn breakdown_omits_unknown_size_instead_of_reporting_zero() {
    let _reg = TestRegistry::new("{}");
    let mock =
        MockIndex::new(|_| json!({"totalResults":1,"results":[row("missing.txt","C:\\x",None)]}));
    let out = mock
        .tools()
        .count_stats(&json!({"query":"*","url":mock.url,"breakdown_by_extension":true}))
        .unwrap();
    let entry = &out.structured["breakdown"][0];
    assert_eq!(entry["count"], 1);
    assert_eq!(entry["size_accuracy"], "unavailable");
    assert!(entry.get("size").is_none());
}
#[test]
fn all_failed_search_keeps_remaining_results_unknown() {
    let _reg = TestRegistry::new("{}");
    let a = unused_url();
    let b = unused_url();
    let mut tools = Tools::new(Client::new(Config::from_url(&a).unwrap()));
    let out = tools.search(&json!({"query":"*","url":[a,b]})).unwrap();
    assert_eq!(out.structured["total_accuracy"], "partial");
    assert!(out.structured["has_more"].is_null());
    assert!(out.structured["next_offset"].is_null());
}
#[test]
fn missing_total_is_not_silently_reported_as_exact_zero() {
    let _reg = TestRegistry::new("{}");
    let mock = MockIndex::new(|_| json!({"results":[]}));
    assert!(mock
        .tools()
        .search(&json!({"query":"*","url":mock.url}))
        .is_err());
}
