//! HTTP/2 over TLS (feature `http2`): the same router answering streams on
//! a connection that negotiated ALPN `h2`, next to HTTP/1.1 on the same port.

#![cfg(feature = "http2")]
#![allow(clippy::unwrap_used)]

use std::convert::Infallible;
use std::sync::Mutex;
use std::time::Duration;

use futures_core::Stream;
use rcgen::{CertificateParams, KeyPair};
use rusty_axum::response::sse::{Event, Sse};
use rusty_axum::routing::{get, post};
use rusty_axum::{serve, ConnectInfo, IntoResponse, Router, ServeOptions, Server};
use rusty_http::async_tokio::AsyncTransport;
use rusty_http::body::response_framing;
use rusty_http::head::RequestHead;
use rusty_http::{HeaderMap, Method, Version};
use rusty_hyper::{connect, Body, ClientHandle, ConnectOptions, HeaderField};
use rusty_tls::{AsyncTlsStream, TlsAcceptor, TrustPolicy};
use rusty_tokio::io::{AsyncWriteExt, TcpListener, TcpStream};
use rusty_tokio::sync::oneshot;
use rusty_tokio::time::timeout;

const FIVE: Duration = Duration::from_secs(5);

/// Told when the `/hang` handler starts, and when it is dropped.
static STARTED: Mutex<Option<oneshot::Sender<()>>> = Mutex::new(None);
static DROPPED: Mutex<Option<oneshot::Sender<()>>> = Mutex::new(None);
/// Keeps `/hang` waiting on a channel that never fires but stays alive, so
/// only the server can drop it: a task waiting on nothing at all is dropped
/// by the runtime as unreachable.
static HOLD: Mutex<Vec<oneshot::Sender<()>>> = Mutex::new(Vec::new());

struct Guard(Option<oneshot::Sender<()>>);

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}

/// Events that never come: the stream stays open until shutdown.
struct Forever;

impl Stream for Forever {
    type Item = Result<Event, Infallible>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Pending
    }
}

struct Two(u8);

impl Stream for Two {
    type Item = Result<Event, Infallible>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        if self.0 == 2 {
            return std::task::Poll::Ready(None);
        }
        self.0 += 1;
        std::task::Poll::Ready(Some(Ok(Event::default().data(self.0.to_string()))))
    }
}

fn app() -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/echo", post(|body: String| async move { body }))
        .route("/two", get(|| async { Sse::new(Two(0)) }))
        .route("/forever", get(|| async { Sse::new(Forever) }))
        .route(
            "/peer",
            get(|ConnectInfo(peer)| async move { peer.to_string() }),
        )
        .route(
            "/host",
            get(|req: rusty_axum::Request| async move {
                req.headers().get("host").unwrap_or("").to_string()
            }),
        )
        .route("/hang", get(hang))
}

async fn hang() -> impl IntoResponse {
    let _guard = Guard(DROPPED.lock().unwrap().take());
    let (hold, wait) = oneshot::channel::<()>();
    HOLD.lock().unwrap().push(hold);
    drop(Guard(STARTED.lock().unwrap().take()));
    let _ = wait.await;
    "unreachable"
}

struct Tls {
    acceptor: TlsAcceptor,
    trust: TrustPolicy,
}

fn tls() -> Tls {
    let key = KeyPair::generate().unwrap();
    let names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let cert = CertificateParams::new(names)
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let alpn = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let acceptor =
        TlsAcceptor::new_with_alpn(vec![cert.der().to_vec()], key.serialize_der(), alpn).unwrap();
    let trust = TrustPolicy::PinnedAnchors(vec![rustls::pki_types::CertificateDer::from(
        cert.der().to_vec(),
    )]);
    Tls { acceptor, trust }
}

fn start(options: ServeOptions) -> (Server, TrustPolicy) {
    let Tls { acceptor, trust } = tls();
    let listener =
        TcpListener::from_std(std::net::TcpListener::bind("127.0.0.1:0").unwrap()).unwrap();
    let options = ServeOptions {
        http2: true,
        ..options
    };
    (
        serve(listener, Some(acceptor), options, app()).unwrap(),
        trust,
    )
}

async fn h2(server: &Server, trust: &TrustPolicy) -> ClientHandle {
    let options = ConnectOptions {
        trust: trust.clone(),
        ..ConnectOptions::default()
    };
    let url = format!("https://{}", server.local_addr());
    connect(&url, options).await.unwrap().client().clone()
}

/// Sends a request and reads the whole response: (status, headers, body).
async fn call(
    h: &ClientHandle,
    method: &str,
    path: &str,
    body: &[u8],
) -> (u16, Vec<HeaderField>, Vec<u8>) {
    let fields = vec![
        HeaderField::new(":method", method),
        HeaderField::new(":scheme", "https"),
        HeaderField::new(":path", path),
        HeaderField::new(":authority", "example.test"),
    ];
    let (mut send, resp) = h.send_request(fields, body.is_empty()).await.unwrap();
    if !body.is_empty() {
        send.send_data(body.to_vec(), true).await.unwrap();
    }
    let mut resp = timeout(FIVE, resp).await.unwrap().unwrap();
    let mut bytes = Vec::new();
    while let Some(piece) = timeout(FIVE, resp.body.next()).await.unwrap() {
        if let Body::Data(d) = piece.unwrap() {
            bytes.extend(d);
        }
    }
    let status = header(&resp.headers, ":status").unwrap().parse().unwrap();
    (status, resp.headers, bytes)
}

