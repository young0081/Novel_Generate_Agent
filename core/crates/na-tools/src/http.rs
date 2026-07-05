//! A minimal, dependency-free HTTP/1.1 [`Fetcher`] over raw `tokio::net`.
//!
//! [`HttpFetcher`] implements the [`Fetcher`](crate::Fetcher) trait for plain
//! `http://` URLs using a raw [`tokio::net::TcpStream`] — no `reqwest`, no
//! `hyper`. It performs the smallest correct GET that real servers accept:
//!
//! 1. Parse the URL into scheme / host / port / path.
//! 2. Open a TCP connection and write a `GET <path> HTTP/1.1` request with the
//!    mandatory `Host` header and `Connection: close` (so the server signals EOF
//!    and we can read until the socket closes).
//! 3. Read the whole response, split the status line + headers from the body.
//! 4. Decode the body honoring **both** `Content-Length` and
//!    `Transfer-Encoding: chunked`.
//!
//! The whole exchange runs under a [`tokio::time::timeout`] derived from
//! `timeout_ms`. `https://` URLs are rejected with
//! [`CoreError::invalid_input`] — TLS is intentionally out of scope here and is
//! meant to be supplied by a separate TLS-capable fetcher in a later phase.
//!
//! Status codes `>= 400` are surfaced as a [`CoreError`] (`tool` for client/4xx,
//! `model` for server/5xx) so the caller sees an error rather than an HTML error
//! page treated as content.

use std::time::Duration;

use na_common::{CoreError, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::tool::{BoxFuture, Fetcher};

/// Hard ceiling on how many bytes of response body we will accumulate, to avoid
/// an unbounded server response exhausting memory. 8 MiB is plenty for prose.
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// A real HTTP/1.1 fetcher over raw TCP (plaintext `http://` only).
#[derive(Debug, Clone, Copy)]
pub struct HttpFetcher {
    /// Total timeout for the connect + request + read, in milliseconds.
    pub timeout_ms: u64,
}

impl Default for HttpFetcher {
    /// A 30-second timeout.
    fn default() -> Self {
        HttpFetcher { timeout_ms: 30_000 }
    }
}

impl HttpFetcher {
    /// Construct with an explicit total timeout in milliseconds.
    pub fn new(timeout_ms: u64) -> Self {
        HttpFetcher { timeout_ms }
    }

    /// The fetch implementation, separated so it can be wrapped in a timeout.
    async fn fetch_inner(&self, url: &str) -> Result<String> {
        let parts = ParsedUrl::parse(url)?;

        let addr = format!("{}:{}", parts.host, parts.port);
        let mut stream = TcpStream::connect(&addr)
            .await
            .map_err(|e| CoreError::from(e).with_context(format!("connecting to {addr}")))?;

        // Build and send the request. `Connection: close` lets us read to EOF.
        let request = format!(
            "GET {path} HTTP/1.1\r\n\
             Host: {host}\r\n\
             User-Agent: na-tools/0.1\r\n\
             Accept: */*\r\n\
             Connection: close\r\n\
             \r\n",
            path = parts.path,
            host = parts.host_header(),
        );
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|e| CoreError::from(e).with_context("writing HTTP request"))?;
        stream
            .flush()
            .await
            .map_err(|e| CoreError::from(e).with_context("flushing HTTP request"))?;

        // Read the entire response (headers + body) until the server closes.
        let mut raw: Vec<u8> = Vec::with_capacity(8 * 1024);
        let mut buf = [0u8; 8 * 1024];
        loop {
            let n = stream
                .read(&mut buf)
                .await
                .map_err(|e| CoreError::from(e).with_context("reading HTTP response"))?;
            if n == 0 {
                break;
            }
            raw.extend_from_slice(&buf[..n]);
            if raw.len() > MAX_BODY_BYTES {
                return Err(CoreError::tool(format!(
                    "HTTP response exceeded {MAX_BODY_BYTES} bytes"
                )));
            }
        }

        let response = HttpResponse::parse(&raw)?;
        if response.status >= 400 {
            let msg = format!("HTTP {} {} for {url}", response.status, response.reason);
            return Err(if response.status >= 500 {
                CoreError::model(msg)
            } else {
                CoreError::tool(msg)
            });
        }

        Ok(String::from_utf8_lossy(&response.body).into_owned())
    }
}

impl Fetcher for HttpFetcher {
    fn fetch<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            let timeout = Duration::from_millis(self.timeout_ms.max(1));
            match tokio::time::timeout(timeout, self.fetch_inner(url)).await {
                Ok(inner) => inner,
                Err(_elapsed) => Err(CoreError::timeout(format!(
                    "HTTP fetch of {url} exceeded {} ms",
                    self.timeout_ms
                ))),
            }
        })
    }
}

