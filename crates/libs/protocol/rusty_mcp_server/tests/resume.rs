#![allow(clippy::unwrap_used)]
//! Resumable replies over real sockets: with `resume_buffer` set, an answer
//! is an event stream with ids, and a client that lost its connection fetches
//! the rest with `GET` and `Last-Event-ID`.

use rusty_json::Value;
use rusty_mcp_proto::{CallToolResult, ContentBlock, Tool};
use rusty_mcp_server::{bind_http, HttpConfig, Server};
use rusty_serve::{Limits, ShutdownHandle};
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn schema() -> Value {
    let mut s = Value::object();
    s.insert("type", "object");
    s
}

fn text(s: &str) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::text(s)],
        ..CallToolResult::default()
    }
}

struct Fixture {
    addr: SocketAddr,
    stop: ShutdownHandle,
    cancelled: Arc<AtomicBool>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.shutdown();
    }
}

fn start(config: HttpConfig) -> Fixture {
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let server = Arc::new(
        Server::builder("resume-fixture", "1")
            .tool(Tool::new("quick", schema()), |_c, _p| {
                Ok(text("quick done"))
            })
            .tool(Tool::new("slow", schema()), |_c, _p| {
                std::thread::sleep(Duration::from_millis(500));
                Ok(text("slow done"))
            })
            .tool(Tool::new("wait", schema()), move |ctx, _p| {
                let until = Instant::now() + Duration::from_secs(10);
                while !ctx.is_cancelled() && Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(5));
                }
                flag.store(ctx.is_cancelled(), Ordering::SeqCst);
                Ok(text("stopped"))
            })
            .build()
            .unwrap(),
    );
    let http = bind_http(
        server,
        "127.0.0.1:0".parse().unwrap(),
        config,
        Limits::default(),
    )
    .unwrap();
    let addr = http.local_addr().unwrap();
    let stop = http.shutdown_handle().unwrap();
    std::thread::spawn(move || http.run().unwrap());
    Fixture {
        addr,
        stop,
        cancelled,
    }
}

fn resumable() -> HttpConfig {
    HttpConfig {
        resume_buffer: 1 << 20,
        resume_grace: Duration::from_millis(400),
        keep_alive: Duration::from_millis(100),
        ..HttpConfig::default()
    }
}

/// A socket reading a response as it arrives.
struct Conn {
    sock: TcpStream,
    seen: String,
}

impl Conn {
    fn send(addr: SocketAddr, method: &str, headers: &[(&str, &str)], body: &str) -> Self {
        let mut sock = TcpStream::connect(addr).unwrap();
        sock.set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let mut raw = format!(
            "{method} /mcp HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {}\r\n",
            body.len()
        );
        for (k, v) in headers {
            raw.push_str(&format!("{k}: {v}\r\n"));
        }
        raw.push_str("\r\n");
        raw.push_str(body);
        sock.write_all(raw.as_bytes()).unwrap();
        Self {
            sock,
            seen: String::new(),
        }
    }

    /// Read until `until` holds or `limit` passes; whether it held. A closed
    /// connection also ends the wait.
    fn read_until(&mut self, limit: Duration, until: impl Fn(&str) -> bool) -> bool {
        let end = Instant::now() + limit;
        let mut buf = [0u8; 4096];
        while Instant::now() < end {
            if until(&self.seen) {
                return true;
            }
            match self.sock.read(&mut buf) {
                Ok(0) => return until(&self.seen),
                Ok(n) => self.seen.push_str(&String::from_utf8_lossy(&buf[..n])),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => return until(&self.seen),
            }
        }
        until(&self.seen)
    }

    fn status(&self) -> u16 {
        self.seen
            .split(' ')
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    /// The first event id in the stream (the priming event's).
    fn first_id(&self) -> String {
        self.seen
            .lines()
            .find_map(|l| l.strip_prefix("id: "))
            .map(|s| s.trim().to_owned())
            .unwrap_or_else(|| panic!("no event id in {:?}", self.seen))
    }

    fn hang_up(self) {
        let _ = self.sock.shutdown(Shutdown::Both);
    }
}

const ACCEPT: &str = "application/json, text/event-stream";

fn call_body(tool: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"{tool}","_meta":{{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{{}}}}}}}}"#
    )
}

fn call(f: &Fixture, tool: &str) -> Conn {
    Conn::send(
        f.addr,
        "POST",
        &[
            ("Accept", ACCEPT),
            ("Content-Type", "application/json"),
            ("MCP-Protocol-Version", "2026-07-28"),
            ("Mcp-Method", "tools/call"),
            ("Mcp-Name", tool),
            ("Connection", "close"),
        ],
        &call_body(tool),
    )
}

fn resume(f: &Fixture, last_id: &str) -> Conn {
    Conn::send(
        f.addr,
        "GET",
        &[
            ("Accept", "text/event-stream"),
            ("MCP-Protocol-Version", "2026-07-28"),
            ("Last-Event-ID", last_id),
            ("Connection", "close"),
        ],
        "",
    )
}

const WAIT: Duration = Duration::from_secs(5);

#[test]
fn an_answer_is_a_stream_with_a_priming_event_and_ids() {
    let f = start(resumable());
    let mut c = call(&f, "quick");
    assert!(
        c.read_until(WAIT, |s| s.contains("quick done")),
        "{:?}",
        c.seen
    );
    assert_eq!(c.status(), 200);
    assert!(c.seen.contains("text/event-stream"), "{:?}", c.seen);
    let first = c.first_id();
    assert!(first.starts_with('s') && first.ends_with("-0"), "{first}");
    assert!(c.seen.contains(&first.replace("-0", "-1")), "{:?}", c.seen);
}

