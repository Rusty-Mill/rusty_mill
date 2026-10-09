#![allow(clippy::unwrap_used)]
//! The client's Streamable HTTP transport against `rusty_mcp_server`'s HTTP
//! transport over real sockets, plus a raw-socket check of the headers.

mod common;

use common::{builder, eliciting, first_text, serve, serve_with, with_tasks, Asker, SECS};
use rusty_mcp_client_native::json::Value;
use rusty_mcp_client_native::{
    Client, ClientConfig, ClientError, Handler, HttpConfig, HttpTransport, NoHandler,
};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn transport(url: &str) -> HttpTransport {
    HttpTransport::new(HttpConfig::new(url)).unwrap()
}

fn connect<H: Handler>(url: &str, config: ClientConfig, handler: H) -> Client<HttpTransport, H> {
    Client::connect(transport(url), config, handler, SECS).unwrap()
}

fn classic(mut c: ClientConfig) -> ClientConfig {
    c.versions.retain(|v| !v.is_stateless());
    c
}

fn args(a: i64, b: i64) -> Option<Value> {
    let mut v = Value::object();
    v.insert("a", a);
    v.insert("b", b);
    Some(v)
}

#[test]
fn stateless_calls_carry_the_routing_headers_the_server_checks() {
    let s = serve();
    let mut c = connect(&s.url, ClientConfig::new("c", "1"), NoHandler);
    assert_eq!(c.session().negotiated().unwrap().as_str(), "2026-07-28");
    assert_eq!(c.list_tools().unwrap().len(), 7, "pages followed");
    assert_eq!(first_text(&c.call_tool("add", args(40, 2)).unwrap()), "42");
    assert_eq!(c.read_resource("mem://a").unwrap().contents.len(), 1);
    // The server answers 400 if Mcp-Method / Mcp-Name were wrong or missing.
    let err = c.call_tool("add", None).unwrap_err();
    assert!(matches!(err, ClientError::Rpc(e) if e.code.0 == -32602));
}

#[test]
fn a_classic_session_is_kept_and_ended_on_drop() {
    let s = serve();
    let mut c = connect(&s.url, classic(ClientConfig::new("c", "1")), NoHandler);
    assert_eq!(c.session().negotiated().unwrap().as_str(), "2025-11-25");
    assert_eq!(first_text(&c.call_tool("add", args(1, 2)).unwrap()), "3");
    drop(c); // sends DELETE; must not hang
}

#[test]
fn a_progress_call_arrives_as_an_event_stream_with_its_notifications() {
    struct Progress(Vec<f64>);
    impl Handler for Progress {
        fn notification(&mut self, method: &str, params: Option<&Value>) {
            if method == "notifications/progress" {
                self.0.push(params.unwrap()["progress"].as_f64().unwrap());
            }
        }
    }
    let s = serve();
    let mut c = connect(&s.url, ClientConfig::new("c", "1"), Progress(vec![]));
    let mut params = Value::object();
    params.insert("name", "progress");
    let mut meta = Value::object();
    meta.insert("progressToken", "t");
    params.insert("_meta", meta);
    let done = c.call("tools/call", Some(params)).unwrap();
    assert_eq!(done["content"][0]["text"].as_str(), Some("done"));
    assert_eq!(c.handler().0, [1.0, 2.0, 3.0]);
}

#[test]
fn input_required_and_tasks_work_across_posts() {
    let s = serve();
    let mut c = connect(
        &s.url,
        with_tasks(),
        Asker {
            accept: true,
            asked: vec![],
        },
    );
    assert_eq!(
        first_text(&c.call_tool("confirm", None).unwrap()),
        "pending accepted"
    );
    assert_eq!(first_text(&c.call_tool("job", None).unwrap()), "job done");
    assert_eq!(
        first_text(&c.call_tool("ask_job", None).unwrap()),
        "hello Ann"
    );
    assert_eq!(c.handler().asked, ["Proceed?", "Name?"]);
}

#[test]
fn input_required_over_a_classic_session_is_refused_by_the_server_not_hung() {
    let s = serve();
    let mut c = connect(
        &s.url,
        classic(eliciting()),
        Asker {
            accept: true,
            asked: vec![],
        },
    );
    let err = c.call_tool("confirm", None).unwrap_err();
    assert!(
        matches!(&err, ClientError::Rpc(e) if e.code.0 == -32600),
        "{err}"
    );
}

