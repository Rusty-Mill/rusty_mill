#![allow(clippy::unwrap_used)]
//! The Streamable HTTP transport over real sockets: validation, status
//! mapping, plain-JSON versus event-stream replies, and cancellation by
//! hanging up.

use rusty_json::Value;
use rusty_mcp_proto::{CallToolResult, ContentBlock, Tool};
use rusty_mcp_server::{bind_http, HttpConfig, Server};
use rusty_serve::{Limits, ShutdownHandle};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
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
        Server::builder("http-fixture", "1")
            .tool(Tool::new("add", schema()), |_c, call| {
                let n = |k: &str| {
                    call.arguments
                        .as_ref()
                        .and_then(|a| a.get(k))
                        .and_then(Value::as_i64)
                };
                Ok(text(
                    &(n("a").unwrap_or(0) + n("b").unwrap_or(0)).to_string(),
                ))
            })
            .tool(Tool::new("progress", schema()), |ctx, _c| {
                for i in 1..=3 {
                    ctx.progress(f64::from(i), Some(3.0), None);
                }
                Ok(text("finished"))
            })
            .tool(Tool::new("slow", schema()), |_c, _p| {
                std::thread::sleep(Duration::from_millis(400));
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

fn fast() -> HttpConfig {
    HttpConfig {
        sse_after: Duration::from_millis(100),
        keep_alive: Duration::from_millis(50),
        ..HttpConfig::default()
    }
}

struct Reply {
    status: u16,
    head: String,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().skip(1).find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case(name).then_some(v.trim())
        })
    }

    fn json(&self) -> Value {
        Value::from_json_str(&self.body).unwrap_or_else(|e| panic!("{e}: {:?}", self.body))
    }

    /// The `data:` payloads of an event-stream body.
    fn events(&self) -> Vec<Value> {
        self.body
            .lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .map(|d| Value::from_json_str(d).unwrap())
            .collect()
    }
}

/// Undo `Transfer-Encoding: chunked`.
fn dechunk(raw: &str) -> String {
    let mut out = String::new();
    let mut rest = raw;
    while let Some((len, tail)) = rest.split_once("\r\n") {
        let n = usize::from_str_radix(len.trim(), 16).unwrap_or(0);
        if n == 0 {
            break;
        }
        out.push_str(&tail[..n]);
        rest = &tail[n + 2..];
    }
    out
}

const JSON_AND_SSE: &str = "application/json, text/event-stream";

fn request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Reply {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut raw = format!(
        "{method} {path} HTTP/1.1\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    let have = |n: &str| headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(n));
    if !have("host") {
        raw.push_str(&format!("Host: {addr}\r\n"));
    }
    for (k, v) in headers {
        raw.push_str(&format!("{k}: {v}\r\n"));
    }
    raw.push_str("\r\n");
    raw.push_str(body);
    stream.write_all(raw.as_bytes()).unwrap();
    let mut all = String::new();
    let _ = stream.read_to_string(&mut all);
    let (head, body) = all.split_once("\r\n\r\n").unwrap_or((&all, ""));
    let status = head
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        dechunk(body)
    } else {
        body.to_owned()
    };
    Reply {
        status,
        head: head.to_owned(),
        body,
    }
}

fn post(addr: SocketAddr, headers: &[(&str, &str)], body: &str) -> Reply {
    let mut h = vec![
        ("Accept", JSON_AND_SSE),
        ("Content-Type", "application/json"),
    ];
    h.extend_from_slice(headers);
    request(addr, "POST", "/mcp", &h, body)
}

fn code(r: &Reply) -> i64 {
    r.json()
        .get("error")
        .unwrap()
        .get("code")
        .unwrap()
        .as_i64()
        .unwrap()
}

const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"c","version":"1"}}}"#;
const LIST: &str = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
const META: &str = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}"#;

fn modern_call(id: i64, tool: &str, extra_meta: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{tool}","arguments":{{"a":2,"b":3}},"_meta":{{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{{}}{extra_meta}}}}}}}"#
    )
}

fn modern_headers<'a>(method: &'a str, name: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![
        ("MCP-Protocol-Version", "2026-07-28"),
        ("Mcp-Method", method),
        ("Mcp-Name", name),
    ]
}

