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
use rusty_axum::response::sse::{Event, KeepAlive, Sse};
use rusty_axum::routing::{get, post};
use rusty_axum::{
    serve, serve_with, Accept, Accepted, Admission, BoxFuture, ConnectInfo, IntoResponse, Router,
    ServeOptions, Server,
};
use rusty_http::async_tokio::AsyncTransport;
use rusty_http::body::{response_framing, Framing};
use rusty_http::head::{RequestHead, ResponseHead};
use rusty_http::{HeaderMap, Method, Version};
use rusty_tls::{AsyncTlsStream, TlsAcceptor, TrustPolicy};
use rusty_tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, TcpListener, TcpStream};
use rusty_tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use rusty_tokio::time::timeout;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

type Events = UnboundedReceiver<Result<Event, Infallible>>;

/// Hands the next `/events` request the receiving end of a channel.
static EVENTS: Mutex<Option<Events>> = Mutex::new(None);
/// The same for `/events10`, the HTTP/1.0 test's own, so the two tests
/// cannot race for one channel.
static EVENTS_10: Mutex<Option<Events>> = Mutex::new(None);
/// The same for `/quiet`, the keep-alive test's own.
static QUIET: Mutex<Option<Events>> = Mutex::new(None);

fn app() -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/echo", post(|body: String| async move { body }))
        .route("/events", get(events))
        .route("/events10", get(events_10))
        .route("/finite", get(finite))
        .route("/quiet", get(quiet))
        .route(
            "/peer",
            get(|ConnectInfo(peer)| async move { peer.to_string() }),
        )
}

