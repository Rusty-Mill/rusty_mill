//! A minimal HTTP/1.1 mock server for client tests.
//!
//! `rusty_wiremock` (this workspace's own mock-server crate) is still a
//! stub -- `MockServer::register` is a no-op and nothing actually binds a
//! listener -- so this hand-rolls just enough to drive a backend client
//! end to end: a background OS thread that accepts one connection per
//! queued response and writes back a fixed status/body. Blocking `std`
//! I/O on its own thread, not async, so it needs no runtime of its own and
//! can't collide with the `tokio` runtime the test itself runs under.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

/// One canned response: status code, reason phrase, and JSON body.
pub struct MockResponse {
    pub status: u16,
    pub reason: &'static str,
    pub body: String,
}

impl MockResponse {
    pub fn ok(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            reason: "OK",
            body: body.into(),
        }
    }

    pub fn status(status: u16, reason: &'static str, body: impl Into<String>) -> Self {
        Self {
            status,
            reason,
            body: body.into(),
        }
    }
}

/// Starts a background thread serving `responses` in order, one per accepted
/// connection, and returns the `http://127.0.0.1:{port}` base URL to point a
/// client at. The thread exits once every response has been served.
pub fn spawn(responses: Vec<MockResponse>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock listener local addr");

    thread::spawn(move || {
        for response in responses {
            let (mut stream, _) = listener.accept().expect("accept mock connection");
            read_request_head(&mut stream);

            let payload = format!(
                "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response.status,
                response.reason,
                response.body.len(),
                response.body,
            );
            let _ = stream.write_all(payload.as_bytes());
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    });

    format!("http://{addr}")
}

/// Reads (and discards) bytes off `stream` until the end of the HTTP header
/// block, then drains exactly as much of the request body as the headers'
/// own `Content-Length` promises, so the client's write completes before
/// this side responds and closes the connection. Stopping right after the
/// headers left any request body still in flight when the response arrived
/// -- a race that a bodyless GET/POST never hits, but a JSON-body POST does,
/// and Windows' TCP stack surfaces it as a hard connection reset far more
/// readily than Linux's (the write raced a peer close instead of quietly
/// truncating).
fn read_request_head(stream: &mut std::net::TcpStream) {
    let mut buf = [0u8; 8192];
    let mut total = 0usize;
    let header_end = loop {
        if total == buf.len() {
            break total;
        }
        let n = stream.read(&mut buf[total..]).unwrap_or(0);
        if n == 0 {
            break total;
        }
        total += n;
        if let Some(pos) = buf[..total].windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };

    let content_length = content_length(&buf[..header_end]);
    let mut body_read = total - header_end;
    while body_read < content_length {
        let n = stream.read(&mut buf).unwrap_or(0);
        if n == 0 {
            break;
        }
        body_read += n;
    }
}

/// Parses a `Content-Length` header's value out of a raw HTTP header block,
/// defaulting to 0 (no body) if it's absent or malformed.
fn content_length(head: &[u8]) -> usize {
    String::from_utf8_lossy(head)
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())
                .flatten()
        })
        .unwrap_or(0)
}
