//! The fake tick served over loopback HTTP with the default production client, and
//! counters of what it answered, for tests that measure storage traffic.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;

use rusty_mill_board::tick::{TickApi, TickRequest};
use serde_json::Value;

use super::FakeTick;

/// Requests answered and response-body bytes sent since the last [`Running::take_counts`].
#[derive(Default)]
struct Counters {
    requests: AtomicUsize,
    bytes: AtomicUsize,
}

/// Storage traffic over a span: requests answered and response-body bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Traffic {
    /// Requests the fixture answered.
    pub requests: usize,
    /// Response-body bytes it sent.
    pub bytes: usize,
}

impl Running {
    /// The traffic since the previous call (or since start), resetting the counters.
    pub fn take_counts(&self) -> Traffic {
        Traffic {
            requests: self.counters.requests.swap(0, Ordering::SeqCst),
            bytes: self.counters.bytes.swap(0, Ordering::SeqCst),
        }
    }
}

/// A [`FakeTick`] served over real loopback HTTP, counting what it answers.
pub struct Running {
    /// Base URL for [`rusty_mill_board::tick::TickHttp::new`].
    pub url: String,
    counters: Arc<Counters>,
    addr: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Running {
    /// Serve `tick`, accepting only `token` as the bearer.
    pub fn start(tick: FakeTick, token: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture");
        let addr = listener.local_addr().expect("fixture address");
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let counters = Arc::new(Counters::default());
        let counting = counters.clone();
        let thread = Some(std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
                serve(stream.expect("accept"), &tick, token, &counting);
            }
        }));
        Self {
            url: format!("http://{addr}"),
            counters,
            addr,
            stop,
            thread,
        }
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        self.thread
            .take()
            .expect("server thread")
            .join()
            .expect("join server");
    }
}
fn serve(mut stream: TcpStream, tick: &FakeTick, token: &str, counters: &Counters) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    let head_end = loop {
        let n = stream.read(&mut buffer).expect("request head");
        assert!(n > 0);
        bytes.extend_from_slice(&buffer[..n]);
        if let Some(index) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let head = String::from_utf8(bytes[..head_end].to_vec()).expect("HTTP head");
    let mut lines = head.lines();
    let mut first = lines.next().expect("request line").split_whitespace();
    let method = first.next().expect("method");
    let path = first
        .next()
        .expect("path")
        .strip_prefix("/api/v1")
        .expect("API prefix");
    let headers: Vec<_> = lines.filter_map(|l| l.split_once(':')).collect();
    let header = |name: &str| {
        headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim().to_owned())
    };
    assert_eq!(
        header("Authorization").as_deref(),
        Some(format!("Bearer {token}").as_str())
    );
    let len = header("Content-Length")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    while bytes.len() - head_end < len {
        let n = stream.read(&mut buffer).expect("request body");
        assert!(n > 0);
        bytes.extend_from_slice(&buffer[..n]);
    }
    let (status, body) = tick
        .send(&TickRequest {
            method,
            path: path.into(),
            body: if len == 0 {
                None
            } else {
                Some(
                    serde_json::from_slice(&bytes[head_end..head_end + len]).expect("JSON request"),
                )
            },
            if_match: header("If-Match"),
            if_absent: header("If-None-Match").as_deref() == Some("*"),
            if_task_match: header("If-Task-Match"),
        })
        .expect("fixture route");
    let body = body.to_string();
    counters.requests.fetch_add(1, Ordering::SeqCst);
    counters.bytes.fetch_add(body.len(), Ordering::SeqCst);
    let response = format!(
        "HTTP/1.1 {status} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    // The client deliberately closes oversized responses before reading the body.
    let _ = stream.write_all(response.as_bytes());
}
/// One request straight to the in-memory fixture (no HTTP), for seeding. Panics unless it succeeds.
pub fn call(
    tick: &mut FakeTick,
    method: rusty_http::Method,
    path: &str,
    body: Value,
    fenced: bool,
) -> Value {
    let (status, body) = tick
        .send(&TickRequest {
            method: &method.to_string(),
            path: path.strip_prefix("/api/v1").expect("API path").into(),
            body: Some(body),
            if_task_match: fenced.then(|| "1".into()),
            ..Default::default()
        })
        .expect("fixture request");
    assert!(status < 300, "fixture status {status}");
    body
}
