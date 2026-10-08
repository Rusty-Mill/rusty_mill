//! A tiny, dependency-free HTTP/1.1 server (thread-per-connection).
//!
//! Deliberately minimal — just enough to serve the single-page UI and a couple
//! of JSON endpoints — so the unified app stays in keeping with the workspace's
//! dependency-light ethos (no async runtime, no web framework). It handles one
//! request per connection (`Connection: close`) and reads a bounded body.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use crate::gzip;

/// Hard cap on a request body — comfortably above a large `.replay`.
pub const MAX_BODY: usize = 64 * 1024 * 1024;

/// A parsed request: method, path (with any query string), headers, and the raw body.
pub struct Request {
    pub method: String,
    pub path: String,
    /// Header `(name, value)` pairs in arrival order; names keep their wire casing.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    /// The path with any `?query` stripped.
    pub fn route(&self) -> &str {
        self.path.split('?').next().unwrap_or(&self.path)
    }

    /// The first header named `name` (ASCII case-insensitive), if any.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A streamed body: runs after the headers are sent and writes until it returns.
pub type Stream = Box<dyn FnOnce(&mut dyn Write) -> io::Result<()>>;

/// A response to write back: status, content-type, and a byte body (or a stream).
pub struct Response {
    pub status: u16,
    pub content_type: String,
    /// Extra headers (`Location`, `Set-Cookie`, ...), written after the fixed ones.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// When set, the body is this stream (Server-Sent Events) instead of `body`.
    pub stream: Option<Stream>,
}

impl Response {
    pub fn html(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            content_type: "text/html; charset=utf-8".into(),
            headers: Vec::new(),
            body: body.into(),
            stream: None,
        }
    }
    pub fn json(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            content_type: "application/json".into(),
            headers: Vec::new(),
            body: body.into(),
            stream: None,
        }
    }
    pub fn text(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8".into(),
            headers: Vec::new(),
            body: body.into().into_bytes(),
            stream: None,
        }
    }
    /// A `302 Found` to `location`.
    pub fn redirect(location: impl Into<String>) -> Self {
        Self::text(302, "").with_header("Location", location)
    }
    /// This response with one more header. CR and LF are stripped from the name
    /// and value: a line break would let a caller-controlled value inject headers.
    pub fn with_header(mut self, name: &str, value: impl Into<String>) -> Self {
        let clean = |s: &str| s.replace(['\r', '\n'], "");
        self.headers.push((clean(name), clean(&value.into())));
        self
    }
    /// A Server-Sent Events response: `stream` writes the events.
    pub fn events(stream: Stream) -> Self {
        let mut r = Self::text(200, "");
        r.content_type = "text/event-stream".into();
        r.stream = Some(stream);
        r.with_header("Cache-Control", "no-cache")
    }
    pub fn not_found() -> Self {
        Self::text(404, "not found")
    }
}

fn status_reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        302 => "Found",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        _ => "OK",
    }
}

/// Serve `handler` on `host:port` until the process is killed. Blocking.
pub fn serve<H>(host: &str, port: u16, handler: H) -> io::Result<()>
where
    H: Fn(&Request) -> Response + Send + Sync + 'static,
{
    let listener = TcpListener::bind((host, port))?;
    let handler = Arc::new(handler);
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("accept error: {e}");
                continue;
            }
        };
        let handler = Arc::clone(&handler);
        thread::spawn(move || {
            if let Err(e) = handle_connection(stream, handler.as_ref()) {
                eprintln!("connection error: {e}");
            }
        });
    }
    Ok(())
}

fn handle_connection<H>(stream: TcpStream, handler: &H) -> io::Result<()>
where
    H: Fn(&Request) -> Response,
{
    let (response, gzip_ok) = match read_request(&stream) {
        Ok(Some(req)) => {
            let ok = req
                .header("accept-encoding")
                .is_some_and(|v| v.contains("gzip"));
            (handler(&req), ok)
        }
        Ok(None) => return Ok(()), // empty/closed connection
        Err(e) => (Response::text(400, format!("bad request: {e}")), false),
    };
    let response = if gzip_ok {
        compressed(response)
    } else {
        response
    };
    write_response(&stream, response)
}

fn read_request(stream: &TcpStream) -> io::Result<Option<Request>> {
    let mut reader = BufReader::new(stream);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(None);
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or("/").to_string();

    let mut content_length = 0usize;
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break; // end of headers
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid content-length")
                })?;
            }
        }
    }

    if content_length > MAX_BODY {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "body too large"));
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }

    Ok(Some(Request {
        method,
        path,
        headers,
        body,
    }))
}

/// Bodies smaller than this are not worth compressing.
const GZIP_MIN_BYTES: usize = 1024;

/// `resp` gzip-encoded, if it is a text or JSON body big enough to gain from it.
fn compressed(mut resp: Response) -> Response {
    let text =
        resp.content_type.starts_with("text/") || resp.content_type.starts_with("application/json");
    if resp.status != 200 || resp.stream.is_some() || !text || resp.body.len() < GZIP_MIN_BYTES {
        return resp;
    }
    let z = gzip::compress(&resp.body);
    if z.len() >= resp.body.len() {
        return resp;
    }
    resp.body = z;
    resp.with_header("Content-Encoding", "gzip")
        .with_header("Vary", "Accept-Encoding")
}

fn write_response(mut stream: &TcpStream, mut resp: Response) -> io::Result<()> {
    let extra: String = resp
        .headers
        .iter()
        .map(|(n, v)| format!("{n}: {v}\r\n"))
        .collect();
    let length = if resp.stream.is_some() {
        String::new()
    } else {
        format!("Content-Length: {}\r\n", resp.body.len())
    };
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\n{length}Connection: close\r\n{extra}\r\n",
        resp.status,
        status_reason(resp.status),
        resp.content_type,
    );
    stream.write_all(header.as_bytes())?;
    match resp.stream.take() {
        Some(events) => {
            stream.flush()?;
            events(&mut stream)?;
        }
        None => stream.write_all(&resp.body)?,
    }
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_big_text_bodies_are_gzipped_and_say_so() {
        let big = "<td>row</td>".repeat(500);
        let z = compressed(Response::html(big.clone()));
        assert!(z.body.len() < big.len() / 5 && z.body[..2] == [0x1f, 0x8b]);
        assert!(z
            .headers
            .iter()
            .any(|(n, v)| n == "Content-Encoding" && v == "gzip"));
        assert!(z.headers.iter().any(|(n, _)| n == "Vary"));
        assert!(
            compressed(Response::html("tiny")).headers.is_empty(),
            "too small"
        );
        assert!(
            compressed(Response::text(404, big.clone()))
                .headers
                .is_empty(),
            "errors stay plain"
        );
        let mut png = Response::html(big);
        png.content_type = "image/png".into();
        assert!(
            compressed(png).headers.is_empty(),
            "binary types are left alone"
        );
    }

    #[test]
    fn header_values_cannot_inject_new_headers() {
        let r = Response::text(200, "").with_header("Location", "/a\r\nSet-Cookie: evil=1");
        assert_eq!(
            r.headers[0].1, "/aSet-Cookie: evil=1",
            "line breaks are removed"
        );
        let r = Response::redirect("/ok");
        assert_eq!((r.status, r.headers[0].0.as_str()), (302, "Location"));
    }
}