#[test]
fn a_connection_refused_fails_the_call_at_once() {
    // A port nothing listens on.
    let free = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = free.local_addr().unwrap();
    drop(free);
    let started = Instant::now();
    let err = Client::connect(
        transport(&format!("http://{addr}/mcp")),
        ClientConfig::new("c", "1"),
        NoHandler,
        Duration::from_secs(10),
    )
    .err()
    .unwrap();
    assert!(matches!(err, ClientError::Rpc(_)), "{err}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "waited for the timeout"
    );
}

#[test]
fn a_timed_out_stateless_call_hangs_up_and_the_server_cancels_it() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let b = builder().tool(
        rusty_mcp_client_native::proto::Tool::new("hang", {
            let mut s = Value::object();
            s.insert("type", "object");
            s
        }),
        move |ctx, _p| {
            let until = Instant::now() + Duration::from_secs(10);
            while !ctx.is_cancelled() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            flag.store(ctx.is_cancelled(), Ordering::SeqCst);
            Ok(rusty_mcp_client_native::proto::CallToolResult::default())
        },
    );
    let s = serve_with(b);
    let mut config = ClientConfig::new("c", "1");
    config.call_timeout = Duration::from_millis(400);
    let mut c = Client::connect(transport(&s.url), config, NoHandler, SECS).unwrap();
    assert!(matches!(
        c.call_tool("hang", None).unwrap_err(),
        ClientError::Timeout
    ));
    let until = Instant::now() + Duration::from_secs(5);
    while !cancelled.load(Ordering::SeqCst) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        cancelled.load(Ordering::SeqCst),
        "the server never saw the hang-up"
    );
}

#[test]
fn a_classic_timeout_cancels_through_the_session() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let b = builder().tool(
        rusty_mcp_client_native::proto::Tool::new("hang", {
            let mut s = Value::object();
            s.insert("type", "object");
            s
        }),
        move |ctx, _p| {
            let until = Instant::now() + Duration::from_secs(10);
            while !ctx.is_cancelled() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            flag.store(ctx.is_cancelled(), Ordering::SeqCst);
            Ok(rusty_mcp_client_native::proto::CallToolResult::default())
        },
    );
    let s = serve_with(b);
    let mut config = classic(ClientConfig::new("c", "1"));
    config.call_timeout = Duration::from_millis(400);
    let mut c = Client::connect(transport(&s.url), config, NoHandler, SECS).unwrap();
    assert!(matches!(
        c.call_tool("hang", None).unwrap_err(),
        ClientError::Timeout
    ));
    let until = Instant::now() + Duration::from_secs(5);
    while !cancelled.load(Ordering::SeqCst) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        cancelled.load(Ordering::SeqCst),
        "notifications/cancelled never arrived"
    );
}

#[derive(Default)]
struct Seen(Vec<(String, Option<Value>)>);

impl Handler for Seen {
    fn notification(&mut self, method: &str, params: Option<&Value>) {
        self.0.push((method.to_owned(), params.cloned()));
    }
}

#[test]
fn a_classic_session_receives_pushed_updates_for_what_it_followed() {
    let s = serve();
    let mut c = connect(
        &s.url,
        classic(ClientConfig::new("c", "1")),
        Seen::default(),
    );
    c.call("resources/subscribe", {
        let mut p = Value::object();
        p.insert("uri", "mem://a");
        Some(p)
    })
    .unwrap();
    s.changes.resource_updated("mem://b"); // not followed
    s.changes.resource_updated("mem://a");
    s.changes.tools_changed();
    let deadline = Instant::now() + Duration::from_secs(5);
    while c.handler().0.len() < 2 && Instant::now() < deadline {
        c.pump(Duration::from_millis(100)).unwrap();
    }
    let methods: Vec<&str> = c.handler().0.iter().map(|(m, _)| m.as_str()).collect();
    assert_eq!(
        methods,
        [
            "notifications/resources/updated",
            "notifications/tools/list_changed"
        ]
    );
    assert_eq!(
        c.handler().0[0].1.as_ref().unwrap()["uri"].as_str(),
        Some("mem://a")
    );
}

