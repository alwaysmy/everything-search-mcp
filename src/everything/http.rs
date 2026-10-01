//! Bounded HTTP/1.x transport. One deadline covers DNS, connect, write and read.
//! The stdio dispatcher remains synchronous; this bounds each backend request.
use super::{base64, Config};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

const MAX_BODY: usize = 16 * 1024 * 1024;
const MAX_HEADERS: usize = 64 * 1024;
const MAX_LINE: usize = 8 * 1024;
// OS name resolution is not cancellable. Bound outstanding workers so repeated
// DNS timeouts cannot leave arbitrarily many detached threads behind.
static DNS_WORKERS: AtomicUsize = AtomicUsize::new(0);

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "Everything HTTP request deadline exceeded",
            )
        })
}
fn resolve(cfg: &Config, deadline: Instant) -> io::Result<Vec<SocketAddr>> {
    if let Ok(ip) = cfg.host.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, cfg.port)]);
    }
    DNS_WORKERS
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
            if n < 32 {
                Some(n + 1)
            } else {
                None
            }
        })
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                "too many unfinished DNS lookups; retry later",
            )
        })?;
    let (tx, rx) = mpsc::sync_channel(1);
    let host = cfg.host.clone();
    let port = cfg.port;
    if let Err(e) = std::thread::Builder::new()
        .name("everything-dns".into())
        .spawn(move || {
            let result = (host.as_str(), port)
                .to_socket_addrs()
                .map(|a| a.take(16).collect());
            DNS_WORKERS.fetch_sub(1, Ordering::SeqCst);
            let _ = tx.send(result);
        })
    {
        DNS_WORKERS.fetch_sub(1, Ordering::SeqCst);
        return Err(e);
    }
    rx.recv_timeout(remaining(deadline)?).map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "DNS lookup exceeded request deadline",
        )
    })?
}
struct DeadlineStream {
    stream: TcpStream,
    deadline: Instant,
}
impl Read for DeadlineStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream
            .set_read_timeout(Some(remaining(self.deadline)?))?;
        self.stream.read(buf)
    }
}
impl Write for DeadlineStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream
            .set_write_timeout(Some(remaining(self.deadline)?))?;
        self.stream.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}
fn line(reader: &mut impl BufRead, budget: &mut usize) -> Result<String, String> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_LINE.min(*budget) + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(|e| format!("HTTP read failed: {e}"))?;
    if bytes.len() > MAX_LINE || bytes.len() > *budget {
        return Err("HTTP headers exceed size limit".into());
    }
    *budget -= bytes.len();
    if !bytes.ends_with(b"\r\n") {
        return Err("incomplete or malformed HTTP header/chunk line".into());
    }
    bytes.truncate(bytes.len() - 2);
    String::from_utf8(bytes).map_err(|_| "HTTP header is not valid UTF-8/ASCII".into())
}
fn body(
    reader: &mut impl BufRead,
    length: Option<usize>,
    chunked: bool,
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    if chunked {
        let mut framing_budget = MAX_HEADERS;
        loop {
            let header = line(reader, &mut framing_budget)?;
            let n = usize::from_str_radix(header.split(';').next().unwrap_or("").trim(), 16)
                .map_err(|_| "invalid HTTP chunk size")?;
            if n == 0 {
                while !line(reader, &mut framing_budget)?.is_empty() {}
                break;
            }
            if n > MAX_BODY - out.len() {
                return Err("HTTP body exceeds 16 MiB limit".into());
            }
            let start = out.len();
            out.resize(start + n, 0);
            reader
                .read_exact(&mut out[start..])
                .map_err(|e| format!("incomplete HTTP chunk: {e}"))?;
            let mut crlf = [0; 2];
            reader
                .read_exact(&mut crlf)
                .map_err(|e| format!("incomplete HTTP chunk: {e}"))?;
            if crlf != *b"\r\n" {
                return Err("invalid HTTP chunk terminator".into());
            }
        }
    } else if let Some(n) = length {
        if n > MAX_BODY {
            return Err("HTTP body exceeds 16 MiB limit".into());
        }
        out.resize(n, 0);
        reader
            .read_exact(&mut out)
            .map_err(|e| format!("incomplete HTTP body: {e}"))?;
    } else {
        reader
            .take((MAX_BODY + 1) as u64)
            .read_to_end(&mut out)
            .map_err(|e| format!("HTTP read failed: {e}"))?;
        if out.len() > MAX_BODY {
            return Err("HTTP body exceeds 16 MiB limit".into());
        }
    }
    Ok(out)
}