#[test]
fn a_classic_client_works_with_or_without_echoing_its_session() {
    let f = start(fast());
    let init = post(f.addr, &[], INIT);
    assert_eq!(init.status, 200);
    assert_eq!(init.header("Mcp-Session-Id").map(str::len), Some(32));
    assert!(init
        .header("content-type")
        .unwrap()
        .starts_with("application/json"));
    assert_eq!(
        init.json()
            .get("result")
            .unwrap()
            .get("protocolVersion")
            .and_then(Value::as_str),
        Some("2025-06-18")
    );

    // After `initialize` a classic client sends the version in a header.
    let listed = post(f.addr, &[("MCP-Protocol-Version", "2025-06-18")], LIST);
    assert_eq!(listed.status, 200);
    assert_eq!(
        listed
            .json()
            .get("result")
            .unwrap()
            .get("tools")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert!(
        listed.json().get("result").unwrap().get("ttlMs").is_none(),
        "no cache hints for a classic revision"
    );

    // A pre-2025-06-18 client sends no header: 2025-03-26 is assumed.
    assert_eq!(post(f.addr, &[], LIST).status, 200);

    // A classic error is an HTTP 200 JSON-RPC error.
    let unknown = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"zzz"}}"#;
    let r = post(f.addr, &[("MCP-Protocol-Version", "2025-06-18")], unknown);
    assert_eq!((r.status, code(&r)), (200, -32602));

    // A notification is accepted and has no body.
    let note = post(
        f.addr,
        &[],
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );
    assert_eq!((note.status, note.body.as_str()), (202, ""));
}

