//! Live interop with the `h2` crate, the independent oracle, over plain
//! h2c: the oracle as client of our server, and as server of our client.
//! Our side runs on `rusty_tokio`, the oracle on `tokio`.

mod common;

use std::future::poll_fn;
use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;
use common::*;
use rusty_hyper::{connect, serve, ConnectOptions, ServeOptions};

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
/// trailers and returns `x-n` as `x-echo`; one connection, on a tokio runtime.
fn h2_echo_server(rt: &tokio::runtime::Runtime) -> SocketAddr {
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let addr = listener.local_addr().unwrap();
    rt.spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        tcp.set_nodelay(true).unwrap();
        let Ok(mut conn) = h2::server::handshake(tcp).await else {
            return;
        };
        while let Some(Ok((req, mut respond))) = conn.accept().await {
            tokio::spawn(async move {
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

fn runtimes() -> (rusty_tokio::Runtime, tokio::runtime::Runtime) {
    (
        rusty_tokio::Runtime::new().unwrap(),
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap(),
    )
}

#[test]
fn the_h2_crate_calls_our_server() {
    let (ours, oracle) = runtimes();
    let server = ours.block_on(async { serve(listener(), None, ServeOptions::default(), echo) });
    let addr = server.local_addr();
    oracle.block_on(async {
        let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (client, conn) = h2::client::handshake(tcp).await.unwrap();
        tokio::spawn(conn);
        let calls: Vec<_> = (0..16usize)
            .map(|i| {
                let client = client.clone();
                tokio::spawn(async move {
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
