//! The MCP endpoint served from inside Everything: a loopback HTTP listener that
//! speaks JSON-RPC (MCP) and forwards every tool call to the crate's normal HTTP
//! backend.
//!
//! Transport shape follows MCP's Streamable HTTP: a client POSTs one JSON-RPC
//! message (or a batch array) and gets the reply back. `Accept` decides the body
//! format — `application/json` when the client accepts it, otherwise a one-shot
//! `text/event-stream` frame. A stateless server is legitimate for a tool-only MCP
//! server: no session id is issued and no server-initiated stream is opened, so
//! `GET` is `405`, exactly as the spec allows for servers that offer no SSE stream.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{json, Value};

use crate::jsonrpc::{Handler, Server};

/// Builds one MCP server per request.
///
/// The tool layer holds no state between calls (a `Tools` is a `Config` and
/// nothing else), so a shared `Server` behind a mutex would buy nothing and cost
/// head-of-line blocking: one slow `count_stats` would hold up every request that
/// arrived beside it. A factory keeps concurrent calls independent, which is what
/// Everything's own HTTP server already does one layer down.
///
/// Generic over the handler so `tests/plugin_http.rs` can prove that independence
/// with a handler it controls, instead of inferring it from tool timings.
pub type HandlerFactory<H> = Arc<dyn Fn() -> Server<H> + Send + Sync>;

const MAX_HEAD: usize = 64 * 1024;
const MAX_BODY: usize = 4 * 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a connection may sit idle between requests before it is closed.
///
/// Keep-alive means an idle connection pins its thread, and this code runs inside
/// `Everything.exe`. Five seconds is long enough to cover the gap between tool
/// calls in one agent turn and short enough that a forgotten connection does not
/// hold anything for long.
const KEEP_ALIVE_IDLE: Duration = Duration::from_secs(5);

/// Connections served at once, idle ones included.
///
/// Thread-per-connection is fine for a tool endpoint only while the count is
/// bounded: 100 idle sockets were measured holding 100 threads inside the host.
/// Past this limit new connections are refused with `503` rather than queued.
///
/// The limit costs nothing when idle — it is a counter, not an allocation — so it
/// is set well above what a normal session needs (measured: 64 connections added
/// 1 MB RSS and are gone again 5 s later). `EVERYTHING_MCP_MAX_CONNECTIONS`
/// overrides it for a machine running an unusual number of agents.
const MAX_CONNECTIONS: usize = 256;

/// `EVERYTHING_MCP_MAX_CONNECTIONS`, or [`MAX_CONNECTIONS`].
fn configured_max_connections() -> usize {
    std::env::var("EVERYTHING_MCP_MAX_CONNECTIONS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|count| *count > 0)
        .unwrap_or(MAX_CONNECTIONS)
}

/// Admission limits for one endpoint.
///
/// Kept per endpoint rather than global so a test can run with two slots and a
/// 200 ms idle window without changing what any other endpoint does.
struct Limits {
    open: AtomicUsize,
    max_connections: usize,
    idle: Duration,
}

impl Limits {
    fn new(max_connections: usize, idle: Duration) -> Arc<Self> {
        Arc::new(Self {
            open: AtomicUsize::new(0),
            max_connections,
            idle,
        })
    }
}

/// Holds one connection slot until that connection's thread ends.
struct ConnSlot(Arc<Limits>);

impl Drop for ConnSlot {
    fn drop(&mut self) {
        self.0.open.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A running endpoint. Dropping it stops the listener thread.
pub struct Endpoint<H: Handler + Send + 'static> {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// The handler type is only carried so callers stay typed end to end; the
    /// listener thread owns the factory and never hands it back.
    handler: std::marker::PhantomData<H>,
}

impl<H: Handler + Send + 'static> Endpoint<H> {
    /// Bind `127.0.0.1:port` and start accepting. Port `0` asks the OS for a free
    /// port; the chosen one is available from [`Endpoint::addr`].
    pub fn start(port: u16, handler: HandlerFactory<H>) -> std::io::Result<Self> {
        Self::start_with(port, handler, configured_max_connections(), KEEP_ALIVE_IDLE)
    }

    /// [`Endpoint::start`] with the admission limits spelled out. Tests use this
    /// to exercise the caps without opening 64 sockets or waiting five seconds.
    pub fn start_with(
        port: u16,
        handler: HandlerFactory<H>,
        max_connections: usize,
        idle: Duration,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_in_thread = Arc::clone(&stop);
        let limits = Limits::new(max_connections, idle);
        let thread = std::thread::Builder::new()
            .name("everything-mcp-http".into())
            .spawn(move || accept_loop(listener, stop_in_thread, handler, limits))?;
        Ok(Self {
            addr,
            stop,
            thread: Some(thread),
            handler: std::marker::PhantomData,
        })
    }

    /// The address actually bound.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Stop accepting and wait for the listener thread to finish.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the blocking accept() so the loop can observe the flag.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(500));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl<H: Handler + Send + 'static> Drop for Endpoint<H> {
    fn drop(&mut self) {
        self.stop();
    }
}

fn accept_loop<H: Handler + Send + 'static>(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    handler: HandlerFactory<H>,
    limits: Arc<Limits>,
) {
    loop {
        match listener.accept() {
            Ok((mut stream, _peer)) => {
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                // Take a slot before spawning: the thread is the thing being
                // bounded, and a refused connection costs one write and no thread.
                if limits.open.fetch_add(1, Ordering::SeqCst) >= limits.max_connections {
                    limits.open.fetch_sub(1, Ordering::SeqCst);
                    let _ = write_plain(
                        &mut stream,
                        503,
                        "Service Unavailable",
                        "too many connections",
                        false,
                    );
                    continue;
                }
                let slot = ConnSlot(Arc::clone(&limits));
                let idle = limits.idle;
                let handler = Arc::clone(&handler);
                // One short-lived thread per request: the backend call is what
                // takes time, and it already runs in parallel.
                let _ = std::thread::Builder::new()
                    .name("everything-mcp-conn".into())
                    .spawn(move || {
                        // Dropped with the thread, releasing the connection slot.
                        let _slot = slot;
                        let _ = handle_connection(stream, handler, idle);
                    });
            }
            Err(_) => {
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                // Transient accept failure (e.g. ECONNABORTED): keep serving.
                std::thread::yield_now();
            }
        }
    }
}

