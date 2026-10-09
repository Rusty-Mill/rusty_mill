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
    stop: ShutdownHandle,
}

impl Drop for Up {
    fn drop(&mut self) {
        self.stop.shutdown();
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
    Up { addr, stop }
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
