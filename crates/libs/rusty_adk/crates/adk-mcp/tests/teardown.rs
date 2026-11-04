#![allow(clippy::unwrap_used)]
//! A stdio session whose peer never closes its input still ends: when the
//! runtime serving it is dropped, and when the task serving it is aborted.
//! Clean end of input is covered by `versions.rs`.

use std::sync::mpsc;
use std::time::Duration;

use adk_mcp::{serve_stream, serve_tools};
use rusty_tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};
use rusty_tokio::Runtime;
use serde_json::json;

/// Runs `f` on its own thread and fails if it hasn't returned within ten
/// seconds, so a teardown deadlock fails the test instead of hanging CI.
fn within_deadline(f: impl FnOnce() + Send + 'static) {
    let (done, finished) = mpsc::channel();
    std::thread::spawn(move || {
        f();
        let _ = done.send(());
    });
    finished
        .recv_timeout(Duration::from_secs(10))
        .expect("the stdio session did not shut down");
}

fn runtimes() -> [Runtime; 2] {
    [
        Runtime::new().unwrap(),
        rusty_tokio::Builder::new_current_thread().build().unwrap(),
    ]
}

/// Starts a session and completes `initialize`, so the pumps and the
/// blocking loop are all live. Returns the server task and the client's
/// halves; the input half stays open for as long as the caller holds it.
async fn live_session() -> (
    rusty_tokio::JoinHandle<adk_core::Result<()>>,
    DuplexStream,
    BufReader<DuplexStream>,
) {
    let (mut to_server, server_in) = rusty_tokio::io::duplex(1 << 16);
    let (server_out, from_server) = rusty_tokio::io::duplex(1 << 16);
    let server = serve_tools("teardown", vec![]);
    let served =
        rusty_tokio::spawn(async move { serve_stream(&server, server_in, server_out).await });
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": {"name": "t", "version": "0"}}});
    to_server
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    let mut from_server = BufReader::new(from_server);
    let mut line = String::new();
    from_server.read_line(&mut line).await.unwrap();
    assert!(line.contains("\"result\""), "no initialize reply: {line}");
    (served, to_server, from_server)
}

#[test]
fn dropping_the_runtime_ends_a_session_whose_input_is_still_open() {
    for runtime in runtimes() {
        within_deadline(move || {
            let (_served, to_server, _from_server) = runtime.block_on(live_session());
            drop(runtime);
            drop(to_server);
        });
    }
}

#[test]
fn aborting_the_server_task_ends_the_session_whose_input_is_still_open() {
    for runtime in runtimes() {
        within_deadline(move || {
            runtime.block_on(async {
                let (served, _to_server, mut from_server) = live_session().await;
                served.abort();
                // The writer pump closes the output once the session ends.
                let mut rest = String::new();
                let read = from_server.read_line(&mut rest).await.unwrap();
                assert_eq!(read, 0, "unexpected output after abort: {rest}");
            });
        });
    }
}
