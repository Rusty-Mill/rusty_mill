//! Live interop with the `h2` crate, the independent oracle, over plain
//! h2c: the oracle as client of our server, and as server of our client.
//! Both sides run on `rusty_tokio`, each on a runtime of its own; the
//! oracle's sockets go through [`TokioIo`].
#![allow(clippy::unwrap_used)]

mod common;

use std::future::poll_fn;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{ready, Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use common::*;
use rusty_hyper::{connect, ConnectOptions, ServeOptions};

/// A `rusty_tokio` socket under the `tokio::io` traits `h2` is written
/// against, so the oracle runs on `rusty_tokio`: `tokio` supplies only the
/// trait definitions, never a runtime (#903). Twin of `rusty_h2`'s.
struct TokioIo<T>(T);

impl<T: rusty_tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for TokioIo<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let mut ours = rusty_tokio::io::ReadBuf::new(buf.initialize_unfilled());
        ready!(Pin::new(&mut self.0).poll_read(cx, &mut ours))?;
        let n = ours.filled().len();
        buf.advance(n);
        Poll::Ready(Ok(()))
    }
}

impl<T: rusty_tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for TokioIo<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

async fn h2_send(send: &mut h2::SendStream<Bytes>, mut data: Bytes, end: bool) {
    if data.is_empty() {
        send.send_data(Bytes::new(), end).unwrap();
        return;
    }
    while !data.is_empty() {
        send.reserve_capacity(data.len());
        let cap = poll_fn(|cx| send.poll_capacity(cx)).await.unwrap().unwrap();
        let chunk = data.split_to(cap.min(data.len()));
        send.send_data(chunk, end && data.is_empty()).unwrap();
    }
}

async fn h2_drain(
    body: &mut h2::RecvStream,
) -> Result<(Vec<u8>, Option<http::HeaderMap>), h2::Error> {
    let mut out = Vec::new();
    while let Some(chunk) = body.data().await {
        let chunk = chunk?;
        body.flow_control().release_capacity(chunk.len()).unwrap();
        out.extend_from_slice(&chunk);
    }
    Ok((out, body.trailers().await?))
}

/// An `h2` server that echoes each request body with `grpc-status: 0`
/// trailers and returns `x-n` as `x-echo`; one connection, on the oracle's runtime.
fn h2_echo_server(rt: &rusty_tokio::Runtime) -> SocketAddr {
    let listener = rt.block_on(async { listener() });
    let addr = listener.local_addr().unwrap();
    rt.spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        tcp.set_nodelay(true).unwrap();
        let Ok(mut conn) = h2::server::handshake(TokioIo(tcp)).await else {
            return;
        };
        while let Some(Ok((req, mut respond))) = conn.accept().await {
            rusty_tokio::spawn(async move {
                let echo = req.headers().get("x-n").cloned();
                let mut body = req.into_body();
                let Ok((bytes, _)) = h2_drain(&mut body).await else {
                    return;
                };
                let mut resp = http::Response::builder().status(200);
                if let Some(v) = echo {
                    resp = resp.header("x-echo", v);
                }
                let mut send = respond
                    .send_response(resp.body(()).unwrap(), false)
                    .unwrap();
                h2_send(&mut send, Bytes::from(bytes), false).await;
                let mut trailers = http::HeaderMap::new();
                trailers.insert("grpc-status", "0".parse().unwrap());
                send.send_trailers(trailers).unwrap();
            });
        }
    });
    addr
}

/// One runtime for our side, one for the oracle's.
fn runtimes() -> (rusty_tokio::Runtime, rusty_tokio::Runtime) {
    (
        rusty_tokio::Runtime::new().unwrap(),
        rusty_tokio::Runtime::new().unwrap(),
    )
}

#[test]
fn the_h2_crate_calls_our_server() {
    let (ours, oracle) = runtimes();
    let server = ours.block_on(async { start(listener(), None, ServeOptions::default(), echo) });
    let addr = server.local_addr();
    oracle.block_on(async {
        let tcp = rusty_tokio::io::TcpStream::connect(addr).await.unwrap();
        let (client, conn) = h2::client::handshake(TokioIo(tcp)).await.unwrap();
        rusty_tokio::spawn(conn);
        let calls: Vec<_> = (0..16usize)
            .map(|i| {
                let client = client.clone();
                rusty_tokio::spawn(async move {
                    let mut ready = client.ready().await.unwrap();
                    let req = http::Request::post("http://localhost/echo")
                        .header("content-type", "application/grpc")
                        .header("x-n", i.to_string())
                        .body(())
                        .unwrap();
                    let (fut, mut send) = ready.send_request(req, false).unwrap();
                    let body = pattern(5_000 + i * 1_000);
                    h2_send(&mut send, Bytes::from(body.clone()), true).await;
                    let (parts, mut rb) = fut.await.unwrap().into_parts();
                    let (bytes, trailers) = h2_drain(&mut rb).await.unwrap();
                    assert_eq!(parts.status, 200);
                    assert_eq!(parts.headers["x-echo"], i.to_string().as_str());
                    assert_eq!(bytes, body);
                    assert_eq!(trailers.unwrap()["grpc-status"], "0");
                })
            })
            .collect();
        for c in calls {
            c.await.unwrap();
        }
    });
    ours.block_on(server.shutdown());
}

#[test]
fn our_client_calls_the_h2_crate() {
    let (ours, oracle) = runtimes();
    let addr = h2_echo_server(&oracle);
    ours.block_on(async move {
        rusty_tokio::time::timeout(Duration::from_secs(20), async move {
            let conn = connect(&format!("http://{addr}"), ConnectOptions::default())
                .await
                .unwrap();
            let body = pattern(300_000);
            let r = call(conn.client(), "http", &[("x-n", "3".into())], body.clone())
                .await
                .unwrap();
            assert_eq!(header(&r.headers, ":status").as_deref(), Some("200"));
            assert_eq!(header(&r.headers, "x-echo").as_deref(), Some("3"));
            assert_eq!(r.body, body);
            let trailers = r.trailers.expect("trailers");
            assert_eq!(header(&trailers, "grpc-status").as_deref(), Some("0"));
        })
        .await
        .expect("timed out");
    });
}
