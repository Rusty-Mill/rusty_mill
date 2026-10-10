#![allow(clippy::unwrap_used)]
//! Upstream connections against a real `rusty_mcp_server`: the revision
//! header on fallback, one budget for a whole operation, and one connection
//! for a whole paged listing.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agentgateway_config::{McpTarget, McpTargetKind, StreamableHttpTarget};
use agentgateway_mcp::{HeaderOverride, Override, Target};
use rusty_mcp_server::json::Value;
use rusty_mcp_server::proto::{
    CallToolParams, CallToolResult, ContentBlock, ErrorData, ProtocolVersion, Tool,
};
use rusty_mcp_server::{CallContext, HttpConfig, Server, ServerBuilder, ToolSource, bind_http};
use rusty_serve::{Limits, ShutdownHandle};

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

struct Up {
    addr: SocketAddr,
    stop: Option<ShutdownHandle>,
}

impl Drop for Up {
    fn drop(&mut self) {
        if let Some(stop) = &self.stop {
            stop.shutdown();
        }
    }
}

fn serve(builder: ServerBuilder) -> Up {
    let server = Arc::new(builder.build().unwrap());
    let http = bind_http(
        server,
        "127.0.0.1:0".parse().unwrap(),
        HttpConfig {
            sse_after: Duration::from_millis(50),
            ..HttpConfig::default()
        },
        Limits::default(),
    )
    .unwrap();
    let addr = http.local_addr().unwrap();
    let stop = http.shutdown_handle().unwrap();
    std::thread::spawn(move || http.run().unwrap());
    Up {
        addr,
        stop: Some(stop),
    }
}

fn target(up: &Up, connections: usize, timeout: Duration) -> Target {
    let config = McpTarget {
        name: "up".to_owned(),
        kind: McpTargetKind::Mcp(StreamableHttpTarget {
            host: "127.0.0.1".to_owned(),
            port: up.addr.port(),
            path: "/mcp".to_owned(),
        }),
        filters: Vec::new(),
    };
    Target::connect_with(
        &config,
        &Override::default(),
        Some(timeout),
        connections,
        "t",
    )
    .unwrap()
}

fn call(name: &str) -> CallToolParams {
    CallToolParams::new(name)
}

#[test]
fn a_classic_only_upstream_that_rejects_the_stateless_header_is_still_reached() {
    // Discovery names `2026-07-28` in a header this server refuses; the
    // fallback must stop sending it, or `initialize` is refused too.
    let up = serve(
        Server::builder("classic", "1")
            .versions(vec![ProtocolVersion::new("2025-11-25")])
            .tool(Tool::new("t", schema()), |_c, _p| Ok(text("classic"))),
    );
    let target = target(&up, 2, Duration::from_secs(5));
    let result = target.call(&call("t"), &HeaderOverride::default()).unwrap();
    assert!(matches!(&result.content[0], ContentBlock::Text { text, .. } if text == "classic"));
}

fn slow_tool(builder: ServerBuilder, ms: u64) -> ServerBuilder {
    builder.tool(Tool::new("slow", schema()), move |_c, _p| {
        std::thread::sleep(Duration::from_millis(ms));
        Ok(text("done"))
    })
}

#[test]
fn waiting_for_a_busy_connection_counts_against_the_same_budget_as_the_call() {
    let up = serve(slow_tool(Server::builder("slow", "1"), 700));
    let target = Arc::new(target(&up, 1, Duration::from_secs(1)));
    let first = {
        let target = Arc::clone(&target);
        std::thread::spawn(move || target.call(&call("slow"), &HeaderOverride::default()))
    };
    std::thread::sleep(Duration::from_millis(100)); // let it take the connection
    let started = Instant::now();
    let second = target.call(&call("slow"), &HeaderOverride::default());
    let took = started.elapsed();
    assert!(
        second.is_err(),
        "queued 600 ms plus a 700 ms call is over a 1 s budget"
    );
    assert!(
        took < Duration::from_millis(1_150),
        "the budget is absolute, not restarted after the wait: {took:?}"
    );
    assert!(first.join().unwrap().is_ok());
}

/// Eight one-tool pages, each slow to list; records the session of each
/// list request.
struct Many(Arc<Mutex<Vec<Option<String>>>>, u64);

impl ToolSource for Many {
    fn tools(&self) -> Vec<Tool> {
        Vec::new()
    }

    fn tools_for(&self, ctx: &CallContext) -> Result<Vec<Tool>, ErrorData> {
        self.0
            .lock()
            .unwrap()
            .push(ctx.caller().header("mcp-session-id").map(str::to_owned));
        std::thread::sleep(Duration::from_millis(self.1));
        Ok((0..8)
            .map(|i| Tool::new(format!("t{i}"), schema()))
            .collect())
    }

    fn call(
        &self,
        _ctx: &CallContext,
        _call: &CallToolParams,
    ) -> Option<Result<CallToolResult, ErrorData>> {
        Some(Ok(text("ok")))
    }
}

#[test]
fn a_paged_listing_has_one_budget_for_all_its_pages() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let up = serve(
        Server::builder("pages", "1")
            .page_size(1)
            .tool_source(Many(seen, 150)),
    );
    let target = target(&up, 2, Duration::from_millis(600));
    let started = Instant::now();
    let result = target.tools(&HeaderOverride::default());
    let took = started.elapsed();
    assert!(result.is_err(), "8 pages of 150 ms cannot fit in 600 ms");
    assert!(
        took < Duration::from_millis(900),
        "the listing stops at the budget instead of running every page: {took:?}"
    );
}

