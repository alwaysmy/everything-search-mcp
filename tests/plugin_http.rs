//! End-to-end check of the in-process MCP endpoint: bind it on a loopback port,
//! speak HTTP to it, and confirm the MCP handshake and tool listing come back.
//!
//! This needs Everything's HTTP server for nothing — `initialize` and `tools/list`
//! are answered by the tool layer alone, so the test also runs on a machine where
//! Everything is not running. A `tools/call` smoke test lives in
//! the live scripts under `TEST_SCRIPTS\`, which do require the real index.

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

/// `Expect: 100-continue` is a handshake, not a header to ignore: a client that
/// sends it waits for the interim response before sending the body. Answering
/// nothing would leave it waiting until it times out.
#[test]
fn answers_expect_100_continue_before_the_body() {
    let mut endpoint = Endpoint::start(0, shared_handler()).expect("bind endpoint");
    let port = endpoint.addr().port();

    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string();
    let head = format!(
        "POST /mcp HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nAccept: application/json\r\nExpect: 100-continue\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream.write_all(head.as_bytes()).expect("write head");

    let (status, _, _) = read_response(&mut stream);
    assert_eq!(status, 100, "server did not answer the continue handshake");

    stream.write_all(body.as_bytes()).expect("write body");
    let (status, _, response) = read_response(&mut stream);
    assert_eq!(status, 200, "request after continue failed: {response}");

    endpoint.stop();
}

/// A chunked body read by a Content-Length-only parser looks like no body at all.
/// Refusing it at the door is the difference between a clear 411 and a confusing
/// "empty body" from the JSON-RPC layer.
#[test]
fn refuses_chunked_request_bodies() {
    let mut endpoint = Endpoint::start(0, shared_handler()).expect("bind endpoint");
    let port = endpoint.addr().port();

    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .write_all(
            b"POST /mcp HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n",
        )
        .expect("write chunked request");
    let (status, _, body) = read_response(&mut stream);
    assert_eq!(
        status, 411,
        "expected 411 for chunked body, got {status}: {body}"
    );

    endpoint.stop();
}

/// Keep-alive turns every idle socket into a held thread, and this code runs
/// inside Everything.exe. The cap is what keeps that from becoming a way to stall
/// the file manager; the refusal has to be a real answer, not a hang.
#[test]
fn refuses_connections_beyond_the_limit() {
    use std::time::Duration;

    let mut endpoint = Endpoint::start_with(0, shared_handler(), 2, Duration::from_secs(5))
        .expect("bind endpoint");
    let port = endpoint.addr().port();

    let held: Vec<TcpStream> = (0..2)
        .map(|_| TcpStream::connect(("127.0.0.1", port)).expect("connect"))
        .collect();
    // Slots are taken at accept, so connecting is enough to fill them.
    std::thread::sleep(Duration::from_millis(200));

    let mut extra = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let (status, head, body) = read_response(&mut extra);
    assert_eq!(
        status, 503,
        "expected a refusal, got {status}: {head} {body}"
    );

    // Releasing a slot must let the next client in.
    drop(held);
    std::thread::sleep(Duration::from_millis(300));
    let mut after = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let ping = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string();
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nAccept: application/json\r\nContent-Length: {}\r\n\r\n{ping}",
        ping.len()
    );
    after
        .write_all(request.as_bytes())
        .expect("write after release");
    let (status, _, body) = read_response(&mut after);
    assert_eq!(status, 200, "slot was not released: {body}");

    endpoint.stop();
}