struct Request {
    method: String,
    path: String,
    accept: String,
    keep_alive: bool,
    /// `Transfer-Encoding: chunked`. Not implemented — refused with 411 rather
    /// than mis-read as an empty body, which is what a Content-Length-only parser
    /// would do.
    chunked: bool,
    body: Vec<u8>,
}

/// One client connection.
///
/// `buf` outlives individual requests on purpose: with keep-alive a single read
/// can already contain the next request's bytes, and throwing them away would drop
/// a request the client believes it sent.
struct Conn {
    stream: TcpStream,
    buf: Vec<u8>,
}

impl Conn {
    fn new(stream: TcpStream) -> Self {
        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
        Self {
            stream,
            buf: Vec::with_capacity(4096),
        }
    }

    fn set_read_timeout(&mut self, timeout: Duration) {
        let _ = self.stream.set_read_timeout(Some(timeout));
    }

    fn read_request(&mut self) -> std::io::Result<Request> {
        let mut chunk = [0u8; 4096];
        let head_end = loop {
            if let Some(pos) = find_subslice(&self.buf, b"\r\n\r\n") {
                break pos;
            }
            if self.buf.len() > MAX_HEAD {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "request head too large",
                ));
            }
            let n = self.stream.read(&mut chunk)?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "eof in request head",
                ));
            }
            self.buf.extend_from_slice(&chunk[..n]);
        };

        let head = String::from_utf8_lossy(&self.buf[..head_end]).to_string();
        let mut lines = head.split("\r\n");
        let request_line = lines.next().unwrap_or_default();
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_ascii_uppercase();
        let path = parts.next().unwrap_or("/").to_string();
        let version = parts.next().unwrap_or("HTTP/1.1").to_string();

        let mut content_length = 0usize;
        let mut accept = String::new();
        let mut connection = String::new();
        let mut transfer_encoding = String::new();
        let mut expect = String::new();
        for line in lines {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim();
            match name.as_str() {
                "content-length" => content_length = value.parse().unwrap_or(0),
                "accept" => accept = value.to_ascii_lowercase(),
                "connection" => connection = value.to_ascii_lowercase(),
                "transfer-encoding" => transfer_encoding = value.to_ascii_lowercase(),
                "expect" => expect = value.to_ascii_lowercase(),
                _ => {}
            }
        }
        let chunked = transfer_encoding.contains("chunked");

        // A client that asked for `100 Continue` is waiting for this line before
        // it sends the body. Without it the server would sit in read() until the
        // client gives up, which looks like a hang rather than a protocol error.
        if expect.contains("100-continue") && content_length > 0 && !chunked {
            self.stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
            self.stream.flush()?;
        }
        if content_length > MAX_BODY {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "request body too large",
            ));
        }

        let body_start = head_end + 4;
        while self.buf.len() < body_start + content_length {
            let n = self.stream.read(&mut chunk)?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "eof in request body",
                ));
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
        let body = self.buf[body_start..body_start + content_length].to_vec();
        // Whatever follows this body belongs to the next request on this socket.
        self.buf.drain(..body_start + content_length);

        // HTTP/1.1 reuses the connection unless told otherwise; 1.0 is the reverse.
        let keep_alive = if connection.contains("close") {
            false
        } else if connection.contains("keep-alive") {
            true
        } else {
            version != "HTTP/1.0"
        };

        Ok(Request {
            method,
            path,
            accept,
            keep_alive,
            chunked,
            body,
        })
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn handle_connection<H: Handler + Send + 'static>(
    stream: TcpStream,
    handler: HandlerFactory<H>,
    idle: Duration,
) -> std::io::Result<()> {
    let mut conn = Conn::new(stream);

    // Serve requests on this connection for as long as the client wants it. A
    // loopback handshake costs ~0.17 ms, which was the single largest piece of
    // this endpoint's overhead, and every MCP client reuses connections when the
    // server lets it.
    loop {
        let request = match conn.read_request() {
            Ok(request) => request,
            // EOF or an idle timeout: the normal way a connection ends. There is
            // nobody left to answer, so nothing is reported.
            Err(_) => return Ok(()),
        };
        if !handle_one(&mut conn, request, &handler)? {
            return Ok(());
        }
        // The first request gets the generous timeout (a client that connected
        // but is slow to speak); once served, the connection is only worth
        // holding while the client is actively using it.
        conn.set_read_timeout(idle);
    }
}

