//! Live interop with the `h2` crate, the independent oracle, in both
//! directions over loopback TCP. Our side runs on `rusty_tokio`, the oracle
//! on `tokio`; each test owns one runtime of each kind.

use std::future::poll_fn;
use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;
use h2::{Reason, SendStream};
use rusty_h2::client::builder::Builder as ClientBuilder;
use rusty_h2::driver::{self, Body, ClientHandle, Error};
use rusty_h2::error::ErrorCode;
use rusty_h2::hpack::HeaderField;
use rusty_h2::server::builder::Builder as ServerBuilder;

fn hf(n: &str, v: &str) -> HeaderField {
    HeaderField::new(n, v)
}

fn req_fields(path: &str, extra: &[(&str, String)]) -> Vec<HeaderField> {
    let mut f = vec![
        hf(":method", "POST"),
        hf(":scheme", "http"),
        hf(":path", path),
        hf(":authority", "localhost"),
        hf("content-type", "application/grpc"),
    ];
    f.extend(extra.iter().map(|(n, v)| hf(n, v)));
    f
}

fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i.wrapping_mul(31) % 251) as u8).collect()
}

// ---- the oracle's halves ----------------------------------------------------

/// Sends `data` on an `h2` stream, waiting for the capacity it grants.
async fn h2_send(send: &mut SendStream<Bytes>, mut data: Bytes, end: bool) {
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

/// Reads an `h2` body to its end: the bytes and the trailers.
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
/// trailers, and returns each request's `x-n` header as `x-echo`. Serves one
/// connection on a tokio runtime and yields its address.
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
                let mut resp = http::Response::builder()
                    .status(200)
                    .header("content-type", "application/grpc");
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

async fn h2_call(
    client: &mut h2::client::SendRequest<Bytes>,
    path: &str,
    n: Option<String>,
    body: Vec<u8>,
) -> (http::response::Parts, Vec<u8>, Option<http::HeaderMap>) {
    let mut req = http::Request::post(format!("http://localhost{path}"))
        .header("content-type", "application/grpc");
    if let Some(n) = n {
        req = req.header("x-n", n);
    }
    let ready = std::mem::replace(client, client.clone())
        .ready()
        .await
        .unwrap();
    let mut ready = ready;
    let (fut, mut send) = ready.send_request(req.body(()).unwrap(), false).unwrap();
    h2_send(&mut send, Bytes::from(body), true).await;
    let (parts, mut rb) = fut.await.unwrap().into_parts();
    let (bytes, trailers) = h2_drain(&mut rb).await.unwrap();
    (parts, bytes, trailers)
}

// ---- our halves ----------------------------------------------------------------

async fn ours_client(addr: SocketAddr) -> ClientHandle {
    let tcp = rusty_tokio::io::TcpStream::connect(addr).await.unwrap();
    tcp.set_nodelay(true).unwrap();
    let (handle, drv) = driver::client(tcp, ClientBuilder::new());
    rusty_tokio::spawn(drv);
    handle
}

async fn ours_call(
    h: &ClientHandle,
    path: &str,
    extra: &[(&str, String)],
    body: Vec<u8>,
) -> (Vec<HeaderField>, Vec<u8>, Option<Vec<HeaderField>>) {
    let (mut send, resp) = h
        .send_request(req_fields(path, extra), false)
        .await
        .unwrap();
    send.send_data(body, true).await.unwrap();
    let mut resp = resp.await.unwrap();
    let (mut bytes, mut trailers) = (Vec::new(), None);
    while let Some(piece) = resp.body.next().await {
        match piece.unwrap() {
            Body::Data(d) => bytes.extend(d),
            Body::Trailers(t) => trailers = Some(t),
        }
    }
    (resp.headers, bytes, trailers)
}

/// Our server on a rusty_tokio runtime, echoing like the oracle's.
fn ours_echo_server(rt: &rusty_tokio::Runtime) -> SocketAddr {
    let listener = rt.block_on(async {
        rusty_tokio::io::TcpListener::bind("127.0.0.1:0".parse().unwrap()).unwrap()
    });
    let addr = listener.local_addr().unwrap();
    rt.spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        tcp.set_nodelay(true).unwrap();
        let (mut acceptor, drv) = driver::server(tcp, ServerBuilder::new());
        rusty_tokio::spawn(drv);
        while let Some(Ok((mut req, respond))) = acceptor.accept().await {
            rusty_tokio::spawn(async move {
                let echo = req
                    .headers
                    .iter()
                    .find(|h| h.name == b"x-n")
                    .map(|h| h.value.clone());
                let mut bytes = Vec::new();
                while let Some(Ok(piece)) = req.body.next().await {
                    if let Body::Data(d) = piece {
                        bytes.extend(d);
                    }
                }
                let mut fields = vec![hf(":status", "200"), hf("content-type", "application/grpc")];
                if let Some(v) = echo {
                    fields.push(HeaderField::new("x-echo", v));
                }
                let mut send = respond.send_response(fields, false).await.unwrap();
                send.send_data(bytes, false).await.unwrap();
                send.send_trailers(vec![hf("grpc-status", "0")])
                    .await
                    .unwrap();
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

fn within<F: std::future::Future>(rt: &rusty_tokio::Runtime, secs: u64, f: F) -> F::Output {
    rt.block_on(async {
        rusty_tokio::time::timeout(Duration::from_secs(secs), f)
            .await
            .expect("timed out")
    })
}

fn within_tokio<F: std::future::Future>(
    rt: &tokio::runtime::Runtime,
    secs: u64,
    f: F,
) -> F::Output {
    rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(secs), f)
            .await
            .expect("timed out")
    })
}

// ---- our client, their server ---------------------------------------------------------

#[test]
fn our_client_gets_headers_body_and_trailers_from_h2() {
    let (ours, theirs) = runtimes();
    let addr = h2_echo_server(&theirs);
    within(&ours, 10, async {
        let h = ours_client(addr).await;
        let (headers, body, trailers) = ours_call(&h, "/svc/Echo", &[], b"hello".to_vec()).await;
        assert!(headers.contains(&hf(":status", "200")));
        assert_eq!(body, b"hello");
        assert_eq!(trailers, Some(vec![hf("grpc-status", "0")]));
    });
}

#[test]
fn our_client_moves_two_megabytes_each_way_through_h2() {
    let (ours, theirs) = runtimes();
    let addr = h2_echo_server(&theirs);
    within(&ours, 30, async {
        let h = ours_client(addr).await;
        let body = pattern(2 << 20);
        let (_, got, _) = ours_call(&h, "/svc/Echo", &[], body.clone()).await;
        assert!(got == body);
    });
}

#[test]
fn our_client_runs_many_concurrent_calls_against_h2() {
    let (ours, theirs) = runtimes();
    let addr = h2_echo_server(&theirs);
    within(&ours, 20, async {
        let h = ours_client(addr).await;
        let mut tasks = Vec::new();
        for i in 0..48usize {
            let h = h.clone();
            tasks.push(rusty_tokio::spawn(async move {
                let body = pattern(1 + i * 997);
                let (_, got, _) = ours_call(&h, "/svc/Echo", &[], body.clone()).await;
                assert!(got == body);
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
    });
}

#[test]
fn our_hpack_encoder_and_decoder_agree_with_h2s_across_many_requests() {
    let (ours, theirs) = runtimes();
    let addr = h2_echo_server(&theirs);
    within(&ours, 30, async {
        let h = ours_client(addr).await;
        for i in 0..200usize {
            // Repeated, growing, long and short values churn the dynamic tables.
            let n = format!("{}-{}", i, "v".repeat((i * 37) % 3000));
            let (headers, body, _) =
                ours_call(&h, "/svc/Hpack", &[("x-n", n.clone())], vec![i as u8]).await;
            assert!(headers.contains(&hf("x-echo", &n)), "request {i}");
            assert_eq!(body, vec![i as u8]);
        }
    });
}

#[test]
fn our_ping_is_acknowledged_by_h2() {
    let (ours, theirs) = runtimes();
    let addr = h2_echo_server(&theirs);
    within(&ours, 10, async {
        let h = ours_client(addr).await;
        h.ping().await.unwrap();
    });
}

#[test]
fn an_h2_reset_fails_our_response_with_its_reason() {
    let (ours, theirs) = runtimes();
    let listener = theirs
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let addr = listener.local_addr().unwrap();
    theirs.spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        tcp.set_nodelay(true).unwrap();
        let Ok(mut conn) = h2::server::handshake(tcp).await else {
            return;
        };
        while let Some(Ok((_req, mut respond))) = conn.accept().await {
            respond.send_reset(Reason::REFUSED_STREAM);
        }
    });
    within(&ours, 10, async {
        let h = ours_client(addr).await;
        let (_s, resp) = h
            .send_request(req_fields("/svc/Refuse", &[]), true)
            .await
            .unwrap();
        match resp.await {
            Err(Error::Reset(ErrorCode::RefusedStream)) => {}
            other => panic!("{:?}", other.err()),
        }
    });
}

#[test]
fn a_trailers_only_response_from_h2_has_headers_and_no_body() {
    let (ours, theirs) = runtimes();
    let listener = theirs
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let addr = listener.local_addr().unwrap();
    theirs.spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        tcp.set_nodelay(true).unwrap();
        let Ok(mut conn) = h2::server::handshake(tcp).await else {
            return;
        };
        while let Some(Ok((_req, mut respond))) = conn.accept().await {
            let r = http::Response::builder()
                .status(200)
                .header("content-type", "application/grpc")
                .header("grpc-status", "12")
                .header("grpc-message", "nope")
                .body(())
                .unwrap();
            respond.send_response(r, true).unwrap();
        }
    });
    within(&ours, 10, async {
        let h = ours_client(addr).await;
        let (_s, resp) = h
            .send_request(req_fields("/svc/Missing", &[]), true)
            .await
            .unwrap();
        let mut resp = resp.await.unwrap();
        assert!(resp.headers.contains(&hf("grpc-status", "12")));
        assert!(resp.body.next().await.is_none());
    });
}

#[test]
fn our_reset_reaches_h2_as_cancel() {
    let (ours, theirs) = runtimes();
    let listener = theirs
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    theirs.spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        tcp.set_nodelay(true).unwrap();
        let Ok(mut conn) = h2::server::handshake(tcp).await else {
            return;
        };
        let tx = tx.clone();
        tokio::spawn(async move {
            while let Some(Ok((req, _respond))) = conn.accept().await {
                let tx = tx.clone();
                tokio::spawn(async move {
                    let mut body = req.into_body();
                    while let Some(chunk) = body.data().await {
                        match chunk {
                            Ok(c) => body.flow_control().release_capacity(c.len()).unwrap(),
                            Err(e) => {
                                let _ = tx.send(e.reason());
                                return;
                            }
                        }
                    }
                });
            }
        });
    });
    within(&ours, 10, async {
        let h = ours_client(addr).await;
        let (mut send, _resp) = h
            .send_request(req_fields("/svc/Cancel", &[]), false)
            .await
            .unwrap();
        send.send_data(b"part".to_vec(), false).await.unwrap();
        send.reset(ErrorCode::Cancel);
        rusty_tokio::time::sleep(Duration::from_millis(500)).await;
    });
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        Some(Reason::CANCEL)
    );
}