async fn quiet() -> impl IntoResponse {
    let rx = QUIET.lock().unwrap().take().unwrap();
    let keep_alive = KeepAlive::new()
        .interval(Duration::from_millis(50))
        .text("ping");
    Sse::new(Recv(Some(Box::pin(next(rx))))).keep_alive(keep_alive)
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

#[rusty_tokio::test]
async fn handlers_see_the_peer_address() {
    let server = start(ServeOptions::default(), None);
    let tcp = TcpStream::connect_addr(server.local_addr()).await.unwrap();
    let local = tcp.local_addr().unwrap();
    let mut conn = AsyncTransport::new(tcp);
    send(&mut conn, Method::Get, "/peer", &[], b"").await;
    assert_eq!(read(&mut conn, &Method::Get).await.1, local.to_string());
}

/// A quiet stream gets the keep-alive comment; an event resets the wait.
#[rusty_tokio::test]
async fn sse_keep_alive_fills_quiet_time() {
    let (tx, rx) = unbounded_channel();
    *QUIET.lock().unwrap() = Some(rx);
    let server = start(ServeOptions::default(), None);
    let mut conn = connect(&server).await;
    send(&mut conn, Method::Get, "/quiet", &[], b"").await;
    let head = conn.read_response_head(1024).await.unwrap();
    let framing = response_framing(&head.headers, &Method::Get, head.status).unwrap();
    let mut reader = conn.into_body_reader(framing);
    macro_rules! chunk {
        () => {
            timeout(Duration::from_secs(5), reader.next_chunk())
                .await
                .unwrap()
                .unwrap()
                .unwrap()
        };
    }
    assert_eq!(chunk!(), b":ping\n\n");
    tx.send(Ok(Event::default().data("a"))).unwrap();
    let mut seen = chunk!();
    while seen == b":ping\n\n" {
        seen = chunk!();
    }
    assert_eq!(seen, b"data: a\n\n");
    assert_eq!(chunk!(), b":ping\n\n");
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

fn certificate() -> (TlsAcceptor, TrustPolicy) {
    let key = KeyPair::generate().unwrap();
    let cert = CertificateParams::new(vec!["localhost".to_string()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let acceptor = TlsAcceptor::new(vec![cert.der().to_vec()], key.serialize_der()).unwrap();
    let trust = TrustPolicy::PinnedAnchors(vec![rustls::pki_types::CertificateDer::from(
        cert.der().to_vec(),
    )]);
    (acceptor, trust)
}

fn start_with(options: ServeOptions, accept: impl Accept) -> Server {
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let listener = TcpListener::from_std(std_listener).unwrap();
    serve_with(listener, accept, options, app()).unwrap()
}

/// Hands the `n`th connection the `n`th acceptor, round robin: a stand-in
/// for a certificate chosen by SNI.
struct RoundRobin(Vec<TlsAcceptor>, AtomicUsize);

impl Accept for RoundRobin {
    fn accept(&self, tcp: TcpStream, _peer: SocketAddr) -> BoxFuture<'static, Accepted> {
        let n = self.1.fetch_add(1, Ordering::SeqCst);
        let acceptor = self.0[n % self.0.len()].clone();
        Box::pin(async move {
            let mut tls = acceptor.accept_async(tcp).unwrap();
            match tls.complete_handshake().await {
                Ok(()) => Accepted::Tls(Box::new(tls)),
                Err(_) => Accepted::Done,
            }
        })
    }
}

async fn get_health_over_tls(server: &Server, trust: &TrustPolicy) -> u16 {
    let tcp = TcpStream::connect_addr(server.local_addr()).await.unwrap();
    let tls = AsyncTlsStream::new(tcp, "localhost", trust).unwrap();
    let mut conn = AsyncTransport::new(tls);
    send(&mut conn, Method::Get, "/health", &[], b"").await;
    conn.get_mut().flush().await.unwrap();
    let (head, body) = timeout(Duration::from_secs(5), read(&mut conn, &Method::Get))
        .await
        .unwrap();
    assert_eq!(body, "ok");
    head.status.as_u16()
}

#[rusty_tokio::test]
async fn an_acceptor_chooses_the_certificate_per_connection() {
    let (first, first_trust) = certificate();
    let (second, second_trust) = certificate();
    let server = start_with(
        ServeOptions::default(),
        RoundRobin(vec![first, second], AtomicUsize::new(0)),
    );
    // Each client trusts only the certificate its connection is handed.
    assert_eq!(get_health_over_tls(&server, &first_trust).await, 200);
    assert_eq!(get_health_over_tls(&server, &second_trust).await, 200);
}

/// The request the takeover test sends.
const TAKEN_REQUEST: &[u8] = b"GET /health HTTP/1.1\r\nhost: localhost\r\n\r\n";

/// Takes every connection over: answers it with raw bytes on its own task
/// and reports how that went.
struct TakeOver(UnboundedSender<std::io::Result<()>>);

impl Accept for TakeOver {
    fn accept(&self, tcp: TcpStream, _peer: SocketAddr) -> BoxFuture<'static, Accepted> {
        let done = self.0.clone();
        rusty_tokio::spawn(async move {
            let _ = done.send(take_over(tcp).await);
        });
        Box::pin(async { Accepted::Done })
    }
}

/// Reads the whole request before answering and closes with a FIN: a
/// socket closed with unread input may be reset, which can discard the
/// answer before the client reads it.
async fn take_over(mut tcp: TcpStream) -> std::io::Result<()> {
    let mut request = vec![0u8; TAKEN_REQUEST.len()];
    tcp.read_exact(&mut request).await?;
    if request != TAKEN_REQUEST {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("unexpected request {request:?}"),
        ));
    }
    tcp.write_all(b"taken").await?;
    tcp.shutdown().await
}

#[rusty_tokio::test]
async fn a_connection_taken_over_is_not_served_as_http() {
    let (done, mut taken) = unbounded_channel();
    let server = start_with(ServeOptions::default(), TakeOver(done));
    let tcp = TcpStream::connect_addr(server.local_addr()).await.unwrap();
    tcp.write_all(TAKEN_REQUEST).await.unwrap();
    let mut seen = Vec::new();
    let mut buf = [0u8; 64];
    loop {
        let n = timeout(Duration::from_secs(5), tcp.read(&mut buf))
            .await
            .expect("the answer or its end within the bound")
            .expect("reading the answer");
        if n == 0 {
            break;
        }
        seen.extend_from_slice(&buf[..n]);
    }
    assert_eq!(seen, b"taken");
    timeout(Duration::from_secs(5), taken.recv())
        .await
        .expect("the takeover task finished within the bound")
        .expect("the takeover task reported")
        .expect("the takeover task read, answered and shut down");
    timeout(Duration::from_secs(5), server.shutdown())
        .await
        .unwrap();
}