/// Answer one request. Returns whether the connection may be reused.
fn handle_one<H: Handler + Send + 'static>(
    conn: &mut Conn,
    request: Request,
    handler: &HandlerFactory<H>,
) -> std::io::Result<bool> {
    let stream = &mut conn.stream;

    // Refuse a framing this server cannot read, instead of treating the body as
    // empty and answering "empty body" three layers later.
    if request.chunked {
        let _ = write_plain(
            stream,
            411,
            "Length Required",
            "chunked request bodies are not supported; send Content-Length",
            false,
        );
        return Ok(false);
    }

    // MCP endpoint sits at "/" and "/mcp"; everything else is a 404.
    if request.path != "/" && request.path != "/mcp" {
        let _ = write_plain(stream, 404, "Not Found", "not found", false);
        return Ok(false);
    }

    match request.method.as_str() {
        "POST" => {}
        // No server-initiated stream and no session state: nothing to GET/DELETE.
        _ => {
            let mut extra = String::from("Allow: POST\r\n");
            if request.method == "GET" || request.method == "DELETE" {
                extra.push_str("Mcp-Allow: POST\r\n");
            }
            let _ = write_response(
                stream,
                405,
                "Method Not Allowed",
                "text/plain",
                &extra,
                b"",
                false,
            );
            return Ok(false);
        }
    }

    let body = match std::str::from_utf8(&request.body) {
        Ok(body) => body.trim(),
        Err(_) => {
            let _ = write_plain(stream, 400, "Bad Request", "body is not UTF-8", false);
            return Ok(false);
        }
    };
    if body.is_empty() {
        let _ = write_plain(stream, 400, "Bad Request", "empty body", false);
        return Ok(false);
    }

    let parsed: Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(_) => {
            let error = json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": {"code": -32700, "message": "parse error"},
            });
            let text = error.to_string();
            let _ = write_rpc(stream, &request.accept, &text, false);
            return Ok(false);
        }
    };

    // One JSON-RPC message, or a batch of them.
    let replies: Vec<Value> = {
        // A fresh server for this request: nothing is shared, so a slow tool call
        // cannot delay the requests arriving on other connections.
        let mut server = handler();
        match parsed {
            Value::Array(messages) => messages
                .iter()
                .filter_map(|message| server.handle_line(&message.to_string()))
                .collect(),
            message => server
                .handle_line(&message.to_string())
                .into_iter()
                .collect(),
        }
    };

    if replies.is_empty() {
        // Notification only: accepted, nothing to say back.
        let _ = write_response(
            stream,
            202,
            "Accepted",
            "text/plain",
            "",
            b"",
            request.keep_alive,
        );
        return Ok(request.keep_alive);
    }

    let payload = match replies.len() {
        1 => replies.into_iter().next().unwrap_or(Value::Null),
        _ => Value::Array(replies),
    };
    write_rpc(
        stream,
        &request.accept,
        &payload.to_string(),
        request.keep_alive,
    )?;
    Ok(request.keep_alive)
}

fn write_rpc(
    stream: &mut TcpStream,
    accept: &str,
    json_text: &str,
    keep_alive: bool,
) -> std::io::Result<()> {
    // Prefer plain JSON when the client accepts it; otherwise emit a one-shot SSE
    // frame, which is the shape a streamable-HTTP client expects when it insists
    // on an event stream.
    let wants_sse = accept.contains("text/event-stream") && !accept.contains("application/json");
    if wants_sse {
        let body = format!("event: message\ndata: {json_text}\n\n");
        write_response(
            stream,
            200,
            "OK",
            "text/event-stream; charset=utf-8",
            "Cache-Control: no-cache\r\n",
            body.as_bytes(),
            keep_alive,
        )
    } else {
        write_response(
            stream,
            200,
            "OK",
            "application/json; charset=utf-8",
            "",
            json_text.as_bytes(),
            keep_alive,
        )
    }
}

fn write_plain(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    text: &str,
    keep_alive: bool,
) -> std::io::Result<()> {
    write_response(
        stream,
        status,
        reason,
        "text/plain; charset=utf-8",
        "",
        text.as_bytes(),
        keep_alive,
    )
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    extra_headers: &str,
    body: &[u8],
    keep_alive: bool,
) -> std::io::Result<()> {
    let connection = if keep_alive { "keep-alive" } else { "close" };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n{extra_headers}Connection: {connection}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}
