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
    assert_eq!(init["result"]["serverInfo"]["name"], "everything_search_mcp");

    let listed = rpc(port, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    let tools = listed["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("no tools in {listed}"));
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    for expected in ["everything_search", "everything_count_stats", "everything_file_details"] {
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