/// Outlasts any test.
struct Stall;

impl Accept for Stall {
    fn accept(&self, tcp: TcpStream, _peer: SocketAddr) -> BoxFuture<'static, Accepted> {
        Box::pin(async move {
            rusty_tokio::time::sleep(Duration::from_secs(60)).await;
            Accepted::Plain(tcp)
        })
    }
}

#[rusty_tokio::test]
async fn the_handshake_timeout_bounds_the_acceptor() {
    let options = ServeOptions {
        handshake_timeout: Duration::from_millis(100),
        ..ServeOptions::default()
    };
    let server = start_with(options, Stall);
    let tcp = TcpStream::connect_addr(server.local_addr()).await.unwrap();
    assert!(closed(tcp).await, "the stalled connection is closed");
}

/// Serves plain connections under `Admission`.
struct Limited(Admission);

impl Accept for Limited {
    fn accept(&self, tcp: TcpStream, _peer: SocketAddr) -> BoxFuture<'static, Accepted> {
        Box::pin(async move { Accepted::Plain(tcp) })
    }

    fn admission(&self) -> Option<&Admission> {
        Some(&self.0)
    }
}

/// Sends the head of a `POST /echo` with a three-byte body, and no body yet.
async fn post_head_only(server: &Server) -> AsyncTransport<TcpStream> {
    let mut conn = connect(server).await;
    let mut map = HeaderMap::new();
    map.insert("host", "localhost").unwrap();
    map.insert("content-length", "3").unwrap();
    let head = RequestHead {
        method: Method::Post,
        target: "/echo".into(),
        version: Version::Http11,
        headers: map,
    };
    conn.write_request_head(&head).await.unwrap();
    conn
}

#[rusty_tokio::test]
async fn admission_counts_a_request_from_its_head_and_sheds_before_reading_a_body() {
    let server = start_with(
        ServeOptions::default(),
        Limited(Admission::new().with_max_in_flight(1)),
    );
    // An upload in progress holds the one place.
    let mut uploading = post_head_only(&server).await;
    rusty_tokio::time::sleep(Duration::from_millis(50)).await;

    // The next request is shed at once, its body never waited for.
    let mut shed = post_head_only(&server).await;
    let (head, body) = timeout(Duration::from_secs(5), read(&mut shed, &Method::Post))
        .await
        .expect("shed without waiting for the body");
    assert_eq!(head.status.as_u16(), 503);
    assert_eq!(head.headers.get("retry-after"), Some("1"));
    assert_eq!(head.headers.get("connection"), Some("close"));
    assert_eq!(body, "Service Unavailable: too many requests in flight");
    assert!(closed(shed.into_inner()).await);

    // The upload finishes, and its place comes back.
    uploading.write_body(b"abc").await.unwrap();
    assert_eq!(read(&mut uploading, &Method::Post).await.1, "abc");
    let mut next = connect(&server).await;
    send(&mut next, Method::Get, "/health", &[], b"").await;
    assert_eq!(read(&mut next, &Method::Get).await.0.status.as_u16(), 200);
}

#[rusty_tokio::test]
async fn the_admission_deadline_covers_reading_the_body() {
    let options = ServeOptions {
        idle_timeout: Duration::from_secs(30),
        ..ServeOptions::default()
    };
    let server = start_with(
        options,
        Limited(Admission::new().with_timeout(Duration::from_millis(200))),
    );
    let mut stalled = post_head_only(&server).await;
    let (head, _) = timeout(Duration::from_secs(5), read(&mut stalled, &Method::Post))
        .await
        .expect("answered at the deadline, not after the idle timeout");
    assert_eq!(head.status.as_u16(), 504);
    assert_eq!(head.headers.get("connection"), Some("close"));
    assert!(closed(stalled.into_inner()).await);

    let mut quick = connect(&server).await;
    send(&mut quick, Method::Post, "/echo", &[], b"abc").await;
    assert_eq!(read(&mut quick, &Method::Post).await.1, "abc");
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
