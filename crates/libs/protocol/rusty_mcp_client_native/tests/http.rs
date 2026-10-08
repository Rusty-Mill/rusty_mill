#![allow(clippy::unwrap_used)]
//! The client's Streamable HTTP transport against `rusty_mcp_server`'s HTTP
//! transport over real sockets, plus a raw-socket check of the headers.

mod common;

use common::{builder, eliciting, first_text, with_tasks, Asker, SECS};
use rusty_mcp_client_native::json::Value;
use rusty_mcp_client_native::{
    Client, ClientConfig, ClientError, Handler, HttpConfig, HttpTransport, NoHandler,
};
use rusty_mcp_server::{bind_http, ChangeBroadcaster, ChangeKinds, HttpConfig as ServerHttp};
use rusty_serve::{Limits, ShutdownHandle};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Running {
    url: String,
    stop: ShutdownHandle,
    changes: ChangeBroadcaster,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.shutdown();
    }
}

fn serve_with(builder: rusty_mcp_server::ServerBuilder) -> Running {
    let changes = ChangeBroadcaster::new();
    let server = builder
        .notify_changes(&changes, ChangeKinds::all())
        .build()
        .unwrap();
    let config = ServerHttp {
        sse_after: Duration::from_millis(100),
        keep_alive: Duration::from_millis(100),
        ..ServerHttp::default()
    };
    let http = bind_http(
        Arc::new(server),
        "127.0.0.1:0".parse().unwrap(),
        config,
        Limits::default(),
    )
    .unwrap();
    let url = format!("http://{}/mcp", http.local_addr().unwrap());
    let stop = http.shutdown_handle().unwrap();
    std::thread::spawn(move || http.run().unwrap());
    Running { url, stop, changes }
}

fn serve() -> Running {
    serve_with(builder())
}

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
    let mut c = Client::connect(
        transport(&s.url),
        ClientConfig::new("c", "1"),
        NoHandler,
        Duration::from_millis(400),
    )
    .unwrap();
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
    let mut c = Client::connect(
        transport(&s.url),
        classic(ClientConfig::new("c", "1")),
        NoHandler,
        Duration::from_millis(400),
    )
    .unwrap();
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
            let mut buf = [0u8; 8192];
            let n = stream.read(&mut buf).unwrap_or(0);
            let _ = tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());
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