/// A parsed `http://` URL broken into its addressing parts.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedUrl {
    host: String,
    port: u16,
    /// Path + optional query, always starting with `/`.
    path: String,
    /// Whether the default port was used (so the Host header can omit it).
    default_port: bool,
}

impl ParsedUrl {
    fn parse(url: &str) -> Result<Self> {
        let rest = if let Some(r) = url.strip_prefix("http://") {
            r
        } else if url.starts_with("https://") {
            return Err(CoreError::invalid_input(
                "https not supported without TLS; plug a TLS fetcher in a later phase",
            ));
        } else {
            return Err(CoreError::invalid_input(format!(
                "unsupported URL scheme (expected http://): {url}"
            )));
        };

        // Split authority from path: the first '/' after the authority.
        let (authority, path) = match rest.find('/') {
            Some(idx) => (&rest[..idx], &rest[idx..]),
            None => (rest, "/"),
        };
        if authority.is_empty() {
            return Err(CoreError::invalid_input(format!("URL has no host: {url}")));
        }

        // Strip optional userinfo (`user:pass@host`) — we ignore credentials.
        let host_port = match authority.rsplit_once('@') {
            Some((_userinfo, hp)) => hp,
            None => authority,
        };

        // Split host:port. IPv6 literals are bracketed: `[::1]:8080`.
        let (host, port, default_port) = if let Some(stripped) = host_port.strip_prefix('[') {
            // IPv6 literal.
            let end = stripped
                .find(']')
                .ok_or_else(|| CoreError::invalid_input(format!("malformed IPv6 host in {url}")))?;
            let host = &stripped[..end];
            let after = &stripped[end + 1..];
            let port = after.strip_prefix(':');
            match port {
                Some(p) => (host.to_string(), parse_port(p, url)?, false),
                None => (host.to_string(), 80, true),
            }
        } else {
            match host_port.rsplit_once(':') {
                Some((h, p)) => (h.to_string(), parse_port(p, url)?, false),
                None => (host_port.to_string(), 80, true),
            }
        };

        if host.is_empty() {
            return Err(CoreError::invalid_input(format!(
                "URL has empty host: {url}"
            )));
        }

        let path = if path.is_empty() {
            "/".to_string()
        } else {
            path.to_string()
        };

        Ok(ParsedUrl {
            host,
            port,
            path,
            default_port,
        })
    }

    /// The value for the `Host:` header (omits the port when it is the default).
    fn host_header(&self) -> String {
        if self.default_port || self.port == 80 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

fn parse_port(s: &str, url: &str) -> Result<u16> {
    s.parse::<u16>()
        .map_err(|_| CoreError::invalid_input(format!("invalid port {s:?} in {url}")))
}

/// A parsed HTTP response: status, reason phrase, and decoded body bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HttpResponse {
    status: u16,
    reason: String,
    body: Vec<u8>,
}

impl HttpResponse {
    fn parse(raw: &[u8]) -> Result<Self> {
        // Find the header/body boundary (\r\n\r\n).
        let split = find_subslice(raw, b"\r\n\r\n")
            .ok_or_else(|| CoreError::protocol("malformed HTTP response: no header terminator"))?;
        let header_bytes = &raw[..split];
        let body_start = split + 4;
        let body_raw = &raw[body_start..];

        let header_text = String::from_utf8_lossy(header_bytes);
        let mut lines = header_text.split("\r\n");

        let status_line = lines
            .next()
            .ok_or_else(|| CoreError::protocol("empty HTTP response"))?;
        let (status, reason) = parse_status_line(status_line)?;

        // Collect the headers we care about.
        let mut content_length: Option<usize> = None;
        let mut chunked = false;
        for line in lines {
            if line.is_empty() {
                continue;
            }
            if let Some((name, value)) = line.split_once(':') {
                let name = name.trim().to_ascii_lowercase();
                let value = value.trim();
                match name.as_str() {
                    "content-length" => {
                        content_length = value.parse::<usize>().ok();
                    }
                    "transfer-encoding" => {
                        if value.to_ascii_lowercase().contains("chunked") {
                            chunked = true;
                        }
                    }
                    _ => {}
                }
            }
        }

        let body = if chunked {
            decode_chunked(body_raw)?
        } else if let Some(len) = content_length {
            let end = len.min(body_raw.len());
            body_raw[..end].to_vec()
        } else {
            // No framing headers: take everything we read (Connection: close).
            body_raw.to_vec()
        };

        Ok(HttpResponse {
            status,
            reason,
            body,
        })
    }
}

/// Parse `HTTP/1.1 200 OK` into `(200, "OK")`.
fn parse_status_line(line: &str) -> Result<(u16, String)> {
    let mut parts = line.splitn(3, ' ');
    let _version = parts
        .next()
        .filter(|v| v.starts_with("HTTP/"))
        .ok_or_else(|| CoreError::protocol(format!("bad HTTP status line: {line:?}")))?;
    let code = parts
        .next()
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or_else(|| CoreError::protocol(format!("bad HTTP status code in: {line:?}")))?;
    let reason = parts.next().unwrap_or("").trim().to_string();
    Ok((code, reason))
}

/// Decode a `Transfer-Encoding: chunked` body into the concatenated payload.
fn decode_chunked(mut data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        // Read the chunk-size line (hex), up to CRLF.
        let line_end = find_subslice(data, b"\r\n")
            .ok_or_else(|| CoreError::protocol("chunked body: missing size CRLF"))?;
        let size_line = std::str::from_utf8(&data[..line_end])
            .map_err(|_| CoreError::protocol("chunked body: non-UTF8 size line"))?;
        // A chunk-size may be followed by ';' chunk-extensions; ignore them.
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16)
            .map_err(|_| CoreError::protocol(format!("chunked body: bad size {size_hex:?}")))?;

