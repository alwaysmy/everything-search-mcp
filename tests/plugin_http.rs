//! End-to-end check of the in-process MCP endpoint: bind it on a loopback port,
//! speak HTTP to it, and confirm the MCP handshake and tool listing come back.
//!
//! This needs Everything's HTTP server for nothing — `initialize` and `tools/list`
//! are answered by the tool layer alone, so the test also runs on a machine where
//! Everything is not running. A `tools/call` smoke test lives in
//! `tests/live_search.rs`, which does require the real index.

use std::io::{Read, Write};
use std::net::TcpStream;

use serde_json::{json, Value};

use everything_mcp_core::plugin::server::Endpoint;
use everything_mcp_core::plugin::shared_handler;

fn http_post(port: u16, body: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to endpoint");
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).expect("write request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();
    (status, body)
}

/// Read exactly one HTTP response, leaving the socket usable for the next one.
fn read_response(stream: &mut TcpStream) -> (u16, String, String) {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 1024];
    let head_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        let n = stream.read(&mut chunk).expect("read response head");
        assert!(n > 0, "eof before response head finished");
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    let length: usize = head
        .lines()
        .find_map(|line| {
            let lower = line.to_ascii_lowercase();
            lower
                .strip_prefix("content-length:")
                .and_then(|value| value.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut chunk).expect("read response body");
        assert!(n > 0, "eof before response body finished");
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    (status, head, String::from_utf8_lossy(&body).to_string())
}

fn rpc(port: u16, payload: Value) -> Value {
    let (status, body) = http_post(port, &payload.to_string());
    assert_eq!(status, 200, "unexpected status for {payload}: {body}");
    serde_json::from_str(&body).unwrap_or_else(|error| panic!("not JSON ({error}): {body}"))
}

#[test]
fn mcp_handshake_and_tool_list_over_loopback_http() {
    let mut endpoint = Endpoint::start(0, shared_handler()).expect("bind endpoint");
    let port = endpoint.addr().port();

    let init = rpc(
        port,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0"}
            }
        }),
    );
    assert_eq!(
        init["result"]["serverInfo"]["name"],
        "everything_search_mcp"
    );

    let listed = rpc(
        port,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    );
    let tools = listed["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("no tools in {listed}"));
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    for expected in [
        "everything_search",
        "everything_count_stats",
        "everything_file_details",
    ] {
        assert!(names.contains(&expected), "missing {expected} in {names:?}");
    }

    // A notification alone must be accepted without a body.
    let (status, body) = http_post(
        port,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );
    assert_eq!(status, 202, "notification should be 202, body {body}");

    endpoint.stop();
}

/// Keep-alive: two calls on one socket, then a pipelined pair in a single write.
/// Without the leftover-buffer handling the second half of a pipelined pair is
/// silently dropped, which looks like a hung client rather than a lost request.
#[test]
fn reuses_a_connection_for_several_requests() {
    let mut endpoint = Endpoint::start(0, shared_handler()).expect("bind endpoint");
    let port = endpoint.addr().port();

    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string();
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nAccept: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );

    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    for id in 1..=2 {
        let reply = request.replace("\"id\":1", &format!("\"id\":{id}"));
        stream.write_all(reply.as_bytes()).expect("write request");
        let (status, head, response) = read_response(&mut stream);
        assert_eq!(status, 200, "request {id}: {response}");
        assert!(
            head.to_ascii_lowercase().contains("connection: keep-alive"),
            "server did not offer keep-alive: {head}"
        );
        let parsed: Value = serde_json::from_str(&response).expect("json response");
        assert_eq!(parsed["id"], json!(id));
    }

    // Pipelined: both requests leave in one write, both must be answered.
    let first = request.replace("\"id\":1", "\"id\":7");
    let second = request.replace("\"id\":1", "\"id\":8");
    stream
        .write_all(format!("{first}{second}").as_bytes())
        .expect("write pipelined pair");
    for id in 7..=8 {
        let (status, _, response) = read_response(&mut stream);
        assert_eq!(status, 200, "pipelined {id}: {response}");
        let parsed: Value = serde_json::from_str(&response).expect("json response");
        assert_eq!(parsed["id"], json!(id));
    }

    // And `Connection: close` must actually end the connection.
    let closing = request.replace("\"id\":1", "\"id\":9").replace(
        "Accept: application/json",
        "Accept: application/json\r\nConnection: close",
    );
    stream
        .write_all(closing.as_bytes())
        .expect("write closing request");
    let (status, head, _) = read_response(&mut stream);
    assert_eq!(status, 200);
    assert!(
        head.to_ascii_lowercase().contains("connection: close"),
        "closing request did not get a close: {head}"
    );
    let mut tail = [0u8; 1];
    assert_eq!(
        stream.read(&mut tail).unwrap_or(0),
        0,
        "socket stayed open after Connection: close"
    );

    endpoint.stop();
}