#[test]
fn an_h2_graceful_shutdown_ends_our_driver_and_refuses_new_calls() {
    let (ours, theirs) = runtimes();
    let listener = theirs
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let addr = listener.local_addr().unwrap();
    theirs.spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        tcp.set_nodelay(true).unwrap();
        let Ok(mut conn) = h2::server::handshake(tcp).await else {
            return;
        };
        conn.graceful_shutdown();
        while conn.accept().await.is_some() {}
    });
    within(&ours, 10, async {
        let tcp = rusty_tokio::io::TcpStream::connect(addr).await.unwrap();
        tcp.set_nodelay(true).unwrap();
        let (h, drv) = driver::client(tcp, ClientBuilder::new());
        let task = rusty_tokio::spawn(drv);
        assert!(task.await.unwrap().is_ok(), "the driver ends cleanly");
        assert!(h
            .send_request(req_fields("/svc/Late", &[]), true)
            .await
            .is_err());
    });
}

// ---- their client, our server --------------------------------------------------------------

fn h2_client(rt: &tokio::runtime::Runtime, addr: SocketAddr) -> h2::client::SendRequest<Bytes> {
    rt.block_on(async {
        let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
        tcp.set_nodelay(true).unwrap();
        let (client, conn) = h2::client::handshake(tcp).await.unwrap();
        tokio::spawn(async move {
            let _ = conn.await;
        });
        client
    })
}

