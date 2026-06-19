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

/// Hard cap on a request body — comfortably above a large `.replay`.
const MAX_BODY: usize = 64 * 1024 * 1024;

/// A parsed request: method, path (with any query string), and the raw body.
pub struct Request {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
}

impl Request {
    /// The path with any `?query` stripped.
    pub fn route(&self) -> &str {
        self.path.split('?').next().unwrap_or(&self.path)
    }
}

/// A response to write back: status, content-type, and a byte body.
pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

impl Response {
    pub fn html(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            content_type: "text/html; charset=utf-8".into(),
            body: body.into(),
        }
    }
    pub fn json(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            content_type: "application/json".into(),
            body: body.into(),
        }
    }
    pub fn text(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8".into(),
            body: body.into().into_bytes(),
        }
    }
    pub fn not_found() -> Self {
        Self::text(404, "not found")
    }
}

fn status_reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
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
    let response = match read_request(&stream) {
        Ok(Some(req)) => handler(&req),
        Ok(None) => return Ok(()), // empty/closed connection
        Err(e) => Response::text(400, format!("bad request: {e}")),
    };
    write_response(&stream, &response)
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

    Ok(Some(Request { method, path, body }))
}

fn write_response(mut stream: &TcpStream, resp: &Response) -> io::Result<()> {
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        resp.status,
        status_reason(resp.status),
        resp.content_type,
        resp.body.len(),
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(&resp.body)?;
    stream.flush()
}