/// Regression guard for head-of-line blocking, as a barrier rather than a
/// stopwatch: request A's handler refuses to return until request B's handler
/// releases it. With one shared `Server` behind a mutex, B is never dispatched,
/// A waits until its own timeout, and this test fails deterministically. No
/// timing thresholds, so it cannot flake on a loaded machine.
#[test]
fn concurrent_requests_do_not_queue_behind_each_other() {
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use everything_mcp_core::jsonrpc::{Handler, Server, ToolOutput};
    use everything_mcp_core::plugin::server::HandlerFactory;
    // The trait's signatures name the crate's own serde_json, not the test's copy.
    use everything_mcp_core::serde_json::{json as core_json, Value as CoreValue};

    /// Blocks `block` until another connection runs `release`.
    struct Barrier {
        entered: Sender<()>,
        release: Arc<Mutex<Sender<()>>>,
        waiting: Arc<Mutex<Receiver<()>>>,
    }

    impl Handler for Barrier {
        fn list_tools(&self) -> CoreValue {
            core_json!([])
        }

        fn call_tool(&mut self, name: &str, _args: &CoreValue) -> Result<ToolOutput, String> {
            match name {
                "block" => {
                    // Tell the test we are inside the handler, then wait for the
                    // peer request. A serialized endpoint never gets to the peer.
                    self.entered.send(()).ok();
                    let channel = self.waiting.lock().unwrap();
                    channel.recv_timeout(Duration::from_secs(10)).map_err(|_| {
                        "peer request never arrived: calls were serialized".to_string()
                    })?;
                    Ok(ToolOutput::new("released", core_json!({"ok": true})))
                }
                "release" => {
                    self.release.lock().unwrap().send(()).ok();
                    Ok(ToolOutput::new(
                        "released the peer",
                        core_json!({"ok": true}),
                    ))
                }
                other => Err(format!("unknown tool {other}")),
            }
        }
    }

    let (entered, entered_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release = Arc::new(Mutex::new(release_tx));
    let waiting = Arc::new(Mutex::new(release_rx));

    let factory: HandlerFactory<Barrier> = {
        let release = Arc::clone(&release);
        let waiting = Arc::clone(&waiting);
        let entered = entered.clone();
        Arc::new(move || {
            Server::new(Barrier {
                entered: entered.clone(),
                release: Arc::clone(&release),
                waiting: Arc::clone(&waiting),
            })
        })
    };

    let mut endpoint = Endpoint::start(0, factory).expect("bind endpoint");
    let port = endpoint.addr().port();

    let blocked = {
        let body = json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "block", "arguments": {}}
        })
        .to_string();
        std::thread::spawn(move || http_post(port, &body))
    };

    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the blocking call never reached its handler");

    let (status, body) = http_post(
        port,
        &json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "release", "arguments": {}}
        })
        .to_string(),
    );
    assert_eq!(status, 200, "release call failed: {body}");

    let (status, body) = blocked.join().expect("blocking request panicked");
    assert_eq!(status, 200, "blocked call failed: {body}");
    let reply: Value = serde_json::from_str(&body).expect("blocked call returned non-JSON");
    assert!(
        reply["result"]["isError"] == json!(false),
        "blocked call was not released by its peer: {reply}"
    );

    endpoint.stop();
}

/// Guards the "one server per request" shape itself, so a lock cannot come back
/// unnoticed behind a still-parallel-looking API.
#[test]
fn every_request_builds_its_own_server() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use everything_mcp_core::everything::{Client, Config};
    use everything_mcp_core::jsonrpc::Server;
    use everything_mcp_core::plugin::server::HandlerFactory;
    use everything_mcp_core::tools::Tools;

    let built = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&built);
    let factory: HandlerFactory<Tools> = Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        Server::new(Tools::new(Client::new(Config::from_env())))
    });

    let mut endpoint = Endpoint::start(0, factory).expect("bind endpoint");
    let port = endpoint.addr().port();

    for id in 1..=3 {
        let (status, body) = http_post(
            port,
            &json!({"jsonrpc": "2.0", "id": id, "method": "ping"}).to_string(),
        );
        assert_eq!(status, 200, "ping {id}: {body}");
    }

    assert_eq!(
        built.load(Ordering::SeqCst),
        3,
        "expected one server per request, not one shared server"
    );

    endpoint.stop();
}