#[test]
fn a_dropped_connection_is_resumed_and_gets_the_result() {
    let f = start(resumable());
    let mut c = call(&f, "slow");
    assert!(c.read_until(WAIT, |s| s.contains("\r\n\r\nid: ") || s.contains("id: s")));
    let priming = c.first_id();
    assert!(!c.seen.contains("slow done"), "answered before the drop");
    c.hang_up();

    let mut r = resume(&f, &priming);
    assert!(
        r.read_until(WAIT, |s| s.contains("slow done")),
        "{:?}",
        r.seen
    );
    assert_eq!(r.status(), 200);
    assert!(
        !r.seen.contains(&format!("id: {priming}\n")),
        "the event the client already had was replayed: {:?}",
        r.seen
    );
}

#[test]
fn a_finished_stream_can_be_replayed_from_any_event() {
    let f = start(resumable());
    let mut c = call(&f, "quick");
    assert!(c.read_until(WAIT, |s| s.contains("quick done")));
    let priming = c.first_id();
    let mut r = resume(&f, &priming);
    assert!(
        r.read_until(WAIT, |s| s.contains("quick done")),
        "{:?}",
        r.seen
    );
    // From the last event there is nothing left, and the stream ends.
    let last = priming.replace("-0", "-1");
    let mut r = resume(&f, &last);
    let t = Instant::now();
    r.read_until(WAIT, |_| false); // returns when the server closes the stream
    assert!(
        t.elapsed() < Duration::from_secs(3),
        "the stream stayed open"
    );
    assert!(!r.seen.contains("quick done"), "{:?}", r.seen);
}

#[test]
fn an_unknown_or_malformed_id_is_404() {
    let f = start(resumable());
    for id in ["s0000000000000000-0", "s12", "sxyz-1", "s-", "nonsense"] {
        let mut r = resume(&f, id);
        r.read_until(WAIT, |s| s.contains("\r\n\r\n"));
        // `nonsense` is not a resumable id, so the ordinary GET rules apply.
        let expected = if id == "nonsense" { 405 } else { 404 };
        assert_eq!(r.status(), expected, "{id}: {:?}", r.seen);
    }
}

#[test]
fn a_request_nobody_returns_for_is_cancelled_after_the_grace() {
    let f = start(resumable());
    let mut c = call(&f, "wait");
    assert!(c.read_until(WAIT, |s| s.contains("id: s")));
    c.hang_up();
    let end = Instant::now() + Duration::from_secs(5);
    while !f.cancelled.load(Ordering::SeqCst) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(f.cancelled.load(Ordering::SeqCst), "never cancelled");
}

#[test]
fn a_reader_that_comes_back_in_time_keeps_the_request_alive() {
    let f = start(HttpConfig {
        resume_grace: Duration::from_millis(600),
        ..resumable()
    });
    let mut c = call(&f, "wait");
    assert!(c.read_until(WAIT, |s| s.contains("id: s")));
    let priming = c.first_id();
    c.hang_up();
    std::thread::sleep(Duration::from_millis(200));
    let mut r = resume(&f, &priming);
    r.read_until(Duration::from_millis(1500), |s| s.contains("stopped"));
    assert!(
        !f.cancelled.load(Ordering::SeqCst),
        "cancelled while a reader was attached"
    );
    drop(r);
}

#[test]
fn nothing_is_kept_unless_asked_for() {
    let f = start(HttpConfig::default());
    let mut c = call(&f, "quick");
    assert!(c.read_until(WAIT, |s| s.contains("quick done")));
    assert!(c.seen.contains("application/json"), "{:?}", c.seen);
    assert!(!c.seen.contains("text/event-stream"), "{:?}", c.seen);
}

#[test]
fn the_oldest_streams_are_dropped_when_the_buffer_is_full() {
    let f = start(HttpConfig {
        resume_buffer: 600,
        ..resumable()
    });
    let mut first = call(&f, "quick");
    assert!(first.read_until(WAIT, |s| s.contains("quick done")));
    let old = first.first_id();
    let mut newest = String::new();
    for _ in 0..4 {
        let mut c = call(&f, "quick");
        assert!(c.read_until(WAIT, |s| s.contains("quick done")));
        newest = c.first_id();
    }
    let mut gone = resume(&f, &old);
    gone.read_until(WAIT, |s| s.contains("\r\n\r\n"));
    assert_eq!(gone.status(), 404, "{:?}", gone.seen);
    let mut kept = resume(&f, &newest);
    assert!(
        kept.read_until(WAIT, |s| s.contains("quick done")),
        "{:?}",
        kept.seen
    );
}

#[test]
fn a_classic_initialize_still_opens_a_session_when_replies_are_streams() {
    let f = start(resumable());
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"c","version":"1"}}}"#;
    let mut c = Conn::send(
        f.addr,
        "POST",
        &[
            ("Accept", ACCEPT),
            ("Content-Type", "application/json"),
            ("Connection", "close"),
        ],
        body,
    );
    assert!(
        c.read_until(WAIT, |s| s.contains("protocolVersion")),
        "{:?}",
        c.seen
    );
    assert!(
        c.seen.to_ascii_lowercase().contains("mcp-session-id: "),
        "{:?}",
        c.seen
    );
}
