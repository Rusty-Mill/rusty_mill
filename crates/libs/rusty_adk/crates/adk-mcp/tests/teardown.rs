#![allow(clippy::unwrap_used)]
//! A stdio session whose peer never closes its input still ends: when the
//! runtime serving it is dropped, and when the task serving it is aborted.
//! Clean end of input is covered by `versions.rs`.

use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::task::{Context, Poll};
use std::time::Duration;

use adk_core::Schema;
use adk_mcp::{serve_stream, serve_tools};
use adk_tools::FunctionTool;
use rusty_tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader, DuplexStream};
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

/// A peer that never reads: every write stays pending. Records its drop.
struct StalledOutput(Arc<AtomicBool>);

impl AsyncWrite for StalledOutput {
    fn poll_write(self: Pin<&mut Self>, _: &mut Context<'_>, _: &[u8]) -> Poll<io::Result<usize>> {
        Poll::Pending
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Pending
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Pending
    }
}

impl Drop for StalledOutput {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[test]
fn aborting_the_server_task_ends_a_session_whose_peer_stopped_reading() {
    for runtime in runtimes() {
        within_deadline(move || {
            runtime.block_on(async {
                // Held only by the server: once the blocking loop is gone, so is it.
                let tool = FunctionTool::new("noop", "Does nothing.", Schema::object(), |_, _| {
                    Box::pin(async { Ok(json!({})) })
                })
                .shared();
                let tool_alive = Arc::downgrade(&tool);
                let output_dropped = Arc::new(AtomicBool::new(false));
                let (mut to_server, server_in) = rusty_tokio::io::duplex(1 << 16);
                let server_out = StalledOutput(Arc::clone(&output_dropped));
                let server = serve_tools("teardown", vec![tool]);
                let served = rusty_tokio::spawn(async move {
                    serve_stream(&server, server_in, server_out).await
                });
                // More replies than the output channel holds, so the blocking
                // loop ends up waiting to hand one to the stalled writer.
                for id in 0..64 {
                    let ping = json!({"jsonrpc": "2.0", "id": id, "method": "ping"});
                    to_server
                        .write_all(format!("{ping}\n").as_bytes())
                        .await
                        .unwrap();
                }
                rusty_tokio::time::sleep(Duration::from_millis(100)).await;
                served.abort();
                while tool_alive.strong_count() > 0 || !output_dropped.load(Ordering::Acquire) {
                    rusty_tokio::time::sleep(Duration::from_millis(10)).await;
                }
                // The peer's input is still open; nothing here read or closed
                // the server's output.
                drop(to_server);
            });
        });
    }
}
