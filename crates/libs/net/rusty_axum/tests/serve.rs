//! The server over real sockets: keep-alive, limits, streaming, shutdown
//! and TLS, read with `rusty_http`'s client side.

#![allow(clippy::unwrap_used)]

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::task::{ready, Context, Poll};
use std::time::Duration;

use futures_core::Stream;
use rcgen::{CertificateParams, KeyPair};
use rusty_axum::response::sse::{Event, Sse};
use rusty_axum::routing::{get, post};
use rusty_axum::{serve, IntoResponse, Router, ServeOptions, Server};
use rusty_http::async_tokio::AsyncTransport;
use rusty_http::body::{response_framing, Framing};
use rusty_http::head::{RequestHead, ResponseHead};
use rusty_http::{HeaderMap, Method, Version};
use rusty_tls::{AsyncTlsStream, TlsAcceptor, TrustPolicy};
use rusty_tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, TcpListener, TcpStream};
use rusty_tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use rusty_tokio::time::timeout;
use std::sync::Mutex;

type Events = UnboundedReceiver<Result<Event, Infallible>>;

/// Hands the next `/events` request the receiving end of a channel.
static EVENTS: Mutex<Option<Events>> = Mutex::new(None);
/// The same for `/events10`, the HTTP/1.0 test's own, so the two tests
/// cannot race for one channel.
static EVENTS_10: Mutex<Option<Events>> = Mutex::new(None);

fn app() -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/echo", post(|body: String| async move { body }))
        .route("/events", get(events))
        .route("/events10", get(events_10))
        .route("/finite", get(finite))
}

/// A short stream that ends on its own.
async fn finite() -> impl IntoResponse {
    let (tx, rx) = unbounded_channel();
    for word in ["one", "two"] {
        tx.send(Ok(Event::default().data(word))).unwrap();
    }
    drop(tx);
    Sse::new(Recv(Some(Box::pin(next(rx)))))
}

async fn events() -> impl IntoResponse {
    let rx = EVENTS.lock().unwrap().take().unwrap();
    Sse::new(Recv(Some(Box::pin(next(rx)))))
}

async fn events_10() -> impl IntoResponse {
    let rx = EVENTS_10.lock().unwrap().take().unwrap();
    Sse::new(Recv(Some(Box::pin(next(rx)))))
}

fn start(options: ServeOptions, tls: Option<TlsAcceptor>) -> Server {
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let listener = TcpListener::from_std(std_listener).unwrap();
    serve(listener, tls, options, app()).unwrap()
}

async fn connect(server: &Server) -> AsyncTransport<TcpStream> {
    AsyncTransport::new(TcpStream::connect_addr(server.local_addr()).await.unwrap())
}

async fn send<T: AsyncRead + AsyncWrite + Unpin + Send>(
    conn: &mut AsyncTransport<T>,
    method: Method,
    target: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) {
    let mut map = HeaderMap::new();
    map.insert("host", "localhost").unwrap();
    for (name, value) in headers {
        map.insert(name, value).unwrap();
    }
    if !body.is_empty() {
        map.insert("content-length", &body.len().to_string())
            .unwrap();
    }
    let head = RequestHead {
        method,
        target: target.to_string(),
        version: Version::Http11,
        headers: map,
    };
    conn.write_request_head(&head).await.unwrap();
    conn.write_body(body).await.unwrap();
}

async fn read<T: AsyncRead + AsyncWrite + Unpin + Send>(
    conn: &mut AsyncTransport<T>,
    method: &Method,
) -> (ResponseHead, String) {
    let head = conn.read_response_head(16 * 1024).await.unwrap();
    let framing = response_framing(&head.headers, method, head.status).unwrap();
    let body = conn.read_body(framing).await.unwrap();
    (head, String::from_utf8(body).unwrap())
}

async fn closed(tcp: TcpStream) -> bool {
    let mut buf = [0u8; 64];
    matches!(
        timeout(Duration::from_secs(5), tcp.read(&mut buf)).await,
        Ok(Ok(0)) | Ok(Err(_))
    )
}

