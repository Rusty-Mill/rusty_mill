//! A fake OTLP/HTTP collector for the telemetry tests: it records what is
//! POSTed, parsed as JSON so a test can look at real structure rather than scan
//! bytes for a string, and answers a fixed status.
#![allow(dead_code)]

use std::{
    io::{Read as _, Write as _},
    net::{SocketAddr, TcpListener},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use rusty_json::Value;

/// One request the collector received.
#[derive(Debug, Clone)]
pub struct Push {
    pub path: String,
    pub content_type: String,
    pub body: Value,
}

pub struct Collector {
    pub addr: SocketAddr,
    received: Arc<Mutex<Vec<Push>>>,
}

impl Collector {
    /// Start a collector that answers every POST with `status`.
    pub fn start(status: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the collector");
        let addr = listener.local_addr().expect("address");
        let received: Arc<Mutex<Vec<Push>>> = Arc::default();

        let store = Arc::clone(&received);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let store = Arc::clone(&store);
                thread::spawn(move || {
                    let mut raw = Vec::new();
                    let mut buf = [0u8; 16 * 1024];
                    while let Ok(n) = stream.read(&mut buf) {
                        raw.extend_from_slice(&buf[..n]);
                        if n == 0 || is_complete(&raw) {
                            break;
                        }
                    }
                    if let Some(push) = parse(&raw) {
                        store.lock().expect("lock").push(push);
                    }
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                    );
                });
            }
        });

        Self { addr, received }
    }

    /// The base URL, for `OtelConfig::with_endpoint`.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn pushes(&self) -> Vec<Push> {
        self.received.lock().expect("lock").clone()
    }

    /// Every span received, across pushes, in arrival order.
    pub fn spans(&self) -> Vec<Value> {
        self.pushes()
            .iter()
            .filter(|push| push.path == "/v1/traces")
            .flat_map(|push| {
                let spans = &push.body["resourceSpans"][0]["scopeSpans"][0]["spans"];
                spans.as_array().cloned().unwrap_or_default()
            })
            .collect()
    }

    /// Wait up to `limit` for `done` to hold of what has arrived.
    pub fn wait_until(&self, limit: Duration, done: impl Fn(&Collector) -> bool) -> bool {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if done(self) {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        done(self)
    }
}

fn head_and_body(raw: &[u8]) -> Option<(String, &[u8])> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    Some((
        String::from_utf8_lossy(&raw[..end]).into_owned(),
        &raw[end + 4..],
    ))
}

fn header(head: &str, name: &str) -> Option<String> {
    head.lines().skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_owned())
    })
}

fn is_complete(raw: &[u8]) -> bool {
    let Some((head, body)) = head_and_body(raw) else {
        return false;
    };
    let length: usize = header(&head, "content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    body.len() >= length
}

fn parse(raw: &[u8]) -> Option<Push> {
    let (head, body) = head_and_body(raw)?;
    let request_line = head.lines().next()?;
    assert!(request_line.starts_with("POST "), "{request_line}");
    Some(Push {
        path: request_line.split_whitespace().nth(1)?.to_owned(),
        content_type: header(&head, "content-type").unwrap_or_default(),
        body: Value::parse(std::str::from_utf8(body).ok()?).ok()?,
    })
}

/// An address nothing is listening on.
pub fn dead_address() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("address")
}
