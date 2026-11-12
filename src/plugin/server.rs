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

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
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
/// Request body cap. MCP `tools/call` payloads are a few hundred bytes; 4 MiB
/// only created a memory ceiling of `MAX_BODY × MAX_CONNECTIONS`.
const MAX_BODY: usize = 256 * 1024;
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
    /// Live connection sockets, so [`Endpoint::stop`] can force every handler
    /// thread out of its blocking read instead of abandoning it inside a DLL that
    /// the host is about to unload.
    sockets: Mutex<HashMap<u64, TcpStream>>,
    next_id: AtomicU64,
}

impl Limits {
    fn new(max_connections: usize, idle: Duration) -> Arc<Self> {
        Arc::new(Self {
            open: AtomicUsize::new(0),
            max_connections,
            idle,
            sockets: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(0),
        })
    }
}

/// One connection's slot and registry entry, released when its thread ends.
struct ConnSlot {
    limits: Arc<Limits>,
    id: u64,
}

impl Drop for ConnSlot {
    fn drop(&mut self) {
        self.limits
            .sockets
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(&self.id);
        self.limits.open.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A running endpoint. Dropping it stops the listener thread.
pub struct Endpoint<H: Handler + Send + 'static> {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    limits: Arc<Limits>,
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
        // Non-blocking accept: the loop polls the stop flag, so shutdown never
        // depends on a self-connect wake-up succeeding.
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_in_thread = Arc::clone(&stop);
        let limits = Limits::new(max_connections, idle);
        let limits_in_thread = Arc::clone(&limits);
        let thread = std::thread::Builder::new()
            .name("everything-mcp-http".into())
            .spawn(move || accept_loop(listener, stop_in_thread, handler, limits_in_thread))?;
        Ok(Self {
            addr,
            stop,
            thread: Some(thread),
            limits,
            handler: std::marker::PhantomData,
        })
    }

    /// The address actually bound.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Stop accepting, evict every live connection, and wait for the threads.
    ///
    /// This runs on Everything's main thread just before the host may unload the
    /// DLL, so it must not return while a handler thread is still executing code
    /// from this module. Connections are shut down by socket (their blocking read
    /// then fails and the thread exits), then the accept thread is joined, then
    /// the slot count is awaited.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);

        // Clone-free shutdown: each handler owns a dup of its socket.
        let sockets = self
            .limits
            .sockets
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        for socket in sockets.values() {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
        drop(sockets);

        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }

