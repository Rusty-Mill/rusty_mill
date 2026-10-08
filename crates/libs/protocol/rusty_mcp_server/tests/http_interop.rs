#![allow(clippy::unwrap_used)]
//! The Streamable HTTP transport against the `rmcp` HTTP client, in both
//! handshake modes: the same scenario as the stdio interop test, over real
//! sockets.

mod common;

use common::{exercise, Mode, Recorder};
use rmcp::model::ProtocolVersion;
use rmcp::service::ClientLifecycleMode;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{ClientServiceExt, ServiceExt};
use rusty_json::Value;
use rusty_mcp_proto::{CallToolResult, ContentBlock, ErrorCode, ErrorData, Tool};
use rusty_mcp_server::{bind_http, HttpConfig, Server};
use rusty_serve::Limits;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

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

/// The same tools as `examples/interop_server.rs`.
fn serve(saw_cancel: Arc<AtomicBool>) -> (String, rusty_serve::ShutdownHandle) {
    let server = Server::builder("interop", "0.1.0")
        .page_size(3)
        .tool(Tool::new("add", schema()), |_ctx, call| {
            let n = |k: &str| {
                call.arguments
                    .as_ref()
                    .and_then(|a| a.get(k))
                    .and_then(Value::as_i64)
            };
            match (n("a"), n("b")) {
                (Some(a), Some(b)) => Ok(text(&(a + b).to_string())),
                _ => Err(ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    "a and b are required",
                )),
            }
        })
        .tool(Tool::new("fail", schema()), |_ctx, _call| {
            Ok(CallToolResult {
                is_error: Some(true),
                ..text("it broke")
            })
        })
        .tool(Tool::new("progress", schema()), |ctx, _call| {
            for i in 1..=3 {
                ctx.progress(f64::from(i), Some(3.0), Some("step"));
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(text("finished"))
        })
        .tool(Tool::new("wait", schema()), move |ctx, _call| {
            // Bounded, so a call nobody can cancel does not outlive the test.
            let until = std::time::Instant::now() + Duration::from_secs(20);
            while !ctx.is_cancelled() && std::time::Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            saw_cancel.store(true, Ordering::SeqCst);
            Ok(text("stopped"))
        })
        .build()
        .unwrap();
    let config = HttpConfig {
        keep_alive: Duration::from_millis(50),
        ..HttpConfig::default()
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
    (url, stop)
}

async fn run(mode: Mode) {
    let saw_cancel = Arc::new(AtomicBool::new(false));
    let (url, stop) = serve(saw_cancel.clone());
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(url),
    );
    let recorder = Recorder::default();
    let client = match mode {
        Mode::Classic => recorder.clone().serve(transport).await.unwrap(),
        Mode::Stateless => recorder
            .clone()
            .serve_with_lifecycle(
                transport,
                ClientLifecycleMode::Discover {
                    preferred_versions: vec![ProtocolVersion::V_2026_07_28],
                },
            )
            .await
            .unwrap(),
    };
    // The classic handshake cancels with a `notifications/cancelled` POST,
    // which a sessionless server cannot match to a request running on another
    // connection; only the stateless mode, which hangs up, is observable.
    let observed = move || saw_cancel.load(Ordering::SeqCst);
    let cancel_check: Option<&dyn Fn() -> bool> = match mode {
        Mode::Classic => None,
        Mode::Stateless => Some(&observed),
    };
    exercise(mode, client, recorder, cancel_check).await;
    stop.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_rmcp_http_client_works_with_the_initialize_handshake() {
    run(Mode::Classic).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_rmcp_http_client_works_with_server_discover() {
    run(Mode::Stateless).await;
}
