#![allow(clippy::unwrap_used)]
//! The Streamable HTTP transport against the `rmcp` HTTP client, in both
//! handshake modes: the same scenario as the stdio interop test, over real
//! sockets.

mod common;
#[path = "common/fixture.rs"]
mod fixture;

use common::{exercise, Mode, Recorder};
use rmcp::model::ProtocolVersion;
use rmcp::service::ClientLifecycleMode;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{ClientServiceExt, ServiceExt};
use rusty_mcp_server::{bind_http, HttpConfig};
use rusty_serve::Limits;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// The same server as `examples/interop_server.rs`, over HTTP.
fn serve(saw_cancel: Arc<AtomicBool>) -> (String, rusty_serve::ShutdownHandle) {
    let server = fixture::interop_server(move || saw_cancel.store(true, Ordering::SeqCst));
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
    let observed = move || saw_cancel.load(Ordering::SeqCst);
    if matches!(mode, Mode::Classic) {
        common::classic_subscribe(&client, &recorder).await;
    }
    exercise(mode, client, recorder, &observed).await;
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