/// An idle kept-alive connection must actually go away, otherwise a client that
/// closed its side uncleanly leaves a thread behind for as long as the process
/// lives.
#[test]
fn closes_an_idle_kept_alive_connection() {
    use std::time::{Duration, Instant};

    let mut endpoint = Endpoint::start_with(0, shared_handler(), 8, Duration::from_millis(200))
        .expect("bind endpoint");
    let port = endpoint.addr().port();

    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let ping = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string();
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nAccept: application/json\r\nContent-Length: {}\r\n\r\n{ping}",
        ping.len()
    );
    stream.write_all(request.as_bytes()).expect("write request");
    let (status, head, _) = read_response(&mut stream);
    assert_eq!(status, 200);
    assert!(head.to_ascii_lowercase().contains("connection: keep-alive"));

    // The server closes after its idle window; observe the socket going away.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut tail = [0u8; 1];
    loop {
        match stream.read(&mut tail) {
            Ok(0) => break,
            Ok(_) => panic!("server sent data on an idle connection"),
            Err(error) => {
                assert!(
                    Instant::now() < deadline,
                    "idle connection was not closed: {error}"
                );
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    endpoint.stop();
}

/// A request the parser cannot honour must get a status line, not a silent close.
/// "Connection reset" with no body is the failure mode this whole endpoint exists
/// to avoid looking like.
#[test]
fn refuses_what_it_cannot_parse_with_a_status() {
    let mut endpoint = Endpoint::start(0, shared_handler()).expect("bind endpoint");
    let port = endpoint.addr().port();

    // Body larger than MAX_BODY.
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .write_all(
            b"POST /mcp HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: 300000\r\n\r\n",
        )
        .expect("write oversized head");
    let (status, _, _) = read_response(&mut stream);
    assert_eq!(status, 413, "expected 413 for an oversized body");

    // Content-Length that is not a number.
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .write_all(b"POST /mcp HTTP/1.1\r\nHost: x\r\nContent-Length: nope\r\n\r\n")
        .expect("write bad content-length");
    let (status, _, _) = read_response(&mut stream);
    assert_eq!(status, 400, "expected 400 for a bad Content-Length");

    endpoint.stop();
}

/// Loopback is not an authorization boundary: without this, any web page can POST
/// here with a non-JSON content type and no preflight.
#[test]
fn refuses_a_foreign_origin_and_a_non_json_body() {
    let mut endpoint = Endpoint::start(0, shared_handler()).expect("bind endpoint");
    let port = endpoint.addr().port();

    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string();
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: x\r\nOrigin: https://evil.example\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).expect("write");
    let (status, _, _) = read_response(&mut stream);
    assert_eq!(status, 403, "expected 403 for a foreign Origin");

    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: x\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).expect("write");
    let (status, _, _) = read_response(&mut stream);
    assert_eq!(status, 415, "expected 415 for a non-JSON content type");

    // A loopback Origin is the legitimate case and must still work.
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: x\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).expect("write");
    let (status, _, reply) = read_response(&mut stream);
    assert_eq!(status, 200, "loopback origin should be served: {reply}");

    endpoint.stop();
}

/// Clients add query strings, and a proxy-configured client sends absolute-form
/// targets. Both are the same endpoint.
#[test]
fn accepts_a_query_string_and_an_absolute_target() {
    let mut endpoint = Endpoint::start(0, shared_handler()).expect("bind endpoint");
    let port = endpoint.addr().port();
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string();

    for target in [
        "/mcp?trace=1".to_string(),
        format!("http://127.0.0.1:{port}/mcp"),
    ] {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        let request = format!(
            "POST {target} HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).expect("write");
        let (status, _, reply) = read_response(&mut stream);
        assert_eq!(status, 200, "{target} should be served: {reply}");
    }

    endpoint.stop();
}

/// stop() runs on Everything's main thread before the host may unload this DLL, so
/// it must return promptly and must not leave a handler thread running.
#[test]
fn stop_with_a_live_connection_returns_promptly_and_frees_the_port() {
    use std::time::{Duration, Instant};

    let mut endpoint = Endpoint::start(0, shared_handler()).expect("bind endpoint");
    let port = endpoint.addr().port();

    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string();
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream.write_all(request.as_bytes()).expect("write");
    let (status, _, _) = read_response(&mut stream);
    assert_eq!(status, 200);
    // The connection is now idle and kept alive: exactly the state a shutdown
    // must be able to get out of.

    let started = Instant::now();
    endpoint.stop();
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "stop() took {elapsed:?} with one idle connection"
    );

    // The port must be free again, or a restart after a host-initiated stop fails.
    let rebound = std::net::TcpListener::bind(("127.0.0.1", port));
    assert!(rebound.is_ok(), "port {port} was not released: {rebound:?}");
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