#[test]
fn h2_gets_headers_body_and_trailers_from_our_server() {
    let (ours, theirs) = runtimes();
    let addr = ours_echo_server(&ours);
    let mut client = h2_client(&theirs, addr);
    let (parts, body, trailers) = within_tokio(
        &theirs,
        10,
        h2_call(&mut client, "/svc/Echo", None, b"hello".to_vec()),
    );
    assert_eq!(parts.status, 200);
    assert_eq!(body, b"hello");
    assert_eq!(trailers.unwrap().get("grpc-status").unwrap(), "0");
}

#[test]
fn h2_moves_two_megabytes_each_way_through_our_server() {
    let (ours, theirs) = runtimes();
    let addr = ours_echo_server(&ours);
    let mut client = h2_client(&theirs, addr);
    let body = pattern(2 << 20);
    let (_, got, _) = within_tokio(
        &theirs,
        30,
        h2_call(&mut client, "/svc/Echo", None, body.clone()),
    );
    assert!(got == body);
}

#[test]
fn h2_runs_many_concurrent_calls_against_our_server() {
    let (ours, theirs) = runtimes();
    let addr = ours_echo_server(&ours);
    let client = h2_client(&theirs, addr);
    within_tokio(&theirs, 20, async move {
        let mut tasks = Vec::new();
        for i in 0..48usize {
            let mut c = client.clone();
            tasks.push(tokio::spawn(async move {
                let body = pattern(1 + i * 997);
                let (_, got, _) = h2_call(&mut c, "/svc/Echo", None, body.clone()).await;
                assert!(got == body);
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
    });
}

#[test]
fn h2s_hpack_encoder_and_decoder_agree_with_ours_across_many_requests() {
    let (ours, theirs) = runtimes();
    let addr = ours_echo_server(&ours);
    let mut client = h2_client(&theirs, addr);
    within_tokio(&theirs, 30, async move {
        for i in 0..200usize {
            let n = format!("{}-{}", i, "v".repeat((i * 37) % 3000));
            let (parts, body, _) =
                h2_call(&mut client, "/svc/Hpack", Some(n.clone()), vec![i as u8]).await;
            assert_eq!(
                parts.headers.get("x-echo").unwrap(),
                n.as_str(),
                "request {i}"
            );
            assert_eq!(body, vec![i as u8]);
        }
    });
}

#[test]
fn an_h2_reset_reaches_our_server_body_as_cancel() {
    let (ours, theirs) = runtimes();
    let listener = ours.block_on(async {
        rusty_tokio::io::TcpListener::bind("127.0.0.1:0".parse().unwrap()).unwrap()
    });
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    // A request reset before it is accepted is dropped unseen, so the reset
    // is only sent once our server has accepted the request.
    let (accepted_tx, accepted_rx) = std::sync::mpsc::channel();
    ours.spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        tcp.set_nodelay(true).unwrap();
        let (mut acceptor, drv) = driver::server(tcp, ServerBuilder::new());
        rusty_tokio::spawn(drv);
        if let Some(Ok((mut req, _respond))) = acceptor.accept().await {
            let _ = accepted_tx.send(());
            while let Some(piece) = req.body.next().await {
                if let Err(Error::Reset(code)) = piece {
                    let _ = tx.send(code);
                }
            }
        }
    });
    let client = h2_client(&theirs, addr);
    within_tokio(&theirs, 10, async move {
        let mut c = client.ready().await.unwrap();
        let req = http::Request::post("http://localhost/svc/Cancel")
            .body(())
            .unwrap();
        let (_fut, mut send) = c.send_request(req, false).unwrap();
        h2_send(&mut send, Bytes::from_static(b"part"), false).await;
        while accepted_rx.try_recv().is_err() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        send.send_reset(Reason::CANCEL);
        tokio::time::sleep(Duration::from_millis(500)).await;
    });
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        ErrorCode::Cancel
    );
}