#[test]
fn a_stateless_client_gets_cache_hints_and_http_status_codes() {
    let f = start(fast());
    let list = format!(r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{{{META}}}}}"#);
    let r = post(f.addr, &modern_headers("tools/list", ""), &list);
    assert_eq!(r.status, 200);
    let result = r.json();
    let result = result.get("result").unwrap();
    assert_eq!(result.get("ttlMs").and_then(Value::as_i64), Some(0));
    assert_eq!(
        result.get("cacheScope").and_then(Value::as_str),
        Some("public")
    );

    let called = post(
        f.addr,
        &modern_headers("tools/call", "add"),
        &modern_call(3, "add", ""),
    );
    assert_eq!(called.status, 200);
    assert_eq!(
        called
            .json()
            .get("result")
            .unwrap()
            .get("content")
            .unwrap()
            .get_index(0)
            .unwrap()
            .get("text")
            .and_then(Value::as_str),
        Some("5")
    );

    // Modern errors map to statuses: unknown tool 400, unknown method 404.
    let r = post(
        f.addr,
        &modern_headers("tools/call", "zzz"),
        &modern_call(4, "zzz", ""),
    );
    assert_eq!((r.status, code(&r)), (400, -32602));
    let nope = format!(r#"{{"jsonrpc":"2.0","id":5,"method":"nope","params":{{{META}}}}}"#);
    let r = post(f.addr, &modern_headers("nope", ""), &nope);
    assert_eq!((r.status, code(&r)), (404, -32601));
    // `server/discover` works without a handshake.
    let discover =
        format!(r#"{{"jsonrpc":"2.0","id":6,"method":"server/discover","params":{{{META}}}}}"#);
    let r = post(f.addr, &modern_headers("server/discover", ""), &discover);
    assert_eq!(r.status, 200);
    assert!(r
        .json()
        .get("result")
        .unwrap()
        .get("supportedVersions")
        .is_some());
}

#[test]
fn standard_headers_must_agree_with_the_body() {
    let f = start(fast());
    let call = modern_call(1, "add", "");
    let cases: Vec<(Vec<(&str, &str)>, &str)> = vec![
        (
            vec![("MCP-Protocol-Version", "2026-07-28"), ("Mcp-Name", "add")],
            "missing Mcp-Method",
        ),
        (modern_headers("tools/list", "add"), "wrong Mcp-Method"),
        (
            vec![
                ("MCP-Protocol-Version", "2026-07-28"),
                ("Mcp-Method", "tools/call"),
            ],
            "missing Mcp-Name",
        ),
        (modern_headers("tools/call", "other"), "wrong Mcp-Name"),
        (
            modern_headers("tools/call", "=?base64?!!!?="),
            "bad base64 Mcp-Name",
        ),
    ];
    for (headers, what) in cases {
        let r = post(f.addr, &headers, &call);
        assert_eq!((r.status, code(&r)), (400, -32020), "{what}: {}", r.body);
    }
    // A name that needs encoding arrives wrapped in the base64 sentinel.
    let r = post(
        f.addr,
        &modern_headers("tools/call", "=?base64?YWRk?="),
        &call,
    );
    assert_eq!(r.status, 200, "{}", r.body);
}

#[test]
fn version_header_and_meta_must_agree() {
    let f = start(fast());
    // `_meta` names a revision but the header is missing, or differs.
    let call = modern_call(1, "add", "");
    let r = post(
        f.addr,
        &[("Mcp-Method", "tools/call"), ("Mcp-Name", "add")],
        &call,
    );
    assert_eq!((r.status, code(&r)), (400, -32020));
    let mut h = modern_headers("tools/call", "add");
    h[0] = ("MCP-Protocol-Version", "2025-06-18");
    let r = post(f.addr, &h, &call);
    assert_eq!((r.status, code(&r)), (400, -32020));
    // A 2026 header with no `_meta` version is invalid params.
    let bare = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    let r = post(f.addr, &modern_headers("tools/list", ""), bare);
    assert_eq!((r.status, code(&r)), (400, -32602));
    // An unknown header version, and a header that disagrees with `initialize`.
    let r = post(f.addr, &[("MCP-Protocol-Version", "2031-01-01")], LIST);
    assert_eq!((r.status, code(&r)), (400, -32022));
    let r = post(f.addr, &[("MCP-Protocol-Version", "2025-03-26")], INIT);
    assert_eq!((r.status, code(&r)), (400, -32600));
}

#[test]
fn http_level_requests_are_refused_with_the_right_status() {
    let f = start(HttpConfig {
        allowed_origins: vec!["https://app.example".into()],
        ..fast()
    });
    let get = request(
        f.addr,
        "GET",
        "/mcp",
        &[("Accept", "text/event-stream")],
        "",
    );
    assert_eq!(get.status, 405);
    assert_eq!(get.header("Allow"), Some("POST, DELETE"));
    assert_eq!(
        request(f.addr, "DELETE", "/mcp", &[], "").status,
        400,
        "DELETE without a session id"
    );
    assert_eq!(
        request(
            f.addr,
            "POST",
            "/other",
            &[
                ("Accept", JSON_AND_SSE),
                ("Content-Type", "application/json")
            ],
            LIST
        )
        .status,
        404
    );
    assert_eq!(
        request(
            f.addr,
            "POST",
            "/mcp?x=1",
            &[
                ("Accept", JSON_AND_SSE),
                ("Content-Type", "application/json")
            ],
            INIT
        )
        .status,
        200,
        "a query string is ignored"
    );

    assert_eq!(
        post(f.addr, &[("Host", "evil.example")], INIT).status,
        403,
        "DNS rebinding"
    );
    assert_eq!(
        post(f.addr, &[("Host", "localhost:9999")], INIT).status,
        200,
        "any port on an allowed host"
    );
    assert_eq!(post(f.addr, &[("Host", "[::1]:80")], INIT).status, 200);
    assert_eq!(
        post(f.addr, &[("Origin", "https://evil.example")], INIT).status,
        403,
        "cross-origin"
    );
    assert_eq!(
        post(f.addr, &[("Origin", "https://app.example")], INIT).status,
        200,
        "a listed origin"
    );

    let no_sse = request(
        f.addr,
        "POST",
        "/mcp",
        &[
            ("Accept", "application/json"),
            ("Content-Type", "application/json"),
        ],
        INIT,
    );
    assert_eq!(no_sse.status, 406);
    let wrong_type = request(
        f.addr,
        "POST",
        "/mcp",
        &[("Accept", JSON_AND_SSE), ("Content-Type", "text/plain")],
        INIT,
    );
    assert_eq!(wrong_type.status, 415);

    let r = post(f.addr, &[], "{not json");
    assert_eq!((r.status, code(&r)), (400, -32700));
    let r = post(f.addr, &[], "42");
    assert_eq!((r.status, code(&r)), (400, -32600));
    let r = post(f.addr, &[], r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#);
    assert_eq!(
        (r.status, code(&r)),
        (400, -32600),
        "batches are not supported"
    );
}

#[test]
fn a_browser_origin_is_refused_by_default() {
    let f = start(fast());
    assert_eq!(
        post(f.addr, &[("Origin", "http://localhost:3000")], INIT).status,
        403
    );
    // And a custom host list can open the server up.
    let open = start(HttpConfig {
        allowed_hosts: Vec::new(),
        ..fast()
    });
    assert_eq!(
        post(open.addr, &[("Host", "anything.example")], INIT).status,
        200
    );
}

#[test]
fn a_quick_silent_call_is_plain_json() {
    let f = start(fast());
    let r = post(
        f.addr,
        &modern_headers("tools/call", "add"),
        &modern_call(1, "add", ""),
    );
    assert!(
        r.header("content-type")
            .unwrap()
            .starts_with("application/json"),
        "{}",
        r.head
    );
    assert!(r.header("transfer-encoding").is_none());
}

#[test]
fn progress_switches_the_reply_to_an_event_stream() {
    let f = start(fast());
    let call = modern_call(7, "progress", r#","progressToken":"tok""#);
    let r = post(f.addr, &modern_headers("tools/call", "progress"), &call);
    assert_eq!(r.status, 200);
    assert_eq!(r.header("content-type"), Some("text/event-stream"));
    let events = r.events();
    assert_eq!(
        events.len(),
        4,
        "three progress notifications and the answer: {}",
        r.body
    );
    for (i, e) in events[..3].iter().enumerate() {
        assert_eq!(
            e.get("method").and_then(Value::as_str),
            Some("notifications/progress")
        );
        assert_eq!(
            e.get("params")
                .unwrap()
                .get("progress")
                .and_then(Value::as_f64),
            Some((i + 1) as f64)
        );
        assert_eq!(
            e.get("params")
                .unwrap()
                .get("progressToken")
                .and_then(Value::as_str),
            Some("tok")
        );
    }
    let last = &events[3];
    assert_eq!(last.get("id").and_then(Value::as_i64), Some(7));
    assert!(last.get("result").is_some());
}

#[test]
fn a_slow_call_becomes_an_event_stream_with_keep_alives() {
    let f = start(fast());
    let r = post(
        f.addr,
        &modern_headers("tools/call", "slow"),
        &modern_call(1, "slow", ""),
    );
    assert_eq!(r.header("content-type"), Some("text/event-stream"));
    assert!(
        r.body.contains(": ping"),
        "kept alive while it ran: {:?}",
        r.body
    );
    let events = r.events();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0]
            .get("result")
            .unwrap()
            .get("content")
            .unwrap()
            .get_index(0)
            .unwrap()
            .get("text")
            .and_then(Value::as_str),
        Some("slow done")
    );
}

#[test]
fn hanging_up_cancels_the_call() {
    let f = start(fast());
    let mut stream = TcpStream::connect(f.addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let body = modern_call(1, "wait", "");
    let raw = format!(
        "POST /mcp HTTP/1.1\r\nHost: {}\r\nAccept: {JSON_AND_SSE}\r\nContent-Type: application/json\r\nMCP-Protocol-Version: 2026-07-28\r\nMcp-Method: tools/call\r\nMcp-Name: wait\r\nContent-Length: {}\r\n\r\n{body}",
        f.addr,
        body.len()
    );
    stream.write_all(raw.as_bytes()).unwrap();
    // Wait for the stream to start (its head), then hang up.
    let mut buf = [0u8; 256];
    assert!(
        stream.read(&mut buf).unwrap() > 0,
        "the event stream started"
    );
    drop(stream);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !f.cancelled.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        f.cancelled.load(Ordering::SeqCst),
        "the tool was never cancelled"
    );
}

#[test]
fn slow_calls_do_not_block_each_other() {
    let f = start(fast());
    let started = Instant::now();
    let a = std::thread::spawn({
        let addr = f.addr;
        move || {
            post(
                addr,
                &modern_headers("tools/call", "slow"),
                &modern_call(1, "slow", ""),
            )
        }
    });
    let b = std::thread::spawn({
        let addr = f.addr;
        move || {
            post(
                addr,
                &modern_headers("tools/call", "slow"),
                &modern_call(2, "slow", ""),
            )
        }
    });
    let (a, b) = (a.join().unwrap(), b.join().unwrap());
    assert!(a.body.contains("slow done") && b.body.contains("slow done"));
    assert!(
        started.elapsed() < Duration::from_millis(750),
        "ran in parallel, took {:?}",
        started.elapsed()
    );
}

#[test]
fn a_listener_that_hangs_up_deregisters_and_one_that_stays_hears_changes() {
    use rusty_mcp_proto::ResourceTemplate;
    use rusty_mcp_server::{ChangeBroadcaster, ChangeKinds};

    let changes = ChangeBroadcaster::new();
    let server = Arc::new(
        Server::builder("listen", "1")
            .resource_template(ResourceTemplate::new("mem://{x}", "m"), |_c, _v, r| {
                Ok(rusty_mcp_proto::ReadResourceResult {
                    contents: vec![rusty_mcp_proto::ResourceContents::Text {
                        uri: r.uri,
                        mime_type: None,
                        text: String::new(),
                        meta: None,
                    }],
                    ..Default::default()
                })
            })
            .notify_changes(&changes, ChangeKinds::all_resources())
            .build()
            .unwrap(),
    );
    let http = bind_http(
        server,
        "127.0.0.1:0".parse().unwrap(),
        fast(),
        Limits::default(),
    )
    .unwrap();
    let addr = http.local_addr().unwrap();
    let stop = http.shutdown_handle().unwrap();
    std::thread::spawn(move || http.run().unwrap());

    let body = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"subscriptions/listen","params":{{"notifications":{{"resourcesListChanged":true,"resourceSubscriptions":["mem://a"]}},{META}}}}}"#
    );
    let open = |body: &str| {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let raw = format!(
            "POST /mcp HTTP/1.1\r\nHost: {addr}\r\nAccept: {JSON_AND_SSE}\r\nContent-Type: application/json\r\nMCP-Protocol-Version: 2026-07-28\r\nMcp-Method: subscriptions/listen\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(raw.as_bytes()).unwrap();
        stream
    };
    let read_until = |stream: &mut TcpStream, needle: &str| {
        let mut seen = String::new();
        let mut buf = [0u8; 512];
        let deadline = Instant::now() + Duration::from_secs(5);
        while !seen.contains(needle) && Instant::now() < deadline {
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => seen.push_str(&String::from_utf8_lossy(&buf[..n])),
            }
        }
        seen
    };

    let mut stays = open(&body);
    let acked = read_until(&mut stays, "subscriptions/acknowledged");
    assert!(
        acked.contains("text/event-stream") && acked.contains("subscriptionId"),
        "{acked}"
    );
    let mut leaves = open(&body.replace(r#""id":1"#, r#""id":2"#));
    read_until(&mut leaves, "subscriptions/acknowledged");
    assert_eq!(changes.listeners(), 2);

    drop(leaves);
    let deadline = Instant::now() + Duration::from_secs(5);
    while changes.listeners() > 1 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(changes.listeners(), 1, "the listener that hung up left");

    changes.resource_updated("mem://a");
    let heard = read_until(&mut stays, "notifications/resources/updated");
    assert!(
        heard.contains("mem://a"),
        "the one that stayed hears the change: {heard}"
    );
    stop.shutdown();
}

const CLASSIC: (&str, &str) = ("MCP-Protocol-Version", "2025-06-18");

fn open_session(f: &Fixture) -> String {
    post(f.addr, &[], INIT)
        .header("Mcp-Session-Id")
        .unwrap()
        .to_owned()
}

#[test]
fn a_session_id_is_issued_only_for_a_classic_initialize_and_only_when_enabled() {
    let f = start(fast());
    assert_ne!(open_session(&f), open_session(&f), "ids are unique");
    let discover = post(
        f.addr,
        &modern_headers("server/discover", ""),
        &format!(r#"{{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{{{META}}}}}"#),
    );
    assert!(discover.header("Mcp-Session-Id").is_none(), "stateless");
    let off = start(HttpConfig {
        max_sessions: 0,
        ..fast()
    });
    let init = post(off.addr, &[], INIT);
    assert_eq!(init.status, 200);
    assert!(init.header("Mcp-Session-Id").is_none());
}

#[test]
fn an_unknown_or_ended_session_is_404_and_delete_ends_it() {
    let f = start(fast());
    let bogus = post(f.addr, &[CLASSIC, ("Mcp-Session-Id", "nope")], LIST);
    assert_eq!((bogus.status, code(&bogus)), (404, -32600));

    let id = open_session(&f);
    let ok = post(f.addr, &[CLASSIC, ("Mcp-Session-Id", &id)], LIST);
    assert_eq!(ok.status, 200);
    let end = request(f.addr, "DELETE", "/mcp", &[("Mcp-Session-Id", &id)], "");
    assert_eq!(end.status, 200);
    let after = post(f.addr, &[CLASSIC, ("Mcp-Session-Id", &id)], LIST);
    assert_eq!(after.status, 404, "ended");
    let again = request(f.addr, "DELETE", "/mcp", &[("Mcp-Session-Id", &id)], "");
    assert_eq!(again.status, 404);
}

#[test]
fn a_cancelled_notification_on_the_session_stops_the_running_call() {
    let f = start(fast());
    let id = open_session(&f);
    let addr = f.addr;
    let session = id.clone();
    let call = std::thread::spawn(move || {
        let body = r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"wait"}}"#;
        post(addr, &[CLASSIC, ("Mcp-Session-Id", &session)], body)
    });
    // Let the call get going, then cancel it from another connection.
    std::thread::sleep(Duration::from_millis(300));
    let note = r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":7}}"#;
    let ack = post(f.addr, &[CLASSIC, ("Mcp-Session-Id", &id)], note);
    assert_eq!(ack.status, 202);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !f.cancelled.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(f.cancelled.load(Ordering::SeqCst), "never cancelled");
    let reply = call.join().unwrap();
    assert!(
        !reply.body.contains("stopped"),
        "a cancelled call gets no answer: {:?}",
        reply.body
    );
}