pub(super) fn get(cfg: &Config, path_and_query: &str) -> Result<String, String> {
    let deadline = Instant::now()
        .checked_add(cfg.timeout)
        .ok_or("request timeout is too large")?;
    let addresses = resolve(cfg, deadline)
        .map_err(|e| format!("Everything HTTP address resolution failed: {e}"))?;
    let mut last = io::Error::new(io::ErrorKind::AddrNotAvailable, "address has no IP results");
    let mut connected = None;
    for address in addresses {
        let timeout = remaining(deadline).map_err(|e| e.to_string())?;
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(s) => {
                connected = Some(s);
                break;
            }
            Err(e) => last = e,
        }
    }
    let addr = cfg.address();
    let stream=connected.ok_or_else(||format!(
        "cannot reach Everything's HTTP server at http://{addr} ({last}). Check reachability and Everything: Tools > Options > HTTP Server. For a local server, bind it to localhost."))?;
    let mut stream = DeadlineStream { stream, deadline };
    let mut req=format!("GET {path_and_query} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nAccept: application/json\r\n");
    if let Some((user, pass)) = &cfg.auth {
        req.push_str(&format!(
            "Authorization: Basic {}\r\n",
            base64(&format!("{user}:{pass}"))
        ));
    }
    req.push_str("\r\n");
    stream
        .write_all(req.as_bytes())
        .map_err(|e| format!("HTTP write failed: {e}"))?;
    let mut reader = BufReader::new(stream);
    let mut header_budget = MAX_HEADERS;
    let status = line(&mut reader, &mut header_budget)?;
    let mut fields = status.split_whitespace();
    let version = fields.next().unwrap_or("");
    let code = fields.next().unwrap_or("");
    if !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        return Err("invalid HTTP status line".into());
    }
    if code != "200" {
        if matches!(code, "401" | "403") {
            return Err(format!("the Everything HTTP server at http://{addr} refused the request ({status}). Check its credentials (http://user:password@host:port)."));
        }
        return Err(format!("Everything HTTP server returned: {status}"));
    }
    let mut length = None;
    let mut chunked = false;
    loop {
        let header = line(&mut reader, &mut header_budget)?;
        if header.is_empty() {
            break;
        }
        let (key, value) = header.split_once(':').ok_or("malformed HTTP header")?;
        if key.eq_ignore_ascii_case("content-length") {
            let n = value
                .trim()
                .parse::<usize>()
                .map_err(|_| "invalid HTTP Content-Length")?;
            if length.is_some() {
                return Err("duplicate HTTP Content-Length".into());
            }
            if n > MAX_BODY {
                return Err("HTTP body exceeds 16 MiB limit".into());
            }
            length = Some(n);
        } else if key.eq_ignore_ascii_case("transfer-encoding") {
            if chunked || !value.trim().eq_ignore_ascii_case("chunked") {
                return Err("unsupported HTTP Transfer-Encoding".into());
            }
            chunked = true;
        }
    }
    if chunked && length.is_some() {
        return Err("ambiguous HTTP length and chunked framing".into());
    }
    let bytes = body(&mut reader, length, chunked)?;
    remaining(deadline).map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|_| "Everything HTTP body is not UTF-8".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    fn request(
        response: impl FnOnce(TcpStream) + Send + 'static,
        timeout: Duration,
    ) -> (Result<String, String>, Duration) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut cfg =
            Config::from_url(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        cfg.timeout = timeout;
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut b = [0u8; 1024];
            let _ = socket.read(&mut b);
            response(socket);
        });
        let start = Instant::now();
        let result = get(&cfg, "/");
        let elapsed = start.elapsed();
        worker.join().unwrap();
        (result, elapsed)
    }
    fn reply(raw: &str) -> Result<String, String> {
        let raw = raw.to_owned();
        request(
            move |mut s| {
                let _ = s.write_all(raw.as_bytes());
            },
            Duration::from_secs(1),
        )
        .0
    }
    #[test]
    fn decodes_chunked_body_and_trailers() {
        assert_eq!(reply("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3;ext=ok\r\nabc\r\n2\r\nde\r\n0\r\nX-Test: yes\r\n\r\n").unwrap(),"abcde");
    }
    #[test]
    fn supports_length_and_eof_framing() {
        assert_eq!(reply("HTTP/1.0 200 OK\r\n\r\nhello").unwrap(), "hello");
        let (result, time) = request(
            |mut s| {
                s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello")
                    .unwrap();
                std::thread::sleep(Duration::from_millis(250));
            },
            Duration::from_millis(100),
        );
        assert_eq!(result.unwrap(), "hello");
        assert!(time < Duration::from_millis(200));
    }
    #[test]
    fn refuses_truncated_or_ambiguous_responses() {
        for raw in [
            "HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nshort",
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nshortXX",
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 0\r\n\r\n0\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: nope\r\n\r\n",
            "HTTP/1.1 2000 Not-OK\r\nContent-Length: 0\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nContent-Length: 1\r\n\r\n",
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\n\r\n",
        ] {
            assert!(reply(raw).is_err(), "accepted {raw:?}");
        }
    }
    #[test]
    fn refuses_oversized_body_before_allocation() {
        assert!(reply("HTTP/1.1 200 OK\r\nContent-Length: 18446744073709551615\r\n\r\n").is_err());
        assert!(
            reply("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1000001\r\n")
                .unwrap_err()
                .contains("16 MiB")
        );
        let (result, _) = request(
            |mut s| {
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\n\r\n");
                let block = [b'x'; 65536];
                for _ in 0..257 {
                    if s.write_all(&block).is_err() {
                        break;
                    }
                }
            },
            Duration::from_secs(2),
        );
        assert!(result.unwrap_err().contains("16 MiB"));
    }
    #[test]
    fn header_line_has_a_hard_size_limit() {
        let raw = format!("HTTP/1.1 200 OK\r\nX: {}\r\n\r\n", "x".repeat(MAX_LINE));
        assert!(reply(&raw).unwrap_err().contains("size limit"));
    }
    #[test]
    fn total_deadline_stops_a_peer_that_keeps_dripping_bytes() {
        let (result, time) = request(
            |mut s| {
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\n\r\n");
                for _ in 0..40 {
                    if s.write_all(b" ").is_err() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(15));
                }
            },
            Duration::from_millis(60),
        );
        assert!(result.is_err());
        assert!(
            time < Duration::from_millis(400),
            "deadline elapsed {time:?}"
        );
    }
    #[test]
    fn hostname_resolution_and_ipv6_address_format() {
        let mut cfg = Config::from_url("http://localhost:23333").unwrap();
        assert!(!resolve(&cfg, Instant::now() + Duration::from_secs(1))
            .unwrap()
            .is_empty());
        cfg.host = "::1".into();
        assert_eq!(cfg.address(), "[::1]:23333");
        assert!(remaining(Instant::now() - Duration::from_millis(1)).is_err());
    }
}