#[rusty_tokio::test]
async fn keep_alive_serves_requests_in_order_on_one_connection() {
    let server = start(ServeOptions::default(), None);
    let mut conn = connect(&server).await;
    for _ in 0..3 {
        send(&mut conn, Method::Get, "/health", &[], b"").await;
        let (head, body) = read(&mut conn, &Method::Get).await;
        assert_eq!(head.status.as_u16(), 200);
        assert_eq!(head.headers.get("content-length"), Some("2"));
        assert_eq!(body, "ok");
    }
    send(&mut conn, Method::Post, "/echo", &[], b"hello").await;
    assert_eq!(read(&mut conn, &Method::Post).await.1, "hello");
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn head_gets_headers_without_a_body() {
    let server = start(ServeOptions::default(), None);
    let mut conn = connect(&server).await;
    send(&mut conn, Method::Head, "/health", &[], b"").await;
    let (head, body) = read(&mut conn, &Method::Head).await;
    assert_eq!(head.headers.get("content-length"), Some("2"));
    assert!(body.is_empty());
    // Still usable: the next response starts where expected.
    send(&mut conn, Method::Get, "/health", &[], b"").await;
    assert_eq!(read(&mut conn, &Method::Get).await.1, "ok");
}

/// HEAD of a streamed GET: no body and no length, since none is known,
/// and the connection still serves the next request correctly.
#[rusty_tokio::test]
async fn head_of_a_stream_has_no_length() {
    let server = start(ServeOptions::default(), None);
    let mut conn = connect(&server).await;
    send(&mut conn, Method::Head, "/finite", &[], b"").await;
    let (head, body) = read(&mut conn, &Method::Head).await;
    assert_eq!(head.status.as_u16(), 200);
    assert_eq!(head.headers.get("content-type"), Some("text/event-stream"));
    assert_eq!(head.headers.get("content-length"), None);
    assert_eq!(head.headers.get("transfer-encoding"), None);
    assert!(body.is_empty());
    send(&mut conn, Method::Get, "/finite", &[], b"").await;
    let (head, body) = read(&mut conn, &Method::Get).await;
    assert_eq!(head.headers.get("transfer-encoding"), Some("chunked"));
    assert_eq!(body, "data: one\n\ndata: two\n\n");
}

#[rusty_tokio::test]
async fn expect_continue_is_answered_before_the_body() {
    let server = start(ServeOptions::default(), None);
    let mut conn = connect(&server).await;
    let mut map = HeaderMap::new();
    map.insert("expect", "100-continue").unwrap();
    map.insert("content-length", "3").unwrap();
    let head = RequestHead {
        method: Method::Post,
        target: "/echo".into(),
        version: Version::Http11,
        headers: map,
    };
    conn.write_request_head(&head).await.unwrap();
    let interim = conn.read_response_head(1024).await.unwrap();
    assert_eq!(interim.status.as_u16(), 100);
    conn.write_body(b"abc").await.unwrap();
    assert_eq!(read(&mut conn, &Method::Post).await.1, "abc");
}

#[rusty_tokio::test]
async fn limits_answer_and_close() {
    let options = ServeOptions {
        max_body_bytes: 4,
        ..ServeOptions::default()
    };
    let server = start(options, None);
    let mut conn = connect(&server).await;
    send(&mut conn, Method::Post, "/echo", &[], b"too long").await;
    let (head, _) = read(&mut conn, &Method::Post).await;
    assert_eq!(head.status.as_u16(), 413);
    assert_eq!(head.headers.get("connection"), Some("close"));
    assert!(closed(conn.into_inner()).await);

    let tcp = TcpStream::connect_addr(server.local_addr()).await.unwrap();
    tcp.write_all(b"NOT A REQUEST\r\n\r\n").await.unwrap();
    let mut conn = AsyncTransport::new(tcp);
    let (head, _) = read(&mut conn, &Method::Get).await;
    assert_eq!(head.status.as_u16(), 400);
}

#[rusty_tokio::test]
async fn http_1_0_and_connection_close_end_the_connection() {
    let server = start(ServeOptions::default(), None);
    let mut conn = connect(&server).await;
    send(
        &mut conn,
        Method::Get,
        "/health",
        &[("connection", "close")],
        b"",
    )
    .await;
    let (head, _) = read(&mut conn, &Method::Get).await;
    assert_eq!(head.headers.get("connection"), Some("close"));
    assert!(closed(conn.into_inner()).await);

    let tcp = TcpStream::connect_addr(server.local_addr()).await.unwrap();
    tcp.write_all(b"GET /health HTTP/1.0\r\n\r\n")
        .await
        .unwrap();
    let mut conn = AsyncTransport::new(tcp);
    assert_eq!(read(&mut conn, &Method::Get).await.1, "ok");
    assert!(closed(conn.into_inner()).await);
}

#[rusty_tokio::test]
async fn idle_connections_are_closed() {
    let options = ServeOptions {
        idle_timeout: Duration::from_millis(100),
        ..ServeOptions::default()
    };
    let server = start(options, None);
    let tcp = TcpStream::connect_addr(server.local_addr()).await.unwrap();
    assert!(closed(tcp).await);
}

#[rusty_tokio::test]
async fn connections_past_the_cap_are_closed_unserved() {
    let options = ServeOptions {
        max_connections: 1,
        ..ServeOptions::default()
    };
    let server = start(options, None);
    let mut first = connect(&server).await;
    send(&mut first, Method::Get, "/health", &[], b"").await;
    assert_eq!(read(&mut first, &Method::Get).await.1, "ok");
    let second = TcpStream::connect_addr(server.local_addr()).await.unwrap();
    assert!(closed(second).await);
}

/// SSE is written as produced, and shutdown ends the stream cleanly.
/// The only test using `EVENTS`, so tests running in parallel cannot race.
#[rusty_tokio::test]
async fn sse_streams_and_shutdown_ends_it() {
    let (tx, rx): (UnboundedSender<_>, _) = unbounded_channel();
    *EVENTS.lock().unwrap() = Some(rx);
    let server = start(ServeOptions::default(), None);
    let mut conn = connect(&server).await;
    send(&mut conn, Method::Get, "/events", &[], b"").await;
    let head = conn.read_response_head(1024).await.unwrap();
    assert_eq!(head.headers.get("transfer-encoding"), Some("chunked"));
    assert_eq!(head.headers.get("content-type"), Some("text/event-stream"));
    let framing = response_framing(&head.headers, &Method::Get, head.status).unwrap();
    assert_eq!(framing, Framing::Chunked);
    let mut reader = conn.into_body_reader(framing);

    tx.send(Ok(Event::default().event("token").data("a")))
        .unwrap();
    let first = timeout(Duration::from_secs(5), reader.next_chunk())
        .await
        .unwrap();
    assert_eq!(first.unwrap().unwrap(), b"event: token\ndata: a\n\n");

    // The sender is still open: only shutdown can end this stream.
    timeout(Duration::from_secs(5), server.shutdown())
        .await
        .unwrap();
    let end = timeout(Duration::from_secs(5), reader.next_chunk())
        .await
        .unwrap();
    assert_eq!(
        end.unwrap(),
        None,
        "the body ends with its terminating chunk"
    );
    drop(tx);
}

/// HTTP/1.0 has no chunked coding: the stream goes raw and the closed
/// connection ends it, also when shutdown ends the stream.
#[rusty_tokio::test]
async fn http_1_0_streams_are_close_delimited() {
    let (tx, rx) = unbounded_channel();
    *EVENTS_10.lock().unwrap() = Some(rx);
    let server = start(ServeOptions::default(), None);
    let tcp = TcpStream::connect_addr(server.local_addr()).await.unwrap();
    tcp.write_all(b"GET /events10 HTTP/1.0\r\n\r\n")
        .await
        .unwrap();
    let mut conn = AsyncTransport::new(tcp);
    let head = conn.read_response_head(1024).await.unwrap();
    assert_eq!(head.headers.get("transfer-encoding"), None);
    assert_eq!(head.headers.get("content-length"), None);
    assert_eq!(head.headers.get("connection"), Some("close"));
    let framing = response_framing(&head.headers, &Method::Get, head.status).unwrap();
    assert_eq!(framing, Framing::Close);
    let mut reader = conn.into_body_reader(framing);

    tx.send(Ok(Event::default().data("a"))).unwrap();
    let first = timeout(Duration::from_secs(5), reader.next_chunk())
        .await
        .unwrap();
    assert_eq!(
        first.unwrap().unwrap(),
        b"data: a\n\n",
        "raw bytes, no chunk framing"
    );

    timeout(Duration::from_secs(5), server.shutdown())
        .await
        .unwrap();
    let end = timeout(Duration::from_secs(5), reader.next_chunk())
        .await
        .unwrap();
    assert_eq!(end.unwrap(), None, "the close ends the body");
    drop(tx);
}

#[rusty_tokio::test]
async fn serves_over_tls() {
    let key = KeyPair::generate().unwrap();
    let cert = CertificateParams::new(vec!["localhost".to_string()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let acceptor = TlsAcceptor::new(vec![cert.der().to_vec()], key.serialize_der()).unwrap();
    let server = start(ServeOptions::default(), Some(acceptor));
    let trust = TrustPolicy::PinnedAnchors(vec![rustls::pki_types::CertificateDer::from(
        cert.der().to_vec(),
    )]);
    let tcp = TcpStream::connect_addr(server.local_addr()).await.unwrap();
    let tls = AsyncTlsStream::new(tcp, "localhost", &trust).unwrap();
    let mut conn = AsyncTransport::new(tls);
    for _ in 0..2 {
        send(&mut conn, Method::Get, "/health", &[], b"").await;
        conn.get_mut().flush().await.unwrap();
        let (head, body) = timeout(Duration::from_secs(5), read(&mut conn, &Method::Get))
            .await
            .unwrap();
        assert_eq!(head.status.as_u16(), 200);
        assert_eq!(body, "ok");
    }
}

type Next = Pin<Box<dyn Future<Output = (Option<Result<Event, Infallible>>, Events)> + Send>>;

async fn next(mut rx: Events) -> (Option<Result<Event, Infallible>>, Events) {
    (rx.recv().await, rx)
}

/// A channel receiver as a `Stream`, for the test only.
struct Recv(Option<Next>);

impl Stream for Recv {
    type Item = Result<Event, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Some(fut) = self.0.as_mut() else {
            return Poll::Ready(None);
        };
        let (item, rx) = ready!(fut.as_mut().poll(cx));
        self.0 = item.is_some().then(|| Box::pin(next(rx)) as Next);
        Poll::Ready(item)
    }
}