fn header(fields: &[HeaderField], name: &str) -> Option<String> {
    fields
        .iter()
        .find(|f| f.name == name.as_bytes())
        .map(|f| String::from_utf8(f.value.clone()).unwrap())
}

#[rusty_tokio::test]
async fn h2_and_http_1_1_share_the_tls_port() {
    let (server, trust) = start(ServeOptions::default());
    let h = h2(&server, &trust).await;
    let (status, headers, body) = call(&h, "GET", "/health", b"").await;
    assert_eq!((status, body.as_slice()), (200, &b"ok"[..]));
    assert_eq!(header(&headers, "content-length").as_deref(), Some("2"));
    let (_, _, body) = call(&h, "POST", "/echo", b"over h2").await;
    assert_eq!(body, b"over h2");

    // A client that offers no ALPN gets HTTP/1.1 on the same port.
    let tcp = TcpStream::connect_addr(server.local_addr()).await.unwrap();
    let mut conn = AsyncTransport::new(AsyncTlsStream::new(tcp, "localhost", &trust).unwrap());
    let mut headers = HeaderMap::new();
    headers.insert("host", "localhost").unwrap();
    let head = RequestHead {
        method: Method::Get,
        target: "/health".to_string(),
        version: Version::Http11,
        headers,
    };
    conn.write_request_head(&head).await.unwrap();
    conn.get_mut().flush().await.unwrap();
    let head = timeout(FIVE, conn.read_response_head(16 * 1024))
        .await
        .unwrap()
        .unwrap();
    let framing = response_framing(&head.headers, &Method::Get, head.status).unwrap();
    assert_eq!(conn.read_body(framing).await.unwrap(), b"ok");
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn handlers_see_the_peer_and_authority_as_host() {
    let (server, trust) = start(ServeOptions::default());
    let h = h2(&server, &trust).await;
    let (_, _, peer) = call(&h, "GET", "/peer", b"").await;
    assert!(String::from_utf8(peer).unwrap().starts_with("127.0.0.1:"));
    let (_, _, host) = call(&h, "GET", "/host", b"").await;
    assert_eq!(host, b"example.test");
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn head_keeps_the_length_and_sends_no_body() {
    let (server, trust) = start(ServeOptions::default());
    let h = h2(&server, &trust).await;
    let (status, headers, body) = call(&h, "HEAD", "/health", b"").await;
    assert_eq!(status, 200);
    assert_eq!(header(&headers, "content-length").as_deref(), Some("2"));
    assert!(body.is_empty());
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_body_over_the_limit_is_refused() {
    let options = ServeOptions {
        max_body_bytes: 4,
        ..ServeOptions::default()
    };
    let (server, trust) = start(options);
    let h = h2(&server, &trust).await;
    let (status, _, _) = call(&h, "POST", "/echo", b"too long").await;
    assert_eq!(status, 413);
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_streamed_body_arrives_and_ends() {
    let (server, trust) = start(ServeOptions::default());
    let h = h2(&server, &trust).await;
    let (status, headers, body) = call(&h, "GET", "/two", b"").await;
    assert_eq!(status, 200);
    assert_eq!(header(&headers, "content-length"), None);
    assert_eq!(body, b"data: 1\n\ndata: 2\n\n");
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn shutdown_ends_a_stream_cleanly() {
    let (server, trust) = start(ServeOptions::default());
    let h = h2(&server, &trust).await;
    let fields = vec![
        HeaderField::new(":method", "GET"),
        HeaderField::new(":scheme", "https"),
        HeaderField::new(":path", "/forever"),
        HeaderField::new(":authority", "localhost"),
    ];
    let (_send, resp) = h.send_request(fields, true).await.unwrap();
    let mut resp = timeout(FIVE, resp).await.unwrap().unwrap();
    let stopping = rusty_tokio::spawn(server.shutdown());
    let end = timeout(FIVE, resp.body.next()).await.unwrap();
    assert!(end.is_none(), "the body ends without an error: {end:?}");
    timeout(FIVE, stopping).await.unwrap().unwrap();
}

#[rusty_tokio::test]
async fn a_client_reset_drops_the_handler() {
    let (started_tx, started) = oneshot::channel();
    let (dropped_tx, dropped) = oneshot::channel();
    *STARTED.lock().unwrap() = Some(started_tx);
    *DROPPED.lock().unwrap() = Some(dropped_tx);
    let (server, trust) = start(ServeOptions::default());
    let h = h2(&server, &trust).await;
    let fields = vec![
        HeaderField::new(":method", "GET"),
        HeaderField::new(":scheme", "https"),
        HeaderField::new(":path", "/hang"),
        HeaderField::new(":authority", "localhost"),
    ];
    let (mut send, _resp) = h.send_request(fields, true).await.unwrap();
    timeout(FIVE, started).await.unwrap().unwrap();
    send.reset(rusty_h2::error::ErrorCode::Cancel);
    timeout(FIVE, dropped).await.unwrap().unwrap();
    server.shutdown().await;
}