#[test]
fn every_page_of_a_listing_goes_over_one_session_even_with_competing_traffic() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let up = serve(
        Server::builder("pages", "1")
            .versions(vec![ProtocolVersion::new("2025-11-25")])
            .page_size(1)
            .tool_source(Many(Arc::clone(&seen), 20)),
    );
    let target = Arc::new(target(&up, 2, Duration::from_secs(20)));
    // Competing calls keep taking and returning connections.
    let stop = Arc::new(AtomicBool::new(false));
    let noise = {
        let (target, stop) = (Arc::clone(&target), Arc::clone(&stop));
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let _ = target.call(&call("t0"), &HeaderOverride::default());
            }
        })
    };
    std::thread::sleep(Duration::from_millis(100));
    seen.lock().unwrap().clear();
    let tools = target.tools(&HeaderOverride::default()).unwrap();
    stop.store(true, Ordering::SeqCst);
    noise.join().unwrap();
    assert_eq!(tools.len(), 8);
    // The source is asked on every list request, so these are the pages.
    let ids = seen.lock().unwrap().clone();
    assert_eq!(ids.len(), 8, "one request per page: {ids:?}");
    assert!(ids[0].is_some(), "a classic session: {ids:?}");
    assert!(
        ids.iter().all(|id| *id == ids[0]),
        "pages hopped between sessions: {ids:?}"
    );
}

#[test]
fn a_paged_listing_takes_one_connection_for_all_its_pages() {
    // A cursor belongs to the session that issued it, so the pages must not be
    // spread over leases: one lease for the listing, however many pages.
    let up = serve(
        Server::builder("pages", "1")
            .page_size(1)
            .tool_source(Many(Arc::new(Mutex::new(Vec::new())), 0)),
    );
    let target = target(&up, 2, Duration::from_secs(5));
    let before = target.leases();
    assert_eq!(target.tools(&HeaderOverride::default()).unwrap().len(), 8);
    assert_eq!(target.leases() - before, 1);
}

/// A bare HTTP upstream with no discovery whose handshakes after the first
/// are slow: 600 ms to refuse the discovery, 600 ms to initialize. Every
/// request is its own connection.
fn sluggish_classic_upstream(call_ms: u64) -> Up {
    use std::io::{Read, Write};
    use std::sync::atomic::AtomicUsize;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handshakes = Arc::new(AtomicUsize::new(0));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let handshakes = Arc::clone(&handshakes);
            std::thread::spawn(move || {
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                let _ = stream.set_read_timeout(Some(Duration::from_millis(300)));
                while let Ok(n @ 1..) = stream.read(&mut buf) {
                    raw.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&raw);
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let want = head
                            .to_ascii_lowercase()
                            .split("content-length:")
                            .nth(1)
                            .and_then(|r| r.split("\r\n").next())
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if body.len() >= want {
                            break;
                        }
                    }
                }
                let text = String::from_utf8_lossy(&raw).into_owned();
                let slow = handshakes.load(Ordering::SeqCst) >= 1;
                let id: String = text
                    .split("\"id\":")
                    .nth(1)
                    .map(|r| r.chars().take_while(char::is_ascii_digit).collect())
                    .unwrap_or_default();
                let (status, body) = if text.contains("server/discover") {
                    if slow {
                        std::thread::sleep(Duration::from_millis(600));
                    }
                    (
                        "200 OK",
                        format!(
                            r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32601,"message":"no discovery"}}}}"#
                        ),
                    )
                } else if text.contains("\"initialize\"") {
                    if slow {
                        std::thread::sleep(Duration::from_millis(600));
                    }
                    handshakes.fetch_add(1, Ordering::SeqCst);
                    (
                        "200 OK",
                        format!(
                            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"protocolVersion":"2025-11-25","capabilities":{{}},"serverInfo":{{"name":"s","version":"1"}}}}}}"#
                        ),
                    )
                } else if text.contains("tools/call") {
                    std::thread::sleep(Duration::from_millis(call_ms));
                    (
                        "200 OK",
                        format!(
                            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"content":[{{"type":"text","text":"ok"}}]}}}}"#
                        ),
                    )
                } else {
                    ("202 Accepted", String::new())
                };
                let reply = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
            });
        }
    });
    Up { addr, stop: None }
}

#[test]
fn a_lazily_opened_connection_handshakes_within_the_operation_budget() {
    // The first connection is dialled at once and is quick. It is busy for
    // 500 ms with a call, so the next call opens a second one, whose
    // handshake needs 600 ms to learn there is no discovery and 600 ms more to
    // initialize: over a 1 s budget in all, which must end at the budget.
    let up = sluggish_classic_upstream(500);
    let target = Arc::new(target(&up, 2, Duration::from_secs(1)));
    let busy = {
        let target = Arc::clone(&target);
        std::thread::spawn(move || target.call(&call("t"), &HeaderOverride::default()))
    };
    std::thread::sleep(Duration::from_millis(100));
    let started = Instant::now();
    let second = target.call(&call("t"), &HeaderOverride::default());
    let took = started.elapsed();
    assert!(second.is_err(), "1.2 s of handshake cannot fit in 1 s");
    assert!(
        took < Duration::from_millis(1_100),
        "the handshake must not get a fresh budget per step: {took:?}"
    );
    assert!(busy.join().unwrap().is_ok());
}