#[test]
fn a_cancel_cannot_reach_another_sessions_call() {
    let f = start(fast());
    let (mine, theirs) = (open_session(&f), open_session(&f));
    let addr = f.addr;
    let session = theirs.clone();
    let call = std::thread::spawn(move || {
        let body = r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"wait"}}"#;
        post(addr, &[CLASSIC, ("Mcp-Session-Id", &session)], body)
    });
    std::thread::sleep(Duration::from_millis(300));
    let note = r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":7}}"#;
    assert_eq!(
        post(f.addr, &[CLASSIC, ("Mcp-Session-Id", &mine)], note).status,
        202
    );
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !f.cancelled.load(Ordering::SeqCst),
        "wrong session cancelled it"
    );
    // Ending the owner's session does cancel it.
    request(f.addr, "DELETE", "/mcp", &[("Mcp-Session-Id", &theirs)], "");
    call.join().unwrap();
    assert!(f.cancelled.load(Ordering::SeqCst));
}

#[test]
fn sessions_are_capped_and_idle_ones_expire() {
    let f = start(HttpConfig {
        max_sessions: 1,
        session_idle: Duration::from_millis(200),
        ..fast()
    });
    let first = open_session(&f);
    let full = post(f.addr, &[], INIT);
    assert_eq!((full.status, code(&full)), (503, -32603));
    std::thread::sleep(Duration::from_millis(300));
    let gone = post(f.addr, &[CLASSIC, ("Mcp-Session-Id", &first)], LIST);
    assert_eq!(gone.status, 404, "idle too long");
    assert_eq!(post(f.addr, &[], INIT).status, 200, "room again");
}