        // Handlers wake from the shutdown within milliseconds; the bound is a
        // backstop, not the expected path.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while self.limits.open.load(Ordering::SeqCst) > 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
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
    // A non-blocking listener turns accept() into a poll, which is what lets
    // shutdown be a flag check instead of a wake-up packet that may never arrive.
    let mut consecutive_errors = 0u32;
    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        match listener.accept() {
            Ok((mut stream, _peer)) => {
                consecutive_errors = 0;
                // The listener polls, but each connection must block on read:
                // inherited non-blocking would turn every request into WouldBlock.
                let _ = stream.set_nonblocking(false);
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
                let id = limits.next_id.fetch_add(1, Ordering::SeqCst);
                // The handler keeps its own handle; the registry holds a dup so
                // stop() can shut the connection down from the main thread.
                if let Ok(control) = stream.try_clone() {
                    limits
                        .sockets
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .insert(id, control);
                }
                let slot = ConnSlot {
                    limits: Arc::clone(&limits),
                    id,
                };
                let idle = limits.idle;
                let handler = Arc::clone(&handler);
                let stop_for_thread = Arc::clone(&stop);
                // One short-lived thread per request: the backend call is what
                // takes time, and it already runs in parallel.
                let _ = std::thread::Builder::new()
                    .name("everything-mcp-conn".into())
                    // A blocking read needs no 2 MB of stack.
                    .stack_size(256 * 1024)
                    .spawn(move || {
                        // Dropped with the thread, releasing the slot and the
                        // registry entry.
                        let _slot = slot;
                        let _ = handle_connection(stream, handler, idle, stop_for_thread);
                    });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                // Handle exhaustion is persistent, not transient: backing off
                // keeps a spinning thread out of the host's process.
                consecutive_errors += 1;
                if consecutive_errors > 50 {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// Why a request could not be read.
///
/// The distinction matters to the client: closing silently is right when the peer
/// went away, and wrong when the request was invalid — that shows up as "tool call
/// hung / connection reset" with nothing to diagnose.
enum ReadFault {
    /// Peer closed, timed out, or the socket was shut down by [`Endpoint::stop`].
    Closed,
    /// Protocol error worth a status line before the socket closes.
    Protocol(u16, &'static str, &'static str),
}

impl From<std::io::Error> for ReadFault {
    fn from(_: std::io::Error) -> Self {
        ReadFault::Closed
    }
}

struct Request {
    method: String,
    path: String,
    accept: String,
    origin: String,
    content_type: String,
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

/// How often a blocking read wakes up to check whether the endpoint is stopping.
/// A recv() cannot be cancelled from another thread on Windows, so shutdown has to
/// be observed by polling rather than assumed.
const STOP_POLL: Duration = Duration::from_millis(200);

impl Conn {
    fn new(stream: TcpStream) -> Self {
        let _ = stream.set_read_timeout(Some(STOP_POLL));
        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
        // Two writes per response; Nagle would add tens of milliseconds on some
        // stacks, which is the same order as the whole endpoint budget.
        let _ = stream.set_nodelay(true);
        Self {
            stream,
            buf: Vec::with_capacity(4096),
        }
    }

    /// One read that wakes at least every [`STOP_POLL`] and honours both the
    /// endpoint's stop flag and the request's own deadline.
    fn read_chunk(
        &mut self,
        chunk: &mut [u8],
        stop: &AtomicBool,
        deadline: std::time::Instant,
    ) -> Result<usize, ReadFault> {
        loop {
            match self.stream.read(chunk) {
                Ok(0) => return Err(ReadFault::Closed),
                Ok(n) => return Ok(n),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    if stop.load(Ordering::SeqCst) || std::time::Instant::now() >= deadline {
                        return Err(ReadFault::Closed);
                    }
                }
                Err(_) => return Err(ReadFault::Closed),
            }
        }
    }

    fn read_request(&mut self, stop: &AtomicBool, timeout: Duration) -> Result<Request, ReadFault> {
        let deadline = std::time::Instant::now() + timeout;
        let mut chunk = [0u8; 4096];
        let head_end = loop {
            if let Some(pos) = find_subslice(&self.buf, b"\r\n\r\n") {
                break pos;
            }
            if self.buf.len() > MAX_HEAD {
                return Err(ReadFault::Protocol(
                    431,
                    "Request Header Fields Too Large",
                    "request head too large",
                ));
            }
            let n = self.read_chunk(&mut chunk, stop, deadline)?;
            if n == 0 {
                return Err(ReadFault::Closed);
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
        let mut origin = String::new();
        let mut content_type = String::new();
        for line in lines {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim();
            match name.as_str() {
                "content-length" => match value.parse::<usize>() {
                    Ok(length) => content_length = length,
                    // A number too large for usize, or not a number at all: a
                    // Content-Length the parser cannot honour is a protocol error,
                    // never a zero-length body.
                    Err(_) => {
                        return Err(ReadFault::Protocol(
                            400,
                            "Bad Request",
                            "bad Content-Length",
                        ))
                    }
                },
                "accept" => accept = value.to_ascii_lowercase(),
                "connection" => connection = value.to_ascii_lowercase(),
                "transfer-encoding" => transfer_encoding = value.to_ascii_lowercase(),
                "expect" => expect = value.to_ascii_lowercase(),
                "origin" => origin = value.to_ascii_lowercase(),
                "content-type" => content_type = value.to_ascii_lowercase(),
                _ => {}
            }
        }
        let chunked = transfer_encoding.contains("chunked");

        // Refuse an oversized body *before* inviting it: answering 100 Continue
        // and then closing makes the client push megabytes into a dead socket.
        if content_length > MAX_BODY {
            return Err(ReadFault::Protocol(
                413,
                "Payload Too Large",
                "request body exceeds the limit",
            ));
        }

        // A client that asked for `100 Continue` is waiting for this line before
        // it sends the body. Without it the server would sit in read() until the
        // client gives up, which looks like a hang rather than a protocol error.
        if expect.contains("100-continue") && content_length > 0 && !chunked {
            self.stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
            self.stream.flush()?;
        }

        let body_start = head_end + 4;
        while self.buf.len() < body_start + content_length {
            let n = self.read_chunk(&mut chunk, stop, deadline)?;
            if n == 0 {
                return Err(ReadFault::Closed);
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
            origin,
            content_type,
            keep_alive,
            chunked,
            body,
        })
    }
}

/// Whether an `Origin` header names this machine.
///
/// The endpoint has no authentication; the only thing that keeps a random web page
/// from driving it is refusing origins it cannot legitimately have.
fn is_loopback_origin(origin: &str) -> bool {
    let rest = match origin.strip_prefix("http://") {
        Some(rest) => rest,
        None => match origin.strip_prefix("https://") {
            Some(rest) => rest,
            None => return false,
        },
    };
    let host = rest.split(['/', ':']).next().unwrap_or("");
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
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
    stop: Arc<AtomicBool>,
) -> std::io::Result<()> {
    let mut conn = Conn::new(stream);

    // Serve requests on this connection for as long as the client wants it. A
    // loopback handshake costs ~0.17 ms, which was the single largest piece of
    // this endpoint's overhead, and every MCP client reuses connections when the
    // server lets it.
    let mut first = true;
    loop {
        // The first request gets the generous timeout (a client may connect and be
        // slow to speak); once served, the connection is only worth holding while
        // the client is actively using it.
        let timeout = if first { IO_TIMEOUT } else { idle };
        first = false;
        let request = match conn.read_request(&stop, timeout) {
            Ok(request) => request,
            // EOF, an idle timeout, or the socket being shut down by stop(): the
            // normal ways a connection ends. There is nobody left to answer.
            Err(ReadFault::Closed) => return Ok(()),
            Err(ReadFault::Protocol(status, reason, message)) => {
                let _ = write_plain(&mut conn.stream, status, reason, message, false);
                return Ok(());
            }
        };
        if !handle_one(&mut conn, request, &handler)? {
            return Ok(());
        }
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

    // A browser can POST here without a preflight when the content type is not
    // JSON. Loopback is not an authorization boundary on a shared machine, so
    // anything with a foreign Origin is refused, and a non-JSON body is refused
    // before it reaches the tool layer.
    if !request.origin.is_empty() && !is_loopback_origin(&request.origin) {
        let _ = write_plain(stream, 403, "Forbidden", "origin not allowed", false);
        return Ok(false);
    }
    if !request.content_type.is_empty() && !request.content_type.contains("application/json") {
        let _ = write_plain(
            stream,
            415,
            "Unsupported Media Type",
            "expected application/json",
            false,
        );
        return Ok(false);
    }

    // Match on the path alone: clients add query strings (inspectors, tracing) and
    // some send absolute-form targets when an HTTP proxy is configured.
    let path = request
        .path
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/');
    let path = if path.is_empty() { "/" } else { path };
    let path = path
        .strip_prefix("http://")
        .or_else(|| path.strip_prefix("https://"))
        .and_then(|rest| rest.find('/').map(|at| &rest[at..]))
        .unwrap_or(path);

    // MCP endpoint sits at "/" and "/mcp"; everything else is a 404.
    if path != "/" && path != "/mcp" {
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
