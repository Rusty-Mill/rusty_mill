#![allow(clippy::unwrap_used)]
//! The async face of the client, on a tokio runtime (the executor the
//! consumers use), against `rusty_mcp_server`.

mod common;

use common::{builder, first_text, serve, with_tasks, Asker, SECS};
use rusty_mcp_client_native::json::Value;
use rusty_mcp_client_native::{
    AsyncClient, ClientConfig, ClientError, Handler, HttpConfig, HttpTransport, NoHandler, Recv,
    StdioTransport, Transport,
};
use rusty_mcp_server::{serve_lines, StdioConfig};
use std::io::BufReader;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::Duration;

fn stdio() -> StdioTransport {
    let (server_in, client_out) = std::io::pipe().unwrap();
    let (client_in, server_out) = std::io::pipe().unwrap();
    let server = builder().build().unwrap();
    std::thread::spawn(move || {
        let _ = serve_lines(
            Arc::new(server),
            BufReader::new(server_in),
            server_out,
            StdioConfig::default(),
        );
    });
    StdioTransport::new(client_in, client_out)
}

fn args(a: i64, b: i64) -> Option<Value> {
    let mut v = Value::object();
    v.insert("a", a);
    v.insert("b", b);
    Some(v)
}

async fn connect() -> AsyncClient<StdioTransport, NoHandler> {
    AsyncClient::connect(stdio(), ClientConfig::new("c", "1"), NoHandler, SECS)
        .await
        .unwrap()
}

#[tokio::test]
async fn calls_are_futures_and_the_handshake_facts_are_available() {
    let c = connect().await;
    assert_eq!(c.negotiated().unwrap().as_str(), "2026-07-28");
    assert_eq!(c.server_info().unwrap().name, "fixture");
    c.ping().await.unwrap();
    assert_eq!(c.list_tools().await.unwrap().len(), 7);
    assert_eq!(
        first_text(&c.call_tool("add", args(20, 22)).await.unwrap()),
        "42"
    );
    let err = c.call_tool("add", None).await.unwrap_err();
    assert!(matches!(err, ClientError::Rpc(e) if e.code.0 == -32602));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_tasks_share_one_client() {
    let c = Arc::new(connect().await);
    let handles: Vec<_> = (0..16)
        .map(|i| {
            let c = Arc::clone(&c);
            tokio::spawn(async move {
                let r = c.call_tool("add", args(i, 100)).await.unwrap();
                (i, first_text(&r))
            })
        })
        .collect();
    for h in handles {
        let (i, got) = h.await.unwrap();
        assert_eq!(got, (i + 100).to_string());
    }
}

#[tokio::test]
async fn a_dropped_future_does_not_harm_the_next_call() {
    let c = connect().await;
    let slow = c.call_tool("slow", None);
    drop(slow); // never awaited; the call still runs and its answer is discarded
    assert_eq!(
        first_text(&c.call_tool("add", args(1, 1)).await.unwrap()),
        "2"
    );
}

#[tokio::test]
async fn a_failed_handshake_is_an_error_not_a_hang() {
    let (server_in, client_out) = std::io::pipe().unwrap();
    let (client_in, server_out) = std::io::pipe().unwrap();
    drop((server_in, server_out));
    let t = StdioTransport::new(client_in, client_out);
    let err = AsyncClient::connect(t, ClientConfig::new("c", "1"), NoHandler, SECS)
        .await
        .err()
        .unwrap();
    assert!(
        matches!(err, ClientError::Closed | ClientError::Io(_)),
        "{err}"
    );
}

/// A transport that can be killed from outside.
struct Killable {
    inner: StdioTransport,
    dead: Arc<AtomicBool>,
}

impl Transport for Killable {
    fn send(&mut self, m: &rusty_mcp_client_native::proto::Message) -> std::io::Result<()> {
        if self.dead.load(Ordering::SeqCst) {
            return Err(std::io::ErrorKind::BrokenPipe.into());
        }
        self.inner.send(m)
    }

    fn recv(&mut self, timeout: Duration) -> std::io::Result<Recv> {
        if self.dead.load(Ordering::SeqCst) {
            return Ok(Recv::Closed);
        }
        self.inner.recv(timeout)
    }
}

#[tokio::test]
async fn calls_after_the_connection_dies_fail_with_closed() {
    let dead = Arc::new(AtomicBool::new(false));
    let t = Killable {
        inner: stdio(),
        dead: Arc::clone(&dead),
    };
    let c = AsyncClient::connect(t, ClientConfig::new("c", "1"), NoHandler, SECS)
        .await
        .unwrap();
    c.ping().await.unwrap();
    dead.store(true, Ordering::SeqCst);
    // The idle worker notices and stops; new calls must not hang.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let err = tokio::time::timeout(Duration::from_secs(5), c.ping())
        .await
        .expect("the call hung")
        .unwrap_err();
    assert!(matches!(err, ClientError::Closed), "{err}");
}

#[tokio::test]
async fn questions_and_tasks_work_through_the_async_face() {
    let t = stdio();
    let c = AsyncClient::connect(
        t,
        with_tasks(),
        Asker {
            accept: true,
            asked: vec![],
        },
        SECS,
    )
    .await
    .unwrap();
    assert_eq!(
        first_text(&c.call_tool("confirm", None).await.unwrap()),
        "pending accepted"
    );
    assert_eq!(
        first_text(&c.call_tool("ask_job", None).await.unwrap()),
        "hello Ann"
    );
}

struct Forward(std::sync::mpsc::Sender<(String, Option<Value>)>);

impl Handler for Forward {
    fn notification(&mut self, method: &str, params: Option<&Value>) {
        let _ = self.0.send((method.to_owned(), params.cloned()));
    }
}

#[tokio::test]
async fn notifications_arrive_while_no_call_is_running() {
    let s = serve();
    let (tx, rx) = channel();
    let mut config = ClientConfig::new("c", "1");
    config.versions.retain(|v| !v.is_stateless());
    let c = AsyncClient::connect(
        HttpTransport::new(HttpConfig::new(&s.url)).unwrap(),
        config,
        Forward(tx),
        SECS,
    )
    .await
    .unwrap();
    let mut p = Value::object();
    p.insert("uri", "mem://a");
    c.call("resources/subscribe", Some(p)).await.unwrap();
    s.changes.resource_updated("mem://a");
    // No call is running: the idle worker must deliver it on its own.
    let got = tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(5)))
        .await
        .unwrap()
        .expect("no notification while idle");
    assert_eq!(got.0, "notifications/resources/updated");
    assert_eq!(got.1.unwrap()["uri"].as_str(), Some("mem://a"));
}