        data = &data[line_end + 2..]; // skip "<size>\r\n"

        if size == 0 {
            // Last chunk; the remainder is optional trailers we ignore.
            break;
        }
        if data.len() < size {
            return Err(CoreError::protocol("chunked body: truncated chunk data"));
        }
        out.extend_from_slice(&data[..size]);
        data = &data[size..];

        // Each chunk's data is followed by a CRLF.
        if data.len() < 2 || &data[..2] != b"\r\n" {
            return Err(CoreError::protocol("chunked body: missing trailing CRLF"));
        }
        data = &data[2..];

        if out.len() > MAX_BODY_BYTES {
            return Err(CoreError::tool(format!(
                "chunked HTTP body exceeded {MAX_BODY_BYTES} bytes"
            )));
        }
    }
    Ok(out)
}

/// Find the first index of `needle` within `haystack`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // ---- URL parsing ----

    #[test]
    fn parses_basic_url() {
        let u = ParsedUrl::parse("http://example.com/path?q=1").unwrap();
        assert_eq!(u.host, "example.com");
        assert_eq!(u.port, 80);
        assert_eq!(u.path, "/path?q=1");
        assert!(u.default_port);
        assert_eq!(u.host_header(), "example.com");
    }

    #[test]
    fn parses_host_with_port_and_no_path() {
        let u = ParsedUrl::parse("http://localhost:8080").unwrap();
        assert_eq!(u.host, "localhost");
        assert_eq!(u.port, 8080);
        assert_eq!(u.path, "/");
        assert!(!u.default_port);
        assert_eq!(u.host_header(), "localhost:8080");
    }

    #[test]
    fn parses_ipv6_literal() {
        let u = ParsedUrl::parse("http://[::1]:9000/x").unwrap();
        assert_eq!(u.host, "::1");
        assert_eq!(u.port, 9000);
        assert_eq!(u.path, "/x");
    }

    #[test]
    fn strips_userinfo() {
        let u = ParsedUrl::parse("http://user:pass@host.test/p").unwrap();
        assert_eq!(u.host, "host.test");
        assert_eq!(u.port, 80);
    }

    #[test]
    fn https_is_rejected() {
        let err = ParsedUrl::parse("https://secure.test/").unwrap_err();
        assert!(err.is(na_common::ErrorKind::InvalidInput));
        assert!(err.message.contains("https not supported"));
    }

    #[test]
    fn non_http_scheme_rejected() {
        let err = ParsedUrl::parse("ftp://x/").unwrap_err();
        assert!(err.is(na_common::ErrorKind::InvalidInput));
    }

    // ---- chunked decoding (unit) ----

    #[test]
    fn decode_chunked_concatenates() {
        // Build the chunked encoding programmatically so the sizes are correct.
        let pieces = ["Wiki", "pedia", " in \r\n\r\nchunks."];
        let mut body = String::new();
        for p in pieces {
            body.push_str(&format!("{:x}\r\n{p}\r\n", p.len()));
        }
        body.push_str("0\r\n\r\n");
        let decoded = decode_chunked(body.as_bytes()).unwrap();
        // The CRLFs *inside* a chunk's data are payload and must survive.
        assert_eq!(
            String::from_utf8(decoded).unwrap(),
            "Wikipedia in \r\n\r\nchunks."
        );
    }

    #[test]
    fn decode_chunked_handles_extensions() {
        let body = b"3;name=value\r\nabc\r\n0\r\n\r\n";
        let decoded = decode_chunked(body).unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), "abc");
    }

    #[test]
    fn parse_status_line_ok() {
        let (code, reason) = parse_status_line("HTTP/1.1 404 Not Found").unwrap();
        assert_eq!(code, 404);
        assert_eq!(reason, "Not Found");
    }

    // ---- end-to-end against an in-process server ----

    /// Spawn a one-shot TCP server that runs `handler(request_bytes) -> response`
    /// for a single connection and returns the bound `http://` URL.
    async fn serve_once<F>(handler: F) -> String
    where
        F: FnOnce(Vec<u8>) -> Vec<u8> + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            // Read the request headers (until CRLFCRLF) — enough for a GET.
            let mut req = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let n = sock.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                req.extend_from_slice(&buf[..n]);
                if find_subslice(&req, b"\r\n\r\n").is_some() {
                    break;
                }
            }
            let resp = handler(req);
            sock.write_all(&resp).await.unwrap();
            sock.flush().await.unwrap();
            // Drop closes the socket -> client sees EOF.
        });
        format!("http://{addr}/")
    }

    #[tokio::test]
    async fn fetches_content_length_body() {
        let url = serve_once(|req| {
            // Sanity: the request should be a GET with a Host header.
            let text = String::from_utf8_lossy(&req);
            assert!(text.starts_with("GET / HTTP/1.1"));
            assert!(text.contains("Host: 127.0.0.1"));
            let body = "第一章 你好，世界";
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            )
            .into_bytes()
        })
        .await;

        let fetcher = HttpFetcher::new(5_000);
        let got = fetcher.fetch(&url).await.unwrap();
        assert_eq!(got, "第一章 你好，世界");
    }

    #[tokio::test]
    async fn fetches_chunked_body() {
        let url = serve_once(|_req| {
            let mut resp = String::from("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n");
            resp.push_str("4\r\nWiki\r\n");
            resp.push_str("5\r\npedia\r\n");
            resp.push_str("0\r\n\r\n");
            resp.into_bytes()
        })
        .await;

        let fetcher = HttpFetcher::default();
        let got = fetcher.fetch(&url).await.unwrap();
        assert_eq!(got, "Wikipedia");
    }

    #[tokio::test]
    async fn no_content_length_reads_to_eof() {
        let url = serve_once(|_req| {
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nbody-until-eof".to_vec()
        })
        .await;
        let fetcher = HttpFetcher::new(5_000);
        let got = fetcher.fetch(&url).await.unwrap();
        assert_eq!(got, "body-until-eof");
    }

    #[tokio::test]
    async fn http_error_status_is_error() {
        let url = serve_once(|_req| {
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nnot here!".to_vec()
        })
        .await;
        let fetcher = HttpFetcher::new(5_000);
        let err = fetcher.fetch(&url).await.unwrap_err();
        assert!(err.is(na_common::ErrorKind::Tool));
        assert!(err.message.contains("404"));
    }

    #[tokio::test]
    async fn server_error_status_maps_to_model() {
        let url = serve_once(|_req| {
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n".to_vec()
        })
        .await;
        let fetcher = HttpFetcher::new(5_000);
        let err = fetcher.fetch(&url).await.unwrap_err();
        assert!(err.is(na_common::ErrorKind::Model));
        assert!(err.message.contains("503"));
    }

    #[tokio::test]
    async fn https_fetch_errors() {
        let fetcher = HttpFetcher::default();
        let err = fetcher.fetch("https://example.com/").await.unwrap_err();
        assert!(err.is(na_common::ErrorKind::InvalidInput));
        assert!(err.message.contains("https not supported"));
    }

    #[tokio::test]
    async fn timeout_when_server_hangs() {
        // Server accepts but never replies; the fetch must time out fast.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            // Hold the socket open without responding.
            tokio::time::sleep(Duration::from_secs(30)).await;
            drop(sock);
        });
        let url = format!("http://{addr}/");
        let fetcher = HttpFetcher::new(150);
        let err = fetcher.fetch(&url).await.unwrap_err();
        assert!(err.is(na_common::ErrorKind::Timeout), "{err}");
    }

    #[tokio::test]
    async fn works_through_fetcher_trait_object() {
        let url =
            serve_once(|_req| b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec()).await;
        let fetcher: std::sync::Arc<dyn Fetcher> = std::sync::Arc::new(HttpFetcher::new(5_000));
        let got = fetcher.fetch(&url).await.unwrap();
        assert_eq!(got, "hello");
    }
}