#[test]
fn headers_are_what_a_server_expects() {
    // A raw server that records the first request and answers 500.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            // Headers and body may arrive in separate reads.
            let _ = stream.set_read_timeout(Some(Duration::from_millis(300)));
            let mut got = Vec::new();
            let mut buf = [0u8; 8192];
            while let Ok(n @ 1..) = stream.read(&mut buf) {
                got.extend_from_slice(&buf[..n]);
                if got.windows(26).any(|w| w == b"notifications/initialized\"") {
                    break;
                }
            }
            let _ = tx.send(String::from_utf8_lossy(&got).into_owned());
            let _ = stream.write_all(
                b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    let mut config = HttpConfig::new(format!("http://{addr}/mcp"));
    config.bearer_token = Some("sekrit".to_owned());
    config.headers.push(("X-Trace".to_owned(), "t1".to_owned()));
    let _ = Client::connect(
        HttpTransport::new(config).unwrap(),
        ClientConfig::new("c", "1"),
        NoHandler,
        Duration::from_secs(5),
    );
    let head = rx.recv_timeout(SECS).unwrap().to_ascii_lowercase();
    assert!(head.starts_with("post /mcp http/1.1"), "{head}");
    assert!(head.contains("authorization: bearer sekrit"), "{head}");
    assert!(head.contains("x-trace: t1"), "{head}");
    assert!(head.contains("content-type: application/json"), "{head}");
    assert!(
        head.contains("accept: application/json, text/event-stream"),
        "{head}"
    );
}

#[test]
fn a_redirect_is_not_followed_and_nothing_is_sent_to_where_it_points() {
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    target.set_nonblocking(true).unwrap();
    let target_url = format!("http://{}/capture", target.local_addr().unwrap());
    let origin = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin_url = format!("http://{}/mcp", origin.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in origin.incoming() {
            let mut stream = stream.unwrap();
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 307 Temporary Redirect\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            );
        }
    });
    let mut config = HttpConfig::new(origin_url);
    config
        .headers
        .push(("X-Api-Key".to_owned(), "secret".to_owned()));
    let err = Client::connect(
        HttpTransport::new(config).unwrap(),
        ClientConfig::new("c", "1"),
        NoHandler,
        Duration::from_secs(5),
    )
    .err()
    .unwrap();
    assert!(
        matches!(&err, ClientError::Rpc(e) if e.message.contains("307")),
        "{err}"
    );
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        target.accept().is_err(),
        "the redirect target was contacted"
    );
}

#[test]
fn the_handshake_timeout_and_the_call_timeout_are_separate() {
    let s = serve();
    let mut config = ClientConfig::new("c", "1");
    config.call_timeout = Duration::from_millis(150);
    // A generous handshake budget, a short call budget: `slow` takes 400 ms.
    let mut c = Client::connect(
        transport(&s.url),
        config,
        NoHandler,
        Duration::from_secs(10),
    )
    .unwrap();
    assert!(matches!(
        c.call_tool("slow", None).unwrap_err(),
        ClientError::Timeout
    ));
}

#[test]
fn a_notification_sent_just_before_closing_is_not_lost() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            // Headers and body may arrive in separate reads.
            let _ = stream.set_read_timeout(Some(Duration::from_millis(300)));
            let mut got = Vec::new();
            let mut buf = [0u8; 8192];
            while let Ok(n @ 1..) = stream.read(&mut buf) {
                got.extend_from_slice(&buf[..n]);
                if got.windows(26).any(|w| w == b"notifications/initialized\"") {
                    break;
                }
            }
            let _ = tx.send(String::from_utf8_lossy(&got).into_owned());
            let _ = stream.write_all(
                b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    let mut t = HttpTransport::new(HttpConfig::new(format!("http://{addr}/mcp"))).unwrap();
    use rusty_mcp_client_native::Transport;
    t.send(&rusty_mcp_client_native::proto::Message::Notification {
        method: "notifications/initialized".to_owned(),
        params: None,
    })
    .unwrap();
    drop(t); // closes at once: the POST must still go out
    let got = rx
        .recv_timeout(SECS)
        .expect("the notification never arrived");
    assert!(got.contains("notifications/initialized"), "{got}");
}
